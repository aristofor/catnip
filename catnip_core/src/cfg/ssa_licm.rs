// FILE: catnip_core/src/cfg/ssa_licm.rs
//! Loop-Invariant Code Motion (LICM) on SSA form, with a guarded hoist block.
//!
//! For each natural `while` loop, identifies instructions whose operands are
//! all defined outside the loop. These compute the same value on every
//! iteration and are hoisted out -- into a block guarded by a copy of the loop
//! condition (`if cond { hoisted }` right before the loop), NOT into the
//! unconditional preheader. The guard closes the speculation hole: on a
//! zero-trip loop the original never runs the body, so the hoisted code must
//! not run either (it could fault, and it must not define its target).
//! The guarded form is equivalent to loop rotation for hoist placement,
//! without touching the loop structure: the emitted CFG shape is exactly the
//! builder's if-without-else pattern (guard -> hoist/skip -> header), so the
//! reconstruction needs no new region kind.
//!
//! Soundness gates, each refusing rather than degrading:
//! - `while` only; a `for` guard would need a non-consuming emptiness test on
//!   the iterable (deferred);
//! - the loop condition must be a pure expression: the header re-tests it right
//!   after the guard, so it is evaluated once more than in the source;
//! - pure RHS, single target, single def of the target inside the loop;
//! - every in-loop use of the target's variable (instruction uses, and block
//!   conditions via `current_defs`, the same exit-of-block assumption
//!   rename_conditions relies on) must read exactly the hoisted definition's
//!   value -- a stale use (nested-loop condition testing the pre-loop value,
//!   a read before the def in iteration one) would observe the hoist.
//!
//! Requires dominators + SSA form; recomputes dominators itself when it
//! inserted guard blocks, so the CFG is consistent for downstream passes.

use super::graph::ControlFlowGraph;
use super::ssa::{SSAContext, SSAValue, ValueDef};
use super::ssa_builder::{node_opcode, setlocals_names};
use super::ssa_cse::{extract_rhs_opcode, pure_opcodes};
use crate::cfg::analysis::{compute_dominators, detect_loops};
use crate::cfg::edge::EdgeType;
use crate::ir::{IR, IROpCode};
use crate::semantic::passes::collect_refs;
use std::collections::HashSet;

/// Result of LICM.
pub struct LICMResult {
    /// Number of instructions hoisted
    pub hoisted: usize,
}

