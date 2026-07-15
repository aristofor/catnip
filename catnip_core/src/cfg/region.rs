// FILE: catnip_core/src/cfg/region.rs
//! Region detection for CFG reconstruction.
//!
//! Identifies control flow regions (if/elif/else, while, for, match) in a CFG
//! and reconstructs structured code.

use super::edge::EdgeType;
use super::graph::ControlFlowGraph;
use crate::ir::{IR, IROpCode};
use std::collections::HashSet;

/// Opcode of an IR node, if it carries one.
fn node_opcode(ir: &IR) -> Option<IROpCode> {
    match ir {
        IR::Op { opcode, .. } => Some(*opcode),
        _ => None,
    }
}

/// Region-based CFG reconstructor.
pub struct RegionReconstructor<'a> {
    cfg: &'a ControlFlowGraph,
    visited: HashSet<usize>,
}

impl<'a> RegionReconstructor<'a> {
    /// Filter out Nop instructions (dead code markers from DSE/CSE/LICM/IV).
    fn filter_nops(instructions: &[IR]) -> Vec<IR> {
        instructions
            .iter()
            .filter(|op| node_opcode(op) != Some(IROpCode::Nop))
            .cloned()
            .collect()
    }

    pub fn new(cfg: &'a ControlFlowGraph) -> Self {
        Self {
            cfg,
            visited: HashSet::new(),
        }
    }

    /// Reconstruct IR nodes from CFG.
    pub fn reconstruct(&mut self) -> Vec<IR> {
        let Some(entry_id) = self.cfg.entry else {
            return Vec::new();
        };

        let Some(exit_id) = self.cfg.exit else {
            return Vec::new();
        };

        self.reconstruct_from_block(entry_id, exit_id)
    }

    /// Reconstruct code from a block until we reach the exit.
    fn reconstruct_from_block(&mut self, start: usize, end: usize) -> Vec<IR> {
        let mut result = Vec::new();
        let mut current = start;

        while current != end && !self.visited.contains(&current) {
            self.visited.insert(current);

            let Some(block) = self.cfg.blocks.get(&current) else {
                break;
            };

            // A match header carries a preserved OpMatch (op-preservation
            // strategy): emit it as-is and resume at the merge the builder
            // recorded. The per-arm blocks are encoded inside the OpMatch, so
            // they are not walked from the CFG. Checked before the
            // successor-count dispatch because a two-arm match has two successors
            // and would otherwise be taken for an if/else.
            if block
                .instructions
                .iter()
                .any(|op| node_opcode(op) == Some(IROpCode::OpMatch))
            {
                result.extend(Self::filter_nops(&block.instructions));
                debug_assert!(
                    block.match_merge.is_some(),
                    "CFG: match header without a recorded merge block in '{}'",
                    self.cfg.name
                );
                match block.match_merge {
                    Some(merge) => {
                        current = merge;
                        continue;
                    }
                    None => break,
                }
            }

            // Analyze block successors to determine structure
            match block.successors.len() {
                0 => {
                    // No successors - end of region
                    result.extend(Self::filter_nops(&block.instructions));
                    break;
                }

                1 => {
                    // Single successor
                    let edge_id = block.successors[0];
                    let edge = &self.cfg.edges[edge_id];
                    let target = edge.target;
                    let edge_type = edge.edge_type;

                    // break/continue/return are preserved as instructions
                    // (OpBreak/OpContinue/OpReturn) in this block; their CFG edge
                    // jumps out of the current region (loop exit, function exit).
                    // Emit the block and stop — following the edge would pull
                    // post-region code (the loop's successor, code after a
                    // function) into the body being reconstructed.
                    if matches!(edge_type, EdgeType::Break | EdgeType::Continue | EdgeType::Return) {
                        result.extend(Self::filter_nops(&block.instructions));
                        break;
                    }

                    // Check if this is a loop back-edge
                    if self.is_back_edge(current, target) {
                        // We've reached the back edge that closes the loop body
                        // currently being reconstructed. The loop *structure* is
                        // owned by the enclosing `reconstruct_while` (entered via
                        // the two-successor `is_while_header` path); here we just
                        // emit this tail block's statements and stop. Re-entering
                        // `reconstruct_while` on `current` would be wrong: a body
                        // tail carries no branch condition.
                        result.extend(Self::filter_nops(&block.instructions));
                        break;
                    } else {
                        // Linear flow - add instructions and continue
                        result.extend(Self::filter_nops(&block.instructions));

                        // Reached the region end: stop without emitting the end
                        // block's own instructions. They belong to whoever owns the
                        // region ending here (the top-level loop continues from a
                        // merge point after an if, the caller handles the exit).
                        // Emitting them here would duplicate post-region code into
                        // every branch.
                        if target == end {
                            break;
                        }

                        current = target;
                    }
                }

                2 => {
                    // Two-way branch - could be if/else or while loop header
                    // Check if this is a while loop header
                    if self.is_while_header(current) {
                        // Loop header (while or for) - reconstruct the loop
                        if let Some(loop_op) = self.reconstruct_loop(current) {
                            result.push(loop_op);
                        }
                        // Continue from loop exit
                        if let Some(exit) = self.find_loop_exit(current) {
                            current = exit;
                        } else {
                            break;
                        }
                    } else {
                        // Regular if/else structure
                        // Use preserved condition from CFG construction
                        let condition = block.condition.clone();

                        // Add block instructions (filter Nops)
                        result.extend(
                            block
                                .instructions
                                .iter()
                                .filter(|op| node_opcode(op) != Some(IROpCode::Nop))
                                .cloned(),
                        );

                        // Reconstruct if statement
                        if let Some((if_op, next_block)) = self.reconstruct_if_with_condition(current, end, condition) {
                            result.push(if_op);
                            current = next_block;
                        } else {
                            break;
                        }
                    }
                }

                _ => {
                    // Multiple branches (3+) - likely match statement
                    // For now, just add instructions and break
                    result.extend(Self::filter_nops(&block.instructions));
                    break;
                }
            }
        }

        result
    }

