// FILE: catnip_rs/src/ir/to_python.rs
//! Conversion IR → Python objects
//!
//! Allows using pure_transforms (Rust) while maintaining
//! compatibility with the existing Python API.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};

use super::opcode::IROpCode;
use super::pure::{BroadcastType, IR};
use crate::constants::*;

/// Cache for Python module imports
struct PythonCache<'py> {
    nodes_module: Bound<'py, PyAny>,
    rs_module: Bound<'py, PyAny>,
    builtins: Bound<'py, PyAny>,
}

impl<'py> PythonCache<'py> {
    fn new(py: Python<'py>) -> PyResult<Self> {
        Ok(Self {
            nodes_module: py.import(PY_MOD_NODES)?.as_any().clone(),
            rs_module: py.import("catnip._rs")?.as_any().clone(),
            builtins: py.import("builtins")?.as_any().clone(),
        })
    }
}

/// Convert IR → Python object compatible with the legacy IR
pub fn ir_pure_to_python(py: Python, ir: IR) -> PyResult<Py<PyAny>> {
    let cache = PythonCache::new(py)?;
    ir_pure_to_python_impl(py, ir, &cache)
}

/// Internal implementation with cache
fn ir_pure_to_python_impl(py: Python, ir: IR, cache: &PythonCache) -> PyResult<Py<PyAny>> {
    match ir {
        // Scalaires primitifs
        IR::Int(i) => Ok(i.into_pyobject(py)?.into_any().unbind()),
        IR::Float(f) => Ok(f.into_pyobject(py)?.into_any().unbind()),
        IR::String(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
        IR::Bytes(v) => Ok(PyBytes::new(py, &v).into_any().unbind()),
        IR::Bool(b) => Ok(b.into_pyobject(py)?.as_any().clone().unbind()),
        IR::None => Ok(py.None()),
        IR::Decimal(s) => {
            let decimal_mod = py.import("decimal")?;
            let decimal_cls = decimal_mod.getattr("Decimal")?;
            Ok(decimal_cls.call1((s,))?.unbind())
        }
        IR::Imaginary(s) => {
            let imag: f64 = s
                .parse()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("invalid imaginary: {}", e)))?;
            let builtins = py.import("builtins")?;
            let complex_cls = builtins.getattr("complex")?;
            Ok(complex_cls.call1((0.0, imag))?.unbind())
        }

        // Identifier → retourner string directement (sera wrappé par le transformer Python)
        IR::Identifier(name) => Ok(name.into_pyobject(py)?.into_any().unbind()),

        // Ref → créer un objet Ref Python
        IR::Ref(name, start_byte, end_byte) => {
            let ref_class = cache.nodes_module.getattr("Ref")?;
            Ok(ref_class.call1((name, start_byte, end_byte))?.unbind())
        }

        // Program (top-level sequence) → PyList
        IR::Program(items) => {
            let py_items: Vec<_> = items
                .into_iter()
                .map(|i| ir_pure_to_python_impl(py, i, cache))
                .collect::<PyResult<_>>()?;
            Ok(PyList::new(py, &py_items)?.into())
        }

        // List
        IR::List(items) => {
            let py_items: Vec<_> = items
                .into_iter()
                .map(|i| ir_pure_to_python_impl(py, i, cache))
                .collect::<PyResult<_>>()?;
            Ok(PyList::new(py, &py_items)?.into())
        }

        // Tuple
        IR::Tuple(items) => {
            let py_items: Vec<_> = items
                .into_iter()
                .map(|i| ir_pure_to_python_impl(py, i, cache))
                .collect::<PyResult<_>>()?;
            Ok(PyTuple::new(py, &py_items)?.into())
        }

        // Dict
        IR::Dict(pairs) => {
            let dict = PyDict::new(py);
            for (key, value) in pairs {
                let py_key = ir_pure_to_python_impl(py, key, cache)?;
                let py_value = ir_pure_to_python_impl(py, value, cache)?;
                dict.set_item(py_key, py_value)?;
            }
            Ok(dict.into())
        }

        // Set
        IR::Set(items) => {
            let py_items: Vec<_> = items
                .into_iter()
                .map(|i| ir_pure_to_python_impl(py, i, cache))
                .collect::<PyResult<_>>()?;
            let set_class = cache.builtins.getattr("set")?;
            Ok(set_class.call1((PyList::new(py, &py_items)?,))?.unbind())
        }

        // Op node → créer un objet IR Python
        IR::Op {
            opcode,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        } => {
            let op_class = cache.nodes_module.getattr("Op")?;

            // Convert args
            let py_args: Vec<_> = args
                .into_iter()
                .map(|a| ir_pure_to_python_impl(py, a, cache))
                .collect::<PyResult<_>>()?;

            // Convert kwargs
            let py_kwargs = PyDict::new(py);
            for (k, v) in kwargs {
                py_kwargs.set_item(k, ir_pure_to_python_impl(py, v, cache)?)?;
            }

            // Create Op object: Op(opcode_int, args, kwargs, start_byte, end_byte)
            let opcode_int = opcode as u8 as i32; // Convert to i32 for Python
            let py_args_tuple = PyTuple::new(py, &py_args)?;

            let op_obj = op_class.call1((opcode_int, py_args_tuple, py_kwargs, start_byte, end_byte))?;

            // Set tail attribute
            op_obj.setattr("tail", tail)?;

            Ok(op_obj.unbind())
        }

        // Call → créer un Op(CALL, [func, *args], kwargs)
        // Flatten comme le fait l'analyseur sémantique (visit_call)
        IR::Call {
            func,
            args,
            kwargs,
            start_byte,
            end_byte,
            ..
        } => {
            let py_func = ir_pure_to_python_impl(py, *func, cache)?;

            // Flatten: [func, arg1, arg2, ...] comme Op(CALL)
            let mut all_args = vec![py_func];
            for a in args {
                all_args.push(ir_pure_to_python_impl(py, a, cache)?);
            }
            let py_args_tuple = PyTuple::new(py, &all_args)?;

            let py_kwargs = PyDict::new(py);
            for (k, v) in kwargs {
                py_kwargs.set_item(k, ir_pure_to_python_impl(py, v, cache)?)?;
            }

            let opcode_int = IROpCode::Call as u8 as i32;
            let op_class = cache.rs_module.getattr("Op")?;
            let op_obj = op_class.call1((opcode_int, py_args_tuple, py_kwargs, start_byte, end_byte))?;

            Ok(op_obj.unbind())
        }

        // Pattern variants
        IR::PatternLiteral(value) => {
            let pattern_class = cache.nodes_module.getattr("PatternLiteral")?;
            let py_value = ir_pure_to_python_impl(py, *value, cache)?;
            Ok(pattern_class.call1((py_value,))?.unbind())
        }

        IR::PatternVar(name) => {
            let pattern_class = cache.nodes_module.getattr("PatternVar")?;
            Ok(pattern_class.call1((name,))?.unbind())
        }

        IR::PatternWildcard => {
            let pattern_class = cache.nodes_module.getattr("PatternWildcard")?;
            Ok(pattern_class.call0()?.unbind())
        }

        IR::PatternOr(patterns) => {
            let pattern_class = cache.nodes_module.getattr("PatternOr")?;
            let py_patterns: Vec<_> = patterns
                .into_iter()
                .map(|p| ir_pure_to_python_impl(py, p, cache))
                .collect::<PyResult<_>>()?;
            let patterns_tuple = PyTuple::new(py, &py_patterns)?;
            Ok(pattern_class.call1((patterns_tuple,))?.unbind())
        }

        IR::PatternTuple(patterns) => {
            let pattern_class = cache.nodes_module.getattr("PatternTuple")?;
            let py_patterns: Vec<_> = patterns
                .into_iter()
                .map(|p| ir_pure_to_python_impl(py, p, cache))
                .collect::<PyResult<_>>()?;
            let patterns_tuple = PyTuple::new(py, &py_patterns)?;
            Ok(pattern_class.call1((patterns_tuple,))?.unbind())
        }

        // Slice → Op(SLICE, start, stop, step) so Ref nodes get resolved
        IR::Slice { start, stop, step } => {
            let op_class = cache.nodes_module.getattr("Op")?;
            let py_start = ir_pure_to_python_impl(py, *start, cache)?;
            let py_stop = ir_pure_to_python_impl(py, *stop, cache)?;
            let py_step = ir_pure_to_python_impl(py, *step, cache)?;
            let opcode_int = IROpCode::Slice as u8 as i32;
            let py_args = PyTuple::new(py, &[py_start, py_stop, py_step])?;
            let kwargs = PyDict::new(py);
            Ok(op_class.call1((opcode_int, py_args, kwargs, 0usize, 0usize))?.unbind())
        }

        // Struct pattern
        IR::PatternStruct { name, fields } => {
            let pattern_class = cache.nodes_module.getattr("PatternStruct")?;
            let fields_list = PyList::new(py, &fields)?;
            Ok(pattern_class.call1((name, fields_list))?.unbind())
        }

        // Broadcast
        IR::Broadcast {
            target,
            operator,
            operand,
            broadcast_type,
        } => {
            // Convert to Python Broadcast object
            let broadcast_class = cache.nodes_module.getattr("Broadcast")?;

            // Convert target (may be None)
            let py_target = if let Some(t) = target {
                ir_pure_to_python_impl(py, *t, cache)?
            } else {
                py.None()
            };

            // Convert operator
            let py_operator = ir_pure_to_python_impl(py, *operator, cache)?;

            // Convert operand (may be None)
            let py_operand = if let Some(op) = operand {
                ir_pure_to_python_impl(py, *op, cache)?
            } else {
                py.None()
            };

            // Determine is_filter from broadcast_type
            // ND operations are not filters
            let is_filter = matches!(broadcast_type, BroadcastType::If);

            // Create Broadcast(target, operator, operand, is_filter)
            let broadcast_obj = broadcast_class.call1((py_target, py_operator, py_operand, is_filter))?;

            Ok(broadcast_obj.unbind())
        }
    }
}

// Tests: covered by the Python test suite (873+ integration tests)
