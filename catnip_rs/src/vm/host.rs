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
use crate::constants::PY_MOD_ND;
use indexmap::IndexMap;
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

    fn store_global(&self, py: Python<'_>, name: &str, value: Value) -> Result<(), super::core::VMError>;

    fn delete_global(&self, py: Python<'_>, name: &str) -> Result<(), super::core::VMError>;

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

    /// Get an attribute from a Python object. Includes "did you mean?" in error.
    fn obj_getattr(&self, py: Python<'_>, obj: Value, name: &str) -> Result<Value, super::core::VMError>;

    /// Set an attribute on a Python object.
    fn obj_setattr(&self, py: Python<'_>, obj: Value, name: &str, value: Value) -> Result<(), super::core::VMError>;

    /// Get an item from a collection (obj[key]).
    fn obj_getitem(&self, py: Python<'_>, obj: Value, key: Value) -> Result<Value, super::core::VMError>;

    /// Set an item in a collection (obj[key] = value).
    fn obj_setitem(&self, py: Python<'_>, obj: Value, key: Value, value: Value) -> Result<(), super::core::VMError>;

    // --- Membership test ---

    /// Test `item in container` via `operator.contains(container, item)`.
    fn contains_op(&self, py: Python<'_>, item: Value, container: Value) -> Result<Value, super::core::VMError>;

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
        if is_tuple {
            Ok(pyo3::types::PyTuple::new(py, result_list)
                .to_vm(py)?
                .into_any()
                .unbind())
        } else {
            Ok(result_list.into_any().unbind())
        }
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
        if is_tuple {
            Ok(pyo3::types::PyTuple::new(py, result_list)
                .to_vm(py)?
                .into_any()
                .unbind())
        } else {
            Ok(result_list.into_any().unbind())
        }
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

impl VmHost for ContextHost {
    #[inline]
    fn lookup_global(&self, py: Python<'_>, name: &str) -> Result<Option<Value>, super::core::VMError> {
        match self.ctx_globals {
            Some(ref g) => lookup_ctx_global(py, g, name),
            None => Ok(None),
        }
    }

    #[inline]
    fn store_global(&self, py: Python<'_>, name: &str, value: Value) -> Result<(), super::core::VMError> {
        if let Some(ref g) = self.ctx_globals {
            store_ctx_global(py, g, name, value)
        } else {
            Ok(())
        }
    }

    #[inline]
    fn delete_global(&self, py: Python<'_>, name: &str) -> Result<(), super::core::VMError> {
        if let Some(ref g) = self.ctx_globals {
            delete_ctx_global(py, g, name)
        } else {
            Ok(())
        }
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
    fn obj_getattr(&self, py: Python<'_>, obj: Value, name: &str) -> Result<Value, super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_bound = py_obj.bind(py);
        let result = py_bound.getattr(name).map_err(|e| {
            let msg = e.to_string();
            super::core::VMError::RuntimeError(super::core::py_attr_error_msg(py_bound, name, &msg))
        })?;
        Value::from_pyobject(py, &result).to_vm(py)
    }

    #[inline]
    fn obj_setattr(&self, py: Python<'_>, obj: Value, name: &str, value: Value) -> Result<(), super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_value = value.to_pyobject(py);
        py_obj
            .bind(py)
            .setattr(name, py_value)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    #[inline]
    fn obj_getitem(&self, py: Python<'_>, obj: Value, key: Value) -> Result<Value, super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_key = key.to_pyobject(py);
        let result = py_obj
            .bind(py)
            .get_item(py_key)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        Value::from_pyobject(py, &result).to_vm(py)
    }

    #[inline]
    fn obj_setitem(&self, py: Python<'_>, obj: Value, key: Value, value: Value) -> Result<(), super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_key = key.to_pyobject(py);
        let py_value = value.to_pyobject(py);
        py_obj
            .bind(py)
            .set_item(py_key, py_value)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    #[inline]
    fn contains_op(&self, py: Python<'_>, item: Value, container: Value) -> Result<Value, super::core::VMError> {
        let container_obj = container.to_pyobject(py);
        let item_obj = item.to_pyobject(py);
        // operator.contains(container, item) == (item in container)
        let result = self
            .cached_contains
            .bind(py)
            .call1((container_obj, item_obj))
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        let is_true = result
            .is_truthy()
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        Ok(Value::from_bool(is_true))
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
#[pyclass(unsendable)]
pub struct GlobalsProxy {
    globals: Globals,
}

impl GlobalsProxy {
    pub fn new(globals: Globals) -> Self {
        Self { globals }
    }
}

#[pymethods]
impl GlobalsProxy {
    fn __setitem__(&self, py: Python<'_>, key: &str, value: Bound<'_, PyAny>) -> PyResult<()> {
        let val = Value::from_pyobject(py, &value).map_err(pyo3::exceptions::PyValueError::new_err)?;
        self.globals.borrow_mut().insert(key.to_string(), val);
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
        if let Ok(dict) = other.cast::<pyo3::types::PyDict>() {
            let mut g = self.globals.borrow_mut();
            for (key, value) in dict.iter() {
                if let Ok(name) = key.extract::<String>() {
                    if let Ok(val) = Value::from_pyobject(py, &value) {
                        g.insert(name, val);
                    }
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ND arity validation (standalone)
// ---------------------------------------------------------------------------

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

    /// Build a standalone host with builtins injected into globals.
    pub fn new(py: Python<'_>) -> PyResult<Self> {
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

            // Decorators: jit/pure as identity, cached with basic memoization
            if let Ok(id_fn) = py.eval(c"lambda f: f", None, None) {
                if let Ok(val) = Value::from_pyobject(py, &id_fn) {
                    g.insert("jit".to_string(), val);
                    g.insert("pure".to_string(), val);
                }
            }
            if let Ok(memo_mod) = py.import("catnip.cachesys.memoization") {
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
            let injected = if let Ok(rs_mod) = py.import("catnip._rs") {
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

        // Inject builtin constant namespaces (ND, INT)
        {
            let nd = Py::new(py, crate::core::builtins::make_nd(py))?;
            if let Ok(val) = Value::from_pyobject(py, nd.bind(py)) {
                globals.borrow_mut().insert("ND".to_string(), val);
            }
            let int_ns = Py::new(py, crate::core::builtins::make_int(py))?;
            if let Ok(val) = Value::from_pyobject(py, int_ns.bind(py)) {
                globals.borrow_mut().insert("INT".to_string(), val);
            }
        }

        // Inject import() builtin via _ImportWrapper + GlobalsProxy
        {
            let proxy = GlobalsProxy {
                globals: Rc::clone(&globals),
            };
            let proxy_obj = Py::new(py, proxy)?;
            // Build a fake context with .globals pointing to the proxy
            let types = py.import("types")?;
            let ns = types.getattr("SimpleNamespace")?.call0()?;
            ns.setattr("globals", proxy_obj)?;
            let import_ns = PyDict::new(py);
            py.run(
                c"
def _parse_import_name(raw):
    if not isinstance(raw, str):
        from catnip.exc import CatnipTypeError
        raise CatnipTypeError(f'import name must be a string, got {type(raw).__name__}')
    if not raw:
        raise ValueError('import name cannot be empty')
    name, _, alias = raw.partition(':')
    if not name:
        raise ValueError(f\"empty name in import spec '{raw}'\")
    if _ and not alias:
        raise ValueError(f\"empty alias in import spec '{raw}'\")
    return (name, alias) if alias else (name, name)

class _StandaloneImportWrapper:
    def __init__(self, ctx):
        self.ctx = ctx
        self._loader = None

    def _get_loader(self):
        if self._loader is None:
            from catnip.loader import ModuleLoader
            self._loader = ModuleLoader(self.ctx)
        return self._loader

    def __call__(self, spec, *names, wild=False, protocol=None):
        from pathlib import Path

        caller_dir = None
        meta = self.ctx.globals.get('META')
        if meta is not None:
            try:
                caller_dir = Path(meta.file).parent
            except AttributeError:
                pass

        namespace = self._get_loader().import_module(spec, caller_dir=caller_dir, protocol=protocol)
        if names and wild:
            from catnip.exc import CatnipTypeError
            raise CatnipTypeError('cannot combine selective names with wild=True')
        if names:
            resolved = []
            for raw in names:
                name, alias = _parse_import_name(raw)
                if not hasattr(namespace, name):
                    raise AttributeError(f\"module '{spec}' has no attribute '{name}'\")
                resolved.append((alias, getattr(namespace, name)))
            for alias, value in resolved:
                self.ctx.globals[alias] = value
            return None
        if wild:
            for name in dir(namespace):
                if name.startswith('_') or name == 'META':
                    continue
                self.ctx.globals[name] = getattr(namespace, name)
            return None
        return namespace
",
                Some(&import_ns),
                None,
            )?;
            let wrapper_cls = import_ns
                .get_item("_StandaloneImportWrapper")?
                .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("missing import wrapper"))?;
            let import_fn = wrapper_cls.call1((ns,))?;
            if let Ok(val) = Value::from_pyobject(py, &import_fn) {
                globals.borrow_mut().insert("import".to_string(), val);
            }
        }

        Ok(Self {
            globals,
            ops,
            iter_fn,
            cached_contains,
            no_context: None,
            nd_config: NdConfig::default(),
            worker_pool: RefCell::new(None),
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

    /// Try to execute ND recursion via native Rust workers.
    /// Returns None if the lambda/captures/seeds aren't fully freezable (triggers Python fallback).
    fn try_native_nd_recursion(
        &self,
        py: Python<'_>,
        elements: &[Py<PyAny>],
        lambda: &Bound<'_, PyAny>,
    ) -> Result<Option<Vec<catnip_core::freeze::FrozenValue>>, super::core::VMError> {
        let lambda_data = match self.extract_freezable_lambda(py, lambda) {
            Some(v) => v,
            None => return Ok(None),
        };

        // Freeze all seed elements
        let seeds: Option<Vec<_>> = elements
            .iter()
            .map(|elem| {
                let val = Value::from_pyobject(py, elem.bind(py)).ok()?;
                crate::freeze::value_to_frozen(py, val)
            })
            .collect();
        let seeds = match seeds {
            Some(s) => s,
            None => return Ok(None),
        };

        self.submit_to_worker_pool(
            &lambda_data.encoded_ir,
            &lambda_data.captures,
            &lambda_data.param_names,
            &seeds,
        )
    }

    /// Try to execute ND map via native Rust workers.
    /// Returns None if not fully freezable.
    fn try_native_nd_map(
        &self,
        py: Python<'_>,
        elements: &[Py<PyAny>],
        func: &Bound<'_, PyAny>,
    ) -> Result<Option<Vec<catnip_core::freeze::FrozenValue>>, super::core::VMError> {
        let lambda_data = match self.extract_freezable_lambda(py, func) {
            Some(v) => v,
            None => return Ok(None),
        };

        let seeds: Option<Vec<_>> = elements
            .iter()
            .map(|elem| {
                let val = Value::from_pyobject(py, elem.bind(py)).ok()?;
                crate::freeze::value_to_frozen(py, val)
            })
            .collect();
        let seeds = match seeds {
            Some(s) => s,
            None => return Ok(None),
        };

        self.submit_to_worker_pool(
            &lambda_data.encoded_ir,
            &lambda_data.captures,
            &lambda_data.param_names,
            &seeds,
        )
    }

    /// Lazy-init worker pool and submit a batch. Returns None on any failure (triggers Python fallback).
    fn submit_to_worker_pool(
        &self,
        encoded_ir: &[u8],
        captures: &[(String, catnip_core::freeze::FrozenValue)],
        param_names: &[String],
        seeds: &[catnip_core::freeze::FrozenValue],
    ) -> Result<Option<Vec<catnip_core::freeze::FrozenValue>>, super::core::VMError> {
        let mut pool = self.worker_pool.borrow_mut();
        if pool.is_none() {
            match crate::nd::worker_pool::WorkerPool::new(crate::nd::worker_pool::default_pool_size()) {
                Ok(p) => *pool = Some(p),
                Err(_) => return Ok(None),
            }
        }
        let worker_pool = pool.as_mut().unwrap();
        match worker_pool.submit_batch(encoded_ir, captures, param_names, seeds) {
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
}

impl VmHost for VMHost {
    #[inline]
    fn lookup_global(&self, _py: Python<'_>, name: &str) -> Result<Option<Value>, super::core::VMError> {
        Ok(self.globals.borrow().get(name).copied())
    }

    #[inline]
    fn store_global(&self, _py: Python<'_>, name: &str, value: Value) -> Result<(), super::core::VMError> {
        self.globals.borrow_mut().insert(name.to_string(), value);
        Ok(())
    }

    #[inline]
    fn delete_global(&self, _py: Python<'_>, name: &str) -> Result<(), super::core::VMError> {
        self.globals.borrow_mut().swap_remove(name);
        Ok(())
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
    fn obj_getattr(&self, py: Python<'_>, obj: Value, name: &str) -> Result<Value, super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_bound = py_obj.bind(py);
        let result = py_bound.getattr(name).map_err(|e| {
            let msg = e.to_string();
            super::core::VMError::RuntimeError(super::core::py_attr_error_msg(py_bound, name, &msg))
        })?;
        Value::from_pyobject(py, &result).to_vm(py)
    }

    #[inline]
    fn obj_setattr(&self, py: Python<'_>, obj: Value, name: &str, value: Value) -> Result<(), super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_value = value.to_pyobject(py);
        py_obj
            .bind(py)
            .setattr(name, py_value)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    #[inline]
    fn obj_getitem(&self, py: Python<'_>, obj: Value, key: Value) -> Result<Value, super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_key = key.to_pyobject(py);
        let result = py_obj
            .bind(py)
            .get_item(py_key)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        Value::from_pyobject(py, &result).to_vm(py)
    }

    #[inline]
    fn obj_setitem(&self, py: Python<'_>, obj: Value, key: Value, value: Value) -> Result<(), super::core::VMError> {
        let py_obj = obj.to_pyobject(py);
        let py_key = key.to_pyobject(py);
        let py_value = value.to_pyobject(py);
        py_obj
            .bind(py)
            .set_item(py_key, py_value)
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    }

    #[inline]
    fn contains_op(&self, py: Python<'_>, item: Value, container: Value) -> Result<Value, super::core::VMError> {
        let container_obj = container.to_pyobject(py);
        let item_obj = item.to_pyobject(py);
        let result = self
            .cached_contains
            .bind(py)
            .call1((container_obj, item_obj))
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        let is_true = result
            .is_truthy()
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
        Ok(Value::from_bool(is_true))
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
        validate_nd_arity_standalone(py, lambda, 2, "~~")?;
        match self.nd_config.mode {
            NdMode::Sequential => {
                // Default sequential path
                let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let result_list = pyo3::types::PyList::empty(py);
                for elem_result in target.try_iter().to_vm(py)? {
                    let elem = elem_result.to_vm(py)?;
                    let nd_result = self.execute_nd_recursion(py, &elem, lambda)?;
                    result_list
                        .append(nd_result)
                        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                }
                if is_tuple {
                    Ok(pyo3::types::PyTuple::new(py, result_list)
                        .to_vm(py)?
                        .into_any()
                        .unbind())
                } else {
                    Ok(result_list.into_any().unbind())
                }
            }
            NdMode::Thread => {
                let _guard = NdNestingGuard::enter()?;

                // Collect elements with GIL
                let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let elements: Vec<Py<PyAny>> = target
                    .try_iter()
                    .to_vm(py)?
                    .map(|r| r.map(|e| e.unbind()))
                    .collect::<Result<Vec<_>, _>>()
                    .to_vm(py)?;

                let lambda_ref = lambda.clone().unbind();
                let memoize = self.nd_config.memoize;
                // Snapshot globals (Value is Copy, HashMap is Clone)
                let globals_snapshot = self.globals.borrow().clone();

                // Save parent thread-local pointers so worker threads can inherit them.
                // The parent VM is not executing during py.detach, so the pointers are stable.
                let parent_func_table = super::value::save_func_table() as usize;
                let parent_struct_registry = super::value::save_struct_registry() as usize;

                // map_with: each rayon thread gets its own HashMap clone,
                // wraps it in Rc<RefCell<_>> for set_vm_globals
                let results: Vec<Result<Py<PyAny>, String>> = py.detach(|| {
                    elements
                        .into_par_iter()
                        .map_with(globals_snapshot, |thread_map, elem| {
                            Python::attach(|py| {
                                let thread_globals: Globals = Rc::new(RefCell::new(thread_map.clone()));
                                set_vm_globals(thread_globals);

                                // Inherit parent's func_table and struct_registry for VmFunc resolution
                                if parent_func_table != 0 {
                                    super::value::set_func_table(
                                        parent_func_table as *const super::value::FunctionTable,
                                    );
                                }
                                if parent_struct_registry != 0 {
                                    super::value::set_struct_registry(
                                        parent_struct_registry as *const super::structs::StructRegistry,
                                    );
                                }

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

                // Collect results back with GIL
                let result_list = pyo3::types::PyList::empty(py);
                for r in results {
                    let val = r.map_err(super::core::VMError::RuntimeError)?;
                    result_list
                        .append(val)
                        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                }
                if is_tuple {
                    Ok(pyo3::types::PyTuple::new(py, result_list)
                        .to_vm(py)?
                        .into_any()
                        .unbind())
                } else {
                    Ok(result_list.into_any().unbind())
                }
            }
            NdMode::Process => {
                let _guard = NdNestingGuard::enter()?;

                let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let elements: Vec<Py<PyAny>> = target
                    .try_iter()
                    .to_vm(py)?
                    .map(|r| r.map(|e| e.unbind()))
                    .collect::<Result<Vec<_>, _>>()
                    .to_vm(py)?;

                // Try native Rust worker path
                if let Some(results) = self.try_native_nd_recursion(py, &elements, lambda)? {
                    let result_list = pyo3::types::PyList::empty(py);
                    for frozen in &results {
                        let val = crate::freeze::frozen_to_value(py, frozen);
                        let py_obj = val.to_pyobject(py);
                        result_list
                            .append(py_obj)
                            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                    }
                    if is_tuple {
                        return Ok(pyo3::types::PyTuple::new(py, result_list)
                            .to_vm(py)?
                            .into_any()
                            .unbind());
                    } else {
                        return Ok(result_list.into_any().unbind());
                    }
                }

                // Fallback: Python ProcessPoolExecutor
                let map_err = |e: pyo3::PyErr| super::core::VMError::RuntimeError(e.to_string());

                let nd_module = py.import(PY_MOD_ND).map_err(map_err)?;
                let worker_fn = nd_module.getattr("_worker_execute_simple").map_err(map_err)?;
                let worker_init = nd_module.getattr("_worker_init").map_err(map_err)?;

                let mp = py.import("multiprocessing").map_err(map_err)?;
                let mp_context = mp.call_method1("get_context", ("spawn",)).map_err(map_err)?;
                let cf = py.import("concurrent.futures").map_err(map_err)?;
                let kwargs = pyo3::types::PyDict::new(py);
                kwargs.set_item("mp_context", &mp_context).map_err(map_err)?;
                kwargs.set_item("initializer", &worker_init).map_err(map_err)?;
                let executor = cf
                    .getattr("ProcessPoolExecutor")
                    .and_then(|cls| cls.call((), Some(&kwargs)))
                    .map_err(map_err)?;

                let futures: Vec<Bound<'_, PyAny>> = elements
                    .iter()
                    .map(|elem| {
                        executor
                            .call_method1("submit", (&worker_fn, elem, lambda))
                            .map_err(map_err)
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let result_list = pyo3::types::PyList::empty(py);
                for future in &futures {
                    let r = future.call_method0("result").map_err(map_err)?;
                    result_list.append(r).map_err(map_err)?;
                }
                let _ = executor.call_method1("shutdown", (false,));

                if is_tuple {
                    Ok(pyo3::types::PyTuple::new(py, result_list)
                        .to_vm(py)?
                        .into_any()
                        .unbind())
                } else {
                    Ok(result_list.into_any().unbind())
                }
            }
        }
    }

    fn broadcast_nd_map(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        func: &Bound<'_, PyAny>,
    ) -> Result<Py<PyAny>, super::core::VMError> {
        validate_nd_arity_standalone(py, func, 1, "~>")?;
        match self.nd_config.mode {
            NdMode::Sequential => {
                let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let result_list = pyo3::types::PyList::empty(py);
                for elem_result in target.try_iter().to_vm(py)? {
                    let elem = elem_result.to_vm(py)?;
                    let nd_result = self.execute_nd_map(py, &elem, func)?;
                    result_list
                        .append(nd_result)
                        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                }
                if is_tuple {
                    Ok(pyo3::types::PyTuple::new(py, result_list)
                        .to_vm(py)?
                        .into_any()
                        .unbind())
                } else {
                    Ok(result_list.into_any().unbind())
                }
            }
            NdMode::Thread => {
                // Collect elements with GIL
                let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let elements: Vec<Py<PyAny>> = target
                    .try_iter()
                    .to_vm(py)?
                    .map(|r| r.map(|e| e.unbind()))
                    .collect::<Result<Vec<_>, _>>()
                    .to_vm(py)?;

                let func_ref = func.clone().unbind();
                let globals_snapshot = self.globals.borrow().clone();
                let parent_func_table = super::value::save_func_table() as usize;
                let parent_struct_registry = super::value::save_struct_registry() as usize;

                let results: Vec<Result<Py<PyAny>, String>> = py.detach(|| {
                    elements
                        .into_par_iter()
                        .map_with(globals_snapshot, |thread_map, elem| {
                            Python::attach(|py| {
                                let thread_globals: Globals = Rc::new(RefCell::new(thread_map.clone()));
                                set_vm_globals(thread_globals);
                                if parent_func_table != 0 {
                                    super::value::set_func_table(
                                        parent_func_table as *const super::value::FunctionTable,
                                    );
                                }
                                if parent_struct_registry != 0 {
                                    super::value::set_struct_registry(
                                        parent_struct_registry as *const super::structs::StructRegistry,
                                    );
                                }
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
                if is_tuple {
                    Ok(pyo3::types::PyTuple::new(py, result_list)
                        .to_vm(py)?
                        .into_any()
                        .unbind())
                } else {
                    Ok(result_list.into_any().unbind())
                }
            }
            NdMode::Process => {
                let is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let elements: Vec<Py<PyAny>> = target
                    .try_iter()
                    .to_vm(py)?
                    .map(|r| r.map(|e| e.unbind()))
                    .collect::<Result<Vec<_>, _>>()
                    .to_vm(py)?;

                // Try native Rust worker path (nd_map: 1 param, no recur)
                if let Some(results) = self.try_native_nd_map(py, &elements, func)? {
                    let result_list = pyo3::types::PyList::empty(py);
                    for frozen in &results {
                        let val = crate::freeze::frozen_to_value(py, frozen);
                        let py_obj = val.to_pyobject(py);
                        result_list
                            .append(py_obj)
                            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                    }
                    if is_tuple {
                        return Ok(pyo3::types::PyTuple::new(py, result_list)
                            .to_vm(py)?
                            .into_any()
                            .unbind());
                    } else {
                        return Ok(result_list.into_any().unbind());
                    }
                }

                // Fallback: Python ProcessPoolExecutor
                let map_err = |e: pyo3::PyErr| super::core::VMError::RuntimeError(e.to_string());
                let mp = py.import("multiprocessing").map_err(map_err)?;
                let mp_context = mp.call_method1("get_context", ("spawn",)).map_err(map_err)?;
                let nd_module = py.import(PY_MOD_ND).map_err(map_err)?;
                let worker_init = nd_module.getattr("_worker_init").map_err(map_err)?;
                let cf = py.import("concurrent.futures").map_err(map_err)?;
                let kwargs = pyo3::types::PyDict::new(py);
                kwargs.set_item("mp_context", &mp_context).map_err(map_err)?;
                kwargs.set_item("initializer", &worker_init).map_err(map_err)?;
                let executor = cf
                    .getattr("ProcessPoolExecutor")
                    .and_then(|cls| cls.call((), Some(&kwargs)))
                    .map_err(map_err)?;

                let futures: Vec<Bound<'_, PyAny>> = elements
                    .iter()
                    .map(|elem| {
                        executor
                            .call_method1("submit", (func, elem))
                            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let result_list = pyo3::types::PyList::empty(py);
                for future in &futures {
                    let r = future
                        .call_method0("result")
                        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                    result_list
                        .append(r)
                        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
                }

                let _ = executor
                    .call_method1("shutdown", (false,))
                    .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;

                if is_tuple {
                    Ok(pyo3::types::PyTuple::new(py, result_list)
                        .to_vm(py)?
                        .into_any()
                        .unbind())
                } else {
                    Ok(result_list.into_any().unbind())
                }
            }
        }
    }
}
