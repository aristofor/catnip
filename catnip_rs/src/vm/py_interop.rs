// FILE: catnip_rs/src/vm/py_interop.rs
//! Python interop layer for operations requiring Python callbacks.
//!
//! The Rust VM delegates to Python for:
//! - Name resolution via scope chain
//! - Function calls (user-defined functions)
//! - Complex object operations (strings, lists, dicts)
//! - Attribute/item access
//! - Iteration

use super::OpCode;
use super::frame::{CodeObject, NO_VARARG_IDX, PyCodeObject};
use super::opcode::Instruction;
use super::value::Value;
use indexmap::IndexMap;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::sync::Arc;

// --- PyErr -> VMError conversion (explicit py token, no Python::attach) ---

/// Extract the message from a Python exception (without the class prefix).
///
/// PyO3 `err.to_string()` returns `"TypeError: msg"` -- we want just `"msg"`.
#[inline]
fn pyerr_message(py: Python<'_>, err: &PyErr) -> String {
    err.value(py)
        .str()
        .map(|s| s.to_string())
        .unwrap_or_else(|_| err.to_string())
}

/// Convert a PyErr to VMError using an explicit `py` token.
#[inline]
pub(crate) fn pyerr_to_vmerror(py: Python<'_>, err: PyErr) -> super::core::VMError {
    if err.is_instance_of::<pyo3::exceptions::PySystemExit>(py) {
        let code = err
            .value(py)
            .getattr("code")
            .ok()
            .and_then(|c| c.extract::<i32>().ok())
            .unwrap_or(0);
        super::core::VMError::Exit(code)
    } else if err.is_instance_of::<pyo3::exceptions::PyTypeError>(py) {
        super::core::VMError::TypeError(pyerr_message(py, &err))
    } else if err.is_instance_of::<pyo3::exceptions::PyNameError>(py) {
        super::core::VMError::NameError(pyerr_message(py, &err))
    } else if err.is_instance_of::<pyo3::exceptions::PyZeroDivisionError>(py) {
        super::core::VMError::ZeroDivisionError(pyerr_message(py, &err))
    } else if err.is_instance_of::<pyo3::exceptions::PyIndexError>(py) {
        super::core::VMError::RuntimeError(format!("IndexError: {}", pyerr_message(py, &err)))
    } else if err.is_instance_of::<pyo3::exceptions::PyKeyError>(py) {
        super::core::VMError::RuntimeError(format!("KeyError: {}", pyerr_message(py, &err)))
    } else if err.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
        super::core::VMError::RuntimeError(format!("ValueError: {}", pyerr_message(py, &err)))
    } else if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) {
        super::core::VMError::RuntimeError(format!("AttributeError: {}", pyerr_message(py, &err)))
    } else {
        // Extract Python exception class name for the prefix
        let type_name = err
            .get_type(py)
            .qualname()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "RuntimeError".to_string());
        super::core::VMError::RuntimeError(format!("{}: {}", type_name, pyerr_message(py, &err)))
    }
}

