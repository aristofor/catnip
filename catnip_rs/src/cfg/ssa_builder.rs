// FILE: catnip_rs/src/cfg/ssa_builder.rs
//! SSA construction from CFG.
//!
//! Walks the CFG in reverse postorder, detects variable definitions (SetLocals)
//! and uses (Ref/identifiers), and builds SSA form via Braun's algorithm.

use super::graph::ControlFlowGraph;
use super::ssa::SSAContext;
use crate::core::nodes::Ref;
use crate::core::op::Op;
use crate::ir::opcode::IROpCode;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use std::collections::HashSet;

/// Recursively extract variable names referenced (Ref nodes) in a Python object tree.
///
/// Walks Op.args, PyTuple elements, and Ref nodes. Returns the `ident` of each Ref found.
fn extract_refs(py: Python<'_>, obj: &Bound<'_, PyAny>) -> Vec<String> {
    let mut refs = Vec::new();
    extract_refs_inner(py, obj, &mut refs);
    refs
}

fn extract_refs_inner(py: Python<'_>, obj: &Bound<'_, PyAny>, refs: &mut Vec<String>) {
    // Ref node → collect ident
    if let Ok(r) = obj.extract::<PyRef<Ref>>() {
        refs.push(r.ident.clone());
        return;
    }

    // Op node → recurse into args
    if let Ok(op) = obj.extract::<PyRef<Op>>() {
        let args = op.args.bind(py);
        extract_refs_inner(py, args, refs);
        return;
    }

    // Tuple → recurse into elements
    if let Ok(tuple) = obj.cast::<PyTuple>() {
        for item in tuple.iter() {
            extract_refs_inner(py, &item, refs);
        }
        return;
    }

    // Literals (int, float, str, bool, None) → ignore
}

/// Build SSA form from a CFG.
pub struct SSABuilder;

impl SSABuilder {
    /// Build SSA context from a CFG.
    ///
    /// The CFG must have dominators computed before calling this.
    pub fn build(cfg: &ControlFlowGraph) -> SSAContext {
        let mut ssa = SSAContext::new();

        // Initialize all blocks
        for &block_id in cfg.blocks.keys() {
            ssa.ensure_block(block_id);
        }

        // Phase 0: pre-intern all variable names from SetLocals.
        // Ensures vars.id(name) works for variables defined in loop bodies
        // even when processing loop headers first (RPO visits headers before bodies).
        Self::pre_intern_variables(cfg, &mut ssa);

        // Compute reverse postorder for traversal
        let rpo = Self::reverse_postorder(cfg);

        // Detect loop headers for deferred sealing
        let loop_headers = Self::find_loop_headers(cfg);

        // Phase 1: seal non-loop-header blocks and process definitions/uses
        for &block_id in &rpo {
            if !loop_headers.contains(&block_id) {
                ssa.seal_block(cfg, block_id);
            }
            Self::process_block(cfg, &mut ssa, block_id);
        }

        // Phase 2: seal loop headers (all predecessors now processed)
        for &header in &rpo {
            if loop_headers.contains(&header) {
                ssa.seal_block(cfg, header);
            }
        }

        ssa
    }

