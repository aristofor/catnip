// FILE: catnip_rs/src/vm/host.rs
//! Host abstraction layer for the VM dispatch loop.
//!
//! Decouples the dispatch loop from direct PyO3 calls by routing
//! host operations (global resolution, binary operator fallback,
//! registry access) through a trait. The concrete `ContextHost` impl
//! delegates to the helpers in `py_interop.rs`.

use super::frame::{ClosureParent, Globals, NativeClosureScope, VMFunction};
use super::py_interop::{
    PyResultExt, call_binary_op, delete_ctx_global, lookup_ctx_global, resolve_registry, store_ctx_global,
};
use super::value::Value;
use crate::constants::{PY_MOD_MEMOIZATION, PY_MOD_RS};
use indexmap::IndexMap;
use pyo3::PyTraverseError;
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rayon::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// Thread-local ND nesting depth counter.
// Guards against nested `~~()` calls (e.g. `~~(0, (v, r) => { ~~(v, ...) })`).
// Recursive depth via `recur()` is tracked by NDScheduler.submit_recursive.
thread_local! {
    static ND_NESTING_DEPTH: Cell<usize> = const { Cell::new(0) };
}

// Thread-local globals snapshot for VM re-entry.
// When the VM calls apply_broadcast, struct method dispatch goes through
// Python __add__ → VMFunction.__call__ which creates a fresh VM.
// This thread-local lets the child VM inherit the parent's globals.
thread_local! {
    static VM_GLOBALS: RefCell<Option<Globals>> = const { RefCell::new(None) };
}

/// Set the thread-local VM globals (called before broadcast dispatch).
pub fn set_vm_globals(globals: Globals) {
    VM_GLOBALS.with(|g| {
        *g.borrow_mut() = Some(globals);
    });
}

/// Take the thread-local VM globals (called by VMFunction.__call__).
pub fn take_vm_globals() -> Option<Globals> {
    VM_GLOBALS.with(|g| g.borrow_mut().take())
}

/// Clear the thread-local VM globals (after execution completes).
pub fn clear_vm_globals() {
    VM_GLOBALS.with(|g| {
        *g.borrow_mut() = None;
    });
}

/// RAII guard that decrements the thread-local ND nesting counter on drop.
struct NdNestingGuard;

impl NdNestingGuard {
    /// Increment the counter and check the limit. Returns Ok(guard) or Err.
    fn enter() -> Result<Self, super::core::VMError> {
        ND_NESTING_DEPTH.with(|d| {
            let depth = d.get();
            if depth >= crate::constants::ND_MAX_RECURSION_DEPTH {
                return Err(super::core::VMError::RuntimeError(
                    "maximum ND recursion depth exceeded".to_string(),
                ));
            }
            d.set(depth + 1);
            Ok(Self)
        })
    }
}

impl Drop for NdNestingGuard {
    fn drop(&mut self) {
        ND_NESTING_DEPTH.with(|d| d.set(d.get() - 1));
    }
}

/// Borrowed snapshot of the parent VM's four per-thread registries, handed to
/// ND `Thread` rayon workers in place of smuggling raw pointers across
/// `py.detach` as `usize`.
///
/// The pointee types are fixed in the fields (no untyped `usize -> *const T`
/// reconstruction at each worker), the `Send`/`Sync` justification lives once on
/// the `unsafe impl` below, and [`NdRegistryHandle::install`] returns an RAII
/// guard that restores each worker thread-local on drop. Carrying the four
/// registries plus the nullary-union-method bindings -- not just
/// `func_table`/`struct_registry` -- lets a worker lambda that returns an
/// enum/union variant resolve its symbol AND keep its methods, instead of
/// demoting to a raw index (`symbol_table`/`enum_registry`/the method table were
/// previously absent in workers).
struct NdRegistryHandle {
    func_table: *const super::value::FunctionTable,
    struct_registry: *const super::structs::StructRegistry,
    symbol_table: *mut catnip_core::symbols::SymbolTable,
    enum_registry: *mut super::enums::EnumRegistry,
    /// Nullary-union-method bindings, carried BY VALUE (a `Send` `Arc` handle)
    /// rather than aliased: unlike the four registries (raw pointers to state
    /// owned by the parent's stack frame), this table is owned by its
    /// thread-local, so a worker cannot reach the parent's by pointer. The `Arc`
    /// is installed (one atomic incref) into each worker's thread-local so a
    /// returned variant keeps its methods.
    union_nullary_methods: super::value::UnionNullaryMethodTable,
}

// SAFETY: the four pointer fields alias registries owned by the parent thread's
// stack frame, which stays live for the whole `py.detach` scope (the frame
// holding the `~~`/broadcast call does not return until the parallel map
// completes). They are dereferenced ONLY under `Python::attach`, i.e. with the
// GIL held, which serializes all workers: at most one touches the shared
// registries at a time, and `transplant_to_parent` mutates the parent registry
// only under that serialization while the parent VM is parked in `py.detach`
// touching nothing. The fifth field, `union_nullary_methods`, is NOT an alias --
// it is an owned value (a clone), `Send + Sync` on its own (every `Py` is
// `Send + Sync`), independent of the GIL argument. This is the single locus of
// the GIL-bound soundness argument the per-site `usize` casts used to spread out.
//
// INVARIANT to preserve: no worker may keep a registry reference live ACROSS a
// GIL release. A nested `py.detach` (e.g. nested ND) is fine in itself -- it
// parks the thread between accesses and each access re-acquires the GIL; the
// hazard is holding a `&`/`&mut` into a shared registry while the GIL is
// released, which would let a sibling worker mutate it concurrently.
unsafe impl Send for NdRegistryHandle {}
// SAFETY: shared access from multiple workers is sound for the same reason as
// `Send` above -- every dereference happens under the GIL, which serializes the
// workers; see the `unsafe impl Send` comment for the full contract.
unsafe impl Sync for NdRegistryHandle {}

impl NdRegistryHandle {
    /// Capture the registries currently installed on the parent thread (and a
    /// clone of its nullary-union-method bindings).
    fn capture() -> Self {
        Self {
            func_table: super::value::save_func_table(),
            struct_registry: super::value::save_struct_registry(),
            symbol_table: super::value::save_symbol_table(),
            enum_registry: super::value::save_enum_registry(),
            union_nullary_methods: super::value::snapshot_union_nullary_methods(),
        }
    }

    /// Install the captured registries into the current (worker) thread's
    /// thread-locals, returning a guard that restores the prior values on drop.
    ///
    /// Restoring -- rather than clearing -- matters because rayon may run a work
    /// item on the parent thread itself: there the prior values ARE the parent's
    /// live registries, which must survive after `py.detach` returns. A null
    /// field is skipped so a missing parent registry never clobbers the worker's.
    fn install(&self) -> NdRegistryGuard {
        let prev = Self::capture();
        if !self.func_table.is_null() {
            super::value::set_func_table(self.func_table);
        }
        if !self.struct_registry.is_null() {
            super::value::set_struct_registry(self.struct_registry);
        }
        if !self.symbol_table.is_null() {
            super::value::set_symbol_table(self.symbol_table);
        }
        if !self.enum_registry.is_null() {
            super::value::set_enum_registry(self.enum_registry);
        }
        super::value::set_union_nullary_methods(self.union_nullary_methods.clone());
        NdRegistryGuard { prev }
    }
}

/// RAII guard from [`NdRegistryHandle::install`]; restores the worker
/// thread-local registries to their pre-install values on drop.
struct NdRegistryGuard {
    prev: NdRegistryHandle,
}

impl Drop for NdRegistryGuard {
    fn drop(&mut self) {
        super::value::restore_func_table(self.prev.func_table);
        super::value::restore_struct_registry(self.prev.struct_registry);
        super::value::restore_symbol_table(self.prev.symbol_table);
        super::value::restore_enum_registry(self.prev.enum_registry);
        super::value::set_union_nullary_methods(self.prev.union_nullary_methods.clone());
    }
}

// ---------------------------------------------------------------------------
// ND parallelism configuration
// ---------------------------------------------------------------------------

/// ND broadcast execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NdMode {
    /// Process elements one by one (default).
    Sequential,
    /// Distribute elements across rayon threads.
    Thread,
    /// Distribute elements across worker processes (ProcessPoolExecutor).
    Process,
}

/// ND broadcast configuration.
#[derive(Debug, Clone)]
pub struct NdConfig {
    pub mode: NdMode,
    pub memoize: bool,
}

impl Default for NdConfig {
    fn default() -> Self {
        Self {
            mode: NdMode::Sequential,
            memoize: true,
        }
    }
}

/// Binary operators delegated to the host (Python `operator` module).
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    TrueDiv,
    FloorDiv,
    Mod,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Host operations required by the VM dispatch loop.
///
/// Abstracts Python-side interactions so the dispatch loop doesn't
/// call PyO3 directly for these categories. `py: Python<'_>` stays
/// in signatures because Value conversions still need the GIL (Phase 3).
pub trait VmHost {
    // --- Global resolution (ctx_globals) ---