/// Extension trait to convert PyResult<T> to VMResult<T> with explicit py token.
pub(crate) trait PyResultExt<T> {
    fn to_vm(self, py: Python<'_>) -> Result<T, super::core::VMError>;
}

impl<T> PyResultExt<T> for PyResult<T> {
    #[inline]
    fn to_vm(self, py: Python<'_>) -> Result<T, super::core::VMError> {
        self.map_err(|e| pyerr_to_vmerror(py, e))
    }
}

// --- Centralized Python interop helpers for the VM dispatch loop ---

/// Lookup a name in Python context.globals, returning a Value if found.
#[inline]
pub(crate) fn lookup_ctx_global(
    py: Python<'_>,
    globals: &Py<PyDict>,
    name: &str,
) -> Result<Option<Value>, super::core::VMError> {
    match globals.bind(py).get_item(name) {
        Ok(Some(val)) => {
            let value =
                Value::from_pyobject(py, &val).map_err(|e| super::core::VMError::RuntimeError(e.to_string()))?;
            Ok(Some(value))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(super::core::VMError::RuntimeError(e.to_string())),
    }
}

/// Store a value into Python context.globals.
#[inline]
pub(crate) fn store_ctx_global(
    py: Python<'_>,
    globals: &Py<PyDict>,
    name: &str,
    value: Value,
) -> Result<(), super::core::VMError> {
    let py_value = value.to_pyobject(py);
    globals
        .bind(py)
        .set_item(name, py_value)
        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
}

/// Call a Python binary operator (operator.add, operator.lt, etc.) with Value args.
#[inline]
pub(crate) fn call_binary_op(
    py: Python<'_>,
    op: &Py<PyAny>,
    a: Value,
    b: Value,
) -> Result<Value, super::core::VMError> {
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let py_result = op.call1(py, (&py_a, &py_b)).to_vm(py)?;
    Value::from_pyobject(py, py_result.bind(py)).to_vm(py)
}

/// Delete a name from Python context.globals.
#[inline]
pub(crate) fn delete_ctx_global(py: Python<'_>, globals: &Py<PyDict>, name: &str) -> Result<(), super::core::VMError> {
    globals
        .bind(py)
        .del_item(name)
        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
}

/// Extract a typed value from a PyTuple at the given index.
/// Combines get_item + extract with VMError mapping.
#[inline]
pub(crate) fn tuple_extract<'py, T>(tuple: &Bound<'py, PyTuple>, idx: usize) -> Result<T, super::core::VMError>
where
    for<'a> T: FromPyObject<'a, 'py, Error = PyErr>,
{
    tuple
        .get_item(idx)
        .map_err(|e: PyErr| super::core::VMError::RuntimeError(e.to_string()))?
        .extract()
        .map_err(|e: PyErr| super::core::VMError::RuntimeError(e.to_string()))
}

/// Get a raw Bound<PyAny> from a PyTuple at the given index.
#[inline]
pub(crate) fn tuple_get<'py>(
    tuple: &Bound<'py, PyTuple>,
    idx: usize,
) -> Result<Bound<'py, PyAny>, super::core::VMError> {
    tuple
        .get_item(idx)
        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
}

/// Cast a Bound<PyAny> to &Bound<PyTuple>.
#[inline]
pub(crate) fn cast_tuple<'a, 'py>(obj: &'a Bound<'py, PyAny>) -> Result<&'a Bound<'py, PyTuple>, super::core::VMError> {
    obj.cast::<PyTuple>()
        .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
}

/// Convert TAG_STRUCT values to TAG_PYOBJ in captured closure scope.
/// Struct instances are registry-indexed; the index is only valid in the VM
/// that owns the registry. Converting eagerly to PyObject (CatnipStructProxy)
/// makes closures portable across child VMs (e.g. ND recursion).
pub(crate) fn portabilize_struct_values(
    py: Python<'_>,
    captured: &mut IndexMap<String, Value>,
    registry: &mut super::structs::StructRegistry,
) {
    for val in captured.values_mut() {
        if val.is_struct_instance() {
            // Convert to PyObject first (reads via thread-local, immutable)
            let py_obj = val.to_pyobject(py);
            // Decref old struct reference (the TAG_PYOBJ takes over ownership)
            let idx = val.as_struct_instance_idx().unwrap();
            if let Some(fields) = registry.decref(idx) {
                super::structs::cascade_decref_fields(registry, fields);
            }
            *val = Value::from_owned_pyobject(py_obj);
        }
    }
}

/// Resolve the registry from the Python context.
#[inline]
pub(crate) fn resolve_registry<'py>(
    py: Python<'py>,
    py_context: &Option<Py<PyAny>>,
) -> Result<Bound<'py, PyAny>, super::core::VMError> {
    if let Some(ref ctx) = py_context {
        ctx.bind(py)
            .getattr("_registry")
            .map_err(|e| super::core::VMError::RuntimeError(e.to_string()))
    } else {
        Err(super::core::VMError::RuntimeError("Context not available".to_string()))
    }
}

