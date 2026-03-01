// FILE: catnip_rs/src/core/op.rs
use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

#[pyclass(name = "Op", module = "catnip._rs", subclass, from_py_object)]
#[derive(Debug)]
pub struct Op {
    #[pyo3(get, set)]
    pub ident: i32,
    #[pyo3(get, set)]
    pub args: Py<PyAny>,
    #[pyo3(get, set)]
    pub kwargs: Py<PyAny>,
    #[pyo3(get, set)]
    pub tail: bool,
    #[pyo3(get, set)]
    pub start_byte: isize,
    #[pyo3(get, set)]
    pub end_byte: isize,
}

impl Clone for Op {
    fn clone(&self) -> Self {
        Python::attach(|py| Op {
            ident: self.ident,
            args: self.args.clone_ref(py),
            kwargs: self.kwargs.clone_ref(py),
            tail: self.tail,
            start_byte: self.start_byte,
            end_byte: self.end_byte,
        })
    }
}

impl Op {
    /// Create an Op from Rust (not PyO3)
    pub fn from_rust(
        _py: Python<'_>,
        ident: i32,
        args: Py<PyAny>,
        kwargs: Py<PyAny>,
        tail: bool,
        start_byte: isize,
        end_byte: isize,
    ) -> Self {
        Op {
            ident,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        }
    }
}

#[pymethods]
impl Op {
    #[new]
    #[pyo3(signature = (ident, args=None, kwargs=None, start_byte=None, end_byte=None))]
    fn new(
        py: Python<'_>,
        ident: Bound<'_, PyAny>,
        args: Option<Py<PyAny>>,
        kwargs: Option<Py<PyAny>>,
        start_byte: Option<isize>,
        end_byte: Option<isize>,
    ) -> PyResult<Self> {
        // Convert ident to i32 (accept both int and str)
        let ident_value = if let Ok(int_val) = ident.extract::<i32>() {
            // Direct integer opcode
            int_val
        } else if let Ok(str_val) = ident.extract::<&str>() {
            // String opcode name - convert via OpCode enum
            // Handle common aliases for backward compatibility
            let normalized = match str_val.to_lowercase().as_str() {
                "set_local" => "SET_LOCALS".to_string(),
                "inv" => "BNOT".to_string(), // inv is alias for bitwise NOT
                "not" | "bool_not" => "NOT".to_string(), // logical NOT
                "and" | "bool_and" => "AND".to_string(), // logical AND
                "or" | "bool_or" => "OR".to_string(), // logical OR
                "bit_and" => "BAND".to_string(), // bitwise AND
                "bit_or" => "BOR".to_string(), // bitwise OR
                "bit_xor" => "BXOR".to_string(), // bitwise XOR
                _ => str_val.to_uppercase(),
            };

            let opcode_module = py.import("catnip.semantic.opcode")?;
            let opcode_class = opcode_module.getattr("OpCode")?;
            let opcode_attr = opcode_class.getattr(normalized.as_str())?;
            opcode_attr.extract::<i32>()?
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "ident must be either an integer opcode or a string opcode name",
            ));
        };

        let args = args.unwrap_or_else(|| PyTuple::empty(py).into());
        let kwargs = kwargs.unwrap_or_else(|| PyDict::new(py).into());
        Ok(Op {
            ident: ident_value,
            args,
            kwargs,
            tail: false,
            start_byte: start_byte.unwrap_or(-1),
            end_byte: end_byte.unwrap_or(-1),
        })
    }

    #[inline]
    pub fn get_ident(&self) -> i32 {
        self.ident
    }

    #[inline]
    pub fn get_args(&self) -> Py<PyAny> {
        Python::attach(|py| self.args.clone_ref(py))
    }

    #[inline]
    pub fn get_kwargs(&self) -> Py<PyAny> {
        Python::attach(|py| self.kwargs.clone_ref(py))
    }

    #[inline]
    fn is_tail(&self) -> bool {
        self.tail
    }

    fn __repr__(&self) -> PyResult<String> {
        Python::attach(|py| {
            let args_repr = self.args.bind(py).repr()?;
            let kwargs_repr = self.kwargs.bind(py).repr()?;

            // Convert opcode integer to name via Python OpCode enum
            let ident_repr = match py.import("catnip.semantic.opcode") {
                Ok(opcode_module) => match opcode_module.getattr("OpCode") {
                    Ok(opcode_class) => match opcode_class.call1((self.ident,)) {
                        Ok(opcode) => match opcode.getattr("name") {
                            Ok(name) => name
                                .extract::<String>()
                                .unwrap_or_else(|_| self.ident.to_string()),
                            Err(_) => self.ident.to_string(),
                        },
                        Err(_) => self.ident.to_string(),
                    },
                    Err(_) => self.ident.to_string(),
                },
                Err(_) => self.ident.to_string(),
            };

            Ok(format!("<Op {} {} {}>", ident_repr, args_repr, kwargs_repr))
        })
    }

    fn __richcmp__(&self, other: &Self, op: CompareOp) -> PyResult<bool> {
        match op {
            CompareOp::Eq => {
                if self.ident != other.ident {
                    return Ok(false);
                }
                Python::attach(|py| {
                    let args_eq = self.args.bind(py).eq(other.args.bind(py))?;
                    if !args_eq {
                        return Ok(false);
                    }
                    let kwargs_eq = self.kwargs.bind(py).eq(other.kwargs.bind(py))?;
                    Ok(kwargs_eq)
                })
            }
            CompareOp::Ne => {
                let eq = self.__richcmp__(other, CompareOp::Eq)?;
                Ok(!eq)
            }
            _ => Err(pyo3::exceptions::PyTypeError::new_err("Not implemented")),
        }
    }

    // Pickle support
    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = PyDict::new(py);
        state.set_item("ident", self.ident)?;
        state.set_item("args", self.args.clone_ref(py))?;
        state.set_item("kwargs", self.kwargs.clone_ref(py))?;
        state.set_item("tail", self.tail)?;
        state.set_item("start_byte", self.start_byte)?;
        state.set_item("end_byte", self.end_byte)?;
        Ok(state.into())
    }

    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        self.ident = state.get_item("ident")?.unwrap().extract()?;
        self.args = state.get_item("args")?.unwrap().unbind();
        self.kwargs = state.get_item("kwargs")?.unwrap().unbind();
        self.tail = state.get_item("tail")?.unwrap().extract()?;
        self.start_byte = state.get_item("start_byte")?.unwrap().extract()?;
        self.end_byte = state.get_item("end_byte")?.unwrap().extract()?;
        Ok(())
    }

    fn __getnewargs__(&self) -> (i32,) {
        (0,) // dummy args for __new__, real state restored by __setstate__
    }
}
