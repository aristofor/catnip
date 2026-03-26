// FILE: catnip_rs/src/ir/json.rs
//! JSON serialization bridge for PyO3
//!
//! Exposes IR JSON serialization to Python scripts.

use super::pure::IR;
use indexmap::IndexMap;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

/// Convert a Python object (Op or IR) to Rust IR
pub fn python_to_ir_pure(_py: Python, obj: &Bound<'_, PyAny>) -> PyResult<IR> {
    // Fast path: already a PyIRNode
    if let Ok(node) = obj.cast::<super::pyclass::PyIRNode>() {
        return Ok(node.borrow().inner.clone());
    }

    // Essayer d'extraire comme Int (avant bool car bool peut être extrait comme int)
    if obj.is_instance_of::<pyo3::types::PyInt>() {
        if let Ok(val) = obj.extract::<i64>() {
            return Ok(IR::Int(val));
        }
    }

    // Essayer d'extraire comme Float
    if obj.is_instance_of::<pyo3::types::PyFloat>() {
        if let Ok(val) = obj.extract::<f64>() {
            return Ok(IR::Float(val));
        }
    }

    // Essayer d'extraire comme Bool
    if obj.is_instance_of::<pyo3::types::PyBool>() {
        if let Ok(val) = obj.extract::<bool>() {
            return Ok(IR::Bool(val));
        }
    }

    // Essayer d'extraire comme String
    if obj.is_instance_of::<pyo3::types::PyString>() {
        if let Ok(val) = obj.extract::<String>() {
            return Ok(IR::String(val));
        }
    }

    // Essayer d'extraire comme Bytes
    if obj.is_instance_of::<pyo3::types::PyBytes>() {
        if let Ok(val) = obj.extract::<Vec<u8>>() {
            return Ok(IR::Bytes(val));
        }
    }

    // Vérifier None
    if obj.is_none() {
        return Ok(IR::None);
    }

    // Essayer d'extraire comme Tuple (avant de gérer comme objet IR)
    if let Ok(tuple) = obj.cast::<PyTuple>() {
        let items: Result<Vec<_>, _> = tuple.iter().map(|item| python_to_ir_pure(_py, &item)).collect();
        return Ok(IR::Tuple(items?));
    }

    // Essayer d'extraire comme List
    if let Ok(list) = obj.cast::<PyList>() {
        let items: Result<Vec<_>, _> = list.iter().map(|item| python_to_ir_pure(_py, &item)).collect();
        return Ok(IR::List(items?));
    }

    // Round-trip: slice() Python → IR::Slice
    if let Ok(type_name) = obj.get_type().name() {
        if type_name == "slice" {
            let start = obj.getattr("start")?;
            let stop = obj.getattr("stop")?;
            let step = obj.getattr("step")?;
            return Ok(IR::Slice {
                start: Box::new(python_to_ir_pure(_py, &start)?),
                stop: Box::new(python_to_ir_pure(_py, &stop)?),
                step: Box::new(python_to_ir_pure(_py, &step)?),
            });
        }
    }

    // Si c'est un objet avec attributs (Op, IR), extraire les champs
    // L'objet IR Python utilise 'ident' au lieu de 'opcode'
    if let Ok(ident_obj) = obj.getattr("ident") {
        if let Ok(opcode_val) = ident_obj.extract::<u8>() {
            // Convertir en IROpCode
            if let Some(opcode) = super::opcode::IROpCode::from_u8(opcode_val) {
                // Extraire args (tuple ou list)
                let args_obj = obj.getattr("args")?;
                let args: Vec<IR> = if let Ok(tuple) = args_obj.cast::<PyTuple>() {
                    tuple
                        .iter()
                        .map(|arg| python_to_ir_pure(_py, &arg))
                        .collect::<Result<Vec<_>, _>>()?
                } else if let Ok(list) = args_obj.cast::<PyList>() {
                    list.iter()
                        .map(|arg| python_to_ir_pure(_py, &arg))
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    vec![]
                };

                // Extraire kwargs
                let kwargs_obj = obj.getattr("kwargs")?;
                let mut kwargs = IndexMap::new();
                if let Ok(dict) = kwargs_obj.cast::<PyDict>() {
                    for (k, v) in dict.iter() {
                        let key = k.extract::<String>()?;
                        let val = python_to_ir_pure(_py, &v)?;
                        kwargs.insert(key, val);
                    }
                }

                // Extraire tail
                let tail = obj.getattr("tail")?.extract::<bool>().unwrap_or(false);

                // Extraire start_byte et end_byte
                let start_byte = obj
                    .getattr("start_byte")
                    .ok()
                    .and_then(|x| x.extract::<usize>().ok())
                    .unwrap_or(0);
                let end_byte = obj
                    .getattr("end_byte")
                    .ok()
                    .and_then(|x| x.extract::<usize>().ok())
                    .unwrap_or(0);

                return Ok(IR::Op {
                    opcode,
                    args,
                    kwargs,
                    tail,
                    start_byte,
                    end_byte,
                });
            }
        }
    }

    // Gérer Identifier et Ref
    if let Ok(type_name) = obj.getattr("__class__") {
        let type_str = type_name.str()?.to_string();
        if type_str.contains("Identifier") {
            if let Ok(name) = obj.getattr("name") {
                return Ok(IR::Identifier(name.extract::<String>()?));
            }
        }
        if type_str.contains("Ref") {
            if let Ok(ident) = obj.getattr("ident") {
                let sb: isize = obj.getattr("start_byte").and_then(|v| v.extract()).unwrap_or(-1);
                let eb: isize = obj.getattr("end_byte").and_then(|v| v.extract()).unwrap_or(-1);
                return Ok(IR::Ref(ident.extract::<String>()?, sb, eb));
            }
        }
    }

    Err(PyValueError::new_err(format!(
        "Cannot convert Python object to IR: {:?}",
        obj
    )))
}

