// FILE: catnip_rs/src/semantic/dead_code_elimination.rs
//! Dead code elimination optimization pass
//!
//! Port of DeadCodeEliminationPass from catnip/semantic/optimizer.pyx
//!
//! Removes code that will never execute:
//! - if True { block1 } else { block2 } → block1
//! - if False { block } → else_block (or None)
//! - while False { block } → None
//! - block() → None (empty block)
//! - match x { _ if False => ... } → case removed
//! - match x { _ if True => body } → guard simplified
//! - match x { _ => body, ... } → trailing cases removed (unreachable)
//! - match x { _ => body } (single catchall) → body

use super::opcode::OpCode;
use super::optimizer::{OptimizationPass, default_visit_ir};
use crate::constants::*;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

#[pyclass(name = "DeadCodeEliminationPass")]
pub struct DeadCodeEliminationPass;

#[pymethods]
impl DeadCodeEliminationPass {
    #[new]
    fn new() -> Self {
        DeadCodeEliminationPass
    }

    /// Visit a node and apply optimizations
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for DeadCodeEliminationPass {
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

        // Apply dead code elimination
        self.eliminate_dead_code(py, visited_bound)
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit_op(self, py, node)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node.clone().unbind())
    }
}

impl DeadCodeEliminationPass {
    /// Eliminate dead code from an IR node
    fn eliminate_dead_code(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let ident = node.getattr("ident")?;
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        // Handle if with constant condition
        if self.is_op_match(&ident, OpCode::OP_IF)? && args_tuple.len() >= 1 {
            let branches = args_tuple.get_item(0)?;
            let else_block = if args_tuple.len() > 1 {
                Some(args_tuple.get_item(1)?)
            } else {
                None
            };

            // Check if first branch has constant True condition
            if let Ok(branches_iter) = branches.try_iter() {
                let branches_vec: Vec<Bound<'_, PyAny>> = branches_iter.collect::<PyResult<_>>()?;

                if !branches_vec.is_empty() {
                    let first_branch = &branches_vec[0];
                    if let Ok(branch_tuple) = first_branch.cast::<PyTuple>() {
                        if branch_tuple.len() >= 2 {
                            let condition = branch_tuple.get_item(0)?;
                            let block = branch_tuple.get_item(1)?;

                            // if True { block } → block
                            if self.is_python_true(&condition)? {
                                return Ok(block.unbind());
                            }
                        }
                    }
                }

                // Check if all conditions are False
                let mut all_false = true;
                for branch in &branches_vec {
                    if let Ok(branch_tuple) = branch.cast::<PyTuple>() {
                        if branch_tuple.len() >= 1 {
                            let condition = branch_tuple.get_item(0)?;
                            if !self.is_python_false(&condition)? {
                                all_false = false;
                                break;
                            }
                        }
                    }
                }

                if all_false {
                    // No branch will execute, return else block or None
                    return Ok(else_block.map(|b| b.unbind()).unwrap_or_else(|| py.None()));
                }
            }
        }

        // while with constant False condition
        if self.is_op_match(&ident, OpCode::OP_WHILE)? && args_tuple.len() == 2 {
            let condition = args_tuple.get_item(0)?;
            // while False { block } → None
            if self.is_python_false(&condition)? {
                return Ok(py.None());
            }
        }

        // Block with no statements
        if self.is_op_match(&ident, OpCode::OP_BLOCK)? && args_tuple.len() == 0 {
            return Ok(py.None());
        }

        // Match with dead cases
        if self.is_op_match(&ident, OpCode::OP_MATCH)? && args_tuple.len() == 2 {
            let value_expr = args_tuple.get_item(0)?;
            let cases = args_tuple.get_item(1)?;

            if let Ok(cases_iter) = cases.try_iter() {
                let cases_vec: Vec<Bound<'_, PyAny>> = cases_iter.collect::<PyResult<_>>()?;
                let mut live_cases: Vec<Bound<'_, PyAny>> = Vec::new();

                for case in &cases_vec {
                    let pattern = case.get_item(0)?;
                    let guard = case.get_item(1)?;
                    let body = case.get_item(2)?;

                    // Guard constant False: skip case entirely
                    if !guard.is_none() && self.is_python_false(&guard)? {
                        continue;
                    }

                    let is_catchall = self.is_catchall_pattern(&pattern)?;

                    // Guard constant True: simplify to no guard
                    let effective_no_guard;
                    if !guard.is_none() && self.is_python_true(&guard)? {
                        let simplified = PyTuple::new(py, &[pattern.unbind(), py.None(), body.unbind()])?;
                        live_cases.push(simplified.into_any());
                        effective_no_guard = true;
                    } else {
                        effective_no_guard = guard.is_none();
                        live_cases.push(case.clone());
                    }

                    // Wildcard/Var without guard is a catchall: stop (remaining cases unreachable)
                    if effective_no_guard && is_catchall {
                        break;
                    }
                }

                // Single catchall without guard: replace match with body
                if live_cases.len() == 1 {
                    let sole_case = &live_cases[0];
                    let pattern = sole_case.get_item(0)?;
                    let guard = sole_case.get_item(1)?;
                    if guard.is_none() && self.is_catchall_pattern(&pattern)? {
                        let body = sole_case.get_item(2)?;
                        return Ok(body.unbind());
                    }
                }

                // Rebuild if cases changed
                if live_cases.len() != cases_vec.len() {
                    if live_cases.is_empty() {
                        return Ok(py.None());
                    }
                    let ir_class = py.import(PY_MOD_TRANSFORMER)?.getattr("IR")?;
                    let new_cases = PyTuple::new(py, &live_cases)?;
                    let new_args = PyTuple::new(py, &[value_expr.unbind(), new_cases.into_any().unbind()])?;
                    let kwargs = node.getattr("kwargs")?;
                    return ir_class
                        .call1((node.getattr("ident")?, new_args, kwargs))
                        .map(|obj| obj.unbind());
                }
            }
        }

        // No elimination applied
        Ok(node.clone().unbind())
    }