    fn lookup_global(&self, py: Python<'_>, name: &str) -> Result<Option<Value>, super::core::VMError>;

    /// Store a global. Returns the DISPLACED entry when the host keeps `Value`
    /// entries (VMHost map): it is owned and the caller must release it with
    /// the registry in hand (`decref_discard`) -- a plain `Value::decref` is a
    /// no-op on TAG_STRUCT and used to leak one registry count per overwrite.
    /// Hosts backed by Python dicts (ContextHost) return `None` (CPython owns
    /// the displaced object's refs).
    fn store_global(&self, py: Python<'_>, name: &str, value: Value) -> Result<Option<Value>, super::core::VMError>;

    /// Delete a global. Same displaced-entry contract as `store_global`.
    fn delete_global(&self, py: Python<'_>, name: &str) -> Result<Option<Value>, super::core::VMError>;

    // --- Binary operator fallback (PyObject operands) ---

    fn binary_op(&self, py: Python<'_>, op: BinaryOp, a: Value, b: Value) -> Result<Value, super::core::VMError>;

    // --- Registry access (ND/broadcast) ---

    fn resolve_registry<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyAny>, super::core::VMError>;

    // --- Iteration ---

    /// Call the host's `iter()` builtin on a Python object.
    fn get_iter<'py>(
        &self,
        py: Python<'py>,
        obj: &Bound<'py, PyAny>,
    ) -> Result<Bound<'py, PyAny>, super::core::VMError>;

    // --- Global existence check ---

    /// Check if a name exists in ctx_globals (without fetching its value).
    fn has_global(&self, py: Python<'_>, name: &str) -> bool;

    // --- Closure parent construction ---

    /// Build the closure parent for a function/struct/trait method.
    /// Uses native chain if available, else falls back to PyGlobals.
    fn build_closure_parent(&self, py: Python<'_>, frame_closure: Option<&NativeClosureScope>) -> ClosureParent;

    // --- Attribute/item access (Python fallback) ---
    // Default bodies: both hosts dispatch through the same Python protocols;
    // only `contains_op` needs host state (the cached `operator.contains`).

    /// The host's cached `operator.contains` callable.
    fn cached_contains(&self) -> &Py<PyAny>;

    /// Get an attribute from a Python object. Includes "did you mean?" in error.
    fn obj_getattr(&self, py: Python<'_>, obj: Value, name: &str) -> Result<Value, super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_bound = py_obj.bind(py);
        if let Some(v) = native_plugin_getattr(py, py_bound, name)? {
            return Ok(v);
        }
        let result = py_bound.getattr(name).map_err(|e| {
            let msg = e.to_string();
            super::core::VMError::RuntimeError(super::core::py_attr_error_msg(py_bound, name, &msg))
        })?;
        Value::from_pyobject(py, &result).to_vm(py)
    }

    /// Set an attribute on a Python object.
    fn obj_setattr(&self, py: Python<'_>, obj: Value, name: &str, value: Value) -> Result<(), super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_value = value.to_pyobject(py);
        py_obj
            .bind(py)
            .setattr(name, py_value)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    /// Get an item from a collection (obj[key]).
    fn obj_getitem(&self, py: Python<'_>, obj: Value, key: Value) -> Result<Value, super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_key = key.to_pyobject(py);
        let result = py_obj
            .bind(py)
            .get_item(py_key)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        Value::from_pyobject(py, &result).to_vm(py)
    }

    /// Set an item in a collection (obj[key] = value).
    fn obj_setitem(&self, py: Python<'_>, obj: Value, key: Value, value: Value) -> Result<(), super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_key = key.to_pyobject(py);
        let py_value = value.to_pyobject(py);
        py_obj
            .bind(py)
            .set_item(py_key, py_value)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    // --- Membership test ---

    /// Test `item in container` via `operator.contains(container, item)`.
    fn contains_op(&self, py: Python<'_>, item: Value, container: Value) -> Result<Value, super::core::VMError> {
        let container_obj = container.to_pyobject(py);
        let item_obj = item.to_pyobject(py);
        // operator.contains(container, item) == (item in container)
        let result = self
            .cached_contains()
            .bind(py)
            .call1((container_obj, item_obj))
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        let is_true = result
            .is_truthy()
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        Ok(Value::from_bool(is_true))
    }

    // --- Context access (transition) ---

    /// Borrow the Python context. Needed for pass_context calls,
    /// and pattern matching delegation.
    /// Will be removed when these operations get their own host methods.
    fn context(&self) -> &Option<Py<PyAny>>;

    // --- Introspection ---

    /// Merge host-level globals (builtins, modules) into target dict.
    fn collect_globals(&self, py: Python<'_>, target: &Bound<'_, PyDict>) -> Result<(), super::core::VMError>;

    // --- Broadcast / ND operations ---

    /// Apply broadcast operation on target with operator.
    fn apply_broadcast(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        operator: &Bound<'_, PyAny>,
        operand: Option<&Bound<'_, PyAny>>,
        is_filter: bool,
    ) -> Result<Py<PyAny>, super::core::VMError>;

    /// Execute ND-recursion: `seed ~~ lambda`.
    fn execute_nd_recursion(
        &self,
        py: Python<'_>,
        seed: &Bound<'_, PyAny>,
        lambda: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError>;

    /// Execute ND-map: `data ~> func`.
    fn execute_nd_map(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        func: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError>;

    /// Broadcast ND-recursion over iterable: `data.[~~ lambda]`.
    /// Default impl: sequential iteration calling `execute_nd_recursion` per element.
    fn broadcast_nd_recursion(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        lambda: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
        let result_list = pyo3::types::PyList::empty(py);
        for elem_result in target.try_iter().to_vm(py)? {
            let elem = elem_result.to_vm(py)?;
            let nd_result = self.execute_nd_recursion(py, &elem, lambda)?;
            result_list
                .append(nd_result)
                .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        }
        into_list_or_tuple(py, result_list, is_tuple)
    }

    /// Broadcast ND-map over iterable: `data.[~> func]`.
    /// Default impl: sequential iteration calling `execute_nd_map` per element.
    fn broadcast_nd_map(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        func: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
        let result_list = pyo3::types::PyList::empty(py);
        for elem_result in target.try_iter().to_vm(py)? {
            let elem = elem_result.to_vm(py)?;
            let nd_result = self.execute_nd_map(py, &elem, func)?;
            result_list
                .append(nd_result)
                .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        }
        into_list_or_tuple(py, result_list, is_tuple)
    }
}

/// Concrete host backed by Python context and cached operator refs.
pub struct ContextHost {
    ctx_globals: Option<Py<PyDict>>,
    py_context: Option<Py<PyAny>>,
    /// Operator refs indexed by `BinaryOp as usize`.
    ops: [Py<PyAny>; 11],
    /// Cached iter() builtin.
    iter_fn: Py<PyAny>,
    /// Cached operator.contains for `in` / `not in`.
    cached_contains: Py<PyAny>,
}

impl ContextHost {
    /// Build from the VM's cached refs.
    ///
    /// Called once at the start of `run()`. All refs are cloned from
    /// the VM's cached fields (which are populated by `ensure_builtins_cached`).
    pub fn new(py: Python<'_>, vm: &super::core::VM) -> Self {
        let ctx_globals: Option<Py<PyDict>> = vm.py_context().as_ref().and_then(|ctx| {
            ctx.bind(py)
                .getattr("globals")
                .ok()
                .and_then(|g| match g.cast::<PyDict>() {
                    Ok(d) => Some(d.clone().unbind()),
                    Err(_) => None,
                })
        });

        let ops = [
            vm.cached_op(CachedOp::Add).clone_ref(py),
            vm.cached_op(CachedOp::Sub).clone_ref(py),
            vm.cached_op(CachedOp::Mul).clone_ref(py),
            vm.cached_op(CachedOp::TrueDiv).clone_ref(py),
            vm.cached_op(CachedOp::FloorDiv).clone_ref(py),
            vm.cached_op(CachedOp::Mod).clone_ref(py),
            vm.cached_op(CachedOp::Pow).clone_ref(py),
            vm.cached_op(CachedOp::Lt).clone_ref(py),
            vm.cached_op(CachedOp::Le).clone_ref(py),
            vm.cached_op(CachedOp::Gt).clone_ref(py),
            vm.cached_op(CachedOp::Ge).clone_ref(py),
        ];

        let iter_fn = vm.cached_iter_fn().clone_ref(py);
        let cached_contains = vm.cached_contains_fn().clone_ref(py);

        Self {
            ctx_globals,
            py_context: vm.py_context().as_ref().map(|c| c.clone_ref(py)),
            ops,
            iter_fn,
            cached_contains,
        }
    }
}

/// If `py_bound` is a native plugin object, route attribute access to the
/// plugin's getattr callback (GIL released during the FFI call) and return the
/// converted value. Returns `Ok(None)` for any other object so the caller can
/// fall back to the normal Python attribute protocol.
///
/// Shared by both `VmHost` implementations to mirror `catnip_vm::host`.
pub(crate) fn native_plugin_getattr(
    py: Python<'_>,
    py_bound: &Bound<'_, PyAny>,
    name: &str,
) -> Result<Option<Value>, super::core::VMError> {
    let Ok(po) = py_bound.cast::<crate::loader::native_plugin::NativePluginObject>() else {
        return Ok(None);
    };
    let (handle, cbs) = po
        .borrow()
        .handle_and_callbacks()
        .ok_or_else(|| super::core::VMError::RuntimeError("invalid plugin object".into()))?;
    let getattr_fn = cbs
        .getattr
        .ok_or_else(|| super::core::VMError::RuntimeError(format!("plugin object has no attribute '{name}'")))?;
    let name_owned = name.to_string();
    let bits = py
        .detach(move || {
            catnip_vm::plugin::call_plugin_getattr(handle, getattr_fn, &name_owned, &cbs)
                .map(|v| v.bits())
                .map_err(|e| e.to_string())
        })
        .map_err(super::core::VMError::RuntimeError)?;
    let pyres = crate::vm::py_interop::vm_value_to_py(py, catnip_vm::Value::from_raw(bits)).to_vm(py)?;
    Ok(Some(Value::from_pyobject(py, pyres.bind(py)).to_vm(py)?))
}

impl VmHost for ContextHost {
    #[inline]
    fn lookup_global(&self, py: Python<'_>, name: &str) -> Result<Option<Value>, super::core::VMError> {
        match self.ctx_globals {
            Some(ref g) => lookup_ctx_global(py, g, name),
            None => Ok(None),
        }
    }

    #[inline]
    fn store_global(&self, py: Python<'_>, name: &str, value: Value) -> Result<Option<Value>, super::core::VMError> {
        if let Some(ref g) = self.ctx_globals {
            store_ctx_global(py, g, name, value)?;
        }
        Ok(None)
    }

    #[inline]
    fn delete_global(&self, py: Python<'_>, name: &str) -> Result<Option<Value>, super::core::VMError> {
        if let Some(ref g) = self.ctx_globals {
            delete_ctx_global(py, g, name)?;
        }
        Ok(None)
    }

    #[inline]
    fn binary_op(&self, py: Python<'_>, op: BinaryOp, a: Value, b: Value) -> Result<Value, super::core::VMError> {
        call_binary_op(py, &self.ops[op as usize], a, b)
    }

    #[inline]
    fn resolve_registry<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyAny>, super::core::VMError> {
        resolve_registry(py, &self.py_context)
    }

    #[inline]
    fn get_iter<'py>(
        &self,
        py: Python<'py>,
        obj: &Bound<'py, PyAny>,
    ) -> Result<Bound<'py, PyAny>, super::core::VMError> {
        self.iter_fn
            .bind(py)
            .call1((obj,))
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    #[inline]
    fn has_global(&self, py: Python<'_>, name: &str) -> bool {
        match self.ctx_globals {
            Some(ref g) => g.bind(py).contains(name).unwrap_or(false),
            None => false,
        }
    }

    #[inline]
    fn build_closure_parent(&self, py: Python<'_>, frame_closure: Option<&NativeClosureScope>) -> ClosureParent {
        match frame_closure {
            Some(parent_closure) => ClosureParent::Native(parent_closure.clone()),
            None => match self.ctx_globals {
                Some(ref g) => ClosureParent::PyGlobals(g.clone_ref(py)),
                None => ClosureParent::None,
            },
        }
    }

    #[inline]
    fn cached_contains(&self) -> &Py<PyAny> {
        &self.cached_contains
    }

    #[inline]
    fn context(&self) -> &Option<Py<PyAny>> {
        &self.py_context
    }

    fn collect_globals(&self, py: Python<'_>, target: &Bound<'_, PyDict>) -> Result<(), super::core::VMError> {
        if let Some(ref g) = self.ctx_globals {
            target
                .update(g.bind(py).as_mapping())
                .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        }
        Ok(())
    }

    fn apply_broadcast(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        operator: &Bound<'_, PyAny>,
        operand: Option<&Bound<'_, PyAny>>,
        is_filter: bool,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        let registry = self.resolve_registry(py)?;
        registry
            .call_method("_apply_broadcast", (target, operator, operand, is_filter), None)
            .map(|r| r.unbind())
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    fn execute_nd_recursion(
        &self,
        py: Python<'_>,
        seed: &Bound<'_, PyAny>,
        lambda: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        let _guard = NdNestingGuard::enter()?;

        // Fast path: VMFunction lambda in sequential mode → use NDVmDecl (frame stack)
        if lambda.cast::<VMFunction>().is_ok() {
            let decl = pyo3::Py::new(py, crate::nd::NDVmDecl::with_memoize(lambda.clone().unbind(), true))
                .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
            return decl
                .bind(py)
                .call1((seed,))
                .map(|r| r.unbind())
                .map_err(|e| super::core::VMError::RuntimeError(e.to_string()));
        }

        // Slow path: Python NDScheduler (threads, processes, non-VMFunction)
        let registry = self.resolve_registry(py)?;
        registry
            .call_method("execute_nd_recursion_py", (seed, lambda), None)
            .map(|r| r.unbind())
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    fn execute_nd_map(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        func: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        let registry = self.resolve_registry(py)?;
        registry
            .call_method("execute_nd_map_py", (data, func), None)
            .map(|r| r.unbind())
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }
}

/// Enum for accessing cached operator refs from VM.
/// Used only by `ContextHost::new()` to index into VM's cached fields.
#[derive(Clone, Copy)]
pub enum CachedOp {
    Add,
    Sub,
    Mul,
    TrueDiv,
    FloorDiv,
    Mod,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
}

// ---------------------------------------------------------------------------
// GlobalsProxy - dict-like Python wrapper over Globals
// ---------------------------------------------------------------------------

/// Exposes Globals to Python as a dict-like object.
/// Used by `_ImportWrapper` to write imported names into the standalone globals.
///
/// An optional `mirror` (the real Python `context.globals`) makes writes
/// bidirectional: the VM reads the Rust IndexMap, but the AST executor resolves
/// names from the Python context, so register-time mutations must land in both.
#[pyclass(unsendable)]
pub struct GlobalsProxy {
    globals: Globals,
    mirror: Option<Py<PyAny>>,
    /// Id of the struct registry that indexes the TAG_STRUCT values living in
    /// `globals`. A displaced struct must be released against it: `Value::decref`
    /// is a deliberate no-op on struct (release needs the registry in hand), so
    /// a struct-blind proxy pinned every overwritten instance. Resolved through
    /// the registry identity table (`proxy_registry_decref`), cross-pipeline-safe
    /// like `CatnipStructProxy::drop`. `0` = no registry in scope at construction
    /// (the import/extension proxies, whose feeder VM registry is not reachable
    /// there); struct release then no-ops, matching the prior behavior.
    registry_id: u64,
}

impl GlobalsProxy {
    /// Build a proxy over `globals`, binding the registry that owns the map's
    /// struct slots so a displaced instance is released struct-aware (a plain
    /// `Value::decref` no-ops on struct and would pin it). `registry_id == 0`
    /// keeps the struct-blind behavior when no feeder registry is in scope.
    /// Used by `Pipeline::globals` and the import proxy.
    pub fn with_registry(globals: Globals, registry_id: u64) -> Self {
        Self {
            globals,
            mirror: None,
            registry_id,
        }
    }

    /// Like `with_registry`, but also mirror every write into a Python dict-like
    /// object (the real `context.globals`), so AST-mode name resolution sees
    /// them. Used by the extension proxy.
    pub fn with_mirror_registry(globals: Globals, mirror: Py<PyAny>, registry_id: u64) -> Self {
        Self {
            globals,
            mirror: Some(mirror),
            registry_id,
        }
    }

    /// The struct registry id this proxy releases displaced structs against
    /// (`0` if none was bound).
    pub fn registry_id(&self) -> u64 {
        self.registry_id
    }

    /// Expose the inner Rc for use by ImportLoader.
    pub fn globals_rc(&self) -> Globals {
        Rc::clone(&self.globals)
    }

    /// Release a value displaced from the globals map. A TAG_STRUCT needs its
    /// registry in hand (`Value::decref` no-ops on struct and would pin the
    /// instance), so route it through the registry named by `registry_id`. Safe
    /// because every struct is stored via `with_struct_registry_installed`, so
    /// its index belongs to `registry_id` -- the release never targets an
    /// unrelated instance. A no-op if that registry is already gone, or if no
    /// registry was bound (`registry_id == 0`, same as the prior `decref`).
    /// Every other Value releases directly.
    fn release_displaced(&self, old: Value) {
        match old.as_struct_instance_idx() {
            Some(idx) if self.registry_id != 0 => {
                super::value::proxy_registry_decref(self.registry_id, idx);
            }
            _ => old.decref(),
        }
    }
}

#[pymethods]
impl GlobalsProxy {
    fn __setitem__(&self, py: Python<'_>, key: &str, value: Bound<'_, PyAny>) -> PyResult<()> {
        // Convert with THIS proxy's registry installed as the thread-local, so a
        // struct value is indexed into `registry_id` (not whatever registry a
        // prior execution left installed) -- the invariant `release_displaced`
        // relies on to release against the right registry.
        let val = super::value::with_struct_registry_installed(self.registry_id, || Value::from_pyobject(py, &value))
            .map_err(pyo3::exceptions::PyValueError::new_err)?;
        // `Value` is `Copy` with manual refcounting: the displaced entry must be
        // released or its slot leaks. Bind it out of the `borrow_mut` first so
        // the release runs OUTSIDE the borrow -- a pyobj release can run `__del__`
        // that re-enters this map, and a struct release cascades into fields.
        let displaced = self.globals.borrow_mut().insert(key.to_string(), val);
        if let Some(old) = displaced {
            self.release_displaced(old);
        }
        if let Some(mirror) = &self.mirror {
            mirror.bind(py).set_item(key, &value)?;
        }
        Ok(())
    }

    fn __getitem__(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
        match self.globals.borrow().get(key) {
            Some(val) => Ok(val.to_pyobject(py)),
            None => Err(pyo3::exceptions::PyKeyError::new_err(key.to_string())),
        }
    }

    #[pyo3(signature = (key, default=None))]
    fn get(&self, py: Python<'_>, key: &str, default: Option<Py<PyAny>>) -> Py<PyAny> {
        match self.globals.borrow().get(key) {
            Some(val) => val.to_pyobject(py),
            None => default.unwrap_or_else(|| py.None()),
        }
    }

    fn __contains__(&self, key: &str) -> bool {
        self.globals.borrow().contains_key(key)
    }

    fn __len__(&self) -> usize {
        self.globals.borrow().len()
    }

    fn keys(&self) -> Vec<String> {
        self.globals.borrow().keys().cloned().collect()
    }

    fn items(&self, py: Python<'_>) -> Vec<(String, Py<PyAny>)> {
        self.globals
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.to_pyobject(py)))
            .collect()
    }

    fn __iter__(&self) -> Vec<String> {
        self.globals.borrow().keys().cloned().collect()
    }

    fn update(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<()> {
        let Ok(dict) = other.cast::<pyo3::types::PyDict>() else {
            return Ok(());
        };
        let mirror = self.mirror.as_ref().map(|m| m.bind(py));
        // Single pass over `dict` (re-iterating after a release could observe a
        // dict mutated by a displaced value's `__del__`). Displaced values are
        // collected and released AFTER the borrow ends -- a struct or pyobj
        // release must not run while the map is borrowed (a pyobj `__del__` can
        // re-enter it, a struct cascades into fields; mirrors `inject_from_pydict`).
        // Convert with this proxy's registry installed so struct values index
        // into `registry_id` (see `__setitem__`). A mirror `set_item` error is
        // captured and raised only after the pending releases, so it can never
        // strand an owned value.
        let mut displaced: Vec<Value> = Vec::new();
        let mut mirror_err: Option<pyo3::PyErr> = None;
        super::value::with_struct_registry_installed(self.registry_id, || {
            let mut g = self.globals.borrow_mut();
            for (key, value) in dict.iter() {
                if let Ok(name) = key.extract::<String>() {
                    if let Ok(val) = Value::from_pyobject(py, &value) {
                        if let Some(old) = g.insert(name, val) {
                            displaced.push(old);
                        }
                    }
                    if let Some(mirror) = &mirror {
                        if let Err(e) = mirror.set_item(&key, &value) {
                            mirror_err = Some(e);
                            break;
                        }
                    }
                }
            }
        });
        for old in displaced {
            self.release_displaced(old);
        }
        match mirror_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// ND arity validation (standalone)
// ---------------------------------------------------------------------------

/// Which ND operator a shared broadcast dispatch is running.
#[derive(Clone, Copy)]
enum NdKind {
    /// `~~` -- lambda(value, recur); nesting is depth-guarded.
    Recursion,
    /// `~>` -- func(value); no recur handle, cannot nest.
    Map,
}

/// Materialize an ND result: the accumulated list as-is, or converted to a
/// tuple when the broadcast target was one.
fn into_list_or_tuple(
    py: Python<'_>,
    result_list: pyo3::Bound<'_, pyo3::types::PyList>,
    is_tuple: bool,
) -> Result<Py<PyAny>, super::core::VMError> {
    if is_tuple {
        Ok(pyo3::types::PyTuple::new(py, result_list)
            .to_vm(py)?
            .into_any()
            .unbind())
    } else {
        Ok(result_list.into_any().unbind())
    }
}

/// Release a `Value` minted by `from_pyobject`/`frozen_to_value` once its
/// payload has been cloned out (`to_pyobject`/`value_to_frozen`). Registry-aware:
/// struct counts go through `decref_discard`; plain `decref` if no registry is
/// installed. Skipping this strands one ObjectTable handle or registry slot per
/// value on a long-lived host.
fn release_minted(val: Value) {
    let reg = super::value::save_struct_registry();
    if reg.is_null() {
        val.decref();
    } else {
        // SAFETY: non-null, live thread-local registry under the GIL.
        super::core::decref_discard(unsafe { &*reg }, val);
    }
}

/// Validate that a callable has the expected parameter count for ND operations.
///
/// Fast path: downcast to `VMFunction` and read `nargs` directly.
/// Fallback: try `.getattr("params")` (AST lambdas) or `.getattr("vm_code").getattr("nargs")` (Python VMFunction).
/// Builtins (neither path works): skip silently.
fn validate_nd_arity_standalone(
    py: Python<'_>,
    func: &Bound<'_, PyAny>,
    expected: usize,
    op_name: &str,
) -> Result<(), super::core::VMError> {
    // Fast path: Rust VMFunction
    if let Ok(vmfunc) = func.cast::<VMFunction>() {
        let nargs = vmfunc.borrow().vm_code.borrow(py).inner.nargs;
        if nargs != expected {
            return Err(super::core::VMError::TypeError(format!(
                "{op_name} expects a function with {expected} parameters{}, got {nargs}",
                if op_name == "~~" { " (value, recur)" } else { "" },
            )));
        }
        return Ok(());
    }

    // Fallback: AST lambda with .params attribute
    if let Ok(params) = func.getattr("params") {
        if let Ok(nargs) = params.len() {
            if nargs != expected {
                return Err(super::core::VMError::TypeError(format!(
                    "{op_name} expects a function with {expected} parameters{}, got {nargs}",
                    if op_name == "~~" { " (value, recur)" } else { "" },
                )));
            }
            return Ok(());
        }
    }

    // Fallback: Python VMFunction wrapper
    if let Ok(vm_code) = func.getattr("vm_code") {
        if let Ok(nargs_attr) = vm_code.getattr("nargs") {
            if let Ok(nargs) = nargs_attr.extract::<usize>() {
                if nargs != expected {
                    return Err(super::core::VMError::TypeError(format!(
                        "{op_name} expects a function with {expected} parameters{}, got {nargs}",
                        if op_name == "~~" { " (value, recur)" } else { "" },
                    )));
                }
                return Ok(());
            }
        }
    }

    // Builtin or unknown callable: skip validation
    Ok(())
}

// ---------------------------------------------------------------------------
// VMHost - standalone host backed by Rust HashMap globals
// ---------------------------------------------------------------------------

/// Host backed by Rust-owned globals and cached Python operator refs.
///
/// Used by `Executor` to run the VM without a Python Context.
/// Builtins (abs, len, range, etc.) are populated from Python `builtins`
/// module at construction time, then stored in the globals HashMap.
pub struct VMHost {
    globals: Globals,
    /// Operator refs indexed by `BinaryOp as usize`.
    ops: [Py<PyAny>; 11],
    /// Cached iter() builtin.
    iter_fn: Py<PyAny>,
    /// Cached operator.contains for `in` / `not in`.
    cached_contains: Py<PyAny>,
    /// Always None - pass_context not supported in standalone.
    no_context: Option<Py<PyAny>>,
    /// ND broadcast parallelism config.
    nd_config: NdConfig,
    /// Lazy-initialized pool of Rust worker processes for NdMode::Process.
    /// SAFETY: RefCell is correct here because VMHost is only accessed from the VM thread.
    /// VmHost trait methods take &self, but NdMode::Process needs mutability for lazy init.
    worker_pool: RefCell<Option<crate::nd::worker_pool::WorkerPool>>,
    /// Module import policy (set via --policy CLI flag).
    module_policy: Option<Py<PyAny>>,
    /// Direct strong reference to the ImportLoader injected as the `import`
    /// builtin. The map only holds it through an ObjectTable slot -- a Python
    /// reference the loader's own `__traverse__` claims for the collector, so
    /// without this anchor a young-generation collection can see the loader's
    /// wrapper cycle as closed-dead while an OLD (unscanned, thus silent)
    /// pipeline still uses it, and clear it. A plain C-level reference from
    /// this host is never subtracted when the host's pipeline is outside the
    /// collected generation: the cluster stays reachable for as long as the
    /// pipeline lives, whatever the generations.
    import_loader: Option<Py<PyAny>>,
}

type FrozenCapture = (String, catnip_core::freeze::FrozenValue);

struct FreezableLambda {
    encoded_ir: Vec<u8>,
    captures: Vec<FrozenCapture>,
    param_names: Vec<String>,
}

impl VMHost {
    /// Cache operator refs and builtins iter/contains.
    #[allow(clippy::type_complexity)]
    fn setup_operators(py: Python<'_>) -> PyResult<([Py<PyAny>; 11], Py<PyAny>, Py<PyAny>)> {
        let op_mod = py.import("operator")?;
        let ops = [
            op_mod.getattr("add")?.unbind(),
            op_mod.getattr("sub")?.unbind(),
            op_mod.getattr("mul")?.unbind(),
            op_mod.getattr("truediv")?.unbind(),
            op_mod.getattr("floordiv")?.unbind(),
            op_mod.getattr("mod")?.unbind(),
            op_mod.getattr("pow")?.unbind(),
            op_mod.getattr("lt")?.unbind(),
            op_mod.getattr("le")?.unbind(),
            op_mod.getattr("gt")?.unbind(),
            op_mod.getattr("ge")?.unbind(),
        ];
        let iter_fn = py.import("builtins")?.getattr("iter")?.unbind();
        let cached_contains = op_mod.getattr("contains")?.unbind();
        Ok((ops, iter_fn, cached_contains))
    }

    /// Build a standalone host with builtins injected into globals. `registry_id`
    /// is `0`: this path (broadcast workers) creates its VM -- and registry --
    /// after the host, so no feeder id is available. The executor path passes a
    /// real id to `new_with_policy` so the import/extension proxies release
    /// displaced struct globals struct-aware.
    pub fn new(py: Python<'_>) -> PyResult<Self> {
        Self::new_with_policy(py, None, 0)
    }

    pub fn new_with_policy(py: Python<'_>, module_policy: Option<Py<PyAny>>, registry_id: u64) -> PyResult<Self> {
        let globals: Globals = Rc::new(RefCell::new(IndexMap::new()));
        let (ops, iter_fn, cached_contains) = Self::setup_operators(py)?;

        // Inject Python builtins into globals
        let builtins = py.import("builtins")?;
        let builtin_names = [
            "abs",
            "min",
            "max",
            "range",
            "len",
            "int",
            "float",
            "str",
            "bool",
            "list",
            "dict",
            "tuple",
            "set",
            "complex",
            "sorted",
            "reversed",
            "enumerate",
            "zip",
            "map",
            "filter",
            "format",
            "repr",
            "ascii",
            "sum",
            "round",
            "isinstance",
            "hasattr",
            "getattr",
            "setattr",
            "print",
            "input",
            "open",
            "chr",
            "ord",
            "hex",
            "bin",
            "oct",
            "hash",
            "id",
            "callable",
            "iter",
            "next",
            "any",
            "all",
            "slice",
            "frozenset",
            "bytes",
            "bytearray",
            "memoryview",
            "object",
            "super",
            "staticmethod",
            "classmethod",
            "property",
            "vars",
            "dir",
            // Builtin exception types -- resolvable as values (e.g.
            // `contextlib.suppress(ValueError)`), not just in `raise`/`except`
            // positions. Mirrors `catnip_core::exception::ExceptionKind`.
            "Exception",
            "TypeError",
            "ValueError",
            "NameError",
            "IndexError",
            "KeyError",
            "AttributeError",
            "ZeroDivisionError",
            "RuntimeError",
            "MemoryError",
            "ArithmeticError",
            "LookupError",
        ];
        {
            let mut g = globals.borrow_mut();
            for name in &builtin_names {
                if let Ok(obj) = builtins.getattr(*name) {
                    if let Ok(val) = Value::from_pyobject(py, &obj) {
                        g.insert(name.to_string(), val);
                    }
                }
            }
            // Constants
            g.insert("True".to_string(), Value::TRUE);
            g.insert("False".to_string(), Value::FALSE);
            g.insert("None".to_string(), Value::NIL);

            // Decorators: jit/pure as identity, cached with basic memoization.
            // Each key gets its OWN ObjectTable handle (separate `from_pyobject`):
            // `Value` is `Copy` with manual refcounting, so sharing one handle
            // across two keys would make a later per-key `decref` (on overwrite or
            // teardown) double-release the slot.
            if let Ok(id_fn) = py.eval(c"lambda f: f", None, None) {
                if let Ok(val) = Value::from_pyobject(py, &id_fn) {
                    g.insert("jit".to_string(), val);
                }
                if let Ok(val) = Value::from_pyobject(py, &id_fn) {
                    g.insert("pure".to_string(), val);
                }
            }
            if let Ok(memo_mod) = py.import(PY_MOD_MEMOIZATION) {
                if let Ok(cached_fn) = memo_mod.getattr("standalone_cached") {
                    if let Ok(val) = Value::from_pyobject(py, &cached_fn) {
                        g.insert("cached".to_string(), val);
                    }
                }
            }
        }

        // Inject freeze/thaw builtins
        // Try _rs module first (available when installed as package),
        // then try wrap_pyfunction (works in embedded/standalone mode).
        {
            let mut g = globals.borrow_mut();
            let injected = if let Ok(rs_mod) = py.import(PY_MOD_RS) {
                let mut ok = false;
                for name in &["freeze", "thaw"] {
                    if let Ok(func) = rs_mod.getattr(*name) {
                        if let Ok(val) = Value::from_pyobject(py, &func) {
                            g.insert(name.to_string(), val);
                            ok = true;
                        }
                    }
                }
                ok
            } else {
                false
            };
            if !injected {
                if let Ok(freeze_fn) = pyo3::wrap_pyfunction!(crate::freeze::freeze, py) {
                    if let Ok(val) = Value::from_pyobject(py, freeze_fn.as_any()) {
                        g.insert("freeze".to_string(), val);
                    }
                }
                if let Ok(thaw_fn) = pyo3::wrap_pyfunction!(crate::freeze::thaw, py) {
                    if let Ok(val) = Value::from_pyobject(py, thaw_fn.as_any()) {
                        g.insert("thaw".to_string(), val);
                    }
                }
            }
        }

        // Inject fold/reduce as inline Python (no catnip.context import needed)
        {
            let ns = PyDict::new(py);
            py.run(
                c"
def fold(xs, init, f):
    acc = init
    for x in xs:
        acc = f(acc, x)
    return acc

def reduce(xs, f):
    it = iter(xs)
    try:
        acc = next(it)
    except StopIteration:
        raise ValueError('reduce() of empty sequence with no initial value')
    for x in it:
        acc = f(acc, x)
    return acc
",
                Some(&ns),
                None,
            )?;
            let mut g = globals.borrow_mut();
            for name in &["fold", "reduce"] {
                if let Ok(Some(obj)) = ns.get_item(*name) {
                    if let Ok(val) = Value::from_pyobject(py, &obj) {
                        g.insert(name.to_string(), val);
                    }
                }
            }
        }

        // Inject spread helpers (__catnip_spread_{list,tuple,set,dict})
        {
            let spread_ns = PyDict::new(py);
            py.run(
                c"
def __catnip_spread_list(entries):
    import builtins
    out = []
    for entry in entries:
        if entry[0]:
            out.extend(entry[1])
        else:
            out.append(entry[1])
    return out

def __catnip_spread_tuple(entries):
    import builtins
    return builtins.tuple(__catnip_spread_list(entries))

def __catnip_spread_set(entries):
    import builtins
    out = builtins.set()
    for entry in entries:
        if entry[0]:
            out.update(entry[1])
        else:
            out.add(entry[1])
    return out

def __catnip_spread_dict(entries):
    import builtins
    out = builtins.dict()
    for entry in entries:
        if entry[0]:
            out.update(entry[1])
        else:
            out[entry[1]] = entry[2]
    return out
",
                Some(&spread_ns),
                None,
            )?;
            let mut g = globals.borrow_mut();
            for name in &[
                "__catnip_spread_list",
                "__catnip_spread_tuple",
                "__catnip_spread_set",
                "__catnip_spread_dict",
            ] {
                if let Ok(Some(obj)) = spread_ns.get_item(*name) {
                    if let Ok(val) = Value::from_pyobject(py, &obj) {
                        g.insert(name.to_string(), val);
                    }
                }
            }
        }

        // Inject META into globals (for import() caller_dir resolution)
        let meta = Py::new(py, crate::core::meta::CatnipMeta::new())?;
        meta.bind(py).setattr("main", true)?;
        {
            let val = Value::from_pyobject(py, meta.bind(py)).map_err(pyo3::exceptions::PyRuntimeError::new_err)?;
            globals.borrow_mut().insert("META".to_string(), val);
        }

        // Inject builtin constant namespaces (ND, RUNTIME)
        {
            let nd = Py::new(py, crate::core::builtins::make_nd(py))?;
            if let Ok(val) = Value::from_pyobject(py, nd.bind(py)) {
                globals.borrow_mut().insert("ND".to_string(), val);
            }
            let rt = Py::new(py, crate::core::builtins::make_runtime(py))?;
            if let Ok(val) = Value::from_pyobject(py, rt.bind(py)) {
                globals.borrow_mut().insert("RUNTIME".to_string(), val);
            }
        }

        // Inject import() builtin via Rust ImportLoader
        let import_loader: Option<Py<PyAny>> = {
            let proxy = GlobalsProxy::with_registry(Rc::clone(&globals), registry_id);
            let proxy_obj = Py::new(py, proxy)?;

            // .cat loading callback: delegates to Python ModuleLoader
            let cat_loader_ns = PyDict::new(py);
            py.run(
                c"
def _make_cat_loader(globals_proxy):
    import types
    ctx = types.SimpleNamespace(globals=globals_proxy)
    from catnip.loader import ModuleLoader
    loader = ModuleLoader(ctx)
    def cat_loader(path, name):
        return loader.load_catnip_module(path, module_name=name)
    return cat_loader
",
                Some(&cat_loader_ns),
                None,
            )?;
            let make_fn = cat_loader_ns
                .get_item("_make_cat_loader")?
                .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("missing _make_cat_loader"))?;
            let cat_loader = make_fn.call1((proxy_obj.bind(py),))?;

            // Create the Rust ImportLoader
            let loader = crate::loader::ImportLoader::create(
                py,
                Rc::clone(&globals),
                module_policy.as_ref().map(|p| p.clone_ref(py)),
                Some(cat_loader.unbind()),
                registry_id,
            )?;
            if let Ok(val) = Value::from_pyobject(py, loader.bind(py)) {
                globals.borrow_mut().insert("import".to_string(), val);
            }
            Some(loader.into_any())
        };

        Ok(Self {
            globals,
            ops,
            iter_fn,
            cached_contains,
            no_context: None,
            nd_config: NdConfig::default(),
            worker_pool: RefCell::new(None),
            module_policy,
            import_loader,
        })
    }

    /// Build a host sharing an existing globals Arc (for child VMs).
    pub fn with_globals(py: Python<'_>, globals: Globals) -> PyResult<Self> {
        let (ops, iter_fn, cached_contains) = Self::setup_operators(py)?;
        Ok(Self {
            globals,
            ops,
            iter_fn,
            cached_contains,
            no_context: None,
            nd_config: NdConfig::default(),
            worker_pool: RefCell::new(None),
            module_policy: None,
            import_loader: None,
        })
    }

    /// Get a reference to the shared globals.
    pub fn globals(&self) -> &Globals {
        &self.globals
    }

    /// Set the Python context for pass_context and registry access.
    pub fn set_context(&mut self, context: Py<PyAny>) {
        self.no_context = Some(context);
    }

    /// Report to the cyclic GC the references this host owns: the Python
    /// context and policy, plus one Python reference per ObjectTable slot
    /// reachable from the globals map (`inject_globals` copied the context's
    /// builtins -- wrappers and `RUNTIME` referencing the context back -- into
    /// that map as `Value` handles, with the matching strong `Py` references
    /// held by the global `OBJECT_TABLE`, invisible to the collector).
    ///
    /// The host is the map's *owner*; co-holders of the `Rc` (the
    /// `ImportLoader`, transient `GlobalsProxy`s) must NOT report these
    /// handles, or a live pipeline's wrapper cluster looks like a closed dead
    /// cycle and gets cleared under it. Dedup-by-slot lives in
    /// `visit_obj_handles` (several globals may share one slot); `try_borrow`
    /// guards against a re-entrant GC mid-execution.
    pub fn gc_traverse(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Some(ref ctx) = self.no_context {
            visit.call(ctx)?;
        }
        if let Some(ref policy) = self.module_policy {
            visit.call(policy)?;
        }
        if let Some(ref loader) = self.import_loader {
            visit.call(loader)?;
        }
        if let Ok(globals) = self.globals.try_borrow() {
            crate::vm::value::visit_obj_handles(globals.values().copied(), visit)?;
        }
        Ok(())
    }

    /// Drop the references reported by `gc_traverse`. `Value` is `Copy` with
    /// manual refcounting: every handle must be `decref`'d (a slot reached by
    /// `k` aliased globals holds `k` handle refcounts -- no dedup here, unlike
    /// the traverse) or the `OBJECT_TABLE` slots leak.
    pub fn gc_clear(&mut self) {
        self.no_context = None;
        self.module_policy = None;
        self.import_loader = None;
        self.drain_globals(None);
    }

    /// Decref and drop every handle in the globals map, releasing OUTSIDE the
    /// borrow (a pyobj `__del__` may re-enter this map; holding the borrow across
    /// the release would leave a half-drained map visible to it -- like the
    /// sibling `inject_from_pydict`/`set_global` fixes). `registry`:
    ///
    /// - `Some(reg)` -- struct-aware drain for an EPHEMERAL host
    ///   (`VMFunction::__call__`): its fresh map may hold TAG_STRUCT values
    ///   (`build_globals_from_context` re-interns context proxies, increffing a
    ///   PARENT registry slot), on which `Value::decref` is a deliberate no-op
    ///   (one leaked registry count per re-entrant call otherwise; found by the
    ///   intra-session ledger on `broadcast + exported instance`).
    /// - `None` -- teardown only (GC `__clear__`): the map's surviving struct
    ///   slots die with their registry anyway, so no registry is needed. Forcing
    ///   the caller to spell out `None` keeps the struct-blind path a conscious
    ///   choice, not the default-obvious short name that silently re-leaks.
    ///
    /// Idempotent (draining an already-drained map is a no-op), so the two GC
    /// `__clear__` paths (`PyPipeline`, `ImportLoader`) compose in either order.
    ///
    /// Deliberately NOT called from a `Drop` impl: when the pipeline dies by
    /// plain refcounting, the still-full map (kept alive by the loader's `Rc`)
    /// is what lets `ImportLoader::__traverse__` report the slots' `Py`
    /// references at the next collection -- draining it early leaves leaked
    /// constant handles (CodeObject pools) as the slots' only holders,
    /// unreported, and the context cycle stays pinned forever.
    pub(crate) fn drain_globals(&self, registry: Option<&super::structs::StructRegistry>) {
        let drained: Vec<Value> = match self.globals.try_borrow_mut() {
            Ok(mut globals) => globals.drain(..).map(|(_, v)| v).collect(),
            Err(_) => return,
        };
        for value in drained {
            match registry {
                Some(reg) => super::core::decref_discard(reg, value),
                None => value.decref(),
            }
        }
    }

    /// Set module import policy (used by --policy CLI flag).
    pub fn set_module_policy(&mut self, policy: Py<PyAny>) {
        self.module_policy = Some(policy);
    }

    /// Set the ND broadcast mode.
    pub fn set_nd_mode(&mut self, mode: NdMode) {
        self.nd_config.mode = mode;
    }

    pub fn set_nd_memoize(&mut self, memoize: bool) {
        self.nd_config.memoize = memoize;
    }

    /// Set META.file for relative import resolution.
    pub fn set_meta_file(&self, py: Python<'_>, path: &str) -> PyResult<()> {
        let g = self.globals.borrow();
        if let Some(meta_val) = g.get("META") {
            let meta_obj = meta_val.to_pyobject(py);
            meta_obj.bind(py).setattr("file", path)?;
        }
        Ok(())
    }

    /// Shared `~~`/`~>` dispatch over the ND modes: sequential leaf calls,
    /// rayon threads, or the native process worker pool (thread fallback).
    /// The two operators differ only by arity, leaf executor, thread runner
    /// and the nesting guard (recursion is depth-guarded, map cannot recurse).
    fn broadcast_nd(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        func: &Bound<'_, PyAny>,
        kind: NdKind,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        let (arity, op) = match kind {
            NdKind::Recursion => (2, "~~"),
            NdKind::Map => (1, "~>"),
        };
        validate_nd_arity_standalone(py, func, arity, op)?;

        match self.nd_config.mode {
            NdMode::Sequential => {
                let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let result_list = pyo3::types::PyList::empty(py);
                for elem_result in target.try_iter().to_vm(py)? {
                    let elem = elem_result.to_vm(py)?;
                    let nd_result = match kind {
                        NdKind::Recursion => self.execute_nd_recursion(py, &elem, func)?,
                        NdKind::Map => self.execute_nd_map(py, &elem, func)?,
                    };
                    result_list
                        .append(nd_result)
                        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                }
                into_list_or_tuple(py, result_list, is_tuple)
            }
            NdMode::Thread | NdMode::Process => {
                // Recursion is depth-guarded against runaway nesting; map has
                // no recur handle so it cannot nest.
                let _guard = match kind {
                    NdKind::Recursion => Some(NdNestingGuard::enter()?),
                    NdKind::Map => None,
                };

                // Collect elements with GIL
                let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let elements: Vec<Py<PyAny>> = target
                    .try_iter()
                    .to_vm(py)?
                    .map(|r| r.map(|e| e.unbind()))
                    .collect::<Result<Vec<_>, _>>()
                    .to_vm(py)?;

                // Process mode: try the native Rust worker path first
                if matches!(self.nd_config.mode, NdMode::Process) {
                    if let Some(results) = self.try_native_nd_batch(py, &elements, func)? {
                        let result_list = pyo3::types::PyList::empty(py);
                        for frozen in &results {
                            let val = crate::freeze::frozen_to_value(py, frozen);
                            let py_obj = val.to_pyobject(py);
                            // py_obj holds its own clone; release the thawed value
                            // (same discipline as the seed-freeze loops).
                            release_minted(val);
                            result_list
                                .append(py_obj)
                                .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                        }
                        return into_list_or_tuple(py, result_list, is_tuple);
                    }
                    // Fallthrough: thread (rayon) -- pickle-free, handles what
                    // the native worker can't freeze.
                }

                match kind {
                    NdKind::Recursion => self.run_nd_recursion_thread(py, func, elements, is_tuple),
                    NdKind::Map => self.run_nd_map_thread(py, func, elements, is_tuple),
                }
            }
        }
    }

    /// Try to execute an ND batch (`~~` recursion or `~>` map: same freeze +
    /// submit path) via native Rust workers. Returns None if the lambda,
    /// captures or seeds aren't fully freezable (caller falls back to threads).
    fn try_native_nd_batch(
        &self,
        py: Python<'_>,
        elements: &[Py<PyAny>],
        func: &Bound<'_, PyAny>,
    ) -> Result<Option<Vec<catnip_core::freeze::FrozenValue>>, super::core::VMError> {
        let lambda_data = match self.extract_freezable_lambda(py, func) {
            Some(v) => v,
            None => return Ok(None),
        };

        // Freeze all seed elements
        let seeds: Option<Vec<_>> = elements
            .iter()
            .map(|elem| {
                let val = Value::from_pyobject(py, elem.bind(py)).ok()?;
                let frozen = crate::freeze::value_to_frozen(py, val);
                // from_pyobject took a ref (struct incref / owned py handle);
                // release it now that the value is read into a FrozenValue.
                release_minted(val);
                frozen
            })
            .collect();
        let seeds = match seeds {
            Some(s) => s,
            None => return Ok(None),
        };

        self.submit_to_worker_pool(
            py,
            &lambda_data.encoded_ir,
            &lambda_data.captures,
            &lambda_data.param_names,
            &seeds,
        )
    }

    /// Lazy-init worker pool and submit a batch. Returns None on any failure (triggers Python fallback).
    fn submit_to_worker_pool(
        &self,
        py: Python<'_>,
        encoded_ir: &[u8],
        captures: &[(String, catnip_core::freeze::FrozenValue)],
        param_names: &[String],
        seeds: &[catnip_core::freeze::FrozenValue],
    ) -> Result<Option<Vec<catnip_core::freeze::FrozenValue>>, super::core::VMError> {
        // A builtin shadowed by a user VM function (e.g. `str = (x) => {...}`)
        // can't be frozen: the worker resolves the name to its own pre-seeded
        // builtin instead, a silent divergence no error would catch. Fall back
        // to the thread path. ponytail: keyed on Python builtins (hasattr);
        // Catnip-only builtins (fold/reduce) shadowed stay a rare residual.
        if let Ok(py_builtins) = py.import("builtins") {
            let shadowed = self
                .globals
                .borrow()
                .iter()
                .any(|(name, val)| val.is_vmfunc() && py_builtins.hasattr(name.as_str()).unwrap_or(false));
            if shadowed {
                return Ok(None);
            }
        }

        // Ship struct type defs only when a struct is actually in play: an
        // int-only batch stays on the native path even if an unsupported struct
        // type exists elsewhere in the registry. When a struct is present,
        // collect every live type (D3 over-approximation); a type outside the v1
        // frontier makes the collector return None -> fall back.
        // Freeze the parent globals the callback (or a struct method it calls)
        // may reference by name; non-freezable globals (functions, modules) are
        // skipped. Over-approximation (all freezable globals), like type_defs --
        // closure-transitive selection is the deferred optimization.
        let globals: Vec<(String, catnip_core::freeze::FrozenValue)> = self
            .globals
            .borrow()
            .iter()
            .filter_map(|(name, val)| crate::freeze::value_to_frozen(py, *val).map(|f| (name.clone(), f)))
            .collect();

        // A struct in play -- in a seed, a capture, or a global -- needs its type
        // shipped too.
        let needs_types = seeds.iter().any(crate::freeze::frozen_has_struct)
            || captures.iter().any(|(_, v)| crate::freeze::frozen_has_struct(v))
            || globals.iter().any(|(_, v)| crate::freeze::frozen_has_struct(v));
        let type_defs = if needs_types {
            let ptr = super::value::save_struct_registry();
            if ptr.is_null() {
                return Ok(None);
            }
            // SAFETY: non-null, names the live thread-local registry under the GIL.
            let registry = unsafe { &*ptr };
            match crate::freeze::collect_frozen_struct_types(py, registry) {
                Some(t) => t,
                None => return Ok(None),
            }
        } else {
            Vec::new()
        };

        let mut pool = self.worker_pool.borrow_mut();
        if pool.is_none() {
            match crate::nd::worker_pool::WorkerPool::new(crate::nd::worker_pool::default_pool_size()) {
                Ok(p) => *pool = Some(p),
                Err(_) => return Ok(None),
            }
        }
        let worker_pool = pool.as_mut().unwrap();
        match worker_pool.submit_batch(encoded_ir, captures, param_names, seeds, &type_defs, &globals) {
            Ok(results) => Ok(Some(results)),
            Err(_) => Ok(None),
        }
    }

    /// Extract frozen IR, captures, and param names from a VMFunction.
    /// Returns None if the function isn't freezable.
    fn extract_freezable_lambda(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> Option<FreezableLambda> {
        // Downcast to VMFunction
        let vm_func: pyo3::PyRef<'_, super::frame::VMFunction> = func.extract().ok()?;

        // Get the CodeObject and check for encoded_ir
        let code_obj = vm_func.vm_code.borrow(py);
        let encoded_ir = code_obj.inner.encoded_ir.as_ref()?.as_ref().clone();

        // Extract param names from CodeObject
        let param_names: Vec<String> = code_obj.inner.varnames[..code_obj.inner.nargs].to_vec();

        // Freeze closure captures
        let captures = if let Some(ref scope) = vm_func.native_closure {
            crate::freeze::freeze_captures(py, scope)?
        } else {
            Vec::new()
        };

        Some(FreezableLambda {
            encoded_ir,
            captures,
            param_names,
        })
    }

    /// Run ND-recursion over `elements` on the rayon thread pool (the Thread
    /// mode body). Also the Process fallback when `try_native` can't freeze the
    /// batch: pickle-free, GIL-bound, handles everything the native worker
    /// can't. The caller owns the `NdNestingGuard`.
    fn run_nd_recursion_thread(
        &self,
        py: Python<'_>,
        lambda: &Bound<'_, PyAny>,
        elements: Vec<Py<PyAny>>,
        is_tuple: bool,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        let lambda_ref = lambda.clone().unbind();
        let memoize = self.nd_config.memoize;
        let globals_snapshot = self.globals.borrow().clone();
        let registries = NdRegistryHandle::capture();

        let results: Vec<Result<Py<PyAny>, String>> = py.detach(|| {
            elements
                .into_par_iter()
                .map_with(globals_snapshot, |thread_map, elem| {
                    Python::attach(|py| {
                        let thread_globals: Globals = Rc::new(RefCell::new(thread_map.clone()));
                        set_vm_globals(thread_globals);
                        let _registry_guard = registries.install();

                        let lambda_bound = lambda_ref.bind(py);
                        let elem_bound = elem.bind(py);
                        let decl = Py::new(
                            py,
                            crate::nd::NDParallelDecl::with_memoize(lambda_ref.clone_ref(py), memoize),
                        )
                        .map_err(|e| e.to_string())?;
                        lambda_bound
                            .call1((elem_bound, decl.bind(py)))
                            .map(|r| r.unbind())
                            .map_err(|e| e.to_string())
                    })
                })
                .collect()
        });

        let result_list = pyo3::types::PyList::empty(py);
        for r in results {
            let val = r.map_err(super::core::VMError::RuntimeError)?;
            result_list
                .append(val)
                .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        }
        into_list_or_tuple(py, result_list, is_tuple)
    }

    /// Run ND-map over `elements` on the rayon thread pool (the Thread mode
    /// body); also the Process fallback when `try_native` can't freeze the batch.
    fn run_nd_map_thread(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        elements: Vec<Py<PyAny>>,
        is_tuple: bool,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        let func_ref = func.clone().unbind();
        let globals_snapshot = self.globals.borrow().clone();
        let registries = NdRegistryHandle::capture();

        let results: Vec<Result<Py<PyAny>, String>> = py.detach(|| {
            elements
                .into_par_iter()
                .map_with(globals_snapshot, |thread_map, elem| {
                    Python::attach(|py| {
                        let thread_globals: Globals = Rc::new(RefCell::new(thread_map.clone()));
                        set_vm_globals(thread_globals);
                        let _registry_guard = registries.install();
                        let func_bound = func_ref.bind(py);
                        let elem_bound = elem.bind(py);
                        crate::core::broadcast::nd_map(py, elem_bound, func_bound).map_err(|e| e.to_string())
                    })
                })
                .collect()
        });

        let result_list = pyo3::types::PyList::empty(py);
        for r in results {
            let val = r.map_err(super::core::VMError::RuntimeError)?;
            result_list
                .append(val)
                .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        }
        into_list_or_tuple(py, result_list, is_tuple)
    }
}

impl VmHost for VMHost {
    #[inline]
    fn lookup_global(&self, _py: Python<'_>, name: &str) -> Result<Option<Value>, super::core::VMError> {
        // Voie A: hand back a fully owned value (pyobj handle, bigint Arc,
        // struct registry slot) so both VmHost impls share the ContextHost
        // contract (from_pyobject returns owned); the IndexMap keeps its ref.
        Ok(self.globals.borrow().get(name).copied().inspect(|v| {
            v.clone_refcount();
        }))
    }

    #[inline]
    fn store_global(&self, _py: Python<'_>, name: &str, value: Value) -> Result<Option<Value>, super::core::VMError> {
        // The map OWNS one ref per entry (wip/GLOBALS_OWNERSHIP.md): take it
        // here, hand the overwritten one back to the caller (the dispatch owns
        // the registry, so a displaced TAG_STRUCT gets a real release there --
        // `old.decref()` here was a struct no-op and leaked one registry count
        // per overwrite). The drains (drain_globals, ImportLoader::__clear__)
        // release exactly what was taken, on both teardown paths.
        value.clone_refcount();
        Ok(self.globals.borrow_mut().insert(name.to_string(), value))
    }

    #[inline]
    fn delete_global(&self, _py: Python<'_>, name: &str) -> Result<Option<Value>, super::core::VMError> {
        Ok(self.globals.borrow_mut().swap_remove(name))
    }

    #[inline]
    fn binary_op(&self, py: Python<'_>, op: BinaryOp, a: Value, b: Value) -> Result<Value, super::core::VMError> {
        call_binary_op(py, &self.ops[op as usize], a, b)
    }

    fn resolve_registry<'py>(&self, _py: Python<'py>) -> Result<Bound<'py, PyAny>, super::core::VMError> {
        // Standalone uses Rust-native broadcast, no Python registry needed
        Err(super::core::VMError::RuntimeError(
            "Registry not available in standalone mode".to_string(),
        ))
    }

    #[inline]
    fn get_iter<'py>(
        &self,
        py: Python<'py>,
        obj: &Bound<'py, PyAny>,
    ) -> Result<Bound<'py, PyAny>, super::core::VMError> {
        self.iter_fn
            .bind(py)
            .call1((obj,))
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    #[inline]
    fn has_global(&self, _py: Python<'_>, name: &str) -> bool {
        self.globals.borrow().contains_key(name)
    }

    #[inline]
    fn build_closure_parent(&self, _py: Python<'_>, frame_closure: Option<&NativeClosureScope>) -> ClosureParent {
        match frame_closure {
            Some(parent_closure) => ClosureParent::Native(parent_closure.clone()),
            None => ClosureParent::Globals(Rc::clone(&self.globals)),
        }
    }

    #[inline]
    fn cached_contains(&self) -> &Py<PyAny> {
        &self.cached_contains
    }

    #[inline]
    fn context(&self) -> &Option<Py<PyAny>> {
        &self.no_context
    }

    fn collect_globals(&self, _py: Python<'_>, _target: &Bound<'_, PyDict>) -> Result<(), super::core::VMError> {
        // VMHost globals are already in self.globals (Rust HashMap),
        // collected directly by the VM dispatch. Nothing extra to merge.
        Ok(())
    }

    fn apply_broadcast(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        operator: &Bound<'_, PyAny>,
        operand: Option<&Bound<'_, PyAny>>,
        is_filter: bool,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        use crate::core::broadcast;

        // Install globals so struct methods can find names via VMFunction.__call__
        set_vm_globals(Rc::clone(&self.globals));
        // Ensure globals are cleared on all exit paths
        struct ClearGuard;
        impl Drop for ClearGuard {
            fn drop(&mut self) {
                clear_vm_globals();
            }
        }
        let _guard = ClearGuard;

        let map_err = |e: pyo3::PyErr| super::core::VMError::RuntimeError(e.to_string());

        if is_filter {
            // Filter: operator is a callable condition
            if operator.is_callable() {
                return broadcast::filter_conditional(py, target, operator).map_err(map_err);
            }
            // Boolean mask
            if broadcast::is_boolean_mask(py, operator).unwrap_or(false) {
                return broadcast::filter_by_mask(py, target, operator).map_err(map_err);
            }
        }

        // Boolean mask (no operand, non-callable)
        if operand.is_none() && !operator.is_callable() {
            if let Ok(true) = broadcast::is_boolean_mask(py, operator) {
                return broadcast::filter_by_mask(py, target, operator).map_err(map_err);
            }
            // List/tuple with non-boolean elements: type error
            if operator.is_instance_of::<pyo3::types::PyList>() || operator.is_instance_of::<pyo3::types::PyTuple>() {
                return Err(super::core::VMError::TypeError(
                    "Mask must be a list or tuple of booleans, got list with non-boolean elements".to_string(),
                ));
            }
        }

        // Map: callable operator
        if operator.is_callable() {
            return broadcast::broadcast_map(py, target, operator).map_err(map_err);
        }

        // String operator dispatch
        if let Ok(op_str) = operator.extract::<String>() {
            // SIMD fast path for homogeneous numeric lists
            if is_filter {
                if let Some(operand) = operand {
                    if let Some(result) = broadcast::simd::try_simd_filter(py, target, &op_str, operand) {
                        return result.map_err(map_err);
                    }
                }
            } else if let Some(operand) = operand {
                if let Some(result) = broadcast::simd::try_simd_map(py, target, &op_str, operand) {
                    return result.map_err(map_err);
                }
            }

            let op_mod = py.import("operator").map_err(map_err)?;

            // Binary operators (with operand)
            if let Some(operand) = operand {
                let binary_fn = match op_str.as_str() {
                    "+" => Some("add"),
                    "-" => Some("sub"),
                    "*" => Some("mul"),
                    "/" => Some("truediv"),
                    "//" => Some("floordiv"),
                    "%" => Some("mod"),
                    "**" => Some("pow"),
                    "<" => Some("lt"),
                    "<=" => Some("le"),
                    ">" => Some("gt"),
                    ">=" => Some("ge"),
                    "==" => Some("eq"),
                    "!=" => Some("ne"),
                    "&" => Some("and_"),
                    "|" => Some("or_"),
                    "^" => Some("xor"),
                    "<<" => Some("lshift"),
                    ">>" => Some("rshift"),
                    _ => None,
                };
                if let Some(fn_name) = binary_fn {
                    let op_fn = op_mod.getattr(fn_name).map_err(map_err)?;
                    return broadcast::broadcast_binary_op(py, target, &op_fn, operand, is_filter).map_err(map_err);
                }

                // Logical operators (keywords, need lambda)
                if op_str == "and" || op_str == "or" {
                    let ns = pyo3::types::PyDict::new(py);
                    ns.set_item("__operand__", operand).map_err(map_err)?;
                    let func = if op_str == "and" {
                        py.eval(c"lambda x: x and __operand__", Some(&ns), Some(&ns))
                    } else {
                        py.eval(c"lambda x: x or __operand__", Some(&ns), Some(&ns))
                    }
                    .map_err(map_err)?;
                    if is_filter {
                        return broadcast::filter_conditional(py, target, &func).map_err(map_err);
                    }
                    return broadcast::broadcast_map(py, target, &func).map_err(map_err);
                }
            }

            // Unary operators (no operand needed)
            let unary_fn = match op_str.as_str() {
                "abs" => Some("abs"),
                "-" => Some("neg"),
                "+" => Some("pos"),
                "~" => Some("invert"),
                "not" => Some("not_"),
                _ => None,
            };
            if let Some(fn_name) = unary_fn {
                let op_fn = op_mod.getattr(fn_name).map_err(map_err)?;
                if is_filter {
                    return broadcast::filter_conditional(py, target, &op_fn).map_err(map_err);
                }
                return broadcast::broadcast_map(py, target, &op_fn).map_err(map_err);
            }
        }

        Err(super::core::VMError::RuntimeError(format!(
            "Unsupported broadcast operation in standalone mode: {}",
            operator
        )))
    }

    fn execute_nd_recursion(
        &self,
        py: Python<'_>,
        seed: &Bound<'_, PyAny>,
        lambda: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        validate_nd_arity_standalone(py, lambda, 2, "~~")?;
        let _guard = NdNestingGuard::enter()?;
        // Wrap lambda in NDVmDecl so recur(v) calls lambda(v, recur)
        let decl = Py::new(
            py,
            crate::nd::NDVmDecl::with_memoize(lambda.clone().unbind(), self.nd_config.memoize),
        )
        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        let decl_bound = decl.bind(py);
        let result = lambda
            .call1((seed, decl_bound))
            .map(|r| r.unbind())
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()));
        // Clear abort flag after ND recursion completes
        crate::nd::clear_nd_abort();
        result
    }

    fn execute_nd_map(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        func: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        validate_nd_arity_standalone(py, func, 1, "~>")?;
        use crate::core::broadcast;
        broadcast::nd_map(py, data, func).map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    fn broadcast_nd_recursion(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        lambda: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        self.broadcast_nd(py, target, lambda, NdKind::Recursion)
    }

    fn broadcast_nd_map(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        func: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        self.broadcast_nd(py, target, func, NdKind::Map)
    }
}
