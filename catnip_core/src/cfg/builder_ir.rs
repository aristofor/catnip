// FILE: catnip_core/src/cfg/builder_ir.rs
//! Build CFG from IR (OpCode level).

use super::edge::EdgeType;
use super::graph::ControlFlowGraph;
use crate::ir::{IR, IROpCode};

/// Opcode of an IR node, if it carries one.
fn node_opcode(ir: &IR) -> Option<IROpCode> {
    match ir {
        IR::Op { opcode, .. } => Some(*opcode),
        _ => None,
    }
}

/// Build CFG from IR OpCode nodes.
pub struct IRCFGBuilder {
    cfg: ControlFlowGraph,
    current_block: Option<usize>,
    break_targets: Vec<usize>,
    continue_targets: Vec<usize>,
}

impl IRCFGBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            cfg: ControlFlowGraph::new(name),
            current_block: None,
            break_targets: Vec::new(),
            continue_targets: Vec::new(),
        }
    }

    pub fn build(mut self, ops: Vec<IR>) -> ControlFlowGraph {
        // Create entry and exit blocks
        let entry = self.cfg.create_block("entry");
        let exit = self.cfg.create_block("exit");
        self.cfg.set_entry(entry);
        self.cfg.set_exit(exit);
        self.current_block = Some(entry);

        // Process all ops
        for op in ops {
            if self.current_block.is_none() {
                break;
            }
            self.process_node(op);
        }

        // Connect current block to exit if no successors
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.blocks.get(&current) {
                if block.successors.is_empty() {
                    self.cfg.add_edge(current, exit, EdgeType::Fallthrough);
                }
            }
        }

        debug_assert!(
            self.cfg.verify().is_ok(),
            "IRCFGBuilder produced a malformed CFG: {:?}",
            self.cfg.verify()
        );
        self.cfg
    }

    fn process_node(&mut self, op: IR) {
        match node_opcode(&op) {
            Some(IROpCode::OpIf) => self.process_if(op),
            Some(IROpCode::OpWhile) => self.process_while(op),
            Some(IROpCode::OpFor) => self.process_for(op),
            Some(IROpCode::OpReturn) => self.process_return(op),
            Some(IROpCode::OpBreak) => self.process_break(op),
            Some(IROpCode::OpContinue) => self.process_continue(op),
            Some(IROpCode::OpMatch) => self.process_match(op),
            Some(IROpCode::OpBlock) => self.process_block(op),
            _ => {
                // Regular instruction
                if let Some(current) = self.current_block {
                    if let Some(block) = self.cfg.get_block_mut(current) {
                        block.add_instruction(op);
                    }
                }
            }
        }
    }

    fn process_if(&mut self, op: IR) {
        // IF structure: args = [Tuple([Tuple([condition, then_block]), ...]), else_or_none]
        let then_block = self.cfg.create_block("if_then");
        let else_block = self.cfg.create_block("if_else");
        let merge_block = self.cfg.create_block("if_merge");

        // Extract condition, then-branch and else-branch as owned clones up front.
        let (cond, then_node, else_node) = match &op {
            IR::Op { args, .. } => {
                let branches: Vec<IR> = match args.first() {
                    Some(IR::Tuple(pairs)) => pairs.clone(),
                    _ => Vec::new(),
                };
                let (cond, then_node) = match branches.first() {
                    Some(IR::Tuple(p)) => (p.first().cloned(), p.get(1).cloned()),
                    _ => (None, None),
                };
                let original_else = match args.get(1) {
                    Some(e) if !matches!(e, IR::None) => Some(e.clone()),
                    _ => None,
                };
                // Multiple branch pairs are an elif chain. Peel the first pair
                // here and fold the rest into a nested OpIf used as this if's
                // else, so each elif is built by a recursive `process_if`. The
                // pipeline reconstructs it as nested if/else -- equivalent to
                // the chain. Reading only `branches.first()` dropped every elif.
                let else_node = if branches.len() > 1 {
                    let rest = IR::Tuple(branches[1..].to_vec());
                    let inner_else = original_else.unwrap_or(IR::None);
                    Some(IR::op(IROpCode::OpIf, vec![rest, inner_else]))
                } else {
                    original_else
                };
                (cond, then_node, else_node)
            }
            _ => (None, None, None),
        };

        // Store condition in current block, then add edges
        if let Some(current) = self.current_block {
            if let Some(cond) = cond {
                if let Some(block) = self.cfg.get_block_mut(current) {
                    block.set_condition(cond);
                }
            }
            self.cfg.add_edge(current, then_block, EdgeType::ConditionalTrue);
            self.cfg.add_edge(current, else_block, EdgeType::ConditionalFalse);
        }

        // Process then branch
        self.current_block = Some(then_block);
        if let Some(then_node) = then_node {
            self.process_body(then_node);
        }

        // Connect then to merge if no successors
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.blocks.get(&current) {
                if block.successors.is_empty() {
                    self.cfg.add_edge(current, merge_block, EdgeType::Fallthrough);
                }
            }
        }

        // Process else branch (if present)
        self.current_block = Some(else_block);
        if let Some(else_node) = else_node {
            self.process_body(else_node);
        }

        // Connect else to merge if no successors
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.blocks.get(&current) {
                if block.successors.is_empty() {
                    self.cfg.add_edge(current, merge_block, EdgeType::Fallthrough);
                }
            }
        }

        self.current_block = Some(merge_block);
    }

    fn process_while(&mut self, op: IR) {
        let header = self.cfg.create_block("while_header");
        let body = self.cfg.create_block("while_body");
        let exit_block = self.cfg.create_block("while_exit");

        // Edge to header
        if let Some(current) = self.current_block {
            self.cfg.add_edge(current, header, EdgeType::Fallthrough);
        }

        // args[0] = condition, args[1] = body
        let (cond, body_node) = match &op {
            IR::Op { args, .. } => (args.first().cloned(), args.get(1).cloned()),
            _ => (None, None),
        };

        if let Some(cond) = cond {
            if let Some(block) = self.cfg.get_block_mut(header) {
                block.set_condition(cond);
            }
        }

        // Add WHILE to header (for backward compat with other passes)
        if let Some(block) = self.cfg.get_block_mut(header) {
            block.add_instruction(op.clone());
        }

        // Edges from header
        self.cfg.add_edge(header, body, EdgeType::ConditionalTrue);
        self.cfg.add_edge(header, exit_block, EdgeType::ConditionalFalse);

        // Process body
        self.break_targets.push(exit_block);
        self.continue_targets.push(header);
        self.current_block = Some(body);

        if let Some(body_node) = body_node {
            self.process_body(body_node);
        }

        // Back edge to header
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.blocks.get(&current) {
                if block.successors.is_empty() {
                    self.cfg.add_edge(current, header, EdgeType::Unconditional);
                }
            }
        }

        self.break_targets.pop();
        self.continue_targets.pop();
        self.current_block = Some(exit_block);
    }

    fn process_for(&mut self, op: IR) {
        let header = self.cfg.create_block("for_header");
        let body = self.cfg.create_block("for_body");
        let exit_block = self.cfg.create_block("for_exit");

        if let Some(current) = self.current_block {
            self.cfg.add_edge(current, header, EdgeType::Fallthrough);
        }

        if let Some(block) = self.cfg.get_block_mut(header) {
            block.add_instruction(op.clone());
        }

        self.cfg.add_edge(header, body, EdgeType::ConditionalTrue);
        self.cfg.add_edge(header, exit_block, EdgeType::ConditionalFalse);

        self.break_targets.push(exit_block);
        self.continue_targets.push(header);
        self.current_block = Some(body);

        // args[2] = body
        let body_node = match &op {
            IR::Op { args, .. } => args.get(2).cloned(),
            _ => None,
        };
        if let Some(body_node) = body_node {
            self.process_body(body_node);
        }

        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.blocks.get(&current) {
                if block.successors.is_empty() {
                    self.cfg.add_edge(current, header, EdgeType::Unconditional);
                }
            }
        }

        self.break_targets.pop();
        self.continue_targets.pop();
        self.current_block = Some(exit_block);
    }

    fn process_return(&mut self, op: IR) {
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.get_block_mut(current) {
                block.add_instruction(op);
            }

            if let Some(exit) = self.cfg.exit {
                self.cfg.add_edge(current, exit, EdgeType::Return);
            }
        }
        self.current_block = None;
    }

    fn process_break(&mut self, op: IR) {
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.get_block_mut(current) {
                block.add_instruction(op);
            }

            if let Some(&target) = self.break_targets.last() {
                self.cfg.add_edge(current, target, EdgeType::Break);
            }
        }
        self.current_block = None;
    }

    fn process_continue(&mut self, op: IR) {
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.get_block_mut(current) {
                block.add_instruction(op);
            }

            if let Some(&target) = self.continue_targets.last() {
                self.cfg.add_edge(current, target, EdgeType::Continue);
            }
        }
        self.current_block = None;
    }

    fn process_match(&mut self, op: IR) {
        // MATCH structure: args = [subject, Tuple([Tuple([pattern, guard, body]), ...])]
        // Create one block per case arm + merge block
        let merge_block = self.cfg.create_block("match_merge");

        // Add match header instruction to current block, and record the merge
        // block explicitly: reconstruction cannot infer it from the graph when an
        // arm ends in break/continue/return (that arm never reaches the merge).
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.get_block_mut(current) {
                block.add_instruction(op.clone());
                block.match_merge = Some(merge_block);
            }
        }

        let header_block = self.current_block;

        // Extract case bodies (args[1] = cases tuple; each case = Tuple([pattern, guard, body]))
        let case_bodies: Vec<Option<IR>> = match &op {
            IR::Op { args, .. } => match args.get(1) {
                Some(IR::Tuple(cases)) => cases
                    .iter()
                    .map(|case| match case {
                        IR::Tuple(c) if c.len() >= 3 => Some(c[2].clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };

        for (i, body) in case_bodies.into_iter().enumerate() {
            let case_block = self.cfg.create_block(format!("match_case_{}", i));

            // Edge from header to case
            if let Some(hdr) = header_block {
                self.cfg.add_edge(hdr, case_block, EdgeType::ConditionalTrue);
            }

            // Process case body
            self.current_block = Some(case_block);
            if let Some(body_op) = body {
                self.process_node(body_op);
            }

            // Connect case to merge
            if let Some(current) = self.current_block {
                if let Some(block) = self.cfg.blocks.get(&current) {
                    if block.successors.is_empty() {
                        self.cfg.add_edge(current, merge_block, EdgeType::Fallthrough);
                    }
                }
            }
        }

        self.current_block = Some(merge_block);
    }

    /// A standalone `{...}` block creates its own scope: its locals do not leak
    /// into the enclosing scope, and it evaluates to its last expression.
    /// Preserve it opaquely -- flattening its statements would leak the bindings
    /// (cf. tests/optimization/test_cse.py) and is only correct for control-flow
    /// bodies, which `process_body` handles. For Phase 2 this round-trips the
    /// block identically; seeing into it for inter-block passes is deferred.
    fn process_block(&mut self, op: IR) {
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.get_block_mut(current) {
                block.add_instruction(op);
            }
        }
    }

    /// Flatten a control-flow body (loop body, `if` branch) into the current
    /// block. Unlike a standalone block, a body's bindings leak into the
    /// enclosing scope, so its children are processed inline -- including a
    /// trailing value expression -- rather than preserved as an OpBlock.
    fn process_body(&mut self, body: IR) {
        if node_opcode(&body) == Some(IROpCode::OpBlock) {
            let children = match body {
                IR::Op { args, .. } => args,
                _ => Vec::new(),
            };
            for child in children {
                if self.current_block.is_none() {
                    break;
                }
                self.process_node(child);
            }
        } else {
            self.process_node(body);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iropcodes_values() {
        // Verify IROpCode values match expected values for CFG construction
        assert_eq!(IROpCode::OpIf as i32, 50);
        assert_eq!(IROpCode::OpWhile as i32, 51);
        assert_eq!(IROpCode::OpFor as i32, 52);
        assert_eq!(IROpCode::OpMatch as i32, 53);
        assert_eq!(IROpCode::OpBlock as i32, 54);
        assert_eq!(IROpCode::OpReturn as i32, 55);
        assert_eq!(IROpCode::OpBreak as i32, 56);
        assert_eq!(IROpCode::OpContinue as i32, 57);
    }
}