/// Convert Python IR to JSON
#[pyfunction]
pub fn ir_to_json(py: Python, obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let ir = python_to_ir_pure(py, obj)?;
    ir.to_json()
        .map_err(|e| PyValueError::new_err(format!("JSON serialization error: {}", e)))
}

/// Convert Python IR to pretty JSON
#[pyfunction]
pub fn ir_to_json_pretty(py: Python, obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let ir = python_to_ir_pure(py, obj)?;
    ir.to_json_pretty()
        .map_err(|e| PyValueError::new_err(format!("JSON serialization error: {}", e)))
}

/// Convert Python IR to compact JSON (minified)
#[pyfunction]
pub fn ir_to_json_compact(py: Python, obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let ir = python_to_ir_pure(py, obj)?;
    Ok(ir.to_compact_json())
}

/// Convert Python IR to compact JSON (pretty-printed)
#[pyfunction]
pub fn ir_to_json_compact_pretty(py: Python, obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let ir = python_to_ir_pure(py, obj)?;
    Ok(ir.to_compact_json_pretty())
}

/// Convert JSON to Python IR (Op nodes)
#[pyfunction]
pub fn ir_from_json(py: Python, json: &str) -> PyResult<Py<PyAny>> {
    let ir = IR::from_json(json).map_err(|e| PyValueError::new_err(format!("JSON deserialization error: {}", e)))?;
    super::ir_pure_to_python(py, ir)
}

/// Register the module in PyO3
pub fn register_module(m: &Bound<'_, pyo3::types::PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ir_to_json, m)?)?;
    m.add_function(wrap_pyfunction!(ir_to_json_pretty, m)?)?;
    m.add_function(wrap_pyfunction!(ir_to_json_compact, m)?)?;
    m.add_function(wrap_pyfunction!(ir_to_json_compact_pretty, m)?)?;
    m.add_function(wrap_pyfunction!(ir_from_json, m)?)?;
    Ok(())
}
