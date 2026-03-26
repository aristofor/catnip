// FILE: catnip_rs/src/core/meta.rs
//! Dynamic namespace object for module metadata.
//!
//! Replaces Python's `types.SimpleNamespace` for the META global,
//! providing attribute-based storage with `resolve_exports()` for
//! validated module export resolution.

use pyo3::exceptions::{PyAttributeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};
use std::collections::HashMap;

/// Dynamic namespace for module metadata (META global).
#[pyclass(module = "catnip._rs", name = "CatnipMeta")]
#[derive(Debug)]
pub struct CatnipMeta {
    attrs: HashMap<String, Py<PyAny>>,
}

#[pymethods]
impl CatnipMeta {
    #[new]
    pub fn new() -> Self {
        Self { attrs: HashMap::new() }
    }

    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        match self.attrs.get(name) {
            Some(value) => Ok(value.clone_ref(py)),
            None => Err(PyAttributeError::new_err(format!(
                "'CatnipMeta' object has no attribute '{name}'"
            ))),
        }
    }

    fn __setattr__(&mut self, py: Python<'_>, name: &str, value: Py<PyAny>) {
        self.attrs.insert(name.to_string(), value.clone_ref(py));
    }

    fn __delattr__(&mut self, name: &str) -> PyResult<()> {
        match self.attrs.remove(name) {
            Some(_) => Ok(()),
            None => Err(PyAttributeError::new_err(format!(
                "'CatnipMeta' object has no attribute '{name}'"
            ))),
        }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let mut keys: Vec<&String> = self.attrs.keys().collect();
        keys.sort();

        let parts: Vec<String> = keys
            .iter()
            .map(|k| {
                let v = &self.attrs[*k];
                let repr = v
                    .bind(py)
                    .repr()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|_| "...".to_string());
                format!("{k}={repr}")
            })
            .collect();

        if parts.is_empty() {
            "CatnipMeta()".to_string()
        } else {
            format!("CatnipMeta({})", parts.join(", "))
        }
    }

    fn __dir__(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.attrs.keys().cloned().collect();
        keys.sort();
        keys
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(other_meta) = other.extract::<PyRef<CatnipMeta>>() {
            if self.attrs.len() != other_meta.attrs.len() {
                return Ok(false);
            }
            for (k, v) in &self.attrs {
                match other_meta.attrs.get(k) {
                    Some(ov) => {
                        let eq = v.bind(py).eq(ov.bind(py))?;
                        if !eq {
                            return Ok(false);
                        }
                    }
                    None => return Ok(false),
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn __bool__(&self) -> bool {
        true
    }

    /// Validate and resolve META.exports against a globals dict.
    ///
    /// Returns a dict of {name: value} if exports is set, or None if not.
    fn resolve_exports(&self, py: Python<'_>, globals: &Bound<'_, PyAny>) -> PyResult<Option<Py<PyDict>>> {
        let exports_val = match self.attrs.get("exports") {
            Some(v) => v.bind(py),
            None => return Ok(None),
        };

        // Validate type: must be list, tuple, or set
        if !exports_val.is_instance_of::<PyList>()
            && !exports_val.is_instance_of::<PyTuple>()
            && !exports_val.is_instance_of::<PySet>()
        {
            return Err(PyTypeError::new_err(
                "META.exports must be a list, tuple, or set of symbol names",
            ));
        }

        let result = PyDict::new(py);
        let iter = exports_val.try_iter()?;
        for item in iter {
            let item = item?;
            let name: String = item
                .extract()
                .map_err(|_| PyTypeError::new_err("META.exports entries must be strings"))?;
            // Support both dict and dict-like objects (GlobalsProxy)
            let value = globals.get_item(&name);
            match value {
                Ok(v) => {
                    result.set_item(&name, v)?;
                }
                Err(_) => {
                    return Err(PyAttributeError::new_err(format!(
                        "META.exports references unknown symbol: '{name}'"
                    )));
                }
            }
        }

        Ok(Some(result.unbind()))
    }
}

impl Default for CatnipMeta {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_is_empty() {
        let meta = CatnipMeta::new();
        assert!(meta.attrs.is_empty());
        assert!(meta.__bool__());
    }

    #[test]
    fn test_setattr_getattr() {
        Python::attach(|py| {
            let mut meta = CatnipMeta::new();
            let val = 42i64.into_pyobject(py).unwrap().into_any().unbind();
            meta.__setattr__(py, "x", val);
            let got = meta.__getattr__(py, "x").unwrap();
            let got_val: i64 = got.extract(py).unwrap();
            assert_eq!(got_val, 42);
        });
    }

    #[test]
    fn test_getattr_missing() {
        Python::attach(|py| {
            let meta = CatnipMeta::new();
            let err = meta.__getattr__(py, "nope");
            assert!(err.is_err());
        });
    }

    #[test]
    fn test_delattr() {
        Python::attach(|py| {
            let mut meta = CatnipMeta::new();
            let val = 1i64.into_pyobject(py).unwrap().into_any().unbind();
            meta.__setattr__(py, "x", val);
            assert!(meta.__delattr__("x").is_ok());
            assert!(meta.__getattr__(py, "x").is_err());
        });
    }

    #[test]
    fn test_delattr_missing() {
        let mut meta = CatnipMeta::new();
        assert!(meta.__delattr__("nope").is_err());
    }

    #[test]
    fn test_repr_empty() {
        Python::attach(|py| {
            let meta = CatnipMeta::new();
            assert_eq!(meta.__repr__(py), "CatnipMeta()");
        });
    }

    #[test]
    fn test_repr_sorted() {
        Python::attach(|py| {
            let mut meta = CatnipMeta::new();
            let v1 = 1i64.into_pyobject(py).unwrap().into_any().unbind();
            let v2 = 2i64.into_pyobject(py).unwrap().into_any().unbind();
            meta.__setattr__(py, "b", v1);
            meta.__setattr__(py, "a", v2);
            let repr = meta.__repr__(py);
            assert!(repr.starts_with("CatnipMeta(a="));
        });
    }

    #[test]
    fn test_dir() {
        Python::attach(|py| {
            let mut meta = CatnipMeta::new();
            let val = py.None();
            meta.__setattr__(py, "z", val.clone_ref(py));
            meta.__setattr__(py, "a", val);
            let d = meta.__dir__();
            assert_eq!(d, vec!["a".to_string(), "z".to_string()]);
        });
    }

    #[test]
    fn test_eq() {
        Python::attach(|py| {
            let mut m1 = CatnipMeta::new();
            let mut m2 = CatnipMeta::new();
            let v = 42i64.into_pyobject(py).unwrap().into_any().unbind();
            m1.__setattr__(py, "x", v.clone_ref(py));
            m2.__setattr__(py, "x", v);

            let m2_obj = Py::new(py, m2).unwrap();
            assert!(m1.__eq__(py, m2_obj.bind(py)).unwrap());
        });
    }

    #[test]
    fn test_resolve_exports_none() {
        Python::attach(|py| {
            let meta = CatnipMeta::new();
            let globals = PyDict::new(py);
            let result = meta.resolve_exports(py, &globals).unwrap();
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_resolve_exports_valid() {
        Python::attach(|py| {
            let meta = &mut CatnipMeta::new();
            let exports = PyList::new(py, ["foo", "bar"]).unwrap();
            meta.__setattr__(py, "exports", exports.into_any().unbind());

            let globals = PyDict::new(py);
            globals.set_item("foo", 1).unwrap();
            globals.set_item("bar", 2).unwrap();

            let result = meta.resolve_exports(py, &globals).unwrap().unwrap();
            let dict = result.bind(py);
            assert_eq!(dict.len(), 2);
            let foo: i64 = dict.get_item("foo").unwrap().unwrap().extract().unwrap();
            assert_eq!(foo, 1);
        });
    }

    #[test]
    fn test_resolve_exports_bad_type() {
        Python::attach(|py| {
            let meta = &mut CatnipMeta::new();
            let val = "not_a_list".into_pyobject(py).unwrap().into_any().unbind();
            meta.__setattr__(py, "exports", val);

            let globals = PyDict::new(py);
            let err = meta.resolve_exports(py, &globals);
            assert!(err.is_err());
        });
    }

    #[test]
    fn test_resolve_exports_missing_symbol() {
        Python::attach(|py| {
            let meta = &mut CatnipMeta::new();
            let exports = PyList::new(py, ["missing"]).unwrap();
            meta.__setattr__(py, "exports", exports.into_any().unbind());

            let globals = PyDict::new(py);
            let err = meta.resolve_exports(py, &globals);
            assert!(err.is_err());
        });
    }
}
