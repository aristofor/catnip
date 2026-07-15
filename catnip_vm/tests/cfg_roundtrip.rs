//! Round-trip harness for the (dormant) CFG engine.
//!
//! Real Catnip programs → IR → CFG → SSA → SSA destruction → reconstruction,
//! with the structural verifiers active at each stage. Part of the Phase 0
//! "filet" (`wip/CFG_SSA_REWIRING.md`): it exercises the freshly ported pure-Rust
//! CFG engine on actual parser output, so a reconstruction bug surfaces as a
//! loud, localized failure instead of silent miscompilation once the engine is
//! wired.
//!
//! Scope: structural verifiers AND an execution-level differential. The
//! structural tests check the reconstruction has the right shape; the
//! `execution_differential` test checks it has the right behaviour, running a
//! program and its CFG round-tripped copy through `PurePipeline::execute_ir` and
//! asserting the same value. The behavioural half caught two bugs the shape
//! checks missed: a malformed SSA-destruction copy (`var = (var,)` instead of
//! `var = var`) and post-`if` code duplicated into both branches.
//!
//! Categorization (a Phase 0 finding): the verifiers pass for every construct,
//! and reconstruction (`region.rs`) now round-trips every construct in the
//! corpus — linear, flat if/else, `while`, `for`, nested loops and `match`.
//!
//! Holes closed under this net: the `while` back-edge bug (reconstructing a loop
//! body re-entered `reconstruct_while` on the body tail, which has no condition —
//! fixed by emitting the tail's statements and stopping), and `for` (was rebuilt
//! as a `while`; now the loop kind and its operands are read from the loop op the
//! builder stores in the header). Nested loops then fell out for free. All were
//! masked before because `make compile` builds in release (debug_assert off) and
//! the old Python tests only checked block counts.
//!
//! Caveat on `match`: it round-trips by *op preservation*, not true
//! reconstruction (the builder leaves the original `OpMatch` in the header and we
//! emit it). Fine for un-optimized IR; at wiring this must become real per-arm
//! reconstruction, or optimizations on the arm blocks would be dropped.

use catnip_core::cfg::analysis::compute_dominators;
use catnip_core::cfg::ssa_builder::SSABuilder;
use catnip_core::cfg::ssa_destruction::{destroy_ssa, destroy_ssa_versioned, maximal_naming};
use catnip_core::cfg::ssa_dse::{apply_dse, global_dse};
use catnip_core::cfg::ssa_gvn::{gvn, materialize_gvn, materialize_gvn_versioned};
use catnip_core::cfg::ssa_licm::licm;
use catnip_core::cfg::{IRCFGBuilder, reconstruct_from_cfg};
use catnip_core::ir::{IR, IROpCode};
use catnip_vm::pipeline::PurePipeline;

/// Parse source into its list of top-level IR statements (pre-semantic).
fn statements(source: &str) -> Vec<IR> {
    let mut p = PurePipeline::new().unwrap();
    let ir = p
        .parse_to_ir(source, false)
        .unwrap_or_else(|e| panic!("parse failed for {source:?}: {e}"));
    match ir {
        IR::Program(stmts) => stmts,
        other => vec![other],
    }
}

/// Run the full dormant CFG pipeline on a snippet, asserting the verifiers pass
/// at each stage. Returns the reconstructed IR statements.
fn round_trip(source: &str) -> Vec<IR> {
    let stmts = statements(source);

    let mut cfg = IRCFGBuilder::new("rt").build(stmts);
    assert!(cfg.verify().is_ok(), "CFG malformed for {source:?}: {:?}", cfg.verify());

    compute_dominators(&mut cfg);

    let ssa = SSABuilder::build(&cfg);
    assert!(
        ssa.verify(&cfg).is_ok(),
        "SSA invalid for {source:?}: {:?}",
        ssa.verify(&cfg)
    );

    destroy_ssa(&mut cfg, &ssa);
    assert!(
        cfg.verify().is_ok(),
        "CFG malformed after SSA destruction for {source:?}: {:?}",
        cfg.verify()
    );

    reconstruct_from_cfg(&cfg)
}

fn assert_round_trips(corpus: &[&str]) {
    for src in corpus {
        let out = round_trip(src);
        assert!(!out.is_empty(), "reconstruction empty for {src:?}");
    }
}

/// Linear statement sequences: build → SSA → destruct → reconstruct cleanly.
#[test]
fn round_trip_linear() {
    assert_round_trips(&[
        "x = 1",
        "x = 1\ny = 2\nz = x + y",
        "a = 10\nb = a + 5\nc = b * 2\nd = c - a",
    ]);
}

/// Flat if/else: condition preserved on the branch block, both arms linear.
#[test]
fn round_trip_if_else() {
    assert_round_trips(&[
        "x = 5\nif x > 0 { y = 1 } else { y = 2 }",
        "a = 1\nb = 2\nif a < b { c = a } else { c = b }\nd = c + 1",
    ]);
}

/// Simple while: reconstructed via the loop-header path. The body tail's back
/// edge now stops cleanly instead of re-entering `reconstruct_while` on a block
/// with no condition.
#[test]
fn round_trip_while() {
    assert_round_trips(&[
        "x = 0\nwhile x < 10 { x = x + 1 }",
        "n = 100\ns = 0\nwhile n > 0 { s = s + n\nn = n - 1 }",
    ]);
}