/// Run LICM on the CFG in SSA form.
///
/// For each natural `while` loop with a pure condition:
/// 1. Find the preheader (or create one via edge splitting)
/// 2. Collect hoistable instructions (see the module-level gates)
/// 3. Move them into a fresh block guarded by a copy of the loop condition,
///    inserted between the preheader and the header
pub fn licm(cfg: &mut ControlFlowGraph, ssa: &SSAContext) -> LICMResult {
    let pure_ops = pure_opcodes();
    let loops = detect_loops(cfg);
    let mut total_hoisted = 0;
    let mut inserted_guard = false;

    for (header, loop_blocks) in &loops {
        // `while` with a pure condition only: the guard duplicates the
        // condition, and the header re-tests it right after the hoisted code.
        let Some(cond) = while_condition(cfg, *header) else {
            continue;
        };
        if !expr_is_pure(&cond, &pure_ops) {
            continue;
        }

        // Sources of every edge re-entering the header (the natural back-edge
        // and any `continue`): a candidate must dominate them all, or it does
        // not run on every iteration -- hoisting a def that lives on a
        // conditional path (an if branch, a tail cut off by `continue`) turns
        // a conditional store into an unconditional one (found by the Phase 4
        // property harness: `while .. { if cond { b = inv; continue } }` with
        // the branch never taken still defined `b` after hoisting).
        let back_edge_sources: Vec<usize> = loop_blocks
            .iter()
            .copied()
            .filter(|&b| {
                cfg.blocks.get(&b).is_some_and(|blk| {
                    blk.successors
                        .iter()
                        .any(|&e| cfg.edges.get(e).is_some_and(|edge| edge.target == *header))
                })
            })
            .collect();

        // Early exits (`break`/`return`) leave an iteration before the
        // candidate has run: refuse the whole loop (v1 -- finer would be
        // per-candidate dominance over each exiting block). Two detections,
        // both needed:
        //
        // - any non-header loop block with an edge leaving the natural loop:
        //   the exiting blocks themselves (a branch ending in `break`) reach
        //   no back-edge, so they are NOT in `loop_blocks` -- what is visible
        //   from inside is the edge toward them;
        // - a `match` op preserved in a loop block: its arms are opaque to
        //   the CFG (op-preservation -- the arm blocks carry the whole arm as
        //   one instruction and a Fallthrough edge to the merge), so an arm's
        //   `break`/`continue`/`return` has NO edge; scan the preserved IR
        //   instead (found by the Phase 4 property harness: an arm's break
        //   skipped the candidate but the hoisted form still ran it).
        let has_early_exit = loop_blocks.iter().any(|&b| {
            cfg.blocks.get(&b).is_some_and(|blk| {
                let escaping_edge = b != *header
                    && blk
                        .successors
                        .iter()
                        .any(|&e| cfg.edges.get(e).is_some_and(|edge| !loop_blocks.contains(&edge.target)));
                escaping_edge
                    || blk.instructions.iter().any(|op| {
                        matches!(op, IR::Op { opcode, .. } if *opcode == IROpCode::OpMatch)
                            && contains_opaque_loop_control(op)
                    })
            })
        });
        if has_early_exit {
            continue;
        }

        // A call (or broadcast of a user function) in the loop body can read
        // any free name through a captured closure: late binding resolves a
        // closure's globals at the call, not where it was built. Hoisting a
        // candidate `x = inv` above such a call moves the value the call
        // observes from the pre-loop `x` to `inv` on the first iteration. The
        // read is invisible to the SSA use sets (the name is not an operand of
        // the call site), so `value_reaches_all_loop_uses` cannot catch it.
        // Refuse the whole loop when any body block holds a call (v1 wholesale
        // refusal, like the early-exit guard; finer would be per-candidate "the
        // def precedes every call"). Found by the Phase 4 property harness:
        // `f0 = () => b` then `while .. { e = f0()\n b = d - 2 }` hoisted the
        // invariant `b = d - 2` and f0() then read the hoisted value.
        let has_body_call = loop_blocks.iter().any(|&b| {
            cfg.blocks.get(&b).is_some_and(|blk| {
                blk.instructions.iter().any(|op| {
                    // A preserved loop-header op (OpWhile/OpFor) duplicates the
                    // body blocks scanned on their own -- do not double-count.
                    !matches!(node_opcode(op), Some(IROpCode::OpWhile | IROpCode::OpFor)) && contains_call(op)
                }) || blk.condition.as_ref().is_some_and(contains_call)
            })
        });
        if has_body_call {
            continue;
        }

        // Collect definitions inside the loop
        let loop_defs = collect_loop_defs(ssa, loop_blocks);

        // Names assigned inside opaque match arms: those defs have no SSA
        // value (op-preservation), so `is_loop_invariant` cannot see them --
        // an operand rewritten by an arm looked loop-invariant and was
        // hoisted with its pre-loop value (found by the Phase 4 property
        // harness: `a = g * -1` hoisted while an arm did `g = 0`).
        let mut opaque_def_names: HashSet<String> = HashSet::new();
        for &block_id in loop_blocks {
            if let Some(block) = cfg.blocks.get(&block_id) {
                for op in &block.instructions {
                    if matches!(op, IR::Op { opcode, .. } if *opcode == IROpCode::OpMatch) {
                        collect_opaque_def_names(op, &mut opaque_def_names);
                    }
                }
            }
        }

        // Find hoistable instructions
        let mut to_hoist: Vec<(usize, usize, IR)> = Vec::new();

        for &block_id in loop_blocks {
            if block_id == *header {
                continue; // Don't hoist from header (contains loop condition)
            }

            // Runs-every-iteration guard: the candidate's block dominates
            // every re-entry into the header.
            let dominates_all_back_edges = back_edge_sources.iter().all(|&src| {
                cfg.blocks
                    .get(&src)
                    .is_some_and(|blk| blk.dominators.contains(&block_id))
            });
            if !dominates_all_back_edges {
                continue;
            }

            let Some(block) = cfg.blocks.get(&block_id) else {
                continue;
            };

            for (instr_idx, op) in block.instructions.iter().enumerate() {
                // Only hoist SetLocals whose RHS is pure -- checked recursively,
                // not just the head opcode: `t = f() + 1` has a pure Add at the
                // top with a call nested in its args, and hoisting it would move
                // the call's effects and faults across iterations.
                if extract_rhs_opcode(op).is_none() {
                    continue;
                }
                let rhs_pure = matches!(op, IR::Op { args, .. }
                    if args.get(1).is_some_and(|rhs| expr_is_pure(rhs, &pure_ops)));
                if !rhs_pure {
                    continue;
                }

                // The target must have a single definition in the loop: hoisting
                // one of several defs of the same variable drops a per-iteration
                // reset (e.g. `t = a + 1` followed by `t = t + i`).
                if !target_single_def_in_loop(ssa, op, loop_blocks) {
                    continue;
                }

                // Check if all operands are defined outside the loop
                if !is_loop_invariant(ssa, block_id, instr_idx, &loop_defs) {
                    continue;
                }

                // Neither the target nor any RHS operand may touch a name an
                // opaque match arm assigns (invisible to the SSA def sets).
                if !opaque_def_names.is_empty() {
                    let target_opaque = matches!(op, IR::Op { args, .. }
                        if args.first().is_some_and(|t| mentions_any_name(t, &opaque_def_names)));
                    let rhs_opaque = matches!(op, IR::Op { args, .. }
                        if args.get(1).is_some_and(|rhs| mentions_any_name(rhs, &opaque_def_names)));
                    if target_opaque || rhs_opaque {
                        continue;
                    }
                }

                // Every in-loop reader of the target must see the hoisted def's
                // value, not a stale pre-loop version (header phi, nested-loop
                // condition, read-before-def in iteration one).
                let Some(value) = def_value_at(ssa, block_id, instr_idx) else {
                    continue;
                };
                if !value_reaches_all_loop_uses(cfg, ssa, value, loop_blocks) {
                    continue;
                }

                to_hoist.push((block_id, instr_idx, op.clone()));
            }
        }

        if to_hoist.is_empty() {
            continue;
        }

        // Preheader only once there is something to hoist: no gratuitous CFG
        // mutation (edge splits) on loops LICM leaves alone.
        let Some(preheader) = find_or_create_preheader(cfg, *header, loop_blocks) else {
            continue;
        };

        // Build the guarded block first (with clones, in source order -- the
        // candidates are mutually independent, an instruction reading another
        // hoisted target fails the invariance check), and only then Nop the
        // originals: a failed insertion degrades to "no hoist", never to
        // dropped code.
        let hoisted_ops: Vec<IR> = to_hoist.iter().map(|(_, _, op)| op.clone()).collect();
        if insert_guarded_hoist(cfg, preheader, *header, cond, hoisted_ops).is_none() {
            continue;
        }
        for (block_id, instr_idx, _) in &to_hoist {
            if let Some(block) = cfg.get_block_mut(*block_id) {
                if let Some(IR::Op { opcode, .. }) = block.instructions.get_mut(*instr_idx) {
                    *opcode = IROpCode::Nop;
                }
            }
        }
        total_hoisted += to_hoist.len();
        inserted_guard = true;
    }

    // Guard insertion changed the graph: recompute so downstream consumers
    // (reconstruction's merge search, a rebuilt SSA) see consistent dominators.
    if inserted_guard {
        compute_dominators(cfg);
    }

    LICMResult { hoisted: total_hoisted }
}

