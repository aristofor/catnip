// FILE: catnip_rs/src/vm/structs.rs
//! Native struct types for the Catnip VM.
//!
//! Provides O(1) field access by position instead of Python getattr.

use super::value::Value;
use catnip_tools::suggest::{format_suggestion, suggest_similar};
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use std::collections::{HashMap, HashSet};

pub type StructTypeId = u32;

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
}

/// A struct type registered in the VM.
pub struct StructType {
    pub id: StructTypeId,
    pub name: String,
    pub fields: Vec<StructField>,
    /// Raw VMFunction callables keyed by method name.
    pub methods: HashMap<String, Py<PyAny>>,
    /// Static methods callable on type and instances (no self binding).
    pub static_methods: HashMap<String, Py<PyAny>>,
    /// List of trait names this struct implements.
    pub implements: Vec<String>,
    /// Method resolution order (MRO) — includes struct parents and traits.
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
            .field(
                "static_methods",
                &format!("<{} static>", self.static_methods.len()),
            )
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

/// Python-visible proxy for struct instances returned by to_pyobject.
#[pyclass(module = "catnip._rs", name = "CatnipStruct")]
pub struct CatnipStructProxy {
    pub type_name: String,
    pub field_names: Vec<String>,
    pub field_values: Vec<Py<PyAny>>,
    /// Raw VMFunction callables (not yet bound).
    pub methods: HashMap<String, Py<PyAny>>,
    /// Static methods (no self binding).
    pub static_methods: HashMap<String, Py<PyAny>>,
    /// Back-reference to the CatnipStructType (AST mode). None for VM-created proxies.
    pub struct_type: Option<Py<CatnipStructType>>,
    /// VM struct instance index for round-trip through Python collections.
    /// Allows from_pyobject to restore TAG_STRUCT instead of falling back to TAG_PYOBJ.
    pub native_instance_idx: Option<u32>,
}

#[pymethods]
impl CatnipStructProxy {
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        // Check fields first
        for (i, fname) in self.field_names.iter().enumerate() {
            if fname == name {
                return Ok(self.field_values[i].clone_ref(py));
            }
        }
        // Then methods - create BoundCatnipMethod on the fly
        if let Some(func) = self.methods.get(name) {
            let instance: Py<PyAny> = Py::new(
                py,
                CatnipStructProxy {
                    type_name: self.type_name.clone(),
                    field_names: self.field_names.clone(),
                    field_values: self.field_values.iter().map(|v| v.clone_ref(py)).collect(),
                    methods: self
                        .methods
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                        .collect(),
                    static_methods: self
                        .static_methods
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                        .collect(),
                    struct_type: self.struct_type.as_ref().map(|st| st.clone_ref(py)),
                    native_instance_idx: self.native_instance_idx,
                },
            )?
            .into_any();
            let bound = Py::new(
                py,
                crate::core::BoundCatnipMethod {
                    func: func.clone_ref(py),
                    instance,
                    super_source_type: None,
                    native_instance_idx: None,
                },
            )?;
            return Ok(bound.into_any());
        }
        // Then static methods - return raw callable (no self binding)
        if let Some(func) = self.static_methods.get(name) {
            return Ok(func.clone_ref(py));
        }
        // Try static methods from the type (AST mode)
        if let Some(ref st) = self.struct_type {
            let st_ref = st.bind(py).borrow();
            if let Some(func) = st_ref.static_methods.get(name) {
                return Ok(func.clone_ref(py));
            }
        }
        let mut candidates: Vec<&str> = self.field_names.iter().map(|s| s.as_str()).collect();
        candidates.extend(self.methods.keys().map(|s| s.as_str()));
        candidates.extend(self.static_methods.keys().map(|s| s.as_str()));
        let suggestions = suggest_similar(name, &candidates, 1, 0.6);
        let msg = match format_suggestion(&suggestions) {
            Some(hint) => format!("'{}' has no attribute '{name}'. {hint}", self.type_name),
            None => format!("'{}' has no attribute '{name}'", self.type_name),
        };
        Err(pyo3::exceptions::PyAttributeError::new_err(msg))
    }

    fn __setattr__(&mut self, name: &str, value: Py<PyAny>) -> PyResult<()> {
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

    fn __str__(&self, py: Python<'_>) -> String {
        self.__repr__(py)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let fields: Vec<String> = self
            .field_names
            .iter()
            .zip(&self.field_values)
            .map(|(n, v)| {
                let repr = v
                    .bind(py)
                    .repr()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|_| "?".into());
                format!("{n}={repr}")
            })
            .collect();
        format!("{}({})", self.type_name, fields.join(", "))
    }

