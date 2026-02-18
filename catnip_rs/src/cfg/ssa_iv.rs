// FILE: catnip_rs/src/cfg/ssa_iv.rs
//! Induction Variable detection and Strength Reduction on SSA form.
//!
//! Detects Basic Induction Variables (BIVs) at loop headers: variables
//! that change by a constant step each iteration (phi with linear recurrence).
//!
//! Detects Derived Induction Variables (DIVs) in loop bodies: expressions
//! like `j = i * scale` where `i` is a BIV and `scale` is a constant.
//!
//! Applies strength reduction: replaces `j = i * scale` (multiplication
//! per iteration) with `j = j + step*scale` (cheaper addition).
//!
//! Two-phase design:
//!   Phase 1 (detect_ivs): runs during SSA, uses phi analysis
//!   Phase 2 (apply_iv_strength_reduction): runs after SSA destruction

use super::graph::ControlFlowGraph;
use super::ssa::{SSAContext, SSAValue, ValueDef};
use super::ssa_cse::extract_rhs_opcode;
use crate::cfg::analysis::detect_loops;
use crate::core::nodes::Ref;
use crate::core::op::Op;
use crate::ir::opcode::IROpCode;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::collections::{HashMap, HashSet};

/// Initial value of a BIV before loop entry.
#[derive(Debug, Clone)]
pub enum IVInit {
    /// Compile-time constant
    Constant(i64),
    /// Variable defined outside loop (unknown value)
    Variable(String),
}

/// A Basic Induction Variable: phi at loop header with linear recurrence.
#[derive(Debug, Clone)]
pub struct BasicIV {
    pub var_name: String,
    pub var_id: usize,
    pub phi_value: SSAValue,
    pub init: IVInit,
    /// Step magnitude (always positive)
    pub step: i64,
    /// true = Add, false = Sub
    pub is_add: bool,
    pub header: usize,
    pub update_block: usize,
    pub update_instr_idx: usize,
}

/// A Derived Induction Variable: `j = biv * scale` in loop body.
#[derive(Debug, Clone)]
pub struct DerivedIV {
    pub var_name: String,
    pub block: usize,
    pub instr_idx: usize,
    /// Index into IVResult.bivs
    pub biv_idx: usize,
    pub scale: i64,
}

/// Result of IV detection (Phase 1). Consumed by Phase 2.
pub struct IVResult {
    pub bivs: Vec<BasicIV>,
    pub divs: Vec<DerivedIV>,
    /// header_block_id -> preheader_block_id
    pub preheaders: HashMap<usize, usize>,
}

// ---------------------------------------------------------------------------
// Phase 1: Detection
// ---------------------------------------------------------------------------

/// Detect induction variables in all loops of the CFG.
///
/// Requires dominators computed and SSA form built.
pub fn detect_ivs(cfg: &mut ControlFlowGraph, ssa: &SSAContext) -> IVResult {
    let loops = detect_loops(cfg);
    let mut bivs = Vec::new();
    let mut divs = Vec::new();
    let mut preheaders: HashMap<usize, usize> = HashMap::new();

    for (header, loop_blocks) in &loops {
        // Detect BIVs from loop header phis
        let loop_bivs_start = bivs.len();
        detect_bivs(cfg, ssa, *header, loop_blocks, &mut bivs);

        if bivs.len() == loop_bivs_start {
            continue; // No BIVs in this loop
        }

        // Find preheader (needed for Phase 2)
        if let Some(ph) = super::ssa_licm::find_or_create_preheader(cfg, *header, loop_blocks) {
            preheaders.insert(*header, ph);
        }

        // Detect DIVs in loop body
        detect_divs(
            cfg,
            ssa,
            *header,
            loop_blocks,
            &bivs,
            loop_bivs_start,
            &mut divs,
        );
    }

    IVResult {
        bivs,
        divs,
        preheaders,
    }
}

