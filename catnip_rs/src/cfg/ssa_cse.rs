// FILE: catnip_rs/src/cfg/ssa_cse.rs
//! Inter-block Common Subexpression Elimination on SSA form.
//!
//! Walks the dominator tree. For each SetLocals whose RHS is a pure op,
//! builds a key from (rhs_opcode, SSA values of operands). If an identical
//! expression is available in a dominating block, marks the instruction
//! as redundant.

use super::graph::ControlFlowGraph;
use super::ssa::{SSAContext, SSAExprKey, SSAValue};
use crate::core::op::Op;
use crate::ir::opcode::IROpCode;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use std::collections::{HashMap, HashSet};

/// Pure operations safe for CSE (no side effects, deterministic).
pub fn pure_opcodes() -> HashSet<i32> {
    let mut ops = HashSet::new();
    // Arithmetic
    ops.insert(IROpCode::Add as i32);
    ops.insert(IROpCode::Sub as i32);
    ops.insert(IROpCode::Mul as i32);
    ops.insert(IROpCode::Div as i32);
    ops.insert(IROpCode::TrueDiv as i32);
    ops.insert(IROpCode::FloorDiv as i32);
    ops.insert(IROpCode::Mod as i32);
    ops.insert(IROpCode::Pow as i32);
    ops.insert(IROpCode::Neg as i32);
    ops.insert(IROpCode::Pos as i32);
    // Comparison
    ops.insert(IROpCode::Eq as i32);
    ops.insert(IROpCode::Ne as i32);
    ops.insert(IROpCode::Lt as i32);
    ops.insert(IROpCode::Le as i32);
    ops.insert(IROpCode::Gt as i32);
    ops.insert(IROpCode::Ge as i32);
    // Logical
    ops.insert(IROpCode::And as i32);
    ops.insert(IROpCode::Or as i32);
    ops.insert(IROpCode::Not as i32);
    // Bitwise
    ops.insert(IROpCode::BAnd as i32);
    ops.insert(IROpCode::BOr as i32);
    ops.insert(IROpCode::BXor as i32);
    ops.insert(IROpCode::BNot as i32);
    ops.insert(IROpCode::LShift as i32);
    ops.insert(IROpCode::RShift as i32);
    // Access (pure reads)
    ops.insert(IROpCode::GetAttr as i32);
    ops.insert(IROpCode::GetItem as i32);
    ops
}

/// Extract the opcode of the RHS of a SetLocals instruction.
///
/// SetLocals args = (names, rhs, unpack).
/// Returns Some(rhs_opcode) if the RHS is an Op, None if it's a literal or Ref.
pub fn extract_rhs_opcode(op: &crate::core::op::Op) -> Option<i32> {
    if op.ident != IROpCode::SetLocals as i32 {
        return None;
    }
    Python::attach(|py| {
        let args = op.get_args();
        let args_bound = args.bind(py);
        let args_tuple = args_bound.cast::<PyTuple>().ok()?;
        if args_tuple.len() < 2 {
            return None;
        }
        let rhs = args_tuple.get_item(1).ok()?;
        let rhs_op = rhs.extract::<PyRef<Op>>().ok()?;
        Some(rhs_op.ident)
    })
}

/// Result of inter-block CSE.
pub struct CSEResult {
    /// Number of instructions eliminated
    pub eliminated: usize,
    /// Replacements: (block_id, instr_idx) -> SSA value of the dominating expression
    pub replacements: HashMap<(usize, usize), SSAValue>,
}

