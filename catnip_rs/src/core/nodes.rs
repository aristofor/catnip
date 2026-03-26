// FILE: catnip_rs/src/core/nodes.rs
//! AST node structures for Catnip.

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Represents a reference to an identifier in the AST.
#[pyclass(module = "catnip._rs", name = "Ref", from_py_object)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ref {
    #[pyo3(get)]
    pub ident: String,
    #[pyo3(get)]
    pub start_byte: isize,
    #[pyo3(get)]
    pub end_byte: isize,
}

#[pymethods]
impl Ref {
    #[new]
    #[pyo3(signature = (ident=String::new(), start_byte=-1, end_byte=-1))]
    fn new(ident: String, start_byte: isize, end_byte: isize) -> Self {
        Self {
            ident,
            start_byte,
            end_byte,
        }
    }

    fn __repr__(&self) -> String {
        format!("<Ref {}>", self.ident)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(other_ref) = other.extract::<PyRef<Ref>>() {
            Ok(self.ident == other_ref.ident)
        } else {
            Ok(false)
        }
    }

    fn __hash__(&self) -> isize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        self.ident.hash(&mut hasher);
        hasher.finish() as isize
    }

    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("ident", &self.ident)?;
        state.set_item("start_byte", self.start_byte)?;
        state.set_item("end_byte", self.end_byte)?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.ident = state.get_item("ident")?.unwrap().extract()?;
        self.start_byte = state.get_item("start_byte")?.unwrap().extract()?;
        self.end_byte = state.get_item("end_byte")?.unwrap().extract()?;
        Ok(())
    }
}

/// Signal value to trigger a tail-call jump.
///
/// Instead of executing a function call normally (which would push a new scope),
/// return a TailCall object that signals the trampoline loop to rebind parameters
/// in the current scope and jump to the function body.
///
/// This enables O(1) stack space for tail-recursive functions.
#[pyclass(module = "catnip._rs", name = "TailCall")]
pub struct TailCall {
    #[pyo3(get)]
    pub func: Py<PyAny>,
    #[pyo3(get)]
    pub args: Py<PyAny>,
    #[pyo3(get)]
    pub kwargs: Py<PyAny>,
}

#[pymethods]
impl TailCall {
    #[new]
    #[pyo3(signature = (func=None, args=None, kwargs=None))]
    fn new(py: Python<'_>, func: Option<Py<PyAny>>, args: Option<Py<PyAny>>, kwargs: Option<Py<PyAny>>) -> Self {
        let func = func.unwrap_or_else(|| py.None());
        let args = args.unwrap_or_else(|| py.None());
        let kwargs = kwargs.unwrap_or_else(|| py.None());
        Self { func, args, kwargs }
    }

    fn __repr__(&self) -> PyResult<String> {
        Python::attach(|py| Ok(format!("<TailCall {}>", self.func.bind(py).repr()?)))
    }

    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("func", self.func.clone_ref(py))?;
        state.set_item("args", self.args.clone_ref(py))?;
        state.set_item("kwargs", self.kwargs.clone_ref(py))?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.func = state.get_item("func")?.unwrap().unbind();
        self.args = state.get_item("args")?.unwrap().unbind();
        self.kwargs = state.get_item("kwargs")?.unwrap().unbind();
        Ok(())
    }
}
