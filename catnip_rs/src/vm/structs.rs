// FILE: catnip_rs/src/vm/structs.rs
//! Native struct types for the Catnip VM.
//!
//! Provides O(1) field access by position instead of Python getattr.

use super::value::Value;
use catnip_tools::suggest::{format_suggestion, suggest_similar};
use pyo3::PyTraverseError;
use pyo3::exceptions::PyTypeError;
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub type StructTypeId = u32;

pub struct StructParents {
    pub implements: Vec<String>,
    pub mro: Vec<String>,
    pub parent_names: Vec<String>,
}

pub struct StructMethods {
    pub instance: IndexMap<String, Py<PyAny>>,
    pub statics: IndexMap<String, Py<PyAny>>,
    pub abstract_methods: HashSet<MethodKey>,
}

/// Discriminant for method dispatch (instance vs static).
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub enum MethodKind {
    Instance,
    Static,
}

/// Composite key for abstract method tracking: (name, kind).
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct MethodKey {
    pub name: String,
    pub kind: MethodKind,
}

/// A field in a struct type definition.
#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub has_default: bool,
    pub default: Value,
    /// Runtime type contract from the field annotation (`x: int`), classified
    /// once at struct definition. `ParamCheck::None` for unannotated or
    /// unenforceable fields. Travels with the field across inheritance/trait
    /// merges (the field is cloned), mirroring the PureVM `PureStructField`.
    pub check: catnip_core::vm::opcode::ParamCheck,
}

/// A struct type registered in the VM.
pub struct StructType {
    pub id: StructTypeId,
    pub name: String,
    pub fields: Vec<StructField>,
    /// Raw VMFunction callables keyed by method name.
    pub methods: IndexMap<String, Py<PyAny>>,
    /// Static methods callable on type and instances (no self binding).
    pub static_methods: IndexMap<String, Py<PyAny>>,
    /// List of trait names this struct implements.
    pub implements: Vec<String>,
    /// Method resolution order (MRO) - includes struct parents and traits.
    pub mro: Vec<String>,
    /// Direct parent struct names (from extends).
    pub parent_names: Vec<String>,
    /// Abstract methods that remain unimplemented.
    pub abstract_methods: HashSet<MethodKey>,
}

impl StructType {
    /// Find field index by name.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }

    /// Collect all accessible names (fields + methods + static methods).
    pub fn available_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.fields.iter().map(|f| f.name.as_str()).collect();
        names.extend(self.methods.keys().map(|k| k.as_str()));
        names.extend(self.static_methods.keys().map(|k| k.as_str()));
        names
    }
}

// Manual Debug since Py<PyAny> doesn't implement Debug
impl std::fmt::Debug for StructType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructType")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("fields", &self.fields)
            .field("methods", &format!("<{} methods>", self.methods.len()))
            .field("static_methods", &format!("<{} static>", self.static_methods.len()))
            .field("implements", &self.implements)
            .field("mro", &self.mro)
            .finish()
    }
}

/// A live struct instance with field values stored by position.
#[derive(Debug, Clone)]
pub struct StructInstance {
    pub type_id: StructTypeId,
    pub fields: Vec<Value>,
}

/// Slot wrapper for refcount tracking in StructRegistry.
/// refcount uses AtomicU32 so incref can take &self (needed when accessing
/// the registry through the *const thread-local while to_pyobject recurses)
/// and satisfies Send+Sync required by PyO3 #[pyclass].
///
/// `frozen` is set when the instance is hashed (proxy __hash__ or any other
/// hashing path). Once true, SetAttr in the VM and __setattr__ on the proxy
/// must reject mutation, otherwise dict/set lookups using the prior hash
/// would silently break.
#[derive(Debug)]
struct InstanceSlot {
    instance: StructInstance,
    refcount: AtomicU32,
    frozen: AtomicBool,
    /// True if this slot is a `clone_from_parent` bit-copy of a parent slot.
    /// The clone takes one count per non-struct heap field (their global
    /// counts -- OBJECT_TABLE, heap Arcs -- are shared with the parent's
    /// copy), so every child-side release (SetAttr overwrite, cascade on
    /// decref-to-zero, registry drop) consumes the child's own ref, never
    /// the parent's. See `clone_from_parent` and `Drop for StructRegistry`.
    snapshot: bool,
}

impl InstanceSlot {
    /// Sole constructor so every live slot hits [`LIVE_STRUCT_INSTANCES`]
    /// (`Drop` decrements it; a literal `InstanceSlot { .. }` would underflow).
    fn new(instance: StructInstance, refcount: u32, frozen: bool, snapshot: bool) -> Self {
        LIVE_STRUCT_INSTANCES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        InstanceSlot {
            instance,
            refcount: AtomicU32::new(refcount),
            frozen: AtomicBool::new(frozen),
            snapshot,
        }
    }
}

impl Drop for InstanceSlot {
    fn drop(&mut self) {
        LIVE_STRUCT_INSTANCES.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Python-visible proxy for struct instances returned by to_pyobject.
#[pyclass(module = "catnip._rs", name = "CatnipStruct")]
pub struct CatnipStructProxy {
    pub type_name: String,
    pub field_names: Vec<String>,
    pub field_values: Vec<Py<PyAny>>,
    /// Raw VMFunction callables (not yet bound).
    pub methods: IndexMap<String, Py<PyAny>>,
    /// Static methods (no self binding).
    pub static_methods: IndexMap<String, Py<PyAny>>,
    /// Back-reference to the CatnipStructType (AST mode). None for VM-created proxies.
    pub struct_type: Option<Py<CatnipStructType>>,
    /// VM struct instance index for round-trip through Python collections.
    /// Allows from_pyobject to restore TAG_STRUCT instead of falling back to TAG_PYOBJ.
    pub native_instance_idx: Option<u32>,
    /// Identity of the registry that owns `native_instance_idx`. The proxy
    /// resolves it through the identity table, never the set-and-leave
    /// thread-local registry, so incref/decref/freeze hit its own registry.
    /// 0 when there is no native instance (AST mode).
    pub native_registry_id: u64,
    /// Set to true after the first __hash__ call. Mutating a hashed struct would
    /// break Python's hash/eq invariants (dict/set lookup), so __setattr__ rejects
    /// further field writes once this flag is on.
    pub frozen: bool,
}

impl CatnipStructProxy {
    /// Detached DEEP copy: own refs on the field values, no registry anchor,
    /// recursing into nested struct fields. The execution-boundary rule
    /// (BROADCAST_SPEC decision 4, deep 2026-07-11): a Catnip callback invoked
    /// FROM Python receives private copies of struct arguments, so mutations at
    /// ANY depth do not escape across the boundary. Identity-preserving (a
    /// nested proxy shared by two fields is copied once) and cycle-safe (the
    /// copy is registered before its fields are filled). Non-proxy fields
    /// (lists, dicts, scalars) are shared by ref, like the source shares them.
    pub(crate) fn detached_copy(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let mut memo: std::collections::HashMap<u64, Py<PyAny>> = std::collections::HashMap::new();
        self.deep_detached_copy(py, &mut memo)
    }

    /// Recursive worker for [`detached_copy`]. `memo` maps a source proxy
    /// address to its copy so shared and cyclic references resolve to one copy.
    fn deep_detached_copy(
        &self,
        py: Python<'_>,
        memo: &mut std::collections::HashMap<u64, Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let key = std::ptr::from_ref(self) as u64;
        if let Some(existing) = memo.get(&key) {
            return Ok(existing.clone_ref(py));
        }
        // Phase 1: build the copy with EMPTY fields and register it before
        // filling, so a self/back reference during the fill resolves to this
        // copy instead of looping.
        let copy = Py::new(
            py,
            CatnipStructProxy {
                type_name: self.type_name.clone(),
                field_names: self.field_names.clone(),
                field_values: Vec::new(),
                methods: self.methods.iter().map(|(k, v)| (k.clone(), v.clone_ref(py))).collect(),
                static_methods: self
                    .static_methods
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                    .collect(),
                struct_type: self.struct_type.as_ref().map(|st| st.clone_ref(py)),
                native_instance_idx: None,
                native_registry_id: 0,
                frozen: self.frozen,
            },
        )?;
        memo.insert(key, copy.clone_ref(py).into_any());
        // Phase 2: fill -- recurse into nested struct proxies, share the rest.
        let mut fields = Vec::with_capacity(self.field_values.len());
        for v in &self.field_values {
            let bound = v.bind(py);
            if let Ok(proxy) = bound.cast::<CatnipStructProxy>() {
                fields.push(proxy.borrow().deep_detached_copy(py, memo)?);
            } else {
                fields.push(v.clone_ref(py));
            }
        }
        copy.borrow_mut(py).field_values = fields;
        Ok(copy.into_any())
    }
}

impl Drop for CatnipStructProxy {
    /// Release the registry slot this proxy held. The real finalizer: PyO3 runs
    /// `Drop` via `tp_dealloc` (it does not wire `fn __del__` to `tp_finalize`).
    /// No-op for AST-mode proxies (no native instance) and for proxies that
    /// outlive their broadcast child (the registry is gone -- see
    /// `proxy_registry_decref`).
    fn drop(&mut self) {
        if let Some(idx) = self.native_instance_idx {
            super::value::proxy_registry_decref(self.native_registry_id, idx);
        }
    }
}

#[pymethods]
impl CatnipStructProxy {
    /// Participate in CPython's cyclic GC. A struct instance carries its own
    /// copy of the type's `methods`/`static_methods` (callables capturing the
    /// scope -> registry -> ctx) plus a back-reference to `struct_type`, so a
    /// retained instance would otherwise pin its context through an opaque Rust
    /// pyclass. `field_values` may also hold other context-referencing values.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for v in &self.field_values {
            visit.call(v)?;
        }
        for v in self.methods.values() {
            visit.call(v)?;
        }
        for v in self.static_methods.values() {
            visit.call(v)?;
        }
        if let Some(ref st) = self.struct_type {
            visit.call(st)?;
        }
        Ok(())
    }

    /// Break the instance's reference cycles. Only called by the GC on an
    /// otherwise-unreachable proxy; the `Drop` impl (registry-slot release) still
    /// runs afterwards at `tp_dealloc` and touches no `Py` fields.
    fn __clear__(&mut self) {
        self.field_values.clear();
        self.methods.clear();
        self.static_methods.clear();
        self.struct_type = None;
    }

    fn __getattr__(slf: PyRef<'_, Self>, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        // Check fields first
        for (i, fname) in slf.field_names.iter().enumerate() {
            if fname == name {
                return Ok(slf.field_values[i].clone_ref(py));
            }
        }
        // Then methods - create BoundCatnipMethod on the fly, bound to THIS
        // proxy. Binding a clone made `self` a detached copy in AST mode
        // (cloned field_values): a method mutating self wrote to the clone and
        // the mutation silently died with the call. The VM never noticed --
        // its clone carried native_instance_idx, so SetAttr reached the shared
        // registry slot anyway.
        if let Some(func) = slf.methods.get(name) {
            let func = func.clone_ref(py);
            let native_instance_idx = slf.native_instance_idx;
            let native_registry_id = slf.native_registry_id;
            let instance: Py<PyAny> = slf.into_pyobject(py)?.into_any().unbind();
            let bound = Py::new(
                py,
                crate::core::BoundCatnipMethod {
                    func,
                    instance,
                    super_source_type: None,
                    native_instance_idx,
                    native_registry_id,
                },
            )?;
            return Ok(bound.into_any());
        }
        // Then static methods - return raw callable (no self binding)
        if let Some(func) = slf.static_methods.get(name) {
            return Ok(func.clone_ref(py));
        }
        // Try static methods from the type (AST mode)
        if let Some(ref st) = slf.struct_type {
            let st_ref = st.bind(py).borrow();
            if let Some(func) = st_ref.static_methods.get(name) {
                return Ok(func.clone_ref(py));
            }
        }
        let mut candidates: Vec<&str> = slf.field_names.iter().map(|s| s.as_str()).collect();
        candidates.extend(slf.methods.keys().map(|s| s.as_str()));
        candidates.extend(slf.static_methods.keys().map(|s| s.as_str()));
        let suggestions = suggest_similar(name, &candidates, 1, 0.6);
        let msg = match format_suggestion(&suggestions) {
            Some(hint) => format!("'{}' has no attribute '{name}'. {hint}", slf.type_name),
            None => format!("'{}' has no attribute '{name}'", slf.type_name),
        };
        Err(pyo3::exceptions::PyAttributeError::new_err(msg))
    }

