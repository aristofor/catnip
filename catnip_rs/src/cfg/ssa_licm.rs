// FILE: catnip_rs/src/cfg/ssa_licm.rs
//! Loop-Invariant Code Motion (LICM) on SSA form.
//!
//! For each natural loop, identifies instructions whose operands are all
//! defined outside the loop. These instructions compute the same value
//! on every iteration and can be hoisted to a preheader block.
//!
//! Requires:
//! - Dominators computed (for loop detection)
//! - SSA form (for reaching definitions)
//! - split_edge() on ControlFlowGraph (for preheader creation)

use super::graph::ControlFlowGraph;
use super::ssa::{SSAContext, SSAValue, ValueDef};
use super::ssa_cse::{extract_rhs_opcode, pure_opcodes};
use crate::cfg::analysis::detect_loops;
use std::collections::HashSet;

/// Result of LICM.
pub struct LICMResult {
    /// Number of instructions hoisted
    pub hoisted: usize,
}

/// Run LICM on the CFG in SSA form.
///
/// For each natural loop:
/// 1. Find the preheader (or create one via edge splitting)
/// 2. For each instruction in the loop body:
///    - If pure and all operands defined outside the loop, hoist to preheader
pub fn licm(cfg: &mut ControlFlowGraph, ssa: &SSAContext) -> LICMResult {
    let pure_ops = pure_opcodes();
    let loops = detect_loops(cfg);
    let mut total_hoisted = 0;

    for (header, loop_blocks) in &loops {
        // Find or create preheader
        let preheader = match find_or_create_preheader(cfg, *header, loop_blocks) {
            Some(ph) => ph,
            None => continue,
        };

        // Collect definitions inside the loop
        let loop_defs = collect_loop_defs(ssa, loop_blocks);

        // Find hoistable instructions
        let mut to_hoist: Vec<(usize, usize, crate::core::op::Op)> = Vec::new();

        for &block_id in loop_blocks {
            if block_id == *header {
                continue; // Don't hoist from header (contains loop condition)
            }

            let Some(block) = cfg.blocks.get(&block_id) else {
                continue;
            };

            for (instr_idx, op) in block.instructions.iter().enumerate() {
                // Only hoist SetLocals with pure RHS
                let Some(rhs_opcode) = extract_rhs_opcode(op) else {
                    continue;
                };
                if !pure_ops.contains(&rhs_opcode) {
                    continue;
                }

                // Check if all operands are defined outside the loop
                if is_loop_invariant(ssa, block_id, instr_idx, &loop_defs) {
                    to_hoist.push((block_id, instr_idx, op.clone()));
                }
            }
        }

        // Hoist instructions to preheader
        for (block_id, instr_idx, op) in to_hoist.iter().rev() {
            // Add to preheader (before its terminator)
            if let Some(ph_block) = cfg.get_block_mut(preheader) {
                let insert_pos = if ph_block.instructions.is_empty() {
                    0
                } else {
                    ph_block.instructions.len()
                };
                ph_block.instructions.insert(insert_pos, op.clone());
            }

            // Replace original with Nop
            if let Some(block) = cfg.get_block_mut(*block_id) {
                if *instr_idx < block.instructions.len() {
                    block.instructions[*instr_idx].ident = crate::ir::opcode::IROpCode::Nop as i32;
                }
            }

            total_hoisted += 1;
        }
    }

    LICMResult { hoisted: total_hoisted }
}