/// Detect Basic Induction Variables at a loop header.
fn detect_bivs(
    cfg: &ControlFlowGraph,
    ssa: &SSAContext,
    header: usize,
    loop_blocks: &HashSet<usize>,
    bivs: &mut Vec<BasicIV>,
) {
    let live_params = ssa.get_live_params(header);
    if live_params.is_empty() {
        return;
    }

    let preds = SSAContext::get_predecessor_blocks(cfg, header);
    if preds.len() != 2 {
        return; // Only handle simple loops with 1 outside + 1 back-edge predecessor
    }

    // Classify predecessors
    let (outside_idx, inside_idx) =
        if !loop_blocks.contains(&preds[0]) && loop_blocks.contains(&preds[1]) {
            (0, 1)
        } else if loop_blocks.contains(&preds[0]) && !loop_blocks.contains(&preds[1]) {
            (1, 0)
        } else {
            return; // Can't classify
        };

    for param in &live_params {
        // Get incoming values
        let init_value = match param.incoming.get(outside_idx) {
            Some(Some(v)) => *v,
            _ => continue,
        };
        let update_value = match param.incoming.get(inside_idx) {
            Some(Some(v)) => *v,
            _ => continue,
        };

        // Trace update to its defining instruction
        let (update_block, update_instr_idx) = match ssa.value_defs.get(&update_value) {
            Some(ValueDef::Instruction { block, instr_idx }) => (*block, *instr_idx),
            _ => continue,
        };

        if !loop_blocks.contains(&update_block) {
            continue;
        }

        // Check if update instruction is SetLocals with Add/Sub RHS
        let Some(block) = cfg.blocks.get(&update_block) else {
            continue;
        };
        let Some(update_op) = block.instructions.get(update_instr_idx) else {
            continue;
        };

        let Some(rhs_opcode) = extract_rhs_opcode(update_op) else {
            continue;
        };
        let is_add = if rhs_opcode == IROpCode::Add as i32 {
            true
        } else if rhs_opcode == IROpCode::Sub as i32 {
            false
        } else {
            continue;
        };

        // Extract step constant and verify one operand is the phi variable
        let var_name = match ssa.vars.name(param.value.var) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let Some(step) = extract_biv_step(update_op, &var_name) else {
            continue;
        };

        // Filter trivial cases
        if step == 0 || step == 1 {
            continue;
        }

        // Extract init value
        let init = extract_init_value(cfg, ssa, init_value);

        bivs.push(BasicIV {
            var_name,
            var_id: param.value.var,
            phi_value: param.value,
            init,
            step,
            is_add,
            header,
            update_block,
            update_instr_idx,
        });
    }
}

/// Extract the constant step from a BIV update instruction.
///
/// The update instruction is SetLocals(names, Add/Sub(Ref(var), constant)).
/// Returns Some(step) if pattern matches, None otherwise.
fn extract_biv_step(op: &Op, var_name: &str) -> Option<i64> {
    Python::attach(|py| {
        let args = op.get_args();
        let args_bound = args.bind(py);
        let args_tuple = args_bound.cast::<PyTuple>().ok()?;
        if args_tuple.len() < 2 {
            return None;
        }

        // args[1] = rhs Op (Add or Sub)
        let rhs = args_tuple.get_item(1).ok()?;
        let rhs_op = rhs.extract::<PyRef<Op>>().ok()?;
        let rhs_args = rhs_op.args.bind(py);
        let rhs_tuple = rhs_args.cast::<PyTuple>().ok()?;

        if rhs_tuple.len() != 2 {
            return None;
        }

        let left = rhs_tuple.get_item(0).ok()?;
        let right = rhs_tuple.get_item(1).ok()?;

        // Pattern: Ref(var) op constant  OR  constant op Ref(var)
        if let (Ok(r), Ok(c)) = (left.extract::<PyRef<Ref>>(), right.extract::<i64>()) {
            if r.ident == var_name {
                return Some(c);
            }
        }
        if let (Ok(c), Ok(r)) = (left.extract::<i64>(), right.extract::<PyRef<Ref>>()) {
            if r.ident == var_name {
                return Some(c);
            }
        }

        None
    })
}

