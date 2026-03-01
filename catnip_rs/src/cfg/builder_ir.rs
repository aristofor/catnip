// FILE: catnip_rs/src/cfg/builder_ir.rs
//! Build CFG from IR (OpCode level).

use super::edge::EdgeType;
use super::graph::{ControlFlowGraph, PyControlFlowGraph};
use crate::core::op::Op;
use crate::ir::opcode::IROpCode;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

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

    pub fn build(mut self, ops: Vec<Op>) -> ControlFlowGraph {
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

        self.cfg
    }

    fn process_node(&mut self, op: Op) {
        let opcode = op.ident;

        // Match using IROpCode enum values
        if opcode == IROpCode::OpIf as i32 {
            self.process_if(op);
        } else if opcode == IROpCode::OpWhile as i32 {
            self.process_while(op);
        } else if opcode == IROpCode::OpFor as i32 {
            self.process_for(op);
        } else if opcode == IROpCode::OpReturn as i32 {
            self.process_return(op);
        } else if opcode == IROpCode::OpBreak as i32 {
            self.process_break(op);
        } else if opcode == IROpCode::OpContinue as i32 {
            self.process_continue(op);
        } else if opcode == IROpCode::OpMatch as i32 {
            self.process_match(op);
        } else if opcode == IROpCode::OpBlock as i32 {
            self.process_block(op);
        } else {
            // Regular instruction
            if let Some(current) = self.current_block {
                if let Some(block) = self.cfg.get_block_mut(current) {
                    block.add_instruction(op);
                }
            }
        }
    }

    fn process_if(&mut self, op: Op) {
        // IF structure: args = (((condition, then_block),), else_block_or_none)
        let then_block = self.cfg.create_block("if_then");
        let else_block = self.cfg.create_block("if_else");
        let merge_block = self.cfg.create_block("if_merge");

        // Edge from current to then/else (conditional)
        if let Some(current) = self.current_block {
            self.cfg
                .add_edge(current, then_block, EdgeType::ConditionalTrue);
            self.cfg
                .add_edge(current, else_block, EdgeType::ConditionalFalse);
        }

        // Process then branch: extract from nested tuple args[0][0][1]
        self.current_block = Some(then_block);
        Python::attach(|py| {
            let args = op.get_args();
            let args_bound = args.bind(py);
            if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                if args_tuple.len() >= 1 {
                    // args[0] is ((condition, then_block),)
                    if let Ok(nested) = args_tuple.get_item(0) {
                        if let Ok(nested_tuple) = nested.cast::<PyTuple>() {
                            if nested_tuple.len() >= 1 {
                                // nested[0] is (condition, then_block)
                                if let Ok(pair) = nested_tuple.get_item(0) {
                                    if let Ok(pair_tuple) = pair.cast::<PyTuple>() {
                                        if pair_tuple.len() >= 2 {
                                            // pair[1] is then_block
                                            if let Ok(then_op) = pair_tuple.get_item(1) {
                                                if let Ok(op) = then_op.extract::<Op>() {
                                                    self.process_node(op);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // Connect then to merge if no successors
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.blocks.get(&current) {
                if block.successors.is_empty() {
                    self.cfg
                        .add_edge(current, merge_block, EdgeType::Fallthrough);
                }
            }
        }

        // Process else branch: args[1] is else_block (if present)
        self.current_block = Some(else_block);
        Python::attach(|py| {
            let args = op.get_args();
            let args_bound = args.bind(py);
            if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    // args[1] is else_block (if present)
                    if let Ok(else_item) = args_tuple.get_item(1) {
                        // Check if it's not None
                        if !else_item.is_none() {
                            if let Ok(op) = else_item.extract::<Op>() {
                                self.process_node(op);
                            }
                        }
                    }
                }
            }
        });

        // Connect else to merge if no successors
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.blocks.get(&current) {
                if block.successors.is_empty() {
                    self.cfg
                        .add_edge(current, merge_block, EdgeType::Fallthrough);
                }
            }
        }

        self.current_block = Some(merge_block);
    }

    fn process_while(&mut self, op: Op) {
        let header = self.cfg.create_block("while_header");
        let body = self.cfg.create_block("while_body");
        let exit_block = self.cfg.create_block("while_exit");

        // Edge to header
        if let Some(current) = self.current_block {
            self.cfg.add_edge(current, header, EdgeType::Fallthrough);
        }

        // Add WHILE to header
        if let Some(block) = self.cfg.get_block_mut(header) {
            block.add_instruction(op.clone());
        }

        // Edges from header
        self.cfg.add_edge(header, body, EdgeType::ConditionalTrue);
        self.cfg
            .add_edge(header, exit_block, EdgeType::ConditionalFalse);

        // Process body (simplified - extract from args requires Python GIL)
        self.break_targets.push(exit_block);
        self.continue_targets.push(header);
        self.current_block = Some(body);

        // Extract and process body statements
        Python::attach(|py| {
            let args = op.get_args();
            let args_bound = args.bind(py);
            if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    if let Ok(body_op) = args_tuple.get_item(1) {
                        if let Ok(op) = body_op.extract::<Op>() {
                            self.process_node(op);
                        }
                    }
                }
            }
        });

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

    fn process_for(&mut self, op: Op) {
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
        self.cfg
            .add_edge(header, exit_block, EdgeType::ConditionalFalse);

        self.break_targets.push(exit_block);
        self.continue_targets.push(header);
        self.current_block = Some(body);

        // Extract body
        Python::attach(|py| {
            let args = op.get_args();
            let args_bound = args.bind(py);
            if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                if args_tuple.len() >= 3 {
                    if let Ok(body_op) = args_tuple.get_item(2) {
                        if let Ok(op) = body_op.extract::<Op>() {
                            self.process_node(op);
                        }
                    }
                }
            }
        });

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

    fn process_return(&mut self, op: Op) {
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

    fn process_break(&mut self, op: Op) {
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

    fn process_continue(&mut self, op: Op) {
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

    fn process_match(&mut self, op: Op) {
        // MATCH structure: args = (subject, ((pattern1, guard1, body1), (pattern2, guard2, body2), ...))
        // Create one block per case arm + merge block
        let merge_block = self.cfg.create_block("match_merge");

        // Add match header instruction to current block
        if let Some(current) = self.current_block {
            if let Some(block) = self.cfg.get_block_mut(current) {
                block.add_instruction(op.clone());
            }
        }

        let header_block = self.current_block;

        // Extract case arms from args
        Python::attach(|py| {
            let args = op.get_args();
            let args_bound = args.bind(py);
            if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                if args_tuple.len() >= 2 {
                    // args[1] = cases tuple
                    if let Ok(cases) = args_tuple.get_item(1) {
                        if let Ok(cases_tuple) = cases.cast::<PyTuple>() {
                            for (i, case_item) in cases_tuple.iter().enumerate() {
                                let case_block = self.cfg.create_block(format!("match_case_{}", i));

                                // Edge from header to case
                                if let Some(hdr) = header_block {
                                    self.cfg
                                        .add_edge(hdr, case_block, EdgeType::ConditionalTrue);
                                }

                                // Process case body
                                self.current_block = Some(case_block);
                                if let Ok(case_tuple) = case_item.cast::<PyTuple>() {
                                    // case_tuple = (pattern, guard, body)
                                    if case_tuple.len() >= 3 {
                                        if let Ok(body) = case_tuple.get_item(2) {
                                            if let Ok(body_op) = body.extract::<Op>() {
                                                self.process_node(body_op);
                                            }
                                        }
                                    }
                                }

                                // Connect case to merge
                                if let Some(current) = self.current_block {
                                    if let Some(block) = self.cfg.blocks.get(&current) {
                                        if block.successors.is_empty() {
                                            self.cfg.add_edge(
                                                current,
                                                merge_block,
                                                EdgeType::Fallthrough,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        self.current_block = Some(merge_block);
    }

    fn process_block(&mut self, op: Op) {
        // For blocks, we need to handle two cases:
        // 1. Structured blocks with Op children (after semantic analysis)
        // 2. Blocks with raw values (before semantic analysis)
        //
        // Since CFG building happens before semantic analysis, we typically see
        // raw values. In this case, we don't recursively process - we just add
        // the block Op itself as an instruction.

        Python::attach(|py| {
            let args = op.get_args();
            let args_bound = args.bind(py);

            if let Ok(args_tuple) = args_bound.cast::<PyTuple>() {
                let mut has_op_children = false;

                for item in args_tuple.iter() {
                    if item.extract::<Op>().is_ok() {
                        has_op_children = true;
                        break;
                    }
                }

                if has_op_children {
                    // Structured block with Op children - process recursively
                    for item in args_tuple.iter() {
                        if self.current_block.is_none() {
                            break;
                        }
                        if let Ok(stmt) = item.extract::<Op>() {
                            self.process_node(stmt);
                        }
                    }
                } else {
                    // Raw values (literals) - add the block Op itself as an instruction
                    if let Some(current) = self.current_block {
                        if let Some(block) = self.cfg.get_block_mut(current) {
                            block.add_instruction(op);
                        }
                    }
                }
            }
        });
    }
}

/// Build control flow graph from IR OpCode list.
#[pyfunction]
pub fn build_cfg_from_ir(
    _py: Python<'_>,
    ops: &Bound<'_, PyList>,
    name: &str,
) -> PyResult<PyControlFlowGraph> {
    let mut op_vec = Vec::new();
    for item in ops.iter() {
        if let Ok(op) = item.extract::<Op>() {
            op_vec.push(op);
        }
    }

    let builder = IRCFGBuilder::new(name);
    let cfg = builder.build(op_vec);

    Ok(PyControlFlowGraph { inner: cfg })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iropcodes_values() {
        // Verify IROpCode values match expected values for CFG construction
        assert_eq!(IROpCode::OpIf as i32, 32);
        assert_eq!(IROpCode::OpWhile as i32, 33);
        assert_eq!(IROpCode::OpFor as i32, 34);
        assert_eq!(IROpCode::OpMatch as i32, 35);
        assert_eq!(IROpCode::OpBlock as i32, 36);
        assert_eq!(IROpCode::OpReturn as i32, 37);
        assert_eq!(IROpCode::OpBreak as i32, 38);
        assert_eq!(IROpCode::OpContinue as i32, 39);
    }
}