    fn __setattr__(&mut self, name: &str, value: Py<PyAny>) -> PyResult<()> {
        let registry_frozen = self
            .native_instance_idx
            .is_some_and(|idx| super::value::proxy_registry_is_frozen(self.native_registry_id, idx));
        if self.frozen || registry_frozen {
            return Err(PyTypeError::new_err(format!(
                "cannot mutate '{}' after it has been hashed (used as dict/set key)",
                self.type_name
            )));
        }
        for (i, fname) in self.field_names.iter().enumerate() {
            if fname == name {
                self.field_values[i] = value;
                return Ok(());
            }
        }
        let candidates: Vec<&str> = self.field_names.iter().map(|s| s.as_str()).collect();
        let suggestions = suggest_similar(name, &candidates, 1, 0.6);
        let msg = match format_suggestion(&suggestions) {
            Some(hint) => format!("'{}' has no field '{name}'. {hint}", self.type_name),
            None => format!("'{}' has no field '{name}'", self.type_name),
        };
        Err(pyo3::exceptions::PyAttributeError::new_err(msg))
    }

    fn __dir__(&self) -> Vec<String> {
        let mut names: Vec<String> = self.field_names.clone();
        names.extend(self.methods.keys().cloned());
        names.extend(self.static_methods.keys().cloned());
        names
    }

    fn __str__(&self, py: Python<'_>) -> String {
        self.__repr__(py)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let fields: Vec<String> = self
            .field_names
            .iter()
            .zip(&self.field_values)
            .map(|(n, v)| {
                let repr = v.bind(py).repr().map(|r| r.to_string()).unwrap_or_else(|_| "?".into());
                format!("{n}={repr}")
            })
            .collect();
        format!("{}({})", self.type_name, fields.join(", "))
    }

    fn __hash__(slf: Bound<'_, Self>) -> PyResult<isize> {
        let py = slf.py();

        // Strategy decision (matches __richcmp__ fallback for op_eq):
        //   - op_hash defined           -> user hash (user owns op_hash/op_eq consistency)
        //   - op_eq without op_hash     -> unhashable (would break a == b => hash(a) == hash(b))
        //   - neither                   -> structural hash (mirrors structural fallback eq)
        enum Strategy {
            Custom(Py<PyAny>),
            Unhashable,
            Structural,
        }
        let (strategy, type_name) = {
            let r = slf.borrow();
            let s = if let Some(f) = r.methods.get("op_hash") {
                Strategy::Custom(f.clone_ref(py))
            } else if r.methods.contains_key("op_eq") {
                Strategy::Unhashable
            } else {
                Strategy::Structural
            };
            (s, r.type_name.clone())
        };

        // Hash values are signed (Py_hash_t = isize). Negative results from
        // user-defined op_hash are valid; the structural path goes through
        // i64 to preserve the full bit pattern.
        let hash_value: isize = match strategy {
            Strategy::Custom(func) => {
                let args = PyTuple::new(py, [slf.as_any()])?;
                let result = func.call(py, &args, None)?;
                result.extract::<isize>(py)?
            }
            Strategy::Unhashable => {
                return Err(PyTypeError::new_err(format!(
                    "unhashable type: '{type_name}' (defines op_eq without op_hash)"
                )));
            }
            Strategy::Structural => {
                // type_name + each field's Python __hash__.
                // Raises TypeError (via field.hash()) if any field is unhashable.
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                let r = slf.borrow();
                r.type_name.hash(&mut hasher);
                for field in &r.field_values {
                    let h = field.bind(py).hash()?;
                    h.hash(&mut hasher);
                }
                hasher.finish() as i64 as isize
            }
        };

        // Hash succeeded -> instance can be a dict/set key, so further mutation
        // would break hash stability. Lock __setattr__ on this proxy and, in VM
        // mode, on the registry-backed instance too (so direct SetAttr through
        // bytecode also rejects mutation).
        let (native_idx, native_registry_id) = {
            let b = slf.borrow();
            (b.native_instance_idx, b.native_registry_id)
        };
        if let Some(idx) = native_idx {
            super::value::proxy_registry_freeze(native_registry_id, idx);
        }
        slf.borrow_mut().frozen = true;
        Ok(hash_value)
    }

    fn __richcmp__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>, op: pyo3::pyclass::CompareOp) -> PyResult<Py<PyAny>> {
        use pyo3::pyclass::CompareOp;
        let py = slf.py();
        let method_name = match op {
            CompareOp::Eq => "op_eq",
            CompareOp::Ne => "op_ne",
            CompareOp::Lt => "op_lt",
            CompareOp::Le => "op_le",
            CompareOp::Gt => "op_gt",
            CompareOp::Ge => "op_ge",
        };
        let result = Self::dispatch_binop(&slf, py, method_name, &other);
        match result {
            Ok(val) if val.bind(py).is(py.NotImplemented()) => {
                // No method defined -- structural fallback for eq/ne
                match op {
                    CompareOp::Eq => Self::structural_eq(&slf, py, &other),
                    CompareOp::Ne => Self::structural_ne(&slf, py, &other),
                    _ => Ok(py.NotImplemented()),
                }
            }
            other => other,
        }
    }

    // --- Operator overloading: forward dunders to op_* methods ---

    fn __add__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_add", &other)
    }

    fn __sub__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_sub", &other)
    }

    fn __mul__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_mul", &other)
    }

    fn __truediv__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_div", &other)
    }

    fn __floordiv__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_floordiv", &other)
    }

    fn __mod__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_mod", &other)
    }

    fn __pow__(
        slf: Bound<'_, Self>,
        other: Bound<'_, PyAny>,
        _modulo: Option<Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_pow", &other)
    }

    fn __neg__(slf: Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        Self::dispatch_unaryop(&slf, slf.py(), "op_neg")
    }

    fn __pos__(slf: Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        Self::dispatch_unaryop(&slf, slf.py(), "op_pos")
    }

    // --- Bitwise operators ---

    fn __and__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_band", &other)
    }

    fn __or__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_bor", &other)
    }

    fn __xor__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_bxor", &other)
    }

    fn __lshift__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_lshift", &other)
    }

    fn __rshift__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_binop(&slf, slf.py(), "op_rshift", &other)
    }

    fn __invert__(slf: Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        Self::dispatch_unaryop(&slf, slf.py(), "op_bnot")
    }

    // --- Reverse operators: dispatch to same op_* with swapped args ---

    fn __radd__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_add", &other)
    }

    fn __rsub__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_sub", &other)
    }

    fn __rmul__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_mul", &other)
    }

    fn __rtruediv__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_div", &other)
    }

    fn __rfloordiv__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_floordiv", &other)
    }

    fn __rmod__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_mod", &other)
    }

    fn __rpow__(
        slf: Bound<'_, Self>,
        other: Bound<'_, PyAny>,
        _modulo: Option<Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_pow", &other)
    }

    fn __rand__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_band", &other)
    }

    fn __ror__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_bor", &other)
    }

    fn __rxor__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_bxor", &other)
    }

    fn __rlshift__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_lshift", &other)
    }

    fn __rrshift__(slf: Bound<'_, Self>, other: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::dispatch_rbinop(&slf, slf.py(), "op_rshift", &other)
    }

    // --- Membership ---

    fn __contains__(slf: Bound<'_, Self>, item: Bound<'_, PyAny>) -> PyResult<bool> {
        let py = slf.py();
        let result = Self::dispatch_binop(&slf, py, "op_in", &item)?;
        if result.bind(py).is(py.NotImplemented()) {
            return Err(PyTypeError::new_err(format!(
                "argument of type '{}' is not iterable",
                slf.borrow().type_name
            )));
        }
        result.bind(py).is_truthy()
    }
}

impl CatnipStructProxy {
    /// Dispatch a binary operator to the corresponding op_* method.
    /// Returns NotImplemented if the method doesn't exist (Python protocol).
    fn dispatch_binop(
        slf: &Bound<'_, Self>,
        py: Python<'_>,
        op_name: &str,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let func = {
            let r = slf.borrow();
            r.methods.get(op_name).map(|f| f.clone_ref(py))
        };
        if let Some(func) = func {
            let args = PyTuple::new(py, [slf.as_any(), other])?;
            func.call(py, &args, None)
        } else {
            Ok(py.NotImplemented())
        }
    }

    /// Dispatch a reverse binary operator: call op_* with (self, other).
    /// `other` is the left-hand operand. For non-commutative ops the user's
    /// op_* method receives (self, other) and must handle the reversed order.
    /// Returns NotImplemented if the method doesn't exist (Python protocol).
    fn dispatch_rbinop(
        slf: &Bound<'_, Self>,
        py: Python<'_>,
        op_name: &str,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let func = {
            let r = slf.borrow();
            r.methods.get(op_name).map(|f| f.clone_ref(py))
        };
        if let Some(func) = func {
            // (self, other) where other is the left-hand value
            let args = PyTuple::new(py, [slf.as_any(), other.as_any()])?;
            func.call(py, &args, None)
        } else {
            Ok(py.NotImplemented())
        }
    }

    /// Dispatch a unary operator to the corresponding op_* method.
    /// Raises TypeError if the method doesn't exist.
    fn dispatch_unaryop(slf: &Bound<'_, Self>, py: Python<'_>, op_name: &str) -> PyResult<Py<PyAny>> {
        let func = {
            let r = slf.borrow();
            r.methods.get(op_name).map(|f| f.clone_ref(py))
        };
        if let Some(func) = func {
            let args = PyTuple::new(py, [slf.as_any()])?;
            func.call(py, &args, None)
        } else {
            let type_name = slf.borrow().type_name.clone();
            Err(PyTypeError::new_err(format!(
                "bad operand type for unary op: '{type_name}'"
            )))
        }
    }

    /// Structural equality: same type + all fields equal.
    fn structural_eq(slf: &Bound<'_, Self>, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(other_struct) = other.cast::<CatnipStructProxy>() {
            let self_ref = slf.borrow();
            let other_ref = other_struct.borrow();
            if self_ref.type_name != other_ref.type_name {
                return Ok(false.into_pyobject(py).unwrap().to_owned().into_any().unbind());
            }
            if self_ref.field_values.len() != other_ref.field_values.len() {
                return Ok(false.into_pyobject(py).unwrap().to_owned().into_any().unbind());
            }
            for (a, b) in self_ref.field_values.iter().zip(&other_ref.field_values) {
                if !a.bind(py).eq(b.bind(py))? {
                    return Ok(false.into_pyobject(py).unwrap().to_owned().into_any().unbind());
                }
            }
            Ok(true.into_pyobject(py).unwrap().to_owned().into_any().unbind())
        } else {
            Ok(false.into_pyobject(py).unwrap().to_owned().into_any().unbind())
        }
    }

    /// Structural inequality: negation of structural_eq.
    fn structural_ne(slf: &Bound<'_, Self>, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let eq_result = Self::structural_eq(slf, py, other)?;
        let is_eq: bool = eq_result.extract(py)?;
        Ok((!is_eq).into_pyobject(py).unwrap().to_owned().into_any().unbind())
    }
}

