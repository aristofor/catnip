// FILE: catnip_rs/src/cfg/ssa_gvn.rs
//! Global Value Numbering (GVN) on SSA form.
//!
//! Walks the dominator tree in preorder. For each SetLocals with a pure RHS,
//! assigns a value number based on (rhs_opcode, VN of operands).
//! Two expressions with the same value number compute the same value.

use super::graph::ControlFlowGraph;
use super::ssa::{SSAContext, SSAValue, ValueDef};
use super::ssa_cse::{extract_rhs_opcode, pure_opcodes};
use std::collections::HashMap;

/// A value number: identifies a unique computed value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueNumber(pub u32);

/// Expression key for value numbering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct VNExprKey {
    opcode: i32,
    operand_vns: Vec<ValueNumber>,
}

/// GVN state.
pub struct GVNContext {
    /// SSA value -> value number
    value_to_vn: HashMap<SSAValue, ValueNumber>,
    /// Expression -> value number (for CSE)
    expr_to_vn: HashMap<VNExprKey, ValueNumber>,
    /// Next value number to assign
    next_vn: u32,
    /// Value number -> canonical SSA value (first definition with this VN)
    vn_to_canonical: HashMap<ValueNumber, SSAValue>,
}

impl GVNContext {
    fn new() -> Self {
        Self {
            value_to_vn: HashMap::new(),
            expr_to_vn: HashMap::new(),
            next_vn: 0,
            vn_to_canonical: HashMap::new(),
        }
    }

    fn fresh_vn(&mut self, value: SSAValue) -> ValueNumber {
        let vn = ValueNumber(self.next_vn);
        self.next_vn += 1;
        self.value_to_vn.insert(value, vn);
        self.vn_to_canonical.entry(vn).or_insert(value);
        vn
    }

    fn get_vn(&mut self, value: SSAValue) -> ValueNumber {
        if let Some(&vn) = self.value_to_vn.get(&value) {
            vn
        } else {
            self.fresh_vn(value)
        }
    }

    fn lookup_or_add(&mut self, key: VNExprKey, defining_value: SSAValue) -> ValueNumber {
        if let Some(&vn) = self.expr_to_vn.get(&key) {
            self.value_to_vn.insert(defining_value, vn);
            vn
        } else {
            let vn = self.fresh_vn(defining_value);
            self.expr_to_vn.insert(key, vn);
            vn
        }
    }
}

/// Result of GVN.
pub struct GVNResult {
    /// Number of redundant expressions found
    pub redundant: usize,
    /// Replacements: SSA value -> canonical SSA value with same VN
    pub replacements: HashMap<SSAValue, SSAValue>,
}

/// Run GVN on the CFG in SSA form.
pub fn gvn(cfg: &ControlFlowGraph, ssa: &SSAContext) -> GVNResult {
    let pure_ops = pure_opcodes();
    let mut ctx = GVNContext::new();
    let mut replacements: HashMap<SSAValue, SSAValue> = HashMap::new();
    let mut redundant = 0;

    // Assign initial VNs to phi-defined values
    for (value, def) in &ssa.value_defs {
        if matches!(def, ValueDef::BlockParam { .. }) {
            ctx.fresh_vn(*value);
        }
    }

    let dom_preorder = dominator_preorder(cfg);

    for &block_id in &dom_preorder {
        let Some(block) = cfg.blocks.get(&block_id) else {
            continue;
        };

        for (instr_idx, op) in block.instructions.iter().enumerate() {
            let def_value = find_def_value(ssa, block_id, instr_idx);

            // Only process SetLocals with a pure RHS op
            let rhs_opcode = extract_rhs_opcode(op);
            let is_pure_setlocals = rhs_opcode
                .map(|opc| pure_ops.contains(&opc))
                .unwrap_or(false);

            if !is_pure_setlocals {
                // Non-pure or non-SetLocals: unique VN
                if let Some(value) = def_value {
                    ctx.fresh_vn(value);
                }
                continue;
            }

            let Some(value) = def_value else {
                continue;
            };

            // Build VN expression key from instruction_uses
            let uses = ssa.get_uses(block_id, instr_idx);
            if uses.is_empty() {
                ctx.fresh_vn(value);
                continue;
            }

            let operand_vns: Vec<ValueNumber> = uses.iter().map(|v| ctx.get_vn(*v)).collect();
            let key = VNExprKey {
                opcode: rhs_opcode.unwrap(),
                operand_vns,
            };

            let vn = ctx.lookup_or_add(key, value);

            if let Some(&canonical) = ctx.vn_to_canonical.get(&vn) {
                if canonical != value {
                    replacements.insert(value, canonical);
                    redundant += 1;
                }
            }
        }
    }

    GVNResult {
        redundant,
        replacements,
    }
}

fn find_def_value(ssa: &SSAContext, block_id: usize, instr_idx: usize) -> Option<SSAValue> {
    for (value, def) in &ssa.value_defs {
        if let ValueDef::Instruction {
            block,
            instr_idx: idx,
        } = def
        {
            if *block == block_id && *idx == instr_idx {
                return Some(*value);
            }
        }
    }
    None
}

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

/// Apply GVN results to the CFG.
pub fn apply_gvn(cfg: &mut ControlFlowGraph, result: &GVNResult) {
    // GVN replacements are SSAValue → SSAValue mappings.
    // Full application requires rewriting uses during SSA destruction.
    let _ = (cfg, result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::analysis::compute_dominators;
    use crate::cfg::edge::EdgeType;
    use crate::cfg::ssa_builder::SSABuilder;
    use crate::ir::opcode::IROpCode;

    #[test]
    fn test_gvn_empty() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let ssa = SSABuilder::build(&cfg);

        let result = gvn(&cfg, &ssa);
        assert_eq!(result.redundant, 0);
    }

    #[test]
    fn test_gvn_context_fresh_vn() {
        let mut ctx = GVNContext::new();
        let v1 = SSAValue::new(0, 0);
        let v2 = SSAValue::new(1, 0);

        let vn1 = ctx.fresh_vn(v1);
        let vn2 = ctx.fresh_vn(v2);

        assert_ne!(vn1, vn2);
        assert_eq!(ctx.get_vn(v1), vn1);
        assert_eq!(ctx.get_vn(v2), vn2);
    }

    #[test]
    fn test_gvn_context_lookup_or_add() {
        let mut ctx = GVNContext::new();
        let v1 = SSAValue::new(0, 0);
        let v2 = SSAValue::new(0, 1);

        let key = VNExprKey {
            opcode: IROpCode::Add as i32,
            operand_vns: vec![ValueNumber(0)],
        };

        let vn1 = ctx.lookup_or_add(key.clone(), v1);
        let vn2 = ctx.lookup_or_add(key, v2);

        assert_eq!(vn1, vn2);
    }
}
