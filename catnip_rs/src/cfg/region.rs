// FILE: catnip_rs/src/cfg/region.rs
//! Region detection for CFG reconstruction.
//!
//! Identifies control flow regions (if/elif/else, while, for, match) in a CFG
//! and reconstructs structured code.

use super::edge::EdgeType;
use super::graph::ControlFlowGraph;
use crate::constants::*;
use crate::core::op::Op;
use crate::ir::opcode::IROpCode;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::collections::HashSet;

/// Region-based CFG reconstructor.
pub struct RegionReconstructor<'py> {
    py: Python<'py>,
    cfg: &'py ControlFlowGraph,
    visited: HashSet<usize>,
}

impl<'py> RegionReconstructor<'py> {
    /// Filter out Nop instructions (dead code markers from DSE/CSE/LICM/IV).
    fn filter_nops(instructions: &[Op]) -> Vec<Op> {
        instructions
            .iter()
            .filter(|op| op.ident != IROpCode::Nop as i32)
            .cloned()
            .collect()
    }

    pub fn new(py: Python<'py>, cfg: &'py ControlFlowGraph) -> Self {
        Self {
            py,
            cfg,
            visited: HashSet::new(),
        }
    }

    /// Reconstruct Op nodes from CFG.
    pub fn reconstruct(&mut self) -> PyResult<Vec<Op>> {
        let Some(entry_id) = self.cfg.entry else {
            return Ok(Vec::new());
        };

        let Some(exit_id) = self.cfg.exit else {
            return Ok(Vec::new());
        };

        self.reconstruct_from_block(entry_id, exit_id)
    }

    /// Reconstruct code from a block until we reach the exit.
    fn reconstruct_from_block(&mut self, start: usize, end: usize) -> PyResult<Vec<Op>> {
        let mut result = Vec::new();
        let mut current = start;

        while current != end && !self.visited.contains(&current) {
            self.visited.insert(current);

            let Some(block) = self.cfg.blocks.get(&current) else {
                break;
            };

            // Analyze block successors to determine structure
            match block.successors.len() {
                0 => {
                    // No successors - end of region
                    result.extend(Self::filter_nops(&block.instructions));
                    break;
                }

                1 => {
                    // Single successor
                    let edge_id = block.successors[0];
                    let edge = &self.cfg.edges[edge_id];
                    let target = edge.target;

                    // Check if this is a loop back-edge
                    if self.is_back_edge(current, target) {
                        // This is a while loop - reconstruct it
                        if let Some(while_op) = self.reconstruct_while(current, target)? {
                            result.push(while_op);
                        }
                        break;
                    } else {
                        // Linear flow - add instructions and continue
                        result.extend(Self::filter_nops(&block.instructions));

                        // Check if target is the exit block - if so, add its instructions too
                        if target == end {
                            if let Some(exit_block) = self.cfg.blocks.get(&target) {
                                result.extend(Self::filter_nops(&exit_block.instructions));
                            }
                            break;
                        }

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
                        // Use preserved condition from CFG construction
                        let condition = block.condition.clone();

                        // Add block instructions (filter Nops)
                        result.extend(
                            block
                                .instructions
                                .iter()
                                .filter(|op| op.ident != IROpCode::Nop as i32)
                                .cloned(),
                        );

                        // Reconstruct if statement
                        if let Some((if_op, next_block)) =
                            self.reconstruct_if_with_condition(current, end, condition)?
                        {
                            result.push(if_op);
                            current = next_block;
                        } else {
                            break;
                        }
                    }
                }

                _ => {
                    // Multiple branches (3+) - likely match statement
                    // For now, just add instructions and break
                    result.extend(Self::filter_nops(&block.instructions));
                    break;
                }
            }
        }

        Ok(result)
    }