/// Callable struct type for AST mode. Replaces `dataclasses.make_dataclass`.
///
/// Acts as the constructor: `Point(1, 2)` calls `CatnipStructType.__call__`
/// which creates a `CatnipStructProxy` with the field values.
#[pyclass(module = "catnip._rs", name = "CatnipStructType")]
pub struct CatnipStructType {
    pub name: String,
    pub field_names: Vec<String>,
    /// None = required field, Some(value) = default value
    pub field_defaults: Vec<Option<Py<PyAny>>>,
    /// Runtime type contract per field (parallel to `field_names`), classified
    /// at struct definition. `ParamCheck::None` for unannotated/unenforceable
    /// fields. Empty for types not built from a source struct (union variants,
    /// VM-side proxies), which carry no enforced field annotations in v1.
    pub field_checks: Vec<catnip_core::vm::opcode::ParamCheck>,
    /// Per-variant payload-field templates for a generic union variant type
    /// (parallel to `field_names`); empty for a plain struct. Combined with the
    /// use-site type arguments at the generic-nominal boundary (`CheckGeneric`)
    /// to substitute `T` in a payload contract. See `compute_field_template`.
    pub field_templates: Vec<catnip_core::vm::opcode::FieldTemplate>,
    /// Weak reference to the execution context, used by `__call__` to resolve
    /// whether a nominal field type is known at runtime (mirrors the VM host).
    /// Weak, not strong: the context's globals hold this type, so a strong ref
    /// would form an uncollectable `ctx <-> type` cycle that leaks every
    /// `Catnip()` session. `None` when built outside an AST execution (union
    /// variants, VM proxies), where field-type enforcement does not apply.
    pub ctx_weakref: Option<Py<PyAny>>,
    /// Method callables keyed by name (raw VMFunction/Lambda)
    pub methods: IndexMap<String, Py<PyAny>>,
    /// Static methods (no self binding, callable on type and instances).
    pub static_methods: IndexMap<String, Py<PyAny>>,
    /// Post-constructor init function (if defined)
    pub init_fn: Option<Py<PyAny>>,
    /// Direct parent struct names (from extends).
    pub parent_names: Vec<String>,
    /// Method resolution order (struct parents).
    pub mro: Vec<String>,
    /// Implemented trait names (from `implements(...)`), for nominal subtyping.
    pub implements: Vec<String>,
    /// Abstract methods that remain unimplemented.
    pub abstract_methods: HashSet<MethodKey>,
}

#[pymethods]
impl CatnipStructType {
    /// Participate in CPython's cyclic GC. The type is stored in `ctx.globals`
    /// and its `methods`/`static_methods`/`init_fn` are callables (Lambda /
    /// VMFunction) that capture the defining scope, reaching the registry and
    /// thus the context back -- a `ctx.globals -> type -> method -> ctx` cycle
    /// the collector cannot see (a Rust pyclass is opaque to it). Any type that
    /// declares a method would otherwise leak its context. `ctx_weakref` is a
    /// weakref (it does not retain the context) but is still surfaced so its
    /// own refcount is balanced. `field_checks` holds no `Py` and is skipped.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for v in self.field_defaults.iter().flatten() {
            visit.call(v)?;
        }
        if let Some(ref w) = self.ctx_weakref {
            visit.call(w)?;
        }
        for v in self.methods.values() {
            visit.call(v)?;
        }
        for v in self.static_methods.values() {
            visit.call(v)?;
        }
        if let Some(ref f) = self.init_fn {
            visit.call(f)?;
        }
        Ok(())
    }

    /// Break the type's reference cycles by dropping the strong references
    /// reported by `__traverse__`. Only called by the GC on an otherwise-
    /// unreachable type. Clearing the weakref is harmless (it holds nothing).
    fn __clear__(&mut self) {
        for default in &mut self.field_defaults {
            *default = None;
        }
        self.ctx_weakref = None;
        self.methods.clear();
        self.static_methods.clear();
        self.init_fn = None;
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(
        slf: &Bound<'_, Self>,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let this = slf.borrow();

        // Guard: cannot instantiate struct with unresolved abstract methods
        if !this.abstract_methods.is_empty() {
            let names: Vec<&str> = this.abstract_methods.iter().map(|k| k.name.as_str()).collect();
            return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "Cannot instantiate '{}': unresolved abstract method(s): {}",
                this.name,
                names.join(", ")
            )));
        }

        let n_fields = this.field_names.len();
        let n_args = args.len();

        if n_args > n_fields {
            return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "{}() takes at most {} argument(s) ({} given)",
                this.name, n_fields, n_args
            )));
        }

        // Build field values: positional args first, then kwargs, then defaults
        let mut field_values: Vec<Option<Py<PyAny>>> = std::iter::repeat_with(|| None).take(n_fields).collect();

        // Apply positional args
        for (i, slot) in field_values.iter_mut().enumerate().take(n_args) {
            *slot = Some(args.get_item(i)?.unbind());
        }

        // Apply kwargs
        if let Some(kw) = kwargs {
            for (key, value) in kw.iter() {
                let key_str: String = key.extract()?;
                let idx = this.field_names.iter().position(|n| n == &key_str).ok_or_else(|| {
                    pyo3::exceptions::PyTypeError::new_err(format!(
                        "{}() got unexpected keyword argument '{}'",
                        this.name, key_str
                    ))
                })?;
                if field_values[idx].is_some() {
                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "{}() got multiple values for argument '{}'",
                        this.name, key_str
                    )));
                }
                field_values[idx] = Some(value.unbind());
            }
        }

        // Apply defaults for missing fields
        for (slot, default) in field_values.iter_mut().zip(this.field_defaults.iter()) {
            if slot.is_none() {
                if let Some(default) = default {
                    *slot = Some(default.clone_ref(py));
                }
            }
        }

        // Validate all fields are set
        let mut final_values = Vec::with_capacity(n_fields);
        for (i, v) in field_values.into_iter().enumerate() {
            match v {
                Some(val) => final_values.push(val),
                None => {
                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "{}() missing required argument: '{}'",
                        this.name, this.field_names[i]
                    )));
                }
            }
        }

        // Enforce declared field types (boundary at the constructor), parity with
        // the VM. Two passes mirror the other executors: validate every field
        // read-only, then coerce primitives. On a mismatch we return before any
        // coercion, and `final_values` drops here, releasing each Py via RAII.
        // Only runs when a ctx weakref was captured (AST-defined struct); union
        // variants and VM proxies carry empty `field_checks`/no ctx, so this is a
        // no-op. The boundary helpers live in the `ast-executor`-gated `function`
        // module. We upgrade the weakref to a strong ref for the duration of the
        // check; if the context is already gone (never during a live session) the
        // checks stay inert rather than rejecting a possibly-valid value.
        #[cfg(feature = "ast-executor")]
        if let Some(ctx) = this
            .ctx_weakref
            .as_ref()
            .and_then(|wr| wr.bind(py).call0().ok())
            .filter(|obj| !obj.is_none())
            .map(|obj| obj.unbind())
        {
            let ctx = &ctx;
            use catnip_core::vm::opcode::ParamCheck;
            for (i, check) in this.field_checks.iter().enumerate() {
                if matches!(check, ParamCheck::None) {
                    continue;
                }
                let bound = final_values[i].bind(py);
                if !crate::core::function::field_value_ok_ast(py, bound, check, ctx) {
                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "field '{}' of '{}' expects '{}' but got '{}'",
                        this.field_names[i],
                        this.name,
                        catnip_core::vm::opcode::format_param_check(check),
                        crate::core::function::nominal_value_type_name_ast(py, bound)
                    )));
                }
            }
            for (i, check) in this.field_checks.iter().enumerate() {
                if let ParamCheck::Primitive(code) = check {
                    let coerced = crate::core::function::boundary_coerce_py(py, final_values[i].clone_ref(py), *code)?;
                    final_values[i] = coerced;
                }
            }
        }

        // Clone methods and type info while borrow is active
        let methods: IndexMap<String, Py<PyAny>> =
            this.methods.iter().map(|(k, v)| (k.clone(), v.clone_ref(py))).collect();
        let static_methods: IndexMap<String, Py<PyAny>> = this
            .static_methods
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let type_name = this.name.clone();
        let field_names = this.field_names.clone();
        drop(this); // release borrow before unbind

        let st_py: Py<CatnipStructType> = slf.clone().unbind();

        let proxy = Py::new(
            py,
            CatnipStructProxy {
                type_name,
                field_names,
                field_values: final_values,
                methods,
                static_methods,
                struct_type: Some(st_py),
                native_instance_idx: None, // AST mode, no VM index
                native_registry_id: 0,
                frozen: false,
            },
        )?;

        // init_fn is called by the registry (AST) or VM dispatch, not here
        Ok(proxy.into_any())
    }

    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        if let Some(func) = self.static_methods.get(name) {
            return Ok(func.clone_ref(py));
        }
        let candidates: Vec<&str> = self.static_methods.keys().map(|s| s.as_str()).collect();
        let suggestions = suggest_similar(name, &candidates, 1, 0.6);
        let msg = match format_suggestion(&suggestions) {
            Some(hint) => format!("<struct '{}'> has no attribute '{name}'. {hint}", self.name),
            None => format!("<struct '{}'> has no attribute '{name}'", self.name),
        };
        Err(pyo3::exceptions::PyAttributeError::new_err(msg))
    }

    fn __dir__(&self) -> Vec<String> {
        self.static_methods.keys().cloned().collect()
    }

    fn __repr__(&self) -> String {
        format!("<struct '{}'>", self.name)
    }

    /// Expose name as a Python attribute for introspection
    #[getter]
    fn __name__(&self) -> &str {
        &self.name
    }
}

/// Proxy for `super` keyword: provides access to parent methods bound to the current instance.
#[pyclass(module = "catnip._rs", name = "SuperProxy")]
pub struct SuperProxy {
    /// Parent methods (unbound VMFunction callables).
    pub methods: IndexMap<String, Py<PyAny>>,
    /// The current instance (to bind as self).
    pub instance: Py<PyAny>,
    /// Maps method name → source type name in the MRO (for cooperative super chain).
    pub method_sources: HashMap<String, String>,
    /// Native struct instance index (if available, for VM round-trip).
    pub native_instance_idx: Option<u32>,
    /// Identity of the registry owning `native_instance_idx` (0 if none).
    pub native_registry_id: u64,
}

#[pymethods]
impl SuperProxy {
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        match self.methods.get(name) {
            Some(func) => {
                let source = self.method_sources.get(name).cloned().unwrap_or_default();
                let bound = Py::new(
                    py,
                    crate::core::BoundCatnipMethod {
                        func: func.clone_ref(py),
                        instance: self.instance.clone_ref(py),
                        super_source_type: Some(source),
                        native_instance_idx: self.native_instance_idx,
                        native_registry_id: self.native_registry_id,
                    },
                )?;
                Ok(bound.into_any())
            }
            None => Err(pyo3::exceptions::PyAttributeError::new_err(format!(
                "super has no method '{name}'"
            ))),
        }
    }

    fn __repr__(&self) -> String {
        format!("<SuperProxy ({} methods)>", self.methods.len())
    }
}

