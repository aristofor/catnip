// FILE: catnip_rs/src/context.rs
//! Execution context for Catnip - core fields and scope operations.
//!
//! Python `Context` subclasses `ContextBase` to add wrappers (JIT, import, etc.).

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
    pub(crate) jit_detector: Option<Py<PyAny>>,
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
            jit_detector: None,
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
        })
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

        // Propagate modified captures to parent scope
        for (name, old_value) in &before {
            if let Some(new_value) = captured.get_item(name)? {
                let changed = !new_value.is(old_value.bind(py));
                if changed {
                    let mut locals = self.locals.borrow_mut(py);
                    locals.set(py, name.clone(), new_value.unbind());
                }
            }
        }

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

    /// Lazy initialization of JIT subsystem.
    fn _init_jit(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.jit_detector.is_none() {
            match py.import(PY_MOD_JIT) {
                Ok(jit_mod) => {
                    let detector_cls = jit_mod.getattr("HotLoopDetector")?;
                    let detector = detector_cls.call1((crate::constants::JIT_THRESHOLD_DEFAULT,))?;
                    self.jit_detector = Some(detector.unbind());
                }
                Err(_) => {
                    // JIT not available
                    self.jit_enabled = false;
                }
            }
        }
        Ok(())
    }
}
