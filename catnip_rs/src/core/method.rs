// FILE: catnip_rs/src/core/method.rs
//! Descriptor protocol for struct methods.
//!
//! CatnipMethod wraps a callable and implements __get__ to auto-bind `self`.

use pyo3::PyTraverseError;
use pyo3::gc::PyVisit;
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
    fn __get__<'py>(&self, py: Python<'py>, obj: &Bound<'py, PyAny>, _cls: &Bound<'py, PyAny>) -> PyResult<Py<PyAny>> {
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
                    native_registry_id: 0,
                },
            )?;
            Ok(bound.into_any())
        }
    }

    fn __repr__(&self) -> String {
        "<CatnipMethod>".to_string()
    }

    /// Participate in CPython's cyclic GC. `func` may reach the execution
    /// context (a `VMFunction` carrying it), so an unbound descriptor caught in
    /// a reference cycle would otherwise pin the context through this opaque
    /// Rust pyclass -- the same leak `BoundCatnipMethod` guards against.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.func)?;
        Ok(())
    }

    /// Break the cycle by dropping the strong reference reported by
    /// `__traverse__`. Only called by the GC on an otherwise-unreachable object.
    fn __clear__(&mut self) {
        Python::attach(|py| {
            self.func = py.None();
        });
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
    /// Identity of the registry owning `native_instance_idx` (0 if none).
    /// Carried so a proxy rebuilt from this method reaches its own registry.
    pub native_registry_id: u64,
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
        "<BoundCatnipMethod>".to_string()
    }

    /// Participate in CPython's cyclic GC. A bound method bound to a module-level
    /// name holds `func` (a `VMFunction` whose `context` is the execution
    /// context) and `instance` (a struct proxy reaching the type and thus the
    /// context back) -- a `ctx.globals -> BoundCatnipMethod -> ctx` cycle the
    /// collector cannot see (a Rust pyclass is opaque to it). Without this,
    /// `m = P(1).f` leaks its context every session.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.func)?;
        visit.call(&self.instance)?;
        Ok(())
    }

    /// Break the cycle by dropping the strong references reported by
    /// `__traverse__`. Only called by the GC on an otherwise-unreachable method.
    fn __clear__(&mut self) {
        Python::attach(|py| {
            self.func = py.None();
            self.instance = py.None();
        });
    }
}
