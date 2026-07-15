// FILE: catnip_rs/src/runtime.rs
//! Runtime introspection for Catnip - exposed as `catnip` builtin.

use crate::pragma::PragmaContext;
use pyo3::PyTraverseError;
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::PyList;

/// Runtime introspection object exposed as `catnip` builtin in Catnip code.
///
/// Reads pragma state directly from Rust (no Python intermediary).
#[pyclass(module = "catnip._rs", name = "CatnipRuntime")]
pub struct CatnipRuntime {
    pragma_context: Option<Py<PragmaContext>>,
    context: Option<Py<PyAny>>,
    modules: Vec<String>,
}

#[pymethods]
impl CatnipRuntime {
    #[new]
    #[pyo3(signature = (pragma_context=None))]
    fn new(pragma_context: Option<Py<PragmaContext>>) -> Self {
        Self {
            pragma_context,
            context: None,
            modules: Vec::new(),
        }
    }

    /// Participate in CPython's cyclic GC. `RUNTIME` is injected into the
    /// context's globals (`globals['catnip']`) and holds the context back via
    /// `_set_context`, closing a `Context <-> CatnipRuntime` cycle the collector
    /// cannot see (a Rust pyclass is opaque to it). Visiting the owned references
    /// lets the collector detect the cycle; `__clear__` breaks it. Without this,
    /// every `Catnip()` session leaks its context.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Some(ref pc) = self.pragma_context {
            visit.call(pc)?;
        }
        if let Some(ref ctx) = self.context {
            visit.call(ctx)?;
        }
        Ok(())
    }

    /// Break the `Context <-> CatnipRuntime` cycle by dropping the strong
    /// references. Only called by the GC on an otherwise-unreachable object.
    fn __clear__(&mut self) {
        self.pragma_context = None;
        self.context = None;
    }

    /// Set the pragma context (called after Catnip init).
    fn _set_pragma_context(&mut self, pc: Py<PragmaContext>) {
        self.pragma_context = Some(pc);
    }

    /// Set the execution context (called after Catnip init).
    fn _set_context(&mut self, ctx: Py<PyAny>) {
        self.context = Some(ctx);
    }

    /// Register a loaded module.
    fn _add_module(&mut self, name: String) {
        if !self.modules.contains(&name) {
            self.modules.push(name);
        }
    }

    #[getter]
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    #[getter]
    fn tco(&self, py: Python<'_>) -> bool {
        match &self.pragma_context {
            Some(pc) => pc.borrow(py).tco_enabled,
            None => true,
        }
    }

    #[getter]
    fn optimize(&self, py: Python<'_>) -> i32 {
        match &self.pragma_context {
            Some(pc) => pc.borrow(py).optimize_level,
            None => 3,
        }
    }

    #[getter]
    fn debug(&self, py: Python<'_>) -> bool {
        match &self.pragma_context {
            Some(pc) => pc.borrow(py).debug_mode,
            None => false,
        }
    }

    #[getter]
    fn jit(&self, py: Python<'_>) -> bool {
        match &self.pragma_context {
            Some(pc) => pc.borrow(py).jit_enabled,
            None => true,
        }
    }

    #[getter]
    fn cache(&self, py: Python<'_>) -> bool {
        match &self.pragma_context {
            Some(pc) => pc.borrow(py).cache_enabled,
            None => true,
        }
    }

    #[getter]
    fn modules(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let list = PyList::new(py, &self.modules)?;
        Ok(list.into())
    }

    #[getter]
    fn extensions(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.context {
            Some(ctx) => {
                let ctx_bound = ctx.bind(py);
                let ext_dict = ctx_bound.getattr("_extensions")?;
                let values = ext_dict.call_method0("values")?;
                let mut result: Vec<Py<PyAny>> = Vec::new();
                for info in values.try_iter()? {
                    let info = info?;
                    let dict = info.call_method0("to_dict")?;
                    result.push(dict.unbind());
                }
                Ok(PyList::new(py, result)?.into_any().unbind())
            }
            None => Ok(PyList::empty(py).into_any().unbind()),
        }
    }

    fn __repr__(&self) -> &'static str {
        "<CatnipRuntime>"
    }
}