    /// Pre-intern all variable names from SetLocals across the CFG.
    ///
    /// Guarantees that `vars.id(name)` succeeds for every local variable,
    /// even when RPO visits a loop header before the body that defines variables.
    fn pre_intern_variables(cfg: &ControlFlowGraph, ssa: &mut SSAContext) {
        Python::attach(|py| {
            for block in cfg.blocks.values() {
                for op in &block.instructions {
                    if op.ident != IROpCode::SetLocals as i32 {
                        continue;
                    }
                    let args = op.get_args();
                    let args_bound = args.bind(py);
                    if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                        if args_tuple.len() >= 1 {
                            if let Ok(names) = args_tuple.get_item(0) {
                                if let Ok(names_tuple) = names.cast::<PyTuple>() {
                                    for name_item in names_tuple.iter() {
                                        if let Ok(name) = name_item.extract::<String>() {
                                            ssa.vars.intern(&name);
                                        }
                                    }
                                } else if let Ok(name) = names.extract::<String>() {
                                    ssa.vars.intern(&name);
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    /// Process a single block: scan uses then defs for each instruction.
    ///
    /// Order matters: uses BEFORE defs (SSA semantics -- RHS evaluated before LHS defined).
    fn process_block(cfg: &ControlFlowGraph, ssa: &mut SSAContext, block_id: usize) {
        let Some(block) = cfg.blocks.get(&block_id) else {
            return;
        };

        // Snapshot initial_defs (reaching defs at block entry, before any instruction)
        if let Some(info) = ssa.blocks.get(&block_id) {
            let snapshot = info.current_defs.clone();
            ssa.blocks.get_mut(&block_id).unwrap().initial_defs = snapshot;
        }

        for (instr_idx, op) in block.instructions.iter().enumerate() {
            // 1. Scan uses (resolves Ref → SSAValue, records in instruction_uses)
            Self::scan_uses(cfg, ssa, block_id, instr_idx, op);

            // 2. Process defs (SetLocals creates new SSA versions)
            if op.ident == IROpCode::SetLocals as i32 {
                Self::process_set_locals(cfg, ssa, block_id, instr_idx, op);
            }
        }
    }

    /// Process a SetLocals instruction: extract variable names and define them.
    fn process_set_locals(
        _cfg: &ControlFlowGraph,
        ssa: &mut SSAContext,
        block_id: usize,
        instr_idx: usize,
        op: &crate::core::op::Op,
    ) {
        Python::attach(|py| {
            let args = op.get_args();
            let args_bound = args.bind(py);

            if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                if args_tuple.len() >= 1 {
                    // args[0] = names (tuple of strings)
                    if let Ok(names) = args_tuple.get_item(0) {
                        if let Ok(names_tuple) = names.cast::<PyTuple>() {
                            for name_item in names_tuple.iter() {
                                if let Ok(name) = name_item.extract::<String>() {
                                    ssa.def_var(block_id, &name, instr_idx);
                                }
                            }
                        } else if let Ok(name) = names.extract::<String>() {
                            // Single name (not wrapped in tuple)
                            ssa.def_var(block_id, &name, instr_idx);
                        }
                    }
                }
            }
        });
    }

    /// Scan an instruction's args for variable references and resolve to SSA values.
    ///
    /// For SetLocals, scans only the RHS (args[1]) -- the LHS names are defs, not uses.
    /// For other ops, scans all args.
    /// Records resolved SSAValues in `ssa.instruction_uses`.
    fn scan_uses(
        cfg: &ControlFlowGraph,
        ssa: &mut SSAContext,
        block_id: usize,
        instr_idx: usize,
        op: &crate::core::op::Op,
    ) {
        Python::attach(|py| {
            let args = op.get_args();
            let args_bound = args.bind(py);

            let ref_names = if op.ident == IROpCode::SetLocals as i32 {
                // SetLocals: args = (names, rhs, unpack) -- scan only RHS
                if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                    if args_tuple.len() >= 2 {
                        if let Ok(rhs) = args_tuple.get_item(1) {
                            extract_refs(py, &rhs)
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                extract_refs(py, &args_bound)
            };

            // Resolve each Ref name to its reaching SSA definition
            let mut uses = Vec::with_capacity(ref_names.len());
            for name in &ref_names {
                if ssa.vars.id(name).is_some() {
                    let val = ssa.use_var(cfg, block_id, name);
                    uses.push(val);
                }
            }

            if !uses.is_empty() {
                ssa.record_uses(block_id, instr_idx, uses);
            }
        });
    }

    /// Compute reverse postorder of the CFG.
    fn reverse_postorder(cfg: &ControlFlowGraph) -> Vec<usize> {
        let Some(entry) = cfg.entry else {
            return Vec::new();
        };

        let mut visited = HashSet::new();
        let mut postorder = Vec::new();

        Self::dfs_postorder(cfg, entry, &mut visited, &mut postorder);

        postorder.reverse();
        postorder
    }

    /// DFS for postorder computation.
    fn dfs_postorder(
        cfg: &ControlFlowGraph,
        block: usize,
        visited: &mut HashSet<usize>,
        postorder: &mut Vec<usize>,
    ) {
        if !visited.insert(block) {
            return;
        }

        if let Some(b) = cfg.blocks.get(&block) {
            for &edge_id in &b.successors {
                if let Some(edge) = cfg.edges.get(edge_id) {
                    Self::dfs_postorder(cfg, edge.target, visited, postorder);
                }
            }
        }

        postorder.push(block);
    }

    /// Find loop headers (blocks that are targets of back edges).
    fn find_loop_headers(cfg: &ControlFlowGraph) -> HashSet<usize> {
        let mut headers = HashSet::new();

        for edge in &cfg.edges {
            // A back edge: target dominates source
            if let Some(source_block) = cfg.blocks.get(&edge.source) {
                if source_block.dominators.contains(&edge.target) {
                    headers.insert(edge.target);
                }
            }
        }

        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::analysis::compute_dominators;
    use crate::cfg::edge::EdgeType;

    #[test]
    fn test_reverse_postorder_linear() {
        let mut cfg = ControlFlowGraph::new("test");
        let a = cfg.create_block("a");
        let b = cfg.create_block("b");
        let c = cfg.create_block("c");

        cfg.set_entry(a);
        cfg.set_exit(c);

        cfg.add_edge(a, b, EdgeType::Fallthrough);
        cfg.add_edge(b, c, EdgeType::Fallthrough);

        let rpo = SSABuilder::reverse_postorder(&cfg);
        assert_eq!(rpo, vec![a, b, c]);
    }

    #[test]
    fn test_reverse_postorder_diamond() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let t = cfg.create_block("true");
        let f = cfg.create_block("false");
        let merge = cfg.create_block("merge");

        cfg.set_entry(entry);
        cfg.set_exit(merge);

        cfg.add_edge(entry, t, EdgeType::ConditionalTrue);
        cfg.add_edge(entry, f, EdgeType::ConditionalFalse);
        cfg.add_edge(t, merge, EdgeType::Fallthrough);
        cfg.add_edge(f, merge, EdgeType::Fallthrough);

        let rpo = SSABuilder::reverse_postorder(&cfg);
        // entry must come first, merge must come last
        assert_eq!(rpo[0], entry);
        assert_eq!(*rpo.last().unwrap(), merge);
    }

    #[test]
    fn test_find_loop_headers() {
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

        let headers = SSABuilder::find_loop_headers(&cfg);
        assert!(headers.contains(&header));
        assert_eq!(headers.len(), 1);
    }

    #[test]
    fn test_builder_empty_cfg() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);

        let ssa = SSABuilder::build(&cfg);
        assert_eq!(ssa.phi_count(), 0);
    }
}