/// The loop condition if `header` is a `while` header, `None` for `for` loops
/// (their guard would need a non-consuming emptiness test) and anything else.
fn while_condition(cfg: &ControlFlowGraph, header: usize) -> Option<IR> {
    let block = cfg.blocks.get(&header)?;
    let is_while = block
        .instructions
        .iter()
        .any(|op| node_opcode(op) == Some(IROpCode::OpWhile));
    if is_while { block.condition.clone() } else { None }
}

/// Whether an expression is pure: refs and literals, composed through pure
/// opcodes only. Anything else (calls, attribute access, unknown nodes) is
/// impure -- evaluating it twice could be observed.
/// Whether the IR subtree holds a `break`/`continue`/`return` that would
/// exit the CURRENT loop early. Descends through opaque regions (match arms,
/// if blocks). A nested loop keeps its own break/continue but a `return`
/// inside it still exits the whole function; lambda/function bodies stop the
/// walk entirely (the parser rejects loop control across that boundary).
fn contains_opaque_loop_control(ir: &IR) -> bool {
    fn walk(ir: &IR, in_current_loop: bool) -> bool {
        match ir {
            IR::Op {
                opcode, args, kwargs, ..
            } => match opcode {
                IROpCode::OpBreak | IROpCode::OpContinue => in_current_loop,
                IROpCode::OpReturn => true,
                IROpCode::OpWhile | IROpCode::OpFor => {
                    args.iter().any(|a| walk(a, false)) || kwargs.iter().any(|(_, v)| walk(v, false))
                }
                IROpCode::OpLambda | IROpCode::FnDef => false,
                _ => {
                    args.iter().any(|a| walk(a, in_current_loop))
                        || kwargs.iter().any(|(_, v)| walk(v, in_current_loop))
                }
            },
            IR::Tuple(items) | IR::List(items) => items.iter().any(|i| walk(i, in_current_loop)),
            _ => false,
        }
    }
    walk(ir, true)
}

