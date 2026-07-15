// FILE: catnip_core/src/cfg/liveness.rs
//! Live variable analysis over the CFG (backward dataflow to a fixpoint).
//!
//! Works on raw variable names, independent of SSA form. Def/use extraction
//! mirrors `ssa_builder.rs`: only `SetLocals` defines variables, a `SetLocals`
//! reads only its RHS, and every other op reads all of its args and kwargs.

use super::basic_block::BasicBlock;
use super::graph::ControlFlowGraph;
use super::ssa_builder::{node_opcode, setlocals_names};
use crate::ir::{IR, IROpCode};
use crate::semantic::passes::collect_refs;
use std::collections::{HashMap, HashSet};

/// Per-block live variable sets (by variable name).
pub struct Liveness {
    pub live_in: HashMap<usize, HashSet<String>>,
    pub live_out: HashMap<usize, HashSet<String>>,
}

/// Variables defined by an instruction: only SetLocals contributes.
fn instr_defs(op: &IR) -> Vec<String> {
    if node_opcode(op) == Some(IROpCode::SetLocals) {
        setlocals_names(op)
    } else {
        Vec::new()
    }
}

/// Refs in a single IR subtree.
fn refs_in(ir: &IR) -> Vec<String> {
    let mut names = Vec::new();
    collect_refs(ir, &mut |n| names.push(n));
    names
}

/// Variables read by an instruction, for dataflow purposes.
///
/// SetLocals reads only its RHS (`args[1]`). Control-flow headers (`OpWhile`,
/// `OpFor`) keep the whole loop body nested (op-preservation), but that body is
/// materialized as *separate CFG blocks*; scanning it here would double-count the
/// body's reads while missing its nested defs, making inner-loop variables look
/// live at the outer header. So a loop header contributes only its scaffolding:
/// the `while` condition (`args[0]`) or the `for` iterable (`args[1]`). Every
/// other op reads all of its args and kwargs.
fn instr_uses(op: &IR) -> Vec<String> {
    match node_opcode(op) {
        Some(IROpCode::SetLocals) => match op {
            IR::Op { args, .. } => args.get(1).map(refs_in).unwrap_or_default(),
            _ => Vec::new(),
        },
        Some(IROpCode::OpWhile) => match op {
            IR::Op { args, .. } => args.first().map(refs_in).unwrap_or_default(),
            _ => Vec::new(),
        },
        Some(IROpCode::OpFor) => match op {
            IR::Op { args, .. } => args.get(1).map(refs_in).unwrap_or_default(),
            _ => Vec::new(),
        },
        _ => match op {
            IR::Op { args, kwargs, .. } => {
                let mut names = Vec::new();
                for a in args {
                    collect_refs(a, &mut |n| names.push(n));
                }
                for (_, v) in kwargs {
                    collect_refs(v, &mut |n| names.push(n));
                }
                names
            }
            other => refs_in(other),
        },
    }
}

/// Local def and upward-exposed use sets for one block.
///
/// A use is upward-exposed when the variable is read before any assignment to
/// it earlier in the block; those are the only uses visible to predecessors.
fn block_def_use(block: &BasicBlock) -> (HashSet<String>, HashSet<String>) {
    let mut def = HashSet::new();
    let mut used = HashSet::new();
    let mut defined_so_far: HashSet<String> = HashSet::new();

    for op in &block.instructions {
        for name in instr_uses(op) {
            if !defined_so_far.contains(&name) {
                used.insert(name);
            }
        }
        for name in instr_defs(op) {
            defined_so_far.insert(name.clone());
            def.insert(name);
        }
    }

    (def, used)
}

