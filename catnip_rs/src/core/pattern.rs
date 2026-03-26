// FILE: catnip_rs/src/core/pattern.rs
//! Pattern matching structures for Catnip match expressions.

use pyo3::prelude::*;
use pyo3::types::PyDict;

// Tag constants for fast dispatch (pointer-based type check instead of string comparison)
pub const TAG_WILDCARD: u8 = 0;
pub const TAG_LITERAL: u8 = 1;
pub const TAG_VAR: u8 = 2;
pub const TAG_OR: u8 = 3;
pub const TAG_TUPLE: u8 = 4;
pub const TAG_STRUCT: u8 = 5;

/// Extract pattern tag via downcast (pointer comparison, no string)
pub fn get_pattern_tag(pattern: &Bound<'_, PyAny>) -> Option<u8> {
    if pattern.cast::<PatternWildcard>().is_ok() {
        Some(TAG_WILDCARD)
    } else if pattern.cast::<PatternLiteral>().is_ok() {
        Some(TAG_LITERAL)
    } else if pattern.cast::<PatternVar>().is_ok() {
        Some(TAG_VAR)
    } else if pattern.cast::<PatternOr>().is_ok() {
        Some(TAG_OR)
    } else if pattern.cast::<PatternTuple>().is_ok() {
        Some(TAG_TUPLE)
    } else if pattern.cast::<PatternStruct>().is_ok() {
        Some(TAG_STRUCT)
    } else {
        None
    }
}

/// Pattern that matches a literal value.
#[pyclass(module = "catnip._rs", name = "PatternLiteral")]
pub struct PatternLiteral {
    #[pyo3(get)]
    pub value: Py<PyAny>,
}

#[pymethods]
impl PatternLiteral {
    #[new]
    #[pyo3(signature = (value=None))]
    fn new(py: Python<'_>, value: Option<Py<PyAny>>) -> Self {
        let value = value.unwrap_or_else(|| py.None());
        Self { value }
    }

    fn __repr__(&self) -> PyResult<String> {
        Python::attach(|py| {
            let value_repr = self.value.bind(py).repr()?;
            Ok(format!("<PatternLiteral {}>", value_repr))
        })
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Python::attach(|py| {
            if let Ok(other_pattern) = other.extract::<PyRef<PatternLiteral>>() {
                let eq = self.value.bind(py).eq(other_pattern.value.bind(py))?;
                Ok(eq)
            } else {
                Ok(false)
            }
        })
    }

    fn __hash__(&self) -> PyResult<isize> {
        Python::attach(|py| self.value.bind(py).hash())
    }

    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("value", self.value.clone_ref(py))?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.value = state.get_item("value")?.unwrap().unbind();
        Ok(())
    }

    fn pattern_tag(&self) -> u8 {
        TAG_LITERAL
    }
}

/// Pattern that captures the matched value into a variable.
#[pyclass(module = "catnip._rs", name = "PatternVar", from_py_object)]
#[derive(Clone)]
pub struct PatternVar {
    #[pyo3(get)]
    pub name: String,
}

#[pymethods]
impl PatternVar {
    #[new]
    #[pyo3(signature = (name=String::new()))]
    fn new(name: String) -> Self {
        Self { name }
    }

    fn __repr__(&self) -> String {
        format!("<PatternVar {}>", self.name)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(other_pattern) = other.extract::<PyRef<PatternVar>>() {
            Ok(self.name == other_pattern.name)
        } else {
            Ok(false)
        }
    }

    fn __hash__(&self) -> isize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        self.name.hash(&mut hasher);
        hasher.finish() as isize
    }

    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("name", &self.name)?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.name = state.get_item("name")?.unwrap().extract()?;
        Ok(())
    }

    fn pattern_tag(&self) -> u8 {
        TAG_VAR
    }
}

/// Pattern that matches anything without capturing.
#[pyclass(module = "catnip._rs", name = "PatternWildcard", from_py_object)]
#[derive(Clone)]
pub struct PatternWildcard;

#[pymethods]
impl PatternWildcard {
    #[new]
    fn new() -> Self {
        Self
    }

    fn __repr__(&self) -> String {
        "<PatternWildcard>".to_string()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(other.extract::<PyRef<PatternWildcard>>().is_ok())
    }