/// Extract the initial value of a BIV from its SSA definition.
fn extract_init_value(cfg: &ControlFlowGraph, ssa: &SSAContext, init_ssa: SSAValue) -> IVInit {
    // Trace to defining instruction
    let Some(def) = ssa.value_defs.get(&init_ssa) else {
        return IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string());
    };

    let (block, instr_idx) = match def {
        ValueDef::Instruction { block, instr_idx } => (*block, *instr_idx),
        ValueDef::BlockParam { .. } => {
            // Init comes from another phi -- treat as variable
            return IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string());
        }
    };

    let Some(b) = cfg.blocks.get(&block) else {
        return IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string());
    };
    let Some(op) = b.instructions.get(instr_idx) else {
        return IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string());
    };

    if op.ident != IROpCode::SetLocals as i32 {
        return IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string());
    }

    // Try to extract literal from RHS
    Python::attach(|py| {
        let args = op.get_args();
        let args_bound = args.bind(py);
        let args_tuple = match args_bound.cast::<PyTuple>() {
            Ok(t) => t,
            Err(_) => {
                return IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string())
            }
        };
        if args_tuple.len() < 2 {
            return IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string());
        }
        let rhs = match args_tuple.get_item(1) {
            Ok(r) => r,
            Err(_) => {
                return IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string())
            }
        };

        // Direct integer literal
        if let Ok(val) = rhs.extract::<i64>() {
            return IVInit::Constant(val);
        }

        IVInit::Variable(ssa.vars.name(init_ssa.var).unwrap_or("?").to_string())
    })
}

/// Detect Derived Induction Variables in loop body blocks.
fn detect_divs(
    cfg: &ControlFlowGraph,
    ssa: &SSAContext,
    header: usize,
    loop_blocks: &HashSet<usize>,
    bivs: &[BasicIV],
    bivs_start: usize,
    divs: &mut Vec<DerivedIV>,
) {
    // Build set of BIV phi values for fast lookup
    let biv_phis: HashMap<SSAValue, usize> = bivs[bivs_start..]
        .iter()
        .enumerate()
        .map(|(i, biv)| (biv.phi_value, bivs_start + i))
        .collect();

    // Count assignments per variable in the loop to filter multi-def vars
    let mut var_def_counts: HashMap<String, usize> = HashMap::new();
    for &block_id in loop_blocks {
        let Some(block) = cfg.blocks.get(&block_id) else {
            continue;
        };
        for op in &block.instructions {
            if op.ident == IROpCode::SetLocals as i32 {
                if let Some(name) = extract_set_locals_name(op) {
                    *var_def_counts.entry(name).or_insert(0) += 1;
                }
            }
        }
    }

    for &block_id in loop_blocks {
        if block_id == header {
            continue; // Skip header
        }

        let Some(block) = cfg.blocks.get(&block_id) else {
            continue;
        };

        for (instr_idx, op) in block.instructions.iter().enumerate() {
            let Some(rhs_opcode) = extract_rhs_opcode(op) else {
                continue;
            };
            if rhs_opcode != IROpCode::Mul as i32 {
                continue;
            }

            // Check one operand is a BIV, the other is a constant
            let uses = ssa.get_uses(block_id, instr_idx);
            let mut found_biv_idx = None;
            for use_val in uses {
                if let Some(&biv_idx) = biv_phis.get(use_val) {
                    found_biv_idx = Some(biv_idx);
                    break;
                }
            }
            let Some(biv_idx) = found_biv_idx else {
                continue;
            };

            // Extract the constant scale and variable name
            let Some((var_name, scale)) = extract_div_info(op, &bivs[biv_idx].var_name) else {
                continue;
            };

            // Filter: skip trivial scales
            if scale == 0 || scale == 1 || scale == -1 {
                continue;
            }

            // Filter: skip if variable has multiple definitions in the loop
            if var_def_counts.get(&var_name).copied().unwrap_or(0) > 1 {
                continue;
            }

            divs.push(DerivedIV {
                var_name,
                block: block_id,
                instr_idx,
                biv_idx,
                scale,
            });
        }
    }
}

/// Extract the variable name from a SetLocals instruction.
fn extract_set_locals_name(op: &Op) -> Option<String> {
    if op.ident != IROpCode::SetLocals as i32 {
        return None;
    }
    Python::attach(|py| {
        let args = op.get_args();
        let args_bound = args.bind(py);
        let args_tuple = args_bound.cast::<PyTuple>().ok()?;
        if args_tuple.is_empty() {
            return None;
        }
        let names = args_tuple.get_item(0).ok()?;
        if let Ok(names_tuple) = names.cast::<PyTuple>() {
            if names_tuple.len() == 1 {
                return names_tuple.get_item(0).ok()?.extract::<String>().ok();
            }
        }
        names.extract::<String>().ok()
    })
}