/// Whether evaluating this IR subtree runs a call: a plain call, a method call
/// (`IR::Call` whose `func` is a `GetAttr`), or a broadcast of a user function.
/// Such an invocation can read any free name through a captured closure -- late
/// binding resolves a closure's globals at the call, not where it was built --
/// a read invisible to the SSA use sets. Lambda/function-definition bodies are
/// NOT traversed: their calls run at the eventual call site, not where the
/// closure is built.
fn contains_call(ir: &IR) -> bool {
    match ir {
        IR::Call { .. } | IR::Broadcast { .. } => true,
        IR::Op {
            opcode, args, kwargs, ..
        } => {
            !matches!(opcode, IROpCode::OpLambda | IROpCode::FnDef)
                && (args.iter().any(contains_call) || kwargs.iter().any(|(_, v)| contains_call(v)))
        }
        IR::Program(items) | IR::List(items) | IR::Tuple(items) | IR::Set(items) => items.iter().any(contains_call),
        IR::Dict(pairs) => pairs.iter().any(|(k, v)| contains_call(k) || contains_call(v)),
        IR::Slice { start, stop, step } => contains_call(start) || contains_call(stop) || contains_call(step),
        IR::PatternLiteral(inner) => contains_call(inner),
        IR::PatternOr(items) | IR::PatternTuple(items) => items.iter().any(contains_call),
        _ => false,
    }
}

/// Collect the names every `SetLocals` under this IR subtree assigns (opaque
/// match arms). Descends everywhere except lambda/function bodies -- their
/// assignments are local to the closure frame; nested loops inside an arm DO
/// assign the enclosing scope, so the walk crosses them.
fn collect_opaque_def_names(ir: &IR, out: &mut HashSet<String>) {
    match ir {
        IR::Op {
            opcode, args, kwargs, ..
        } => {
            if matches!(opcode, IROpCode::OpLambda | IROpCode::FnDef) {
                return;
            }
            if *opcode == IROpCode::SetLocals {
                if let Some(target) = args.first() {
                    collect_target_names(target, out);
                }
            }
            for a in args {
                collect_opaque_def_names(a, out);
            }
            for (_, v) in kwargs {
                collect_opaque_def_names(v, out);
            }
        }
        IR::Tuple(items) | IR::List(items) => {
            for i in items {
                collect_opaque_def_names(i, out);
            }
        }
        _ => {}
    }
}

