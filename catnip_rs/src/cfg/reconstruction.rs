// FILE: catnip_rs/src/cfg/reconstruction.rs
//! CFG reconstruction - rebuild control flow structures from CFG.
//!
//! Transforms a linearized CFG back into structured Op nodes (if, while, for).
//! This allows CFG optimizations to be integrated into the semantic pipeline.

use super::edge::EdgeType;
use super::graph::ControlFlowGraph;
use crate::core::op::Op;
use crate::ir::opcode::IROpCode;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::collections::HashSet;

/// Reconstruct structured code from CFG.
pub struct CFGReconstructor<'py> {
    py: Python<'py>,
    cfg: &'py ControlFlowGraph,
    visited: HashSet<usize>,
}

impl<'py> CFGReconstructor<'py> {
    pub fn new(py: Python<'py>, cfg: &'py ControlFlowGraph) -> Self {
        Self {
            py,
            cfg,
            visited: HashSet::new(),
        }
    }

    /// Reconstruct Op nodes from CFG starting at entry block.
    pub fn reconstruct(&mut self) -> PyResult<Vec<Op>> {
        let Some(entry_id) = self.cfg.entry else {
            return Ok(Vec::new());
        };

        let Some(exit_id) = self.cfg.exit else {
            return Ok(Vec::new());
        };

        // Skip empty CFG (only entry/exit)
        if self.cfg.blocks.len() <= 2 {
            return Ok(Vec::new());
        }

        self.reconstruct_region(entry_id, exit_id)
    }

    /// Reconstruct a region of the CFG between start and end blocks.
    fn reconstruct_region(&mut self, start: usize, end: usize) -> PyResult<Vec<Op>> {
        let mut result = Vec::new();
        let mut current = start;

        while current != end && !self.visited.contains(&current) {
            self.visited.insert(current);

            let Some(block) = self.cfg.blocks.get(&current) else {
                break;
            };

            // Analyze successors to determine control flow structure
            match block.successors.len() {
                0 => {
                    // No successors - end of region
                    result.extend(block.instructions.clone());
                    break;
                }

                1 => {
                    // Single successor - linear flow or loop back-edge
                    let edge_id = block.successors[0];
                    let edge = &self.cfg.edges[edge_id];
                    let target = edge.target;

                    // Check for while loop (back-edge from loop body to header)
                    if self.is_back_edge(current, target) {
                        // This is the end of a loop body - don't reconstruct the loop here
                        // (it will be reconstructed when we visit the header)
                        result.extend(block.instructions.clone());
                        break;
                    } else {
                        // Linear flow
                        result.extend(block.instructions.clone());
                        current = target;
                    }
                }

                2 => {
                    // Two-way branch - could be if/else or while loop header
                    // Check if this is a while loop header
                    if self.is_while_header(current) {
                        // While loop header - reconstruct the loop
                        if let Some(while_op) = self.reconstruct_while(current, current)? {
                            result.push(while_op);
                        }
                        // Continue from loop exit
                        if let Some(exit) = self.find_loop_exit(current)? {
                            current = exit;
                        } else {
                            break;
                        }
                    } else {
                        // Regular if/else structure
                        if let Some((if_op, next_block)) = self.reconstruct_if(current, end)? {
                            result.push(if_op);
                            current = next_block;
                        } else {
                            break;
                        }
                    }
                }

                _ => {
                    // Multiple branches (match statement) - not implemented yet
                    result.extend(block.instructions.clone());
                    break;
                }
            }
        }

        Ok(result)
    }

    /// Reconstruct an if/else structure.
    fn reconstruct_if(&mut self, block_id: usize, end: usize) -> PyResult<Option<(Op, usize)>> {
        let block = self.cfg.blocks.get(&block_id).unwrap();

        // Get the two conditional edges
        let edge1_id = block.successors[0];
        let edge2_id = block.successors[1];

        let edge1 = &self.cfg.edges[edge1_id];
        let edge2 = &self.cfg.edges[edge2_id];

        // Identify true and false branches
        let (true_target, false_target) = match (&edge1.edge_type, &edge2.edge_type) {
            (EdgeType::ConditionalTrue, EdgeType::ConditionalFalse) => (edge1.target, edge2.target),
            (EdgeType::ConditionalFalse, EdgeType::ConditionalTrue) => (edge2.target, edge1.target),
            _ => return Ok(None), // Not a proper if structure
        };

        // Find the merge point (post-dominator of both branches)
        let merge_point = self.find_merge_point(true_target, false_target, end)?;

        // Reconstruct true branch
        let true_ops = self.reconstruct_region(true_target, merge_point)?;

        // Reconstruct false branch
        let false_ops = self.reconstruct_region(false_target, merge_point)?;

        // Extract condition from last instruction of current block
        // In CFG construction, the condition is stored as the last instruction before branching
        // For now, we'll create a placeholder condition
        let condition = self.create_placeholder_condition()?;

        // Build if Op
        let if_op = self.build_if_op(condition, true_ops, false_ops)?;

        Ok(Some((if_op, merge_point)))
    }

