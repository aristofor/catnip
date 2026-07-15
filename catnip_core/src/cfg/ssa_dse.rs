// FILE: catnip_core/src/cfg/ssa_dse.rs
//! Global Dead Store Elimination on SSA form -- sound v1.
//!
//! A store is eliminated only when three independent conditions hold:
//!
//! 1. **Zero SSA uses** -- no tracked reader (instruction operands, live phi
//!    operands, values current at the exit block). Iterated to fixpoint:
//!    removing a store may drop its operand's last use.
//! 2. **Transparent store** -- a single-target `SetLocals` whose RHS is a
//!    scalar literal or a `Ref` to a statically-assigned name. Nothing else:
//!    a "pure" opcode can fault (`10/d`, `int + str`) and can dispatch a user
//!    operator overload that reads any variable by name, so eliminating it
//!    would remove a fault or an observable read. A `Ref` to a name with no
//!    static assignment would raise `NameError`, hence the intern check.
//! 3. **Killed on all paths** -- walking forward from the store, every
//!    execution path reaches a redefinition of the target (the *kill*) before
//!    anything can observe the stored value. The window between store and kill
//!    must be transparent, and the kill itself must be a transparent store:
//!    a call in the window can read the target by name (closures are
//!    late-bound), an op can fault and expose the divergent environment to a
//!    post-abort observer (REPL keeps the scope), and a faulting kill leaves
//!    the store's value observable. Block conditions crossed by the window
//!    evaluate an expression, so they must be provably free of user dispatch
//!    and faults: a scalar literal, or a `Ref` to a name whose every static
//!    definition is a scalar literal (native truthiness). Loop and match
//!    headers carry their preserved op as an instruction, so they are natural
//!    barriers -- the window never enters a loop or a match.
//!
//! Chained kills compose by induction: a kill that is itself eliminated was
//! only eliminated because *its* window to the next kill is transparent, so
//! the concatenated window stays transparent. The fixpoint only decrements
//! use counts (monotone), so the final dead set is unique regardless of
//! iteration order -- compilation stays reproducible for the cache.
//!
//! Match arm blocks are excluded as candidates: reconstruction re-emits the
//! preserved `OpMatch` from the header and never walks the arm blocks, so a
//! Nop there would not materialize and `eliminated` would over-report. Loop
//! bodies are real candidates -- reconstruction rebuilds them from the CFG
//! blocks (only the loop scaffolding comes from the preserved op).
//!
//! Traced extensions (not v1): ops proven non-faulting as RHS/kill/condition
//! (needs type information -- the type-hints track), following `break` edges
//! instead of treating `OpBreak` as a barrier, copy-chain and phi-aware
//! scalar proofs for conditions.

use super::edge::EdgeType;
use super::graph::ControlFlowGraph;
use super::ssa::{SSAContext, ValueDef};
use super::ssa_builder::{node_opcode, setlocals_names};
use super::ssa_cse::is_single_target;
use crate::ir::{IR, IROpCode};
use std::collections::{HashMap, HashSet};

/// Result of global DSE.
pub struct DSEResult {
    /// Number of stores eliminated
    pub eliminated: usize,
    /// Dead instructions: (block_id, instr_idx)
    pub dead: HashSet<(usize, usize)>,
}

/// Whether the RHS at `args[1]` is a scalar literal (evaluating it cannot
/// fault, dispatch user code, or allocate anything mutable).
fn rhs_is_scalar_literal(args: &[IR]) -> bool {
    matches!(
        args.get(1),
        Some(IR::Int(_) | IR::Float(_) | IR::Bool(_) | IR::String(_) | IR::None)
    )
}

/// The single target name of a *transparent* store, `None` otherwise.
///
/// Transparent = single-target `SetLocals` whose RHS is a scalar literal or a
/// `Ref` to an interned (statically assigned) name. Evaluating such a store
/// cannot fault, cannot run user code, and reads nothing by name at a distance
/// -- it is invisible to every untracked observer. This single predicate
/// serves both roles: elimination candidate and window transparency.
fn transparent_store_target(op: &IR, ssa: &SSAContext) -> Option<String> {
    if node_opcode(op) != Some(IROpCode::SetLocals) || !is_single_target(op) {
        return None;
    }
    let IR::Op { args, .. } = op else {
        return None;
    };
    let rhs_ok = match args.get(1) {
        Some(IR::Ref(name, _, _)) => ssa.vars.id(name).is_some(),
        _ => rhs_is_scalar_literal(args),
    };
    if !rhs_ok {
        return None;
    }
    let mut names = setlocals_names(op);
    if names.len() == 1 { names.pop() } else { None }
}

