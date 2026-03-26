// FILE: catnip_rs/src/core/builtins.rs
//! Builtin constant namespaces (ND, INT, ...).
//!
//! Frozen namespaces injected into globals, following the META convention.
//! Prepares the ground for future enum types.

use pyo3::exceptions::PyAttributeError;
use pyo3::prelude::*;
use std::collections::HashMap;

/// Read-only namespace for builtin constants.
#[pyclass(module = "catnip._rs", name = "FrozenNamespace", frozen)]
#[derive(Debug)]
pub struct FrozenNamespace {
    name: String,
    attrs: HashMap<String, Py<PyAny>>,
}

impl FrozenNamespace {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            attrs: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: Py<PyAny>) {
        self.attrs.insert(key.to_string(), value);
    }
}

#[pymethods]
impl FrozenNamespace {
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        match self.attrs.get(name) {
            Some(value) => Ok(value.clone_ref(py)),
            None => Err(PyAttributeError::new_err(format!(
                "'{}' has no attribute '{name}'",
                self.name
            ))),
        }
    }

    fn __repr__(&self) -> String {
        let mut keys: Vec<&String> = self.attrs.keys().collect();
        keys.sort();
        let parts: Vec<String> = keys.iter().map(|k| format!("{}.{k}", self.name)).collect();
        format!("{}({})", self.name, parts.join(", "))
    }

    fn __dir__(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.attrs.keys().cloned().collect();
        keys.sort();
        keys
    }
}

/// Build the ND namespace: ND.sequential, ND.thread, ND.process
pub fn make_nd(py: Python<'_>) -> FrozenNamespace {
    let mut ns = FrozenNamespace::new("ND");
    ns.set(
        "sequential",
        "sequential".into_pyobject(py).unwrap().unbind().into_any(),
    );
    ns.set("thread", "thread".into_pyobject(py).unwrap().unbind().into_any());
    ns.set("process", "process".into_pyobject(py).unwrap().unbind().into_any());
    ns
}

/// Build the INT namespace: INT.max, INT.min
pub fn make_int(py: Python<'_>) -> FrozenNamespace {
    use catnip_core::nanbox::{SMALLINT_MAX, SMALLINT_MIN};
    let max: i64 = SMALLINT_MAX;
    let min: i64 = SMALLINT_MIN;
    let mut ns = FrozenNamespace::new("INT");
    ns.set("max", max.into_pyobject(py).unwrap().unbind().into_any());
    ns.set("min", min.into_pyobject(py).unwrap().unbind().into_any());
    ns
}

// Python-callable constructors
#[pyfunction]
pub fn build_nd(py: Python<'_>) -> FrozenNamespace {
    make_nd(py)
}

#[pyfunction]
pub fn build_int(py: Python<'_>) -> FrozenNamespace {
    make_int(py)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::Python;

    #[test]
    fn test_nd_namespace() {
        Python::attach(|py| {
            let nd = make_nd(py);
            assert_eq!(nd.attrs.len(), 3);
            let val = nd.__getattr__(py, "thread").unwrap();
            assert_eq!(val.extract::<String>(py).unwrap(), "thread");
            assert!(nd.__getattr__(py, "nonexistent").is_err());
        });
    }

    #[test]
    fn test_int_namespace() {
        Python::attach(|py| {
            let int_ns = make_int(py);
            assert_eq!(int_ns.attrs.len(), 2);
            let max = int_ns.__getattr__(py, "max").unwrap();
            assert_eq!(max.extract::<i64>(py).unwrap(), (1_i64 << 46) - 1);
            let min = int_ns.__getattr__(py, "min").unwrap();
            assert_eq!(min.extract::<i64>(py).unwrap(), -(1_i64 << 46));
        });
    }

    #[test]
    fn test_dir() {
        Python::attach(|py| {
            let nd = make_nd(py);
            let keys = nd.__dir__();
            assert_eq!(keys, vec!["process", "sequential", "thread"]);
        });
    }

    #[test]
    fn test_repr() {
        Python::attach(|py| {
            let nd = make_nd(py);
            let repr = nd.__repr__();
            assert!(repr.starts_with("ND("));
            assert!(repr.contains("ND.thread"));
        });
    }
}
