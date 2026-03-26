// FILE: catnip_rs/src/cfg/optimization.rs
//! CFG-based optimizations.

use super::edge::EdgeType;
use super::graph::ControlFlowGraph;
use crate::ir::opcode::IROpCode;

/// Remove unreachable blocks from CFG.
///
/// Dead code elimination: blocks not reachable from entry are removed.
pub fn eliminate_dead_code(cfg: &mut ControlFlowGraph) -> usize {
    let unreachable = cfg.get_unreachable_blocks();
    let count = unreachable.len();

    if !unreachable.is_empty() {
        cfg.remove_unreachable_blocks();
    }

    count
}

/// Merge sequential blocks that can be combined.
///
/// A block B can be merged into A if:
/// - A has exactly one successor (B)
/// - B has exactly one predecessor (A)
/// - Edge type is Fallthrough or Unconditional
pub fn merge_blocks(cfg: &mut ControlFlowGraph) -> usize {
    let mut merged = 0;
    let mut changed = true;

    while changed {
        changed = false;

        // Find pairs to merge
        let block_ids: Vec<_> = cfg.blocks.keys().copied().collect();

        for &block_a_id in &block_ids {
            if !cfg.blocks.contains_key(&block_a_id) {
                continue; // Already merged
            }

            let block_a = match cfg.blocks.get(&block_a_id) {
                Some(b) => b,
                None => continue,
            };

            // A must have exactly one successor
            if block_a.successors.len() != 1 {
                continue;
            }

            let edge_id = block_a.successors[0];
            let edge = &cfg.edges[edge_id];

            // Edge must be Fallthrough or Unconditional
            if !matches!(edge.edge_type, EdgeType::Fallthrough | EdgeType::Unconditional) {
                continue;
            }

            let block_b_id = edge.target;

            // Skip if B is entry or exit
            if Some(block_b_id) == cfg.entry || Some(block_b_id) == cfg.exit {
                continue;
            }

            let block_b = match cfg.blocks.get(&block_b_id) {
                Some(b) => b,
                None => continue,
            };

            // B must have exactly one predecessor
            if block_b.predecessors.len() != 1 {
                continue;
            }

            // Merge B into A
            let b_instructions = block_b.instructions.clone();
            let b_successors = block_b.successors.clone();

            // Add B's instructions to A
            if let Some(block_a_mut) = cfg.blocks.get_mut(&block_a_id) {
                block_a_mut.instructions.extend(b_instructions);
                block_a_mut.successors = b_successors.clone();
            }

            // Update edges: redirect B's successors to come from A
            for &succ_edge_id in &b_successors {
                if let Some(edge) = cfg.edges.get_mut(succ_edge_id) {
                    edge.source = block_a_id;
                }

                // Update successor's predecessor list
                if let Some(edge) = cfg.edges.get(succ_edge_id) {
                    if let Some(succ_block) = cfg.blocks.get_mut(&edge.target) {
                        succ_block
                            .predecessors
                            .retain(|&e| cfg.edges.get(e).map(|ed| ed.source != block_b_id).unwrap_or(true));
                        succ_block.predecessors.push(succ_edge_id);
                    }
                }
            }

            // Remove B
            cfg.blocks.remove(&block_b_id);

            // Remove edge A->B from edge list (mark as invalid)
            // We can't actually remove from Vec without reindexing everything,
            // so we'll leave it and it will be filtered by remove_unreachable_blocks

            merged += 1;
            changed = true;
            break; // Restart from beginning
        }
    }

    // Clean up invalid edges
    if merged > 0 {
        cfg.remove_unreachable_blocks();
    }

    merged
}

/// Simplify CFG by removing empty blocks.
///
/// An empty block with no instructions can be bypassed if:
/// - It has one successor
/// - It's not entry or exit
/// - Edge type is Fallthrough or Unconditional
pub fn remove_empty_blocks(cfg: &mut ControlFlowGraph) -> usize {
    let mut removed = 0;
    let mut changed = true;

    while changed {
        changed = false;

        let block_ids: Vec<_> = cfg.blocks.keys().copied().collect();

        for &block_id in &block_ids {
            if !cfg.blocks.contains_key(&block_id) {
                continue;
            }

            // Skip entry and exit
            if Some(block_id) == cfg.entry || Some(block_id) == cfg.exit {
                continue;
            }

            let block = match cfg.blocks.get(&block_id) {
                Some(b) => b,
                None => continue,
            };

            // Must be empty
            if !block.instructions.is_empty() {
                continue;
            }

            // Must have exactly one successor
            if block.successors.len() != 1 {
                continue;
            }

            let edge_id = block.successors[0];
            let edge = &cfg.edges[edge_id];

            // Edge must be Fallthrough or Unconditional
            if !matches!(edge.edge_type, EdgeType::Fallthrough | EdgeType::Unconditional) {
                continue;
            }

            let target_id = edge.target;

            // Redirect all predecessors to target
            let predecessors = block.predecessors.clone();

            for &pred_edge_id in &predecessors {
                if let Some(pred_edge) = cfg.edges.get_mut(pred_edge_id) {
                    pred_edge.target = target_id;
                }

                // Update target's predecessor list
                if let Some(target_block) = cfg.blocks.get_mut(&target_id) {
                    target_block.predecessors.retain(|&e| e != edge_id);
                    if !target_block.predecessors.contains(&pred_edge_id) {
                        target_block.predecessors.push(pred_edge_id);
                    }
                }
            }

            // Remove the block
            cfg.blocks.remove(&block_id);

            removed += 1;
            changed = true;
            break;
        }
    }

    // Clean up
    if removed > 0 {
        cfg.remove_unreachable_blocks();
    }

    removed
}

