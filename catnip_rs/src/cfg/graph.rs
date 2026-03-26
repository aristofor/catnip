// FILE: catnip_rs/src/cfg/graph.rs
//! Control flow graph representation.

use super::basic_block::BasicBlock;
use super::edge::{CFGEdge, EdgeType};
use pyo3::prelude::*;
use pyo3::types::PyList;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

/// Control flow graph for a function or module.
#[derive(Debug, Clone)]
pub struct ControlFlowGraph {
    /// Name of the function or module
    pub name: String,
    /// All basic blocks indexed by id
    pub blocks: HashMap<usize, BasicBlock>,
    /// All edges
    pub edges: Vec<CFGEdge>,
    /// Entry block ID
    pub entry: Option<usize>,
    /// Exit block ID
    pub exit: Option<usize>,
    /// Next block ID
    next_block_id: usize,
}

impl ControlFlowGraph {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            blocks: HashMap::new(),
            edges: Vec::new(),
            entry: None,
            exit: None,
            next_block_id: 0,
        }
    }

    /// Create a new basic block and add it to the graph.
    pub fn create_block(&mut self, label: impl Into<String>) -> usize {
        let id = self.next_block_id;
        self.next_block_id += 1;
        let block = BasicBlock::new(id, label);
        self.blocks.insert(id, block);
        id
    }

    /// Add a control flow edge between two blocks.
    pub fn add_edge(&mut self, source: usize, target: usize, edge_type: EdgeType) -> usize {
        let edge_id = self.edges.len();
        let edge = CFGEdge::new(source, target, edge_type);
        self.edges.push(edge);

        // Update block edges
        if let Some(block) = self.blocks.get_mut(&source) {
            block.successors.push(edge_id);
        }
        if let Some(block) = self.blocks.get_mut(&target) {
            block.predecessors.push(edge_id);
        }

        edge_id
    }

    /// Add edge with label.
    pub fn add_edge_with_label(
        &mut self,
        source: usize,
        target: usize,
        edge_type: EdgeType,
        label: impl Into<String>,
    ) -> usize {
        let edge_id = self.edges.len();
        let edge = CFGEdge::new(source, target, edge_type).with_label(label);
        self.edges.push(edge);

        if let Some(block) = self.blocks.get_mut(&source) {
            block.successors.push(edge_id);
        }
        if let Some(block) = self.blocks.get_mut(&target) {
            block.predecessors.push(edge_id);
        }

        edge_id
    }

    pub fn set_entry(&mut self, block_id: usize) {
        self.entry = Some(block_id);
    }

    pub fn set_exit(&mut self, block_id: usize) {
        self.exit = Some(block_id);
    }

    pub fn get_block(&self, id: usize) -> Option<&BasicBlock> {
        self.blocks.get(&id)
    }

    pub fn get_block_mut(&mut self, id: usize) -> Option<&mut BasicBlock> {
        self.blocks.get_mut(&id)
    }

    /// Get all blocks reachable from entry via BFS.
    pub fn get_reachable_blocks(&self) -> HashSet<usize> {
        let Some(entry_id) = self.entry else {
            return HashSet::new();
        };

        let mut reachable = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(entry_id);
        reachable.insert(entry_id);

        while let Some(block_id) = queue.pop_front() {
            if let Some(block) = self.blocks.get(&block_id) {
                for &edge_id in &block.successors {
                    if let Some(edge) = self.edges.get(edge_id) {
                        if reachable.insert(edge.target) {
                            queue.push_back(edge.target);
                        }
                    }
                }
            }
        }

        reachable
    }

    /// Get all unreachable blocks.
    pub fn get_unreachable_blocks(&self) -> HashSet<usize> {
        let reachable = self.get_reachable_blocks();
        self.blocks
            .keys()
            .filter(|&id| !reachable.contains(id))
            .copied()
            .collect()
    }

    /// Remove unreachable blocks from the graph.
    pub fn remove_unreachable_blocks(&mut self) {
        let unreachable = self.get_unreachable_blocks();

        // Remove blocks
        for id in &unreachable {
            self.blocks.remove(id);
        }

        // Remove edges involving unreachable blocks
        self.edges
            .retain(|e| !unreachable.contains(&e.source) && !unreachable.contains(&e.target));

        // Rebuild edge indices in blocks
        for block in self.blocks.values_mut() {
            block.predecessors.clear();
            block.successors.clear();
        }

        for (edge_id, edge) in self.edges.iter().enumerate() {
            if let Some(block) = self.blocks.get_mut(&edge.source) {
                block.successors.push(edge_id);
            }
            if let Some(block) = self.blocks.get_mut(&edge.target) {
                block.predecessors.push(edge_id);
            }
        }
    }

    /// Linearize CFG back to a list of Op nodes.
    ///
    /// Performs a topological sort from entry block and emits instructions
    /// from reachable blocks in execution order. Unreachable blocks are omitted.
    pub fn to_ops(&self) -> Vec<crate::core::op::Op> {
        let Some(entry_id) = self.entry else {
            return Vec::new();
        };

        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![entry_id];

        // DFS from entry, collecting instructions
        while let Some(block_id) = stack.pop() {
            // Skip exit block (no instructions) and already visited
            if Some(block_id) == self.exit || !visited.insert(block_id) {
                continue;
            }

            // Add block instructions
            if let Some(block) = self.blocks.get(&block_id) {
                result.extend(block.instructions.clone());

                // Add successors to stack (reversed for DFS order)
                for &edge_id in block.successors.iter().rev() {
                    if let Some(edge) = self.edges.get(edge_id) {
                        if !visited.contains(&edge.target) {
                            stack.push(edge.target);
                        }
                    }
                }
            }
        }

        result
    }

    /// Split an edge by inserting a new block between source and target.
    ///
    /// Returns the ID of the new block. Useful for creating preheaders
    /// (split the edge entering a loop header) without disturbing other edges.
    pub fn split_edge(&mut self, edge_idx: usize, label: impl Into<String>) -> Option<usize> {
        let edge = self.edges.get(edge_idx)?.clone();
        let new_block = self.create_block(label);

        // Redirect original edge: source -> new_block
        if let Some(e) = self.edges.get_mut(edge_idx) {
            e.target = new_block;
        }

        // Update new_block predecessors/successors
        if let Some(b) = self.blocks.get_mut(&new_block) {
            b.predecessors.push(edge_idx);
        }

        // Add new edge: new_block -> original target
        let new_edge_idx = self.add_edge(new_block, edge.target, EdgeType::Fallthrough);

        // Update original target: replace old edge_idx in predecessors with new_edge_idx
        if let Some(target_block) = self.blocks.get_mut(&edge.target) {
            target_block.predecessors.retain(|&e| e != edge_idx);
            if !target_block.predecessors.contains(&new_edge_idx) {
                target_block.predecessors.push(new_edge_idx);
            }
        }

        // Update source block: edge_idx is already in its successors, and it now
        // points to new_block (we modified edges[edge_idx].target above)

        Some(new_block)
    }

    /// Export CFG to DOT format (Graphviz).
    pub fn to_dot(&self) -> String {
        let mut lines = vec![format!("digraph \"{}\" {{", self.name)];
        lines.push("  node [shape=box, fontname=\"monospace\"];".to_string());
        lines.push("  rankdir=TB;".to_string());
        lines.push(String::new());

        // Nodes
        for (&id, block) in &self.blocks {
            let mut label_lines = vec![block.label.clone()];

            // Add up to 5 instructions
            for (i, instr) in block.instructions.iter().take(5).enumerate() {
                let instr_str = format!("{:?}", instr);
                let instr_str = if instr_str.len() > 40 {
                    format!("{}...", &instr_str[..37])
                } else {
                    instr_str
                };
                label_lines.push(format!("{}: {}", i, instr_str.replace('"', "\\\"")));
            }

            if block.instructions.len() > 5 {
                label_lines.push(format!("... ({} more)", block.instructions.len() - 5));
            }

            let label = label_lines.join("\\l") + "\\l";

            // Special styling
            let style = if Some(id) == self.entry {
                ", style=filled, fillcolor=lightgreen"
            } else if Some(id) == self.exit {
                ", style=filled, fillcolor=lightcoral"
            } else {
                ""
            };

            lines.push(format!("  {} [label=\"{}\"{}];", id, label, style));
        }

        lines.push(String::new());

        // Edges
        for edge in &self.edges {
            let edge_label = if let Some(label) = &edge.label {
                format!("{}\\n{}", edge.edge_type, label)
            } else {
                edge.edge_type.to_string()
            };

            let style = match edge.edge_type {
                EdgeType::ConditionalTrue => ", color=green",
                EdgeType::ConditionalFalse => ", color=red",
                EdgeType::Break | EdgeType::Continue => ", style=dashed",
                _ => "",
            };

            lines.push(format!(
                "  {} -> {} [label=\"{}\"{}];",
                edge.source, edge.target, edge_label, style
            ));
        }

        lines.push("}".to_string());
        lines.join("\n")
    }
}

