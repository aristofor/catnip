// FILE: catnip_rs/src/cfg/analysis.rs
//! CFG analysis utilities.

use super::graph::ControlFlowGraph;
use std::collections::{HashMap, HashSet, VecDeque};

/// Compute dominance tree for CFG.
///
/// A block X dominates block Y if every path from entry to Y passes through X.
pub fn compute_dominators(cfg: &mut ControlFlowGraph) {
    let Some(entry_id) = cfg.entry else {
        return;
    };

    let block_ids: Vec<_> = cfg.blocks.keys().copied().collect();

    // Initialize: entry dominates itself, all others dominated by all blocks
    for &id in &block_ids {
        if let Some(block) = cfg.get_block_mut(id) {
            if id == entry_id {
                block.dominators.clear();
                block.dominators.insert(id);
            } else {
                block.dominators = block_ids.iter().copied().collect();
            }
        }
    }

    // Iterative dataflow until fixpoint
    let mut changed = true;
    while changed {
        changed = false;

        for &id in &block_ids {
            if id == entry_id {
                continue;
            }

            let mut new_doms = block_ids.iter().copied().collect::<HashSet<_>>();

            // Get predecessors' dominators
            if let Some(block) = cfg.blocks.get(&id) {
                for &pred_edge_id in &block.predecessors {
                    if let Some(edge) = cfg.edges.get(pred_edge_id) {
                        if let Some(pred_block) = cfg.blocks.get(&edge.source) {
                            new_doms = new_doms
                                .intersection(&pred_block.dominators)
                                .copied()
                                .collect();
                        }
                    }
                }
            }

            // Add self
            new_doms.insert(id);

            // Check if changed
            if let Some(block) = cfg.get_block_mut(id) {
                if new_doms != block.dominators {
                    block.dominators = new_doms;
                    changed = true;
                }
            }
        }
    }

    // Compute immediate dominators
    compute_immediate_dominators(cfg);
}

/// Compute immediate dominator for each block.
///
/// The immediate dominator idom(X) is the unique block that:
/// 1. Dominates X
/// 2. Is dominated by all other dominators of X
fn compute_immediate_dominators(cfg: &mut ControlFlowGraph) {
    let Some(entry_id) = cfg.entry else {
        return;
    };

    let block_ids: Vec<_> = cfg.blocks.keys().copied().collect();

    for &id in &block_ids {
        if id == entry_id {
            continue; // Entry has no idom
        }

        let dominators = if let Some(block) = cfg.blocks.get(&id) {
            block.dominators.clone()
        } else {
            continue;
        };

        // Find idom: dominator closest to id (excluding id itself)
        let mut candidates: Vec<_> = dominators.iter().filter(|&&d| d != id).copied().collect();

        // Sort candidates by dominator set size (smaller = closer to id)
        candidates.sort_by_key(|&cand| {
            cfg.blocks
                .get(&cand)
                .map(|b| b.dominators.len())
                .unwrap_or(0)
        });

        // The candidate with the largest dominator set that still dominates id is the idom
        // Actually, we want the one that is dominated by all others
        if let Some(&idom) = candidates.iter().rev().find(|&&cand| {
            candidates.iter().all(|&other| {
                other == cand
                    || cfg
                        .blocks
                        .get(&other)
                        .map(|b| b.dominators.contains(&cand))
                        .unwrap_or(false)
            })
        }) {
            if let Some(block) = cfg.get_block_mut(id) {
                block.immediate_dominator = Some(idom);
            }

            // Add to dominated set of idom
            if let Some(idom_block) = cfg.get_block_mut(idom) {
                idom_block.dominated.insert(id);
            }
        }
    }
}

/// Detect natural loops in the CFG.
///
/// A natural loop has:
/// - A header block (dominates all blocks in the loop)
/// - A back edge from a block in the loop to the header
///
/// Returns: Vec of (header_id, loop_blocks)
pub fn detect_loops(cfg: &ControlFlowGraph) -> Vec<(usize, HashSet<usize>)> {
    let mut loops = Vec::new();

    // Find back edges (target dominates source)
    for edge in &cfg.edges {
        if let Some(source_block) = cfg.blocks.get(&edge.source) {
            if source_block.dominators.contains(&edge.target) {
                // Back edge found: edge.target is loop header
                let header = edge.target;
                let mut loop_blocks = HashSet::new();

                // BFS from source back to header (blocks in loop)
                let mut queue = VecDeque::new();
                queue.push_back(edge.source);
                loop_blocks.insert(edge.source);
                loop_blocks.insert(header);

                while let Some(block_id) = queue.pop_front() {
                    if let Some(block) = cfg.blocks.get(&block_id) {
                        for &pred_edge_id in &block.predecessors {
                            if let Some(pred_edge) = cfg.edges.get(pred_edge_id) {
                                if pred_edge.source != header
                                    && loop_blocks.insert(pred_edge.source)
                                {
                                    queue.push_back(pred_edge.source);
                                }
                            }
                        }
                    }
                }

                loops.push((header, loop_blocks));
            }
        }
    }

    loops
}

