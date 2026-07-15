// FILE: catnip_rs/src/context.rs
//! Execution context for Catnip - core fields and scope operations.
//!
//! Python `Context` subclasses `ContextBase` to add wrappers (JIT, import, etc.).

use pyo3::PyTraverseError;
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet};

use crate::constants::*;
use crate::core::scope::Scope;

/// Core execution context exposed to Python.
///
/// Stores globals, locals (Scope), and runtime state.
/// Python `Context` inherits from this to add wrappers and builtins.
#[pyclass(name = "ContextBase", module = "catnip._rs", subclass, dict)]
pub struct ContextBase {
    #[pyo3(get, set)]
    pub(crate) globals: Py<PyDict>,

    #[pyo3(get)]
    pub(crate) locals: Py<Scope>,

    #[pyo3(get, set)]
    pub(crate) result: Option<Py<PyAny>>,

    #[pyo3(get, set)]
    pub(crate) pure_functions: Py<PySet>,

    // TCO
    #[pyo3(get, set)]
    pub(crate) tco_enabled: bool,

    // JIT
    #[pyo3(get, set)]
    pub(crate) jit_enabled: bool,
    #[pyo3(get, set)]
    pub(crate) jit_all: bool,
    #[pyo3(get, set)]
    pub(crate) jit_executor: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    pub(crate) jit_matcher: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    pub(crate) jit_codegen: Option<Py<PyAny>>,

    // ND-recursion
    #[pyo3(get, set)]
    pub(crate) nd_scheduler: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    pub(crate) nd_workers: i32,
    #[pyo3(get, set)]
    pub(crate) nd_mode: String,

    // Error context
    #[pyo3(get, set)]
    pub(crate) sourcemap: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    pub(crate) call_stack: Py<PyList>,

    // Logger and memoization (set by Python subclass)
    #[pyo3(get, set)]
    pub(crate) logger: Py<PyAny>,
    #[pyo3(get, set)]
    pub(crate) memoization: Py<PyAny>,

    // Module policy
    #[pyo3(get, set)]
    pub(crate) module_policy: Option<Py<PyAny>>,

    // Extensions
    #[pyo3(get, set)]
    pub(crate) _extensions: Py<PyDict>,

    /// Back-reference to the Registry (set from Python as `ctx._registry`). Held
    /// as a native field rather than an instance-`__dict__` attribute so it is
    /// reachable from `__traverse__`: a `__dict__` attribute would close a
    /// `Context <-> Registry` cycle that the collector cannot see (PyO3's manual
    /// `__traverse__` does not visit the instance dict), leaking every session.
    #[pyo3(get, set, name = "_registry")]
    pub(crate) registry_obj: Option<Py<PyAny>>,
}