impl fmt::Display for ControlFlowGraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "CFG: {}", self.name)?;
        writeln!(
            f,
            "Entry: {:?}",
            self.entry.map(|id| self.blocks.get(&id).map(|b| &b.label))
        )?;
        writeln!(
            f,
            "Exit: {:?}",
            self.exit.map(|id| self.blocks.get(&id).map(|b| &b.label))
        )?;
        writeln!(f, "Blocks: {}", self.blocks.len())?;
        writeln!(f)?;

        // Print blocks in order
        let mut block_ids: Vec<_> = self.blocks.keys().copied().collect();
        block_ids.sort();

        for id in block_ids {
            if let Some(block) = self.blocks.get(&id) {
                write!(f, "{}", block)?;

                // Print successors
                for &edge_id in &block.successors {
                    if let Some(edge) = self.edges.get(edge_id) {
                        if let Some(target) = self.blocks.get(&edge.target) {
                            writeln!(f, "  -> {} ({})", target.label, edge.edge_type)?;
                        }
                    }
                }
                writeln!(f)?;
            }
        }

        Ok(())
    }
}

/// Python wrapper for ControlFlowGraph.
#[pyclass(name = "ControlFlowGraph", module = "catnip._rs.cfg")]
pub struct PyControlFlowGraph {
    pub inner: ControlFlowGraph,
}

