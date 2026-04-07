// FILE: catnip_rs/src/loader/namespace.rs
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use std::collections::HashMap;

/// Module namespace wrapping public attributes for `module.attr` access.
#[pyclass(name = "ModuleNamespace", module = "catnip._rs")]
pub struct ModuleNamespace {
    name: String,
    attrs: HashMap<String, Py<PyAny>>,
}

#[pymethods]
impl ModuleNamespace {
    #[new]
    #[pyo3(signature = (name))]
    pub fn new(name: String) -> Self {
        Self {
            name,
            attrs: HashMap::new(),
        }
    }

    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        self.attrs.get(name).map(|v| v.clone_ref(py)).ok_or_else(|| {
            pyo3::exceptions::PyAttributeError::new_err(format!("Module '{}' has no attribute '{}'", self.name, name))
        })
    }

    fn __dir__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let mut keys: Vec<&str> = self.attrs.keys().map(|s| s.as_str()).collect();
        keys.sort_unstable();
        PyList::new(py, &keys)
    }

    fn __repr__(&self) -> String {
        let mut items: Vec<&str> = self.attrs.keys().map(|s| s.as_str()).collect();
        items.sort_unstable();
        let display = if items.len() > 10 {
            let head: Vec<&str> = items[..10].to_vec();
            format!("{}, ... ({} total)", head.join(", "), items.len())
        } else {
            items.join(", ")
        };
        format!("<ModuleNamespace '{}' [{}]>", self.name, display)
    }
}

impl ModuleNamespace {
    /// Build from a Python module object, extracting public attributes.
    pub fn from_pymodule(_py: Python<'_>, module: &Bound<'_, PyModule>, name: Option<&str>) -> PyResult<Self> {
        let mod_name = match name {
            Some(n) => n.to_string(),
            None => module.name()?.to_string(),
        };
        let mut attrs = HashMap::new();

        let dir = module.dir()?;
        for item in dir.iter() {
            let attr_name: String = item.extract()?;
            if attr_name.starts_with('_') {
                continue;
            }
            if let Ok(val) = module.getattr(attr_name.as_str()) {
                attrs.insert(attr_name, val.unbind());
            }
        }

        Ok(Self { name: mod_name, attrs })
    }

    /// Build from a Python object with dir()/getattr (SimpleNamespace, etc).
    pub fn from_pyobject(_py: Python<'_>, obj: &Bound<'_, PyAny>, name: &str) -> PyResult<Self> {
        let mut attrs = HashMap::new();

        let dir: Bound<'_, PyList> = obj.dir()?.cast_into()?;
        for item in dir.iter() {
            let attr_name: String = item.extract()?;
            if attr_name.starts_with('_') {
                continue;
            }
            if let Ok(val) = obj.getattr(attr_name.as_str()) {
                attrs.insert(attr_name, val.unbind());
            }
        }

        Ok(Self {
            name: name.to_string(),
            attrs,
        })
    }

    /// Build from a Python dict of exports.
    pub fn from_dict(_py: Python<'_>, dict: &Bound<'_, PyDict>, name: &str) -> PyResult<Self> {
        let mut attrs = HashMap::new();

        for (key, val) in dict.iter() {
            let key_str: String = key.extract()?;
            if !key_str.starts_with('_') {
                attrs.insert(key_str, val.unbind());
            }
        }

        Ok(Self {
            name: name.to_string(),
            attrs,
        })
    }

    /// Insert an attribute.
    pub fn set_attr(&mut self, name: String, value: Py<PyAny>) {
        self.attrs.insert(name, value);
    }

    /// Get module name.
    pub fn name(&self) -> &str {
        &self.name
    }
}
