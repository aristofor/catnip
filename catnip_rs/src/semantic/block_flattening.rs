// FILE: catnip_rs/src/semantic/block_flattening.rs
//! Block flattening optimization pass
//!
//! Port of BlockFlatteningPass from catnip/semantic/optimizer.pyx
//!
//! Merges nested blocks into single blocks:
//! block(block(stmt1, stmt2), stmt3) → block(stmt1, stmt2, stmt3)

use super::opcode::OpCode;
use super::optimizer::{default_visit_ir, OptimizationPass};
use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

#[pyclass(name = "BlockFlatteningPass")]
pub struct BlockFlatteningPass;

#[pymethods]
impl BlockFlatteningPass {
    #[new]
    fn new() -> Self {
        BlockFlatteningPass
    }

    /// Visit a node and apply optimizations
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for BlockFlatteningPass {
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

        // Get ident to check if it's a block
        let ident = visited_bound.getattr("ident")?;

        // Only flatten 'block' operations
        if !self.is_block_op(py, &ident)? {
            return Ok(visited);
        }

        // Flatten nested blocks
        let args = visited_bound.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        let mut flattened_args = Vec::new();
        let mut changed = false;

        for arg in args_tuple.iter() {
            // Check if arg is an IR node with ident == OP_BLOCK
            let arg_type = arg.get_type();
            let arg_type_name_obj = arg_type.name()?;
            let arg_type_name = arg_type_name_obj.to_str()?;
            if arg_type_name == catnip::IR {
                if let Ok(arg_ident) = arg.getattr("ident") {
                    if self.is_block_op(py, &arg_ident)? {
                        // Inline the nested block's statements
                        if let Ok(nested_args) = arg.getattr("args") {
                            if let Ok(nested_tuple) = nested_args.cast::<PyTuple>() {
                                for nested_arg in nested_tuple.iter() {
                                    flattened_args.push(nested_arg.unbind());
                                }
                                changed = true;
                                continue;
                            }
                        }
                    }
                }
            }

            // Not a nested block, keep as-is
            flattened_args.push(arg.unbind());
        }

        // If nothing changed, return original
        if !changed {
            return Ok(visited);
        }

        // Create new flattened block
        let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
        let new_args = PyTuple::new(py, &flattened_args)?;
        let kwargs = visited_bound.getattr("kwargs")?;
        let new_node = ir_class.call1((ident, new_args, kwargs))?;
        Ok(new_node.unbind())
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit_op(self, py, node)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node.clone().unbind())
    }
}

impl BlockFlatteningPass {
    /// Check if ident is OpCode::OP_BLOCK or "block" string
    fn is_block_op(&self, _py: Python<'_>, ident: &Bound<'_, PyAny>) -> PyResult<bool> {
        // Try int first
        if let Ok(int_val) = ident.extract::<i32>() {
            return Ok(int_val == OpCode::OP_BLOCK as i32);
        }

        // Try string
        if let Ok(str_val) = ident.extract::<String>() {
            return Ok(str_val == "block");
        }

        Ok(false)
    }
}
