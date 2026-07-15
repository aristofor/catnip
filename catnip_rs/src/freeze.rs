// FILE: catnip_rs/src/freeze.rs
//! Freeze/thaw: serialize Catnip values to postcard bytes and back.
//!
//! Two layers:
//! - `encode`/`decode` in catnip_core: raw postcard (transport, IPC)
//! - `freeze`/`thaw` here: PyO3 wrappers exposed as Catnip builtins

use crate::vm::value::Value;
use catnip_core::freeze::{FreezeError, FrozenField, FrozenMethod, FrozenStructType, FrozenValue};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyFrozenSet, PyInt, PyList, PySet, PyString, PyTuple};

// -- Error conversion --

fn freeze_err(e: FreezeError) -> PyErr {
    match e {
        FreezeError::SerializationError(_) => pyo3::exceptions::PyRuntimeError::new_err(e.to_string()),
        _ => pyo3::exceptions::PyValueError::new_err(e.to_string()),
    }
}

// -- PyObject <-> FrozenValue conversion --

fn py_to_frozen(obj: &Bound<'_, PyAny>) -> PyResult<FrozenValue> {
    // Order matters: PyBool before PyInt (bool is subclass of int in Python)
    if obj.is_none() {
        Ok(FrozenValue::None)
    } else if let Ok(b) = obj.cast::<PyBool>() {
        Ok(FrozenValue::Bool(b.is_true()))
    } else if let Ok(_i) = obj.cast::<PyInt>() {
        let val: i64 = obj
            .extract()
            .map_err(|_| pyo3::exceptions::PyOverflowError::new_err("integer too large to freeze (max i64)"))?;
        Ok(FrozenValue::Int(val))
    } else if let Ok(f) = obj.cast::<PyFloat>() {
        Ok(FrozenValue::Float(f.value()))
    } else if let Ok(s) = obj.cast::<PyString>() {
        Ok(FrozenValue::String(s.to_string()))
    } else if let Ok(b) = obj.cast::<PyBytes>() {
        Ok(FrozenValue::Bytes(b.as_bytes().to_vec()))
    } else if let Ok(list) = obj.cast::<PyList>() {
        let items: PyResult<Vec<_>> = list.iter().map(|item| py_to_frozen(&item)).collect();
        Ok(FrozenValue::List(items?))
    } else if let Ok(tup) = obj.cast::<PyTuple>() {
        let items: PyResult<Vec<_>> = tup.iter().map(|item| py_to_frozen(&item)).collect();
        Ok(FrozenValue::Tuple(items?))
    } else if let Ok(dict) = obj.cast::<PyDict>() {
        let mut entries = Vec::with_capacity(dict.len());
        for (k, v) in dict.iter() {
            entries.push((py_to_frozen(&k)?, py_to_frozen(&v)?));
        }
        Ok(FrozenValue::Dict(entries))
    } else if let Ok(set) = obj.cast::<PySet>() {
        let items: PyResult<Vec<_>> = set.iter().map(|item| py_to_frozen(&item)).collect();
        Ok(FrozenValue::Set(items?))
    } else if let Ok(set) = obj.cast::<PyFrozenSet>() {
        let items: PyResult<Vec<_>> = set.iter().map(|item| py_to_frozen(&item)).collect();
        Ok(FrozenValue::Set(items?))
    } else {
        let type_name = obj.get_type().name()?;
        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "cannot freeze type '{type_name}'"
        )))
    }
}

