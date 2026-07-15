// FILE: catnip_core/src/cfg/basic_block.rs
//! Basic block representation for CFG.

use crate::ir::IR;
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
    /// Instructions in this block (IR nodes)
    pub instructions: Vec<IR>,
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
    pub condition: Option<IR>,
    /// For a `match` header: the block where the arms reconverge (post-match
    /// code). Recorded by the builder because it cannot be inferred reliably
    /// from the graph — an arm ending in break/continue/return never reaches
    /// the merge, so a reachability search over the arms would miss it.
    pub match_merge: Option<usize>,
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
            match_merge: None,
        }
    }

    pub fn add_instruction(&mut self, op: IR) {
        self.instructions.push(op);
    }

    pub fn set_condition(&mut self, cond: IR) {
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

    pub fn terminator(&self) -> Option<&IR> {
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