pub(crate) fn append_instructions_from_bytecode(bytecode: &Bound<'_, PyTuple>, code: &mut CodeObject) -> PyResult<()> {
    const INSTR_TUPLE_LEN: usize = 2;
    const OPCODE_INDEX: usize = 0;
    const ARG_INDEX: usize = 1;
    for (i, item) in bytecode.iter().enumerate() {
        let tuple = item.cast::<PyTuple>().map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err(format!(
                "bytecode[{i}]: expected tuple (op, arg), got {}",
                item.get_type()
                    .name()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|_| "?".to_string())
            ))
        })?;
        if tuple.len() < INSTR_TUPLE_LEN {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "bytecode[{i}]: expected tuple of length >= 2, got length {}",
                tuple.len()
            )));
        }
        let op: u8 = tuple.get_item(OPCODE_INDEX)?.extract()?;
        let arg: u32 = tuple.get_item(ARG_INDEX)?.extract()?;
        if let Some(opcode) = OpCode::from_u8(op) {
            code.instructions.push(Instruction::new(opcode, arg));
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "bytecode[{i}]: unknown VM opcode: {op}"
            )));
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

        Err(pyo3::exceptions::PyNameError::new_err(
            catnip_core::constants::format_name_error(name),
        ))
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
            return Ok(());
        }

        // Neither locals._set nor globals dict worked
        Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "cannot store name '{name}': locals has no _set method and globals is not a dict"
        )))
    }

    /// Call a Python function with arguments.
    pub fn call_function(
        &self,
        py: Python<'_>,
        func: &Bound<'_, PyAny>,
        args: Vec<Value>,
        kwargs: Option<IndexMap<String, Value>>,
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
pub fn convert_code_object(py: Python<'_>, py_code: &Bound<'_, PyAny>) -> PyResult<Arc<CodeObject>> {
    // Fast path: if it's already a PyCodeObject, share the Arc (zero-clone)
    if let Ok(rust_code) = py_code.cast::<PyCodeObject>() {
        return Ok(Arc::clone(&rust_code.borrow().inner));
    }

    let mut code = CodeObject::new(
        py_code
            .getattr("name")?
            .extract::<String>()
            .unwrap_or_else(|_| "<code>".to_string()),
    );

    // Convert bytecode: list of (op, arg) tuples
    let bytecode = py_code.getattr("bytecode")?;
    let bc_tuple = bytecode.cast::<PyTuple>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(format!(
            "CodeObject.bytecode: expected tuple, got {}",
            bytecode
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string())
        ))
    })?;
    append_instructions_from_bytecode(bc_tuple, &mut code)?;

    // Convert constants
    let constants = py_code.getattr("constants")?;
    let const_tuple = constants.cast::<PyTuple>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(format!(
            "CodeObject.constants: expected tuple, got {}",
            constants
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string())
        ))
    })?;
    append_constants_from_tuple(py, const_tuple, &mut code)?;

    // Convert names
    let names = py_code.getattr("names")?;
    let names_tuple = names.cast::<PyTuple>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(format!(
            "CodeObject.names: expected tuple, got {}",
            names
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string())
        ))
    })?;
    for item in names_tuple.iter() {
        code.names.push(item.extract::<String>()?);
    }

    // Other attributes
    code.nlocals = py_code.getattr("nlocals")?.extract().unwrap_or(0);
    code.nargs = py_code.getattr("nargs")?.extract().unwrap_or(0);
    code.vararg_idx = py_code.getattr("vararg_idx")?.extract().unwrap_or(NO_VARARG_IDX);

    // Convert varnames
    let varnames = py_code.getattr("varnames")?;
    let var_tuple = varnames.cast::<PyTuple>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(format!(
            "CodeObject.varnames: expected tuple, got {}",
            varnames
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string())
        ))
    })?;
    for item in var_tuple.iter() {
        code.varnames.push(item.to_string());
    }

    // Convert slotmap
    let slotmap = py_code.getattr("slotmap")?;
    let dict = slotmap.cast::<PyDict>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(format!(
            "CodeObject.slotmap: expected dict, got {}",
            slotmap
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string())
        ))
    })?;
    for (key, value) in dict.iter() {
        let name = key.to_string();
        let slot: usize = value.extract()?;
        code.slotmap.insert(name, slot);
    }

    // Convert defaults
    let defaults = py_code.getattr("defaults")?;
    let def_tuple = defaults.cast::<PyTuple>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(format!(
            "CodeObject.defaults: expected tuple, got {}",
            defaults
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string())
        ))
    })?;
    for item in def_tuple.iter() {
        code.defaults.push(Value::from_pyobject(py, &item)?);
    }

    // Convert freevars
    let freevars = py_code.getattr("freevars")?;
    let fv_tuple = freevars.cast::<PyTuple>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(format!(
            "CodeObject.freevars: expected tuple, got {}",
            freevars
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string())
        ))
    })?;
    for item in fv_tuple.iter() {
        code.freevars.push(item.extract::<String>()?);
    }

    // Pure flag and complexity
    code.is_pure = py_code.getattr("is_pure").and_then(|v| v.extract()).unwrap_or(false);
    code.complexity = code.instructions.len();

    Ok(Arc::new(code))
}