#[pymethods]
impl ContextBase {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(
        py: Python<'_>,
        _args: &Bound<'_, pyo3::types::PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        Ok(Self {
            globals: PyDict::new(py).unbind(),
            locals: Py::new(py, Scope::new(py, None)?)?,
            result: None,
            pure_functions: PySet::empty(py)?.unbind(),
            tco_enabled: true,
            jit_enabled: false,
            jit_all: false,
            jit_executor: None,
            jit_matcher: None,
            jit_codegen: None,
            nd_scheduler: None,
            nd_workers: 0,
            nd_mode: "sequential".to_string(),
            sourcemap: None,
            call_stack: PyList::empty(py).unbind(),
            logger: py.None(),
            memoization: py.None(),
            module_policy: None,
            _extensions: PyDict::new(py).unbind(),
            registry_obj: None,
        })
    }

    /// Participate in CPython's cyclic GC. The context stores its wrappers
    /// (`_JitWrapper`, `_PureWrapper`, ...) in `globals`, and each wrapper holds
    /// `self.ctx` back -- a `Context <-> globals -> wrapper` cycle. `globals` is a
    /// native `ContextBase` field, invisible to the collector unless traversed, so
    /// without this every `Context` (hence every `Catnip()` session) leaks.
    /// Visiting all owned Python references lets the collector see the cycle;
    /// `__clear__` breaks it. (PyO3 visits the instance `__dict__`/`__weakref__`
    /// automatically.)
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.globals)?;
        visit.call(&self.locals)?;
        visit.call(&self.pure_functions)?;
        visit.call(&self.call_stack)?;
        visit.call(&self.logger)?;
        visit.call(&self.memoization)?;
        visit.call(&self._extensions)?;
        for v in [
            &self.result,
            &self.jit_executor,
            &self.jit_matcher,
            &self.jit_codegen,
            &self.nd_scheduler,
            &self.sourcemap,
            &self.module_policy,
            &self.registry_obj,
        ]
        .into_iter()
        .flatten()
        {
            visit.call(v)?;
        }
        Ok(())
    }

    /// Break the wrapper cycle by dropping the strong references that close it.
    /// Only called by the GC on an otherwise-unreachable context. Replacing
    /// `globals`/`_extensions` with fresh empty dicts severs `Context -> wrapper`
    /// (proven sufficient on its own); the `Option` fields are also cleared for
    /// robustness against other reference paths (jit/nd executors).
    fn __clear__(&mut self) {
        Python::attach(|py| {
            self.globals = PyDict::new(py).unbind();
            self._extensions = PyDict::new(py).unbind();
        });
        self.result = None;
        self.jit_executor = None;
        self.jit_matcher = None;
        self.jit_codegen = None;
        self.nd_scheduler = None;
        self.sourcemap = None;
        self.module_policy = None;
        self.registry_obj = None;
    }

    /// Push a new scope.
    #[pyo3(signature = (scope=None, parent=None))]
    fn push_scope(
        &self,
        py: Python<'_>,
        scope: Option<&Bound<'_, PyAny>>,
        parent: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let _ = parent; // reserved for future use
        let mut locals = self.locals.borrow_mut(py);
        locals.push_frame();

        if let Some(s) = scope {
            if let Ok(dict) = s.cast::<PyDict>() {
                for (k, v) in dict.iter() {
                    let name: String = k.extract()?;
                    locals.set(py, name, v.unbind());
                }
            } else if let Ok(symbols) = s.getattr("_symbols") {
                // Scope-like object
                let dict = symbols.cast::<PyDict>()?;
                for (k, v) in dict.iter() {
                    let name: String = k.extract()?;
                    locals.set(py, name, v.unbind());
                }
            }
        }
        Ok(())
    }

    /// Pop the top scope.
    fn pop_scope(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let mut locals = self.locals.borrow_mut(py);
        if locals.depth() > 1 {
            locals.pop_frame();
            Ok(None)
        } else {
            let exc_mod = py.import(PY_MOD_EXC)?;
            let exc_cls = exc_mod.getattr("CatnipWeirdError")?;
            Err(PyErr::from_value(exc_cls.call1(("Cannot pop the global scope.",))?))
        }
    }

    /// Capture current scope for closure creation.
    fn capture_scope(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let locals = self.locals.borrow(py);
        locals.snapshot(py)
    }

    /// Push a new scope with captured variables from a closure.
    fn push_scope_with_capture(&self, py: Python<'_>, captured: &Bound<'_, PyDict>) -> PyResult<()> {
        let mut locals = self.locals.borrow_mut(py);
        locals.push_frame_with_captures(py, captured)
    }

    /// Sync modified variables back to the captured dict.
    fn sync_captures(&self, py: Python<'_>, captured: &Bound<'_, PyDict>) -> PyResult<()> {
        let locals = self.locals.borrow(py);
        locals.sync_to_captures(py, captured)
    }

    /// Sync captures and pop scope in one operation.
    fn pop_scope_with_sync(&self, py: Python<'_>, captured: &Bound<'_, PyDict>) -> PyResult<Option<Py<PyAny>>> {
        // Save state before sync
        let before: Vec<(String, Py<PyAny>)> = captured
            .iter()
            .map(|(k, v)| Ok((k.extract::<String>()?, v.unbind())))
            .collect::<PyResult<Vec<_>>>()?;

        // Sync captures
        {
            let locals = self.locals.borrow(py);
            locals.sync_to_captures(py, captured)?;
        }

        // Pop scope
        let result = {
            let mut locals = self.locals.borrow_mut(py);
            if locals.depth() > 1 {
                locals.pop_frame();
                None
            } else {
                let exc_mod = py.import(PY_MOD_EXC)?;
                let exc_cls = exc_mod.getattr("CatnipWeirdError")?;
                return Err(PyErr::from_value(exc_cls.call1(("Cannot pop the global scope.",))?));
            }
        };

        // No propagation to the parent scope: a closure is a private mutable
        // snapshot (wip/CLOSURE_SEMANTICS.md, settled 2026-07-04) -- the sync
        // above persists the writes into the closure's OWN captured dict
        // (the canonical counter), and nothing leaks upward. Module globals
        // are not captured at all (live resolution, handled in Scope).
        let _ = before;

        Ok(result)
    }

    /// Mark a function as pure for JIT optimization.
    fn mark_pure(&self, py: Python<'_>, func: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Add name to pure_functions set
        if let Ok(name) = func.getattr("__name__") {
            let name_str: String = name.extract()?;
            self.pure_functions.bind(py).add(name_str)?;
        }

        // Mark CodeObject if available
        let code_obj = func.getattr("vm_code").ok().or_else(|| func.getattr("code").ok());

        if let Some(co) = code_obj {
            let _ = co.setattr("is_pure", true);
        }

        Ok(func.clone().unbind())
    }
}