    fn __richcmp__(
        slf: Bound<'_, Self>,
        other: Bound<'_, PyAny>,
        op: pyo3::pyclass::CompareOp,
    ) -> PyResult<Py<PyAny>> {
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
            Ok(val) if val.bind(py).is(&py.NotImplemented()) => {
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
    /// The struct stays as `self` (first arg), the foreign value as `rhs`.
    /// For commutative ops (+, *) this is semantically correct.
    /// For non-commutative ops (-, /) the user's method must handle the swap.
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
            let args = PyTuple::new(py, [slf.as_any(), other.as_any()])?;
            func.call(py, &args, None)
        } else {
            Ok(py.NotImplemented())
        }
    }

    /// Dispatch a unary operator to the corresponding op_* method.
    /// Raises TypeError if the method doesn't exist.
    fn dispatch_unaryop(
        slf: &Bound<'_, Self>,
        py: Python<'_>,
        op_name: &str,
    ) -> PyResult<Py<PyAny>> {
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
    fn structural_eq(
        slf: &Bound<'_, Self>,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        if let Ok(other_struct) = other.cast::<CatnipStructProxy>() {
            let self_ref = slf.borrow();
            let other_ref = other_struct.borrow();
            if self_ref.type_name != other_ref.type_name {
                return Ok(false
                    .into_pyobject(py)
                    .unwrap()
                    .to_owned()
                    .into_any()
                    .unbind());
            }
            if self_ref.field_values.len() != other_ref.field_values.len() {
                return Ok(false
                    .into_pyobject(py)
                    .unwrap()
                    .to_owned()
                    .into_any()
                    .unbind());
            }
            for (a, b) in self_ref.field_values.iter().zip(&other_ref.field_values) {
                if !a.bind(py).eq(b.bind(py))? {
                    return Ok(false
                        .into_pyobject(py)
                        .unwrap()
                        .to_owned()
                        .into_any()
                        .unbind());
                }
            }
            Ok(true
                .into_pyobject(py)
                .unwrap()
                .to_owned()
                .into_any()
                .unbind())
        } else {
            Ok(false
                .into_pyobject(py)
                .unwrap()
                .to_owned()
                .into_any()
                .unbind())
        }
    }

    /// Structural inequality: negation of structural_eq.
    fn structural_ne(
        slf: &Bound<'_, Self>,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let eq_result = Self::structural_eq(slf, py, other)?;
        let is_eq: bool = eq_result.extract(py)?;
        Ok((!is_eq)
            .into_pyobject(py)
            .unwrap()
            .to_owned()
            .into_any()
            .unbind())
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
    /// Method callables keyed by name (raw VMFunction/RustLambda)
    pub methods: HashMap<String, Py<PyAny>>,
    /// Static methods (no self binding, callable on type and instances).
    pub static_methods: HashMap<String, Py<PyAny>>,
    /// Post-constructor init function (if defined)
    pub init_fn: Option<Py<PyAny>>,
    /// Direct parent struct names (from extends).
    pub parent_names: Vec<String>,
    /// Method resolution order (struct parents).
    pub mro: Vec<String>,
    /// Abstract methods that remain unimplemented.
    pub abstract_methods: HashSet<MethodKey>,
}

#[pymethods]
impl CatnipStructType {
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
            let names: Vec<&str> = this
                .abstract_methods
                .iter()
                .map(|k| k.name.as_str())
                .collect();
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
        let mut field_values: Vec<Option<Py<PyAny>>> = (0..n_fields).map(|_| None).collect();

        // Apply positional args
        for i in 0..n_args {
            field_values[i] = Some(args.get_item(i)?.unbind());
        }