/// Collect target names of every `SetLocals` in a subtree (nested defs inside
/// lambda/function bodies, preserved control-flow ops, opaque blocks).
///
/// Traversal mirrors `collect_refs` so no IR variant hosting code is missed.
fn collect_defs_deep(ir: &IR, out: &mut HashSet<String>) {
    match ir {
        IR::Op {
            opcode, args, kwargs, ..
        } => {
            if *opcode == IROpCode::SetLocals {
                for name in setlocals_names(ir) {
                    out.insert(name);
                }
            }
            for arg in args {
                collect_defs_deep(arg, out);
            }
            for (_, v) in kwargs {
                collect_defs_deep(v, out);
            }
        }
        IR::Call { func, args, kwargs, .. } => {
            collect_defs_deep(func, out);
            for arg in args {
                collect_defs_deep(arg, out);
            }
            for (_, v) in kwargs {
                collect_defs_deep(v, out);
            }
        }
        IR::Program(items) | IR::List(items) | IR::Tuple(items) | IR::Set(items) => {
            for item in items {
                collect_defs_deep(item, out);
            }
        }
        IR::Dict(pairs) => {
            for (k, v) in pairs {
                collect_defs_deep(k, out);
                collect_defs_deep(v, out);
            }
        }
        IR::PatternLiteral(v) => collect_defs_deep(v, out),
        IR::PatternOr(ps) | IR::PatternTuple(ps) => {
            for p in ps {
                collect_defs_deep(p, out);
            }
        }
        IR::Slice { start, stop, step } => {
            collect_defs_deep(start, out);
            collect_defs_deep(stop, out);
            collect_defs_deep(step, out);
        }
        _ => {}
    }
}

/// Names safe to evaluate as a crossed block condition: every block-level
/// definition is a transparent scalar-literal store, and the name is never
/// defined inside a nested body (a closure could rebind it to a value whose
/// truthiness dispatches user code). Whole-name over-approximation: no
/// per-version reasoning, any non-literal def anywhere disqualifies the name.
fn scalar_cond_names(cfg: &ControlFlowGraph) -> HashSet<String> {
    let mut all_scalar: HashMap<String, bool> = HashMap::new();
    let mut nested: HashSet<String> = HashSet::new();
    for block in cfg.blocks.values() {
        for op in &block.instructions {
            if node_opcode(op) == Some(IROpCode::SetLocals) {
                let scalar_rhs = if let IR::Op { args, .. } = op {
                    is_single_target(op) && rhs_is_scalar_literal(args)
                } else {
                    false
                };
                for name in setlocals_names(op) {
                    let entry = all_scalar.entry(name).or_insert(true);
                    *entry = *entry && scalar_rhs;
                }
                // Defs can hide inside the RHS (a lambda body); targets
                // (args[0]) are plain refs, skipping them costs nothing.
                if let IR::Op { args, .. } = op {
                    for arg in args.iter().skip(1) {
                        collect_defs_deep(arg, &mut nested);
                    }
                }
            } else {
                collect_defs_deep(op, &mut nested);
            }
        }
        if let Some(cond) = &block.condition {
            collect_defs_deep(cond, &mut nested);
        }
    }
    all_scalar
        .into_iter()
        .filter(|(name, scalar)| *scalar && !nested.contains(name))
        .map(|(name, _)| name)
        .collect()
}

/// Whether a crossed block condition is safe to evaluate inside the window:
/// a scalar literal, or a `Ref` to a proven scalar name (native truthiness,
/// no dispatch, no fault). Comparisons are NOT admitted even on scalars --
/// `int < str` faults, and without type information the window would expose
/// the divergent environment to a post-abort observer.
fn cond_is_transparent(cond: &IR, scalar_names: &HashSet<String>) -> bool {
    match cond {
        IR::Int(_) | IR::Float(_) | IR::Bool(_) | IR::String(_) | IR::None => true,
        IR::Ref(name, _, _) => scalar_names.contains(name),
        _ => false,
    }
}

