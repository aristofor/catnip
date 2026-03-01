// FILE: catnip_rs/src/ir/to_python.rs
//! Conversion IRPure → Python objects
//!
//! Allows using pure_transforms (Rust) while maintaining
//! compatibility with the existing Python API.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};

use super::opcode::IROpCode;
use super::pure::{BroadcastType, IRPure};

/// Cache for Python module imports
struct PythonCache<'py> {
    nodes_module: Bound<'py, PyAny>,
    transformer_module: Bound<'py, PyAny>,
    builtins: Bound<'py, PyAny>,
}

impl<'py> PythonCache<'py> {
    fn new(py: Python<'py>) -> PyResult<Self> {
        Ok(Self {
            nodes_module: py.import("catnip.nodes")?.as_any().clone(),
            transformer_module: py.import("catnip.transformer")?.as_any().clone(),
            builtins: py.import("builtins")?.as_any().clone(),
        })
    }
}

/// Convert IRPure → Python object compatible with the legacy IR
pub fn ir_pure_to_python(py: Python, ir: IRPure) -> PyResult<Py<PyAny>> {
    let cache = PythonCache::new(py)?;
    ir_pure_to_python_impl(py, ir, &cache)
}

/// Internal implementation with cache
fn ir_pure_to_python_impl(py: Python, ir: IRPure, cache: &PythonCache) -> PyResult<Py<PyAny>> {
    match ir {
        // Scalaires primitifs
        IRPure::Int(i) => Ok(i.into_pyobject(py)?.into_any().unbind()),
        IRPure::Float(f) => Ok(f.into_pyobject(py)?.into_any().unbind()),
        IRPure::String(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
        IRPure::Bytes(v) => Ok(PyBytes::new(py, &v).into_any().unbind()),
        IRPure::Bool(b) => Ok(b.into_pyobject(py)?.as_any().clone().unbind()),
        IRPure::None => Ok(py.None()),
        IRPure::Decimal(s) => {
            let decimal_mod = py.import("decimal")?;
            let decimal_cls = decimal_mod.getattr("Decimal")?;
            Ok(decimal_cls.call1((s,))?.unbind())
        }
        IRPure::Imaginary(s) => {
            let imag: f64 = s.parse().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("invalid imaginary: {}", e))
            })?;
            let builtins = py.import("builtins")?;
            let complex_cls = builtins.getattr("complex")?;
            Ok(complex_cls.call1((0.0, imag))?.unbind())
        }

        // Identifier → retourner string directement (sera wrappé par le transformer Python)
        IRPure::Identifier(name) => Ok(name.into_pyobject(py)?.into_any().unbind()),

        // Ref → créer un objet Ref Python
        IRPure::Ref(name, start_byte, end_byte) => {
            let ref_class = cache.nodes_module.getattr("Ref")?;
            Ok(ref_class.call1((name, start_byte, end_byte))?.unbind())
        }

        // Program (top-level sequence) → PyList
        IRPure::Program(items) => {
            let py_items: Vec<_> = items
                .into_iter()
                .map(|i| ir_pure_to_python_impl(py, i, cache))
                .collect::<PyResult<_>>()?;
            Ok(PyList::new(py, &py_items)?.into())
        }

        // List
        IRPure::List(items) => {
            let py_items: Vec<_> = items
                .into_iter()
                .map(|i| ir_pure_to_python_impl(py, i, cache))
                .collect::<PyResult<_>>()?;
            Ok(PyList::new(py, &py_items)?.into())
        }

        // Tuple
        IRPure::Tuple(items) => {
            let py_items: Vec<_> = items
                .into_iter()
                .map(|i| ir_pure_to_python_impl(py, i, cache))
                .collect::<PyResult<_>>()?;
            Ok(PyTuple::new(py, &py_items)?.into())
        }

        // Dict
        IRPure::Dict(pairs) => {
            let dict = PyDict::new(py);
            for (key, value) in pairs {
                let py_key = ir_pure_to_python_impl(py, key, cache)?;
                let py_value = ir_pure_to_python_impl(py, value, cache)?;
                dict.set_item(py_key, py_value)?;
            }
            Ok(dict.into())
        }

        // Set
        IRPure::Set(items) => {
            let py_items: Vec<_> = items
                .into_iter()
                .map(|i| ir_pure_to_python_impl(py, i, cache))
                .collect::<PyResult<_>>()?;
            let set_class = cache.builtins.getattr("set")?;
            Ok(set_class.call1((PyList::new(py, &py_items)?,))?.unbind())
        }

        // Op node → créer un objet IR Python
        IRPure::Op {
            opcode,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        } => {
            // Use cached IR class (from transformer module)
            let op_class = cache.transformer_module.getattr("IR")?;

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

            let op_obj =
                op_class.call1((opcode_int, py_args_tuple, py_kwargs, start_byte, end_byte))?;

            // Set tail attribute
            op_obj.setattr("tail", tail)?;

            Ok(op_obj.unbind())
        }

        // Call → créer un objet Call
        IRPure::Call {
            func,
            args,
            kwargs,
            start_byte,
            end_byte,
        } => {
            let py_func = ir_pure_to_python_impl(py, *func, cache)?;

            let py_args: Vec<_> = args
                .into_iter()
                .map(|a| ir_pure_to_python_impl(py, a, cache))
                .collect::<PyResult<_>>()?;
            let py_args_tuple = PyTuple::new(py, &py_args)?;

            let py_kwargs = PyDict::new(py);
            for (k, v) in kwargs {
                py_kwargs.set_item(k, ir_pure_to_python_impl(py, v, cache)?)?;
            }

            // Create Call object with source positions
            let call_class = cache.transformer_module.getattr("Call")?;
            let call_obj = call_class.call1((py_func, py_args_tuple, py_kwargs))?;
            call_obj.setattr("start_byte", start_byte as isize)?;
            call_obj.setattr("end_byte", end_byte as isize)?;

            Ok(call_obj.unbind())
        }

        // Pattern variants
        IRPure::PatternLiteral(value) => {
            let pattern_class = cache.nodes_module.getattr("PatternLiteral")?;
            let py_value = ir_pure_to_python_impl(py, *value, cache)?;
            Ok(pattern_class.call1((py_value,))?.unbind())
        }

        IRPure::PatternVar(name) => {
            let pattern_class = cache.nodes_module.getattr("PatternVar")?;
            Ok(pattern_class.call1((name,))?.unbind())
        }

        IRPure::PatternWildcard => {
            let pattern_class = cache.nodes_module.getattr("PatternWildcard")?;
            Ok(pattern_class.call0()?.unbind())
        }

        IRPure::PatternOr(patterns) => {
            let pattern_class = cache.nodes_module.getattr("PatternOr")?;
            let py_patterns: Vec<_> = patterns
                .into_iter()
                .map(|p| ir_pure_to_python_impl(py, p, cache))
                .collect::<PyResult<_>>()?;
            let patterns_tuple = PyTuple::new(py, &py_patterns)?;
            Ok(pattern_class.call1((patterns_tuple,))?.unbind())
        }

        IRPure::PatternTuple(patterns) => {
            let pattern_class = cache.nodes_module.getattr("PatternTuple")?;
            let py_patterns: Vec<_> = patterns
                .into_iter()
                .map(|p| ir_pure_to_python_impl(py, p, cache))
                .collect::<PyResult<_>>()?;
            let patterns_tuple = PyTuple::new(py, &py_patterns)?;
            Ok(pattern_class.call1((patterns_tuple,))?.unbind())
        }

        // Slice → Op(SLICE, start, stop, step) so Ref nodes get resolved
        IRPure::Slice { start, stop, step } => {
            let op_class = cache.transformer_module.getattr("IR")?;
            let py_start = ir_pure_to_python_impl(py, *start, cache)?;
            let py_stop = ir_pure_to_python_impl(py, *stop, cache)?;
            let py_step = ir_pure_to_python_impl(py, *step, cache)?;
            let opcode_int = IROpCode::Slice as u8 as i32;
            let py_args = PyTuple::new(py, &[py_start, py_stop, py_step])?;
            let kwargs = PyDict::new(py);
            Ok(op_class
                .call1((opcode_int, py_args, kwargs, 0usize, 0usize))?
                .unbind())
        }

        // Struct pattern
        IRPure::PatternStruct { name, fields } => {
            let pattern_class = cache.nodes_module.getattr("PatternStruct")?;
            let fields_list = PyList::new(py, &fields)?;
            Ok(pattern_class.call1((name, fields_list))?.unbind())
        }

        // Broadcast
        IRPure::Broadcast {
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
            let broadcast_obj =
                broadcast_class.call1((py_target, py_operator, py_operand, is_filter))?;

            Ok(broadcast_obj.unbind())
        }
    }
}

// Tests: covered by the Python test suite (873+ integration tests)
