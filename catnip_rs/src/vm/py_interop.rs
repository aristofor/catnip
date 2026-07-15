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
        super::core::VMError::IndexError(pyerr_message(py, &err))
    } else if err.is_instance_of::<pyo3::exceptions::PyKeyError>(py) {
        super::core::VMError::KeyError(pyerr_message(py, &err))
    } else if err.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
        super::core::VMError::ValueError(pyerr_message(py, &err))
    } else if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) {
        super::core::VMError::AttributeError(pyerr_message(py, &err))
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
    // Voie A: operands are owned and discarded by this op; `py_a`/`py_b` now
    // hold their own Python refs, so release the operand handles (even on the
    // error path below). Callers are exclusively owned-discard binop arms.
    if a.is_pyobj() {
        a.decref();
    }
    if b.is_pyobj() {
        b.decref();
    }
    let py_result = op.call1(py, (&py_a, &py_b)).to_vm(py)?;
    Value::from_pyobject(py, py_result.bind(py)).to_vm(py)
}

/// Python fallback for the binary bitwise ops (`&`, `|`, `^`) on non-integer
/// operands (numpy arrays, sets, types with custom `__and__`/`__or__`). The
/// native fast paths in `value_ops` only cover Catnip ints/bigints; this mirrors
/// the arithmetic ops' `host.binary_op` fallback for a family that has no
/// `BinaryOp` variant. `fn_name` is an `operator` module name (`and_`/`or_`/`xor`).
///
/// Consumes the owned pyobj operand refs like [`call_binary_op`] (Voie A), so the
/// caller must release the leftover operands with the non-pyobj variant
/// (`release_binop_operands`), never `release_operands`.
#[inline]
pub(crate) fn bitwise_binary_fallback(
    py: Python<'_>,
    fn_name: &str,
    a: Value,
    b: Value,
) -> Result<Value, super::core::VMError> {
    let op_fn = py.import("operator").and_then(|m| m.getattr(fn_name)).to_vm(py)?;
    call_binary_op(py, &op_fn.unbind(), a, b)
}