        // Apply kwargs
        if let Some(kw) = kwargs {
            for (key, value) in kw.iter() {
                let key_str: String = key.extract()?;
                let idx = this
                    .field_names
                    .iter()
                    .position(|n| n == &key_str)
                    .ok_or_else(|| {
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
        for i in 0..n_fields {
            if field_values[i].is_none() {
                if let Some(ref default) = this.field_defaults[i] {
                    field_values[i] = Some(default.clone_ref(py));
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

        // Clone methods and type info while borrow is active
        let methods: HashMap<String, Py<PyAny>> = this
            .methods
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let static_methods: HashMap<String, Py<PyAny>> = this
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
            },
        )?;

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
    pub methods: HashMap<String, Py<PyAny>>,
    /// The current instance (to bind as self).
    pub instance: Py<PyAny>,
    /// Maps method name → source type name in the MRO (for cooperative super chain).
    pub method_sources: HashMap<String, String>,
    /// Native struct instance index (if available, for VM round-trip).
    pub native_instance_idx: Option<u32>,
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

/// Append-only registry for struct types and instances.
pub struct StructRegistry {
    types: Vec<StructType>,
    instances: Vec<StructInstance>,
}

impl StructRegistry {
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            instances: Vec::new(),
        }
    }

    /// Register a new struct type. Returns its unique id.
    pub fn register_type(
        &mut self,
        name: String,
        fields: Vec<StructField>,
        methods: HashMap<String, Py<PyAny>>,
        implements: Vec<String>,
        mro: Vec<String>,
    ) -> StructTypeId {
        self.register_type_with_parents(
            name,
            fields,
            methods,
            implements,
            mro,
            Vec::new(),
            HashSet::new(),
            HashMap::new(),
        )
    }

    /// Register a new struct type with parent info (for super). Returns its unique id.
    pub fn register_type_with_parents(
        &mut self,
        name: String,
        fields: Vec<StructField>,
        methods: HashMap<String, Py<PyAny>>,
        implements: Vec<String>,
        mro: Vec<String>,
        parent_names: Vec<String>,
        abstract_methods: HashSet<MethodKey>,
        static_methods: HashMap<String, Py<PyAny>>,
    ) -> StructTypeId {
        let id = self.types.len() as StructTypeId;
        self.types.push(StructType {
            id,
            name,
            fields,
            methods,
            static_methods,
            implements,
            mro,
            parent_names,
            abstract_methods,
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

    /// Create an instance. Returns its index in the instance pool.
    pub fn create_instance(&mut self, type_id: StructTypeId, field_values: Vec<Value>) -> u32 {
        let idx = self.instances.len() as u32;
        self.instances.push(StructInstance {
            type_id,
            fields: field_values,
        });
        idx
    }

    pub fn get_instance(&self, idx: u32) -> Option<&StructInstance> {
        self.instances.get(idx as usize)
    }

    pub fn get_instance_mut(&mut self, idx: u32) -> Option<&mut StructInstance> {
        self.instances.get_mut(idx as usize)
    }

    /// Convert a native struct instance to a CatnipStructProxy Python object.
    pub fn instance_to_pyobject(&self, py: Python<'_>, idx: u32) -> PyResult<Py<PyAny>> {
        let inst = self.get_instance(idx).ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(format!("struct instance #{idx} not found"))
        })?;
        let ty = self.get_type(inst.type_id).ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(format!(
                "struct type #{} not found",
                inst.type_id
            ))
        })?;

        let field_names: Vec<String> = ty.fields.iter().map(|f| f.name.clone()).collect();
        let field_values: Vec<Py<PyAny>> = inst.fields.iter().map(|v| v.to_pyobject(py)).collect();
        let methods: HashMap<String, Py<PyAny>> = ty
            .methods
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let static_methods: HashMap<String, Py<PyAny>> = ty
            .static_methods
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();

        let proxy = Py::new(
            py,
            CatnipStructProxy {
                type_name: ty.name.clone(),
                field_names,
                field_values,
                methods,
                static_methods,
                struct_type: None,
                native_instance_idx: Some(idx),
            },
        )?;
        Ok(proxy.into_any())
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
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
            HashMap::new(),
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
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
            HashMap::new(),
            vec![],               // implements
            vec!["Point".into()], // mro
        );
        let idx = reg.create_instance(tid, vec![Value::from_int(10), Value::from_int(20)]);
        let inst = reg.get_instance(idx).unwrap();
        assert_eq!(inst.type_id, tid);
        assert_eq!(inst.fields[0].as_int(), Some(10));
        assert_eq!(inst.fields[1].as_int(), Some(20));
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
                },
                StructField {
                    name: "b".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
            HashMap::new(),
            vec![],              // implements
            vec!["Pair".into()], // mro
        );
        let idx = reg.create_instance(tid, vec![Value::from_int(1), Value::from_int(2)]);

        // read
        assert_eq!(reg.get_instance(idx).unwrap().fields[0].as_int(), Some(1));

        // write
        reg.get_instance_mut(idx).unwrap().fields[1] = Value::from_int(99);
        assert_eq!(reg.get_instance(idx).unwrap().fields[1].as_int(), Some(99));
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
            }],
            HashMap::new(),
            vec![],           // implements
            vec!["A".into()], // mro
        );
        let id1 = reg.register_type(
            "B".into(),
            vec![StructField {
                name: "y".into(),
                has_default: false,
                default: Value::NIL,
            }],
            HashMap::new(),
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
                },
                StructField {
                    name: "debug".into(),
                    has_default: true,
                    default: Value::FALSE,
                },
                StructField {
                    name: "level".into(),
                    has_default: true,
                    default: Value::from_int(1),
                },
            ],
            HashMap::new(),
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
                },
                StructField {
                    name: "g".into(),
                    has_default: false,
                    default: Value::NIL,
                },
                StructField {
                    name: "b".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
            HashMap::new(),
            vec![],             // implements
            vec!["RGB".into()], // mro
        );
        let ty = reg.get_type(tid).unwrap();
        assert_eq!(ty.field_index("r"), Some(0));
        assert_eq!(ty.field_index("g"), Some(1));
        assert_eq!(ty.field_index("b"), Some(2));
        assert_eq!(ty.field_index("a"), None);
    }
}