/// Monotonic source of `StructRegistry` ids. 0 is reserved for "no registry"
/// so a proxy without a native origin (AST mode) never resolves through the
/// identity table.
static NEXT_REGISTRY_ID: AtomicU64 = AtomicU64::new(1);

/// Live `InstanceSlot` count across every registry (ledger probe, see
/// `value::debug_live_counts`). Incremented by [`InstanceSlot::new`] (the sole
/// constructor), decremented in `Drop` -- so free-list reclamation, registry
/// teardown and broadcast-child copies (live slots too, they retire with the
/// child) are all counted by construction. Registries die with their pipeline,
/// so a refcount leak shows up as an INTRA-session delta on a reused pipeline,
/// not at the teardown boundary.
pub(crate) static LIVE_STRUCT_INSTANCES: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Proxies materialized to Python per broadcast child registry, keyed by child id.
type MaterializedProxies = RefCell<HashMap<u64, Vec<(u32, Py<CatnipStructProxy>)>>>;

thread_local! {
    /// Proxies materialized to Python while a broadcast child registry was the
    /// active registry, keyed by the child's id. `transplant_to_parent` drains
    /// the entry to re-anchor survivors onto the parent; `StructRegistry::drop`
    /// clears any leftover (a child that errored before transplant). Thread-local
    /// because a child VM's whole lifecycle (clone -> run -> transplant) runs on
    /// one GIL-held thread, so recording and draining never cross threads.
    static MATERIALIZED_PROXIES: MaterializedProxies = RefCell::new(HashMap::new());
}

/// Record a proxy materialized in a broadcast child (keyed by the child id).
fn record_materialized_proxy(registry_id: u64, idx: u32, proxy: Py<CatnipStructProxy>) {
    // `try_with`: a registry can drop during thread teardown, after this
    // thread-local is destroyed (same guard as `unregister_struct_registry`).
    let _ = MATERIALIZED_PROXIES.try_with(|m| {
        m.borrow_mut().entry(registry_id).or_default().push((idx, proxy));
    });
}

/// Take and remove all proxies recorded for a registry id.
fn take_materialized_proxies(registry_id: u64) -> Vec<(u32, Py<CatnipStructProxy>)> {
    MATERIALIZED_PROXIES
        .try_with(|m| m.borrow_mut().remove(&registry_id).unwrap_or_default())
        .unwrap_or_default()
}

/// Registry for struct types and instances with refcount-based slot recycling.
pub struct StructRegistry {
    /// Stable identity used by Python proxies to reach this exact registry
    /// through the identity table, instead of the set-and-leave thread-local.
    id: u64,
    /// Root id of this registry's clone lineage. `clone_from_parent` copies the
    /// parent's instances at identical indices, so a clone (broadcast child) and
    /// its ancestor share an index space. A proxy's `native_instance_idx` is only
    /// valid in a registry of the same lineage; a foreign registry may hold an
    /// unrelated instance at that index. Equal for a fresh registry, inherited by
    /// clones.
    origin_id: u64,
    types: Vec<StructType>,
    /// Instance slots behind a `RefCell` so `decref` (and the whole
    /// `decref_discard`/cascade chain) take `&self`: a reentrant proxy decref (a
    /// pyobj field's `__del__` dropping another proxy of this registry) reaches
    /// the registry through `with_proxy_registry` as a SHARED borrow, never a
    /// second `&mut`. What was aliasing UB with `&mut` becomes a clean shared
    /// access -- or, if a borrow were ever held across a pyobj release, a loud
    /// `RefCell` panic instead of silent corruption. No borrow is held across a
    /// pyobj release: `decref` returns the freed fields and the caller releases
    /// them outside the borrow.
    instances: RefCell<Vec<Option<InstanceSlot>>>,
    free_list: RefCell<Vec<u32>>,
    /// True once this registry is a broadcast child (set by `clone_from_parent`).
    /// Only a child tracks the proxies materialized during its callback, so a
    /// top-level registry -- which never transplants -- records nothing. The
    /// proxies themselves live in the `MATERIALIZED_PROXIES` thread-local, keyed
    /// by registry id, so this `Sync` pyclass field stays a plain flag.
    is_broadcast_child: bool,
    /// Per-variant payload-field templates for generic unions, keyed by the
    /// variant's type_id (mirrors the PureVM `PureStructRegistry`). Only populated
    /// for union payload variant types; read at the `CheckGeneric` boundary to
    /// substitute use-site type arguments into payload contracts. Stored beside the
    /// type to avoid threading a field through every `StructType` construction.
    variant_templates: std::collections::HashMap<StructTypeId, Vec<catnip_core::vm::opcode::FieldTemplate>>,
}

// SAFETY: every access to the `RefCell` fields is serialized by the GIL. The
// registry is only ever touched while a Python thread state is attached: the
// `#[pyclass]` methods hold the GIL, and the rayon ND-broadcast workers
// (`host.rs`, `into_par_iter`) each re-acquire it via `Python::attach` before
// resolving or mutating a parent registry (the GIL-bound contract carried by
// `NdRegistryHandle`). So the non-atomic `RefCell` borrow flag is never touched
// concurrently -- accesses interleave but never overlap. The `Send + Sync` bound
// exists only to satisfy the `#[pyclass]` PyRustVM requirement (same pattern as
// `NativeClosureScope` in `frame.rs`). INVARIANT to preserve: never touch this
// registry off the GIL (e.g. inside a `py.detach` on a rayon worker) -- that
// would race the borrow flag.
//
// DEPENDS ON THE GIL BUILD: a rayon worker's element/result `Py<PyAny>` is
// dropped AFTER its `Python::attach` returns, i.e. off the GIL. If it is a
// `CatnipStructProxy`, its `Drop` decrefs a slot in this registry -- a borrow of
// the flag off the GIL. That is safe today only because PyO3 defers off-GIL `Py`
// drops to a GIL-held flush (its `ReferencePool`), so the decref actually runs
// under the GIL. This assumption breaks under `pyo3_disable_reference_pool` or a
// free-threaded / PEP 703 build (see `wip/MATATABI.md`), either of which would
// invalidate this whole argument and require the flag to become atomic (a real
// lock).
unsafe impl Send for StructRegistry {}
// SAFETY: same GIL-serialization invariant as the `Send` impl above.
unsafe impl Sync for StructRegistry {}

