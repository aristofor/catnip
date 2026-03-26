// FILE: catnip_rs/src/semantic/dead_store_elimination.rs
//! Dead store elimination optimization pass
//!
//! Eliminates redundant assignments that are immediately overwritten:
//! - x = 1; x = 2 → x = 2
//! - Works at IR level within sequential blocks
//! - Detects stores that are overwritten before the stored value is read
//!
//! A store is "dead" if:
//! 1. A variable is assigned
//! 2. Then reassigned before any intervening use
//! 3. The first assignment's value is never read

use super::extract_var_name;
use super::opcode::OpCode;
use super::optimizer::{OptimizationPass, default_visit_ir};
use crate::constants::*;
use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

#[pyclass(name = "DeadStoreEliminationPass")]
pub struct DeadStoreEliminationPass {
    /// Track last assignment index for each variable
    last_store: RwLock<HashMap<String, usize>>,
    /// Track which indices have been read
    read_indices: RwLock<HashSet<usize>>,
}

impl DeadStoreEliminationPass {
    /// Create a new DeadStoreEliminationPass instance (Rust API)
    pub fn new() -> Self {
        DeadStoreEliminationPass {
            last_store: RwLock::new(HashMap::new()),
            read_indices: RwLock::new(HashSet::new()),
        }
    }
}

impl Default for DeadStoreEliminationPass {
    fn default() -> Self {
        Self::new()
    }
}