/// Forward walk from just after the store: `true` iff every path reaches a
/// transparent redefinition of `target` (the kill) through a fully transparent
/// window. Any barrier instruction, non-transparent condition, `Return` edge,
/// or path reaching the exit block refuses the whole candidate.
fn all_paths_killed(
    cfg: &ControlFlowGraph,
    ssa: &SSAContext,
    start_block: usize,
    start_idx: usize,
    target: &str,
    scalar_names: &HashSet<String>,
) -> bool {
    let Some(exit) = cfg.exit else {
        return false;
    };
    let mut work: Vec<(usize, usize)> = vec![(start_block, start_idx)];
    // Non-initial entries always start at instruction 0; visiting a block once
    // is enough (cycles all cross a preserved loop-header op, a barrier).
    let mut visited: HashSet<usize> = HashSet::new();

    while let Some((block_id, idx)) = work.pop() {
        let Some(block) = cfg.blocks.get(&block_id) else {
            return false;
        };

        let mut killed = false;
        for op in block.instructions.iter().skip(idx) {
            if node_opcode(op) == Some(IROpCode::Nop) {
                continue;
            }
            match transparent_store_target(op, ssa) {
                Some(name) if name == target => {
                    killed = true;
                    break;
                }
                Some(_) => continue,
                None => return false, // barrier
            }
        }
        if killed {
            continue;
        }

        // End of block without a kill: the condition (if any) evaluates, then
        // control flows to the successors.
        if let Some(cond) = &block.condition {
            if !cond_is_transparent(cond, scalar_names) {
                return false;
            }
        }
        if block.successors.is_empty() {
            return false; // end of program, value survives
        }
        for &edge_id in &block.successors {
            let edge = &cfg.edges[edge_id];
            if matches!(edge.edge_type, EdgeType::Return) {
                return false;
            }
            if edge.target == exit {
                return false; // observable at exit
            }
            if visited.insert(edge.target) {
                work.push((edge.target, 0));
            }
        }
    }
    true
}

/// Blocks belonging to a match arm: reconstruction re-emits the preserved
/// `OpMatch` and never walks these blocks, so a Nop there would not
/// materialize. DFS from the header's `ConditionalTrue` successors, stopping
/// at the recorded merge, not following break/continue/return edges (the same
/// traversal rule as reconstruction).
fn match_arm_blocks(cfg: &ControlFlowGraph) -> HashSet<usize> {
    let mut arms: HashSet<usize> = HashSet::new();
    for block in cfg.blocks.values() {
        let Some(merge) = block.match_merge else {
            continue;
        };
        if !block
            .instructions
            .iter()
            .any(|op| node_opcode(op) == Some(IROpCode::OpMatch))
        {
            continue;
        }
        let mut stack: Vec<usize> = block
            .successors
            .iter()
            .map(|&eid| &cfg.edges[eid])
            .filter(|e| matches!(e.edge_type, EdgeType::ConditionalTrue))
            .map(|e| e.target)
            .collect();
        while let Some(b) = stack.pop() {
            if b == merge || !arms.insert(b) {
                continue;
            }
            let Some(bb) = cfg.blocks.get(&b) else {
                continue;
            };
            for &eid in &bb.successors {
                let edge = &cfg.edges[eid];
                if matches!(edge.edge_type, EdgeType::Break | EdgeType::Continue | EdgeType::Return) {
                    continue;
                }
                stack.push(edge.target);
            }
        }
    }
    arms
}