/// Compute dominance frontiers for all blocks.
///
/// DF(X) = { Y | Y has a predecessor dominated by X, but Y is not strictly dominated by X }
///
/// Uses the algorithm from Cytron et al. (1991).
pub fn compute_dominance_frontiers(cfg: &ControlFlowGraph) -> HashMap<usize, HashSet<usize>> {
    let mut frontiers: HashMap<usize, HashSet<usize>> = HashMap::new();

    for &block_id in cfg.blocks.keys() {
        frontiers.insert(block_id, HashSet::new());
    }

    for (&block_id, block) in &cfg.blocks {
        // Only consider join points (blocks with 2+ predecessors)
        if block.predecessors.len() < 2 {
            continue;
        }

        for &pred_edge_id in &block.predecessors {
            let Some(edge) = cfg.edges.get(pred_edge_id) else {
                continue;
            };
            let mut runner = edge.source;

            // Walk up the dominator tree from predecessor to idom(block_id)
            while runner != block.immediate_dominator.unwrap_or(runner)
                || (block.immediate_dominator.is_none() && runner != block_id)
            {
                // runner is in DF of block_id if runner != strict dominator of block_id
                if let Some(runner_block) = cfg.blocks.get(&runner) {
                    if !runner_block.dominated.contains(&block_id) || runner == block_id {
                        frontiers.entry(runner).or_default().insert(block_id);
                    }
                }

                // Move up to immediate dominator
                let idom = cfg.blocks.get(&runner).and_then(|b| b.immediate_dominator);
                match idom {
                    Some(next) if next != runner => runner = next,
                    _ => break,
                }
            }
        }
    }

    frontiers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::edge::EdgeType;

    #[test]
    fn test_dominators_simple() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let bb1 = cfg.create_block("bb1");
        let bb2 = cfg.create_block("bb2");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        // Linear: entry -> bb1 -> bb2 -> exit
        cfg.add_edge(entry, bb1, EdgeType::Fallthrough);
        cfg.add_edge(bb1, bb2, EdgeType::Fallthrough);
        cfg.add_edge(bb2, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);

        // entry dominates all
        assert!(cfg.blocks[&bb1].dominators.contains(&entry));
        assert!(cfg.blocks[&bb2].dominators.contains(&entry));
        assert!(cfg.blocks[&exit].dominators.contains(&entry));

        // bb1 dominates bb2 and exit
        assert!(cfg.blocks[&bb2].dominators.contains(&bb1));
        assert!(cfg.blocks[&exit].dominators.contains(&bb1));
    }

    #[test]
    fn test_loop_detection() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let header = cfg.create_block("header");
        let body = cfg.create_block("body");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        // Loop: entry -> header <-> body -> exit
        cfg.add_edge(entry, header, EdgeType::Fallthrough);
        cfg.add_edge(header, body, EdgeType::ConditionalTrue);
        cfg.add_edge(header, exit, EdgeType::ConditionalFalse);
        cfg.add_edge(body, header, EdgeType::Unconditional); // Back edge

        compute_dominators(&mut cfg);
        let loops = detect_loops(&cfg);

        assert_eq!(loops.len(), 1);
        let (loop_header, loop_blocks) = &loops[0];
        assert_eq!(*loop_header, header);
        assert!(loop_blocks.contains(&body));
    }

    #[test]
    fn test_dominance_frontiers_diamond() {
        //   entry -> (true, false) -> merge -> exit
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let true_bb = cfg.create_block("true");
        let false_bb = cfg.create_block("false");
        let merge = cfg.create_block("merge");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        cfg.add_edge(entry, true_bb, EdgeType::ConditionalTrue);
        cfg.add_edge(entry, false_bb, EdgeType::ConditionalFalse);
        cfg.add_edge(true_bb, merge, EdgeType::Fallthrough);
        cfg.add_edge(false_bb, merge, EdgeType::Fallthrough);
        cfg.add_edge(merge, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let df = compute_dominance_frontiers(&cfg);

        // DF(true) = {merge}, DF(false) = {merge}
        assert!(df[&true_bb].contains(&merge));
        assert!(df[&false_bb].contains(&merge));
        // DF(entry) = {} (entry dominates everything)
        assert!(df[&entry].is_empty());
    }

    #[test]
    fn test_dominance_frontiers_linear() {
        // Linear: no join points → all frontiers empty
        let mut cfg = ControlFlowGraph::new("test");
        let a = cfg.create_block("a");
        let b = cfg.create_block("b");
        let c = cfg.create_block("c");

        cfg.set_entry(a);
        cfg.set_exit(c);

        cfg.add_edge(a, b, EdgeType::Fallthrough);
        cfg.add_edge(b, c, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let df = compute_dominance_frontiers(&cfg);

        for (_, frontier) in &df {
            assert!(frontier.is_empty());
        }
    }
}