impl StructRegistry {
    /// Ledger probe companion: (idx, refcount, type name) of every live
    /// instance slot in THIS registry -- attributes a live-instance delta to
    /// concrete slots (the struct counterpart of `debug_live_slot_types`).
    pub fn debug_instance_slots(&self) -> Vec<(u32, u32, String)> {
        self.instances
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| {
                slot.as_ref().map(|s| {
                    let ty = self
                        .types
                        .get(s.instance.type_id as usize)
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| format!("<type {}>", s.instance.type_id));
                    (idx as u32, s.refcount.load(Ordering::Relaxed), ty)
                })
            })
            .collect()
    }

    /// Ledger probe: summed refcount of every live instance slot in THIS
    /// registry -- the struct counterpart of `OBJECT_TABLE`'s `refs`. Probe-time
    /// sum under a single borrow (no incref/decref cost), so a pure-rc pin on a
    /// persistent live instance (which does not move the slot COUNT
    /// `LIVE_STRUCT_INSTANCES`) surfaces as a non-zero delta of this sum.
    pub fn debug_instance_rc_sum(&self) -> u64 {
        self.instances
            .borrow()
            .iter()
            .flatten()
            .map(|s| s.refcount.load(Ordering::Relaxed) as u64)
            .sum()
    }

    pub fn new() -> Self {
        let id = NEXT_REGISTRY_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            origin_id: id,
            types: Vec::new(),
            instances: RefCell::new(Vec::new()),
            free_list: RefCell::new(Vec::new()),
            is_broadcast_child: false,
            variant_templates: std::collections::HashMap::new(),
        }
    }

    /// Record the payload-field templates for a union variant type (built at
    /// `MakeUnion`). Keyed by the variant's type_id.
    pub fn set_variant_templates(
        &mut self,
        type_id: StructTypeId,
        templates: Vec<catnip_core::vm::opcode::FieldTemplate>,
    ) {
        if !templates.is_empty() {
            self.variant_templates.insert(type_id, templates);
        }
    }

    /// The payload-field templates for a union variant type, if any.
    #[inline]
    pub fn variant_templates(&self, type_id: StructTypeId) -> Option<&[catnip_core::vm::opcode::FieldTemplate]> {
        self.variant_templates.get(&type_id).map(Vec::as_slice)
    }

    /// This registry's stable identity (see [`crate::vm::value::register_struct_registry`]).
    #[inline]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Root id of this registry's clone lineage (see field docs).
    #[inline]
    pub fn origin_id(&self) -> u64 {
        self.origin_id
    }

    /// Clone types and instances from a parent registry (for nested VM calls).
    pub fn clone_from_parent(&mut self, py: pyo3::Python<'_>, parent: &StructRegistry) {
        self.types = parent
            .types
            .iter()
            .map(|t| StructType {
                id: t.id,
                name: t.name.clone(),
                fields: t.fields.clone(),
                methods: t.methods.iter().map(|(k, v)| (k.clone(), v.clone_ref(py))).collect(),
                static_methods: t
                    .static_methods
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                    .collect(),
                implements: t.implements.clone(),
                mro: t.mro.clone(),
                parent_names: t.parent_names.clone(),
                abstract_methods: t.abstract_methods.clone(),
            })
            .collect();
        // Clone live instances preserving refcounts (child is an independent snapshot)
        let cloned_instances: Vec<Option<InstanceSlot>> = parent
            .instances
            .borrow()
            .iter()
            .map(|slot| {
                slot.as_ref().map(|s| {
                    // The bit-copied fields share their global counts
                    // (OBJECT_TABLE, heap Arcs) with the parent's slot: take
                    // one count per non-struct field so every child-side
                    // release consumes the child's own ref, never the
                    // parent's. Struct fields carry no independent count (it
                    // lives in the sibling slot, duplicated wholesale here).
                    for f in &s.instance.fields {
                        if !f.is_struct_instance() {
                            f.clone_refcount();
                        }
                    }
                    InstanceSlot::new(
                        s.instance.clone(),
                        s.refcount.load(Ordering::Relaxed),
                        s.frozen.load(Ordering::Relaxed),
                        true,
                    )
                })
            })
            .collect();
        *self.instances.borrow_mut() = cloned_instances;
        *self.free_list.borrow_mut() = parent.free_list.borrow().clone();
        // Generic-union variant templates travel with the shared type index space.
        self.variant_templates = parent.variant_templates.clone();
        // Inherit the lineage: this clone shares the parent's index space, so a
        // proxy from the parent resolves its idx here.
        self.origin_id = parent.origin_id;
        // This registry is now a broadcast child: track the proxies it
        // materializes so they can be re-anchored onto the parent at transplant.
        self.is_broadcast_child = true;
    }

    /// Transplant struct instances created in this (child) registry back into
    /// the parent registry. Copies both reused free-list slots (None in parent,
    /// Some in child) and appended slots (index >= parent len). Returns the
    /// indices freshly transplanted, so the caller can release the child's
    /// VM-internal refcount on the result (it is a phantom once the child dies;
    /// see `CatnipOwnershipProof`). A pass-through struct already present in the
    /// parent is not transplanted and so is absent from the returned list.
    pub fn transplant_to_parent(&self, py: pyo3::Python<'_>, parent: &StructRegistry) -> Vec<u32> {
        // Only a broadcast child transplants (frame.rs guards both directions
        // with the same saved_registry check). The Drop invariant below --
        // "a child's non-snapshot survivors are phantoms" -- relies on it.
        debug_assert!(self.is_broadcast_child);
        // `self` (child) and `parent` are always distinct registries, so their
        // borrows never conflict. `self` is only read; parent is mutated. If they
        // were ever the same object, holding `self.instances.borrow()` across
        // `parent.instances.borrow_mut()` below would panic -- assert it.
        debug_assert!(
            self.id != parent.id,
            "transplant_to_parent: self and parent must be distinct registries"
        );
        let mut transplanted = Vec::new();
        let self_instances = self.instances.borrow();
        let parent_count = parent.instances.borrow().len();
        // Reused free-list slots: index < parent_count, None in parent, Some in child
        for idx in 0..parent_count.min(self_instances.len()) {
            if parent.instances.borrow()[idx].is_none() {
                if let Some(slot) = &self_instances[idx] {
                    let new_slot = InstanceSlot::new(
                        slot.instance.clone(),
                        slot.refcount.load(Ordering::Relaxed),
                        slot.frozen.load(Ordering::Relaxed),
                        false,
                    );
                    parent.instances.borrow_mut()[idx] = Some(new_slot);
                    parent.free_list.borrow_mut().retain(|&i| i != idx as u32);
                    transplanted.push(idx as u32);
                }
            }
        }
        // Appended slots: index >= parent_count
        for idx in parent_count..self_instances.len() {
            if let Some(slot) = &self_instances[idx] {
                while parent.instances.borrow().len() <= idx {
                    // Intermediate holes (index < idx) stay None in the parent --
                    // mark them reclaimable so create_instance can reuse them. The
                    // slot at `idx` itself is filled just below, so it is excluded.
                    let hole = parent.instances.borrow().len() as u32;
                    parent.instances.borrow_mut().push(None);
                    if (hole as usize) < idx {
                        parent.free_list.borrow_mut().push(hole);
                    }
                }
                let new_slot = InstanceSlot::new(
                    slot.instance.clone(),
                    slot.refcount.load(Ordering::Relaxed),
                    slot.frozen.load(Ordering::Relaxed),
                    false,
                );
                parent.instances.borrow_mut()[idx] = Some(new_slot);
                transplanted.push(idx as u32);
            }
        }
        drop(self_instances);

        // Re-anchor proxies materialized during this child's callback onto the
        // parent. The transplant above copied each transplanted slot's refcount
        // -- including the ref held by a proxy materialized here -- into the
        // parent. A proxy stays bound to this child's id, which is unregistered
        // when the child dies, so its later Drop would no-op and leave the copied
        // ref as a phantom the parent never reclaims. Only transplanted slots are
        // re-anchored: a pass-through proxy's ref lived on the child slot (never
        // copied to the parent), so it must keep its child anchor and no-op.
        let recorded = take_materialized_proxies(self.id);
        if !recorded.is_empty() {
            let parent_id = parent.id();
            let parent_is_child = parent.is_broadcast_child;
            let mut reanchored = false;
            for (idx, proxy) in recorded {
                if transplanted.contains(&idx) {
                    proxy.bind(py).borrow_mut().native_registry_id = parent_id;
                    reanchored = true;
                    // Nested broadcast: the parent is itself a child, so it must
                    // re-anchor this proxy again when it transplants upward.
                    if parent_is_child {
                        record_materialized_proxy(parent_id, idx, proxy);
                    }
                }
                // else: pass-through (or already freed) -- drop the strong ref,
                // leaving the proxy bound to this child (no-op on child death).
            }
            // Make the parent reachable so a re-anchored proxy's deferred decref
            // resolves: the parent may not have materialized any proxy itself yet
            // (it registers lazily in instance_to_pyobject).
            if reanchored {
                crate::vm::value::register_struct_registry(parent_id, parent as *const _);
            }
        }
        transplanted
    }

    /// Register a new struct type. Returns its unique id.
    pub fn register_type(
        &mut self,
        name: String,
        fields: Vec<StructField>,
        methods: IndexMap<String, Py<PyAny>>,
        implements: Vec<String>,
        mro: Vec<String>,
    ) -> StructTypeId {
        self.register_type_with_parents(
            name,
            fields,
            StructMethods {
                instance: methods,
                statics: IndexMap::new(),
                abstract_methods: HashSet::new(),
            },
            StructParents {
                implements,
                mro,
                parent_names: Vec::new(),
            },
        )
    }

    /// Register a new struct type with parent info (for super). Returns its unique id.
    pub fn register_type_with_parents(
        &mut self,
        name: String,
        fields: Vec<StructField>,
        methods: StructMethods,
        parents: StructParents,
    ) -> StructTypeId {
        let id = self.types.len() as StructTypeId;
        self.types.push(StructType {
            id,
            name,
            fields,
            methods: methods.instance,
            static_methods: methods.statics,
            implements: parents.implements,
            mro: parents.mro,
            parent_names: parents.parent_names,
            abstract_methods: methods.abstract_methods,
        });
        id
    }

    /// Look up a type by id.
    pub fn get_type(&self, id: StructTypeId) -> Option<&StructType> {
        self.types.get(id as usize)
    }

    /// Look up a type by name. Returns the most recent definition (last registered).
    pub fn find_type_by_name(&self, name: &str) -> Option<&StructType> {
        // Search backwards to find the most recent definition
        self.types.iter().rev().find(|t| t.name == name)
    }

    /// All registered types, in registration order. Used by the ND process
    /// collector to ship every live struct type to a worker (freeze step C).
    pub fn iter_types(&self) -> impl Iterator<Item = &StructType> {
        self.types.iter()
    }

    /// Create an instance (refcount = 1). Reuses freed slots when available.
    pub fn create_instance(&self, type_id: StructTypeId, field_values: Vec<Value>) -> u32 {
        let slot = InstanceSlot::new(
            StructInstance {
                type_id,
                fields: field_values,
            },
            1,
            false,
            false,
        );
        let reused = self.free_list.borrow_mut().pop();
        if let Some(idx) = reused {
            self.instances.borrow_mut()[idx as usize] = Some(slot);
            idx
        } else {
            let mut instances = self.instances.borrow_mut();
            let idx = instances.len() as u32;
            instances.push(Some(slot));
            idx
        }
    }

    /// Deep-copy struct instance `src_idx` into a fresh instance for broadcast
    /// element isolation ((5,1)): recurse into nested struct fields so a callback
    /// mutation at ANY depth stays private to the copy; non-struct fields (lists,
    /// dicts, bigints) are shared by refcount, like the source shares them.
    /// Identity-preserving (a nested struct shared by two fields is copied once)
    /// and cycle-safe (the copy is registered before its fields are filled).
    /// Returns the new index owning one refcount (transferred to the caller).
    pub fn deep_snapshot(&self, src_idx: u32) -> u32 {
        let mut memo: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        self.deep_copy_instance(src_idx, &mut memo)
    }

    /// Recursive worker for [`deep_snapshot`]. `memo` maps a source index to its
    /// copy so shared and cyclic references resolve to a single copy.
    fn deep_copy_instance(&self, src_idx: u32, memo: &mut std::collections::HashMap<u32, u32>) -> u32 {
        if let Some(&new_idx) = memo.get(&src_idx) {
            // Already copied (shared/cyclic): take a ref for the field slot.
            self.incref(new_idx);
            return new_idx;
        }
        let (type_id, n) = self
            .with_instance(src_idx, |inst| (inst.type_id, inst.fields.len()))
            .expect("deep_copy_instance: dead source handle");
        // Phase 1: allocate with NIL placeholders and register BEFORE filling,
        // so a self/back reference during the fill resolves to this copy
        // (cycle-safe). NIL is not refcounted, so the fill below needs no decref.
        let new_idx = self.create_instance(type_id, vec![Value::NIL; n]);
        memo.insert(src_idx, new_idx);
        // Phase 2: fill -- recurse into struct fields, share the rest by refcount.
        for i in 0..n {
            let f = self
                .with_instance(src_idx, |inst| inst.fields[i])
                .expect("deep_copy_instance: source vanished mid-copy");
            let copied = if f.is_struct_instance() {
                let ci = self.deep_copy_instance(f.as_struct_instance_idx().unwrap(), memo);
                Value::from_struct_instance(ci)
            } else {
                f.clone_refcount();
                f
            };
            self.with_instance_mut(new_idx, |inst| inst.fields[i] = copied);
        }
        // Preserve the frozen (hashed) flag: a hashed struct rejects mutation to
        // keep hash/eq stable, and the AST/child-VM copies preserve it -- the
        // callback must not be able to mutate a frozen element's copy (that would
        // diverge from AST/PureVM, which reject it).
        if self.is_frozen(src_idx) {
            self.freeze(new_idx);
        }
        new_idx
    }

    /// Increment the handle refcount (Value was duplicated).
    /// Takes &self (not &mut) thanks to AtomicU32, avoiding aliasing with thread-local access.
    #[inline]
    pub fn incref(&self, idx: u32) {
        let instances = self.instances.borrow();
        let slot = instances[idx as usize]
            .as_ref()
            .expect("StructRegistry: dead handle on incref");
        slot.refcount.fetch_add(1, Ordering::Relaxed);
    }

    /// Mark an instance as frozen (hashed) so subsequent SetAttr must reject it.
    /// Takes &self thanks to AtomicBool.
    #[inline]
    pub fn freeze(&self, idx: u32) {
        if let Some(Some(slot)) = self.instances.borrow().get(idx as usize) {
            slot.frozen.store(true, Ordering::Relaxed);
        }
    }

    /// True if the instance has been hashed and must reject field mutation.
    #[inline]
    pub fn is_frozen(&self, idx: u32) -> bool {
        self.instances
            .borrow()
            .get(idx as usize)
            .and_then(|s| s.as_ref())
            .map(|s| s.frozen.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    /// Decrement the handle refcount. Returns the instance fields if freed (for
    /// cascade cleanup). Takes `&self`: the `RefCell` borrow for the free step
    /// is released before this returns, so the caller's cascade releases pyobj
    /// fields with NO registry borrow held -- a reentrant proxy decref then takes
    /// a fresh shared borrow instead of aliasing a `&mut`.
    #[inline]
    pub fn decref(&self, idx: u32) -> Option<Vec<Value>> {
        let rc = {
            let instances = self.instances.borrow();
            let slot = instances[idx as usize]
                .as_ref()
                .expect("StructRegistry: dead handle on decref");
            slot.refcount.fetch_sub(1, Ordering::Relaxed) - 1
        };
        if rc == 0 {
            // mem::take, not a field move: InstanceSlot has a Drop (ledger).
            let mut freed = self.instances.borrow_mut()[idx as usize].take().unwrap();
            self.free_list.borrow_mut().push(idx);
            Some(std::mem::take(&mut freed.instance.fields))
        } else {
            None
        }
    }

    /// Run `f` against instance `idx`'s data under a shared borrow (the `RefCell`
    /// replacement for the old `get_instance` that returned `&StructInstance`).
    /// The borrow cannot escape `f`, so it is never held across a reentrant point.
    #[inline]
    pub fn with_instance<R>(&self, idx: u32, f: impl FnOnce(&StructInstance) -> R) -> Option<R> {
        self.instances
            .borrow()
            .get(idx as usize)
            .and_then(|s| s.as_ref())
            .map(|s| f(&s.instance))
    }

    /// Mutable counterpart of [`with_instance`]. The caller MUST NOT release a
    /// displaced field value inside `f` (that runs a pyobj `__del__` under the
    /// `borrow_mut` and would panic on reentry) -- return the old value and
    /// release it after this call.
    #[inline]
    pub fn with_instance_mut<R>(&self, idx: u32, f: impl FnOnce(&mut StructInstance) -> R) -> Option<R> {
        self.instances
            .borrow_mut()
            .get_mut(idx as usize)
            .and_then(|s| s.as_mut())
            .map(|s| f(&mut s.instance))
    }

    /// True if instance slot `idx` is live (replaces `get_instance(idx).is_some()`).
    #[inline]
    pub fn has_instance(&self, idx: u32) -> bool {
        self.instances.borrow().get(idx as usize).is_some_and(|s| s.is_some())
    }

    /// Number of live instance slots. Diagnostic for refcount leaks: once every
    /// owner of every struct is gone, this must return to its baseline.
    #[cfg(test)]
    pub fn live_count(&self) -> usize {
        self.instances.borrow().iter().filter(|s| s.is_some()).count()
    }

    /// Convert a native struct instance to a CatnipStructProxy Python object.
    /// Increfs the slot: the proxy owns a refcount, released by CatnipStructProxy::drop (via tp_dealloc).
    pub fn instance_to_pyobject(&self, py: Python<'_>, idx: u32) -> PyResult<Py<PyAny>> {
        // Copy the fields out under the borrow (Value is Copy -- bit-copies, no
        // refcount change), then DROP the borrow before `to_pyobject`: converting
        // a struct field re-enters `instance_to_pyobject` on this same registry,
        // which would panic on a nested `RefCell` borrow.
        let (type_id, fields) = self
            .with_instance(idx, |inst| (inst.type_id, inst.fields.clone()))
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err(format!("struct instance #{idx} not found")))?;
        let ty = self
            .get_type(type_id)
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err(format!("struct type #{type_id} not found")))?;

        let field_names: Vec<String> = ty.fields.iter().map(|f| f.name.clone()).collect();
        let field_values: Vec<Py<PyAny>> = fields.iter().map(|v| v.to_pyobject(py)).collect();
        let methods: IndexMap<String, Py<PyAny>> =
            ty.methods.iter().map(|(k, v)| (k.clone(), v.clone_ref(py))).collect();
        let static_methods: IndexMap<String, Py<PyAny>> = ty
            .static_methods
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let type_name = ty.name.clone();
        // inst/ty borrows ended (NLL) -- all data cloned above
        self.incref(idx);
        // Mirror the registry's frozen state on the proxy: if the slot was
        // hashed (in the VM or by a previous to_pyobject), every materialized
        // proxy must also reject __setattr__, including after the thread-local
        // registry is torn down at the end of execution.
        let frozen = self.is_frozen(idx);

        // Make this registry reachable by the proxy's deferred incref/decref,
        // which run after execution when the thread-local no longer points here.
        crate::vm::value::register_struct_registry(self.id, self as *const _);

        let proxy = Py::new(
            py,
            CatnipStructProxy {
                type_name,
                field_names,
                field_values,
                methods,
                static_methods,
                struct_type: None,
                native_instance_idx: Some(idx),
                native_registry_id: self.id,
                frozen,
            },
        )?;
        // In a broadcast child, remember this proxy so transplant can re-anchor
        // it onto the parent if its slot survives (otherwise its child anchor
        // dies with the child and its transplanted refcount becomes a phantom).
        if self.is_broadcast_child {
            record_materialized_proxy(self.id, idx, proxy.clone_ref(py));
        }
        Ok(proxy.into_any())
    }
}

