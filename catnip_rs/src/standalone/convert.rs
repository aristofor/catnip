// FILE: catnip_rs/src/standalone/convert.rs
//! Conversion IRPure → Op to reuse the existing compiler

use crate::core::Op;
use crate::ir::{IROpCode, IRPure};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};

/// Convert IRPure → Op (PyO3) to reuse the existing compiler
pub fn irpure_to_op(py: Python, ir: &IRPure) -> PyResult<Py<PyAny>> {
    match ir {
        // Literals - Convertir en objets Python bound
        IRPure::Int(n) => Ok((*n).into_pyobject(py)?.into_any().unbind()),
        IRPure::Float(f) => Ok((*f).into_pyobject(py)?.into_any().unbind()),
        IRPure::Bool(b) => Ok((*b).into_pyobject(py)?.to_owned().into_any().unbind()),
        IRPure::String(s) => Ok(PyString::new(py, s).into_any().unbind()),
        IRPure::None => Ok(py.None()),

        // Program (top-level sequence) → PyList (même conversion que List)
        IRPure::Program(items) => {
            let py_items: Result<Vec<_>, _> =
                items.iter().map(|item| irpure_to_op(py, item)).collect();
            Ok(PyList::new(py, py_items?)?.into_any().unbind())
        }

        // Collections
        IRPure::List(items) => {
            let py_items: Result<Vec<_>, _> =
                items.iter().map(|item| irpure_to_op(py, item)).collect();
            Ok(PyList::new(py, py_items?)?.into_any().unbind())
        }

        IRPure::Tuple(items) => {
            let py_items: Result<Vec<_>, _> =
                items.iter().map(|item| irpure_to_op(py, item)).collect();
            Ok(PyTuple::new(py, py_items?)?.into_any().unbind())
        }

        IRPure::Dict(pairs) => {
            let py_dict = PyDict::new(py);
            for (k, v) in pairs {
                let py_key = irpure_to_op(py, k)?;
                let py_val = irpure_to_op(py, v)?;
                py_dict.set_item(py_key, py_val)?;
            }
            Ok(py_dict.into_any().unbind())
        }

        IRPure::Set(items) => {
            let py_items: Result<Vec<_>, _> =
                items.iter().map(|item| irpure_to_op(py, item)).collect();
            let set_type = py.import("builtins")?.getattr("set")?;
            let py_list = PyList::new(py, py_items?)?;
            set_type.call1((py_list,)).map(|obj| obj.unbind())
        }

        // Variables
        IRPure::Identifier(name) => {
            // Identifier dans le corps d'une expression = référence de variable
            // Créer un Ref Python pour lookup de variable
            let nodes_module = py.import("catnip.nodes")?;
            let ref_class = nodes_module.getattr("Ref")?;
            Ok(ref_class.call1((name,))?.unbind())
        }

        IRPure::Ref(name) => {
            // Créer un Ref Python pour lookup de variable
            let nodes_module = py.import("catnip.nodes")?;
            let ref_class = nodes_module.getattr("Ref")?;
            let py_name = PyString::new(py, name);
            ref_class.call1((py_name,)).map(|obj| obj.unbind())
        }

        // Function call
        IRPure::Call {
            func, args, kwargs, ..
        } => {
            // Convertir func
            let py_func = irpure_to_op(py, func)?;

            // Convertir args (func + args)
            let mut all_args = vec![py_func];
            for arg in args {
                all_args.push(irpure_to_op(py, arg)?);
            }
            let py_args_tuple: Py<PyAny> = PyTuple::new(py, all_args)?.unbind().into();

            // Convertir kwargs
            let py_kwargs = PyDict::new(py);
            for (k, v) in kwargs {
                let py_key = PyString::new(py, k);
                let py_val = irpure_to_op(py, v)?;
                py_kwargs.set_item(py_key, py_val)?;
            }
            let py_kwargs_unbind: Py<PyAny> = py_kwargs.into_any().unbind();

            // Créer un Op avec opcode Call
            let op = Op::from_rust(
                py,
                IROpCode::Call as i32,
                py_args_tuple,
                py_kwargs_unbind,
                false,
                0,
                0,
            );

            Ok(Py::new(py, op)?.into_any())
        }

        // Operations
        IRPure::Op {
            opcode,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        } => {
            // Traitement spécial pour OpLambda: les paramètres doivent rester des strings
            if *opcode == IROpCode::OpLambda && args.len() >= 2 {
                // args[0] = paramètres (Tuple d'Identifiers → strings)
                // args[1] = body (convertir normalement avec Identifier → Ref)
                let params_ir = &args[0];
                let body_ir = &args[1];

                // Convertir les paramètres en conservant les Identifiers comme strings
                // Format: soit Identifier (ancien), soit Tuple(String(name), default_value)
                let params_py = match params_ir {
                    IRPure::Tuple(items) => {
                        let param_tuples: Result<Vec<_>, _> = items
                            .iter()
                            .map(|item| match item {
                                // Ancien format: juste un Identifier
                                IRPure::Identifier(name) => Ok(PyTuple::new(
                                    py,
                                    vec![
                                        PyString::new(py, name).into_any(),
                                        py.None().into_bound(py).into_any(),
                                    ],
                                )?
                                .into_any()),
                                // Nouveau format: Tuple(String(name), default_value)
                                IRPure::Tuple(pair) if pair.len() == 2 => {
                                    let name_str =
                                        match &pair[0] {
                                            IRPure::String(s) => s.clone(),
                                            IRPure::Identifier(s) => s.clone(),
                                            _ => return Err(PyErr::new::<
                                                pyo3::exceptions::PyTypeError,
                                                _,
                                            >(
                                                "Lambda param name must be String or Identifier",
                                            )),
                                        };
                                    let default_py = irpure_to_op(py, &pair[1])?;
                                    Ok(PyTuple::new(
                                        py,
                                        vec![
                                            PyString::new(py, &name_str).into_any(),
                                            default_py.into_bound(py).into_any(),
                                        ],
                                    )?
                                    .into_any())
                                }
                                _ => Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                                    "Lambda parameters must be Identifiers or Tuples",
                                )),
                            })
                            .collect();
                        PyTuple::new(py, param_tuples?)?.into_any().unbind()
                    }
                    _ => irpure_to_op(py, params_ir)?,
                };

                // Convertir le body normalement (Identifier → Ref)
                let body_py = irpure_to_op(py, body_ir)?;

                let py_args_tuple = PyTuple::new(py, vec![params_py, body_py])?.unbind().into();
                let py_kwargs = PyDict::new(py);
                let py_kwargs_unbind: Py<PyAny> = py_kwargs.into_any().unbind();

                let op = Op::from_rust(
                    py,
                    *opcode as i32,
                    py_args_tuple,
                    py_kwargs_unbind,
                    *tail,
                    *start_byte as isize,
                    *end_byte as isize,
                );

                return Ok(Py::new(py, op)?.into_any());
            }

            // Convertir les args normalement
            let py_args: Result<Vec<_>, _> = args.iter().map(|arg| irpure_to_op(py, arg)).collect();
            let py_args_tuple = PyTuple::new(py, py_args?)?.unbind().into();

            // Convertir les kwargs
            let py_kwargs = PyDict::new(py);
            for (k, v) in kwargs {
                let py_key = PyString::new(py, k);
                let py_val = irpure_to_op(py, v)?;
                py_kwargs.set_item(py_key, py_val)?;
            }
            let py_kwargs_unbind: Py<PyAny> = py_kwargs.into_any().unbind();

            // Créer l'Op avec le constructeur public Rust
            let op = Op::from_rust(
                py,
                *opcode as i32,
                py_args_tuple,
                py_kwargs_unbind,
                *tail,
                *start_byte as isize,
                *end_byte as isize,
            );

            Ok(Py::new(py, op)?.into_any())
        }

        // Pattern matching
        IRPure::PatternLiteral(value) => {
            let nodes_module = py.import("catnip.nodes")?;
            let pattern_literal_class = nodes_module.getattr("PatternLiteral")?;
            let value_py = irpure_to_op(py, value)?;
            pattern_literal_class
                .call1((value_py,))
                .map(|obj| obj.unbind())
        }

        IRPure::PatternVar(name) => {
            let nodes_module = py.import("catnip.nodes")?;
            let pattern_var_class = nodes_module.getattr("PatternVar")?;
            let name_py = PyString::new(py, name);
            pattern_var_class.call1((name_py,)).map(|obj| obj.unbind())
        }

        IRPure::PatternWildcard => {
            let nodes_module = py.import("catnip.nodes")?;
            let pattern_wildcard_class = nodes_module.getattr("PatternWildcard")?;
            pattern_wildcard_class.call0().map(|obj| obj.unbind())
        }

        IRPure::PatternOr(patterns) => {
            let nodes_module = py.import("catnip.nodes")?;
            let pattern_or_class = nodes_module.getattr("PatternOr")?;
            let py_patterns: Result<Vec<_>, _> =
                patterns.iter().map(|p| irpure_to_op(py, p)).collect();
            let py_list = PyList::new(py, py_patterns?)?;
            pattern_or_class.call1((py_list,)).map(|obj| obj.unbind())
        }

        IRPure::PatternTuple(patterns) => {
            let nodes_module = py.import("catnip.nodes")?;
            let pattern_tuple_class = nodes_module.getattr("PatternTuple")?;
            let py_patterns: Result<Vec<_>, _> =
                patterns.iter().map(|p| irpure_to_op(py, p)).collect();
            let py_list = PyList::new(py, py_patterns?)?;
            pattern_tuple_class
                .call1((py_list,))
                .map(|obj| obj.unbind())
        }

        IRPure::PatternStruct { name, fields } => {
            let nodes_module = py.import("catnip.nodes")?;
            let pattern_struct_class = nodes_module.getattr("PatternStruct")?;
            let fields_list = PyList::new(py, fields)?;
            pattern_struct_class
                .call1((name, fields_list))
                .map(|obj| obj.unbind())
        }

        IRPure::Slice { start, stop, step } => {
            // Convert to Python slice object: slice(start, stop, step)
            let py_start = irpure_to_op(py, start)?;
            let py_stop = irpure_to_op(py, stop)?;
            let py_step = irpure_to_op(py, step)?;

            let builtins = py.import("builtins")?;
            let slice_class = builtins.getattr("slice")?;
            slice_class
                .call1((py_start, py_stop, py_step))
                .map(|obj| obj.unbind())
        }

        IRPure::Broadcast {
            target,
            operator,
            operand,
            broadcast_type,
        } => {
            // Convert to catnip.nodes.Broadcast object
            let nodes_module = py.import("catnip.nodes")?;
            let broadcast_class = nodes_module.getattr("Broadcast")?;

            let py_target = if let Some(t) = target {
                irpure_to_op(py, t)?
            } else {
                py.None()
            };

            let py_operator = irpure_to_op(py, operator)?;

            let py_operand = if let Some(o) = operand {
                irpure_to_op(py, o)?
            } else {
                py.None()
            };

            // Determine is_filter based on broadcast_type
            use crate::ir::pure::BroadcastType;
            let is_filter = matches!(broadcast_type, BroadcastType::If);

            // Broadcast(target, operator, operand, is_filter)
            broadcast_class
                .call1((py_target, py_operator, py_operand, is_filter))
                .map(|obj| obj.unbind())
        }
    }
}