    /// Reconstruct a while loop.
    fn reconstruct_while(&mut self, header: usize, _back_target: usize) -> PyResult<Option<Op>> {
        let header_block = self.cfg.blocks.get(&header).unwrap();

        // Extract condition from header block (last instruction before branching)
        let condition = self.extract_condition_from_header(header_block)?;

        // Find loop body (target of ConditionalTrue edge from header)
        let body_block = self.find_loop_body(header)?;

        // Find loop exit (target of ConditionalFalse edge from header)
        let loop_exit = self.find_loop_exit(header)?;

        // Reconstruct loop body
        let body_ops = if let Some(body) = body_block {
            self.reconstruct_region(body, header)?
        } else {
            Vec::new()
        };

        // Build while Op
        let while_op = self.build_while_op(condition, body_ops)?;

        // Mark loop exit as visited if it exists
        if let Some(exit) = loop_exit {
            self.visited.insert(exit);
        }

        Ok(Some(while_op))
    }

    /// Check if edge from source to target is a back-edge (loop).
    fn is_back_edge(&self, source: usize, target: usize) -> bool {
        // Back-edge: target dominates source
        if let Some(source_block) = self.cfg.blocks.get(&source) {
            source_block.dominators.contains(&target)
        } else {
            false
        }
    }

    /// Check if a block is a while loop header.
    /// A while header has 2 successors (true/false branches) and the true branch
    /// eventually loops back to the header.
    fn is_while_header(&self, block_id: usize) -> bool {
        let Some(block) = self.cfg.blocks.get(&block_id) else {
            return false;
        };

        // Must have exactly 2 successors
        if block.successors.len() != 2 {
            return false;
        }

        // Find the ConditionalTrue successor (loop body)
        for &edge_id in &block.successors {
            let edge = &self.cfg.edges[edge_id];
            if matches!(edge.edge_type, EdgeType::ConditionalTrue) {
                // Check if there's a path from the body back to this header
                return self.has_path_back_to(edge.target, block_id);
            }
        }

        false
    }

    /// Check if there's a path from 'from' block back to 'to' block.
    fn has_path_back_to(&self, from: usize, to: usize) -> bool {
        let mut visited = HashSet::new();
        let mut queue = vec![from];

        while let Some(current) = queue.pop() {
            if current == to {
                return true;
            }

            if !visited.insert(current) {
                continue;
            }

            if let Some(block) = self.cfg.blocks.get(&current) {
                for &edge_id in &block.successors {
                    let edge = &self.cfg.edges[edge_id];
                    queue.push(edge.target);
                }
            }
        }

        false
    }

    /// Extract condition from header block (last instruction before branching).
    fn extract_condition_from_header(
        &self,
        block: &crate::cfg::basic_block::BasicBlock,
    ) -> PyResult<Py<PyAny>> {
        if block.instructions.is_empty() {
            return self.create_placeholder_condition();
        }

        // Last instruction is typically the condition
        let last_op = &block.instructions[block.instructions.len() - 1];

        // Convert Op to Py<PyAny>
        let py_op: Py<PyAny> = Py::new(self.py, last_op.clone())?.into();
        Ok(py_op)
    }

    /// Find loop body block (target of ConditionalTrue edge from header).
    fn find_loop_body(&self, header: usize) -> PyResult<Option<usize>> {
        let block = self.cfg.blocks.get(&header).unwrap();

        for &edge_id in &block.successors {
            let edge = &self.cfg.edges[edge_id];
            if matches!(edge.edge_type, EdgeType::ConditionalTrue) {
                return Ok(Some(edge.target));
            }
        }

        Ok(None)
    }

    /// Find loop exit block (target of ConditionalFalse edge from header).
    fn find_loop_exit(&self, header: usize) -> PyResult<Option<usize>> {
        let block = self.cfg.blocks.get(&header).unwrap();

        for &edge_id in &block.successors {
            let edge = &self.cfg.edges[edge_id];
            if matches!(edge.edge_type, EdgeType::ConditionalFalse) {
                return Ok(Some(edge.target));
            }
        }

        Ok(None)
    }

