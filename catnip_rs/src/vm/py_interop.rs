// FILE: catnip_rs/src/vm/py_interop.rs
//! Python interop layer for operations requiring Python callbacks.
//!
//! The Rust VM delegates to Python for:
//! - Name resolution via scope chain
//! - Function calls (user-defined functions)
//! - Complex object operations (strings, lists, dicts)
//! - Attribute/item access
//! - Iteration

use super::frame::{CodeObject, PyCodeObject, NO_VARARG_IDX};
use super::opcode::Instruction;
use super::value::Value;
use super::OpCode;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::collections::HashMap;

pub(crate) fn append_instructions_from_bytecode(
    bytecode: &Bound<'_, PyTuple>,
    code: &mut CodeObject,
) -> PyResult<()> {
    const INSTR_TUPLE_LEN: usize = 2;
    const OPCODE_INDEX: usize = 0;
    const ARG_INDEX: usize = 1;
    for item in bytecode.iter() {
        if let Ok(tuple) = item.cast::<PyTuple>() {
            if tuple.len() >= INSTR_TUPLE_LEN {
                let op: u8 = tuple.get_item(OPCODE_INDEX)?.extract()?;
                let arg: u32 = tuple.get_item(ARG_INDEX)?.extract()?;
                if let Some(opcode) = OpCode::from_u8(op) {
                    code.instructions.push(Instruction::new(opcode, arg));
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn append_constants_from_tuple(
    py: Python<'_>,
    constants: &Bound<'_, PyTuple>,
    code: &mut CodeObject,
) -> PyResult<()> {
    for item in constants.iter() {
        let value = Value::from_pyobject(py, &item)?;
        code.constants.push(value);
    }

    Ok(())
}

/// Python context wrapper for VM callbacks.
#[pyclass(module = "catnip._rs")]
pub struct PyVMContext {
    /// Reference to Catnip context.locals for scope resolution
    pub locals: Py<PyAny>,
    /// Reference to Catnip context.globals
    pub globals: Py<PyAny>,
    /// Registry for function execution
    pub registry: Option<Py<PyAny>>,
}

#[pymethods]
impl PyVMContext {
    #[new]
    fn new(locals: Py<PyAny>, globals: Py<PyAny>) -> Self {
        Self {
            locals,
            globals,
            registry: None,
        }
    }

    /// Set the registry for function calls.
    fn set_registry(&mut self, registry: Py<PyAny>) {
        self.registry = Some(registry);
    }
}

impl PyVMContext {
    /// Resolve a name via the scope chain.
    pub fn resolve_name(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        // Try locals._resolve(name) first
        let locals = self.locals.bind(py);
        if let Ok(resolve) = locals.getattr("_resolve") {
            if let Ok(result) = resolve.call1((name,)) {
                return Ok(result.unbind());
            }
        }

        // Try globals
        let globals = self.globals.bind(py);
        if let Ok(result) = globals.get_item(name) {
            if !result.is_none() {
                return Ok(result.unbind());
            }
        }

        Err(pyo3::exceptions::PyNameError::new_err(format!(
            "name '{}' is not defined",
            name
        )))
    }

    /// Store a name via the scope chain.
    pub fn store_name(&self, py: Python<'_>, name: &str, value: Py<PyAny>) -> PyResult<()> {
        // Try locals._set(name, value)
        let locals = self.locals.bind(py);
        if let Ok(set_fn) = locals.getattr("_set") {
            set_fn.call1((name, &value))?;
            return Ok(());
        }

        // Fall back to globals
        let globals = self.globals.bind(py);
        if let Ok(dict) = globals.cast::<PyDict>() {
            dict.set_item(name, value)?;
        }

        Ok(())
    }

    /// Call a Python function with arguments.
    pub fn call_function(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        args: Vec<Value>,
        kwargs: Option<HashMap<String, Value>>,
    ) -> PyResult<Value> {
        // Convert args to PyObjects
        let py_args: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
        let py_args_tuple = PyTuple::new(py, py_args)?;

        let result = if let Some(kw) = kwargs {
            let py_kwargs = PyDict::new(py);
            for (k, v) in kw {
                py_kwargs.set_item(k, v.to_pyobject(py))?;
            }
            func.call(py_args_tuple, Some(&py_kwargs))?
        } else {
            func.call1(py_args_tuple)?
        };

        Value::from_pyobject(py, &result)
    }

    /// Get an attribute from an object.
    pub fn getattr(&self, py: Python<'_>, obj: Value, name: &str) -> PyResult<Value> {
        let py_obj = obj.to_pyobject(py);
        let result = py_obj.bind(py).getattr(name)?;
        Value::from_pyobject(py, &result)
    }

    /// Set an attribute on an object.
    pub fn setattr(&self, py: Python<'_>, obj: Value, name: &str, value: Value) -> PyResult<()> {
        let py_obj = obj.to_pyobject(py);
        py_obj.bind(py).setattr(name, value.to_pyobject(py))?;
        Ok(())
    }

    /// Get an item from a collection.
    pub fn getitem(&self, py: Python<'_>, obj: Value, key: Value) -> PyResult<Value> {
        let py_obj = obj.to_pyobject(py);
        let py_key = key.to_pyobject(py);
        let result = py_obj.bind(py).get_item(py_key)?;
        Value::from_pyobject(py, &result)
    }

    /// Set an item in a collection.
    pub fn setitem(&self, py: Python<'_>, obj: Value, key: Value, value: Value) -> PyResult<()> {
        let py_obj = obj.to_pyobject(py);
        let py_key = key.to_pyobject(py);
        py_obj.bind(py).set_item(py_key, value.to_pyobject(py))?;
        Ok(())
    }

    /// Get an iterator from an object.
    pub fn get_iter(&self, py: Python<'_>, obj: Value) -> PyResult<Value> {
        let py_obj = obj.to_pyobject(py);
        let iter = py_obj.bind(py).try_iter()?;
        Value::from_pyobject(py, iter.as_any())
    }

    /// Get next value from an iterator, or None if exhausted.
    pub fn next_iter(&self, py: Python<'_>, iter: Value) -> PyResult<Option<Value>> {
        let py_iter = iter.to_pyobject(py);
        let iter_bound = py_iter.bind(py);

        // Try to call __next__
        match iter_bound.call_method0("__next__") {
            Ok(result) => Ok(Some(Value::from_pyobject(py, &result)?)),
            Err(e) => {
                if e.is_instance_of::<pyo3::exceptions::PyStopIteration>(py) {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }
}

/// Convert a Python CodeObject to a Rust CodeObject.
pub fn convert_code_object(
    py: Python<'_>,
    py_code: &Bound<'_, PyAny>,
) -> PyResult<super::frame::CodeObject> {
    // Fast path: if it's already a PyCodeObject, extract directly
    if let Ok(rust_code) = py_code.cast::<PyCodeObject>() {
        return Ok(rust_code.borrow().inner.clone_with_py(py));
    }

    let mut code = CodeObject::new(
        py_code
            .getattr("name")?
            .extract::<String>()
            .unwrap_or_else(|_| "<code>".to_string()),
    );

    // Convert bytecode: list of (op, arg) tuples
    let bytecode = py_code.getattr("bytecode")?;
    if let Ok(bc_list) = bytecode.cast::<PyTuple>() {
        append_instructions_from_bytecode(bc_list, &mut code)?;
    }

    // Convert constants
    let constants = py_code.getattr("constants")?;
    if let Ok(const_tuple) = constants.cast::<PyTuple>() {
        append_constants_from_tuple(py, const_tuple, &mut code)?;
    }

    // Convert names
    let names = py_code.getattr("names")?;
    if let Ok(names_tuple) = names.cast::<PyTuple>() {
        for item in names_tuple.iter() {
            code.names.push(item.extract::<String>()?);
        }
    }

    // Other attributes
    code.nlocals = py_code.getattr("nlocals")?.extract().unwrap_or(0);
    code.nargs = py_code.getattr("nargs")?.extract().unwrap_or(0);
    code.vararg_idx = py_code
        .getattr("vararg_idx")?
        .extract()
        .unwrap_or(NO_VARARG_IDX);

    // Convert varnames
    let varnames = py_code.getattr("varnames")?;
    if let Ok(var_tuple) = varnames.cast::<PyTuple>() {
        for item in var_tuple.iter() {
            code.varnames.push(item.to_string());
        }
    }

    // Convert slotmap
    let slotmap = py_code.getattr("slotmap")?;
    if let Ok(dict) = slotmap.cast::<PyDict>() {
        for (key, value) in dict.iter() {
            let name = key.to_string();
            let slot: usize = value.extract()?;
            code.slotmap.insert(name, slot);
        }
    }

    // Convert defaults
    let defaults = py_code.getattr("defaults")?;
    if let Ok(def_tuple) = defaults.cast::<PyTuple>() {
        for item in def_tuple.iter() {
            code.defaults.push(item.unbind());
        }
    }

    // Convert freevars
    let freevars = py_code.getattr("freevars")?;
    if let Ok(fv_tuple) = freevars.cast::<PyTuple>() {
        for item in fv_tuple.iter() {
            code.freevars.push(item.extract::<String>()?);
        }
    }

    // Pure flag and complexity
    code.is_pure = py_code
        .getattr("is_pure")
        .and_then(|v| v.extract())
        .unwrap_or(false);
    code.complexity = code.instructions.len();

    Ok(code)
}

/// Convert Python args to Value array.
pub fn convert_args(py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Vec<Value>> {
    let mut values = Vec::with_capacity(args.len());
    for item in args.iter() {
        values.push(Value::from_pyobject(py, &item)?);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        Python::initialize();
        Python::attach(|py| {
            let locals = PyDict::new(py).into_any().unbind();
            let globals = PyDict::new(py).into_any().unbind();
            let ctx = PyVMContext::new(locals, globals);
            assert!(ctx.registry.is_none());
        });
    }
}
