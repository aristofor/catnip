// FILE: catnip_core/src/semantic/passes/dead_store_elimination.rs
//! Dead store elimination pass (pure Rust).
//!
//! Removes redundant assignments that are overwritten before being read:
//! - x = 1; x = 2 → x = 2

use super::{PurePass, collect_refs, map_children};
use crate::ir::{IR, IROpCode};
use std::collections::{HashMap, HashSet};

pub struct DeadStoreEliminationPass;

impl PurePass for DeadStoreEliminationPass {
    fn name(&self) -> &str {
        "dead_store_elimination"
    }

    fn optimize(&mut self, ir: IR) -> IR {
        let visited = map_children(ir, &mut |child| self.optimize(child));
        eliminate_in_container(visited)
    }
}

/// Analyze a container (Block or Program) and remove dead stores
fn eliminate_in_container(ir: IR) -> IR {
    match ir {
        IR::Op {
            opcode: opcode @ IROpCode::OpBlock,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        } => {
            let filtered = eliminate_in_sequence(args);
            IR::Op {
                opcode,
                args: filtered,
                kwargs,
                tail,
                start_byte,
                end_byte,
            }
        }
        IR::Program(items) => IR::Program(eliminate_in_sequence(items)),
        other => other,
    }
}

/// Analyze a sequence of statements and remove dead stores
fn eliminate_in_sequence(stmts: Vec<IR>) -> Vec<IR> {
    if stmts.len() < 2 {
        return stmts;
    }

    // Track: variable → index of last store
    let mut var_last_store: HashMap<String, usize> = HashMap::new();
    // Track: which store indices have been "used" (their value was read)
    let mut read_store_indices: HashSet<usize> = HashSet::new();
    // Track: all store indices per variable
    let mut var_all_stores: HashMap<String, Vec<usize>> = HashMap::new();

    for (idx, stmt) in stmts.iter().enumerate() {
        // Record reads from this statement
        // For SetLocals, only record reads from the value (not the name)
        if let IR::Op {
            opcode: IROpCode::SetLocals,
            args,
            ..
        } = stmt
        {
            // Record reads from value expression (args[1:])
            for arg in args.iter().skip(1) {
                collect_refs(arg, &mut |name| {
                    if let Some(&store_idx) = var_last_store.get(&name) {
                        read_store_indices.insert(store_idx);
                    }
                });
            }
        } else {
            // Record all refs in non-store statements
            collect_refs(stmt, &mut |name| {
                if let Some(&store_idx) = var_last_store.get(&name) {
                    read_store_indices.insert(store_idx);
                }
            });
        }

        // Then record this statement as a store (if applicable)
        if let IR::Op {
            opcode: IROpCode::SetLocals,
            args,
            ..
        } = stmt
        {
            if args.len() >= 2 {
                if let IR::Identifier(ref name) = args[0] {
                    var_all_stores.entry(name.clone()).or_default().push(idx);
                    var_last_store.insert(name.clone(), idx);
                }
            }
        }
    }

    // A store is dead if:
    // 1. The variable has another store later
    // 2. This store's value was never read
    let mut dead: HashSet<usize> = HashSet::new();
    for indices in var_all_stores.values() {
        // Check all stores except the last
        for &idx in &indices[..indices.len().saturating_sub(1)] {
            if !read_store_indices.contains(&idx) {
                dead.insert(idx);
            }
        }
    }

    if dead.is_empty() {
        return stmts;
    }

    stmts
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !dead.contains(i))
        .map(|(_, s)| s)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(ir: IR) -> IR {
        DeadStoreEliminationPass.optimize(ir)
    }

    fn set_local(name: &str, val: IR) -> IR {
        IR::op(IROpCode::SetLocals, vec![IR::Identifier(name.into()), val])
    }

    #[test]
    fn test_remove_dead_store() {
        // x = 1; x = 2 → x = 2
        let s1 = set_local("x", IR::Int(1));
        let s2 = set_local("x", IR::Int(2));
        let block = IR::op(IROpCode::OpBlock, vec![s1, s2.clone()]);
        let result = opt(block);
        assert_eq!(result.args().unwrap(), &[s2]);
    }

    #[test]
    fn test_keep_used_store() {
        // x = 1; y = x; x = 2 → all kept
        let s1 = set_local("x", IR::Int(1));
        let s2 = set_local("y", IR::Ref("x".into(), 0, 1));
        let s3 = set_local("x", IR::Int(2));
        let block = IR::op(IROpCode::OpBlock, vec![s1.clone(), s2.clone(), s3.clone()]);
        let result = opt(block);
        assert_eq!(result.args().unwrap(), &[s1, s2, s3]);
    }

    #[test]
    fn test_keep_single_store() {
        // x = 1 → kept (no overwrite)
        let s1 = set_local("x", IR::Int(1));
        let block = IR::op(IROpCode::OpBlock, vec![s1.clone()]);
        let result = opt(block);
        assert_eq!(result.args().unwrap(), &[s1]);
    }

    #[test]
    fn test_dead_store_in_program() {
        // Same as block but at program level
        let s1 = set_local("x", IR::Int(1));
        let s2 = set_local("x", IR::Int(2));
        let program = IR::Program(vec![s1, s2.clone()]);
        let result = opt(program);
        if let IR::Program(items) = result {
            assert_eq!(items, vec![s2]);
        } else {
            panic!("Expected Program");
        }
    }

    #[test]
    fn test_multiple_vars() {
        // x = 1; y = 2; x = 3 → removes x = 1, keeps y = 2 and x = 3
        let sx1 = set_local("x", IR::Int(1));
        let sy = set_local("y", IR::Int(2));
        let sx2 = set_local("x", IR::Int(3));
        let block = IR::op(IROpCode::OpBlock, vec![sx1, sy.clone(), sx2.clone()]);
        let result = opt(block);
        assert_eq!(result.args().unwrap(), &[sy, sx2]);
    }
}
