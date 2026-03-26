// FILE: catnip_core/src/semantic/passes/block_flattening.rs
//! Block flattening pass (pure Rust).
//!
//! Merges nested blocks: block(block(a, b), c) → block(a, b, c)

use super::{PurePass, map_children};
use crate::ir::{IR, IROpCode};

pub struct BlockFlatteningPass;

impl PurePass for BlockFlatteningPass {
    fn name(&self) -> &str {
        "block_flattening"
    }

    fn optimize(&mut self, ir: IR) -> IR {
        let visited = map_children(ir, &mut |child| self.optimize(child));
        flatten(visited)
    }
}

fn flatten(ir: IR) -> IR {
    match ir {
        IR::Op {
            opcode: opcode @ IROpCode::OpBlock,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        } => {
            let has_nested = args.iter().any(|a| {
                matches!(
                    a,
                    IR::Op {
                        opcode: IROpCode::OpBlock,
                        ..
                    }
                )
            });
            if !has_nested {
                return IR::Op {
                    opcode,
                    args,
                    kwargs,
                    tail,
                    start_byte,
                    end_byte,
                };
            }
            IR::Op {
                opcode,
                args: inline_blocks(args),
                kwargs,
                tail,
                start_byte,
                end_byte,
            }
        }
        // Also flatten blocks inside Programs (e.g. after DCE simplifies if True { block })
        IR::Program(items) => {
            if !items.iter().any(|a| {
                matches!(
                    a,
                    IR::Op {
                        opcode: IROpCode::OpBlock,
                        ..
                    }
                )
            }) {
                return IR::Program(items);
            }
            IR::Program(inline_blocks(items))
        }
        other => other,
    }
}

/// Inline OpBlock children into a flat list of statements
fn inline_blocks(items: Vec<IR>) -> Vec<IR> {
    let mut flattened = Vec::with_capacity(items.len());
    for item in items {
        if let IR::Op {
            opcode: IROpCode::OpBlock,
            args: inner_args,
            ..
        } = item
        {
            flattened.extend(inner_args);
        } else {
            flattened.push(item);
        }
    }
    flattened
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(ir: IR) -> IR {
        BlockFlatteningPass.optimize(ir)
    }

    #[test]
    fn test_flatten_nested_blocks() {
        let inner = IR::op(IROpCode::OpBlock, vec![IR::Int(1), IR::Int(2)]);
        let outer = IR::op(IROpCode::OpBlock, vec![inner, IR::Int(3)]);
        let result = opt(outer);
        assert_eq!(result.args().unwrap(), &[IR::Int(1), IR::Int(2), IR::Int(3)]);
    }

    #[test]
    fn test_no_flatten_non_block() {
        let ir = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        let result = opt(ir.clone());
        assert_eq!(result, ir);
    }

    #[test]
    fn test_no_change_flat_block() {
        let ir = IR::op(IROpCode::OpBlock, vec![IR::Int(1), IR::Int(2)]);
        let result = opt(ir.clone());
        assert_eq!(result, ir);
    }

    #[test]
    fn test_flatten_block_in_program() {
        let block = IR::op(IROpCode::OpBlock, vec![IR::Int(1), IR::Int(2)]);
        let program = IR::Program(vec![block, IR::Int(3)]);
        let result = opt(program);
        if let IR::Program(items) = result {
            assert_eq!(items, vec![IR::Int(1), IR::Int(2), IR::Int(3)]);
        } else {
            panic!("Expected Program");
        }
    }

    #[test]
    fn test_deep_flatten() {
        let inner = IR::op(IROpCode::OpBlock, vec![IR::Int(1)]);
        let mid = IR::op(IROpCode::OpBlock, vec![inner]);
        let outer = IR::op(IROpCode::OpBlock, vec![mid]);
        // After one pass: outer → [block([1])] → [1]
        // Because inner was flattened first (map_children), then outer flattens
        let result = opt(outer);
        assert_eq!(result.args().unwrap(), &[IR::Int(1)]);
    }
}