impl Default for StructRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StructRegistry {
    fn drop(&mut self) {
        // Remove our identity-table entry so a surviving proxy resolves to
        // nothing (no-op) instead of dereferencing this freed registry.
        crate::vm::value::unregister_struct_registry(self.id);
        // Release any proxies still recorded for this id (a child that errored
        // before transplant). Done after unregister so a proxy dropped here
        // no-ops instead of re-entering this registry while it is being dropped.
        if self.is_broadcast_child {
            drop(take_materialized_proxies(self.id));
        }
        // Release the field counts this registry owns, in one pass over the
        // surviving slots. A slot alive here is pinned by counts that have no
        // release path once the VM is gone (root-frame local, host-map entry),
        // and its pyobj/bigint fields point into GLOBAL stores (OBJECT_TABLE,
        // heap Arcs) that outlive this registry -- dropping them raw leaks one
        // ref per field per session. Struct-tagged fields are skipped: the
        // sibling slot dies in this same pass and releasing it would
        // double-count. Done after unregister, so a Python-side dealloc
        // re-entering through a proxy no-ops instead of aliasing `self`.
        //
        // Two distinct ownerships feed the pass:
        // - a snapshot slot's fields carry the count taken at
        //   `clone_from_parent` -- the child's own share of the global count,
        //   released whatever the registry's state;
        // - otherwise the terminal backstop applies only to a registry that
        //   solely owns its slots' fields: a broadcast child's non-snapshot
        //   survivors are phantoms whose bit-copied fields belong to the
        //   parent's transplanted slots (`transplant_to_parent` only runs
        //   from a child, see its debug_assert) -- releasing them would
        //   double-free the parent's live refs.
        let backstop = !self.is_broadcast_child;
        // `get_mut()` bypasses the RefCell borrow flag: unlike every other
        // release path, a reentrant borrow during the `f.decref()` below would be
        // silent aliasing UB, not a loud double-borrow panic. What keeps it safe
        // is the unregister-first ordering above (a reentrant proxy drop no-ops)
        // plus the executor/broadcast-child clearing the `STRUCT_REGISTRY`
        // thread-local before these field drops -- both external to this fn. If
        // either regresses, this is where it bites.
        for slot in self.instances.get_mut().iter().flatten() {
            if slot.snapshot || backstop {
                for f in &slot.instance.fields {
                    if !f.is_struct_instance() {
                        f.decref();
                    }
                }
            }
        }
        // Field defaults are owned by their type (evaluated at the struct
        // definition) and have no other release path; shadowed (redefined)
        // types stay in the vec and are covered too. A broadcast child's
        // types are bit-copy clones whose default refs belong to the parent,
        // so only the terminal backstop releases them. Struct-tagged defaults
        // are skipped like instance fields: their slot dies in this same pass.
        if backstop {
            for ty in &self.types {
                for f in &ty.fields {
                    if f.has_default && !f.default.is_struct_instance() {
                        f.default.decref();
                    }
                }
            }
        }
    }
}

/// Decref a struct slot, cascading into its fields if it reaches zero. No-op if
/// the slot is already freed -- the proxy and transplant decref paths can run
/// after the slot's owner is gone (a proxy outliving its broadcast child), and
/// `StructRegistry::decref` panics on a dead handle.
///
/// Takes `&self`: the registry's mutable state lives behind a `RefCell`, so a
/// reentrant proxy decref (a pyobj field's `__del__` dropping another proxy of
/// this registry) resolves through `with_proxy_registry` as a shared borrow and
/// takes a FRESH `RefCell` borrow -- never a second `&mut` (the old aliasing UB).
pub fn decref_slot(registry: &StructRegistry, idx: u32) {
    if registry.has_instance(idx) {
        if let Some(fields) = registry.decref(idx) {
            cascade_decref_fields(registry, fields);
        }
    }
}