/// Names inside a `SetLocals` target (`Ref`, `Identifier`, tuple of them).
fn collect_target_names(ir: &IR, out: &mut HashSet<String>) {
    match ir {
        IR::Ref(name, ..) | IR::Identifier(name) => {
            out.insert(name.clone());
        }
        IR::String(s) => {
            out.insert(s.clone());
        }
        IR::Tuple(items) | IR::List(items) => {
            for i in items {
                collect_target_names(i, out);
            }
        }
        _ => {}
    }
}

/// Whether the expression reads or names any of `names` (`Ref`/`Identifier`).
fn mentions_any_name(ir: &IR, names: &HashSet<String>) -> bool {
    match ir {
        IR::Ref(name, ..) | IR::Identifier(name) => names.contains(name),
        IR::Op { args, kwargs, .. } => {
            args.iter().any(|a| mentions_any_name(a, names)) || kwargs.iter().any(|(_, v)| mentions_any_name(v, names))
        }
        IR::Tuple(items) | IR::List(items) => items.iter().any(|i| mentions_any_name(i, names)),
        _ => false,
    }
}

fn expr_is_pure(ir: &IR, pure_ops: &HashSet<i32>) -> bool {
    match ir {
        IR::Ref(..) | IR::Int(_) | IR::Float(_) | IR::Bool(_) | IR::String(_) | IR::None => true,
        IR::Op {
            opcode, args, kwargs, ..
        } => {
            pure_ops.contains(&(*opcode as i32))
                && args.iter().all(|a| expr_is_pure(a, pure_ops))
                && kwargs.iter().all(|(_, v)| expr_is_pure(v, pure_ops))
        }
        _ => false,
    }
}

/// The SSA value defined by the instruction at (block, instr_idx), if any.
fn def_value_at(ssa: &SSAContext, block_id: usize, instr_idx: usize) -> Option<SSAValue> {
    ssa.value_defs.iter().find_map(|(v, d)| match d {
        ValueDef::Instruction { block, instr_idx: idx } if *block == block_id && *idx == instr_idx => Some(*v),
        _ => None,
    })
}

/// Whether every in-loop use of `value`'s variable reads exactly `value`.
///
/// Instruction uses are checked through `resolve_trivial` (a trivial header phi
/// stands for its unique operand). Block conditions are not tracked in
/// `instruction_uses`; a condition mentioning the variable reads the definition
/// live at the block's exit (`current_defs`, the same assumption
/// `rename_conditions` relies on, oracle-checked) -- if that is not `value`,
/// hoisting would change what the condition observes (e.g. a nested loop
/// testing the pre-loop value on its first iteration).
fn value_reaches_all_loop_uses(
    cfg: &ControlFlowGraph,
    ssa: &SSAContext,
    value: SSAValue,
    loop_blocks: &HashSet<usize>,
) -> bool {
    let var = value.var;
    let Some(var_name) = ssa.vars.name(var).map(|s| s.to_string()) else {
        return false;
    };
    let mentions_var = |ir: &IR| {
        let mut found = false;
        collect_refs(ir, &mut |n| {
            if n == var_name {
                found = true;
            }
        });
        found
    };

    for &b in loop_blocks {
        let Some(block) = cfg.blocks.get(&b) else { continue };

        // A loop-header block holds only its op-preserved loop op, whose nested
        // body duplicates the CFG body blocks (checked on their own above/below)
        // -- its recorded instruction uses are artefacts resolved at the header,
        // not real reads. What the header actually evaluates is its condition
        // (`while`, checked via block.condition below) or its iterable (`for`,
        // re-evaluated on every entry: refuse on any mention, conservatively).
        let loop_op = block
            .instructions
            .iter()
            .find(|op| matches!(node_opcode(op), Some(IROpCode::OpWhile | IROpCode::OpFor)));
        if let Some(op) = loop_op {
            if node_opcode(op) == Some(IROpCode::OpFor) {
                if let IR::Op { args, .. } = op {
                    if args.get(1).is_some_and(&mentions_var) {
                        return false;
                    }
                }
            }
        } else {
            for instr_idx in 0..block.instructions.len() {
                for w in ssa.get_uses(b, instr_idx) {
                    if w.var == var && ssa.resolve_trivial(*w) != value {
                        return false;
                    }
                }
            }
        }

        if let Some(cond) = &block.condition {
            if mentions_var(cond) {
                let reaches = ssa
                    .blocks
                    .get(&b)
                    .and_then(|info| info.current_defs.get(&var))
                    .is_some_and(|w| ssa.resolve_trivial(*w) == value);
                if !reaches {
                    return false;
                }
            }
        }
    }
    true
}

