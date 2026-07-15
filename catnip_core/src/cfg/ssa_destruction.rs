// FILE: catnip_core/src/cfg/ssa_destruction.rs
//! SSA destruction: convert from SSA form back to conventional form.
//!
//! For each non-trivial block parameter (phi), insert SetLocals copies
//! at the end of each predecessor block.

use super::graph::ControlFlowGraph;
use super::liveness::{Liveness, compute_liveness};
use super::ssa::{SSAContext, SSAValue, ValueDef};
use super::ssa_builder::{name_of, node_opcode};
use crate::ir::{IR, IROpCode};
use crate::semantic::passes::rewrite_refs;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Reconstructed name chosen for each SSA value during destruction.
///
/// A version keeps its variable's bare name unless it must coexist with another
/// version of the same variable, in which case it gets a distinct name. See
/// [`default_naming`] for the identity (inert) case.
pub type Naming = HashMap<SSAValue, String>;

/// The naming under which destruction is a no-op: every SSA value keeps its
/// variable's bare name. Because Catnip's structured control flow makes
/// op-preservation already a valid out-of-SSA, renaming and phi materialization
/// against this naming leave the IR unchanged (identity mode). A pass that wants
/// two versions live supplies a naming that separates them.
pub fn default_naming(ssa: &SSAContext) -> Naming {
    ssa.value_defs
        .keys()
        .map(|&v| (v, ssa.vars.name(v.var).unwrap_or("").to_string()))
        .collect()
}

/// Maximal-separation naming: every SSA value gets a unique `var#version` name.
///
/// Turns destruction into a full, faithful out-of-SSA (every phi becomes a real
/// copy, every use references its exact version). Used as the synthetic-
/// interference oracle: the reconstructed program must still execute identically
/// to the source, which exercises renaming + phi materialization + the solver on
/// every construct without needing a real optimization pass.
pub fn maximal_naming(ssa: &SSAContext) -> Naming {
    ssa.value_defs
        .keys()
        .map(|&v| {
            let c = ssa.resolve_trivial(v);
            let base = ssa.vars.name(c.var).unwrap_or("");
            (v, format!("{base}__v{}", c.version))
        })
        .collect()
}

/// Rewrite every definition and use in the CFG to its versioned name under
/// `naming`. Uses are matched to SSA values by replaying the builder's use order
/// (`scan_uses` / `collect_refs`); the `debug_assert` guards against zip drift,
/// the one silent-miscompile risk here.
///
/// Note: branch conditions and loop-header operands are not renamed yet (they
/// live outside `instruction_uses`); harmless under [`default_naming`] (inert),
/// required once phi materialization activates the versioned paths.
pub fn rename_versioned(cfg: &mut ControlFlowGraph, ssa: &SSAContext, naming: &Naming) {
    let block_ids: Vec<usize> = cfg.blocks.keys().copied().collect();
    for b in block_ids {
        let count = cfg.blocks.get(&b).map(|bl| bl.instructions.len()).unwrap_or(0);
        for i in 0..count {
            if let Some(block) = cfg.blocks.get_mut(&b) {
                if let Some(op) = block.instructions.get_mut(i) {
                    rename_instruction(op, b, i, ssa, naming);
                }
            }
        }
    }
}

/// Rename one instruction's uses (Ref names) then its defs (SetLocals targets).
fn rename_instruction(op: &mut IR, block: usize, instr_idx: usize, ssa: &SSAContext, naming: &Naming) {
    let uses = ssa.get_uses(block, instr_idx).to_vec();
    let idx = Cell::new(0usize);
    // Rename the k-th var-Ref to the name of the k-th recorded SSA value. The
    // `vars.id` filter mirrors scan_uses: names that are not SSA vars (builtins,
    // globals) are skipped and consume no slot.
    let mut rename_use = |name: &mut String| {
        if ssa.vars.id(name).is_some() {
            let i = idx.get();
            if let Some(v) = uses.get(i) {
                if let Some(new) = naming.get(v) {
                    *name = new.clone();
                }
            }
            idx.set(i + 1);
        }
    };

    // Same subtree selection as scan_uses: SetLocals reads only its RHS, every
    // other op reads all args and kwargs.
    if node_opcode(op) == Some(IROpCode::SetLocals) {
        if let IR::Op { args, .. } = op {
            if let Some(rhs) = args.get_mut(1) {
                rewrite_refs(rhs, &mut rename_use);
            }
        }
    } else {
        match &mut *op {
            IR::Op { args, kwargs, .. } => {
                for a in args.iter_mut() {
                    rewrite_refs(a, &mut rename_use);
                }
                for (_, v) in kwargs.iter_mut() {
                    rewrite_refs(v, &mut rename_use);
                }
            }
            other => rewrite_refs(other, &mut rename_use),
        }
    }
    debug_assert_eq!(
        idx.get(),
        uses.len(),
        "rename_versioned: use zip drifted at ({block}, {instr_idx})"
    );

    if node_opcode(op) == Some(IROpCode::SetLocals) {
        rename_setlocals_targets(op, block, instr_idx, ssa, naming);
    }
}

