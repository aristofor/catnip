// FILE: catnip_rs/src/freeze.rs
//! Freeze/thaw: serialize Catnip values to bincode bytes and back.
//!
//! Two layers:
//! - `encode`/`decode` in catnip_core: raw bincode (transport, IPC)
//! - `freeze`/`thaw` here: PyO3 wrappers exposed as Catnip builtins

use crate::vm::value::Value;
use catnip_core::freeze::{FreezeError, FrozenValue};
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
    } else {
        None
    }
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
        _ => {
            let py_obj = frozen_to_py(py, frozen).unwrap_or_else(|_| py.None());
            Value::from_pyobject(py, py_obj.bind(py)).unwrap_or(Value::NIL)
        }
    }
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

/// Serialize a value to bincode bytes. Builtin: `freeze(value) -> bytes`
#[pyfunction]
pub fn freeze(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let frozen = py_to_frozen(obj)?;
    let bytes = catnip_core::freeze::encode(&frozen).map_err(freeze_err)?;
    Ok(PyBytes::new(py, &bytes).into_any().unbind())
}

/// Deserialize bincode bytes back to a value. Builtin: `thaw(bytes) -> value`
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