/// Extract DIV info from a SetLocals with Mul RHS.
///
/// Returns (var_name, scale) if pattern matches.
fn extract_div_info(op: &Op, biv_var: &str) -> Option<(String, i64)> {
    Python::attach(|py| {
        let args = op.get_args();
        let args_bound = args.bind(py);
        let args_tuple = args_bound.cast::<PyTuple>().ok()?;
        if args_tuple.len() < 2 {
            return None;
        }

        // Extract var name from args[0]
        let names = args_tuple.get_item(0).ok()?;
        let var_name = if let Ok(names_tuple) = names.cast::<PyTuple>() {
            if names_tuple.len() != 1 {
                return None; // Multi-assignment, skip
            }
            names_tuple.get_item(0).ok()?.extract::<String>().ok()?
        } else {
            names.extract::<String>().ok()?
        };

        // Extract Mul operands from args[1]
        let rhs = args_tuple.get_item(1).ok()?;
        let rhs_op = rhs.extract::<PyRef<Op>>().ok()?;
        let rhs_args = rhs_op.args.bind(py);
        let rhs_tuple = rhs_args.cast::<PyTuple>().ok()?;

        if rhs_tuple.len() != 2 {
            return None;
        }

        let left = rhs_tuple.get_item(0).ok()?;
        let right = rhs_tuple.get_item(1).ok()?;

        // Pattern: Ref(biv) * constant  OR  constant * Ref(biv)
        if let (Ok(r), Ok(c)) = (left.extract::<PyRef<Ref>>(), right.extract::<i64>()) {
            if r.ident == biv_var {
                return Some((var_name, c));
            }
        }
        if let (Ok(c), Ok(r)) = (left.extract::<i64>(), right.extract::<PyRef<Ref>>()) {
            if r.ident == biv_var {
                return Some((var_name, c));
            }
        }

        None
    })
}

// ---------------------------------------------------------------------------
// Phase 2: Transformation
// ---------------------------------------------------------------------------

/// Apply strength reduction: replace Mul-based DIVs with Add-based accumulators.
pub fn apply_iv_strength_reduction(cfg: &mut ControlFlowGraph, result: &IVResult) {
    if result.divs.is_empty() {
        return;
    }

    Python::attach(|py| {
        for div in &result.divs {
            let biv = &result.bivs[div.biv_idx];

            // Compute the effective step for the accumulator
            let effective_step = if biv.is_add {
                biv.step * div.scale
            } else {
                -(biv.step * div.scale)
            };

            // 1. Insert init in preheader
            let Some(&preheader) = result.preheaders.get(&biv.header) else {
                continue;
            };

            let init_value = match &biv.init {
                IVInit::Constant(c) => c * div.scale,
                IVInit::Variable(_) => {
                    // Can't pre-compute init * scale at compile time
                    // Insert Mul(Ref(var), scale) in preheader instead
                    if let Ok(init_op) =
                        create_set_locals_mul(py, &div.var_name, &biv.var_name, div.scale)
                    {
                        if let Some(ph_block) = cfg.get_block_mut(preheader) {
                            let pos = find_insert_position(ph_block);
                            ph_block.instructions.insert(pos, init_op);
                        }
                    }
                    // Skip the constant init path
                    nop_and_insert_update(cfg, py, div, effective_step);
                    continue;
                }
            };

            // Insert SetLocals(("j",), literal_init)
            if let Ok(init_op) = create_set_locals_literal(py, &div.var_name, init_value) {
                if let Some(ph_block) = cfg.get_block_mut(preheader) {
                    let pos = find_insert_position(ph_block);
                    ph_block.instructions.insert(pos, init_op);
                }
            }

            // 2. Nop original Mul + 3. Insert Add update
            nop_and_insert_update(cfg, py, div, effective_step);
        }
    });
}

/// Nop the original multiplication and insert the addition update at end of body.
fn nop_and_insert_update(
    cfg: &mut ControlFlowGraph,
    py: Python<'_>,
    div: &DerivedIV,
    effective_step: i64,
) {
    // Nop the original Mul instruction
    if let Some(block) = cfg.get_block_mut(div.block) {
        if div.instr_idx < block.instructions.len() {
            block.instructions[div.instr_idx].ident = IROpCode::Nop as i32;
        }
    }

    // Insert j = j + effective_step at end of body block
    if let Ok(update_op) = create_set_locals_add(py, &div.var_name, effective_step) {
        if let Some(block) = cfg.get_block_mut(div.block) {
            let pos = find_insert_position(block);
            block.instructions.insert(pos, update_op);
        }
    }
}

