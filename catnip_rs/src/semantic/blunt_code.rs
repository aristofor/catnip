// FILE: catnip_rs/src/semantic/blunt_code.rs
//! Blunt code simplification pass
//!
//! Port of catnip/semantic/blunt_code.pyx
//!
//! Simplifies "blunt" code patterns that arise from the parser:
//! - Double negation: not not x → x
//! - NOT inversion on comparisons: not (a == b) → a != b
//! - Boolean comparisons: x == True → x, x == False → not x
//! - Arithmetic identities: x + 0 → x, x * 1 → x, x * 0 → 0
//! - Logical short-circuits: x and False → False, x or True → True
//! - Idempotence: x and x → x, x or x → x
//! - Complement: x and (not x) → False, x or (not x) → True
//! - Constant conditions: if True {...} → {...}

use super::opcode::OpCode;
use super::optimizer::{default_visit_ir, default_visit_op, OptimizationPass};
use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyList, PyTuple};

#[pyclass(name = "BluntCodePass")]
pub struct BluntCodePass;

impl Default for BluntCodePass {
    fn default() -> Self {
        Self::new()
    }
}

impl BluntCodePass {
    /// Create a new BluntCodePass instance (Rust API)
    pub fn new() -> Self {
        BluntCodePass
    }
}

#[pymethods]
impl BluntCodePass {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    /// Visit a node and apply optimizations
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for BluntCodePass {
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit(self, py, node)
    }

    fn visit_ir(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // First visit children using default implementation
        let visited = default_visit_ir(self, py, node)?;
        let visited_bound = visited.bind(py);

        // Check if result is still an IR node
        let node_type = visited_bound.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;
        if type_name != "IR" {
            return Ok(visited);
        }

        // Apply simplifications
        self.simplify_operation(py, visited_bound)
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // First visit children using default implementation
        let visited = default_visit_op(self, py, node)?;
        let visited_bound = visited.bind(py);

        // Check if result is still an Op node
        let node_type = visited_bound.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;
        if type_name != "Op" {
            return Ok(visited);
        }

        // Apply simplifications
        self.simplify_operation(py, visited_bound)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node.clone().unbind())
    }
}