/// Insert the guarded hoist block between `preheader` and `header`:
///
/// ```text
/// preheader -> guard --true--> hoist(hoisted ops) --> header
///                    --false--> skip(empty)       --> header
/// ```
///
/// This is exactly the builder's if-without-else shape (both arms are
/// dedicated blocks falling through to the merge), so `find_merge_point`
/// resolves the header as the merge and reconstruction emits
/// `if cond { hoisted }` followed by the untouched loop.
fn insert_guarded_hoist(
    cfg: &mut ControlFlowGraph,
    preheader: usize,
    header: usize,
    cond: IR,
    hoisted: Vec<IR>,
) -> Option<()> {
    // The preheader -> header edge (the preheader is dedicated: single succ).
    let edge_ph = cfg
        .blocks
        .get(&preheader)?
        .successors
        .iter()
        .copied()
        .find(|&e| cfg.edges.get(e).map(|ed| ed.target == header).unwrap_or(false))?;

    // preheader -> guard -> header, then guard -> hoist -> header; the
    // guard->hoist edge is the retargeted Fallthrough, retyped to the true arm.
    let guard = cfg.split_edge(edge_ph, "licm_guard")?;
    let edge_guard_out = cfg.blocks.get(&guard)?.successors.first().copied()?;
    let hoist_block = cfg.split_edge(edge_guard_out, "licm_hoist")?;
    if let Some(e) = cfg.edges.get_mut(edge_guard_out) {
        e.edge_type = EdgeType::ConditionalTrue;
    }

    // Empty false arm, mirroring the builder's dedicated else block.
    let skip = cfg.create_block("licm_skip");
    cfg.add_edge(guard, skip, EdgeType::ConditionalFalse);
    cfg.add_edge(skip, header, EdgeType::Fallthrough);

    cfg.get_block_mut(guard)?.set_condition(cond);
    let hb = cfg.get_block_mut(hoist_block)?;
    for op in hoisted {
        hb.instructions.push(op);
    }
    Some(())
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

/// Whether a SetLocals' single target has exactly one instruction definition in
/// the loop. A variable defined more than once in the loop (redefined each
/// iteration) must not have one of its defs hoisted out.
fn target_single_def_in_loop(ssa: &SSAContext, op: &IR, loop_blocks: &HashSet<usize>) -> bool {
    let targets = setlocals_names(op);
    let [target] = targets.as_slice() else {
        return false; // unpacking or no target: don't hoist
    };
    let Some(var_id) = ssa.vars.id(target) else {
        return false;
    };
    let count = ssa
        .value_defs
        .iter()
        .filter(|(v, d)| {
            v.var == var_id && matches!(d, ValueDef::Instruction { block, .. } if loop_blocks.contains(block))
        })
        .count();
    count == 1
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
    // Resolve trivial header phis: a variable unchanged in the loop gets a phi
    // defined *in* a loop block, but it stands for its pre-loop value, so a use
    // of it is still invariant. Without this, `t = k * 2` with `k` untouched
    // reads the trivial phi `k` and looks loop-dependent.
    uses.iter().all(|val| !loop_defs.contains(&ssa.resolve_trivial(*val)))
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
