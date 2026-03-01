// FILE: catnip_rs/src/core/method.rs
//! Descriptor protocol for struct methods.
//!
//! CatnipMethod wraps a callable and implements __get__ to auto-bind `self`.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

/// Unbound method descriptor: wraps a function and implements __get__.
#[pyclass(module = "catnip._rs", name = "CatnipMethod")]
pub struct CatnipMethod {
    #[pyo3(get)]
    pub func: Py<PyAny>,
}

#[pymethods]
impl CatnipMethod {
    #[new]
    fn new(func: Py<PyAny>) -> Self {
        Self { func }
    }

    /// Descriptor __get__: return BoundCatnipMethod when accessed on instance.
    fn __get__<'py>(
        &self,
        py: Python<'py>,
        obj: &Bound<'py, PyAny>,
        _cls: &Bound<'py, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        if obj.is_none() {
            // Class-level access: return the raw function
            Ok(self.func.clone_ref(py))
        } else {
            // Instance access: return bound method
            let bound = Py::new(
                py,
                BoundCatnipMethod {
                    func: self.func.clone_ref(py),
                    instance: obj.clone().unbind(),
                    super_source_type: None,
                    native_instance_idx: None,
                },
            )?;
            Ok(bound.into_any())
        }
    }

    fn __repr__(&self) -> String {
        format!("<CatnipMethod>")
    }
}

/// Bound method: prepends instance as first argument on call.
#[pyclass(module = "catnip._rs", name = "BoundCatnipMethod")]
pub struct BoundCatnipMethod {
    #[pyo3(get)]
    pub func: Py<PyAny>,
    #[pyo3(get)]
    pub instance: Py<PyAny>,
    /// If this method was obtained via super, the type whose methods we resolved from.
    /// Used for super chain resolution (so B.super resolves to A, not back to B).
    pub super_source_type: Option<String>,
    /// Native struct instance index for VM round-trip (avoids CatnipStructProxy detour).
    pub native_instance_idx: Option<u32>,
}

#[pymethods]
impl BoundCatnipMethod {
    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        // Prepend self.instance to args
        let mut new_args: Vec<Py<PyAny>> = Vec::with_capacity(args.len() + 1);
        new_args.push(self.instance.clone_ref(py));
        for item in args.iter() {
            new_args.push(item.unbind());
        }
        let new_args_tuple = PyTuple::new(py, &new_args)?;
        self.func.call(py, &new_args_tuple, kwargs)
    }

    fn __repr__(&self) -> String {
        format!("<BoundCatnipMethod>")
    }
}