/// Run global dead store elimination on the CFG in SSA form.
pub fn global_dse(cfg: &ControlFlowGraph, ssa: &SSAContext) -> DSEResult {
    // Step 1: Count uses for each SSA value
    let mut use_counts: HashMap<super::ssa::SSAValue, usize> = HashMap::new();

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

    let arms = match_arm_blocks(cfg);
    let scalar_names = scalar_cond_names(cfg);

    // Step 2: Iterative elimination to fixpoint
    let mut dead: HashSet<(usize, usize)> = HashSet::new();
    let mut changed = true;

    while changed {
        changed = false;

        for (value, def) in &ssa.value_defs {
            let uses = use_counts.get(value).copied().unwrap_or(0);
            if uses > 0 {
                continue;
            }

            let ValueDef::Instruction { block, instr_idx } = def else {
                continue;
            };
            let key = (*block, *instr_idx);
            if dead.contains(&key) || arms.contains(block) {
                continue;
            }

            let Some(op) = cfg.blocks.get(block).and_then(|b| b.instructions.get(*instr_idx)) else {
                continue;
            };
            let Some(target) = transparent_store_target(op, ssa) else {
                continue;
            };
            if !all_paths_killed(cfg, ssa, *block, instr_idx + 1, &target, &scalar_names) {
                continue;
            }

            dead.insert(key);
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

    DSEResult {
        eliminated: dead.len(),
        dead,
    }
}

/// Apply DSE results to the CFG by replacing dead instructions with Nop.
pub fn apply_dse(cfg: &mut ControlFlowGraph, result: &DSEResult) {
    for &(block_id, instr_idx) in &result.dead {
        if let Some(block) = cfg.get_block_mut(block_id) {
            if instr_idx < block.instructions.len() {
                if let IR::Op { opcode, .. } = &mut block.instructions[instr_idx] {
                    *opcode = IROpCode::Nop;
                }
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

    fn store(name: &str, rhs: IR) -> IR {
        IR::op(
            IROpCode::SetLocals,
            vec![IR::Tuple(vec![IR::Ref(name.into(), -1, -1)]), rhs, IR::Bool(false)],
        )
    }

    /// entry -> exit CFG with the given entry instructions.
    fn linear_cfg(instructions: Vec<IR>) -> ControlFlowGraph {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);
        if let Some(block) = cfg.get_block_mut(entry) {
            for op in instructions {
                block.add_instruction(op);
            }
        }
        compute_dominators(&mut cfg);
        cfg
    }

    #[test]
    fn test_dse_empty_cfg() {
        let cfg = linear_cfg(Vec::new());
        let ssa = SSABuilder::build(&cfg);

        let result = global_dse(&cfg, &ssa);
        assert_eq!(result.eliminated, 0);
        assert!(result.dead.is_empty());
    }

    /// `x = 1; x = 2`: the first store is killed in-block, the second reaches
    /// the exit and survives.
    #[test]
    fn test_dse_overwritten_literal_store() {
        let cfg = linear_cfg(vec![store("x", IR::Int(1)), store("x", IR::Int(2))]);
        let ssa = SSABuilder::build(&cfg);

        let result = global_dse(&cfg, &ssa);
        assert_eq!(result.eliminated, 1);
        let entry = cfg.entry.unwrap();
        assert!(result.dead.contains(&(entry, 0)));
    }

    /// An op RHS is never a candidate, even a "pure" one: it can fault or
    /// dispatch an overload.
    #[test]
    fn test_dse_refuses_op_rhs() {
        let rhs = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        let cfg = linear_cfg(vec![store("x", rhs), store("x", IR::Int(2))]);
        let ssa = SSABuilder::build(&cfg);

        let result = global_dse(&cfg, &ssa);
        assert_eq!(result.eliminated, 0);
    }

    /// A kill with an op RHS is a barrier: its evaluation can fault or read
    /// the target by name before the assignment lands.
    #[test]
    fn test_dse_refuses_op_kill() {
        let rhs = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        let cfg = linear_cfg(vec![store("x", IR::Int(1)), store("x", rhs)]);
        let ssa = SSABuilder::build(&cfg);

        let result = global_dse(&cfg, &ssa);
        assert_eq!(result.eliminated, 0);
    }

    /// Transparency of the predicate itself.
    #[test]
    fn test_transparent_store_predicate() {
        let cfg = linear_cfg(vec![store("a", IR::Int(1))]);
        let ssa = SSABuilder::build(&cfg);

        assert_eq!(
            transparent_store_target(&store("x", IR::Int(1)), &ssa),
            Some("x".to_string())
        );
        // Ref to an interned name (a is assigned in the CFG)
        assert_eq!(
            transparent_store_target(&store("x", IR::Ref("a".into(), -1, -1)), &ssa),
            Some("x".to_string())
        );
        // Ref to an unknown name would raise NameError: not transparent
        assert_eq!(
            transparent_store_target(&store("x", IR::Ref("nope".into(), -1, -1)), &ssa),
            None
        );
        // Op RHS: not transparent
        let op_rhs = store("x", IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]));
        assert_eq!(transparent_store_target(&op_rhs, &ssa), None);
        // Unpack: not transparent
        let unpack = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("a".into(), -1, -1), IR::Ref("b".into(), -1, -1)]),
                IR::Ref("a".into(), -1, -1),
                IR::Bool(true),
            ],
        );
        assert_eq!(transparent_store_target(&unpack, &ssa), None);
    }
}