/// Find or create a preheader for a loop.
///
/// A preheader is the single block that enters the loop header from outside.
/// If the header has multiple outside predecessors, we split the edge to create one.
pub(crate) fn find_or_create_preheader(
    cfg: &mut ControlFlowGraph,
    header: usize,
    loop_blocks: &HashSet<usize>,
) -> Option<usize> {
    let header_block = cfg.blocks.get(&header)?;

    // Find predecessors outside the loop
    let outside_preds: Vec<usize> = header_block
        .predecessors
        .iter()
        .filter_map(|&edge_id| {
            let edge = cfg.edges.get(edge_id)?;
            if !loop_blocks.contains(&edge.source) {
                Some(edge_id)
            } else {
                None
            }
        })
        .collect();

    match outside_preds.len() {
        0 => None, // No outside predecessor (infinite loop?)
        1 => {
            // Single outside predecessor: check if it's a dedicated preheader
            let edge = cfg.edges.get(outside_preds[0])?;
            let pred_id = edge.source;
            let pred_block = cfg.blocks.get(&pred_id)?;

            if pred_block.successors.len() == 1 {
                // Already a dedicated preheader
                Some(pred_id)
            } else {
                // Predecessor has other successors: split the edge
                cfg.split_edge(outside_preds[0], "preheader")
            }
        }
        _ => {
            // Multiple outside predecessors: split the first one
            // (a complete implementation would merge them into a single preheader)
            cfg.split_edge(outside_preds[0], "preheader")
        }
    }
}

/// Collect all SSA values defined inside a loop.
fn collect_loop_defs(ssa: &SSAContext, loop_blocks: &HashSet<usize>) -> HashSet<SSAValue> {
    let mut defs = HashSet::new();

    for (value, def) in &ssa.value_defs {
        let block = match def {
            ValueDef::Instruction { block, .. } => *block,
            ValueDef::BlockParam { block, .. } => *block,
        };
        if loop_blocks.contains(&block) {
            defs.insert(*value);
        }
    }

    defs
}

/// Check if an instruction is loop-invariant.
///
/// An instruction is loop-invariant if none of its operands (from instruction_uses)
/// are defined inside the loop.
fn is_loop_invariant(ssa: &SSAContext, block_id: usize, instr_idx: usize, loop_defs: &HashSet<SSAValue>) -> bool {
    let uses = ssa.get_uses(block_id, instr_idx);
    if uses.is_empty() {
        return false; // No tracked uses → conservative: don't hoist
    }
    uses.iter().all(|val| !loop_defs.contains(val))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::analysis::compute_dominators;
    use crate::cfg::edge::EdgeType;
    use crate::cfg::ssa_builder::SSABuilder;

    #[test]
    fn test_licm_no_loops() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let ssa = SSABuilder::build(&cfg);

        let result = licm(&mut cfg, &ssa);
        assert_eq!(result.hoisted, 0);
    }

    #[test]
    fn test_find_preheader_single_pred() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let header = cfg.create_block("header");
        let body = cfg.create_block("body");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        cfg.add_edge(entry, header, EdgeType::Fallthrough);
        cfg.add_edge(header, body, EdgeType::ConditionalTrue);
        cfg.add_edge(header, exit, EdgeType::ConditionalFalse);
        cfg.add_edge(body, header, EdgeType::Unconditional);

        compute_dominators(&mut cfg);

        let mut loop_blocks = HashSet::new();
        loop_blocks.insert(header);
        loop_blocks.insert(body);

        // entry is single outside pred with 1 successor -> it IS the preheader
        let ph = find_or_create_preheader(&mut cfg, header, &loop_blocks);
        assert!(ph.is_some());
        assert_eq!(ph.unwrap(), entry);
    }

    #[test]
    fn test_split_edge_creates_preheader() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let other = cfg.create_block("other");
        let header = cfg.create_block("header");
        let body = cfg.create_block("body");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        // entry has 2 successors (header AND other)
        cfg.add_edge(entry, header, EdgeType::ConditionalTrue);
        cfg.add_edge(entry, other, EdgeType::ConditionalFalse);
        cfg.add_edge(other, exit, EdgeType::Fallthrough);
        cfg.add_edge(header, body, EdgeType::ConditionalTrue);
        cfg.add_edge(header, exit, EdgeType::ConditionalFalse);
        cfg.add_edge(body, header, EdgeType::Unconditional);

        compute_dominators(&mut cfg);

        let mut loop_blocks = HashSet::new();
        loop_blocks.insert(header);
        loop_blocks.insert(body);

        // entry has 2 successors -> split_edge creates preheader
        let ph = find_or_create_preheader(&mut cfg, header, &loop_blocks);
        assert!(ph.is_some());
        // Preheader should be a new block (not entry)
        assert_ne!(ph.unwrap(), entry);
    }
}
