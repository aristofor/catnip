// FILE: catnip_rs/src/cfg/ssa_dse.rs
//! Global Dead Store Elimination on SSA form.
//!
//! Counts references to each SSA value. If a value has 0 uses and its
//! defining instruction is a SetLocals with a pure RHS, the instruction
//! is dead and can be removed. Iterates to fixpoint because removing one
//! dead instruction may make its operands' definitions dead too.

use super::graph::ControlFlowGraph;
use super::ssa::{SSAContext, SSAValue, ValueDef};
use super::ssa_cse::{extract_rhs_opcode, pure_opcodes};
use crate::ir::opcode::IROpCode;
use std::collections::{HashMap, HashSet};

/// Result of global DSE.
pub struct DSEResult {
    /// Number of stores eliminated
    pub eliminated: usize,
    /// Dead instructions: (block_id, instr_idx)
    pub dead: HashSet<(usize, usize)>,
}

/// Run global dead store elimination on the CFG in SSA form.
pub fn global_dse(cfg: &ControlFlowGraph, ssa: &SSAContext) -> DSEResult {
    let pure_ops = pure_opcodes();

    // Step 1: Count uses for each SSA value
    let mut use_counts: HashMap<SSAValue, usize> = HashMap::new();

    // Initialize all defined values with 0 uses
    for value in ssa.value_defs.keys() {
        use_counts.insert(*value, 0);
    }

    // Count uses in phi operands (live phis only)
    for info in ssa.blocks.values() {
        for param in &info.params {
            let is_live = info.current_defs.get(&param.value.var) == Some(&param.value);
            if !is_live {
                continue;
            }
            for incoming in &param.incoming {
                let Some(val) = incoming else {
                    continue;
                };
                *use_counts.entry(*val).or_insert(0) += 1;
            }
        }
    }

    // Count uses from instruction_uses (per-instruction operand tracking)
    for uses in ssa.instruction_uses.values() {
        for val in uses {
            *use_counts.entry(*val).or_insert(0) += 1;
        }
    }

    // Mark values live at exit block as used
    if let Some(exit) = cfg.exit {
        if let Some(info) = ssa.blocks.get(&exit) {
            for value in info.current_defs.values() {
                *use_counts.entry(*value).or_insert(0) += 1;
            }
        }
    }

    // Step 2: Iterative elimination
    let mut dead: HashSet<(usize, usize)> = HashSet::new();
    let mut eliminated = 0;
    let mut changed = true;

    while changed {
        changed = false;

        for (value, def) in &ssa.value_defs {
            let uses = use_counts.get(value).copied().unwrap_or(0);
            if uses > 0 {
                continue;
            }

            if let ValueDef::Instruction { block, instr_idx } = def {
                let key = (*block, *instr_idx);
                if dead.contains(&key) {
                    continue;
                }

                // Check if instruction is a SetLocals with pure RHS
                let is_dead_candidate = if let Some(b) = cfg.blocks.get(block) {
                    if let Some(op) = b.instructions.get(*instr_idx) {
                        let rhs = extract_rhs_opcode(op);
                        rhs.map(|opc| pure_ops.contains(&opc)).unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_dead_candidate {
                    dead.insert(key);
                    eliminated += 1;
                    changed = true;

                    // Decrement use counts for this instruction's operands
                    let operand_uses = ssa.get_uses(*block, *instr_idx).to_vec();
                    for operand_val in &operand_uses {
                        if let Some(count) = use_counts.get_mut(operand_val) {
                            *count = count.saturating_sub(1);
                        }
                    }
                }
            }
        }
    }

    DSEResult { eliminated, dead }
}

/// Apply DSE results to the CFG by replacing dead instructions with Nop.
pub fn apply_dse(cfg: &mut ControlFlowGraph, result: &DSEResult) {
    for &(block_id, instr_idx) in &result.dead {
        if let Some(block) = cfg.get_block_mut(block_id) {
            if instr_idx < block.instructions.len() {
                block.instructions[instr_idx].ident = IROpCode::Nop as i32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::analysis::compute_dominators;
    use crate::cfg::edge::EdgeType;
    use crate::cfg::ssa_builder::SSABuilder;

    #[test]
    fn test_dse_empty_cfg() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let ssa = SSABuilder::build(&cfg);

        let result = global_dse(&cfg, &ssa);
        assert_eq!(result.eliminated, 0);
        assert!(result.dead.is_empty());
    }

    #[test]
    fn test_pure_opcodes_used() {
        let ops = pure_opcodes();
        assert!(!ops.is_empty());
        assert!(ops.contains(&(IROpCode::Add as i32)));
    }
}