    /// Reconstruct an if/elif/else structure with extracted condition.
    fn reconstruct_if_with_condition(
        &mut self,
        block_id: usize,
        end: usize,
        condition: Option<IR>,
    ) -> Option<(IR, usize)> {
        let block = self.cfg.blocks.get(&block_id).unwrap();

        if block.successors.len() != 2 {
            return None;
        }

        let edge1_id = block.successors[0];
        let edge2_id = block.successors[1];

        let edge1 = &self.cfg.edges[edge1_id];
        let edge2 = &self.cfg.edges[edge2_id];

        // Identify true and false branches
        let (true_target, false_target) = match (&edge1.edge_type, &edge2.edge_type) {
            (EdgeType::ConditionalTrue, EdgeType::ConditionalFalse) => (edge1.target, edge2.target),
            (EdgeType::ConditionalFalse, EdgeType::ConditionalTrue) => (edge2.target, edge1.target),
            _ => return None,
        };

        // Find merge point
        let merge_point = self.find_merge_point(block_id, true_target, false_target, end);

        // Reconstruct true branch
        let true_ops = self.reconstruct_from_block(true_target, merge_point);

        // Reconstruct false branch (might be another if for elif)
        let false_ops = self.reconstruct_from_block(false_target, merge_point);

        // Check if false branch is a single if (for elif detection)
        let branches = if let Some(cond) = condition {
            vec![(cond, true_ops)]
        } else {
            // No condition extracted - create placeholder
            vec![(self.create_placeholder_condition(), true_ops)]
        };

        // Build if Op
        let if_op = self.build_if_op_with_branches(branches, false_ops);

        Some((if_op, merge_point))
    }

