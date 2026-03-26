// FILE: catnip_rs/src/transformer/mod.rs
//! Base classes and data structures for the transformer - Rust implementation.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

/// Parameter container for function/method calls.
#[pyclass(module = "catnip._rs")]
#[derive(Debug)]
pub struct Params {
    #[pyo3(get, set)]
    pub args: Py<PyAny>, // Tuple

    #[pyo3(get, set)]
    pub kwargs: Py<PyAny>, // dict
}

#[pymethods]
impl Params {
    #[new]
    #[pyo3(signature = (args=None, kwargs=None))]
    fn new(py: Python, args: Option<Py<PyAny>>, kwargs: Option<Py<PyAny>>) -> PyResult<Self> {
        let args_tuple = if let Some(a) = args {
            let bound = a.bind(py);
            if bound.is_instance_of::<PyTuple>() {
                a
            } else {
                PyTuple::new(py, std::slice::from_ref(bound))?.unbind().into()
            }
        } else {
            PyTuple::empty(py).unbind().into()
        };

        let kwargs_dict = if let Some(k) = kwargs {
            k
        } else {
            // Create empty dict
            PyDict::new(py).into()
        };

        Ok(Self {
            args: args_tuple,
            kwargs: kwargs_dict,
        })
    }

    fn __repr__(&self, py: Python) -> PyResult<String> {
        let args_repr = self.args.bind(py).repr()?.to_string();
        let kwargs_repr = self.kwargs.bind(py).repr()?.to_string();
        Ok(format!("<Params {} {}>", args_repr, kwargs_repr))
    }

    // Pickle support
    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("args", self.args.clone_ref(py))?;
        state.set_item("kwargs", self.kwargs.clone_ref(py))?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.args = state.get_item("args")?.unwrap().unbind();
        self.kwargs = state.get_item("kwargs")?.unwrap().unbind();
        Ok(())
    }

    fn __getnewargs__(&self, py: Python<'_>) -> (Py<PyAny>, Py<PyAny>) {
        (self.args.clone_ref(py), self.kwargs.clone_ref(py))
    }
}

/// Left-value in assignment (wrapper around string).
#[pyclass(module = "catnip._rs", from_py_object)]
#[derive(Debug, Clone)]
pub struct Lvalue {
    #[pyo3(get)]
    value: String,
}

#[pymethods]
impl Lvalue {
    #[new]
    fn new(value: String) -> Self {
        Self { value }
    }

    fn __str__(&self) -> String {
        self.value.clone()
    }

    fn __repr__(&self) -> String {
        self.value.clone()
    }

    fn __hash__(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.value.hash(&mut hasher);
        hasher.finish()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(other_lvalue) = other.extract::<Lvalue>() {
            Ok(self.value == other_lvalue.value)
        } else if let Ok(other_str) = other.extract::<String>() {
            Ok(self.value == other_str)
        } else {
            Ok(false)
        }
    }

    // Pickle support
    fn __getnewargs__(&self) -> (String,) {
        (self.value.clone(),)
    }
}

/// Identifier token (wrapper around string).
#[pyclass(module = "catnip._rs", from_py_object)]
#[derive(Debug, Clone)]
pub struct Identifier {
    #[pyo3(get)]
    value: String,
}

#[pymethods]
impl Identifier {
    #[new]
    fn new(value: String) -> Self {
        Self { value }
    }

    fn __str__(&self) -> String {
        self.value.clone()
    }

    fn __repr__(&self) -> String {
        self.value.clone()
    }

    fn __hash__(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.value.hash(&mut hasher);
        hasher.finish()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(other_id) = other.extract::<Identifier>() {
            Ok(self.value == other_id.value)
        } else if let Ok(other_str) = other.extract::<String>() {
            Ok(self.value == other_str)
        } else {
            Ok(false)
        }
    }

    // Pickle support
    fn __getnewargs__(&self) -> (String,) {
        (self.value.clone(),)
    }
}

/// Attribute access member: .attr
#[pyclass(subclass, module = "catnip._rs")]
#[derive(Debug)]
pub struct Member {
    #[pyo3(get, set)]
    pub ident: Py<PyAny>,
}

#[pymethods]
impl Member {
    #[new]
    fn new(ident: Py<PyAny>) -> Self {
        Self { ident }
    }

    fn __repr__(&self, py: Python) -> PyResult<String> {
        let ident_repr = self.ident.bind(py).repr()?.to_string();
        Ok(format!("<Member {}>", ident_repr))
    }

    // Pickle support
    fn __getnewargs__(&self, py: Python<'_>) -> (Py<PyAny>,) {
        (self.ident.clone_ref(py),)
    }
}

/// Method call member: .method(args)
#[pyclass(extends = Member, module = "catnip._rs")]
#[derive(Debug)]
pub struct CallMember {
    #[pyo3(get, set)]
    pub params: Py<Params>,
}

