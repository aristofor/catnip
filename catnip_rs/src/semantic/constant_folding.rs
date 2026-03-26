// FILE: catnip_rs/src/semantic/constant_folding.rs
//! Constant folding optimization pass
//!
//! Port of ConstantFoldingPass from catnip/semantic/optimizer.pyx
//!
//! Evaluates constant expressions at compile time:
//! - 2 + 3 → 5
//! - "hello" * 3 → "hellohellohello"
//! - not True → False

use super::opcode::OpCode;
use super::optimizer::{OptimizationPass, default_visit_ir};
use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

#[pyclass(name = "ConstantFoldingPass")]
pub struct ConstantFoldingPass;

impl Default for ConstantFoldingPass {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstantFoldingPass {
    /// Create a new ConstantFoldingPass instance (Rust API)
    pub fn new() -> Self {
        ConstantFoldingPass
    }
}

#[pymethods]
impl ConstantFoldingPass {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    /// Visit a node and apply optimizations
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for ConstantFoldingPass {
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
        if type_name != "IR" && type_name != "Op" {
            return Ok(visited);
        }

        // Try to fold if all args are constants
        if self.is_foldable(py, visited_bound)? {
            match self.fold(py, visited_bound) {
                Ok(result) => Ok(result),
                Err(_) => Ok(visited), // If folding fails, return original
            }
        } else {
            Ok(visited)
        }
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit_op(self, py, node)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node.clone().unbind())
    }
}