/// Find insertion position: before terminators (return/break/continue).
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

// ---------------------------------------------------------------------------
// Op creation helpers
// ---------------------------------------------------------------------------

/// Create SetLocals(("var",), literal_int)
fn create_set_locals_literal(py: Python<'_>, var_name: &str, value: i64) -> PyResult<Op> {
    let names = PyTuple::new(py, &[var_name])?;
    let value_py = value.into_pyobject(py)?.into_any();
    let args = PyTuple::new(py, vec![names.into_any(), value_py])?;
    let kwargs = PyDict::new(py);
    Ok(Op::from_rust(
        py,
        IROpCode::SetLocals as i32,
        args.unbind().into(),
        kwargs.unbind().into(),
        false,
        -1,
        -1,
    ))
}

/// Create SetLocals(("var",), Add(Ref("var"), step))
fn create_set_locals_add(py: Python<'_>, var_name: &str, step: i64) -> PyResult<Op> {
    // Create the Add Op: Add(Ref(var), step)
    let ref_node = Py::new(
        py,
        Ref {
            ident: var_name.to_string(),
            start_byte: -1,
            end_byte: -1,
        },
    )?;
    let step_py = step.into_pyobject(py)?.into_any();
    let add_args = PyTuple::new(py, vec![ref_node.into_bound(py).into_any(), step_py])?;
    let add_kwargs = PyDict::new(py);
    let add_op = Op::from_rust(
        py,
        IROpCode::Add as i32,
        add_args.unbind().into(),
        add_kwargs.unbind().into(),
        false,
        -1,
        -1,
    );

    // Wrap in SetLocals
    let names = PyTuple::new(py, &[var_name])?;
    let add_py = Py::new(py, add_op)?;
    let args = PyTuple::new(py, vec![names.into_any(), add_py.into_bound(py).into_any()])?;
    let kwargs = PyDict::new(py);
    Ok(Op::from_rust(
        py,
        IROpCode::SetLocals as i32,
        args.unbind().into(),
        kwargs.unbind().into(),
        false,
        -1,
        -1,
    ))
}

/// Create SetLocals(("var",), Mul(Ref("src_var"), scale))
fn create_set_locals_mul(
    py: Python<'_>,
    var_name: &str,
    src_var: &str,
    scale: i64,
) -> PyResult<Op> {
    let ref_node = Py::new(
        py,
        Ref {
            ident: src_var.to_string(),
            start_byte: -1,
            end_byte: -1,
        },
    )?;
    let scale_py = scale.into_pyobject(py)?.into_any();
    let mul_args = PyTuple::new(py, vec![ref_node.into_bound(py).into_any(), scale_py])?;
    let mul_kwargs = PyDict::new(py);
    let mul_op = Op::from_rust(
        py,
        IROpCode::Mul as i32,
        mul_args.unbind().into(),
        mul_kwargs.unbind().into(),
        false,
        -1,
        -1,
    );

    let names = PyTuple::new(py, &[var_name])?;
    let mul_py = Py::new(py, mul_op)?;
    let args = PyTuple::new(py, vec![names.into_any(), mul_py.into_bound(py).into_any()])?;
    let kwargs = PyDict::new(py);
    Ok(Op::from_rust(
        py,
        IROpCode::SetLocals as i32,
        args.unbind().into(),
        kwargs.unbind().into(),
        false,
        -1,
        -1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::analysis::compute_dominators;
    use crate::cfg::edge::EdgeType;
    use crate::cfg::ssa_builder::SSABuilder;

    #[test]
    fn test_iv_no_loops() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let ssa = SSABuilder::build(&cfg);

        let result = detect_ivs(&mut cfg, &ssa);
        assert_eq!(result.bivs.len(), 0);
        assert_eq!(result.divs.len(), 0);
    }

    #[test]
    fn test_iv_loop_no_phi() {
        // Loop with no instructions -> no BIVs
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
        let ssa = SSABuilder::build(&cfg);

        let result = detect_ivs(&mut cfg, &ssa);
        assert_eq!(result.bivs.len(), 0);
        assert_eq!(result.divs.len(), 0);
    }

    #[test]
    fn test_apply_empty_result() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        let result = IVResult {
            bivs: Vec::new(),
            divs: Vec::new(),
            preheaders: HashMap::new(),
        };

        // Should not panic
        apply_iv_strength_reduction(&mut cfg, &result);
    }
}