/// Eliminate branches to blocks that are always taken or never taken.
///
/// If a conditional branch has a constant condition, convert to unconditional.
pub fn eliminate_constant_branches(cfg: &mut ControlFlowGraph) -> usize {
    let mut eliminated = 0;

    // For now, we can detect simple patterns like:
    // - Both branches going to same target
    // - Branch condition is constant (requires constant folding first)

    let block_ids: Vec<_> = cfg.blocks.keys().copied().collect();

    for &block_id in &block_ids {
        let block = match cfg.blocks.get(&block_id) {
            Some(b) => b,
            None => continue,
        };

        // Check if block ends with IF and has exactly 2 successors
        if block.successors.len() != 2 {
            continue;
        }

        let edge1_id = block.successors[0];
        let edge2_id = block.successors[1];

        let edge1 = &cfg.edges[edge1_id];
        let edge2 = &cfg.edges[edge2_id];

        // Both must be conditional
        if !matches!(edge1.edge_type, EdgeType::ConditionalTrue | EdgeType::ConditionalFalse) {
            continue;
        }
        if !matches!(edge2.edge_type, EdgeType::ConditionalTrue | EdgeType::ConditionalFalse) {
            continue;
        }

        // If both branches go to same target, we can eliminate the condition
        if edge1.target == edge2.target {
            let target = edge1.target;

            // Remove both conditional edges
            if let Some(block_mut) = cfg.blocks.get_mut(&block_id) {
                // Remove the IF instruction (last instruction if it's there)
                if let Some(last_op) = block_mut.instructions.last() {
                    if last_op.ident == IROpCode::OpIf as i32 {
                        block_mut.instructions.pop();
                    }
                }

                // Clear successors
                block_mut.successors.clear();
            }

            // Add single unconditional edge
            let new_edge_id = cfg.add_edge(block_id, target, EdgeType::Unconditional);

            // Update target's predecessors
            if let Some(target_block) = cfg.blocks.get_mut(&target) {
                target_block.predecessors.retain(|&e| e != edge1_id && e != edge2_id);
                target_block.predecessors.push(new_edge_id);
            }

            eliminated += 1;
        }
    }

    eliminated
}

/// Apply all CFG optimizations in order.
///
/// Returns tuple: (dead_blocks, merged, empty_removed, branches_eliminated)
pub fn optimize_cfg(cfg: &mut ControlFlowGraph) -> (usize, usize, usize, usize) {
    let mut total_dead = 0;
    let mut total_merged = 0;
    let mut total_empty = 0;
    let mut total_branches = 0;

    // Iterate until no more changes
    loop {
        let dead = eliminate_dead_code(cfg);
        let branches = eliminate_constant_branches(cfg);
        let empty = remove_empty_blocks(cfg);
        let merged = merge_blocks(cfg);

        total_dead += dead;
        total_merged += merged;
        total_empty += empty;
        total_branches += branches;

        // Stop if no changes
        if dead == 0 && branches == 0 && empty == 0 && merged == 0 {
            break;
        }
    }

    (total_dead, total_merged, total_empty, total_branches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_sequential_blocks() {
        let mut cfg = ControlFlowGraph::new("test");
        let a = cfg.create_block("a");
        let b = cfg.create_block("b");
        let c = cfg.create_block("c");

        cfg.set_entry(a);
        cfg.set_exit(c);

        // a -> b -> c (all fallthrough)
        cfg.add_edge(a, b, EdgeType::Fallthrough);
        cfg.add_edge(b, c, EdgeType::Fallthrough);

        assert_eq!(cfg.blocks.len(), 3);

        let merged = merge_blocks(&mut cfg);

        // Should merge a and b
        assert!(merged > 0);
    }

    #[test]
    fn test_remove_empty_blocks() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let empty = cfg.create_block("empty");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        cfg.add_edge(entry, empty, EdgeType::Fallthrough);
        cfg.add_edge(empty, exit, EdgeType::Fallthrough);

        assert_eq!(cfg.blocks.len(), 3);

        let removed = remove_empty_blocks(&mut cfg);

        assert!(removed > 0);
    }

    #[test]
    fn test_eliminate_constant_branches() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let target = cfg.create_block("target");

        cfg.set_entry(entry);
        cfg.set_exit(target);

        // Both branches go to same target
        cfg.add_edge(entry, target, EdgeType::ConditionalTrue);
        cfg.add_edge(entry, target, EdgeType::ConditionalFalse);

        let eliminated = eliminate_constant_branches(&mut cfg);

        assert_eq!(eliminated, 1);
    }
}