/// The reconstructed `while` is structurally correct, not merely non-panicking:
/// exactly one `OpWhile` at top level, carrying a non-empty body block.
#[test]
fn round_trip_while_structure() {
    let out = round_trip("x = 0\nwhile x < 10 { x = x + 1 }");

    let whiles: Vec<&IR> = out
        .iter()
        .filter(|s| {
            matches!(
                s,
                IR::Op {
                    opcode: IROpCode::OpWhile,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(whiles.len(), 1, "expected exactly one reconstructed while: {out:?}");

    let IR::Op { args, .. } = whiles[0] else { unreachable!() };
    let body = args.get(1).expect("while must have a body");
    let non_empty_body = matches!(body, IR::Op { opcode: IROpCode::OpBlock, args, .. } if !args.is_empty());
    assert!(non_empty_body, "while body should hold the increment: {body:?}");
}

/// `for` loops: reconstructed as a `for` (not a `while`) by reading the loop op
/// the builder stored in the header, recovering target + iterable.
#[test]
fn round_trip_for() {
    assert_round_trips(&[
        "for i in range(10) { y = i }",
        "total = 0\nfor n in items { total = total + n }",
    ]);
}

/// The reconstructed `for` keeps its identity: one `OpFor` (not an `OpWhile`),
/// carrying target, iterable and a non-empty body.
#[test]
fn round_trip_for_structure() {
    let out = round_trip("for i in range(10) { y = i }");

    let fors: Vec<&IR> = out
        .iter()
        .filter(|s| {
            matches!(
                s,
                IR::Op {
                    opcode: IROpCode::OpFor,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(fors.len(), 1, "expected exactly one reconstructed for: {out:?}");
    assert!(
        !out.iter().any(|s| matches!(
            s,
            IR::Op {
                opcode: IROpCode::OpWhile,
                ..
            }
        )),
        "a for must not be rebuilt as a while: {out:?}"
    );

    let IR::Op { args, .. } = fors[0] else { unreachable!() };
    assert_eq!(args.len(), 3, "for op must carry target, iterable, body: {:?}", args);
    let non_empty_body = matches!(&args[2], IR::Op { opcode: IROpCode::OpBlock, args, .. } if !args.is_empty());
    assert!(non_empty_body, "for body should hold its statement: {:?}", args[2]);
}

/// Nested loops: with the back-edge fix (emit + stop) and reconstruct_loop, the
/// inner loop is rebuilt via its own is_while_header path and the inner→outer
/// back edge closes cleanly.
#[test]
fn round_trip_nested() {
    assert_round_trips(&["i = 0\nwhile i < 3 { j = 0\nwhile j < 3 { j = j + 1 }\ni = i + 1 }"]);
}

/// The nesting is preserved: one outer `while` at top level, and its body holds
/// exactly one inner `while` (not flattened, not duplicated).
#[test]
fn round_trip_nested_structure() {
    let out = round_trip("i = 0\nwhile i < 3 { j = 0\nwhile j < 3 { j = j + 1 }\ni = i + 1 }");

    let outer: Vec<&IR> = out
        .iter()
        .filter(|s| {
            matches!(
                s,
                IR::Op {
                    opcode: IROpCode::OpWhile,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(outer.len(), 1, "expected one outer while: {out:?}");

    let IR::Op { args, .. } = outer[0] else { unreachable!() };
    let body = &args[1];
    let IR::Op {
        opcode: IROpCode::OpBlock,
        args: body_stmts,
        ..
    } = body
    else {
        panic!("while body should be a block: {body:?}");
    };
    let inner = body_stmts
        .iter()
        .filter(|s| {
            matches!(
                s,
                IR::Op {
                    opcode: IROpCode::OpWhile,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        inner, 1,
        "outer while body should hold exactly one nested while: {body_stmts:?}"
    );
}

/// match round-trips, but by *op preservation*, not true reconstruction: the
/// builder leaves the original `OpMatch` in the header block and reconstruction
/// emits it, then stops; the per-arm `match_case_N` blocks are never rebuilt.
/// Correct for the dormant net (un-optimized IR), but at wiring this must become
/// real arm reconstruction — otherwise optimizations applied to the arm blocks
/// would be dropped (the stored op carries the original arms).
#[test]
fn round_trip_match() {
    assert_round_trips(&[
        "x = 2\nmatch x { 1 => { y = 1 } 2 => { y = 2 } }",
        "x = 3\nmatch x { 1 => { y = 1 } 2 => { y = 2 } 3 => { y = 3 } _ => { y = 0 } }",
    ]);
}

/// The preserved match keeps exactly one `OpMatch`, with all its arms intact.
#[test]
fn round_trip_match_structure() {
    let out = round_trip("x = 3\nmatch x { 1 => { y = 1 } 2 => { y = 2 } 3 => { y = 3 } }");

    let matches: Vec<&IR> = out
        .iter()
        .filter(|s| {
            matches!(
                s,
                IR::Op {
                    opcode: IROpCode::OpMatch,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(matches.len(), 1, "expected one match: {out:?}");

    let IR::Op { args, .. } = matches[0] else {
        unreachable!()
    };
    let arms = match args.get(1) {
        Some(IR::Tuple(cases)) => cases.len(),
        _ => 0,
    };
    assert_eq!(arms, 3, "match should keep its three arms: {:?}", args.get(1));
}

// ---------------------------------------------------------------------------
// Execution-level differential
//
// The structural tests above prove the reconstruction has the right *shape*.
// This proves it has the right *behaviour*: a program and its CFG round-tripped
// copy must compute the same value. Both go through semantic analysis + compile
// + execute (PurePipeline::execute_ir); the only difference is the CFG round-trip
// inserted between transform and semantic. Each run uses a fresh pipeline so no
// global state bleeds across.
// ---------------------------------------------------------------------------

/// Parse source into its top-level Program IR (pre-semantic).
fn program(source: &str) -> IR {
    IR::Program(statements(source))
}

/// Round-trip a Program IR through the CFG engine: build → SSA → destruct →
/// reconstruct, returning the reconstructed Program.
fn cfg_round_trip(ir: &IR) -> IR {
    let stmts = match ir {
        IR::Program(s) => s.clone(),
        other => vec![other.clone()],
    };
    let mut cfg = IRCFGBuilder::new("rt").build(stmts);
    compute_dominators(&mut cfg);
    let ssa = SSABuilder::build(&cfg);
    destroy_ssa(&mut cfg, &ssa);
    IR::Program(reconstruct_from_cfg(&cfg))
}

/// Assert a program yields the same value before and after a CFG round-trip.
fn assert_same_result(source: &str) {
    let ir = program(source);
    let rebuilt = cfg_round_trip(&ir);

    // Keep both pipelines alive across the comparison.
    let mut p_base = PurePipeline::new().unwrap();
    let mut p_rt = PurePipeline::new().unwrap();
    let baseline = p_base.execute_ir(&ir).expect("baseline run failed");
    let after = p_rt.execute_ir(&rebuilt).expect("round-tripped run failed");

    assert_eq!(
        baseline, after,
        "execution differs after CFG round-trip for {source:?}\n  baseline = {baseline:?}\n  after    = {after:?}\n  rebuilt IR = {rebuilt:?}"
    );
}

/// The reconstructed IR computes the same result as the original — the
/// behavioural half of the Phase 0 net. Programs end in a scalar expression so
/// the final value is directly comparable.
#[test]
fn execution_differential() {
    assert_same_result("a = 3\nb = 4\na + b");
    assert_same_result("x = 7\nif x > 5 { y = 1 } else { y = 2 }\ny");
    assert_same_result("x = 0\nwhile x < 10 { x = x + 1 }\nx");
    assert_same_result("s = 0\nfor i in range(5) { s = s + i }\ns");
    assert_same_result("t = 0\nfor i in range(3) { for j in range(3) { t = t + 1 } }\nt");
    assert_same_result("x = 2\nmatch x { 1 => { y = 10 } 2 => { y = 20 } }\ny");
}

// ---------------------------------------------------------------------------
// Synthetic-interference oracle: maximal_naming
//
// Every SSA value gets a unique `var#version` name, so destruction performs a
// full out-of-SSA -- every phi becomes a real copy, every use references its
// exact version. A faithful transform must still execute identically to the
// source, so this exercises renaming + condition rewriting + phi materialization
// + the parallel-copy solver on real constructs (the identity 2355 gate is inert
// and cannot). See wip/CFG_SSA_REWIRING.md.
// ---------------------------------------------------------------------------

/// Full versioned destruction of a Program under maximal_naming, then reconstruct.
fn cfg_round_trip_maximal(ir: &IR) -> IR {
    let stmts = match ir {
        IR::Program(s) => s.clone(),
        other => vec![other.clone()],
    };
    let mut cfg = IRCFGBuilder::new("rt").build(stmts);
    compute_dominators(&mut cfg);
    let ssa = SSABuilder::build(&cfg);
    let naming = maximal_naming(&ssa);
    destroy_ssa_versioned(&mut cfg, &ssa, &naming);
    IR::Program(reconstruct_from_cfg(&cfg))
}

/// Assert a program yields the same value before and after full versioned
/// destruction under maximal_naming.
fn assert_same_result_maximal(source: &str) {
    let ir = program(source);
    let rebuilt = cfg_round_trip_maximal(&ir);

    let mut p_base = PurePipeline::new().unwrap();
    let mut p_rt = PurePipeline::new().unwrap();
    let baseline = p_base.execute_ir(&ir).expect("baseline run failed");
    let after = p_rt
        .execute_ir(&rebuilt)
        .unwrap_or_else(|e| panic!("maximal-naming run failed for {source:?}: {e:?}"));

    assert_eq!(
        baseline, after,
        "execution differs under maximal_naming for {source:?}\n  baseline = {baseline:?}\n  after    = {after:?}\n  rebuilt  = {rebuilt:?}"
    );
}

/// if/else under maximal separation: branch versions get distinct names, the
/// merge phi materializes a copy in each arm, and the condition is renamed via
/// `current_defs` (the empirical check for that assumption).
#[test]
fn maximal_naming_if_else() {
    assert_same_result_maximal("x = 5\nif x > 0 { y = 1 } else { y = 2 }\ny");
    assert_same_result_maximal("a = 1\nb = 2\nif a < b { c = a } else { c = b }\nc");
    assert_same_result_maximal("x = 5\nif x > 0 { x = 1 } else { x = 2 }\nx");
}

/// while under maximal separation: the header phi for the loop variable
/// materializes a copy in the preheader (from the initial value) and on the
/// back edge (from the body's update); the header condition is renamed via
/// `current_defs[header]` (which must hold the phi).
#[test]
fn maximal_naming_while() {
    assert_same_result_maximal("x = 0\nwhile x < 10 { x = x + 1 }\nx");
    assert_same_result_maximal("n = 100\ns = 0\nwhile n > 0 { s = s + n\nn = n - 1 }\ns");
}

// ---------------------------------------------------------------------------
// LICM as the first real consumer: hoist loop-invariant code, reconstruct, and
// check the result is unchanged (differential oracle at the Rust level, ahead of
// the full 2355 differential). Returns the hoist count so tests can assert that
// motion actually happened. See wip/CFG_SSA_REWIRING.md.
// ---------------------------------------------------------------------------

/// build -> SSA -> LICM -> reconstruct; returns (hoisted, rebuilt Program).
fn licm_round_trip(source: &str) -> (usize, IR) {
    let mut cfg = IRCFGBuilder::new("rt").build(statements(source));
    compute_dominators(&mut cfg);
    let ssa = SSABuilder::build(&cfg);
    let res = licm(&mut cfg, &ssa);
    destroy_ssa(&mut cfg, &ssa);
    (res.hoisted, IR::Program(reconstruct_from_cfg(&cfg)))
}

/// Assert LICM preserves the program's result; returns the hoist count.
fn assert_licm_sound(source: &str) -> usize {
    let ir = program(source);
    let (hoisted, rebuilt) = licm_round_trip(source);

    let mut p_base = PurePipeline::new().unwrap();
    let mut p_rt = PurePipeline::new().unwrap();
    let baseline = p_base.execute_ir(&ir).expect("baseline run failed");
    let after = p_rt
        .execute_ir(&rebuilt)
        .unwrap_or_else(|e| panic!("licm run failed for {source:?}: {e:?}\n{rebuilt:#?}"));

    assert_eq!(
        baseline, after,
        "LICM changed the result for {source:?}\n  baseline = {baseline:?}\n  after    = {after:?}\n  rebuilt  = {rebuilt:?}"
    );
    hoisted
}

#[test]
fn licm_basic_invariant() {
    // t = k * 2 is loop-invariant (k defined before the loop): hoistable.
    let hoisted = assert_licm_sound("k = 3\ns = 0\nwhile s < 20 { t = k * 2\ns = s + t }\ns");
    assert!(
        hoisted >= 1,
        "expected LICM to hoist the invariant t = k * 2 (hoisted={hoisted})"
    );
}

#[test]
fn licm_target_redefined() {
    // t = a + 1 is invariant but t is redefined (t = t + i) in the loop; hoisting
    // it drops the per-iteration reset. Result must be preserved.
    assert_licm_sound("a = 5\ns = 0\ni = 0\nwhile i < 3 { t = a + 1\nt = t + i\ns = s + t\ni = i + 1 }\ns");
}

// ---------------------------------------------------------------------------
// GVN multi-def: versioned destruction lets GVN alias to a canonical whose
// variable is multiply defined -- the case the old single-def restriction
// rejected. Sound with no speculation risk (aliasing is a copy, cannot fault;
// the canonical still computes the expression).
// ---------------------------------------------------------------------------

fn gvn_multidef_round_trip(source: &str) -> (usize, IR) {
    let mut cfg = IRCFGBuilder::new("rt").build(statements(source));
    compute_dominators(&mut cfg);
    let ssa = SSABuilder::build(&cfg);
    let gvn_res = gvn(&cfg, &ssa);
    let naming = maximal_naming(&ssa);
    destroy_ssa_versioned(&mut cfg, &ssa, &naming);
    let applied = materialize_gvn_versioned(&mut cfg, &ssa, &gvn_res, &naming);
    (applied, IR::Program(reconstruct_from_cfg(&cfg)))
}

fn assert_gvn_multidef_sound(source: &str) -> usize {
    let ir = program(source);
    let (applied, rebuilt) = gvn_multidef_round_trip(source);

    let mut p_base = PurePipeline::new().unwrap();
    let mut p_rt = PurePipeline::new().unwrap();
    let baseline = p_base.execute_ir(&ir).expect("baseline run failed");
    let after = p_rt
        .execute_ir(&rebuilt)
        .unwrap_or_else(|e| panic!("gvn multidef run failed for {source:?}: {e:?}\n{rebuilt:#?}"));

    assert_eq!(
        baseline, after,
        "GVN multidef changed the result for {source:?}\n  baseline = {baseline:?}\n  after = {after:?}\n  rebuilt = {rebuilt:?}"
    );
    applied
}

#[test]
fn gvn_multidef_aliases_reassigned_canonical() {
    // x = a + b, then x reassigned; y = a + b is redundant with the FIRST x, whose
    // variable is multiply defined -- the case the single-def restriction rejected.
    let applied = assert_gvn_multidef_sound("a = 3\nb = 4\nx = a + b\nx = x + 1\ny = a + b\ny + x");
    assert!(
        applied >= 1,
        "expected GVN to alias y to the multi-def canonical x (applied={applied})"
    );
    // Canonical reassigned to an unrelated value between its def and the redundant
    // use: aliasing to the wrong version would read 99, not a * a.
    let a2 = assert_gvn_multidef_sound("a = 2\nx = a * a\nx = 99\nz = a * a\nz + x");
    assert!(a2 >= 1, "expected multi-def alias for z (applied={a2})");
}

// ---------------------------------------------------------------------------
// GVN multi-def, production form: additive snapshot (materialize_gvn)
//
// The shipping pipeline must not rename defs or uses (versioned renaming turns
// late-binding closures into early binding, see
// maximal_naming_closure_early_binding_limit). Instead, the canonical value of
// a multi-def variable is captured by a fresh `__gvnN = x` inserted right after
// its def, and redundancies alias the temp. This harness is the exact
// `cfg_roundtrip` hook flow (`pipeline/semantic/mod.rs`).
// ---------------------------------------------------------------------------

/// build -> SSA -> gvn -> materialize_gvn (snapshot) -> reconstruct; the
/// production hook flow. Returns (applied, rebuilt Program).
fn gvn_snapshot_round_trip(source: &str) -> (usize, IR) {
    let mut cfg = IRCFGBuilder::new("rt").build(statements(source));
    compute_dominators(&mut cfg);
    let ssa = SSABuilder::build(&cfg);
    let gvn_res = gvn(&cfg, &ssa);
    let applied = materialize_gvn(&mut cfg, &ssa, &gvn_res);
    destroy_ssa(&mut cfg, &ssa);
    (applied, IR::Program(reconstruct_from_cfg(&cfg)))
}

fn assert_gvn_snapshot_sound(source: &str) -> usize {
    let ir = program(source);
    let (applied, rebuilt) = gvn_snapshot_round_trip(source);

    let mut p_base = PurePipeline::new().unwrap();
    let mut p_rt = PurePipeline::new().unwrap();
    let baseline = p_base.execute_ir(&ir).expect("baseline run failed");
    let after = p_rt
        .execute_ir(&rebuilt)
        .unwrap_or_else(|e| panic!("gvn snapshot run failed for {source:?}: {e:?}\n{rebuilt:#?}"));

    assert_eq!(
        baseline, after,
        "GVN snapshot changed the result for {source:?}\n  baseline = {baseline:?}\n  after = {after:?}\n  rebuilt = {rebuilt:?}"
    );
    applied
}

#[test]
fn gvn_snapshot_multidef() {
    // Same two multi-def cases as the versioned harness, through the production
    // snapshot path.
    let a1 = assert_gvn_snapshot_sound("a = 3\nb = 4\nx = a + b\nx = x + 1\ny = a + b\ny + x");
    assert!(a1 >= 1, "expected a snapshot alias for y (applied={a1})");
    let a2 = assert_gvn_snapshot_sound("a = 2\nx = a * a\nx = 99\nz = a * a\nz + x");
    assert!(a2 >= 1, "expected a snapshot alias for z (applied={a2})");
    // Two redundancies over the same canonical share one snapshot.
    let a3 = assert_gvn_snapshot_sound("a = 3\nx = a * a\nx = 0\ny = a * a\nz = a * a\ny + z + x");
    assert!(a3 >= 2, "expected both y and z aliased (applied={a3})");
}

#[test]
fn gvn_snapshot_fresh_name_dodges_collision() {
    // The program already owns `__gvn0`; the snapshot must pick the next free
    // name instead of clobbering it.
    let applied = assert_gvn_snapshot_sound("__gvn0 = 7\na = 2\nx = a * a\nx = 99\nz = a * a\nz + x + __gvn0");
    assert!(
        applied >= 1,
        "expected a snapshot alias despite the collision (applied={applied})"
    );
}

#[test]
fn gvn_snapshot_preserves_closure_late_binding() {
    // The closure captures x by name and reads it at call time (x = 9 by then).
    // A renaming materialization would corrupt this; the snapshot form must not
    // touch the closure body, while still aliasing y to the first x = a + b.
    let applied = assert_gvn_snapshot_sound("a = 1\nb = 2\nx = a + b\nf = () => { x }\nx = 9\ny = a + b\nf() + y");
    assert!(applied >= 1, "expected a snapshot alias for y (applied={applied})");
}

#[test]
fn licm_speculation_zero_trip() {
    // Loop never runs (s < 0 false). t = 10 / d is invariant but faults (d = 0);
    // an unconditional preheader hoist raises where the original does not. The
    // guarded hoist block (`if cond { hoisted }`) closes this: the hoist DOES
    // happen (asserted) but its code only runs when the loop would.
    let hoisted = assert_licm_sound("d = 0\ns = 0\nwhile s < 0 { t = 10 / d\ns = s + 1 }\ns");
    assert!(hoisted >= 1, "expected the guarded hoist to fire (hoisted={hoisted})");
}

/// Zero-trip definedness: on a loop that never runs, the original never
/// executes `t = d + 1`, and the post-loop read of `t` yields the unset value
/// (a statically-assigned name reads as None, not NameError). An unconditional
/// hoist would set `t` to 6 -- observably different. The guarded hoist must
/// leave `t` unset exactly like the source, while the hoist itself still fires.
#[test]
fn licm_zero_trip_preserves_unset_target() {
    let hoisted = assert_licm_sound("d = 5\ns = 0\nwhile s < 0 { t = d + 1\ns = s + 1 }\nt");
    assert!(hoisted >= 1, "expected the guarded hoist to fire (hoisted={hoisted})");
}

/// The loop condition reads the hoist target: iteration one tests the PRE-loop
/// value, so hoisting would change what the condition observes. The stale-use
/// gate must refuse (t has a header phi mixing the pre-loop value).
#[test]
fn licm_refuses_target_read_by_loop_condition() {
    let hoisted = assert_licm_sound("k = 9\nt = 0\nwhile t < 5 { t = k * 2 }\nt");
    assert_eq!(hoisted, 0, "t is read by the loop condition; hoisting it is unsound");
}

/// A nested loop's condition reads the target defined in the outer body: on the
/// outer loop the inner header's condition is a stale reader (same shape as
/// above, one level down). Refused for both loops.
#[test]
fn licm_refuses_target_read_by_nested_loop_condition() {
    let hoisted = assert_licm_sound("k = 9\nt = 0\nc = 0\nwhile c < 2 { while t < 5 { t = k * 2 }\nc = c + 1 }\nt");
    assert_eq!(hoisted, 0, "t is read by the nested loop condition before its def");
}

/// `for` loops are not hoisted in v1: the guard would need a non-consuming
/// emptiness test on the iterable. The invariant stays in the body.
#[test]
fn licm_skips_for_loops() {
    let hoisted = assert_licm_sound("s = 0\nk = 2\nfor i in range(3) { t = k * 2\ns = s + t }\ns");
    assert_eq!(hoisted, 0, "for loops are excluded from the guarded hoist in v1");
}

/// An impure loop condition (function call) cannot be duplicated into a guard:
/// the whole loop is skipped.
#[test]
fn licm_skips_impure_condition() {
    let hoisted = assert_licm_sound("k = 1\ns = 0\nwhile len([s]) < 0 { t = k + 1\ns = s + 1 }\ns");
    assert_eq!(hoisted, 0, "an impure condition must not be duplicated into a guard");
}

/// Normal-trip loop: the hoist fires, the guarded block runs once, and the
/// post-loop read of the hoisted target matches the source.
#[test]
fn licm_hoist_target_read_after_loop() {
    let hoisted = assert_licm_sound("k = 3\ns = 0\nwhile s < 6 { t = k * 2\ns = s + t }\nt");
    assert!(
        hoisted >= 1,
        "expected the invariant t = k * 2 to hoist (hoisted={hoisted})"
    );
}

/// A call nested under a pure head opcode (`t = bump() + 1`: Add at the top,
/// the call in its args). Hoisting would run bump() once instead of once per
/// iteration -- its effect on the global n is observable. The RHS purity gate
/// must check recursively, not just the head opcode. Structural check only:
/// closures do not capture under this pre-semantic harness, so the observable
/// effect is asserted through `gate_licm_differential` (full pipeline).
#[test]
fn licm_refuses_call_nested_in_pure_rhs() {
    let (hoisted, _) =
        licm_round_trip("n = 0\nbump = () => { n = n + 1\nn }\ns = 0\nwhile s < 3 { t = bump() + 1\ns = s + 1 }\nn");
    assert_eq!(
        hoisted, 0,
        "a nested call must not be hoisted (effects move across iterations)"
    );
}

/// A call in the loop body -- in a *separate* instruction, not the candidate's
/// RHS -- can read the candidate's name through a captured closure (late
/// binding), a read invisible to the SSA use sets. `f0 = () => b` then
/// `while .. { e = f0()\n b = d + -2 }`: the invariant `b = d + -2` is a valid
/// hoist candidate (same shape as `licm_refuses_def_on_conditional_path`), but
/// hoisting it above `e = f0()` makes the call observe the hoisted `b` on the
/// first iteration. The v1 guard refuses the whole loop on any body call.
/// Found by the Phase 4 property harness (`cfg_proptest`).
#[test]
fn licm_refuses_loop_with_closure_call() {
    let (hoisted, _) = licm_round_trip(
        "b = 2\nd = 3\nf0 = () => { b }\nc0 = 0\nwhile c0 < 1 { c0 = c0 + 1\ne = f0()\nb = d + -2 }\ne",
    );
    assert_eq!(
        hoisted, 0,
        "a call in the loop body can read the candidate's name via a closure; refuse the hoist"
    );
}

/// A def on a conditional path does not run on every iteration: hoisting it
/// makes the store unconditional (`b` defined even when the branch is never
/// taken). The candidate's block must dominate every back-edge source. Found
/// by the Phase 4 property harness with the `continue` variant; the plain
/// branch variant has the same shape (the merge block is a back-edge source
/// the branch block does not dominate).
#[test]
fn licm_refuses_def_on_conditional_path() {
    // Inside an if branch, no continue.
    let (hoisted, _) =
        licm_round_trip("a = 1\nd = 3\nb = 2\nc = 0\nwhile c < 1 { c = c + 1\nif 0 > a { b = d + -2 } else { 0 } }\nb");
    assert_eq!(hoisted, 0, "a def inside a branch must not be hoisted");

    // The minimized property-harness counterexample: branch ends in continue,
    // making the branch block itself a back-edge source (it still does not
    // dominate the merge's back-edge).
    let (hoisted, _) = licm_round_trip(
        "a = 1\nd = 3\nb = 2\nc = 0\nwhile c < 1 { c = c + 1\nif 0 > a { b = d + -2\ncontinue } else { 0 } }\nb",
    );
    assert_eq!(hoisted, 0, "a def cut off by continue must not be hoisted");
}

/// An early exit (`break`) leaves an iteration where the candidate has not
/// run yet: with `while cond { if x { break }\nt = inv }`, the breaking
/// iteration never defines `t`, but the hoisted form would. The v1 guard
/// refuses the whole loop.
#[test]
fn licm_refuses_loop_with_early_exit() {
    let (hoisted, _) = licm_round_trip("k = 3\ns = 0\nwhile s < 5 { if s == 0 { break }\nt = k * 2\ns = s + t }\ns");
    assert_eq!(hoisted, 0, "a loop with a break must not hoist (v1 wholesale refusal)");
}

/// Match arms are opaque to the CFG (op-preservation: a Fallthrough edge to
/// the merge whatever the arm does), so an arm's break/continue/return has no
/// edge the escaping-edge guard could see -- the preserved IR is scanned
/// instead. Found by the Phase 4 property harness: the breaking arm skipped
/// `b = d * 0` in the source, but the hoisted form still defined `b`.
#[test]
fn licm_refuses_loop_with_break_in_match_arm() {
    let (hoisted, _) = licm_round_trip(
        "d = 3\nb = 2\nc = 0\nwhile c < 1 {\n    c = c + 1\n    match 0 { 0 => { break } _ => { 0 } }\n    b = d * 0\n}\nb",
    );
    assert_eq!(
        hoisted, 0,
        "a break hidden in an opaque match arm must refuse the hoist"
    );

    // A match without loop control does not block the hoist.
    let (hoisted, _) = licm_round_trip(
        "d = 3\nb = 2\nc = 0\nwhile c < 3 {\n    c = c + 1\n    match c { 1 => { 0 } _ => { 1 } }\n    b = d * 2\n}\nb",
    );
    assert_eq!(hoisted, 1, "a loop-control-free match must not block hoisting");
}

/// Assignments inside opaque match arms have no SSA def, so an operand they
/// rewrite looked loop-invariant: `a = g * -1` was hoisted with the pre-loop
/// `g` while an arm did `g = 0` each iteration (found by the Phase 4 property
/// harness). The preserved IR's SetLocals targets must count as loop defs.
#[test]
fn licm_refuses_operand_assigned_in_match_arm() {
    // Operand g rewritten by an arm: the candidate must stay put.
    let (hoisted, _) = licm_round_trip(
        "g = 5\nb = 2\nc = 0\nwhile c < 2 {\n    c = c + 1\n    a = g * -1\n    match b { 0 => { 0 } _ => { g = 0 } }\n}\na",
    );
    assert_eq!(hoisted, 0, "operand assigned by an opaque arm is not invariant");

    // Target assigned by an arm: hoisting would drop the arm's write ordering.
    let (hoisted, _) = licm_round_trip(
        "d = 3\nb = 2\nc = 0\nwhile c < 2 {\n    c = c + 1\n    a = d * 2\n    match b { 0 => { 0 } _ => { a = 7 } }\n}\na",
    );
    assert_eq!(hoisted, 0, "target also assigned by an opaque arm must refuse");
}

/// match under maximal separation (arms opaque to SSA -- the op-preservation caveat).
#[test]
fn maximal_naming_match() {
    assert_same_result_maximal("x = 2\nmatch x { 1 => { y = 10 } 2 => { y = 20 } }\ny");
    // arm bodies read an outer variable (a ref inside the opaque arms) and define
    // one used after the match.
    assert_same_result_maximal("a = 5\nx = 2\nmatch x { 1 => { b = a } 2 => { b = a + 1 } }\nb");
    // scrutinee is a computed/renamed value; a variable is redefined before and
    // used after.
    assert_same_result_maximal("k = 1\nk = k + 1\nmatch k { 1 => { r = 100 } _ => { r = 200 } }\nr");
}

/// Baseline and after-destruction results under maximal_naming, for cases where
/// the two legitimately differ (documented limits, pinned below).
fn maximal_pair(source: &str) -> (catnip_vm::Value, catnip_vm::Value) {
    let ir = program(source);
    let rebuilt = cfg_round_trip_maximal(&ir);
    let mut p_base = PurePipeline::new().unwrap();
    let mut p_rt = PurePipeline::new().unwrap();
    let baseline = p_base.execute_ir(&ir).expect("baseline run failed");
    let after = p_rt.execute_ir(&rebuilt).expect("maximal run failed");
    (baseline, after)
}

/// Documented limit of versioned renaming: Catnip closures resolve captured
/// names at call time (late binding: `k = 1; f = () => k; k = 2; f()` is 2),
/// but `collect_refs` walks into an `OpLambda` body, so `rename_versioned`
/// rewrites captured refs (and nested SetLocals targets) to the version current
/// at the lambda's *definition* — early binding. This is why the production GVN
/// materialization is an additive snapshot (`materialize_gvn`) and the
/// versioned-rename path stays out of the shipping pipeline until it is
/// lambda-aware (LICM/IV, the passes that move defs, will need that guard).
/// The divergence is pinned: if it disappears, the naming decision in
/// wip/CFG_SSA_REWIRING.md must be re-evaluated.
#[test]
fn maximal_naming_closure_early_binding_limit() {
    // Captured global redefined after the closure's definition: the body ref is
    // renamed to the version at definition (k__v0 = 1); the source reads 2.
    let (base, after) = maximal_pair("k = 1\nf = () => { k }\nk = 2\nf()");
    assert_ne!(
        base, after,
        "renaming inside lambda bodies became transparent; re-evaluate the naming decision"
    );
    // Lambda parameter shadowing a top-level variable: the body ref is renamed
    // to the renamed global (n__v0 = 10), no longer resolving to the parameter.
    let (base, after) = maximal_pair("n = 10\nf = (n) => { n + 1 }\nf(5)");
    assert_ne!(
        base, after,
        "lambda-parameter shadowing under rename became transparent; re-evaluate the naming decision"
    );
    // Coherent by accident today: with a single top-level version, the nested
    // read, the nested write and the final read all land on the same renamed
    // cell, so the result matches.
    assert_same_result_maximal("counter = 0\ninc = () => { counter = counter + 1 }\ninc()\ninc()\ncounter");
}

/// for and nested loops under maximal separation.
#[test]
fn maximal_naming_for_nested() {
    assert_same_result_maximal("s = 0\nfor i in range(5) { s = s + i }\ns");
    assert_same_result_maximal("t = 0\nfor i in range(3) { for j in range(3) { t = t + 1 } }\nt");
    assert_same_result_maximal("i = 0\nwhile i < 3 { j = 0\nwhile j < 3 { j = j + 1 }\ni = i + 1 }\ni");
}

// ---------------------------------------------------------------------------
// Phase 1 gate
//
// The differential above drives the CFG functions directly. This drives them
// through the analyzer gate (`SemanticAnalyzer::set_cfg_enabled`, exposed on the
// pipeline as `set_cfg_enabled`): the same source executed with the gate off and
// on must agree. This is what proves the `analyze_full` hook is wired, not just
// the reconstruction in isolation.
// ---------------------------------------------------------------------------

/// Run `source` through a fresh pipeline with the CFG gate set to `cfg`.
fn run_with_cfg(source: &str, cfg: bool) -> catnip_vm::Value {
    let mut p = PurePipeline::new().unwrap();
    p.set_cfg_enabled(cfg);
    p.execute(source).expect("run failed")
}

/// With the gate enabled, the analyzer round-trips the IR through CFG before
/// compiling; on these constructs the result must match the gate-off run. This
/// proves the `analyze_full` hook is wired, not just the reconstruction in
/// isolation. The corpus is deliberately the subset the round-trip handles
/// correctly today; the known holes live in `gate_roundtrip_holes_phase2`.
#[test]
fn gate_roundtrip_differential() {
    for source in [
        "a = 3\nb = 4\na + b",
        "x = 7\nif x > 5 { y = 1 } else { y = 2 }\ny",
        "x = 0\nwhile x < 10 { x = x + 1 }\nx",
        "s = 0\nfor i in range(5) { s = s + i }\ns",
        "t = 0\nfor i in range(3) { for j in range(3) { t = t + 1 } }\nt",
    ] {
        let off = run_with_cfg(source, false);
        let on = run_with_cfg(source, true);
        assert_eq!(off, on, "gate on/off disagree for {source:?}: {off:?} vs {on:?}");
    }
}

// ---------------------------------------------------------------------------
// Phase 3 — inter-block GVN (first wired redundancy pass, subsumes CSE)
//
// `cfg_roundtrip` now runs `gvn` + `materialize_gvn` between SSA construction and
// destruction. Eliminating a redundant expression must not change the observable
// result (differential), and the redundant definition must actually become a copy
// rather than a recomputation (targeted check).
// ---------------------------------------------------------------------------

/// With GVN active, common subexpressions are eliminated but the result is
/// unchanged. The canonical definitions here are single-def, so the redirect
/// `y = x` is sound.
#[test]
fn gate_gvn_differential() {
    for source in [
        "a = 3\nb = 4\nx = a + b\ny = a + b\nx + y",
        "a = 1\nb = 2\nc = 3\np = a * b\nq = a * b\nr = b * c\np + q + r",
        "n = 10\nu = n - 1\nv = n - 1\nu + v",
        // Redundant expression behind a branch (dominating def is single-def).
        "a = 2\nb = 3\nx = a + b\nif x > 0 { y = a + b } else { y = 0 }\ny",
        // Distinct literals must NOT merge: same variable use, different constant.
        "n = 5\na = n + 1\nb = n + 2\na + b",
        // Multi-def canonical (snapshot path): x is reassigned between its def
        // and the redundancy, so y must alias the snapshot, not the bare name.
        "a = 3\nb = 4\nx = a + b\nx = x + 1\ny = a + b\ny + x",
        "a = 2\nx = a * a\nx = 99\nz = a * a\nz + x",
        // Snapshot must not disturb a closure's late-bound read of x.
        "a = 1\nb = 2\nx = a + b\nf = () => { x }\nx = 9\ny = a + b\nf() + y",
    ] {
        let off = run_with_cfg(source, false);
        let on = run_with_cfg(source, true);
        assert_eq!(off, on, "GVN changed the result for {source:?}: {off:?} vs {on:?}");
    }
}

/// LICM through the analyzer gate: guarded hoist on a normal loop, guarded
/// zero-trip with a faulting invariant (must not raise), unset target after a
/// zero-trip loop, refusal when the condition reads the target, nested loops.
#[test]
fn gate_licm_differential() {
    for source in [
        "k = 3\ns = 0\nwhile s < 20 { t = k * 2\ns = s + t }\ns",
        "k = 3\ns = 0\nwhile s < 6 { t = k * 2\ns = s + t }\nt",
        "d = 0\ns = 0\nwhile s < 0 { t = 10 / d\ns = s + 1 }\ns",
        "d = 5\ns = 0\nwhile s < 0 { t = d + 1\ns = s + 1 }\nt",
        "k = 9\nt = 0\nwhile t < 5 { t = k * 2 }\nt",
        "k = 9\nt = 0\nc = 0\nwhile c < 2 { while t < 5 { t = k * 2 }\nc = c + 1 }\nt",
        "i = 0\nk = 7\nwhile i < 3 { j = 0\nwhile j < 3 { u = k + 1\nj = j + u }\ni = i + 1 }\ni",
        // Call nested under a pure head opcode: bump() must keep running once
        // per iteration (n counts the calls), so the candidate must not hoist.
        "n = 0\nbump = () => { n = n + 1\nn }\ns = 0\nwhile s < 3 { t = bump() + 1\ns = s + 1 }\nn",
        // A closure reads a global the loop later mutates with an invariant rhs,
        // in a separate call instruction. Hoisting `b = d + -2` above `e = f0()`
        // would let the call observe the hoisted value on the first iteration
        // (result 2 instead of 3). The body call must block the hoist. Property
        // harness regression (wip/CFG_SSA_REWIRING.md).
        "b = 2\nd = 3\nf0 = () => { b }\nc0 = 0\nwhile c0 < 1 { c0 = c0 + 1\ne = f0()\nb = d + -2 }\ne + b",
    ] {
        let off = run_with_cfg(source, false);
        let on = run_with_cfg(source, true);
        assert_eq!(off, on, "LICM changed the result for {source:?}: {off:?} vs {on:?}");
    }
}

/// Codex finding (high): canonical lookup keeps the first value seen in dominator
/// preorder, so an expression first seen in the then-branch stays canonical in
/// the sibling else-branch even though its definition does not dominate it.
/// Here `x = a+b` (then) must NOT make `y = a+b` (else) into `y = x`: on the else
/// path `x` is never defined. The single-def guard does not catch it (x has one
/// CFG def); dominance does.
#[test]
fn gvn_no_cross_branch_redirect() {
    // if-statement so both arms are split into CFG blocks (an if-expression would
    // be opaque inside a SetLocals RHS). else is taken; if `y = a+b` was rewritten
    // to `y = x`, x is undefined on this path.
    let src = "a = -1\nb = 3\nif a > 0 { x = a + b } else { y = a + b }\ny";
    let off = run_with_cfg(src, false);
    let on = run_with_cfg(src, true);
    assert_eq!(off, on, "cross-branch CSE miscompiled {src:?}: {off:?} vs {on:?}");
}

/// Codex finding (high): `GetAttr` reads mutable state, so a read after a field
/// write must not reuse the earlier read. With `GetAttr` excluded from
/// `pure_opcodes`, `x` stays 1 and `y` is 2.
#[test]
fn gvn_does_not_collapse_mutable_field_read() {
    let src = "struct Box { v }\nb = Box(1)\nx = b.v\nb.v = 2\ny = b.v\ny";
    let off = run_with_cfg(src, false);
    let on = run_with_cfg(src, true);
    assert_eq!(off, on, "mutable field read collapsed for {src:?}: {off:?} vs {on:?}");
}

/// Codex finding (high): a destructuring assignment `(a,b,...) = e` defines
/// elements of `e`, not `e` itself, so its target must not share the value number
/// of a `tmp = e`. Here `a == tmp` must not collapse to `tmp == tmp`.
#[test]
fn gvn_does_not_value_number_unpack_target() {
    let src = "base = [1, 2]\ntmp = base + base\n(a, b, c, d) = base + base\na == tmp";
    let off = run_with_cfg(src, false);
    let on = run_with_cfg(src, true);
    assert_eq!(
        off, on,
        "unpack target value-numbered as whole RHS for {src:?}: {off:?} vs {on:?}"
    );
}

/// Codex finding (high): materializing a redundant expression as an alias
/// (`y = x`) is unsound when the result is a freshly-allocated mutable value.
/// Here `x = a + b` and `y = a + b` are distinct lists; mutating `x` must not
/// change `y`. GVN aliases only proven immutable scalars, so list concat is
/// recomputed.
#[test]
fn gvn_does_not_alias_mutable_result() {
    let src = "a = [1, 2]\nb = [3, 4]\nx = a + b\ny = a + b\nx[0] = 9\ny[0]";
    let off = run_with_cfg(src, false);
    let on = run_with_cfg(src, true);
    assert_eq!(off, on, "mutable result aliased for {src:?}: {off:?} vs {on:?}");
}

/// Regression for the key bug the full suite caught: `a + 1` and `a + 2` share
/// the same variable use but differ by a constant operand, so they must not be
/// value-numbered together. Keying on `uses` alone (variables only) wrongly
/// merged them.
#[test]
fn gvn_keeps_distinct_literals() {
    use catnip_core::cfg::ssa_gvn::gvn;

    let stmts = statements("a = 5\nx = a + 1\ny = a + 2\nx + y");
    let mut cfg = IRCFGBuilder::new("gvn").build(stmts);
    compute_dominators(&mut cfg);
    let ssa = SSABuilder::build(&cfg);

    let result = gvn(&cfg, &ssa);
    assert_eq!(
        result.redundant, 0,
        "distinct-literal expressions must not be value-numbered together"
    );
}

/// The redundant `y = a + b` is rewritten to a copy of the canonical `x`, so only
/// one `Add`-valued SetLocals (`x`) survives — proof GVN is materialized, not just
/// computed.
#[test]
fn gvn_rewrites_redundant_to_copy() {
    use catnip_core::cfg::ssa_gvn::{gvn, materialize_gvn};

    let stmts = statements("a = 3\nb = 4\nx = a + b\ny = a + b\nx + y");
    let mut cfg = IRCFGBuilder::new("gvn").build(stmts);
    compute_dominators(&mut cfg);
    let ssa = SSABuilder::build(&cfg);

    let result = gvn(&cfg, &ssa);
    assert!(result.redundant >= 1, "expected GVN to find a redundant expression");

    materialize_gvn(&mut cfg, &ssa, &result);

    let add_setlocals = cfg
        .blocks
        .values()
        .flat_map(|b| &b.instructions)
        .filter(|op| {
            matches!(op, IR::Op { opcode: IROpCode::SetLocals, args, .. }
                if matches!(args.get(1), Some(IR::Op { opcode: IROpCode::Add, .. })))
        })
        .count();
    assert_eq!(
        add_setlocals, 1,
        "exactly one Add-valued SetLocals (x) should remain; y must be a copy"
    );
}

/// Two reconstruction holes surfaced by the Codex adversarial review, now closed
/// (Phase 2). `region.rs` used to drop statements after a `match` (the trailing
/// value was lost, so the program took the match's value) and mishandle loops
/// whose body breaks/returns (the header had no back-edge, so it was rebuilt as
/// an if/else and raised "break outside loop"). The fixes: a match header is
/// emitted by op-preservation and reconstruction resumes at the arms' merge; a
/// loop header is detected by its preserved op rather than a back-edge search;
/// and break/continue/return edges stop body reconstruction instead of pulling
/// post-region code in. The corpus stresses both mechanisms beyond the two
/// minimal cases.
#[test]
fn gate_roundtrip_break_continue_post_match() {
    for source in [
        // Post-match code, with a value that differs from every arm.
        "x = 2\nmatch x { 1 => { 10 } 2 => { 20 } }\n99",
        // Post-match code reading a variable the arms define.
        "x = 5\nmatch x { 1 => { y = 1 } _ => { y = 2 } }\ny + 100",
        // Unconditional break (no back-edge to the header).
        "x = 0\nwhile True { x = 1\nbreak\nx = 2 }\nx",
        // Conditional break: the header keeps a back-edge and a break exit.
        "x = 0\nwhile x < 100 { x = x + 1\nif x == 5 { break } }\nx",
        // break inside a for loop.
        "s = 0\nfor i in range(10) { if i == 3 { break }\ns = s + i }\ns",
        // continue inside a for loop.
        "s = 0\nfor i in range(5) { if i == 2 { continue }\ns = s + i }\ns",
        // Loop exit block is itself a match header, followed by more code.
        "x = 0\nwhile x < 5 { x = x + 1 }\nmatch x { 5 => { 50 } _ => { 0 } }\nx + 1",
        // Match arm ending in `continue`, with live post-match code in the loop
        // body: the continue arm never reaches the match merge, so the merge
        // cannot be inferred from an all-arms reachability intersection.
        "s = 0\nfor i in range(5) { match i { 2 => { continue } _ => { s = s + 1 } }\ns = s + 10 }\ns",
        // Match arm ending in `break`, same shape: a naive merge search returns
        // the loop exit and pulls post-loop code into the body.
        "s = 0\nfor i in range(5) { match i { 3 => { break } _ => { s = s + 1 } }\ns = s + 10 }\ns",
    ] {
        let off = run_with_cfg(source, false);
        let on = run_with_cfg(source, true);
        assert_eq!(off, on, "gate on/off disagree for {source:?}: {off:?} vs {on:?}");
    }
}

/// A loop nested inside one branch of an if (itself inside a loop) must stay
/// inside that branch. `find_merge_point` walked *all* edges including
/// back-edges, so from the else branch the walk re-entered the outer loop and
/// reached the then branch's inner while header, which — being dominated by
/// the if header — was taken for the merge: the inner loop was reconstructed
/// at the outer body level and ran even when the branch was not taken
/// (`c1 < 1` with `c1` never bound -> TypeError). Found by the Phase 4
/// property harness; the fix cuts back-edges from the merge search.
#[test]
fn gate_roundtrip_loop_inside_if_branch() {
    for source in [
        // The minimized property-harness counterexample: while in the then
        // branch, branch not taken (a=1 so 0 == a is false).
        "a = 1\nc0 = 0\nwhile c0 < 1 {\n    c0 = c0 + 1\n    if 0 == a {\n        c1 = 0\n        while c1 < 1 { c1 = c1 + 1 }\n    } else { 0 }\n}\na",
        // Branch taken: the inner loop must still run (and only once).
        "a = 0\ns = 0\nc0 = 0\nwhile c0 < 1 {\n    c0 = c0 + 1\n    if 0 == a {\n        c1 = 0\n        while c1 < 3 { c1 = c1 + 1\ns = s + 1 }\n    } else { 0 }\n}\ns",
        // Loop in the else branch, not taken.
        "a = 0\nc0 = 0\nwhile c0 < 1 {\n    c0 = c0 + 1\n    if 0 == a { 0 } else {\n        c1 = 0\n        while c1 < 1 { c1 = c1 + 1 }\n    }\n}\na",
        // for-loop nested in a then branch, not taken.
        "a = 1\ns = 0\nc0 = 0\nwhile c0 < 1 {\n    c0 = c0 + 1\n    if 0 == a {\n        for i in range(3) { s = s + 1 }\n    } else { 0 }\n}\ns",
        // Code after the if inside the loop body: merge and tail are distinct.
        "a = 1\nt = 0\nc0 = 0\nwhile c0 < 2 {\n    c0 = c0 + 1\n    if 0 == a {\n        c1 = 0\n        while c1 < 1 { c1 = c1 + 1 }\n    } else { 0 }\n    t = t + 10\n}\nt",
        // Same shape without the outer loop: if at top level.
        "a = 1\nif 0 == a {\n    c1 = 0\n    while c1 < 1 { c1 = c1 + 1 }\n} else { 0 }\na",
    ] {
        let off = run_with_cfg(source, false);
        let on = run_with_cfg(source, true);
        assert_eq!(off, on, "gate on/off disagree for {source:?}: {off:?} vs {on:?}");
    }
}

/// Structural half of the case above: the rebuilt then-branch block must
/// contain the inner OpWhile (not the outer body).
#[test]
fn round_trip_keeps_loop_inside_branch() {
    let out = round_trip(
        "a = 1\nc0 = 0\nwhile c0 < 1 {\n    c0 = c0 + 1\n    if 0 == a {\n        c1 = 0\n        while c1 < 1 { c1 = c1 + 1 }\n    } else { 0 }\n}\na",
    );

    // Find the outer while, its body, the if, and check the then block.
    fn find_op(stmts: &[IR], op: IROpCode) -> Option<&IR> {
        stmts
            .iter()
            .find(|s| matches!(s, IR::Op { opcode, .. } if *opcode == op))
    }
    let outer = find_op(&out, IROpCode::OpWhile).expect("outer while at top level");
    let IR::Op { args: outer_args, .. } = outer else {
        unreachable!()
    };
    let IR::Op { args: body_stmts, .. } = &outer_args[1] else {
        panic!("outer body is a block")
    };
    let if_op = find_op(body_stmts, IROpCode::OpIf).expect("if inside the outer body");
    assert!(
        find_op(body_stmts, IROpCode::OpWhile).is_none(),
        "inner while must NOT be a sibling of the if in the outer body"
    );
    let IR::Op { args: if_args, .. } = if_op else {
        unreachable!()
    };
    let IR::Tuple(branches) = &if_args[0] else {
        panic!("if branches tuple")
    };
    let IR::Tuple(first_branch) = &branches[0] else {
        panic!("branch tuple")
    };
    let IR::Op { args: then_stmts, .. } = &first_branch[1] else {
        panic!("then block")
    };
    assert!(
        find_op(then_stmts, IROpCode::OpWhile).is_some(),
        "inner while lives inside the then branch"
    );
}

// ---------------------------------------------------------------------------
// Phase 3 — sound DSE v1 (transparent stores, all-paths kill, call barrier)
//
// `global_dse` eliminates a store only when it has zero SSA uses, its RHS is a
// scalar literal or an interned Ref (nothing that can fault or dispatch user
// code), and every forward path reaches a transparent redefinition through a
// transparent window (a call in the window could read the target by name --
// closures are late-bound; a faultable op would expose the divergent
// environment to a post-abort observer). See ssa_dse.rs for the full argument.
// ---------------------------------------------------------------------------

/// build -> SSA -> DSE -> reconstruct; returns (eliminated, rebuilt Program).
fn dse_round_trip(source: &str) -> (usize, IR) {
    let mut cfg = IRCFGBuilder::new("rt").build(statements(source));
    compute_dominators(&mut cfg);
    let ssa = SSABuilder::build(&cfg);
    let res = global_dse(&cfg, &ssa);
    apply_dse(&mut cfg, &res);
    destroy_ssa(&mut cfg, &ssa);
    (res.eliminated, IR::Program(reconstruct_from_cfg(&cfg)))
}

/// Assert DSE preserves the program's result; returns the elimination count.
fn assert_dse_sound(source: &str) -> usize {
    let ir = program(source);
    let (eliminated, rebuilt) = dse_round_trip(source);

    let mut p_base = PurePipeline::new().unwrap();
    let mut p_rt = PurePipeline::new().unwrap();
    let baseline = p_base.execute_ir(&ir).expect("baseline run failed");
    let after = p_rt
        .execute_ir(&rebuilt)
        .unwrap_or_else(|e| panic!("dse run failed for {source:?}: {e:?}\n{rebuilt:#?}"));

    assert_eq!(
        baseline, after,
        "DSE changed the result for {source:?}\n  baseline = {baseline:?}\n  after    = {after:?}\n  rebuilt  = {rebuilt:?}"
    );
    eliminated
}

#[test]
fn dse_eliminates_overwritten_literal() {
    let eliminated = assert_dse_sound("x = 1\nx = 2\nx");
    assert_eq!(eliminated, 1, "the overwritten literal store must be eliminated");
}

#[test]
fn dse_eliminates_ref_store() {
    let eliminated = assert_dse_sound("a = 5\nx = a\nx = 2\na + x");
    assert_eq!(eliminated, 1, "the overwritten Ref store must be eliminated");
}

/// Removing `b = a` drops the last use of `a = 1`, which then dies against its
/// own kill `a = 2` -- the fixpoint cascade across values.
#[test]
fn dse_fixpoint_chain() {
    let eliminated = assert_dse_sound("a = 1\nb = a\nb = 0\na = 2\na + b");
    assert!(
        eliminated >= 2,
        "expected the cascade to kill both stores (eliminated={eliminated})"
    );
}

/// Transparent stores of other variables in the window do not block the kill.
#[test]
fn dse_window_transparent_stores() {
    let eliminated = assert_dse_sound("x = 1\ny = 2\nz = y\nx = 3\nx + y + z");
    assert_eq!(eliminated, 1, "transparent stores must not act as barriers");
}

/// Inter-block kill: both branches redefine x, the crossed condition is a bare
/// Ref to a proven scalar name, so the pre-branch store dies (all-paths rule).
#[test]
fn dse_eliminates_cross_block_kill() {
    let eliminated = assert_dse_sound("c = 1\nx = 1\nif c { x = 2 } else { x = 3 }\nx");
    assert!(
        eliminated >= 1,
        "expected the pre-branch store to die (eliminated={eliminated})"
    );
}

/// Branch stores that reach the program tail survive: only the pre-branch
/// store dies, the two branch kills are barriered by the trailing expression.
#[test]
fn dse_kills_both_branches_but_keeps_final_stores() {
    let eliminated = assert_dse_sound("c = 1\nx = 1\nif c { x = 2 } else { x = 3 }\n5");
    assert_eq!(eliminated, 1, "only the pre-branch store must die");
}

/// An op condition (`c > 0`) is not admitted in the window: without type
/// information a comparison can fault or dispatch a user overload. Traced
/// extension: admit ops proven non-faulting once type hints reach here.
#[test]
fn dse_refuses_op_condition_in_window() {
    let eliminated = assert_dse_sound("c = 1\nx = 1\nif c > 0 { x = 2 } else { x = 3 }\nx");
    assert_eq!(eliminated, 0, "an op condition in the window must refuse the candidate");
}

/// A name rebound inside a closure body is disqualified as a window condition:
/// the closure could rebind it to a value whose truthiness dispatches user code.
#[test]
fn dse_scalar_cond_disqualified_by_closure() {
    let (eliminated, _) = dse_round_trip("c = 1\nf = () => { c = 0 }\nx = 1\nif c { x = 2 } else { x = 3 }\nx");
    assert_eq!(eliminated, 0, "a nested def must disqualify the condition name");
}

/// A partial kill is not enough: the fallthrough path reads x at the tail (the
/// merge phi keeps the store's use count above zero).
#[test]
fn dse_refuses_partial_kill() {
    let eliminated = assert_dse_sound("c = 1\nx = 1\nif c { x = 2 }\nx");
    assert_eq!(eliminated, 0, "a store live on the fallthrough path must survive");
}

/// A "pure" op RHS is never a candidate: `10 / d` faults on d = 0, eliminating
/// it would remove the fault. The instance here does not fault so the
/// differential runs; the refusal is what's asserted.
#[test]
fn dse_refuses_faultable_pure_rhs() {
    let eliminated = assert_dse_sound("d = 2\nx = 10 / d\nx = 5\nx");
    assert_eq!(eliminated, 0, "an op RHS (faultable) must not be a candidate");
}

#[test]
fn dse_refuses_call_rhs() {
    let eliminated = assert_dse_sound("f = () => { 1 }\nx = f()\nx = 2\nx");
    assert_eq!(eliminated, 0, "a call RHS must not be a candidate");
}

/// Unpacking defines elements of the RHS, not the RHS value: never a
/// candidate, even with a transparent Ref RHS.
#[test]
fn dse_refuses_unpack() {
    let eliminated = assert_dse_sound("t = [1, 2]\n(x, y) = t\nx = 5\ny = 6\nx + y");
    assert_eq!(eliminated, 0, "an unpack store must not be a candidate");
}

/// An op instruction in the window is a barrier: its evaluation can fault or
/// dispatch an overload that reads the target by name.
#[test]
fn dse_refuses_op_in_window() {
    let eliminated = assert_dse_sound("a = 1\nb = 2\nx = 1\ny = a + b\nx = 3\nx + y");
    assert_eq!(eliminated, 0, "an op in the window must act as a barrier");
}

/// The kill itself must be transparent: `x = a + 1` can fault before the
/// assignment lands, leaving the eliminated store's value observable.
#[test]
fn dse_refuses_op_kill() {
    let eliminated = assert_dse_sound("a = 1\nx = 1\nx = a + 1\nx");
    assert_eq!(eliminated, 0, "a kill with an op RHS must act as a barrier");
}

/// THE call-barrier case: f reads x by name at call time (late binding), so
/// `x = 1` is observable between its def and its kill even with zero SSA uses.
/// Structural only -- closures do not capture under this pre-semantic harness
/// (same precedent as licm_refuses_call_nested_in_pure_rhs); the observable
/// effect is asserted through the analyzer in gate_dse_differential.
#[test]
fn dse_refuses_call_barrier() {
    let (eliminated, _) = dse_round_trip("f = () => { x }\nx = 1\ng = f()\nx = 2\ng + x");
    assert_eq!(eliminated, 0, "a call in the window must act as a barrier");
}

/// An opaque standalone block in the window is a barrier (it reads by name).
#[test]
fn dse_refuses_opaque_block_barrier() {
    let (eliminated, _) = dse_round_trip("x = 1\n{ 5 }\nx = 2\nx");
    assert_eq!(eliminated, 0, "an opaque block in the window must act as a barrier");
}

/// No kill anywhere: the store survives (the trailing expression is a barrier
/// and the exit refuses).
#[test]
fn dse_keeps_unkilled_store() {
    let eliminated = assert_dse_sound("x = 1\ny = 2\ny");
    assert_eq!(eliminated, 0, "a store with no kill must survive");
}

/// Stores inside match arms are not candidates: reconstruction re-emits the
/// preserved OpMatch, a Nop in an arm block would not materialize and
/// `eliminated` would over-report.
#[test]
fn dse_match_arm_stores_not_candidates() {
    let eliminated = assert_dse_sound("x = 2\nmatch x { 1 => { y = 1\ny = 2 } _ => { y = 3 } }\n7");
    assert_eq!(eliminated, 0, "match arm stores must not be candidates");
}

/// Loop bodies are real candidates: reconstruction rebuilds them from the CFG
/// blocks, so an in-body overwritten store dies for real. The second store
/// survives (barriered by the op instruction `s = s + 1`).
#[test]
fn dse_in_loop_body() {
    let eliminated = assert_dse_sound("s = 0\nwhile s < 3 { t = 1\nt = 2\ns = s + 1 }\ns");
    assert_eq!(eliminated, 1, "the overwritten in-body store must die");
}

/// DSE through the analyzer gate: kills through the full pipeline, call
/// barriers with observable closure effects (the cases the raw-IR harness
/// cannot execute), late-bound double reads, faultable-RHS refusals.
#[test]
fn gate_dse_differential() {
    for source in [
        "x = 1\nx = 2\nx",
        "a = 1\nb = a\nb = 0\na = 2\na + b",
        "c = 1\nx = 1\nif c { x = 2 } else { x = 3 }\nx",
        // Call barrier: f reads x late-bound between store and kill; without
        // the barrier g becomes None and the program faults.
        "f = () => { x }\nx = 1\ng = f()\nx = 2\ng + x",
        // Closure mutation between store and kill: r captures the bumped value.
        "bump = () => { x = x + 1\nx }\nx = 1\nr = bump()\nx = 10\nr + x",
        // Late-binding double read (regression net for creation-point counting).
        "x = 1\nf = () => { x }\na = f()\nx = 2\nb = f()\na + b",
        // Faultable RHS is not a candidate.
        "d = 2\nx = 10 / d\nx = 5\nx",
    ] {
        let off = run_with_cfg(source, false);
        let on = run_with_cfg(source, true);
        assert_eq!(off, on, "DSE changed the result for {source:?}: {off:?} vs {on:?}");
    }
}