#[pymethods]
impl DeadStoreEliminationPass {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    /// Visit a node and apply optimizations
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Clear state from previous invocation
        self.last_store.write().unwrap().clear();
        self.read_indices.write().unwrap().clear();

        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for DeadStoreEliminationPass {
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

        // First pass: identify which stores are dead
        self.last_store.write().unwrap().clear();
        self.read_indices.write().unwrap().clear();

        let args = visited_bound.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        // Scan through all operations to find stores and reads
        for (idx, arg) in args_tuple.iter().enumerate() {
            // Check if this is a SET_LOCALS (store)
            if let Ok(ident) = arg.getattr("ident") {
                if let Ok(opcode_int) = ident.extract::<i32>() {
                    if let Some(opcode) = OpCode::from_i32(opcode_int) {
                        if opcode == OpCode::SET_LOCALS {
                            if let Ok(args_inner) = arg.getattr("args") {
                                if let Ok(args_inner_tuple) = args_inner.cast::<PyTuple>() {
                                    if args_inner_tuple.len() >= 1 {
                                        if let Some(dest_name) = extract_var_name(&args_inner_tuple.get_item(0)?) {
                                            // Mark this as a store for the variable
                                            self.last_store.write().unwrap().insert(dest_name, idx);
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                    }
                }
            }

            // Record reads: any Ref nodes in this operation
            self.record_reads(py, &arg)?;
        }

        // Second pass: filter out dead stores
        let dead_stores = self.identify_dead_stores(py, args_tuple)?;

        // If no dead stores, return the visited node unchanged
        if dead_stores.is_empty() {
            return Ok(visited);
        }

        // Create new args without dead stores
        let mut new_args = Vec::new();
        for (idx, arg) in args_tuple.iter().enumerate() {
            if !dead_stores.contains(&idx) {
                new_args.push(arg.unbind());
            }
        }
        let new_args_tuple = PyTuple::new(py, &new_args)?;

        // Create new IR with filtered args
        let ir_class = py.import(PY_MOD_TRANSFORMER)?.getattr("IR")?;
        let ident = visited_bound.getattr("ident")?;
        let kwargs = visited_bound.getattr("kwargs")?;

        let new_node = ir_class.call1((ident, new_args_tuple, kwargs))?;
        Ok(new_node.unbind())
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit_op(self, py, node)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Record reads
        if let Ok(ident) = node.getattr("ident")?.extract::<String>() {
            // Mark this variable as read
            // This will prevent earlier stores from being marked as dead
            if let Some(idx) = self.last_store.read().unwrap().get(&ident) {
                self.read_indices.write().unwrap().insert(*idx);
            }
        }

        Ok(node.clone().unbind())
    }
}

impl DeadStoreEliminationPass {
    /// Record all Ref nodes in an operation (reads of variables)
    fn record_reads(&self, _py: Python<'_>, op: &Bound<'_, PyAny>) -> PyResult<()> {
        let type_name_obj = op.get_type().name()?;
        let type_name = type_name_obj.to_str()?;

        // Handle Ref nodes directly
        if type_name == catnip::REF {
            if let Ok(ident) = op.getattr("ident")?.extract::<String>() {
                if let Some(idx) = self.last_store.read().unwrap().get(&ident) {
                    self.read_indices.write().unwrap().insert(*idx);
                }
            }
            return Ok(());
        }

        // Recursively check arguments for Ref nodes
        if type_name == "IR" || type_name == catnip::OP {
            if let Ok(args) = op.getattr("args") {
                for arg in args.try_iter()? {
                    let arg = arg?;
                    self.record_reads(_py, &arg)?;
                }
            }
            if let Ok(kwargs) = op.getattr("kwargs") {
                if let Ok(items) = kwargs.call_method0("items") {
                    for item in items.try_iter()? {
                        let item = item?;
                        let item_tuple = item.cast::<PyTuple>()?;
                        let value = item_tuple.get_item(1)?;
                        self.record_reads(_py, &value)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Identify stores that are dead (overwritten before being read)
    fn identify_dead_stores(&self, _py: Python<'_>, args_tuple: &Bound<'_, PyTuple>) -> PyResult<HashSet<usize>> {
        let mut dead_stores = HashSet::new();
        let read_indices = self.read_indices.read().unwrap();

        // For each variable, check if its stores are dead
        let mut var_stores: HashMap<String, Vec<usize>> = HashMap::new();

        // Collect all store indices for each variable
        for idx in 0..args_tuple.len() {
            let arg = args_tuple.get_item(idx)?;

            if let Ok(ident) = arg.getattr("ident") {
                if let Ok(opcode_int) = ident.extract::<i32>() {
                    if let Some(opcode) = OpCode::from_i32(opcode_int) {
                        if opcode == OpCode::SET_LOCALS {
                            if let Ok(args_inner) = arg.getattr("args") {
                                if let Ok(args_inner_tuple) = args_inner.cast::<PyTuple>() {
                                    if args_inner_tuple.len() >= 1 {
                                        if let Some(var_name) = extract_var_name(&args_inner_tuple.get_item(0)?) {
                                            var_stores.entry(var_name).or_default().push(idx);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // For each variable with multiple stores, find dead ones
        for (_var_name, store_indices) in var_stores {
            if store_indices.len() > 1 {
                // Check each store except the last to see if it's dead
                for window in store_indices.windows(2) {
                    let first_store_idx = window[0];
                    let second_store_idx = window[1];

                    // A store is dead if:
                    // 1. It's not the last store for this variable
                    // 2. The variable is not read between the two stores
                    let is_read_between = (first_store_idx + 1..second_store_idx).any(|i| read_indices.contains(&i));

                    if !is_read_between {
                        dead_stores.insert(first_store_idx);
                    }
                }
            }
        }

        Ok(dead_stores)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_pass() {
        let pass = DeadStoreEliminationPass::new();
        assert_eq!(pass.last_store.read().unwrap().len(), 0);
        assert_eq!(pass.read_indices.read().unwrap().len(), 0);
    }

    #[test]
    fn test_multiple_stores_same_var() {
        let pass = DeadStoreEliminationPass::new();
        // Simulate scenario: x = 1; x = 2
        pass.last_store.write().unwrap().insert("x".to_string(), 0);
        pass.last_store.write().unwrap().insert("x".to_string(), 1);

        assert_eq!(pass.last_store.read().unwrap().len(), 1); // Last one wins
        pass.last_store.write().unwrap().clear();
        assert_eq!(pass.last_store.read().unwrap().len(), 0);
    }

    #[test]
    fn test_read_tracking() {
        let pass = DeadStoreEliminationPass::new();
        // Simulate variable read
        pass.last_store.write().unwrap().insert("x".to_string(), 0);
        pass.read_indices.write().unwrap().insert(0);

        assert!(pass.read_indices.read().unwrap().contains(&0));
        pass.read_indices.write().unwrap().clear();
        assert_eq!(pass.read_indices.read().unwrap().len(), 0);
    }
}