/// Rename the target name(s) of a SetLocals to the versioned name of the value
/// it defines at this program point.
fn rename_setlocals_targets(op: &mut IR, block: usize, instr_idx: usize, ssa: &SSAContext, naming: &Naming) {
    let IR::Op { args, .. } = op else { return };
    let Some(target) = args.get_mut(0) else { return };
    match target {
        IR::Tuple(items) => {
            for it in items.iter_mut() {
                rename_one_target(it, block, instr_idx, ssa, naming);
            }
        }
        other => rename_one_target(other, block, instr_idx, ssa, naming),
    }
}

fn rename_one_target(node: &mut IR, block: usize, instr_idx: usize, ssa: &SSAContext, naming: &Naming) {
    let Some(cur) = name_of(node) else { return };
    let Some(var_id) = ssa.vars.id(&cur) else { return };
    // The value this instruction defines for this variable.
    let defined = ssa.value_defs.iter().find_map(|(v, d)| match d {
        ValueDef::Instruction { block: b, instr_idx: i } if v.var == var_id && *b == block && *i == instr_idx => {
            Some(*v)
        }
        _ => None,
    });
    if let Some(v) = defined {
        if let Some(new) = naming.get(&v) {
            set_name(node, new.clone());
        }
    }
}

/// Set the variable name carried by a target node (Ref / Identifier / String).
fn set_name(node: &mut IR, new: String) {
    match node {
        IR::Ref(name, _, _) | IR::Identifier(name) | IR::String(name) => *name = new,
        _ => {}
    }
}

/// Scratch location name for breaking phi-copy cycles. Begins with a character no
/// source or versioned name uses, so it never collides under the naming schemes
/// here. A real optimization consumer must supply a guaranteed-fresh name.
const PHI_SCRATCH: &str = "__phi_tmp";

/// Rename branch conditions to their versioned names.
///
/// A condition is evaluated at its block's exit, so a variable there reads the
/// definition reaching the block end: `current_defs[block][var]` after SSA
/// construction. That assumption is checked empirically by the execution oracle
/// (`catnip_vm`), not assumed sound.
pub fn rename_conditions(cfg: &mut ControlFlowGraph, ssa: &SSAContext, naming: &Naming) {
    let block_ids: Vec<usize> = cfg.blocks.keys().copied().collect();
    for b in block_ids {
        let Some(defs) = ssa.blocks.get(&b).map(|info| &info.current_defs) else {
            continue;
        };
        let Some(block) = cfg.blocks.get_mut(&b) else { continue };
        let Some(cond) = block.condition.as_mut() else { continue };
        rewrite_refs(cond, &mut |name| {
            if let Some(var_id) = ssa.vars.id(name) {
                if let Some(val) = defs.get(&var_id) {
                    if let Some(new) = naming.get(val) {
                        *name = new.clone();
                    }
                }
            }
        });
    }
}

/// Build a `dst = src` SetLocals copy in the canonical layout the parser emits:
/// `args[0]` is a `Tuple([Ref(dst)])`, not a bare identifier -- the scope-leak
/// path keys on that shape, so a bare target would fail to propagate.
pub(crate) fn copy_stmt(dst: &str, src: &str) -> IR {
    IR::op(
        IROpCode::SetLocals,
        vec![
            IR::Tuple(vec![IR::Ref(dst.to_string(), -1, -1)]),
            IR::Ref(src.to_string(), -1, -1),
            IR::Bool(false),
        ],
    )
}