    fn __hash__(&self) -> isize {
        // Constant hash for singleton-like behavior
        0x1234_5678
    }

    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(py.None())
    }

    fn __setstate__(&mut self, _py: Python<'_>, _state: &Bound<'_, PyAny>) -> PyResult<()> {
        Ok(())
    }

    fn pattern_tag(&self) -> u8 {
        TAG_WILDCARD
    }
}

/// Pattern that matches any of multiple patterns.
#[pyclass(module = "catnip._rs", name = "PatternOr")]
pub struct PatternOr {
    #[pyo3(get)]
    pub patterns: Py<PyAny>,
}

#[pymethods]
impl PatternOr {
    #[new]
    #[pyo3(signature = (patterns=None))]
    fn new(py: Python<'_>, patterns: Option<Py<PyAny>>) -> Self {
        let patterns = patterns.unwrap_or_else(|| py.None());
        Self { patterns }
    }

    fn __repr__(&self) -> PyResult<String> {
        Python::attach(|py| {
            let patterns_repr = self.patterns.bind(py).repr()?;
            Ok(format!("<PatternOr {}>", patterns_repr))
        })
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Python::attach(|py| {
            if let Ok(other_pattern) = other.extract::<PyRef<PatternOr>>() {
                let eq = self.patterns.bind(py).eq(other_pattern.patterns.bind(py))?;
                Ok(eq)
            } else {
                Ok(false)
            }
        })
    }

    fn __hash__(&self) -> PyResult<isize> {
        Python::attach(|py| self.patterns.bind(py).hash())
    }

    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("patterns", self.patterns.clone_ref(py))?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.patterns = state.get_item("patterns")?.unwrap().unbind();
        Ok(())
    }

    fn pattern_tag(&self) -> u8 {
        TAG_OR
    }
}

/// Pattern that matches and destructures tuples/lists: (a, b) or (a, *rest, b)
#[pyclass(module = "catnip._rs", name = "PatternTuple")]
pub struct PatternTuple {
    #[pyo3(get)]
    pub patterns: Py<PyAny>,
}

#[pymethods]
impl PatternTuple {
    #[new]
    #[pyo3(signature = (patterns=None))]
    fn new(py: Python<'_>, patterns: Option<Py<PyAny>>) -> Self {
        let patterns = patterns.unwrap_or_else(|| py.None());
        Self { patterns }
    }

    fn __repr__(&self) -> PyResult<String> {
        Python::attach(|py| {
            let patterns_repr = self.patterns.bind(py).repr()?;
            Ok(format!("<PatternTuple {}>", patterns_repr))
        })
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Python::attach(|py| {
            if let Ok(other_pattern) = other.extract::<PyRef<PatternTuple>>() {
                let eq = self.patterns.bind(py).eq(other_pattern.patterns.bind(py))?;
                Ok(eq)
            } else {
                Ok(false)
            }
        })
    }

    fn __hash__(&self) -> PyResult<isize> {
        Python::attach(|py| self.patterns.bind(py).hash())
    }

    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("patterns", self.patterns.clone_ref(py))?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.patterns = state.get_item("patterns")?.unwrap().unbind();
        Ok(())
    }

    fn pattern_tag(&self) -> u8 {
        TAG_TUPLE
    }
}

/// Pattern that matches a struct instance and destructures its fields.
#[pyclass(module = "catnip._rs", name = "PatternStruct")]
pub struct PatternStruct {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub fields: Py<PyAny>,
}

#[pymethods]
impl PatternStruct {
    #[new]
    #[pyo3(signature = (name=String::new(), fields=None))]
    fn new(py: Python<'_>, name: String, fields: Option<Py<PyAny>>) -> Self {
        let fields = fields.unwrap_or_else(|| py.None());
        Self { name, fields }
    }

    fn __repr__(&self) -> String {
        format!("<PatternStruct {}>", self.name)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(other_pattern) = other.extract::<PyRef<PatternStruct>>() {
            Python::attach(|py| {
                Ok(self.name == other_pattern.name && self.fields.bind(py).eq(other_pattern.fields.bind(py))?)
            })
        } else {
            Ok(false)
        }
    }

    fn __hash__(&self) -> isize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        self.name.hash(&mut hasher);
        hasher.finish() as isize
    }

    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("name", &self.name)?;
        state.set_item("fields", self.fields.clone_ref(py))?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.name = state.get_item("name")?.unwrap().extract()?;
        self.fields = state.get_item("fields")?.unwrap().unbind();
        Ok(())
    }

    fn pattern_tag(&self) -> u8 {
        TAG_STRUCT
    }
}
