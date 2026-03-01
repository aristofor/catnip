// FILE: catnip_rs/src/semantic/strength_reduction.rs
//! Strength reduction optimization pass
//!
//! Port of StrengthReductionPass from catnip/semantic/optimizer.pyx
//!
//! Replaces expensive operations with cheaper equivalents:
//! - x ** 2 → x * x (multiplication faster than pow)
//! - x * 1 → x, x * 0 → 0
//! - x + 0 → x, x - 0 → x
//! - x / 1 → x, x // 1 → x

use super::opcode::OpCode;
use super::optimizer::{default_visit_ir, OptimizationPass};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyTuple};

#[pyclass(name = "StrengthReductionPass")]
pub struct StrengthReductionPass;

#[pymethods]
impl StrengthReductionPass {
    #[new]
    fn new() -> Self {
        StrengthReductionPass
    }

    /// Visit a node and apply optimizations
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for StrengthReductionPass {
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit(self, py, node)
    }

    fn visit_ir(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // First visit children
        let visited = default_visit_ir(self, py, node)?;
        let visited_bound = visited.bind(py);

        // Check if result is still an IR node
        let node_type = visited_bound.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;
        if type_name != "IR" {
            return Ok(visited);
        }

        // Try to apply strength reduction
        self.reduce_strength(py, visited_bound)
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit_op(self, py, node)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node.clone().unbind())
    }
}

impl StrengthReductionPass {
    /// Apply strength reduction to an IR node
    fn reduce_strength(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let ident = node.getattr("ident")?;
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        // Only handle binary operations
        if args_tuple.len() != 2 {
            return Ok(node.clone().unbind());
        }

        let arg0 = args_tuple.get_item(0)?;
        let arg1 = args_tuple.get_item(1)?;

        // MUL optimizations
        if self.is_op_match(&ident, OpCode::MUL)? {
            // x * 1 → x, 1 * x → x
            if self.is_value(&arg1, 1)? {
                return Ok(arg0.unbind());
            }
            if self.is_value(&arg0, 1)? {
                return Ok(arg1.unbind());
            }
            // x * 0 → 0, 0 * x → 0
            if self.is_value(&arg1, 0)? || self.is_value(&arg0, 0)? {
                return Ok(0i64.into_pyobject(py)?.into_any().unbind());
            }
        }

        // POW optimizations
        if self.is_op_match(&ident, OpCode::POW)? {
            // x ** 2 → x * x (multiplication is faster)
            if self.is_value(&arg1, 2)? {
                let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
                let mul_ident = (OpCode::MUL as i32).into_pyobject(py)?.into_any();
                let new_args = PyTuple::new(py, &[arg0.clone().unbind(), arg0.unbind()])?;
                let empty_dict = py.import("builtins")?.getattr("dict")?.call0()?;
                return ir_class
                    .call1((mul_ident, new_args, empty_dict))
                    .map(|n| n.unbind());
            }
            // x ** 1 → x
            if self.is_value(&arg1, 1)? {
                return Ok(arg0.unbind());
            }
            // x ** 0 → 1
            if self.is_value(&arg1, 0)? {
                return Ok(1i64.into_pyobject(py)?.into_any().unbind());
            }
        }

        // ADD optimizations
        if self.is_op_match(&ident, OpCode::ADD)? {
            // x + 0 → x, 0 + x → x
            if self.is_value(&arg1, 0)? {
                return Ok(arg0.unbind());
            }
            if self.is_value(&arg0, 0)? {
                return Ok(arg1.unbind());
            }
        }

        // SUB optimizations
        if self.is_op_match(&ident, OpCode::SUB)? {
            // x - 0 → x
            if self.is_value(&arg1, 0)? {
                return Ok(arg0.unbind());
            }
        }

        // TRUEDIV optimizations
        if self.is_op_match(&ident, OpCode::TRUEDIV)? || self.is_op_match(&ident, OpCode::DIV)? {
            // x / 1 → x
            if self.is_value(&arg1, 1)? {
                return Ok(arg0.unbind());
            }
        }

        // FLOORDIV optimizations
        if self.is_op_match(&ident, OpCode::FLOORDIV)? {
            // x // 1 → x
            if self.is_value(&arg1, 1)? {
                return Ok(arg0.unbind());
            }
        }

        // AND optimizations (short-circuit boolean identities)
        if self.is_op_match(&ident, OpCode::AND)? {
            // x && True → x, True && x → x
            if self.is_bool(&arg1, true)? {
                return Ok(arg0.unbind());
            }
            if self.is_bool(&arg0, true)? {
                return Ok(arg1.unbind());
            }
            // x && False → False, False && x → False
            if self.is_bool(&arg1, false)? || self.is_bool(&arg0, false)? {
                return Ok(PyBool::new(py, false).to_owned().into_any().unbind());
            }
        }

        // OR optimizations (short-circuit boolean identities)
        if self.is_op_match(&ident, OpCode::OR)? {
            // x || False → x, False || x → x
            if self.is_bool(&arg1, false)? {
                return Ok(arg0.unbind());
            }
            if self.is_bool(&arg0, false)? {
                return Ok(arg1.unbind());
            }
            // x || True → True, True || x → True
            if self.is_bool(&arg1, true)? || self.is_bool(&arg0, true)? {
                return Ok(PyBool::new(py, true).to_owned().into_any().unbind());
            }
        }

        // No reduction applied
        Ok(node.clone().unbind())
    }

    /// Check if ident matches an opcode
    fn is_op_match(&self, ident: &Bound<'_, PyAny>, opcode: OpCode) -> PyResult<bool> {
        if let Ok(int_val) = ident.extract::<i32>() {
            return Ok(int_val == opcode as i32);
        }
        Ok(false)
    }

    /// Check if a value is a specific boolean
    fn is_bool(&self, obj: &Bound<'_, PyAny>, value: bool) -> PyResult<bool> {
        if let Ok(val) = obj.extract::<bool>() {
            return Ok(val == value);
        }
        Ok(false)
    }

    /// Check if a value is a specific integer
    fn is_value(&self, obj: &Bound<'_, PyAny>, value: i64) -> PyResult<bool> {
        if let Ok(val) = obj.extract::<i64>() {
            return Ok(val == value);
        }
        if let Ok(val) = obj.extract::<f64>() {
            return Ok(val == value as f64);
        }
        Ok(false)
    }
}
