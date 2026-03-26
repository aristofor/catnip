// FILE: catnip_rs/src/cfg/basic_block.rs
//! Basic block representation for CFG.

use crate::core::op::Op;
use std::collections::HashSet;
use std::fmt;

/// Basic block in a control flow graph.
///
/// A basic block is a maximal sequence of instructions with:
/// - Single entry point (first instruction)
/// - Single exit point (last instruction)
/// - No internal control flow (no branches except at the end)
#[derive(Debug, Clone)]
pub struct BasicBlock {
    /// Unique identifier for this block
    pub id: usize,
    /// Human-readable label (e.g., "loop_header", "if_true")
    pub label: String,
    /// Instructions in this block (Op nodes)
    pub instructions: Vec<Op>,
    /// Incoming edge indices (into CFG.edges)
    pub predecessors: Vec<usize>,
    /// Outgoing edge indices (into CFG.edges)
    pub successors: Vec<usize>,
    /// Source line range (optional)
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    /// Dominators (block IDs that dominate this block)
    pub dominators: HashSet<usize>,
    /// Immediate dominator (None for entry block)
    pub immediate_dominator: Option<usize>,
    /// Blocks dominated by this block
    pub dominated: HashSet<usize>,
    /// Branch condition (for blocks ending with conditional edges)
    pub condition: Option<Op>,
}

impl BasicBlock {
    pub fn new(id: usize, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            instructions: Vec::new(),
            predecessors: Vec::new(),
            successors: Vec::new(),
            start_line: None,
            end_line: None,
            dominators: HashSet::new(),
            immediate_dominator: None,
            dominated: HashSet::new(),
            condition: None,
        }
    }

    pub fn add_instruction(&mut self, op: Op) {
        self.instructions.push(op);
    }

    pub fn set_condition(&mut self, cond: Op) {
        self.condition = Some(cond);
    }

    pub fn is_entry(&self) -> bool {
        self.predecessors.is_empty()
    }

    pub fn is_exit(&self) -> bool {
        self.successors.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    pub fn terminator(&self) -> Option<&Op> {
        self.instructions.last()
    }
}

impl fmt::Display for BasicBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}:", self.label)?;
        for (i, instr) in self.instructions.iter().enumerate() {
            writeln!(f, "  {}: {:?}", i, instr)?;
        }
        Ok(())
    }
}