impl BluntCodePass {
    /// Apply simplifications to an operation node (IR or Op)
    fn simplify_operation(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let ident = node.getattr("ident")?;
        let args = node.getattr("args")?;
        let kwargs = node.getattr("kwargs")?;

        // Get node type for reconstruction
        let node_type = node.get_type();

        // Double negation: not not x → x
        // NOT inversion on comparisons: not (a == b) → a != b
        if self.is_op(py, &ident, OpCode::NOT, &["not", "bool_not"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() > 0 {
                    let inner = args_tuple.get_item(0)?;
                    let inner_type = inner.get_type();
                    let inner_type_name_obj = inner_type.name()?;
                    let inner_type_name = inner_type_name_obj.to_str()?;
                    if inner_type_name == catnip::IR || inner_type_name == catnip::OP {
                        if let Ok(inner_ident) = inner.getattr("ident") {
                            // Double negation: not not x → x
                            if self.is_op(py, &inner_ident, OpCode::NOT, &["not", "bool_not"])? {
                                if let Ok(inner_args) = inner.getattr("args") {
                                    if let Ok(inner_tuple) = inner_args.cast::<PyTuple>() {
                                        if inner_tuple.len() > 0 {
                                            return Ok(inner_tuple.get_item(0)?.unbind());
                                        }
                                    }
                                }
                            }

                            // NOT inversion: not (a cmp b) → a inv_cmp b
                            if let Ok(inner_opcode) = inner_ident.extract::<i32>() {
                                if let Some(inverted) = invert_comparison(inner_opcode) {
                                    let inner_args = inner.getattr("args")?;
                                    let inner_kwargs = inner.getattr("kwargs")?;
                                    let inv_ident = inverted.into_pyobject(py)?.into_any().unbind();
                                    return inner
                                        .get_type()
                                        .call1((inv_ident, inner_args, inner_kwargs))
                                        .map(|n| n.unbind());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Comparison with True/False
        if self.is_op(py, &ident, OpCode::EQ, &["eq"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    let left = args_tuple.get_item(0)?;
                    let right = args_tuple.get_item(1)?;

                    // x == True → x
                    if is_python_true(py, &right)? {
                        return Ok(left.unbind());
                    }
                    // True == x → x
                    if is_python_true(py, &left)? {
                        return Ok(right.unbind());
                    }
                    // x == False → not x
                    if is_python_false(py, &right)? {
                        let not_ident = if is_int_ident(&ident.unbind()) {
                            (OpCode::NOT as i32).into_pyobject(py)?.into_any().unbind()
                        } else {
                            "not".into_pyobject(py)?.into_any().unbind()
                        };
                        let new_args = PyTuple::new(py, &[left.unbind()])?;
                        let empty_dict = py.import("builtins")?.getattr("dict")?.call0()?;
                        return node_type
                            .call1((not_ident, new_args, empty_dict))
                            .map(|n| n.unbind());
                    }
                    // False == x → not x
                    if is_python_false(py, &left)? {
                        let not_ident = if is_int_ident(&ident.unbind()) {
                            (OpCode::NOT as i32).into_pyobject(py)?.into_any().unbind()
                        } else {
                            "not".into_pyobject(py)?.into_any().unbind()
                        };
                        let new_args = PyTuple::new(py, &[right.unbind()])?;
                        let empty_dict = py.import("builtins")?.getattr("dict")?.call0()?;
                        return node_type
                            .call1((not_ident, new_args, empty_dict))
                            .map(|n| n.unbind());
                    }
                }
            }
        }

        // Arithmetic identities - ADD
        if self.is_op(py, &ident, OpCode::ADD, &["add"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    let left = args_tuple.get_item(0)?;
                    let right = args_tuple.get_item(1)?;
                    // x + 0 → x
                    if is_zero(py, &right)? {
                        return Ok(left.unbind());
                    }
                    // 0 + x → x
                    if is_zero(py, &left)? {
                        return Ok(right.unbind());
                    }
                }
            }
        }

        // SUB
        if self.is_op(py, &ident, OpCode::SUB, &["sub"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    let left = args_tuple.get_item(0)?;
                    let right = args_tuple.get_item(1)?;
                    // x - 0 → x
                    if is_zero(py, &right)? {
                        return Ok(left.unbind());
                    }
                }
            }
        }

        // MUL
        if self.is_op(py, &ident, OpCode::MUL, &["mul"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    let left = args_tuple.get_item(0)?;
                    let right = args_tuple.get_item(1)?;
                    // x * 0 → 0
                    if is_zero(py, &right)? {
                        return Ok(0i64.into_pyobject(py)?.into_any().unbind());
                    }
                    if is_zero(py, &left)? {
                        return Ok(0i64.into_pyobject(py)?.into_any().unbind());
                    }
                    // x * 1 → x
                    if is_one(py, &right)? {
                        return Ok(left.unbind());
                    }
                    // 1 * x → x
                    if is_one(py, &left)? {
                        return Ok(right.unbind());
                    }
                }
            }
        }

        // DIV / TRUEDIV
        if self.is_op(py, &ident, OpCode::DIV, &["div", "truediv"])?
            || self.is_op(py, &ident, OpCode::TRUEDIV, &["truediv"])?
        {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    let left = args_tuple.get_item(0)?;
                    let right = args_tuple.get_item(1)?;
                    // x / 1 → x
                    if is_one(py, &right)? {
                        return Ok(left.unbind());
                    }
                }
            }
        }

        // FLOORDIV
        if self.is_op(py, &ident, OpCode::FLOORDIV, &["floordiv"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    let left = args_tuple.get_item(0)?;
                    let right = args_tuple.get_item(1)?;
                    // x // 1 → x
                    if is_one(py, &right)? {
                        return Ok(left.unbind());
                    }
                }
            }
        }

        // Logical operations - AND
        if self.is_op(py, &ident, OpCode::AND, &["and", "bool_and"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    let left = args_tuple.get_item(0)?;
                    let right = args_tuple.get_item(1)?;
                    // x and False → False
                    if is_python_false(py, &right)? {
                        let py_false = py.import("builtins")?.getattr("bool")?.call1((false,))?;
                        return Ok(py_false.unbind());
                    }
                    // False and x → False
                    if is_python_false(py, &left)? {
                        let py_false = py.import("builtins")?.getattr("bool")?.call1((false,))?;
                        return Ok(py_false.unbind());
                    }
                    // x and True → x
                    if is_python_true(py, &right)? {
                        return Ok(left.unbind());
                    }
                    // True and x → x
                    if is_python_true(py, &left)? {
                        return Ok(right.unbind());
                    }
                    // x and x → x (idempotence)
                    if left.eq(&right)? {
                        return Ok(left.unbind());
                    }
                    // x and (not x) → False, (not x) and x → False (complement)
                    if is_negation_of(py, &left, &right)? || is_negation_of(py, &right, &left)? {
                        return Ok(PyBool::new(py, false).to_owned().into_any().unbind());
                    }
                }
            }
        }