/// Cascade-decref fields of a freed struct instance.
/// Each field that is itself a struct gets decremented recursively.
/// Non-struct heap types (BigInt, PyObj) are decremented via Value::decref().
///
/// A pyobj field's `__del__` runs with NO registry `RefCell` borrow held
/// (`decref` released its borrow before returning the fields), so a reentrant
/// decref of this same registry takes a fresh borrow instead of panicking or
/// aliasing.
pub fn cascade_decref_fields(registry: &StructRegistry, fields: Vec<Value>) {
    for f in fields {
        if f.is_struct_instance() {
            let idx = f.as_struct_instance_idx().unwrap();
            if let Some(sub_fields) = registry.decref(idx) {
                cascade_decref_fields(registry, sub_fields);
            }
        } else {
            f.decref();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_type() {
        let mut reg = StructRegistry::new();
        let id = reg.register_type(
            "Point".into(),
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            IndexMap::new(),
            vec![],               // implements
            vec!["Point".into()], // mro
        );
        assert_eq!(id, 0);
        let ty = reg.get_type(id).unwrap();
        assert_eq!(ty.name, "Point");
        assert_eq!(ty.fields.len(), 2);
        assert_eq!(ty.fields[0].name, "x");
        assert_eq!(ty.fields[1].name, "y");
    }

    #[test]
    fn test_create_instance() {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "Vec2".into(),
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            IndexMap::new(),
            vec![],               // implements
            vec!["Point".into()], // mro
        );
        let idx = reg.create_instance(tid, vec![Value::from_int(10), Value::from_int(20)]);
        let (inst_type_id, fields) = reg.with_instance(idx, |i| (i.type_id, i.fields.clone())).unwrap();
        assert_eq!(inst_type_id, tid);
        assert_eq!(fields[0].as_int(), Some(10));
        assert_eq!(fields[1].as_int(), Some(20));
    }

    #[test]
    fn test_transplant_pads_holes_into_free_list() {
        // Regression: transplant_to_parent padded parent.instances with None for
        // index gaps but never marked them reusable, so create_instance could
        // never reclaim them -- a slow Vec leak for a long-lived registry.
        Python::attach(|py| {
            let mut parent = StructRegistry::new();
            let tid = parent.register_type(
                "P".into(),
                vec![StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                }],
                IndexMap::new(),
                vec![],
                vec!["P".into()],
            );

            // Child appends A, B, C then drops B, leaving a hole at index 1.
            let mut child = StructRegistry::new();
            child.clone_from_parent(py, &parent);
            let _a = child.create_instance(tid, vec![Value::from_int(0)]);
            let b = child.create_instance(tid, vec![Value::from_int(1)]);
            let _c = child.create_instance(tid, vec![Value::from_int(2)]);
            assert!(child.decref(b).is_some(), "B should reach zero");

            child.transplant_to_parent(py, &parent);

            // The hole the child left (index 1) must be reclaimable: the next
            // allocation reuses it instead of growing the Vec.
            let reused = parent.create_instance(tid, vec![Value::from_int(9)]);
            assert_eq!(reused, b, "transplant hole was not added to the free list");
        });
    }

    #[test]
    fn test_proxy_drop_frees_slot() {
        // Regression target for the cross-VM struct refcount leak: a proxy that
        // outlives its VM owner must release the registry slot when it dies.
        Python::attach(|py| {
            let mut reg = StructRegistry::new();
            let tid = reg.register_type(
                "P".into(),
                vec![StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                }],
                IndexMap::new(),
                vec![],
                vec!["P".into()],
            );
            let idx = reg.create_instance(tid, vec![Value::from_int(1)]); // rc=1 (VM owner)
            assert_eq!(reg.live_count(), 1);

            let proxy = reg.instance_to_pyobject(py, idx).unwrap(); // rc=2 (proxy incref)
            reg.decref(idx); // rc=1: the VM-internal owner is released (result consumed)
            assert_eq!(reg.live_count(), 1);

            drop(proxy); // proxy dies -> must decref -> rc=0 -> slot freed
            assert_eq!(reg.live_count(), 0, "proxy drop leaked the registry slot");
        });
    }

    #[test]
    fn reentrant_cascade_decref_takes_fresh_borrow() {
        // The whole point of the `RefCell` + `&self` refactor: a struct whose
        // pyobj field, when released by the VM-direct cascade, runs Python that
        // drops a proxy of the SAME registry -- re-entering `decref`. With the
        // old `&mut StructRegistry` this aliased the cascade's live `&mut`
        // (aliasing UB); now every path holds only a shared `&self` and the
        // interior `RefCell` is borrowed only in short, non-overlapping windows,
        // so the reentrant decref takes a fresh borrow. Must not panic; both the
        // outer struct and the inner (proxy-held) struct are reclaimed.
        Python::attach(|py| {
            let mut reg = StructRegistry::new();
            let tid = reg.register_type(
                "P".into(),
                vec![StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                }],
                IndexMap::new(),
                vec![],
                vec!["P".into()],
            );
            // Register so a proxy's deferred decref resolves back to this registry.
            crate::vm::value::register_struct_registry(reg.id(), &reg as *const _);

            // Inner instance B, handed out as a Python proxy; drop the VM-owner
            // ref so the proxy holds B's only refcount.
            let idx_b = reg.create_instance(tid, vec![Value::from_int(2)]);
            let proxy_b = reg.instance_to_pyobject(py, idx_b).unwrap(); // rc_b = 2
            reg.decref(idx_b); // rc_b = 1 (proxy_b)

            // Outer instance A whose field is a Python LIST holding B's proxy --
            // a genuine pyobj field (not re-homed to TAG_STRUCT), so its release
            // runs the list dealloc -> proxy drop -> reentrant decref of B.
            let pylist = pyo3::types::PyList::new(py, [proxy_b.bind(py)]).unwrap();
            let field = Value::from_pyobject(py, pylist.as_any()).unwrap();
            drop(proxy_b); // the list now holds B's only proxy
            drop(pylist); // `field` is now the sole owner of the list
            let idx_a = reg.create_instance(tid, vec![field]);
            assert_eq!(reg.live_count(), 2, "A and B both live");

            // Free A: cascade releases the list field -> reentrant decref of B.
            decref_slot(&reg, idx_a);
            assert_eq!(reg.live_count(), 0, "reentrant cascade must reclaim both A and B");
        });
    }

    #[test]
    fn registry_drop_releases_surviving_slot_heap_fields() {
        // Terminal backstop oracle: a slot still live at registry death (its
        // counts held by owners with no release path once the VM is gone --
        // root-frame local, host-map entry) must not drop its heap fields raw.
        // Before the backstop, `p = P(w)` leaked one ref of `w` per session.
        use rug::Integer;
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "P".into(),
            vec![StructField {
                name: "x".into(),
                has_default: false,
                default: Value::NIL,
                check: catnip_core::vm::opcode::ParamCheck::None,
            }],
            IndexMap::new(),
            vec![],
            vec!["P".into()],
        );
        let witness = Value::from_bigint(Integer::from(1) << 80);
        witness.clone_refcount(); // the field's owned ref (strong count 2)
        let idx = reg.create_instance(tid, vec![witness]);
        reg.incref(idx); // a second owner that never releases (the leak shape)
        let _ = idx;
        assert_eq!(witness.bigint_strong_count(), 2);

        // Registry dies with the slot live (rc=2): the backstop must release
        // the field's ref, leaving only ours.
        drop(reg);
        assert_eq!(
            witness.bigint_strong_count(),
            1,
            "registry death dropped a surviving slot's heap field raw"
        );
        witness.decref_bigint();
    }

    /// Registry with one pinned instance whose single field is `witness`
    /// (which keeps one caller-owned count on top of the field's own).
    fn parent_with_pinned_witness_field(witness: Value) -> StructRegistry {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "P".into(),
            vec![StructField {
                name: "x".into(),
                has_default: false,
                default: Value::NIL,
                check: catnip_core::vm::opcode::ParamCheck::None,
            }],
            IndexMap::new(),
            vec![],
            vec!["P".into()],
        );
        witness.clone_refcount(); // the field's owned ref
        let idx = reg.create_instance(tid, vec![witness]);
        reg.incref(idx); // a second owner that never releases (the leak shape)
        reg
    }

    #[test]
    fn snapshot_setattr_releases_child_own_field_count() {
        // Under-count SetAttr cross-VM: a broadcast child's slots are
        // bit-copies of the parent's, their heap fields sharing global counts.
        // The snapshot takes its own count per field at clone_from_parent, so
        // the SetAttr overwrite release consumes the child's ref and the
        // parent's field stays live.
        use rug::Integer;
        Python::attach(|py| {
            let witness = Value::from_bigint(Integer::from(1) << 80);
            let parent = parent_with_pinned_witness_field(witness);
            assert_eq!(witness.bigint_strong_count(), 2); // field + ours

            let mut child = StructRegistry::new();
            child.clone_from_parent(py, &parent);
            assert_eq!(witness.bigint_strong_count(), 3, "snapshot must own its bit-copy");

            // Child-side SetAttr: overwrite the field, releasing the old value
            // (the dispatch's decref_discard) -- must consume the child's count.
            let old = child
                .with_instance_mut(0, |inst| {
                    let old = inst.fields[0];
                    inst.fields[0] = Value::from_int(1);
                    old
                })
                .unwrap();
            old.decref();
            assert_eq!(
                witness.bigint_strong_count(),
                2,
                "child SetAttr released the parent's shared count"
            );

            drop(child); // the snapshot's count was consumed by the overwrite
            assert_eq!(witness.bigint_strong_count(), 2);

            // Parent death: the backstop -- no longer disarmed by having
            // spawned a child -- releases the surviving field.
            drop(parent);
            assert_eq!(
                witness.bigint_strong_count(),
                1,
                "parent backstop skipped after broadcast (at-death leak)"
            );
            witness.decref_bigint();
        });
    }

    #[test]
    fn snapshot_child_death_balances_untouched_copies() {
        // A child that never touches its snapshot releases at death the counts
        // taken at clone_from_parent; the parent's own count is released by
        // its backstop. Net session balance: zero.
        use rug::Integer;
        Python::attach(|py| {
            let witness = Value::from_bigint(Integer::from(1) << 80);
            let parent = parent_with_pinned_witness_field(witness);
            let mut child = StructRegistry::new();
            child.clone_from_parent(py, &parent);
            assert_eq!(witness.bigint_strong_count(), 3);

            drop(child);
            assert_eq!(
                witness.bigint_strong_count(),
                2,
                "child death leaked its snapshot count"
            );
            drop(parent);
            assert_eq!(
                witness.bigint_strong_count(),
                1,
                "parent backstop skipped after broadcast"
            );
            witness.decref_bigint();
        });
    }

    #[test]
    fn snapshot_cascade_release_consumes_child_count() {
        // Child-side decref-to-zero (e.g. a shared global reassigned inside the
        // callback): the field cascade consumes the snapshot's own count, and
        // the parent's copy stays live until its backstop.
        use rug::Integer;
        Python::attach(|py| {
            let witness = Value::from_bigint(Integer::from(1) << 80);
            let parent = parent_with_pinned_witness_field(witness);
            let mut child = StructRegistry::new();
            child.clone_from_parent(py, &parent);
            assert_eq!(witness.bigint_strong_count(), 3);

            // Drop both child-side counts (creation + pin): cascade fires.
            decref_slot(&child, 0);
            decref_slot(&child, 0);
            assert!(!child.has_instance(0), "slot should be freed in the child");
            assert_eq!(
                witness.bigint_strong_count(),
                2,
                "child cascade released the parent's shared count"
            );
            assert!(parent.has_instance(0), "parent copy untouched");

            drop(child);
            assert_eq!(witness.bigint_strong_count(), 2);
            drop(parent);
            assert_eq!(witness.bigint_strong_count(), 1);
            witness.decref_bigint();
        });
    }

    #[test]
    fn closure_drop_leaves_captured_struct_slot_for_registry_drain() {
        // Teardown gate for the captured-struct branch (wip/GLOBALS_OWNERSHIP.md,
        // "le drain propre du NativeClosureScope ... reste à câbler").
        //
        // A closure's captured map owns one ref per entry. `ClosureScopeInner::drop`
        // releases pyobj/bigint/complex via `Value::decref`, but that call is a
        // no-op on a `TAG_STRUCT`: freeing a struct instance needs the registry,
        // which the Drop cannot reach (the thread-local registry pointer is unsound
        // at teardown). So a struct that lands raw in a captured map -- the
        // reachable path being a mutable-closure write through `set_with_py`,
        // which, unlike `MakeFunction`, does NOT portabilize -- is not reclaimed by
        // the Drop alone: its slot survives until a registry-aware decref runs.
        //
        // This oracle pins that contract precisely: the plain Drop leaves the slot
        // live, and the registry-aware decref (what the func_table-slot-release
        // drain of TODO item "Lifecycle PureFunctionTable" will wire) reclaims it.
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "P".into(),
            vec![StructField {
                name: "x".into(),
                has_default: false,
                default: Value::NIL,
                check: catnip_core::vm::opcode::ParamCheck::None,
            }],
            IndexMap::new(),
            vec![],
            vec!["P".into()],
        );
        // The struct's single ref is owned-in by the captured map (the transfer a
        // `set()` performs), so no incref: the map holds the only reference.
        let idx = reg.create_instance(tid, vec![Value::from_int(1)]);
        assert_eq!(reg.live_count(), 1);

        let mut captured = IndexMap::new();
        captured.insert("s".to_string(), Value::from_struct_instance(idx));
        let scope = crate::vm::frame::NativeClosureScope::without_parent(captured);

        // The Drop skips the struct (no registry here): its slot is NOT reclaimed.
        drop(scope);
        assert_eq!(
            reg.live_count(),
            1,
            "ClosureScopeInner::drop must not (and cannot) free a captured struct slot"
        );

        // The registry-aware decref the drain will own reclaims the leaked slot.
        assert!(reg.decref(idx).is_some(), "registry decref should free the slot");
        assert_eq!(
            reg.live_count(),
            0,
            "registry-aware drain reclaims the captured struct slot"
        );
    }

    #[test]
    fn test_transplant_result_no_phantom() {
        // Models a broadcast child returning a struct: the child creates it,
        // transplant moves it to the parent, the result is materialized as a
        // proxy on the parent, then the child and the proxy both die. The slot
        // must return to the parent's free list -- no phantom from the child's
        // transplanted VM-internal refcount.
        Python::attach(|py| {
            let mut parent = StructRegistry::new();
            let tid = parent.register_type(
                "P".into(),
                vec![StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                }],
                IndexMap::new(),
                vec![],
                vec!["P".into()],
            );
            let mut child = StructRegistry::new();
            child.clone_from_parent(py, &parent);
            let idx = child.create_instance(tid, vec![Value::from_int(1)]); // child rc=1 (result VM ref)
            let transplanted = child.transplant_to_parent(py, &parent); // parent gets the slot
            assert!(
                transplanted.contains(&idx),
                "result slot should be reported as transplanted"
            );

            let proxy = parent.instance_to_pyobject(py, idx).unwrap(); // parent incref (proxy ref)
            // Transfer the result's ownership to the proxy: release the child's
            // VM-internal ref that transplant copied into the parent, so the slot
            // is owned solely by the proxy. Mirrors the result boundary in frame.rs.
            decref_slot(&parent, idx);
            drop(child); // child VM ends; its VM-internal ref dies with it
            drop(proxy); // proxy dies -> decref -> rc=0 -> slot freed
            assert_eq!(parent.live_count(), 0, "broadcast transplant left a phantom slot");
        });
    }

    #[test]
    fn test_transplant_escaped_proxy_no_phantom() {
        // A proxy materialized DURING a broadcast callback anchors to the child
        // registry. If it escapes to Python (e.g. appended to a list) and the
        // struct is freshly transplanted, the child's refcount -- including the
        // proxy's ref -- is copied into the parent. After the child dies the
        // proxy's `native_registry_id` names a dead registry, so its Drop no-ops
        // and the copied ref becomes a phantom the parent never reclaims.
        Python::attach(|py| {
            let mut parent = StructRegistry::new();
            let tid = parent.register_type(
                "P".into(),
                vec![StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                }],
                IndexMap::new(),
                vec![],
                vec!["P".into()],
            );
            let mut child = StructRegistry::new();
            child.clone_from_parent(py, &parent);

            // Callback body: create a struct and materialize an escaping proxy.
            let idx = child.create_instance(tid, vec![Value::from_int(1)]); // child rc=1 (VM local)
            let escaped = child.instance_to_pyobject(py, idx).unwrap(); // child rc=2 (proxy, anchored to child)
            child.decref(idx); // callback frame torn down: VM local released -> child rc=1 (proxy only)

            let transplanted = child.transplant_to_parent(py, &parent);
            assert!(transplanted.contains(&idx), "escaped struct should be transplanted");

            drop(child); // child VM ends -> child registry id unregistered
            drop(escaped); // proxy drops -> must release the parent slot
            assert_eq!(parent.live_count(), 0, "escaped-proxy transplant left a phantom slot");
        });
    }

    #[test]
    fn test_transplant_passthrough_proxy_no_overdecref() {
        // The dangerous mirror of the phantom fix: a proxy materialized in a
        // child for a PASS-THROUGH struct (one that already existed in the
        // parent) must NOT be re-anchored. Its incref lived on the child slot
        // and never reached the parent, so re-anchoring it would over-decref the
        // parent slot -- a use-after-free, strictly worse than the phantom.
        Python::attach(|py| {
            let mut parent = StructRegistry::new();
            let tid = parent.register_type(
                "P".into(),
                vec![StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                }],
                IndexMap::new(),
                vec![],
                vec!["P".into()],
            );
            let idx = parent.create_instance(tid, vec![Value::from_int(1)]); // parent rc=1 (parent owns it)

            let mut child = StructRegistry::new();
            child.clone_from_parent(py, &parent); // child has idx too (pass-through)

            // Callback materializes an escaping proxy for the pass-through struct.
            let escaped = child.instance_to_pyobject(py, idx).unwrap(); // child rc=2, anchored to child

            let transplanted = child.transplant_to_parent(py, &parent);
            assert!(
                !transplanted.contains(&idx),
                "pass-through slot must not be transplanted"
            );

            drop(child); // child dies: the proxy's child-side incref dies with it
            drop(escaped); // proxy drops -> must NO-OP (child id dead), never touch the parent
            assert_eq!(
                parent.live_count(),
                1,
                "pass-through proxy over-decref'd the parent slot"
            );

            // The parent still owns the slot and releases it normally.
            decref_slot(&parent, idx);
            assert_eq!(parent.live_count(), 0, "parent could not release its own slot");
        });
    }

    #[test]
    fn test_retarget_releases_old_sibling_slot() {
        // Cross-VM leak: a proxy created by registry A is handed to a sibling
        // registry B (separate VM, unrelated lineage). B does not share A's index
        // space, so from_pyobject takes the orphan path -- it re-creates the struct
        // in B and retargets the proxy. The reference the proxy held on A's slot
        // must be released, or A keeps the slot pinned for its whole life.
        Python::attach(|py| {
            let fields = vec![StructField {
                name: "x".into(),
                has_default: false,
                default: Value::NIL,
                check: catnip_core::vm::opcode::ParamCheck::None,
            }];

            // Registry A: create a struct, export a proxy, drop the VM-internal
            // ref so the proxy is the sole owner (rc=1).
            let mut reg_a = StructRegistry::new();
            let tid_a = reg_a.register_type("P".into(), fields.clone(), IndexMap::new(), vec![], vec!["P".into()]);
            let idx_a = reg_a.create_instance(tid_a, vec![Value::from_int(1)]); // rc=1 (VM owner)
            let proxy = reg_a.instance_to_pyobject(py, idx_a).unwrap(); // rc=2 (proxy incref)
            reg_a.decref(idx_a); // rc=1: VM owner released, proxy holds the last ref
            assert_eq!(reg_a.live_count(), 1);

            // Registry B: a live sibling with the same type but an unrelated
            // lineage (origin_id differs from A's id, forcing the orphan path).
            let mut reg_b = StructRegistry::new();
            reg_b.register_type("P".into(), fields, IndexMap::new(), vec![], vec!["P".into()]);
            assert_ne!(reg_b.origin_id(), reg_a.id(), "sibling must not share A's lineage");

            // Round-trip the proxy through B, as the VM does when the proxy is
            // handed to another execution: orphan path re-creates and retargets.
            crate::vm::value::set_struct_registry(&reg_b as *const _);
            crate::vm::value::register_struct_registry(reg_b.id(), &reg_b as *const _);
            let v = Value::from_pyobject(py, proxy.bind(py)).unwrap();
            let new_idx = v.as_struct_instance_idx().unwrap();
            crate::vm::value::clear_struct_registry();

            // A's slot must be freed now; the proxy was retargeted onto B.
            assert_eq!(reg_a.live_count(), 0, "retarget leaked the sibling A slot");
            assert_eq!(reg_b.live_count(), 1, "B should own the re-created struct");

            // Cleanup: release B's two refs (the from_pyobject result + the proxy).
            decref_slot(&reg_b, new_idx);
            drop(proxy); // proxy (now anchored to B) releases its ref
            assert_eq!(reg_b.live_count(), 0, "B slot leaked after teardown");
        });
    }

    #[test]
    fn test_field_access() {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "Pair".into(),
            vec![
                StructField {
                    name: "a".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "b".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            IndexMap::new(),
            vec![],              // implements
            vec!["Pair".into()], // mro
        );
        let idx = reg.create_instance(tid, vec![Value::from_int(1), Value::from_int(2)]);

        // read
        assert_eq!(reg.with_instance(idx, |i| i.fields[0]).unwrap().as_int(), Some(1));

        // write
        reg.with_instance_mut(idx, |i| i.fields[1] = Value::from_int(99))
            .unwrap();
        assert_eq!(reg.with_instance(idx, |i| i.fields[1]).unwrap().as_int(), Some(99));
    }

    #[test]
    fn test_multiple_types() {
        let mut reg = StructRegistry::new();
        let id0 = reg.register_type(
            "A".into(),
            vec![StructField {
                name: "x".into(),
                has_default: false,
                default: Value::NIL,
                check: catnip_core::vm::opcode::ParamCheck::None,
            }],
            IndexMap::new(),
            vec![],           // implements
            vec!["A".into()], // mro
        );
        let id1 = reg.register_type(
            "B".into(),
            vec![StructField {
                name: "y".into(),
                has_default: false,
                default: Value::NIL,
                check: catnip_core::vm::opcode::ParamCheck::None,
            }],
            IndexMap::new(),
            vec![],           // implements
            vec!["B".into()], // mro
        );
        assert_ne!(id0, id1);
        assert_eq!(reg.get_type(id0).unwrap().name, "A");
        assert_eq!(reg.get_type(id1).unwrap().name, "B");
    }

    #[test]
    fn test_default_fields() {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "Config".into(),
            vec![
                StructField {
                    name: "name".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "debug".into(),
                    has_default: true,
                    default: Value::FALSE,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "level".into(),
                    has_default: true,
                    default: Value::from_int(1),
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            IndexMap::new(),
            vec![],                // implements
            vec!["Config".into()], // mro
        );
        let ty = reg.get_type(tid).unwrap();
        assert!(!ty.fields[0].has_default);
        assert!(ty.fields[1].has_default);
        assert_eq!(ty.fields[1].default.as_bool(), Some(false));
        assert!(ty.fields[2].has_default);
        assert_eq!(ty.fields[2].default.as_int(), Some(1));
    }

    #[test]
    fn test_field_index() {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "RGB".into(),
            vec![
                StructField {
                    name: "r".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "g".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "b".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            IndexMap::new(),
            vec![],             // implements
            vec!["RGB".into()], // mro
        );
        let ty = reg.get_type(tid).unwrap();
        assert_eq!(ty.field_index("r"), Some(0));
        assert_eq!(ty.field_index("g"), Some(1));
        assert_eq!(ty.field_index("b"), Some(2));
        assert_eq!(ty.field_index("a"), None);
    }

    // --- Refcount + free-list tests ---

    fn make_point_type(reg: &mut StructRegistry) -> StructTypeId {
        reg.register_type(
            "Point".into(),
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            IndexMap::new(),
            vec![],
            vec!["Point".into()],
        )
    }

    #[test]
    fn test_create_and_free() {
        let mut reg = StructRegistry::new();
        let tid = make_point_type(&mut reg);
        let idx = reg.create_instance(tid, vec![Value::from_int(1), Value::from_int(2)]);

        // Instance is alive
        assert!(reg.has_instance(idx));

        // Decref -> freed, returns fields
        let fields = reg.decref(idx);
        assert!(fields.is_some());
        let fields = fields.unwrap();
        assert_eq!(fields[0].as_int(), Some(1));
        assert_eq!(fields[1].as_int(), Some(2));

        // Slot is now dead
        assert!(!reg.has_instance(idx));
        assert_eq!(*reg.free_list.borrow(), vec![idx]);
    }

    #[test]
    fn test_incref_decref() {
        let mut reg = StructRegistry::new();
        let tid = make_point_type(&mut reg);
        let idx = reg.create_instance(tid, vec![Value::from_int(10), Value::from_int(20)]);

        // Incref (rc=2)
        reg.incref(idx);

        // First decref (rc=1) -> still alive
        assert!(reg.decref(idx).is_none());
        assert!(reg.has_instance(idx));

        // Second decref (rc=0) -> freed
        assert!(reg.decref(idx).is_some());
        assert!(!reg.has_instance(idx));
    }

    #[test]
    fn test_free_list_reuse() {
        let mut reg = StructRegistry::new();
        let tid = make_point_type(&mut reg);

        let idx0 = reg.create_instance(tid, vec![Value::from_int(0), Value::from_int(0)]);
        let idx1 = reg.create_instance(tid, vec![Value::from_int(1), Value::from_int(1)]);
        let idx2 = reg.create_instance(tid, vec![Value::from_int(2), Value::from_int(2)]);
        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);

        // Free the middle slot
        reg.decref(idx1);
        assert!(!reg.has_instance(idx1));

        // New instance reuses slot 1
        let idx3 = reg.create_instance(tid, vec![Value::from_int(3), Value::from_int(3)]);
        assert_eq!(idx3, idx1);
        assert_eq!(reg.with_instance(idx3, |i| i.fields[0]).unwrap().as_int(), Some(3));

        // Slots 0 and 2 are still alive
        assert_eq!(reg.with_instance(idx0, |i| i.fields[0]).unwrap().as_int(), Some(0));
        assert_eq!(reg.with_instance(idx2, |i| i.fields[0]).unwrap().as_int(), Some(2));
    }

    #[test]
    fn test_cascade_decref() {
        let mut reg = StructRegistry::new();
        let tid = make_point_type(&mut reg);

        // Create inner struct
        let inner_idx = reg.create_instance(tid, vec![Value::from_int(1), Value::from_int(2)]);
        let inner_val = Value::from_struct_instance(inner_idx);

        // Create outer struct holding inner as a field
        let outer_idx = reg.create_instance(tid, vec![inner_val, Value::from_int(0)]);

        // Decref outer -> fields returned for cascade
        let fields = reg.decref(outer_idx).unwrap();
        assert!(!reg.has_instance(outer_idx));

        // Inner is still alive (refcount not yet touched)
        assert!(reg.has_instance(inner_idx));

        // Cascade the fields
        cascade_decref_fields(&reg, fields);

        // Now inner is freed too
        assert!(!reg.has_instance(inner_idx));
    }
}