/// Materialize live phis as parallel-copy batches on predecessor edges.
///
/// Each live block parameter imposes `name(phi) := name(operand)` on every
/// incoming edge. Per edge the batch is sequenced by
/// [`sequentialize_parallel_copies`] (identity copies dropped, cycles broken with
/// [`PHI_SCRATCH`]) and emitted as SetLocals at the predecessor's end; a critical
/// edge is split first. Under [`default_naming`] every copy is `x := x` and
/// vanishes, so materialization is inert.
///
/// A phi whose variable is not live-in at its block is dead: its value is never
/// used before redefinition (e.g. a loop-local reset at the top of the body), so
/// its preheader operand may be undefined. Such phis are skipped -- the liveness
/// gate (Ari's choice: real liveness, `compute_liveness`) that closes the
/// loop-local undefined-operand hole. Operands that did not resolve (`None`) are
/// also skipped.
pub fn materialize_phis(cfg: &mut ControlFlowGraph, ssa: &SSAContext, naming: &Naming, liveness: &Liveness) {
    let block_ids: Vec<usize> = cfg.blocks.keys().copied().collect();
    for b in block_ids {
        let live_in = liveness.live_in.get(&b);
        // (dst name, src name per predecessor index) for each live phi whose
        // variable is actually live at this block.
        let live: Vec<(String, Vec<Option<String>>)> = ssa
            .get_live_params(b)
            .iter()
            .filter(|p| {
                let name = ssa.vars.name(p.value.var).unwrap_or("");
                live_in.map(|s| s.contains(name)).unwrap_or(false)
            })
            .map(|p| {
                let dst = naming.get(&p.value).cloned().unwrap_or_default();
                let srcs = p
                    .incoming
                    .iter()
                    .map(|opt| opt.as_ref().and_then(|v| naming.get(v).cloned()))
                    .collect();
                (dst, srcs)
            })
            .collect();
        if live.is_empty() {
            continue;
        }

        let pred_edges: Vec<usize> = cfg.blocks.get(&b).map(|bl| bl.predecessors.clone()).unwrap_or_default();
        for (i, &edge_id) in pred_edges.iter().enumerate() {
            let mut batch: Vec<(String, String)> = Vec::new();
            for (dst, srcs) in &live {
                if let Some(Some(src)) = srcs.get(i) {
                    batch.push((dst.clone(), src.clone()));
                }
            }
            if batch.is_empty() {
                continue;
            }
            let seq = sequentialize_parallel_copies(&batch, PHI_SCRATCH.to_string());

            // Emit at the predecessor's end, splitting a critical edge first.
            let Some(pred) = cfg.edges.get(edge_id).map(|e| e.source) else {
                continue;
            };
            let critical = cfg.blocks.get(&pred).map(|bl| bl.successors.len() > 1).unwrap_or(false);
            let target = if critical {
                cfg.split_edge(edge_id, "phi_edge").unwrap_or(pred)
            } else {
                pred
            };
            if let Some(tb) = cfg.blocks.get_mut(&target) {
                for (dst, src) in seq {
                    tb.instructions.push(copy_stmt(&dst, &src));
                }
            }
        }
    }
}

/// Real SSA destruction under an explicit naming: rename defs/uses and branch
/// conditions to versioned names, then materialize live phis as parallel copies.
///
/// Inert under [`default_naming`]; a full out-of-SSA under a naming that
/// separates versions. [`maximal_naming`] drives the synthetic execution oracle.
pub fn destroy_ssa_versioned(cfg: &mut ControlFlowGraph, ssa: &SSAContext, naming: &Naming) {
    // Liveness over original names, before rename mutates the instructions.
    let liveness = compute_liveness(cfg);
    rename_versioned(cfg, ssa, naming);
    rename_conditions(cfg, ssa, naming);
    materialize_phis(cfg, ssa, naming, &liveness);
}