#[pymethods]
impl PyControlFlowGraph {
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[getter]
    fn num_blocks(&self) -> usize {
        self.inner.blocks.len()
    }

    #[getter]
    fn num_edges(&self) -> usize {
        self.inner.edges.len()
    }

    fn get_reachable_blocks(&self) -> Vec<usize> {
        self.inner.get_reachable_blocks().into_iter().collect()
    }

    fn get_unreachable_blocks(&self) -> Vec<usize> {
        self.inner.get_unreachable_blocks().into_iter().collect()
    }

    fn remove_unreachable_blocks(&mut self) {
        self.inner.remove_unreachable_blocks();
    }

    fn to_dot(&self) -> String {
        self.inner.to_dot()
    }

    fn visualize(&self, output_file: &str) -> PyResult<()> {
        use std::fs::File;
        use std::io::Write;

        let dot = self.inner.to_dot();
        let mut file = File::create(output_file)?;
        file.write_all(dot.as_bytes())?;

        println!("CFG exported to {}", output_file);
        println!(
            "Visualize with: dot -Tpng {} -o {}",
            output_file,
            output_file.replace(".dot", ".png")
        );

        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "<CFG {} blocks={} edges={}>",
            self.inner.name,
            self.inner.blocks.len(),
            self.inner.edges.len()
        )
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    /// Compute dominance information for all blocks.
    fn compute_dominators(&mut self) {
        super::analysis::compute_dominators(&mut self.inner);
    }

    /// Detect natural loops in the CFG.
    /// Returns list of (header_id, loop_blocks) tuples.
    fn detect_loops(&self) -> Vec<(usize, Vec<usize>)> {
        super::analysis::detect_loops(&self.inner)
            .into_iter()
            .map(|(header, blocks)| (header, blocks.into_iter().collect()))
            .collect()
    }

    /// Get dominator set for a block.
    fn get_dominators(&self, block_id: usize) -> Vec<usize> {
        self.inner
            .blocks
            .get(&block_id)
            .map(|b| b.dominators.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Get immediate dominator for a block.
    fn get_immediate_dominator(&self, block_id: usize) -> Option<usize> {
        self.inner.blocks.get(&block_id).and_then(|b| b.immediate_dominator)
    }

    /// Get blocks dominated by a given block.
    fn get_dominated(&self, block_id: usize) -> Vec<usize> {
        self.inner
            .blocks
            .get(&block_id)
            .map(|b| b.dominated.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Linearize CFG back to a list of Op nodes.
    ///
    /// Performs topological sort from entry and emits instructions from
    /// reachable blocks. Unreachable blocks are omitted (dead code eliminated).
    fn to_ops(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let ops = self.inner.to_ops();
        let py_list = PyList::empty(py);
        for op in ops {
            let py_op: Py<PyAny> = Py::new(py, op)?.into();
            py_list.append(py_op)?;
        }
        Ok(py_list.unbind())
    }

    /// Eliminate dead code (unreachable blocks).
    /// Returns number of blocks removed.
    fn eliminate_dead_code(&mut self) -> usize {
        super::optimization::eliminate_dead_code(&mut self.inner)
    }

    /// Merge sequential blocks that can be combined.
    /// Returns number of merges performed.
    fn merge_blocks(&mut self) -> usize {
        super::optimization::merge_blocks(&mut self.inner)
    }

    /// Remove empty blocks (no instructions).
    /// Returns number of blocks removed.
    fn remove_empty_blocks(&mut self) -> usize {
        super::optimization::remove_empty_blocks(&mut self.inner)
    }

    /// Eliminate constant branches (both branches to same target).
    /// Returns number of branches eliminated.
    fn eliminate_constant_branches(&mut self) -> usize {
        super::optimization::eliminate_constant_branches(&mut self.inner)
    }

    /// Apply all CFG optimizations.
    /// Returns tuple: (dead_blocks, merged, empty_removed, branches_eliminated)
    fn optimize(&mut self) -> (usize, usize, usize, usize) {
        super::optimization::optimize_cfg(&mut self.inner)
    }
}