    /// Reconstruct an if/elif/else structure with extracted condition.
    fn reconstruct_if_with_condition(
        &mut self,
        block_id: usize,
        end: usize,
        condition: Option<Op>,
    ) -> PyResult<Option<(Op, usize)>> {
        let block = self.cfg.blocks.get(&block_id).unwrap();

        if block.successors.len() != 2 {
            return Ok(None);
        }

        let edge1_id = block.successors[0];
        let edge2_id = block.successors[1];

        let edge1 = &self.cfg.edges[edge1_id];
        let edge2 = &self.cfg.edges[edge2_id];

        // Identify true and false branches
        let (true_target, false_target) = match (&edge1.edge_type, &edge2.edge_type) {
            (EdgeType::ConditionalTrue, EdgeType::ConditionalFalse) => (edge1.target, edge2.target),
            (EdgeType::ConditionalFalse, EdgeType::ConditionalTrue) => (edge2.target, edge1.target),
            _ => return Ok(None),
        };

        // Find merge point
        let merge_point = self.find_merge_point(true_target, false_target, end)?;

        // Reconstruct true branch
        let true_ops = self.reconstruct_from_block(true_target, merge_point)?;

        // Reconstruct false branch (might be another if for elif)
        let false_ops = self.reconstruct_from_block(false_target, merge_point)?;

        // Check if false branch is a single if (for elif detection)
        let branches = if let Some(cond) = condition {
            vec![(cond, true_ops)]
        } else {
            // No condition extracted - create placeholder
            vec![(self.create_placeholder_condition()?, true_ops)]
        };

        // Build if Op
        let if_op = self.build_if_op_with_branches(branches, false_ops)?;

        Ok(Some((if_op, merge_point)))
    }

    /// Reconstruct a while loop.
    fn reconstruct_while(&mut self, header: usize, _back_target: usize) -> PyResult<Option<Op>> {
        let header_block = self.cfg.blocks.get(&header).unwrap();

        // Find loop exit (not used yet, but might be needed for more complex loops)
        let _loop_exit = self.find_loop_exit(header)?;

        // Use preserved condition from CFG construction
        let condition = header_block.condition.clone();

        // Find loop body (the target of the ConditionalTrue edge from header)
        let body_block = self.find_loop_body(header)?;

        // Reconstruct loop body
        let body_ops = if let Some(body) = body_block {
            self.reconstruct_from_block(body, header)?
        } else {
            Vec::new()
        };

        // Build while Op (fall back to True only if condition truly missing)
        let condition_op = condition.unwrap_or_else(|| self.create_placeholder_condition().unwrap());

        let while_op = self.build_while_op(condition_op, body_ops)?;

        Ok(Some(while_op))
    }

    /// Find the loop body block (target of ConditionalTrue from header).
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

    /// Find the loop exit block (target of ConditionalFalse from header).
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

