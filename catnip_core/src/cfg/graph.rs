// FILE: catnip_core/src/cfg/graph.rs
//! Control flow graph representation.

use super::basic_block::BasicBlock;
use super::edge::{CFGEdge, EdgeType};
use crate::ir::IR;
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

    /// Linearize CFG back to a list of IR nodes.
    ///
    /// Performs a topological sort from entry block and emits instructions
    /// from reachable blocks in execution order. Unreachable blocks are omitted.
    pub fn to_ops(&self) -> Vec<IR> {
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

    /// Verify structural invariants that must hold whenever the CFG is in a
    /// settled state (after construction, after a complete optimization pass).
    ///
    /// Checks: entry/exit point to existing blocks, every edge references
    /// existing blocks, and predecessor/successor edge lists are bidirectionally
    /// consistent with the edges they index. Does **not** check reachability
    /// (see [`verify_all_reachable`](Self::verify_all_reachable)) since some
    /// passes leave stale edges until `remove_unreachable_blocks` runs.
    pub fn verify(&self) -> Result<(), String> {
        let entry = self
            .entry
            .ok_or_else(|| format!("CFG '{}': no entry block", self.name))?;
        if !self.blocks.contains_key(&entry) {
            return Err(format!("CFG '{}': entry block {} does not exist", self.name, entry));
        }
        if let Some(exit) = self.exit {
            if !self.blocks.contains_key(&exit) {
                return Err(format!("CFG '{}': exit block {} does not exist", self.name, exit));
            }
        }

        // Every edge must reference existing blocks.
        for (i, e) in self.edges.iter().enumerate() {
            if !self.blocks.contains_key(&e.source) {
                return Err(format!(
                    "CFG '{}': edge {} source {} does not exist",
                    self.name, i, e.source
                ));
            }
            if !self.blocks.contains_key(&e.target) {
                return Err(format!(
                    "CFG '{}': edge {} target {} does not exist",
                    self.name, i, e.target
                ));
            }
        }

        // Successor/predecessor lists index edges that actually leave/enter the block.
        for (&bid, block) in &self.blocks {
            for &eid in &block.successors {
                let e = self
                    .edges
                    .get(eid)
                    .ok_or_else(|| format!("CFG '{}': block {} successor edge {} out of range", self.name, bid, eid))?;
                if e.source != bid {
                    return Err(format!(
                        "CFG '{}': block {} lists successor edge {} but edge.source = {}",
                        self.name, bid, eid, e.source
                    ));
                }
            }
            for &eid in &block.predecessors {
                let e = self.edges.get(eid).ok_or_else(|| {
                    format!(
                        "CFG '{}': block {} predecessor edge {} out of range",
                        self.name, bid, eid
                    )
                })?;
                if e.target != bid {
                    return Err(format!(
                        "CFG '{}': block {} lists predecessor edge {} but edge.target = {}",
                        self.name, bid, eid, e.target
                    ));
                }
            }
        }

        Ok(())
    }

    /// Verify every block is reachable from the entry block.
    ///
    /// Separate from [`verify`](Self::verify): only meaningful once dead blocks
    /// have been pruned (`remove_unreachable_blocks`).
    pub fn verify_all_reachable(&self) -> Result<(), String> {
        let reachable = self.get_reachable_blocks();
        for &bid in self.blocks.keys() {
            if !reachable.contains(&bid) {
                return Err(format!("CFG '{}': block {} unreachable from entry", self.name, bid));
            }
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_cfg() -> ControlFlowGraph {
        let mut cfg = ControlFlowGraph::new("verify");
        let a = cfg.create_block("a");
        let b = cfg.create_block("b");
        cfg.set_entry(a);
        cfg.set_exit(b);
        cfg.add_edge(a, b, EdgeType::Fallthrough);
        cfg
    }

    #[test]
    fn test_verify_well_formed() {
        let cfg = linear_cfg();
        assert!(cfg.verify().is_ok());
        assert!(cfg.verify_all_reachable().is_ok());
    }

    #[test]
    fn test_verify_no_entry() {
        let mut cfg = ControlFlowGraph::new("verify");
        cfg.create_block("a");
        assert!(cfg.verify().is_err());
    }

    #[test]
    fn test_verify_dangling_successor() {
        let mut cfg = linear_cfg();
        // Inject an out-of-range successor edge index into the entry block.
        let entry = cfg.entry.unwrap();
        cfg.get_block_mut(entry).unwrap().successors.push(999);
        assert!(cfg.verify().is_err());
    }

    #[test]
    fn test_verify_incoherent_predecessor() {
        let mut cfg = linear_cfg();
        // Edge 0 targets b; claim it as a predecessor of a (wrong direction).
        let entry = cfg.entry.unwrap();
        cfg.get_block_mut(entry).unwrap().predecessors.push(0);
        assert!(cfg.verify().is_err());
    }

    #[test]
    fn test_verify_unreachable_block() {
        let mut cfg = linear_cfg();
        // Orphan block: structurally fine (verify ok) but unreachable.
        cfg.create_block("orphan");
        assert!(cfg.verify().is_ok());
        assert!(cfg.verify_all_reachable().is_err());
    }
}