/// Compute variable liveness over the CFG by iterative backward dataflow.
pub fn compute_liveness(cfg: &ControlFlowGraph) -> Liveness {
    // Local def/use per block.
    let mut def: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut use_: HashMap<usize, HashSet<String>> = HashMap::new();
    for (&bid, block) in &cfg.blocks {
        let (d, u) = block_def_use(block);
        def.insert(bid, d);
        use_.insert(bid, u);
    }

    let mut live_in: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut live_out: HashMap<usize, HashSet<String>> = HashMap::new();
    for &bid in cfg.blocks.keys() {
        live_in.insert(bid, HashSet::new());
        live_out.insert(bid, HashSet::new());
    }

    // Backward equations, iterated until no set changes:
    //   live_out[B] = ∪ live_in[S]  over successors S
    //   live_in[B]  = use[B] ∪ (live_out[B] \ def[B])
    let mut changed = true;
    while changed {
        changed = false;
        for (&bid, block) in &cfg.blocks {
            // Successors: block.successors are edge indices; the successor block
            // id is cfg.edges[edge_id].target (mirror of get_predecessor_blocks).
            let mut new_out = HashSet::new();
            for &edge_id in &block.successors {
                if let Some(edge) = cfg.edges.get(edge_id) {
                    if let Some(succ_in) = live_in.get(&edge.target) {
                        new_out.extend(succ_in.iter().cloned());
                    }
                }
            }

            let mut new_in = use_[&bid].clone();
            let d = &def[&bid];
            for v in &new_out {
                if !d.contains(v) {
                    new_in.insert(v.clone());
                }
            }

            if live_out.get(&bid) != Some(&new_out) {
                live_out.insert(bid, new_out);
                changed = true;
            }
            if live_in.get(&bid) != Some(&new_in) {
                live_in.insert(bid, new_in);
                changed = true;
            }
        }
    }

    Liveness { live_in, live_out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::edge::EdgeType;
    use crate::ir::IROpCode;

    /// SetLocals `target = rhs` in the same 3-arg layout the parser emits.
    fn set_local(target: &str, rhs: IR) -> IR {
        IR::op(
            IROpCode::SetLocals,
            vec![IR::Identifier(target.into()), rhs, IR::Bool(false)],
        )
    }

    /// Push an instruction into a block.
    fn push(cfg: &mut ControlFlowGraph, block: usize, op: IR) {
        cfg.get_block_mut(block).unwrap().add_instruction(op);
    }

    #[test]
    fn test_linear_def_then_use() {
        // A: x = 5   B: x + 1     A -> B
        let mut cfg = ControlFlowGraph::new("linear");
        let a = cfg.create_block("a");
        let b = cfg.create_block("b");
        cfg.set_entry(a);
        cfg.set_exit(b);
        cfg.add_edge(a, b, EdgeType::Fallthrough);

        push(&mut cfg, a, set_local("x", IR::Int(5)));
        push(
            &mut cfg,
            b,
            IR::op(IROpCode::Add, vec![IR::Ref("x".into(), -1, -1), IR::Int(1)]),
        );

        let live = compute_liveness(&cfg);

        // x flows from its def in A across the edge to its use in B.
        assert!(live.live_out[&a].contains("x"));
        assert!(live.live_in[&b].contains("x"));
        // x is defined in A, not upward-exposed there.
        assert!(!live.live_in[&a].contains("x"));
    }

    #[test]
    fn test_dead_def_not_live_out() {
        // A: y = 5 (dead); z = 5     B: z + 1     A -> B
        let mut cfg = ControlFlowGraph::new("dead");
        let a = cfg.create_block("a");
        let b = cfg.create_block("b");
        cfg.set_entry(a);
        cfg.set_exit(b);
        cfg.add_edge(a, b, EdgeType::Fallthrough);

        push(&mut cfg, a, set_local("y", IR::Int(5)));
        push(&mut cfg, a, set_local("z", IR::Int(5)));
        push(
            &mut cfg,
            b,
            IR::op(IROpCode::Add, vec![IR::Ref("z".into(), -1, -1), IR::Int(1)]),
        );

        let live = compute_liveness(&cfg);

        // y is never read: not live at the block exit. z is.
        assert!(!live.live_out[&a].contains("y"));
        assert!(live.live_out[&a].contains("z"));
    }

    #[test]
    fn test_loop_variable_liveness() {
        // entry: i = 0
        // header: i < 10           header -> body, header -> exit
        // body:   i = i + 1        body -> header (back edge)
        let mut cfg = ControlFlowGraph::new("loop");
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

        push(&mut cfg, entry, set_local("i", IR::Int(0)));
        push(
            &mut cfg,
            header,
            IR::op(IROpCode::Lt, vec![IR::Ref("i".into(), -1, -1), IR::Int(10)]),
        );
        push(
            &mut cfg,
            body,
            set_local(
                "i",
                IR::op(IROpCode::Add, vec![IR::Ref("i".into(), -1, -1), IR::Int(1)]),
            ),
        );

        let live = compute_liveness(&cfg);

        // i is live entering the header (read by the condition, and again after
        // the back edge) and live leaving the body (needed by the next header).
        assert!(live.live_in[&header].contains("i"));
        assert!(live.live_out[&body].contains("i"));
    }
}