    /// Check if edge from source to target is a back-edge.
    fn is_back_edge(&self, source: usize, target: usize) -> bool {
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

    /// Find merge point where two branches converge.
    fn find_merge_point(&self, branch1: usize, branch2: usize, default_end: usize) -> PyResult<usize> {
        // Use post-dominance: find the first block that post-dominates both branches
        let successors1 = self.get_all_successors(branch1);
        let successors2 = self.get_all_successors(branch2);

        // Find first common successor
        for succ in &successors1 {
            if successors2.contains(succ) {
                return Ok(*succ);
            }
        }

        Ok(default_end)
    }

    /// Get all successors reachable from a block.
    fn get_all_successors(&self, start: usize) -> Vec<usize> {
        let mut successors = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = vec![start];

        while let Some(block_id) = queue.pop() {
            if !visited.insert(block_id) {
                continue;
            }

            successors.push(block_id);

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

    /// Create a placeholder condition (True).
    fn create_placeholder_condition(&self) -> PyResult<Op> {
        // Create a simple Op that represents True
        // We'll use a dummy opcode and let the semantic analyzer handle it
        // For now, create an Op with Nop and True as an arg
        let builtins = self.py.import("builtins")?;
        let true_val = builtins.getattr("True")?;

        let op_class = self.py.import(PY_MOD_RS)?.getattr("Op")?;
        let ident = IROpCode::Nop as i32;
        let args = PyTuple::new(self.py, vec![true_val])?;
        let kwargs = PyDict::new(self.py);

        let py_op = op_class.call((ident, args, kwargs), None)?;
        let op_ref: PyRef<Op> = py_op.extract()?;
        Ok(op_ref.clone())
    }

    /// Build an if Op from branches and else block.
    fn build_if_op_with_branches(&self, branches: Vec<(Op, Vec<Op>)>, else_ops: Vec<Op>) -> PyResult<Op> {
        // Build branches list: [(condition, block), ...]
        let py_branches = PyList::empty(self.py);

        for (condition, body_ops) in branches {
            let body_block = self.build_block_op(body_ops)?;
            let condition_py: Py<PyAny> = Py::new(self.py, condition)?.into();
            let body_py: Py<PyAny> = Py::new(self.py, body_block)?.into();

            let branch_tuple = PyTuple::new(self.py, vec![condition_py.bind(self.py), body_py.bind(self.py)])?;
            py_branches.append(branch_tuple)?;
        }

        // Build args
        let args_tuple = if !else_ops.is_empty() {
            let else_block = self.build_block_op(else_ops)?;
            let else_py: Py<PyAny> = Py::new(self.py, else_block)?.into();
            PyTuple::new(self.py, vec![&py_branches as &Bound<'_, PyAny>, else_py.bind(self.py)])?
        } else {
            PyTuple::new(self.py, vec![&py_branches as &Bound<'_, PyAny>])?
        };

        // Create if Op
        let op_class = self.py.import(PY_MOD_RS)?.getattr("Op")?;
        let ident = IROpCode::OpIf as i32;
        let kwargs = PyDict::new(self.py);

        let py_op = op_class.call((ident, args_tuple, kwargs), None)?;
        let op_ref: PyRef<Op> = py_op.extract()?;
        Ok(op_ref.clone())
    }

    /// Build a while Op.
    fn build_while_op(&self, condition: Op, body_ops: Vec<Op>) -> PyResult<Op> {
        let body_block = self.build_block_op(body_ops)?;

        let condition_py: Py<PyAny> = Py::new(self.py, condition)?.into();
        let body_py: Py<PyAny> = Py::new(self.py, body_block)?.into();

        let args_tuple = PyTuple::new(self.py, vec![condition_py.bind(self.py), body_py.bind(self.py)])?;

        // Create while Op
        let op_class = self.py.import(PY_MOD_RS)?.getattr("Op")?;
        let ident = IROpCode::OpWhile as i32;
        let kwargs = PyDict::new(self.py);

        let py_op = op_class.call((ident, args_tuple, kwargs), None)?;
        let op_ref: PyRef<Op> = py_op.extract()?;
        Ok(op_ref.clone())
    }

    /// Build a block Op from a list of Op nodes.
    fn build_block_op(&self, ops: Vec<Op>) -> PyResult<Op> {
        let py_ops: Vec<Py<PyAny>> = ops
            .into_iter()
            .map(|op| -> PyResult<Py<PyAny>> { Ok(Py::new(self.py, op)?.into()) })
            .collect::<PyResult<_>>()?;

        let bound_ops: Vec<&Bound<'_, PyAny>> = py_ops.iter().map(|py_obj| py_obj.bind(self.py)).collect();

        let args_tuple = PyTuple::new(self.py, bound_ops)?;

        let op_class = self.py.import(PY_MOD_RS)?.getattr("Op")?;
        let ident = IROpCode::OpBlock as i32;
        let kwargs = PyDict::new(self.py);

        let py_op = op_class.call((ident, args_tuple, kwargs), None)?;
        let op_ref: PyRef<Op> = py_op.extract()?;
        Ok(op_ref.clone())
    }
}

/// Reconstruct Op nodes from CFG using region detection.
pub fn reconstruct_from_cfg(py: Python<'_>, cfg: &ControlFlowGraph) -> PyResult<Vec<Op>> {
    let mut reconstructor = RegionReconstructor::new(py, cfg);
    reconstructor.reconstruct()
}

/// Python wrapper for reconstruct_from_cfg.
#[pyfunction]
pub fn py_reconstruct_from_cfg(
    py: Python<'_>,
    cfg: &crate::cfg::graph::PyControlFlowGraph,
) -> PyResult<Py<pyo3::types::PyList>> {
    let ops = reconstruct_from_cfg(py, &cfg.inner)?;

    // Convert Vec<Op> to Python list
    let py_list = pyo3::types::PyList::empty(py);
    for op in ops {
        py_list.append(op)?;
    }

    Ok(py_list.unbind())
}