/// Python fallback for unary bitwise not (`~`) on non-integer operands. Borrows
/// `a` (does not decref it), so the caller keeps its existing single-operand
/// release (`decref_discard`).
#[inline]
pub(crate) fn bitwise_unary_fallback(py: Python<'_>, a: Value) -> Result<Value, super::core::VMError> {
    let py_a = a.to_pyobject(py);
    let invert = py.import("operator").and_then(|m| m.getattr("invert")).to_vm(py)?;
    let py_result = invert.call1((&py_a,)).to_vm(py)?;
    Value::from_pyobject(py, &py_result).to_vm(py)
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
    registry: &super::structs::StructRegistry,
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

/// Convert a catnip_vm value into a fresh Python reference. The intermediate
/// catnip_rs Value is only a carrier: its owned ref (an ObjectTable handle for
/// composite elements) is released here, the returned Python reference is the
/// only thing kept -- keeping the carrier leaked one slot per sub-object of
/// every composite constant (found by the ledger grid on `struct P { a }`).
fn convert_vm_value_to_py(
    py: Python<'_>,
    vm_val: catnip_vm::Value,
    sub_functions: &[catnip_vm::compiler::CodeObject],
) -> PyResult<Py<PyAny>> {
    let v = convert_vm_value(py, vm_val, sub_functions)?;
    let obj = v.to_pyobject(py);
    v.decref();
    Ok(obj)
}

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
        // SAFETY: is_bigint() checked the tag above, so the payload is a live GMP
        // Integer owned by vm_val; the borrow does not outlive this scope.
        let n = unsafe { vm_val.as_bigint_ref() }.unwrap();
        return Ok(Value::from_bigint(n.clone()));
    }
    if vm_val.is_native_str() {
        // SAFETY: is_native_str() checked the tag above, so the payload is a live
        // native string owned by vm_val; the borrow does not outlive it.
        let s = unsafe { vm_val.as_native_str_ref() }.unwrap();
        let py_str = s.into_pyobject(py)?.into_any();
        return Value::from_pyobject(py, &py_str);
    }
    if vm_val.is_native_tuple() {
        // SAFETY: is_native_tuple() checked the tag above, so the payload is a live
        // native tuple owned by vm_val; the borrow does not outlive it.
        let t = unsafe { vm_val.as_native_tuple_ref() }.unwrap();
        let slice = t.as_slice();
        let mut py_items: Vec<Py<pyo3::PyAny>> = Vec::with_capacity(slice.len());
        for &item in slice {
            py_items.push(convert_vm_value_to_py(py, item, sub_functions)?);
        }
        let py_tuple = PyTuple::new(py, &py_items)?;
        return Value::from_pyobject(py, &py_tuple.into_any());
    }
    if vm_val.is_native_list() {
        // SAFETY: is_native_list() checked the tag above, so the payload is a live
        // native list owned by vm_val; the borrow does not outlive it.
        let l = unsafe { vm_val.as_native_list_ref() }.unwrap();
        let items = l.as_slice_cloned();
        let mut py_items: Vec<Py<pyo3::PyAny>> = Vec::with_capacity(items.len());
        for &item in &items {
            py_items.push(convert_vm_value_to_py(py, item, sub_functions)?);
            item.decref();
        }
        let py_list = pyo3::types::PyList::new(py, &py_items)?;
        return Value::from_pyobject(py, &py_list.into_any());
    }
    if vm_val.is_native_dict() {
        // SAFETY: is_native_dict() checked the tag above, so the payload is a live
        // native dict owned by vm_val; the borrow does not outlive it.
        let d = unsafe { vm_val.as_native_dict_ref() }.unwrap();
        let keys = d.keys();
        let values = d.values();
        let py_dict = PyDict::new(py);
        for (k, v) in keys.into_iter().zip(values.into_iter()) {
            let py_k = convert_vm_value_to_py(py, k, sub_functions)?;
            let py_v = convert_vm_value_to_py(py, v, sub_functions)?;
            py_dict.set_item(py_k, py_v)?;
            // Both `keys()` (to_value) and `values()` (clone_refcount) hand back
            // OWNED values; convert_vm_value_to_py releases only the RS carrier,
            // not the catnip_vm source, so release both here (a heap key leaked
            // its Arc otherwise -- the value path already did this).
            k.decref();
            v.decref();
        }
        return Value::from_pyobject(py, &py_dict.into_any());
    }
    if vm_val.is_native_set() {
        // SAFETY: is_native_set() checked the tag above, so the payload is a live
        // native set owned by vm_val; the borrow does not outlive it.
        let s = unsafe { vm_val.as_native_set_ref() }.unwrap();
        let items = s.to_values();
        let py_set = pyo3::types::PySet::empty(py)?;
        for item in &items {
            py_set.add(convert_vm_value_to_py(py, *item, sub_functions)?)?;
            // `to_values` (to_value per element) hands back OWNED values; the
            // carrier release inside the conversion does not free the source.
            item.decref();
        }
        return Value::from_pyobject(py, &py_set.into_any());
    }
    if vm_val.is_native_bytes() {
        // SAFETY: is_native_bytes() checked the tag above, so the payload is a live
        // native bytes buffer owned by vm_val; the borrow does not outlive it.
        let b = unsafe { vm_val.as_native_bytes_ref() }.unwrap();
        let py_bytes = pyo3::types::PyBytes::new(py, b.as_bytes());
        return Value::from_pyobject(py, &py_bytes.into_any());
    }
    if vm_val.is_complex() {
        // SAFETY: is_complex() checked the tag above, so the payload is a live
        // complex value owned by vm_val; the read does not outlive it.
        let (r, i) = unsafe { vm_val.as_complex_parts() }.unwrap();
        return Ok(Value::from_complex(r, i));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "cannot convert catnip_vm value: unsupported type",
    ))
}

// ========== Native plugin bridge marshalling ==========
//
// Converts between PyObject and `catnip_vm::Value` so the PyO3 VM can call into
// native catnip_vm plugins (loaded via libloading). Reuses `convert_vm_value`
// for the catnip_vm -> Python direction.

/// Convert an owned `catnip_vm::Value` result into a PyObject, releasing the
/// owned reference. Plugin object handles are wrapped in `NativePluginObject`
/// (which takes ownership of the reference and drops it via the plugin's drop
/// callback on GC).
pub(crate) fn vm_value_to_py(py: Python<'_>, vm_val: catnip_vm::Value) -> PyResult<Py<PyAny>> {
    use crate::loader::native_plugin::NativePluginObject;

    if vm_val.is_plugin_object() {
        let obj = NativePluginObject::from_vm(vm_val);
        return Ok(Py::new(py, obj)?.into_any());
    }
    let py_obj = convert_vm_value_to_py(py, vm_val, &[])?;
    vm_val.decref();
    Ok(py_obj)
}