fn frozen_to_py(py: Python<'_>, val: &FrozenValue) -> PyResult<Py<PyAny>> {
    match val {
        FrozenValue::None => Ok(py.None()),
        FrozenValue::Bool(b) => Ok(b.into_pyobject(py)?.to_owned().into_any().unbind()),
        FrozenValue::Int(i) => Ok(i.into_pyobject(py)?.into_any().unbind()),
        FrozenValue::Float(f) => Ok(f.into_pyobject(py)?.into_any().unbind()),
        FrozenValue::String(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
        FrozenValue::Bytes(b) => Ok(PyBytes::new(py, b).into_any().unbind()),
        FrozenValue::List(items) => {
            let py_items: PyResult<Vec<_>> = items.iter().map(|v| frozen_to_py(py, v)).collect();
            Ok(PyList::new(py, py_items?)?.into_any().unbind())
        }
        FrozenValue::Tuple(items) => {
            let py_items: PyResult<Vec<_>> = items.iter().map(|v| frozen_to_py(py, v)).collect();
            Ok(PyTuple::new(py, py_items?)?.into_any().unbind())
        }
        FrozenValue::Dict(entries) => {
            let dict = PyDict::new(py);
            for (k, v) in entries {
                dict.set_item(frozen_to_py(py, k)?, frozen_to_py(py, v)?)?;
            }
            Ok(dict.into_any().unbind())
        }
        FrozenValue::Set(items) => {
            let set = PySet::empty(py)?;
            for item in items {
                set.add(frozen_to_py(py, item)?)?;
            }
            Ok(set.into_any().unbind())
        }
        // Structs thaw nominally against a StructRegistry, not to a plain
        // PyObject -- that path lives in the registry-aware `frozen_to_value`
        // (step C). Reaching here means a struct crossed a registry-less
        // boundary, which the gating is meant to prevent.
        FrozenValue::Struct { type_name, .. } => Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "cannot thaw struct '{type_name}' without a struct registry"
        ))),
    }
}

// -- Value <-> FrozenValue bridge --
// Native types bypass Python entirely; PyObjects delegate to py_to_frozen.

/// Convert a NaN-boxed Value to FrozenValue. Returns None if the value
/// is not freezable (VMFunction, struct instance, BigInt, etc.).
pub fn value_to_frozen(py: Python<'_>, val: Value) -> Option<FrozenValue> {
    if val.is_nil() {
        Some(FrozenValue::None)
    } else if let Some(b) = val.as_bool() {
        Some(FrozenValue::Bool(b))
    } else if let Some(i) = val.as_int() {
        Some(FrozenValue::Int(i))
    } else if let Some(f) = val.as_float() {
        Some(FrozenValue::Float(f))
    } else if val.is_pyobj() {
        let py_obj = val.as_pyobject(py)?;
        py_to_frozen(py_obj.bind(py)).ok()
    } else if val.is_struct_instance() {
        struct_to_frozen(py, val)
    } else {
        None
    }
}

/// Freeze a `TAG_STRUCT` instance nominally, reading the current thread-local
/// `StructRegistry` (installed during a broadcast/ND, like `Value::to_pyobject`).
/// Returns None if no registry is installed or any field isn't freezable.
fn struct_to_frozen(py: Python<'_>, val: Value) -> Option<FrozenValue> {
    let idx = val.as_struct_instance_idx()?;
    let ptr = crate::vm::value::save_struct_registry();
    if ptr.is_null() {
        return None;
    }
    // SAFETY: ptr is non-null and names the live thread-local StructRegistry,
    // single-threaded under the GIL -- same contract as Value::to_pyobject.
    let registry = unsafe { &*ptr };
    let (type_id, field_vals) = registry.with_instance(idx, |inst| (inst.type_id, inst.fields.clone()))?;
    // Field names live on the type; drop the borrow before the recursive calls.
    let (type_name, field_names): (String, Vec<String>) = {
        let ty = registry.get_type(type_id)?;
        (ty.name.clone(), ty.fields.iter().map(|f| f.name.clone()).collect())
    };
    let mut fields = Vec::with_capacity(field_vals.len());
    for (name, fval) in field_names.into_iter().zip(field_vals) {
        fields.push((name, value_to_frozen(py, fval)?));
    }
    Some(FrozenValue::Struct { type_name, fields })
}

/// Convert a FrozenValue back to a NaN-boxed Value.
pub fn frozen_to_value(py: Python<'_>, frozen: &FrozenValue) -> Value {
    match frozen {
        FrozenValue::None => Value::NIL,
        FrozenValue::Bool(b) => {
            if *b {
                Value::TRUE
            } else {
                Value::FALSE
            }
        }
        FrozenValue::Int(i) => Value::from_int(*i),
        FrozenValue::Float(f) => Value::from_float(*f),
        FrozenValue::Struct { type_name, fields } => struct_from_frozen(py, type_name, fields),
        _ => {
            let py_obj = frozen_to_py(py, frozen).unwrap_or_else(|_| py.None());
            Value::from_pyobject(py, py_obj.bind(py)).unwrap_or(Value::NIL)
        }
    }
}

