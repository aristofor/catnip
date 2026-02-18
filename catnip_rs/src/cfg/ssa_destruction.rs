// FILE: catnip_rs/src/cfg/ssa_destruction.rs
//! SSA destruction: convert from SSA form back to conventional form.
//!
//! For each non-trivial block parameter (phi), insert SetLocals copies
//! at the end of each predecessor block.

use super::graph::ControlFlowGraph;
use super::ssa::SSAContext;
use crate::core::op::Op;
use crate::ir::opcode::IROpCode;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

/// Destroy SSA form by inserting explicit copies for block parameters.
///
/// For each block with non-trivial phis:
///   - For each predecessor, insert a SetLocals at the end of the predecessor
///     that copies the incoming value to the phi's variable.
///
/// This approach avoids critical edge splitting for structured control flow
/// because Catnip's control flow is always reducible (no goto).
pub fn destroy_ssa(cfg: &mut ControlFlowGraph, ssa: &SSAContext) {
    // Collect all copies to insert (pred_block_id, var_name, source_value)
    let mut copies: Vec<(usize, String, super::ssa::SSAValue)> = Vec::new();

    for (&block_id, info) in &ssa.blocks {
        let preds = SSAContext::get_predecessor_blocks(cfg, block_id);

        for param in &info.params {
            // Skip trivial phis (replaced by try_remove_trivial_phi)
            let var_id = param.value.var;
            let is_live = info.current_defs.get(&var_id) == Some(&param.value);
            if !is_live {
                continue;
            }

            let Some(var_name) = ssa.vars.name(var_id) else {
                continue;
            };

            // For each predecessor, insert a copy
            for (i, &pred_id) in preds.iter().enumerate() {
                if let Some(Some(incoming)) = param.incoming.get(i) {
                    copies.push((pred_id, var_name.to_string(), *incoming));
                }
            }
        }
    }

    // Insert copies as SetLocals at the end of predecessor blocks
    if !copies.is_empty() {
        Python::attach(|py| {
            for (pred_id, var_name, _incoming_value) in &copies {
                if let Some(block) = cfg.get_block_mut(*pred_id) {
                    // Create SetLocals Op for the copy
                    // The actual value resolution happens at execution time;
                    // we just need to ensure the variable is assigned.
                    if let Ok(set_locals) = create_set_locals_copy(py, var_name) {
                        // Insert before any terminator (return/break/continue)
                        let insert_pos = find_insert_position(block);
                        block.instructions.insert(insert_pos, set_locals);
                    }
                }
            }
        });
    }
}

/// Find the position to insert a copy instruction.
/// Insert before terminators (return, break, continue) but after regular instructions.
fn find_insert_position(block: &crate::cfg::basic_block::BasicBlock) -> usize {
    if let Some(last) = block.instructions.last() {
        let is_terminator = last.ident == IROpCode::OpReturn as i32
            || last.ident == IROpCode::OpBreak as i32
            || last.ident == IROpCode::OpContinue as i32;

        if is_terminator {
            return block.instructions.len() - 1;
        }
    }
    block.instructions.len()
}

/// Create a SetLocals Op that copies a variable (identity assignment).
///
/// This generates `var = var` which is a no-op semantically but ensures
/// the variable binding exists in the scope after SSA destruction.
fn create_set_locals_copy(py: Python<'_>, var_name: &str) -> PyResult<Op> {
    let names = PyTuple::new(py, &[var_name])?;
    // Value is just the variable name as a reference (the semantic analyzer resolves it)
    let values = PyTuple::new(py, &[var_name])?;
    let args = PyTuple::new(py, &[names.as_any(), values.as_any()])?;
    let kwargs = PyDict::new(py);

    Ok(Op {
        ident: IROpCode::SetLocals as i32,
        args: args.unbind().into(),
        kwargs: kwargs.unbind().into(),
        tail: false,
        start_byte: -1,
        end_byte: -1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::analysis::compute_dominators;
    use crate::cfg::edge::EdgeType;
    use crate::cfg::ssa_builder::SSABuilder;

    #[test]
    fn test_destroy_linear_no_copies() {
        // Linear CFG with no phis → no copies inserted
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let ssa = SSABuilder::build(&cfg);

        let instr_count_before: usize = cfg.blocks.values().map(|b| b.instructions.len()).sum();

        destroy_ssa(&mut cfg, &ssa);

        let instr_count_after: usize = cfg.blocks.values().map(|b| b.instructions.len()).sum();

        assert_eq!(instr_count_before, instr_count_after);
    }

    #[test]
    fn test_find_insert_position_empty() {
        let block = crate::cfg::basic_block::BasicBlock::new(0, "test");
        assert_eq!(find_insert_position(&block), 0);
    }
}