/// Sequentialize a batch of *parallel* copies into ordered sequential copies
/// with the same effect, using `scratch` to break cycles (the swap / lost-copy
/// problem, Briggs et al. 1998; Boissinot et al. 2009).
///
/// A parallel copy batch writes every `dst` from the *initial* value of its
/// `src` simultaneously: `(a <- b, b <- a)` swaps `a` and `b`. Emitting those
/// naively in sequence corrupts one value (`a <- b` then `b <- a` leaves both
/// equal to the old `b`). This routine emits copies in an order where each
/// destination is written only once its old value is no longer needed, and when
/// only cycles remain it saves one cycle node into `scratch` to break the
/// dependency.
///
/// Contract:
/// - all `dst` in `copies` are distinct (SSA gives one definition per phi);
/// - `scratch` is a location distinct from every `dst` and `src`;
/// - identity copies (`dst == src`) are dropped;
/// - the returned sequence, executed top to bottom as `dst := src`, leaves every
///   real location holding the value the parallel batch would have given it.
///
/// The location type `L` is generic (variable id, SSA value, register name): the
/// algorithm is graph-shaped, not tied to the IR. `Clone` (not `Copy`) so string
/// names work as locations.
pub fn sequentialize_parallel_copies<L: Clone + Eq + Hash>(copies: &[(L, L)], scratch: L) -> Vec<(L, L)> {
    // Pending copies as (dst, src); drop no-op self copies up front.
    let mut pending: Vec<(L, L)> = copies.iter().filter(|(d, s)| d != s).cloned().collect();

    debug_assert!(
        {
            let mut seen = HashSet::new();
            pending.iter().all(|(d, _)| seen.insert(d.clone()))
        },
        "sequentialize_parallel_copies: destinations must be distinct"
    );
    debug_assert!(
        pending.iter().all(|(d, s)| *d != scratch && *s != scratch),
        "sequentialize_parallel_copies: scratch must not collide with any dst/src"
    );

    let mut seq = Vec::with_capacity(pending.len() + 1);

    while !pending.is_empty() {
        // A destination is free to overwrite iff no pending copy still reads it
        // as a source: its old value is dead, and every source it depends on is
        // still live (any copy overwriting that source would keep this one
        // pending, so it can't have run yet).
        let live_srcs: HashSet<L> = pending.iter().map(|(_, s)| s.clone()).collect();

        if let Some(pos) = pending.iter().position(|(d, _)| !live_srcs.contains(d)) {
            let copy = pending.remove(pos);
            seq.push(copy);
        } else {
            // Every remaining destination is also a source: only cycles are
            // left. Save one node's live value into `scratch` and redirect its
            // readers there, which frees the node for the next iteration.
            let cycle_node = pending[0].0.clone();
            seq.push((scratch.clone(), cycle_node.clone()));
            for (_, src) in pending.iter_mut() {
                if *src == cycle_node {
                    *src = scratch.clone();
                }
            }
        }
    }

    seq
}