/// Thaw a frozen struct against the current thread-local `StructRegistry`: the
/// type must already be registered by name (the collector ships it first).
/// Fields (keyed by name) are placed into the type's positional order via
/// `field_index`, so the parent's and worker's field order need not match.
/// Returns NIL if no registry is installed or the type is unknown -- a clean
/// failure, never a wrong instance.
fn struct_from_frozen(py: Python<'_>, type_name: &str, fields: &[(String, FrozenValue)]) -> Value {
    let ptr = crate::vm::value::save_struct_registry();
    if ptr.is_null() {
        return Value::NIL;
    }
    // SAFETY: same contract as struct_to_frozen / Value::to_pyobject.
    let registry = unsafe { &*ptr };
    // Resolve type_id, arity, and each frozen field's target slot, then drop the
    // borrow before the recursive frozen_to_value calls (which may re-enter).
    let (type_id, arity, positions): (u32, usize, Vec<Option<usize>>) = match registry.find_type_by_name(type_name) {
        Some(ty) => (
            ty.id,
            ty.fields.len(),
            fields.iter().map(|(name, _)| ty.field_index(name)).collect(),
        ),
        None => return Value::NIL,
    };
    let mut positional: Vec<Value> = vec![Value::NIL; arity];
    for ((_, fval), pos) in fields.iter().zip(positions) {
        if let Some(p) = pos {
            positional[p] = frozen_to_value(py, fval);
        }
    }
    let idx = registry.create_instance(type_id, positional);
    Value::from_struct_instance(idx)
}

/// True if a frozen value is or contains a struct. Step-C guard: a struct
/// freezes (above) but its `type_defs` aren't shipped to the worker yet
/// (steps D/E), so the ND process path must fall back rather than send a struct
/// the worker would thaw to NIL -- a wrong result. This becomes the wiring
/// point for `type_defs` in step E.
pub fn frozen_has_struct(v: &FrozenValue) -> bool {
    match v {
        FrozenValue::Struct { .. } => true,
        FrozenValue::List(xs) | FrozenValue::Tuple(xs) | FrozenValue::Set(xs) => xs.iter().any(frozen_has_struct),
        FrozenValue::Dict(kvs) => kvs
            .iter()
            .any(|(k, val)| frozen_has_struct(k) || frozen_has_struct(val)),
        _ => false,
    }
}

// -- Struct type collection (ND process, step C) --

/// Collect every live struct type from a registry as serializable
/// `FrozenStructType`s to embed in a worker command (D3: over-approximate --
/// ship all types, the worker registers by name and ignores the unused ones).
///
/// Returns None -- the parent falls back to the Python path -- as soon as a
/// type is outside the v1 frontier: `extends`/`implements`/abstract, a
/// non-freezable field default, or a method whose IR isn't frozen or that
/// captures a closure (v1 transports no method captures).
pub fn collect_frozen_struct_types(
    py: Python<'_>,
    registry: &crate::vm::structs::StructRegistry,
) -> Option<Vec<FrozenStructType>> {
    let mut out = Vec::new();
    for ty in registry.iter_types() {
        // v1 frontier: flat structs only.
        if !ty.parent_names.is_empty() || !ty.implements.is_empty() || !ty.abstract_methods.is_empty() {
            return None;
        }
        let mut fields = Vec::with_capacity(ty.fields.len());
        for f in &ty.fields {
            let default = value_to_frozen(py, f.default)?;
            // A struct default would force the worker to thaw a struct while the
            // registry is borrowed mut for registration (step D) -- an aliasing
            // hazard. Rare; fall back rather than special-case it.
            if frozen_has_struct(&default) {
                return None;
            }
            fields.push(FrozenField {
                name: f.name.clone(),
                has_default: f.has_default,
                default,
                check: f.check.clone(),
            });
        }
        let mut methods = Vec::with_capacity(ty.methods.len() + ty.static_methods.len());
        for (name, m) in &ty.methods {
            methods.push(method_to_frozen(py, name, m, false)?);
        }
        for (name, m) in &ty.static_methods {
            methods.push(method_to_frozen(py, name, m, true)?);
        }
        out.push(FrozenStructType {
            name: ty.name.clone(),
            fields,
            methods,
        });
    }
    Some(out)
}

// -- Struct type registration (ND process worker, step D) --