    /// Build a while Op from condition and body operations.
    fn build_while_op(&self, condition: Py<PyAny>, body_ops: Vec<Op>) -> PyResult<Op> {
        // Build body block
        let body_block = self.build_block_op(body_ops)?;
        let body_py: Py<PyAny> = Py::new(self.py, body_block)?.into();

        // Create args tuple: (condition, body)
        let args_tuple = PyTuple::new(
            self.py,
            vec![condition.bind(self.py), body_py.bind(self.py)],
        )?;

        // Create while Op via Python API
        let op_class = self.py.import("catnip._rs")?.getattr("Op")?;
        let ident = IROpCode::OpWhile as i32;
        let kwargs = PyDict::new(self.py);

        let py_op = op_class.call((ident, args_tuple, kwargs), None)?;
        let op: Op = py_op.extract()?;

        Ok(op)
    }

    /// Find the merge point where two branches converge.
    fn find_merge_point(
        &self,
        branch1: usize,
        branch2: usize,
        default_end: usize,
    ) -> PyResult<usize> {
        // Simple heuristic: find the first common successor
        let successors1 = self.get_all_successors(branch1);
        let successors2 = self.get_all_successors(branch2);

        // Find intersection
        for succ in &successors1 {
            if successors2.contains(succ) {
                return Ok(*succ);
            }
        }

        // No merge point found, use default end
        Ok(default_end)
    }

    /// Get all successor blocks reachable from a block.
    fn get_all_successors(&self, start: usize) -> HashSet<usize> {
        let mut successors = HashSet::new();
        let mut queue = vec![start];

        while let Some(block_id) = queue.pop() {
            if !successors.insert(block_id) {
                continue; // Already visited
            }

            if let Some(block) = self.cfg.blocks.get(&block_id) {
                for &edge_id in &block.successors {
                    if let Some(edge) = self.cfg.edges.get(edge_id) {
                        queue.push(edge.target);
                    }
                }
            }
        }

        successors
    }

    /// Create a placeholder condition (True for now).
    fn create_placeholder_condition(&self) -> PyResult<Py<PyAny>> {
        // Get Python True singleton
        let builtins = self.py.import("builtins")?;
        let true_val = builtins.getattr("True")?;
        Ok(true_val.unbind())
    }

    /// Build an if Op from condition and branches.
    fn build_if_op(
        &self,
        condition: Py<PyAny>,
        true_ops: Vec<Op>,
        false_ops: Vec<Op>,
    ) -> PyResult<Op> {
        // Build branch: (condition, block)
        let true_block = self.build_block_op(true_ops)?;
        let true_block_py: Py<PyAny> = Py::new(self.py, true_block)?.into();
        let branch_tuple = PyTuple::new(self.py, vec![condition, true_block_py])?;

        // Build branches list
        let branches = PyList::new(self.py, vec![branch_tuple])?;

        // Build args based on whether there's an else block
        let args_tuple = if !false_ops.is_empty() {
            let else_op = self.build_block_op(false_ops)?;
            let else_py: Py<PyAny> = Py::new(self.py, else_op)?.into();
            PyTuple::new(
                self.py,
                vec![&branches as &Bound<'_, PyAny>, else_py.bind(self.py)],
            )?
        } else {
            PyTuple::new(self.py, vec![&branches as &Bound<'_, PyAny>])?
        };

        // Create if Op via Python API
        let op_class = self.py.import("catnip._rs")?.getattr("Op")?;
        let ident = IROpCode::OpIf as i32;
        let kwargs = PyDict::new(self.py);

        let py_op = op_class.call((ident, args_tuple, kwargs), None)?;
        let op: Op = py_op.extract()?;

        Ok(op)
    }

    /// Build a block Op from a list of Op nodes.
    fn build_block_op(&self, ops: Vec<Op>) -> PyResult<Op> {
        // Convert ops to Python objects
        let py_ops: Vec<Py<PyAny>> = ops
            .into_iter()
            .map(|op| -> PyResult<Py<PyAny>> { Ok(Py::new(self.py, op)?.into()) })
            .collect::<PyResult<_>>()?;

        // Bind all Py<PyAny> to Bound<PyAny>
        let bound_ops: Vec<&Bound<'_, PyAny>> =
            py_ops.iter().map(|py_obj| py_obj.bind(self.py)).collect();

        let args_tuple = PyTuple::new(self.py, bound_ops)?;

        // Create block Op via Python API
        let op_class = self.py.import("catnip._rs")?.getattr("Op")?;
        let ident = IROpCode::OpBlock as i32;
        let kwargs = PyDict::new(self.py);

        let py_op = op_class.call((ident, args_tuple, kwargs), None)?;
        let op: Op = py_op.extract()?;

        Ok(op)
    }
}

/// Reconstruct Op nodes from CFG.
pub fn reconstruct_from_cfg(py: Python<'_>, cfg: &ControlFlowGraph) -> PyResult<Vec<Op>> {
    // Ensure dominators are computed
    // Note: CFG should already have dominators computed by optimization pass

    let mut reconstructor = CFGReconstructor::new(py, cfg);
    reconstructor.reconstruct()
}