/// SSA destruction is currently a no-op: reconstruction reads each block's
/// original (op-preserved) instructions, so the source assignments already
/// carry every value across joins and the phis have no instruction-level
/// counterpart to undo.
///
/// The former identity-copy form (`var = var` per phi operand) could never
/// move a value and broke at loop preheaders for variables first defined in a
/// loop body; see `wip/CFG_SSA_REWIRING.md` (Phase 2). A real destruction pass
/// (temporaries + versioning) belongs here once Phase 3 activates the
/// inter-block passes that move values across edges (swap / lost-copy).
pub fn destroy_ssa(_cfg: &mut ControlFlowGraph, _ssa: &SSAContext) {}

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

    // --- Parallel-copy sequentialization ------------------------------------

    /// Reserved scratch location for the tests (kept apart from the 0..K var ids).
    const SCRATCH: usize = 100;

    /// Execute a sequence of `dst := src` copies over a symbolic state where
    /// each location initially holds its own id. Returns the final state map.
    fn run_seq(seq: &[(usize, usize)], locations: usize) -> Vec<usize> {
        let mut state: Vec<usize> = (0..=locations.max(SCRATCH)).collect();
        for &(dst, src) in seq {
            state[dst] = state[src];
        }
        state
    }

    /// Check that `seq` realizes the parallel semantics of `copies`: every real
    /// destination ends holding its source's *initial* value, and every location
    /// that is not a destination is left untouched.
    fn assert_realizes_parallel(copies: &[(usize, usize)], seq: &[(usize, usize)], k: usize) {
        let state = run_seq(seq, k);
        let dsts: HashSet<usize> = copies.iter().map(|(d, _)| *d).collect();
        for &(dst, src) in copies {
            // src's initial value is `src` (identity init), so dst must hold `src`.
            assert_eq!(
                state[dst], src,
                "dst {dst} should hold initial value of src {src}; copies={copies:?} seq={seq:?}"
            );
        }
        for (loc, &held) in state.iter().enumerate().take(k) {
            if !dsts.contains(&loc) {
                assert_eq!(
                    held, loc,
                    "non-destination {loc} was clobbered; copies={copies:?} seq={seq:?}"
                );
            }
        }
    }

    #[test]
    fn test_pcopy_empty() {
        let seq = sequentialize_parallel_copies::<usize>(&[], SCRATCH);
        assert!(seq.is_empty());
    }

    #[test]
    fn test_pcopy_identity_dropped() {
        // (a <- a) is a no-op and must not appear in the output.
        let seq = sequentialize_parallel_copies(&[(0, 0), (1, 1)], SCRATCH);
        assert!(seq.is_empty());
    }

    #[test]
    fn test_pcopy_swap_uses_scratch() {
        // (a <- b, b <- a): a 2-cycle. Requires the scratch temp.
        let copies = [(0, 1), (1, 0)];
        let seq = sequentialize_parallel_copies(&copies, SCRATCH);
        assert!(
            seq.iter().any(|&(d, _)| d == SCRATCH),
            "swap must spill to scratch: {seq:?}"
        );
        assert_realizes_parallel(&copies, &seq, 2);
    }

    #[test]
    fn test_pcopy_three_cycle() {
        // (a<-b, b<-c, c<-a): a 3-cycle, one scratch spill.
        let copies = [(0, 1), (1, 2), (2, 0)];
        let seq = sequentialize_parallel_copies(&copies, SCRATCH);
        assert_eq!(seq.iter().filter(|&&(d, _)| d == SCRATCH).count(), 1);
        assert_realizes_parallel(&copies, &seq, 3);
    }

    #[test]
    fn test_pcopy_chain_no_scratch() {
        // (b<-a, c<-b): acyclic; must sequence leaf-first, no scratch needed.
        let copies = [(1, 0), (2, 1)];
        let seq = sequentialize_parallel_copies(&copies, SCRATCH);
        assert!(
            seq.iter().all(|&(d, _)| d != SCRATCH),
            "chain needs no scratch: {seq:?}"
        );
        assert_realizes_parallel(&copies, &seq, 3);
    }

    #[test]
    fn test_pcopy_fanout() {
        // (b<-a, c<-a): a read by two dsts, acyclic.
        let copies = [(1, 0), (2, 0)];
        let seq = sequentialize_parallel_copies(&copies, SCRATCH);
        assert_realizes_parallel(&copies, &seq, 3);
    }

    #[test]
    fn test_pcopy_two_independent_swaps() {
        // Two disjoint 2-cycles reusing the single scratch: each must fully
        // drain (scratch consumed) before the next is broken.
        let copies = [(0, 1), (1, 0), (2, 3), (3, 2)];
        let seq = sequentialize_parallel_copies(&copies, SCRATCH);
        assert_realizes_parallel(&copies, &seq, 4);
    }

    #[test]
    fn test_pcopy_tail_off_cycle() {
        // (a<-b, b<-a) cycle plus (e<-a) tail: e must read a's old value,
        // emitted before the cycle overwrites a.
        let copies = [(0, 1), (1, 0), (4, 0)];
        let seq = sequentialize_parallel_copies(&copies, SCRATCH);
        assert_realizes_parallel(&copies, &seq, 5);
    }

    /// Exhaustive oracle: over K locations, enumerate every parallel-copy batch
    /// (each var is either not a destination, or a destination reading any of the
    /// K sources) and assert the sequentialization realizes the parallel
    /// semantics. K=4 -> 5^4 = 625 batches; covers every chain / fan-out / cycle
    /// / swap / self / mixed / multi-cycle shape.
    fn exhaustive_for_k(k: usize) {
        // choices[var] in 0..=k: 0 = not a dst, i>=1 = dst reading src (i-1).
        let mut choice = vec![0usize; k];
        loop {
            let copies: Vec<(usize, usize)> = (0..k).filter(|&v| choice[v] != 0).map(|v| (v, choice[v] - 1)).collect();

            let seq = sequentialize_parallel_copies(&copies, SCRATCH);
            assert_realizes_parallel(&copies, &seq, k);

            // A sound sequentialization spills to scratch only when it must: never
            // more than one live scratch save per independent cycle. Bound it by
            // the destination count as a cheap upper sanity check.
            let spills = seq.iter().filter(|&&(d, _)| d == SCRATCH).count();
            assert!(spills <= copies.len(), "too many scratch spills: {seq:?}");

            // odometer increment over base (k+1)
            let mut i = 0;
            loop {
                if i == k {
                    return;
                }
                choice[i] += 1;
                if choice[i] <= k {
                    break;
                }
                choice[i] = 0;
                i += 1;
            }
        }
    }

    #[test]
    fn test_pcopy_exhaustive_k4() {
        exhaustive_for_k(4);
    }

    #[test]
    fn test_pcopy_exhaustive_k5() {
        exhaustive_for_k(5);
    }

    #[test]
    fn test_pcopy_exhaustive_k6() {
        // k=6 admits three disjoint 2-cycles (2+2+2): exercises reuse of the
        // single scratch across three cycle breaks, which k<=5 cannot reach.
        exhaustive_for_k(6);
    }

    // --- Versioned rename -----------------------------------------------------

    fn set_local(target: &str, rhs: IR) -> IR {
        IR::op(
            IROpCode::SetLocals,
            vec![IR::Identifier(target.into()), rhs, IR::Bool(false)],
        )
    }

    /// Linear CFG `entry { x = 5; y = x + 1 } -> exit`, in SSA form.
    fn linear_ssa() -> (ControlFlowGraph, SSAContext) {
        let mut cfg = ControlFlowGraph::new("t");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);
        {
            let b = cfg.get_block_mut(entry).unwrap();
            b.add_instruction(set_local("x", IR::Int(5)));
            b.add_instruction(set_local(
                "y",
                IR::op(IROpCode::Add, vec![IR::Ref("x".into(), -1, -1), IR::Int(1)]),
            ));
        }
        compute_dominators(&mut cfg);
        let ssa = SSABuilder::build(&cfg);
        (cfg, ssa)
    }

    #[test]
    fn test_rename_default_is_inert() {
        let (mut cfg, ssa) = linear_ssa();
        let before = cfg.blocks[&0].instructions.clone();
        rename_versioned(&mut cfg, &ssa, &default_naming(&ssa));
        assert_eq!(cfg.blocks[&0].instructions, before);
    }

    #[test]
    fn test_rename_versioned_single_block() {
        let (mut cfg, ssa) = linear_ssa();
        // x's value defined at (block 0, instr 0).
        let xv = *ssa
            .value_defs
            .iter()
            .find_map(|(v, d)| match d {
                ValueDef::Instruction { block: 0, instr_idx: 0 } if ssa.vars.name(v.var) == Some("x") => Some(v),
                _ => None,
            })
            .expect("x def");

        let mut naming = default_naming(&ssa);
        naming.insert(xv, "x0".to_string());
        rename_versioned(&mut cfg, &ssa, &naming);

        let instrs = &cfg.blocks[&0].instructions;
        // def target of `x = 5` renamed to x0
        match &instrs[0] {
            IR::Op { args, .. } => match &args[0] {
                IR::Identifier(n) | IR::Ref(n, _, _) => assert_eq!(n, "x0"),
                other => panic!("unexpected target {other:?}"),
            },
            other => panic!("expected SetLocals, got {other:?}"),
        }
        // use of x in `y = x + 1` renamed to x0
        let mut names = Vec::new();
        crate::semantic::passes::collect_refs(&instrs[1], &mut |n| names.push(n));
        assert!(names.contains(&"x0".to_string()), "use not renamed: {names:?}");
    }
}