/// Convert Python args to Value array.
pub fn convert_args(py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Vec<Value>> {
    let mut values = Vec::with_capacity(args.len());
    for item in args.iter() {
        values.push(Value::from_pyobject(py, &item)?);
    }
    Ok(values)
}

// ========== catnip_vm → catnip_rs conversion ==========

use super::pattern::{VMPattern as RsVMPattern, VMPatternElement as RsVMPatternElement};

/// Convert a catnip_vm::Value constant to a catnip_rs::Value.
///
/// Tags 0-3 and 6-7 share the same bit layout between both crates,
/// but BigInt uses different GmpInt types so we must extract and re-wrap.
/// Tags 8-13 (NativeStr, NativeList, etc.) are converted to PyObject.
/// VMFunc values are converted to PyCodeObject wraps using `sub_functions`.
fn convert_vm_value(
    py: Python<'_>,
    vm_val: catnip_vm::Value,
    sub_functions: &[catnip_vm::compiler::CodeObject],
) -> PyResult<Value> {
    if vm_val.is_float() {
        return Ok(Value::from_float(vm_val.as_float().unwrap()));
    }
    if vm_val.is_int() {
        return Ok(Value::from_int(vm_val.as_int().unwrap()));
    }
    if vm_val.is_bool() {
        return Ok(Value::from_bool(vm_val.as_bool().unwrap()));
    }
    if vm_val.is_nil() {
        return Ok(Value::NIL);
    }
    if vm_val.is_vmfunc() {
        let idx = vm_val.as_vmfunc_idx();
        if (idx as usize) < sub_functions.len() {
            let sub_code = convert_code_object_from_pure(py, &sub_functions[idx as usize], sub_functions)?;
            let py_code = Py::new(py, PyCodeObject::new(sub_code))?;
            return Value::from_pyobject(py, py_code.bind(py));
        }
        return Ok(Value::from_vmfunc(idx));
    }
    if vm_val.is_bigint() {
        let n = unsafe { vm_val.as_bigint_ref() }.unwrap();
        return Ok(Value::from_bigint(n.clone()));
    }
    if vm_val.is_native_str() {
        let s = unsafe { vm_val.as_native_str_ref() }.unwrap();
        let py_str = s.into_pyobject(py)?.into_any();
        return Value::from_pyobject(py, &py_str);
    }
    if vm_val.is_native_tuple() {
        let t = unsafe { vm_val.as_native_tuple_ref() }.unwrap();
        let slice = t.as_slice();
        let mut py_items: Vec<Py<pyo3::PyAny>> = Vec::with_capacity(slice.len());
        for &item in slice {
            let v = convert_vm_value(py, item, sub_functions)?;
            py_items.push(v.to_pyobject(py));
        }
        let py_tuple = PyTuple::new(py, &py_items)?;
        return Value::from_pyobject(py, &py_tuple.into_any());
    }
    if vm_val.is_native_list() {
        let l = unsafe { vm_val.as_native_list_ref() }.unwrap();
        let items = l.as_slice_cloned();
        let mut py_items: Vec<Py<pyo3::PyAny>> = Vec::with_capacity(items.len());
        for &item in &items {
            let v = convert_vm_value(py, item, sub_functions)?;
            py_items.push(v.to_pyobject(py));
            item.decref();
        }
        let py_list = pyo3::types::PyList::new(py, &py_items)?;
        return Value::from_pyobject(py, &py_list.into_any());
    }
    if vm_val.is_native_dict() {
        let d = unsafe { vm_val.as_native_dict_ref() }.unwrap();
        let keys = d.keys();
        let values = d.values();
        let py_dict = PyDict::new(py);
        for (k, v) in keys.into_iter().zip(values.into_iter()) {
            let py_k = convert_vm_value(py, k, sub_functions)?;
            let py_v = convert_vm_value(py, v, sub_functions)?;
            py_dict.set_item(py_k.to_pyobject(py), py_v.to_pyobject(py))?;
            v.decref();
        }
        return Value::from_pyobject(py, &py_dict.into_any());
    }
    if vm_val.is_native_set() {
        let s = unsafe { vm_val.as_native_set_ref() }.unwrap();
        let items = s.to_values();
        let py_set = pyo3::types::PySet::empty(py)?;
        for item in &items {
            let v = convert_vm_value(py, *item, sub_functions)?;
            py_set.add(v.to_pyobject(py))?;
        }
        return Value::from_pyobject(py, &py_set.into_any());
    }
    if vm_val.is_native_bytes() {
        let b = unsafe { vm_val.as_native_bytes_ref() }.unwrap();
        let py_bytes = pyo3::types::PyBytes::new(py, b.as_bytes());
        return Value::from_pyobject(py, &py_bytes.into_any());
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "cannot convert catnip_vm value: unsupported type",
    ))
}