    /// Check if ident matches an opcode
    fn is_op_match(&self, ident: &Bound<'_, PyAny>, opcode: OpCode) -> PyResult<bool> {
        if let Ok(int_val) = ident.extract::<i32>() {
            return Ok(int_val == opcode as i32);
        }
        Ok(false)
    }

    /// Check if object is a Python truthy constant (bool, int, float, str, None)
    fn is_python_true(&self, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
        // bool first (bool is subclass of int in Python)
        if let Ok(val) = obj.extract::<bool>() {
            return Ok(val);
        }
        if let Ok(val) = obj.extract::<i64>() {
            return Ok(val != 0);
        }
        if let Ok(val) = obj.extract::<f64>() {
            return Ok(val != 0.0);
        }
        if let Ok(val) = obj.extract::<&str>() {
            return Ok(!val.is_empty());
        }
        if obj.is_none() {
            return Ok(false);
        }
        Ok(false)
    }

    /// Check if object is a Python falsy constant (bool, int, float, str, None)
    fn is_python_false(&self, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(val) = obj.extract::<bool>() {
            return Ok(!val);
        }
        if let Ok(val) = obj.extract::<i64>() {
            return Ok(val == 0);
        }
        if let Ok(val) = obj.extract::<f64>() {
            return Ok(val == 0.0);
        }
        if let Ok(val) = obj.extract::<&str>() {
            return Ok(val.is_empty());
        }
        if obj.is_none() {
            return Ok(true);
        }
        Ok(false)
    }

    /// Check if pattern is a catchall (PatternWildcard or PatternVar)
    fn is_catchall_pattern(&self, pattern: &Bound<'_, PyAny>) -> PyResult<bool> {
        let type_name = pattern.get_type().name()?;
        let name = type_name.to_str()?;
        Ok(name == "PatternWildcard" || name == "PatternVar")
    }
}