impl ConstantFoldingPass {
    /// Check if a node can be folded (all arguments are constants)
    fn is_foldable(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<bool> {
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        for arg in args_tuple.iter() {
            if !self.is_constant(py, &arg)? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Check if a value is a constant (not Ref, IR, Op, Identifier)
    fn is_constant(&self, _py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
        let obj_type = obj.get_type();
        let type_name_obj = obj_type.name()?;
        let type_name = type_name_obj.to_str()?;

        // Not constant if it's Ref, IR, Op, Identifier, Call, or Broadcast
        if type_name == catnip::REF
            || type_name == catnip::IR
            || type_name == catnip::OP
            || type_name == catnip::IDENTIFIER
            || type_name == catnip::CALL
            || type_name == catnip::BROADCAST
        {
            return Ok(false);
        }

        // Check nested structures (list, tuple)
        if obj.is_instance_of::<pyo3::types::PyList>() || obj.is_instance_of::<PyTuple>() {
            for item in obj.try_iter()? {
                let item = item?;
                if !self.is_constant(_py, &item)? {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    /// Fold a constant expression
    fn fold(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let ident = node.getattr("ident")?;
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        // Extract OpCode if it's an int
        if let Ok(opcode_int) = ident.extract::<i32>() {
            if let Some(opcode) = OpCode::from_i32(opcode_int) {
                return self.fold_by_opcode(py, opcode, args_tuple);
            }
        }

        // Can't fold
        Ok(node.clone().unbind())
    }

    /// Fold based on opcode
    fn fold_by_opcode(&self, py: Python<'_>, opcode: OpCode, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        match opcode {
            // Arithmetic operations
            OpCode::ADD => self.fold_add(py, args),
            OpCode::SUB => self.fold_sub(py, args),
            OpCode::MUL => self.fold_mul(py, args),
            OpCode::TRUEDIV => self.fold_truediv(py, args),
            OpCode::FLOORDIV => self.fold_floordiv(py, args),
            OpCode::MOD => self.fold_mod(py, args),
            OpCode::POW => self.fold_pow(py, args),
            OpCode::NEG => self.fold_neg(py, args),
            OpCode::POS => self.fold_pos(py, args),

            // Comparison operations
            OpCode::EQ => self.fold_eq(py, args),
            OpCode::NE => self.fold_ne(py, args),
            OpCode::LT => self.fold_lt(py, args),
            OpCode::LE => self.fold_le(py, args),
            OpCode::GT => self.fold_gt(py, args),
            OpCode::GE => self.fold_ge(py, args),

            // Logical operations
            OpCode::AND => self.fold_and(py, args),
            OpCode::OR => self.fold_or(py, args),
            OpCode::NOT => self.fold_not(py, args),

            // Bitwise operations
            OpCode::BAND => self.fold_band(py, args),
            OpCode::BOR => self.fold_bor(py, args),
            OpCode::BXOR => self.fold_bxor(py, args),
            OpCode::BNOT => self.fold_bnot(py, args),
            OpCode::LSHIFT => self.fold_lshift(py, args),
            OpCode::RSHIFT => self.fold_rshift(py, args),

            _ => {
                // Can't fold this operation
                Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Cannot fold this operation",
                ))
            }
        }
    }

    // Arithmetic folding implementations
    fn fold_add(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.add(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            // Chained operations: args[0] is a tuple/list
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.add(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_sub(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.sub(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.sub(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_mul(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.mul(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.mul(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_truediv(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.div(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.div(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_floordiv(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.floor_div(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.floor_div(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_mod(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.rem(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.rem(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_pow(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.pow(&right, py.None()).map(|r| r.unbind())
        } else if args.len() == 1 {
            // Power is right-associative
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[items.len() - 1].clone();
                    for i in (0..items.len() - 1).rev() {
                        result = items[i].pow(&result, py.None())?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_neg(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 1 {
            let arg = args.get_item(0)?;
            arg.neg().map(|r| r.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_pos(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 1 {
            let arg = args.get_item(0)?;
            arg.pos().map(|r| r.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    // Comparison folding
    fn fold_eq(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let values = self.extract_values(args)?;
        if values.len() >= 2 {
            let mut left = values[0].clone();
            for val in &values[1..] {
                if !left.eq(val)? {
                    let py_false = py.import("builtins")?.getattr("bool")?.call1((false,))?;
                    return Ok(py_false.unbind());
                }
                left = val.clone();
            }
            let py_true = py.import("builtins")?.getattr("bool")?.call1((true,))?;
            Ok(py_true.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_ne(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let values = self.extract_values(args)?;
        if values.len() >= 2 {
            let mut left = values[0].clone();
            for val in &values[1..] {
                if !left.ne(val)? {
                    let py_false = py.import("builtins")?.getattr("bool")?.call1((false,))?;
                    return Ok(py_false.unbind());
                }
                left = val.clone();
            }
            let py_true = py.import("builtins")?.getattr("bool")?.call1((true,))?;
            Ok(py_true.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_lt(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let values = self.extract_values(args)?;
        if values.len() >= 2 {
            let mut left = values[0].clone();
            for val in &values[1..] {
                if !left.lt(val)? {
                    let py_false = py.import("builtins")?.getattr("bool")?.call1((false,))?;
                    return Ok(py_false.unbind());
                }
                left = val.clone();
            }
            let py_true = py.import("builtins")?.getattr("bool")?.call1((true,))?;
            Ok(py_true.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_le(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let values = self.extract_values(args)?;
        if values.len() >= 2 {
            let mut left = values[0].clone();
            for val in &values[1..] {
                if !left.le(val)? {
                    let py_false = py.import("builtins")?.getattr("bool")?.call1((false,))?;
                    return Ok(py_false.unbind());
                }
                left = val.clone();
            }
            let py_true = py.import("builtins")?.getattr("bool")?.call1((true,))?;
            Ok(py_true.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_gt(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let values = self.extract_values(args)?;
        if values.len() >= 2 {
            let mut left = values[0].clone();
            for val in &values[1..] {
                if !left.gt(val)? {
                    let py_false = py.import("builtins")?.getattr("bool")?.call1((false,))?;
                    return Ok(py_false.unbind());
                }
                left = val.clone();
            }
            let py_true = py.import("builtins")?.getattr("bool")?.call1((true,))?;
            Ok(py_true.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_ge(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let values = self.extract_values(args)?;
        if values.len() >= 2 {
            let mut left = values[0].clone();
            for val in &values[1..] {
                if !left.ge(val)? {
                    let py_false = py.import("builtins")?.getattr("bool")?.call1((false,))?;
                    return Ok(py_false.unbind());
                }
                left = val.clone();
            }
            let py_true = py.import("builtins")?.getattr("bool")?.call1((true,))?;
            Ok(py_true.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    // Logical folding
    fn fold_and(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() >= 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            let left_bool = left.is_truthy()?;
            let right_bool = right.is_truthy()?;
            let result = left_bool && right_bool;
            let py_bool = py.import("builtins")?.getattr("bool")?.call1((result,))?;
            Ok(py_bool.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let mut result = true;
                for item in seq {
                    let item = item?;
                    if !item.is_truthy()? {
                        result = false;
                        break;
                    }
                }
                let py_bool = py.import("builtins")?.getattr("bool")?.call1((result,))?;
                return Ok(py_bool.unbind());
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_or(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() >= 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            let left_bool = left.is_truthy()?;
            let right_bool = right.is_truthy()?;
            let result = left_bool || right_bool;
            let py_bool = py.import("builtins")?.getattr("bool")?.call1((result,))?;
            Ok(py_bool.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let mut result = false;
                for item in seq {
                    let item = item?;
                    if item.is_truthy()? {
                        result = true;
                        break;
                    }
                }
                let py_bool = py.import("builtins")?.getattr("bool")?.call1((result,))?;
                return Ok(py_bool.unbind());
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_not(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 1 {
            let arg = args.get_item(0)?;
            let bool_val = arg.is_truthy()?;
            let result = !bool_val;
            let py_bool = py.import("builtins")?.getattr("bool")?.call1((result,))?;
            Ok(py_bool.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    // Bitwise folding
    fn fold_band(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() >= 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.bitand(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.bitand(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_bor(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() >= 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.bitor(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.bitor(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_bxor(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() >= 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.bitxor(&right).map(|r| r.unbind())
        } else if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                let items: Vec<Bound<'_, PyAny>> = seq.collect::<PyResult<_>>()?;
                if !items.is_empty() {
                    let mut result = items[0].clone();
                    for item in &items[1..] {
                        result = result.bitxor(item)?;
                    }
                    return Ok(result.unbind());
                }
            }
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_bnot(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 1 {
            let arg = args.get_item(0)?;
            arg.call_method0("__invert__").map(|r| r.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_lshift(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.lshift(&right).map(|r| r.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    fn fold_rshift(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() == 2 {
            let left = args.get_item(0)?;
            let right = args.get_item(1)?;
            left.rshift(&right).map(|r| r.unbind())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid args"))
        }
    }

    // Helper to extract values from args
    fn extract_values<'py>(&self, args: &Bound<'py, PyTuple>) -> PyResult<Vec<Bound<'py, PyAny>>> {
        if args.len() == 1 {
            let arg0 = args.get_item(0)?;
            if let Ok(seq) = arg0.try_iter() {
                return seq.collect::<PyResult<_>>();
            }
        }
        // Direct args
        Ok((0..args.len()).map(|i| args.get_item(i).unwrap()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pass_creation() {
        // Test que la passe peut être créée
        let _pass = ConstantFoldingPass::new();
    }

    // Tests unitaires Rust: voir semantic/tests/test_constant_folding.rs (37 tests)
    // Tests end-to-end: voir tests/optimization/test_constant_folding.py
}