        // OR
        if self.is_op(py, &ident, OpCode::OR, &["or", "bool_or"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    let left = args_tuple.get_item(0)?;
                    let right = args_tuple.get_item(1)?;
                    // x or True → True
                    if is_python_true(py, &right)? {
                        let py_true = py.import("builtins")?.getattr("bool")?.call1((true,))?;
                        return Ok(py_true.unbind());
                    }
                    // True or x → True
                    if is_python_true(py, &left)? {
                        let py_true = py.import("builtins")?.getattr("bool")?.call1((true,))?;
                        return Ok(py_true.unbind());
                    }
                    // x or False → x
                    if is_python_false(py, &right)? {
                        return Ok(left.unbind());
                    }
                    // False or x → x
                    if is_python_false(py, &left)? {
                        return Ok(right.unbind());
                    }
                    // x or x → x (idempotence)
                    if left.eq(&right)? {
                        return Ok(left.unbind());
                    }
                    // x or (not x) → True, (not x) or x → True (complement)
                    if is_negation_of(py, &left, &right)? || is_negation_of(py, &right, &left)? {
                        return Ok(PyBool::new(py, true).to_owned().into_any().unbind());
                    }
                }
            }
        }

        // If with constant condition
        if self.is_op(py, &ident, OpCode::OP_IF, &["if"])? {
            if let Ok(args_tuple) = args.cast::<PyTuple>() {
                if args_tuple.len() >= 1 {
                    let first_arg = args_tuple.get_item(0)?;
                    // Check if it's a list or tuple with branches
                    if first_arg.is_instance_of::<PyList>() || first_arg.is_instance_of::<PyTuple>()
                    {
                        if let Ok(branches_iter) = first_arg.try_iter() {
                            if let Some(Ok(first_branch)) = branches_iter.into_iter().next() {
                                // first_branch should be a (condition, then_body) tuple
                                if let Ok(branch_tuple) = first_branch.cast::<PyTuple>() {
                                    if branch_tuple.len() >= 2 {
                                        let condition = branch_tuple.get_item(0)?;
                                        let then_body = branch_tuple.get_item(1)?;

                                        // if True { then } else { else } → then
                                        if is_python_true(py, &condition)? {
                                            return Ok(then_body.unbind());
                                        }

                                        // if False { then } else { else } → else (if present)
                                        if is_python_false(py, &condition)? {
                                            if args_tuple.len() >= 2 {
                                                return Ok(args_tuple.get_item(1)?.unbind());
                                            } else {
                                                return Ok(py.None());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // No simplification applied, reconstruct node with original args/kwargs
        node_type.call1((ident, args, kwargs)).map(|n| n.unbind())
    }

    /// Check if ident matches opcode or any of the string operations
    fn is_op(
        &self,
        _py: Python<'_>,
        ident: &Bound<'_, PyAny>,
        opcode: OpCode,
        str_ops: &[&str],
    ) -> PyResult<bool> {
        // Try int first
        if let Ok(int_val) = ident.extract::<i32>() {
            return Ok(int_val == opcode as i32);
        }

        // Try string
        if let Ok(str_val) = ident.extract::<String>() {
            for str_op in str_ops {
                if str_val == *str_op {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }
}

// Helper functions for Python value checking

fn is_int_ident(ident: &Py<PyAny>) -> bool {
    Python::attach(|py| ident.bind(py).extract::<i32>().is_ok())
}

fn is_python_true(_py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    // Check if object is Python True singleton
    if let Ok(val) = obj.extract::<bool>() {
        return Ok(val);
    }
    // Also compare identity with True
    if obj.is_truthy()? {
        // Extract as bool and check if it's exactly True (not just truthy)
        if let Ok(b) = obj.extract::<bool>() {
            return Ok(b);
        }
    }
    Ok(false)
}

fn is_python_false(_py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    // Check if object is Python False singleton
    if let Ok(val) = obj.extract::<bool>() {
        return Ok(!val);
    }
    Ok(false)
}

fn is_zero(_py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    // Try to extract as int or float and check if zero
    if let Ok(val) = obj.extract::<i64>() {
        return Ok(val == 0);
    }
    if let Ok(val) = obj.extract::<f64>() {
        return Ok(val == 0.0);
    }
    Ok(false)
}

fn is_one(_py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    // Try to extract as int or float and check if one
    if let Ok(val) = obj.extract::<i64>() {
        return Ok(val == 1);
    }
    if let Ok(val) = obj.extract::<f64>() {
        return Ok(val == 1.0);
    }
    Ok(false)
}

/// Invert a comparison opcode: EQ↔NE, LT↔GE, LE↔GT
fn invert_comparison(opcode: i32) -> Option<i32> {
    match opcode {
        x if x == OpCode::EQ as i32 => Some(OpCode::NE as i32),
        x if x == OpCode::NE as i32 => Some(OpCode::EQ as i32),
        x if x == OpCode::LT as i32 => Some(OpCode::GE as i32),
        x if x == OpCode::LE as i32 => Some(OpCode::GT as i32),
        x if x == OpCode::GT as i32 => Some(OpCode::LE as i32),
        x if x == OpCode::GE as i32 => Some(OpCode::LT as i32),
        _ => None,
    }
}

/// Check if `a` is a NOT node whose argument equals `b`
fn is_negation_of(_py: Python<'_>, a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>) -> PyResult<bool> {
    let type_name_obj = a.get_type().name()?;
    let type_name = type_name_obj.to_str()?;
    if type_name != catnip::IR && type_name != catnip::OP {
        return Ok(false);
    }
    let ident = a.getattr("ident")?;
    if let Ok(int_val) = ident.extract::<i32>() {
        if int_val != OpCode::NOT as i32 {
            return Ok(false);
        }
    } else if let Ok(str_val) = ident.extract::<String>() {
        if str_val != "not" && str_val != "bool_not" {
            return Ok(false);
        }
    } else {
        return Ok(false);
    }
    let args = a.getattr("args")?;
    if let Ok(args_tuple) = args.cast::<PyTuple>() {
        if args_tuple.len() > 0 {
            let inner = args_tuple.get_item(0)?;
            return inner.eq(b);
        }
    }
    Ok(false)
}