/// Convert a catnip_vm VMPattern to a catnip_rs VMPattern.
fn convert_vm_pattern(
    py: Python<'_>,
    pat: &catnip_vm::compiler::VMPattern,
    sub_functions: &[catnip_vm::compiler::CodeObject],
) -> PyResult<RsVMPattern> {
    match pat {
        catnip_vm::compiler::VMPattern::Wildcard => Ok(RsVMPattern::Wildcard),
        catnip_vm::compiler::VMPattern::Literal(val) => {
            let rs_val = convert_vm_value(py, *val, sub_functions)?;
            Ok(RsVMPattern::Literal(rs_val))
        }
        catnip_vm::compiler::VMPattern::Var(slot) => Ok(RsVMPattern::Var(*slot)),
        catnip_vm::compiler::VMPattern::Or(pats) => {
            let converted: PyResult<Vec<_>> = pats.iter().map(|p| convert_vm_pattern(py, p, sub_functions)).collect();
            Ok(RsVMPattern::Or(converted?))
        }
        catnip_vm::compiler::VMPattern::Tuple(elems) => {
            let converted: PyResult<Vec<_>> = elems
                .iter()
                .map(|e| match e {
                    catnip_vm::compiler::VMPatternElement::Pattern(p) => {
                        Ok(RsVMPatternElement::Pattern(convert_vm_pattern(py, p, sub_functions)?))
                    }
                    catnip_vm::compiler::VMPatternElement::Star(slot) => Ok(RsVMPatternElement::Star(*slot)),
                })
                .collect();
            Ok(RsVMPattern::Tuple(converted?))
        }
        catnip_vm::compiler::VMPattern::Struct { name, field_slots } => Ok(RsVMPattern::Struct {
            name: name.clone(),
            field_slots: field_slots.clone(),
        }),
    }
}

/// Convert a catnip_vm CodeObject to a catnip_rs CodeObject.
///
/// `sub_functions` contains all compiled sub-functions (lambdas, methods).
/// VMFunc constants referencing indices into this array are converted to
/// PyCodeObject-backed Values.
fn convert_code_object_from_pure(
    py: Python<'_>,
    vm_code: &catnip_vm::compiler::CodeObject,
    sub_functions: &[catnip_vm::compiler::CodeObject],
) -> PyResult<CodeObject> {
    // Convert constants
    let mut rs_constants = Vec::with_capacity(vm_code.constants.len());
    for c in &vm_code.constants {
        if c.is_vmfunc() {
            let func_idx = c.as_vmfunc_idx();
            if (func_idx as usize) < sub_functions.len() {
                let sub_code = convert_code_object_from_pure(py, &sub_functions[func_idx as usize], sub_functions)?;
                let py_code = Py::new(py, PyCodeObject::new(sub_code))?;
                let val = Value::from_pyobject(py, py_code.bind(py))?;
                rs_constants.push(val);
                continue;
            }
        }
        rs_constants.push(convert_vm_value(py, *c, sub_functions)?);
    }

    // Convert patterns
    let mut rs_patterns = Vec::with_capacity(vm_code.patterns.len());
    for p in &vm_code.patterns {
        rs_patterns.push(convert_vm_pattern(py, p, sub_functions)?);
    }

    // Convert defaults
    let rs_defaults: Vec<Value> = vm_code
        .defaults
        .iter()
        .map(|d| convert_vm_value(py, *d, sub_functions))
        .collect::<PyResult<Vec<Value>>>()?;

    Ok(CodeObject {
        instructions: vm_code.instructions.clone(),
        constants: rs_constants,
        names: vm_code.names.clone(),
        nlocals: vm_code.nlocals,
        varnames: vm_code.varnames.clone(),
        slotmap: vm_code.slotmap.clone(),
        nargs: vm_code.nargs,
        defaults: rs_defaults,
        name: vm_code.name.clone(),
        freevars: vm_code.freevars.clone(),
        vararg_idx: vm_code.vararg_idx,
        is_pure: vm_code.is_pure,
        complexity: vm_code.complexity,
        line_table: vm_code.line_table.clone(),
        patterns: rs_patterns,
        bytecode_hash: std::sync::OnceLock::new(),
        encoded_ir: vm_code.encoded_ir.clone(),
    })
}

/// Convert a PureCompiler CompileOutput to a catnip_rs CodeObject.
pub fn convert_pure_compile_output(
    py: Python<'_>,
    output: &catnip_vm::compiler::CompileOutput,
) -> PyResult<CodeObject> {
    convert_code_object_from_pure(py, &output.code, &output.functions)
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