/// Convert a *borrowed* `catnip_vm::Value` into a PyObject without releasing
/// the SOURCE reference (the value stays owned by the plugin's namespace).
/// The intermediate catnip_rs carrier is still released.
pub(crate) fn vm_value_to_py_borrowed(py: Python<'_>, vm_val: catnip_vm::Value) -> PyResult<Py<PyAny>> {
    convert_vm_value_to_py(py, vm_val, &[])
}

/// Convert a PyObject into a fresh, owned `catnip_vm::Value` (an argument for a
/// plugin call). Callers must `decref` the returned value once the call
/// completes. A `NativePluginObject` is passed through by incrementing its
/// refcount so the post-call decref balances out.
pub(crate) fn convert_py_to_vm_value(obj: &Bound<'_, PyAny>) -> PyResult<catnip_vm::Value> {
    use catnip_vm::Value as VmValue;
    use pyo3::types::{PyBool, PyBytes, PyFloat, PyInt, PyList};

    if obj.is_none() {
        return Ok(VmValue::NIL);
    }
    if let Ok(b) = obj.cast::<PyBool>() {
        return Ok(VmValue::from_bool(b.is_true()));
    }
    if let Ok(po) = obj.cast::<crate::loader::native_plugin::NativePluginObject>() {
        let v = po.borrow().vm_value();
        v.clone_refcount();
        return Ok(v);
    }
    if obj.cast::<PyInt>().is_ok() {
        if let Ok(i) = obj.extract::<i64>() {
            return Ok(VmValue::from_int(i));
        }
        // Promote out-of-range ints to BigInt, symmetric with the VM->Py path
        // (convert_vm_value), instead of rejecting them.
        let n = crate::vm::value::pyobject_to_integer(obj)?;
        return Ok(VmValue::from_bigint(n));
    }
    if let Ok(f) = obj.cast::<PyFloat>() {
        return Ok(VmValue::from_float(f.extract()?));
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(VmValue::from_string(s));
    }
    if let Ok(b) = obj.cast::<PyBytes>() {
        return Ok(VmValue::from_bytes(b.as_bytes().to_vec()));
    }
    if let Ok(list) = obj.cast::<PyList>() {
        let mut items = Vec::with_capacity(list.len());
        for it in list.iter() {
            items.push(convert_py_to_vm_value(&it)?);
        }
        return Ok(VmValue::from_list(items));
    }
    if let Ok(tuple) = obj.cast::<PyTuple>() {
        let mut items = Vec::with_capacity(tuple.len());
        for it in tuple.iter() {
            items.push(convert_py_to_vm_value(&it)?);
        }
        return Ok(VmValue::from_tuple(items));
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map: IndexMap<catnip_vm::collections::ValueKey, VmValue> = IndexMap::new();
        for (k, v) in dict.iter() {
            let kv = convert_py_to_vm_value(&k)?;
            let key = kv
                .to_key()
                .map_err(|e| pyo3::exceptions::PyTypeError::new_err(e.to_string()))?;
            kv.decref();
            map.insert(key, convert_py_to_vm_value(&v)?);
        }
        return Ok(VmValue::from_dict(map));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(format!(
        "cannot pass value of type '{}' to a native plugin",
        obj.get_type().name()?
    )))
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
        catnip_vm::compiler::VMPattern::Struct {
            name,
            variant,
            field_slots,
        } => Ok(RsVMPattern::Struct {
            name: name.clone(),
            variant: variant.clone(),
            field_slots: field_slots.clone(),
        }),
        catnip_vm::compiler::VMPattern::Enum {
            enum_name,
            variant_name,
        } => Ok(RsVMPattern::Enum {
            enum_name: enum_name.clone(),
            variant_name: variant_name.clone(),
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
        union_checks: vm_code.union_checks.clone(),
        composite_checks: vm_code.composite_checks.clone(),
        generic_checks: vm_code.generic_checks.clone(),
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