/// Register frozen struct types into a worker's `StructRegistry`, idempotent by
/// name: a type already present (respawn / repeated batch) is skipped. Each
/// method is recompiled from its frozen IR into a `VMFunction` (no captures --
/// the collector gates methods that capture, step C). v1 registers flat types
/// only (no parents/traits/abstract), matching the collector's frontier.
///
/// The registry is borrowed mut for registration; field defaults are struct-free
/// (gated by the collector) so thawing them touches no registry, and method
/// compilation reads none -- no aliasing with the thread-local registry pointer.
pub fn register_frozen_struct_types(
    py: Python<'_>,
    registry: &mut crate::vm::structs::StructRegistry,
    context: Option<&Py<PyAny>>,
    type_defs: &[FrozenStructType],
) -> Result<(), String> {
    use crate::vm::structs::{StructField, StructMethods, StructParents};
    for td in type_defs {
        if registry.find_type_by_name(&td.name).is_some() {
            continue; // register-if-absent (idempotent by name)
        }
        let fields: Vec<StructField> = td
            .fields
            .iter()
            .map(|f| StructField {
                name: f.name.clone(),
                has_default: f.has_default,
                default: frozen_to_value(py, &f.default),
                check: f.check.clone(),
            })
            .collect();
        let mut instance = indexmap::IndexMap::new();
        let mut statics = indexmap::IndexMap::new();
        for m in &td.methods {
            let func = compile_frozen_method(py, m, context)?;
            if m.is_static {
                statics.insert(m.name.clone(), func);
            } else {
                instance.insert(m.name.clone(), func);
            }
        }
        registry.register_type_with_parents(
            td.name.clone(),
            fields,
            StructMethods {
                instance,
                statics,
                abstract_methods: Default::default(),
            },
            StructParents {
                implements: vec![],
                mro: vec![],
                parent_names: vec![],
            },
        );
    }
    Ok(())
}

/// Recompile a frozen method's IR into a native `VMFunction` (no closure -- v1
/// gates capturing methods), mirroring the lambda compile in `execute_worker_task`.
fn compile_frozen_method(
    py: Python<'_>,
    method: &FrozenMethod,
    context: Option<&Py<PyAny>>,
) -> Result<Py<PyAny>, String> {
    use crate::vm::frame::{PyCodeObject, VMFunction};
    use crate::vm::unified_compiler::{FunctionCompileMeta, UnifiedCompiler};

    let ir: Vec<catnip_core::ir::IR> =
        catnip_core::freeze::decode(&method.encoded_ir).map_err(|e| format!("worker method decode: {e}"))?;
    if ir.is_empty() {
        return Err(format!("worker method '{}': empty IR", method.name));
    }
    let mut compiler = UnifiedCompiler::new();
    let param_types = match ir.get(1) {
        Some(p) => compiler.param_type_codes(py, p),
        None => Vec::new(),
    };
    let code = compiler
        .compile_function_pure(
            py,
            &ir[0],
            FunctionCompileMeta {
                params: method.param_names.clone(),
                param_types,
                name: &method.name,
                defaults: vec![],
                vararg_idx: -1,
                parent_nesting_depth: 0,
            },
        )
        .map_err(|e| format!("worker method '{}' compile: {e}", method.name))?;
    let code_py = Py::new(
        py,
        PyCodeObject {
            inner: std::sync::Arc::new(code),
        },
    )
    .map_err(|e| e.to_string())?;
    let func = Py::new(
        py,
        VMFunction::create_native(py, code_py, None, context.map(|c| c.clone_ref(py))),
    )
    .map_err(|e| e.to_string())?;
    Ok(func.into_any())
}

/// Freeze one struct method (a `VMFunction`) to a `FrozenMethod`. Returns None
/// if the method's IR isn't frozen (non-pure body) or it captures a closure
/// (v1 ships no method captures -- reconstructing from IR alone would drop
/// them, so we fall back rather than return a wrong result).
fn method_to_frozen(py: Python<'_>, name: &str, method: &Py<PyAny>, is_static: bool) -> Option<FrozenMethod> {
    let vm_func: PyRef<'_, crate::vm::frame::VMFunction> = method.bind(py).extract().ok()?;
    if let Some(ref scope) = vm_func.native_closure {
        if !scope.captured_entries().is_empty() {
            return None;
        }
    }
    let code = vm_func.vm_code.borrow(py);
    let encoded_ir = code.inner.encoded_ir.as_ref()?.as_ref().clone();
    let param_names = code.inner.varnames[..code.inner.nargs].to_vec();
    Some(FrozenMethod {
        name: name.to_string(),
        encoded_ir,
        param_names,
        is_static,
    })
}