    /// Reconstruct a loop (`while` or `for`) rooted at `header`.
    ///
    /// The builder stores the original loop op (`OpWhile`/`OpFor`) as an
    /// instruction in the header block; we read it back to recover the loop kind
    /// and its non-body operands — the `while` condition, or the `for` target and
    /// iterable. The body is rebuilt from the CFG (it may have been optimized), so
    /// only the loop scaffolding comes from the stored op.
    fn reconstruct_loop(&mut self, header: usize) -> Option<IR> {
        // Recover the loop op the builder stored in the header. Cloned so the
        // borrow is released before the &mut self body reconstruction below.
        let loop_op = self
            .cfg
            .blocks
            .get(&header)?
            .instructions
            .iter()
            .find(|op| matches!(node_opcode(op), Some(IROpCode::OpWhile | IROpCode::OpFor)))
            .cloned();

        // Reconstruct the loop body (target of the ConditionalTrue edge).
        let body_ops = match self.find_loop_body(header) {
            Some(body) => self.reconstruct_from_block(body, header),
            None => Vec::new(),
        };

        match loop_op {
            // OpFor args: [target, iterable, body]
            Some(IR::Op {
                opcode: IROpCode::OpFor,
                args,
                ..
            }) => {
                let target = args.first().cloned()?;
                let iterable = args.get(1).cloned()?;
                Some(self.build_for_op(target, iterable, body_ops))
            }
            // OpWhile args: [condition, body]
            Some(IR::Op {
                opcode: IROpCode::OpWhile,
                args,
                ..
            }) => {
                let condition = args
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| self.create_placeholder_condition());
                Some(self.build_while_op(condition, body_ops))
            }
            // No stored loop op: fall back to the preserved branch condition (while).
            _ => {
                let condition = self
                    .cfg
                    .blocks
                    .get(&header)
                    .and_then(|b| b.condition.clone())
                    .unwrap_or_else(|| self.create_placeholder_condition());
                Some(self.build_while_op(condition, body_ops))
            }
        }
    }

    /// Find the loop body block (target of ConditionalTrue from header).
    fn find_loop_body(&self, header: usize) -> Option<usize> {
        let block = self.cfg.blocks.get(&header).unwrap();

        for &edge_id in &block.successors {
            let edge = &self.cfg.edges[edge_id];
            if matches!(edge.edge_type, EdgeType::ConditionalTrue) {
                return Some(edge.target);
            }
        }

        None
    }

    /// Find the loop exit block (target of ConditionalFalse from header).
    fn find_loop_exit(&self, header: usize) -> Option<usize> {
        let block = self.cfg.blocks.get(&header).unwrap();

        for &edge_id in &block.successors {
            let edge = &self.cfg.edges[edge_id];
            if matches!(edge.edge_type, EdgeType::ConditionalFalse) {
                return Some(edge.target);
            }
        }

        None
    }

    /// Check if edge from source to target is a back-edge.
    fn is_back_edge(&self, source: usize, target: usize) -> bool {
        if let Some(source_block) = self.cfg.blocks.get(&source) {
            source_block.dominators.contains(&target)
        } else {
            false
        }
    }

    /// Check if a block is a loop header (`while` or `for`).
    ///
    /// Detected by the preserved loop op the builder stores in the header
    /// (op-preservation), the same signal `reconstruct_loop` reads back. More
    /// robust than a structural back-edge search: a loop whose body always
    /// breaks or returns (`while True { ... break }`) has no back-edge to the
    /// header yet is still a loop.
    fn is_while_header(&self, block_id: usize) -> bool {
        let Some(block) = self.cfg.blocks.get(&block_id) else {
            return false;
        };

        block.successors.len() == 2
            && block
                .instructions
                .iter()
                .any(|op| matches!(node_opcode(op), Some(IROpCode::OpWhile | IROpCode::OpFor)))
    }

    /// Find the merge point where the two branches of an if rooted at `header`
    /// reconverge: the first common successor that is dominated by `header`.
    ///
    /// Dominance by the if header is what makes this correct inside loops: a
    /// plain reachability intersection would return the loop header or exit (both
    /// reachable from a branch via a back-edge or a `break`), collapsing the if.
    /// The merge of a structured if always post-dominates both branches and is
    /// dominated by the header, so requiring header-dominance picks it and skips
    /// the branch entries themselves.
    fn find_merge_point(&self, header: usize, branch1: usize, branch2: usize, default_end: usize) -> usize {
        let successors1 = self.get_all_successors(branch1);
        let successors2 = self.get_all_successors(branch2);

        for succ in &successors1 {
            if *succ == branch1 || *succ == branch2 || !successors2.contains(succ) {
                continue;
            }
            if let Some(block) = self.cfg.blocks.get(succ) {
                if block.dominators.contains(&header) {
                    return *succ;
                }
            }
        }

        default_end
    }

    /// Get all successors reachable from a block by forward edges only.
    ///
    /// Back-edges are not followed: a walk that follows them re-enters every
    /// enclosing loop and reaches the *sibling branch's own blocks* through
    /// the loop header, so `find_merge_point` could pick a block inside one
    /// branch as the "common successor". Seen concretely: the header of a
    /// `while` nested in a `then` branch, reached from the `else` via the
    /// outer loop's back-edge, was taken for the merge — the inner loop was
    /// reconstructed outside its branch and ran on the wrong paths (found by
    /// the Phase 4 property harness). Forward-only, the first common
    /// dominated successor is the real merge: any later common block is only
    /// reachable through it.
    fn get_all_successors(&self, start: usize) -> Vec<usize> {
        let mut successors = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = vec![start];

        while let Some(block_id) = queue.pop() {
            if !visited.insert(block_id) {
                continue;
            }

            successors.push(block_id);

            if let Some(block) = self.cfg.blocks.get(&block_id) {
                for &edge_id in &block.successors {
                    if let Some(edge) = self.cfg.edges.get(edge_id) {
                        if self.is_back_edge(block_id, edge.target) {
                            continue;
                        }
                        queue.push(edge.target);
                    }
                }
            }
        }

        successors
    }

    /// Create a placeholder condition (True).
    ///
    /// Defensive fallback for when CFG reconstruction can't recover the original
    /// branch condition. The F29 fix (preserve `BasicBlock.condition` during build)
    /// removed every known trigger; full pytest suite at `optimize_level=3` reports
    /// zero firings. A trigger would silently rewrite `if cond { ... } else { ... }`
    /// into `if True { ... }`, dropping side-effects and the else branch, so it is
    /// treated as an internal invariant violation in debug builds.
    fn create_placeholder_condition(&self) -> IR {
        debug_assert!(
            false,
            "CFG: branch condition missing during reconstruction in '{}'; \
             builder_ir.rs is expected to call set_condition() for every if/while header",
            self.cfg.name
        );
        IR::op(IROpCode::Nop, vec![IR::Bool(true)])
    }

    /// Build an if Op from branches and else block.
    fn build_if_op_with_branches(&self, branches: Vec<(IR, Vec<IR>)>, else_ops: Vec<IR>) -> IR {
        // Build branches container: Tuple([Tuple([condition, block]), ...])
        let branch_tuples: Vec<IR> = branches
            .into_iter()
            .map(|(condition, body_ops)| {
                let body_block = self.build_block_op(body_ops);
                IR::Tuple(vec![condition, body_block])
            })
            .collect();
        let branches_container = IR::Tuple(branch_tuples);

        // Build args
        let args = if !else_ops.is_empty() {
            let else_block = self.build_block_op(else_ops);
            vec![branches_container, else_block]
        } else {
            vec![branches_container]
        };

        IR::op(IROpCode::OpIf, args)
    }

    /// Build a while Op.
    fn build_while_op(&self, condition: IR, body_ops: Vec<IR>) -> IR {
        let body_block = self.build_block_op(body_ops);
        IR::op(IROpCode::OpWhile, vec![condition, body_block])
    }

    /// Build a for Op.
    fn build_for_op(&self, target: IR, iterable: IR, body_ops: Vec<IR>) -> IR {
        let body_block = self.build_block_op(body_ops);
        IR::op(IROpCode::OpFor, vec![target, iterable, body_block])
    }

    /// Build a block Op from a list of IR nodes.
    fn build_block_op(&self, ops: Vec<IR>) -> IR {
        IR::op(IROpCode::OpBlock, ops)
    }
}

/// Reconstruct IR nodes from CFG using region detection.
pub fn reconstruct_from_cfg(cfg: &ControlFlowGraph) -> Vec<IR> {
    let mut reconstructor = RegionReconstructor::new(cfg);
    reconstructor.reconstruct()
}