/// Run inter-block CSE on the CFG in SSA form.
///
/// For each SetLocals whose RHS is a pure op, builds an expression key
/// from (rhs_opcode, instruction_uses). If the same expression was already
/// computed in a dominating block, records a replacement.
pub fn inter_block_cse(cfg: &ControlFlowGraph, ssa: &SSAContext) -> CSEResult {
    let pure_ops = pure_opcodes();
    let mut available: HashMap<SSAExprKey, SSAValue> = HashMap::new();
    let mut replacements: HashMap<(usize, usize), SSAValue> = HashMap::new();
    let mut eliminated = 0;

    let dom_preorder = dominator_preorder(cfg);

    for &block_id in &dom_preorder {
        let Some(block) = cfg.blocks.get(&block_id) else {
            continue;
        };

        for (instr_idx, op) in block.instructions.iter().enumerate() {
            // Only process SetLocals with a pure RHS op
            let Some(rhs_opcode) = extract_rhs_opcode(op) else {
                continue;
            };
            if !pure_ops.contains(&rhs_opcode) {
                continue;
            }

            // Build expression key from pre-computed instruction uses
            let uses = ssa.get_uses(block_id, instr_idx);
            if uses.is_empty() {
                continue;
            }

            let key = SSAExprKey {
                opcode: rhs_opcode,
                operands: uses.iter().map(|v| super::ssa::ExprOperand::Var(*v)).collect(),
            };

            if !key.is_resolved() {
                continue;
            }

            // Find the SSA value defined by this SetLocals
            let def_value = find_def_value(ssa, block_id, instr_idx);

            if let Some(&existing_value) = available.get(&key) {
                // Expression already computed in a dominating block
                if let Some(dv) = def_value {
                    replacements.insert((block_id, instr_idx), existing_value);
                    let _ = dv; // the defined value is now redundant
                    eliminated += 1;
                }
            } else {
                // First occurrence: record in available set
                if let Some(dv) = def_value {
                    available.insert(key, dv);
                }
            }
        }
    }

    CSEResult {
        eliminated,
        replacements,
    }
}

/// Find the SSA value defined by an instruction (if any).
fn find_def_value(ssa: &SSAContext, block_id: usize, instr_idx: usize) -> Option<SSAValue> {
    for (value, def) in &ssa.value_defs {
        if let super::ssa::ValueDef::Instruction { block, instr_idx: idx } = def {
            if *block == block_id && *idx == instr_idx {
                return Some(*value);
            }
        }
    }
    None
}

/// Walk the dominator tree in preorder.
fn dominator_preorder(cfg: &ControlFlowGraph) -> Vec<usize> {
    let Some(entry) = cfg.entry else {
        return Vec::new();
    };

    let mut result = Vec::new();
    let mut stack = vec![entry];

    while let Some(block_id) = stack.pop() {
        result.push(block_id);

        if let Some(block) = cfg.blocks.get(&block_id) {
            let mut children: Vec<usize> = block.dominated.iter().copied().collect();
            children.sort_unstable();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
    }

    result
}

/// Apply CSE results to the CFG by removing eliminated instructions.
///
/// Marks eliminated instructions as Nop (preserving block structure).
pub fn apply_cse(cfg: &mut ControlFlowGraph, result: &CSEResult) {
    for &(block_id, instr_idx) in result.replacements.keys() {
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
    fn test_dominator_preorder_linear() {
        let mut cfg = ControlFlowGraph::new("test");
        let a = cfg.create_block("a");
        let b = cfg.create_block("b");
        let c = cfg.create_block("c");
        cfg.set_entry(a);
        cfg.set_exit(c);
        cfg.add_edge(a, b, EdgeType::Fallthrough);
        cfg.add_edge(b, c, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);

        let preorder = dominator_preorder(&cfg);
        assert_eq!(preorder[0], a);
        assert!(preorder.contains(&b));
        assert!(preorder.contains(&c));
    }

    #[test]
    fn test_cse_empty_cfg() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let ssa = SSABuilder::build(&cfg);

        let result = inter_block_cse(&cfg, &ssa);
        assert_eq!(result.eliminated, 0);
    }

    #[test]
    fn test_pure_opcodes_complete() {
        let ops = pure_opcodes();
        assert!(ops.contains(&(IROpCode::Add as i32)));
        assert!(ops.contains(&(IROpCode::Eq as i32)));
        assert!(ops.contains(&(IROpCode::And as i32)));
        assert!(ops.contains(&(IROpCode::GetAttr as i32)));
        assert!(!ops.contains(&(IROpCode::OpIf as i32)));
        assert!(!ops.contains(&(IROpCode::SetLocals as i32)));
    }
}