// -- Closure captures --

/// Freeze all captured variables in a NativeClosureScope.
/// Returns None if any captured value is not freezable.
pub fn freeze_captures(
    py: Python<'_>,
    scope: &crate::vm::frame::NativeClosureScope,
) -> Option<Vec<(String, FrozenValue)>> {
    let entries = scope.captured_entries();
    let mut frozen = Vec::with_capacity(entries.len());
    for (name, val) in entries {
        frozen.push((name, value_to_frozen(py, val)?));
    }
    Some(frozen)
}

/// Reconstruct a NativeClosureScope from frozen captures (no parent).
/// Parent is not serialized -- same as ClosureScope.__reduce__ which drops parent.
pub fn thaw_captures(py: Python<'_>, captures: &[(String, FrozenValue)]) -> crate::vm::frame::NativeClosureScope {
    let map: indexmap::IndexMap<String, Value> = captures
        .iter()
        .map(|(name, fv)| (name.clone(), frozen_to_value(py, fv)))
        .collect();
    crate::vm::frame::NativeClosureScope::without_parent(map)
}

// -- PyO3 builtins --

/// Serialize a value to postcard bytes. Builtin: `freeze(value) -> bytes`
#[pyfunction]
pub fn freeze(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let frozen = py_to_frozen(obj)?;
    let bytes = catnip_core::freeze::encode(&frozen).map_err(freeze_err)?;
    Ok(PyBytes::new(py, &bytes).into_any().unbind())
}

/// Deserialize postcard bytes back to a value. Builtin: `thaw(bytes) -> value`
#[pyfunction]
pub fn thaw(py: Python<'_>, data: &[u8]) -> PyResult<Py<PyAny>> {
    let frozen: FrozenValue = catnip_core::freeze::decode(data).map_err(freeze_err)?;
    frozen_to_py(py, &frozen)
}