#[pymethods]
impl CallMember {
    #[new]
    #[pyo3(signature = (ident, params=None))]
    fn new(py: Python, ident: Py<PyAny>, params: Option<Py<Params>>) -> PyResult<(Self, Member)> {
        let params_obj = if let Some(p) = params {
            p
        } else {
            Py::new(py, Params::new(py, None, None)?)?
        };

        let member = Member::new(ident);
        Ok((Self { params: params_obj }, member))
    }

    fn __repr__(slf: PyRef<'_, Self>, py: Python) -> PyResult<String> {
        let params_ref = slf.params.borrow(py);
        // Access parent's ident attribute
        let parent = slf.as_super();
        let ident_repr = parent.ident.bind(py).repr()?.to_string();
        let args_repr = params_ref.args.bind(py).repr()?.to_string();
        let kwargs_repr = params_ref.kwargs.bind(py).repr()?.to_string();
        Ok(format!("<CallMember {} {} {}>", ident_repr, args_repr, kwargs_repr))
    }

    // Pickle support
    fn __getnewargs__(slf: PyRef<'_, Self>, py: Python<'_>) -> (Py<PyAny>, Py<Params>) {
        let parent = slf.as_super();
        (parent.ident.clone_ref(py), slf.params.clone_ref(py))
    }
}

/// Function call: func(args, kwargs)
#[pyclass(module = "catnip._rs")]
#[derive(Debug)]
pub struct Call {
    #[pyo3(get, set)]
    pub func: Py<PyAny>,

    #[pyo3(get, set)]
    pub args: Py<PyAny>, // Tuple

    #[pyo3(get, set)]
    pub kwargs: Py<PyAny>, // dict

    #[pyo3(get, set)]
    pub start_byte: isize,

    #[pyo3(get, set)]
    pub end_byte: isize,
}

#[pymethods]
impl Call {
    #[new]
    #[pyo3(signature = (func, args=None, kwargs=None))]
    fn new(py: Python, func: Py<PyAny>, args: Option<Py<PyAny>>, kwargs: Option<Py<PyAny>>) -> PyResult<Self> {
        let args_tuple = if let Some(a) = args {
            let bound = a.bind(py);
            if bound.is_instance_of::<PyTuple>() {
                a
            } else {
                PyTuple::new(py, std::slice::from_ref(bound))?.unbind().into()
            }
        } else {
            PyTuple::empty(py).unbind().into()
        };

        let kwargs_dict = if let Some(k) = kwargs {
            k
        } else {
            // Create empty dict
            PyDict::new(py).into()
        };

        Ok(Self {
            func,
            args: args_tuple,
            kwargs: kwargs_dict,
            start_byte: -1,
            end_byte: -1,
        })
    }

    fn __repr__(&self, py: Python) -> PyResult<String> {
        let func_repr = self.func.bind(py).repr()?.to_string();
        let args_repr = self.args.bind(py).repr()?.to_string();
        let kwargs_repr = self.kwargs.bind(py).repr()?.to_string();
        Ok(format!("<Call {} {} {}>", func_repr, args_repr, kwargs_repr))
    }

    // Pickle support
    fn __getnewargs__(&self, py: Python<'_>) -> (Py<PyAny>, Py<PyAny>, Py<PyAny>) {
        (
            self.func.clone_ref(py),
            self.args.clone_ref(py),
            self.kwargs.clone_ref(py),
        )
    }
}

/// Target for attribute assignment: obj.attr = value
#[pyclass(module = "catnip._rs")]
#[derive(Debug)]
pub struct SetAttrTarget {
    #[pyo3(get, set)]
    pub base: Py<PyAny>,

    #[pyo3(get, set)]
    pub members: Py<PyAny>, // List
}

#[pymethods]
impl SetAttrTarget {
    #[new]
    fn new(base: Py<PyAny>, members: Py<PyAny>) -> Self {
        Self { base, members }
    }

    fn __repr__(&self, py: Python) -> PyResult<String> {
        let base_repr = self.base.bind(py).repr()?.to_string();
        let members_repr = self.members.bind(py).repr()?.to_string();
        Ok(format!("<SetAttrTarget {} {}>", base_repr, members_repr))
    }

    // Pickle support
    fn __getnewargs__(&self, py: Python<'_>) -> (Py<PyAny>, Py<PyAny>) {
        (self.base.clone_ref(py), self.members.clone_ref(py))
    }
}

pub fn register_module(parent_module: &Bound<'_, PyModule>) -> PyResult<()> {
    parent_module.add_class::<Params>()?;
    parent_module.add_class::<Lvalue>()?;
    parent_module.add_class::<Identifier>()?;
    parent_module.add_class::<Member>()?;
    parent_module.add_class::<CallMember>()?;
    parent_module.add_class::<Call>()?;
    parent_module.add_class::<SetAttrTarget>()?;
    Ok(())
}
