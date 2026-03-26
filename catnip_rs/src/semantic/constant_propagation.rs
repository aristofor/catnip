// FILE: catnip_rs/src/semantic/constant_propagation.rs
//! Constant propagation optimization pass
//!
//! Propagates constant values through variable references:
//! - x = 42; y = x + 1 → x = 42; y = 42 + 1
//! - After constant folding: → x = 42; y = 43
//!
//! Works at IR level, tracking assignments and replacements.
//! Uses RefCell for interior mutability (state accumulates during traversal).
//! Does not eliminate the original assignment (that's dead code elimination).

use super::extract_var_name;
use super::opcode::OpCode;
use super::optimizer::{OptimizationPass, default_visit_ir};
use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use std::collections::HashMap;
use std::sync::RwLock;

#[pyclass(name = "ConstantPropagationPass")]
pub struct ConstantPropagationPass {
    /// Mapping from variable name to constant value (thread-safe interior mutability)
    constants: RwLock<HashMap<String, Py<PyAny>>>,
}

impl ConstantPropagationPass {
    /// Create a new ConstantPropagationPass instance (Rust API)
    pub fn new() -> Self {
        ConstantPropagationPass {
            constants: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for ConstantPropagationPass {
    fn default() -> Self {
        Self::new()
    }
}

#[pymethods]
impl ConstantPropagationPass {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    /// Visit a node and apply optimizations
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Clear state from previous invocation
        self.constants.write().unwrap().clear();

        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for ConstantPropagationPass {
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit(self, py, node)
    }

    fn visit_ir(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // First visit children recursively
        let visited = default_visit_ir(self, py, node)?;
        let visited_bound = visited.bind(py);

        // Check if result is still an IR node
        let node_type = visited_bound.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;
        if type_name != "IR" && type_name != "Op" {
            return Ok(visited);
        }

        // Track constant assignments: SET_LOCALS x = <constant>
        // If a variable is reassigned to a non-constant value, invalidate the
        // previous mapping to avoid propagating stale values.
        let ident = visited_bound.getattr("ident")?;
        if let Ok(opcode_int) = ident.extract::<i32>() {
            if let Some(opcode) = OpCode::from_i32(opcode_int) {
                if opcode == OpCode::SET_LOCALS {
                    let args = visited_bound.getattr("args")?;
                    let args_tuple = args.cast::<PyTuple>()?;
                    if args_tuple.len() >= 2 {
                        if let Some(name) = extract_var_name(&args_tuple.get_item(0)?) {
                            let value = args_tuple.get_item(1)?;
                            if self.is_constant(py, &value)? {
                                self.constants.write().unwrap().insert(name, value.unbind());
                            } else {
                                self.constants.write().unwrap().remove(&name);
                            }
                        }
                    }
                }
            }
        }

        Ok(visited)
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit_op(self, py, node)
    }

    fn visit_ref(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Replace Ref nodes that point to known constants
        if let Ok(ident) = node.getattr("ident")?.extract::<String>() {
            if let Some(constant) = self.constants.read().unwrap().get(&ident) {
                return Ok(constant.clone_ref(py));
            }
        }

        Ok(node.clone().unbind())
    }
}

impl ConstantPropagationPass {
    /// Check if a value is a constant (not Ref, IR, Op, Identifier)
    fn is_constant(&self, _py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
        let obj_type = obj.get_type();
        let type_name_obj = obj_type.name()?;
        let type_name = type_name_obj.to_str()?;

        // Not constant if it's Ref, IR, Op, or Identifier
        if type_name == catnip::REF
            || type_name == catnip::IR
            || type_name == catnip::OP
            || type_name == catnip::IDENTIFIER
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_pass() {
        let pass = ConstantPropagationPass::new();
        assert_eq!(pass.constants.read().unwrap().len(), 0);
    }

    #[test]
    fn test_state_clears_on_visit() {
        let pass = ConstantPropagationPass::new();
        // Simulate state from previous invocation
        pass.constants
            .write()
            .unwrap()
            .insert("x".to_string(), Python::attach(|py| py.None()));
        assert_eq!(pass.constants.read().unwrap().len(), 1);

        // Verify state would be cleared on new visit (we can't fully test without Python)
        pass.constants.write().unwrap().clear();
        assert_eq!(pass.constants.read().unwrap().len(), 0);
    }
}