/// Register freeze/thaw functions in the module.
pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(freeze, m)?)?;
    m.add_function(wrap_pyfunction!(thaw, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_to_frozen_nil() {
        Python::attach(|py| {
            let result = value_to_frozen(py, Value::NIL);
            assert_eq!(result, Some(FrozenValue::None));
        });
    }

    #[test]
    fn test_value_to_frozen_bool() {
        Python::attach(|py| {
            assert_eq!(value_to_frozen(py, Value::TRUE), Some(FrozenValue::Bool(true)));
            assert_eq!(value_to_frozen(py, Value::FALSE), Some(FrozenValue::Bool(false)));
        });
    }

    #[test]
    fn test_value_to_frozen_int() {
        Python::attach(|py| {
            assert_eq!(value_to_frozen(py, Value::from_int(42)), Some(FrozenValue::Int(42)));
            assert_eq!(value_to_frozen(py, Value::from_int(-100)), Some(FrozenValue::Int(-100)));
            assert_eq!(value_to_frozen(py, Value::from_int(0)), Some(FrozenValue::Int(0)));
        });
    }

    #[test]
    fn test_value_to_frozen_float() {
        let expected = std::f64::consts::PI;
        Python::attach(|py| match value_to_frozen(py, Value::from_float(expected)) {
            Some(FrozenValue::Float(f)) => assert!((f - expected).abs() < 1e-10),
            other => panic!("expected Float, got {:?}", other),
        });
    }

    #[test]
    fn test_value_to_frozen_string() {
        Python::attach(|py| {
            let s = "hello".into_pyobject(py).unwrap().into_any();
            let val = Value::from_pyobject(py, &s).unwrap();
            assert_eq!(value_to_frozen(py, val), Some(FrozenValue::String("hello".into())));
        });
    }

    #[test]
    fn test_value_to_frozen_list() {
        Python::attach(|py| {
            let list = PyList::new(py, [1i64, 2, 3]).unwrap();
            let val = Value::from_pyobject(py, list.as_any()).unwrap();
            assert_eq!(
                value_to_frozen(py, val),
                Some(FrozenValue::List(vec![
                    FrozenValue::Int(1),
                    FrozenValue::Int(2),
                    FrozenValue::Int(3)
                ]))
            );
        });
    }

    #[test]
    fn test_frozen_to_value_roundtrip() {
        Python::attach(|py| {
            assert!(frozen_to_value(py, &FrozenValue::None).is_nil());
            assert_eq!(frozen_to_value(py, &FrozenValue::Bool(true)).as_bool(), Some(true));
            assert_eq!(frozen_to_value(py, &FrozenValue::Int(42)).as_int(), Some(42));
            assert_eq!(frozen_to_value(py, &FrozenValue::Float(2.5)).as_float(), Some(2.5));
        });
    }

    #[test]
    fn test_frozen_to_value_string() {
        Python::attach(|py| {
            let val = frozen_to_value(py, &FrozenValue::String("test".into()));
            assert!(val.is_pyobj());
            let s: String = val.to_pyobject(py).extract(py).unwrap();
            assert_eq!(s, "test");
        });
    }

    #[test]
    fn test_value_frozen_roundtrip_all_types() {
        Python::attach(|py| {
            let test_values = vec![
                Value::NIL,
                Value::TRUE,
                Value::FALSE,
                Value::from_int(0),
                Value::from_int(70_368_744_177_663),
                Value::from_float(0.0),
                Value::from_float(f64::INFINITY),
            ];
            for original in test_values {
                let frozen = value_to_frozen(py, original).expect("should be freezable");
                let restored = frozen_to_value(py, &frozen);
                if original.is_nil() {
                    assert!(restored.is_nil());
                } else if let Some(b) = original.as_bool() {
                    assert_eq!(restored.as_bool(), Some(b));
                } else if let Some(i) = original.as_int() {
                    assert_eq!(restored.as_int(), Some(i));
                } else if let Some(f) = original.as_float() {
                    if f.is_nan() {
                        assert!(restored.as_float().unwrap().is_nan());
                    } else {
                        assert_eq!(restored.as_float(), Some(f));
                    }
                }
            }
        });
    }

    fn register_flat_type(reg: &mut crate::vm::structs::StructRegistry, name: &str, field_names: &[&str]) -> u32 {
        use crate::vm::structs::{StructField, StructMethods, StructParents};
        use catnip_core::vm::opcode::ParamCheck;
        use indexmap::IndexMap;
        let fields = field_names
            .iter()
            .map(|n| StructField {
                name: (*n).into(),
                has_default: false,
                default: Value::NIL,
                check: ParamCheck::None,
            })
            .collect();
        reg.register_type_with_parents(
            name.into(),
            fields,
            StructMethods {
                instance: IndexMap::new(),
                statics: IndexMap::new(),
                abstract_methods: Default::default(),
            },
            StructParents {
                implements: vec![],
                mro: vec![],
                parent_names: vec![],
            },
        )
    }

    #[test]
    fn test_struct_freeze_thaw_roundtrip_reordered() {
        use crate::vm::structs::StructRegistry;
        Python::attach(|py| {
            let mut reg = StructRegistry::new();
            let tid = register_flat_type(&mut reg, "P", &["x", "y"]);
            let idx = reg.create_instance(tid, vec![Value::from_int(1), Value::from_int(2)]);

            crate::vm::value::set_struct_registry(&reg as *const _);
            let frozen = value_to_frozen(py, Value::from_struct_instance(idx)).expect("struct freezes");
            crate::vm::value::clear_struct_registry();

            match &frozen {
                FrozenValue::Struct { type_name, fields } => {
                    assert_eq!(type_name, "P");
                    assert_eq!(fields[0], ("x".into(), FrozenValue::Int(1)));
                    assert_eq!(fields[1], ("y".into(), FrozenValue::Int(2)));
                }
                other => panic!("expected Struct, got {:?}", other),
            }

            // Fresh registry with REVERSED field order: reorder-by-name must hold.
            let mut fresh = StructRegistry::new();
            register_flat_type(&mut fresh, "P", &["y", "x"]);
            crate::vm::value::set_struct_registry(&fresh as *const _);
            let restored = frozen_to_value(py, &frozen);
            let ridx = restored.as_struct_instance_idx().expect("thaws to a struct");
            let vals = fresh.with_instance(ridx, |inst| inst.fields.clone()).unwrap();
            crate::vm::value::clear_struct_registry();

            // fresh order is [y, x]: slot 0 = y = 2, slot 1 = x = 1.
            assert_eq!(vals[0].as_int(), Some(2));
            assert_eq!(vals[1].as_int(), Some(1));
        });
    }

    #[test]
    fn test_struct_freeze_nested() {
        use crate::vm::structs::StructRegistry;
        Python::attach(|py| {
            let mut reg = StructRegistry::new();
            let qid = register_flat_type(&mut reg, "Q", &["v"]);
            let pid = register_flat_type(&mut reg, "P", &["inner"]);
            let q = reg.create_instance(qid, vec![Value::from_int(7)]);
            let p = reg.create_instance(pid, vec![Value::from_struct_instance(q)]);

            crate::vm::value::set_struct_registry(&reg as *const _);
            let frozen = value_to_frozen(py, Value::from_struct_instance(p)).expect("nested struct freezes");
            crate::vm::value::clear_struct_registry();

            match &frozen {
                FrozenValue::Struct { type_name, fields } => {
                    assert_eq!(type_name, "P");
                    match &fields[0].1 {
                        FrozenValue::Struct {
                            type_name: inner,
                            fields: ifields,
                        } => {
                            assert_eq!(inner, "Q");
                            assert_eq!(ifields[0], ("v".into(), FrozenValue::Int(7)));
                        }
                        other => panic!("expected nested Struct, got {:?}", other),
                    }
                }
                other => panic!("expected Struct, got {:?}", other),
            }
        });
    }

    #[test]
    fn test_frozen_has_struct() {
        let s = FrozenValue::Struct {
            type_name: "P".into(),
            fields: vec![],
        };
        assert!(frozen_has_struct(&s));
        assert!(frozen_has_struct(&FrozenValue::List(vec![
            FrozenValue::Int(1),
            s.clone()
        ])));
        assert!(frozen_has_struct(&FrozenValue::Tuple(vec![s.clone()])));
        assert!(frozen_has_struct(&FrozenValue::Dict(vec![(
            FrozenValue::Int(1),
            s.clone()
        )])));
        assert!(!frozen_has_struct(&FrozenValue::List(vec![
            FrozenValue::Int(1),
            FrozenValue::None
        ])));
        assert!(!frozen_has_struct(&FrozenValue::Int(5)));
    }

    #[test]
    fn test_collect_flat_type() {
        use crate::vm::structs::StructRegistry;
        Python::attach(|py| {
            let mut reg = StructRegistry::new();
            register_flat_type(&mut reg, "P", &["x", "y"]);
            let defs = collect_frozen_struct_types(py, &reg).expect("flat type collects");
            assert_eq!(defs.len(), 1);
            assert_eq!(defs[0].name, "P");
            assert_eq!(defs[0].fields.len(), 2);
            assert_eq!(defs[0].fields[0].name, "x");
            assert_eq!(defs[0].fields[1].name, "y");
            assert!(defs[0].methods.is_empty());
        });
    }

    #[test]
    fn test_collect_gates_v1_frontier() {
        use crate::vm::structs::{MethodKey, MethodKind, StructMethods, StructParents, StructRegistry};
        use indexmap::IndexMap;
        let empty_methods = || StructMethods {
            instance: IndexMap::new(),
            statics: IndexMap::new(),
            abstract_methods: Default::default(),
        };
        Python::attach(|py| {
            // extends -> fallback
            let mut reg = StructRegistry::new();
            reg.register_type_with_parents(
                "Child".into(),
                vec![],
                empty_methods(),
                StructParents {
                    implements: vec![],
                    mro: vec![],
                    parent_names: vec!["Base".into()],
                },
            );
            assert!(
                collect_frozen_struct_types(py, &reg).is_none(),
                "extends must fall back"
            );

            // implements -> fallback
            let mut reg2 = StructRegistry::new();
            reg2.register_type_with_parents(
                "Impl".into(),
                vec![],
                empty_methods(),
                StructParents {
                    implements: vec!["Trait".into()],
                    mro: vec![],
                    parent_names: vec![],
                },
            );
            assert!(
                collect_frozen_struct_types(py, &reg2).is_none(),
                "implements must fall back"
            );

            // abstract method -> fallback
            let mut reg3 = StructRegistry::new();
            let mut abs = std::collections::HashSet::new();
            abs.insert(MethodKey {
                name: "foo".into(),
                kind: MethodKind::Instance,
            });
            reg3.register_type_with_parents(
                "Abs".into(),
                vec![],
                StructMethods {
                    instance: IndexMap::new(),
                    statics: IndexMap::new(),
                    abstract_methods: abs,
                },
                StructParents {
                    implements: vec![],
                    mro: vec![],
                    parent_names: vec![],
                },
            );
            assert!(
                collect_frozen_struct_types(py, &reg3).is_none(),
                "abstract must fall back"
            );
        });
    }

    #[test]
    fn test_register_frozen_types_fields_and_idempotent() {
        use crate::vm::structs::StructRegistry;
        use catnip_core::freeze::{FrozenField, FrozenStructType};
        use catnip_core::vm::opcode::ParamCheck;
        Python::attach(|py| {
            let td = FrozenStructType {
                name: "P".into(),
                fields: vec![
                    FrozenField {
                        name: "x".into(),
                        has_default: false,
                        default: FrozenValue::None,
                        check: ParamCheck::None,
                    },
                    FrozenField {
                        name: "y".into(),
                        has_default: true,
                        default: FrozenValue::Int(5),
                        check: ParamCheck::Primitive(3),
                    },
                ],
                methods: vec![],
            };
            let mut reg = StructRegistry::new();
            register_frozen_struct_types(py, &mut reg, None, std::slice::from_ref(&td)).unwrap();

            let ty = reg.find_type_by_name("P").expect("type registered");
            assert_eq!(ty.fields.len(), 2);
            assert_eq!(ty.fields[0].name, "x");
            assert!(ty.fields[1].has_default);
            assert_eq!(ty.fields[1].default.as_int(), Some(5));

            // Idempotent by name: a second registration is a no-op.
            register_frozen_struct_types(py, &mut reg, None, std::slice::from_ref(&td)).unwrap();
            assert_eq!(reg.iter_types().filter(|t| t.name == "P").count(), 1);
        });
    }

    #[test]
    fn test_register_frozen_type_compiles_method() {
        use crate::vm::structs::StructRegistry;
        use catnip_core::freeze::{FrozenMethod, FrozenStructType};
        Python::attach(|py| {
            // Trivial method body IR (returns 42) exercises compile_frozen_method
            // without needing the full execution machinery (that's step E).
            let method_ir = catnip_core::freeze::encode(&vec![catnip_core::ir::IR::Int(42)]).unwrap();
            let td = FrozenStructType {
                name: "Q".into(),
                fields: vec![],
                methods: vec![FrozenMethod {
                    name: "answer".into(),
                    encoded_ir: method_ir,
                    param_names: vec!["self".into()],
                    is_static: false,
                }],
            };
            let mut reg = StructRegistry::new();
            register_frozen_struct_types(py, &mut reg, None, std::slice::from_ref(&td)).unwrap();
            let ty = reg.find_type_by_name("Q").expect("type registered");
            assert!(ty.methods.contains_key("answer"), "method compiled and registered");
        });
    }

    #[test]
    fn test_freeze_captures_roundtrip() {
        Python::attach(|py| {
            use indexmap::IndexMap;
            let expected = std::f64::consts::PI;
            let mut map = IndexMap::new();
            map.insert("x".to_string(), Value::from_int(42));
            map.insert("y".to_string(), Value::TRUE);
            map.insert("z".to_string(), Value::from_float(expected));

            let scope = crate::vm::frame::NativeClosureScope::without_parent(map);
            let frozen = freeze_captures(py, &scope).expect("all values should be freezable");
            assert_eq!(frozen.len(), 3);

            let restored = thaw_captures(py, &frozen);
            assert_eq!(restored.resolve("x"), Some(Value::from_int(42)));
            assert_eq!(restored.resolve("y"), Some(Value::TRUE));
            match restored.resolve("z") {
                Some(v) => assert!((v.as_float().unwrap() - expected).abs() < 1e-10),
                None => panic!("z not found"),
            }
        });
    }

    #[test]
    fn test_thaw_captures_empty() {
        Python::attach(|py| {
            let scope = thaw_captures(py, &[]);
            assert_eq!(scope.captured_entries().len(), 0);
        });
    }
}
