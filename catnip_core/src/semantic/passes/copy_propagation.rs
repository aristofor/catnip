// FILE: catnip_core/src/semantic/passes/copy_propagation.rs
//! Copy propagation pass (pure Rust).
//!
//! Eliminates redundant copies by replacing uses with their source:
//! - x = y; z = x + 1 → x = y; z = y + 1

use super::{PurePass, map_children};
use crate::ir::{IR, IROpCode};
use std::collections::HashMap;

pub struct CopyPropagationPass {
    copies: HashMap<String, String>,
}

impl CopyPropagationPass {
    pub fn new() -> Self {
        Self { copies: HashMap::new() }
    }
}

impl Default for CopyPropagationPass {
    fn default() -> Self {
        Self::new()
    }
}

impl PurePass for CopyPropagationPass {
    fn name(&self) -> &str {
        "copy_propagation"
    }

    fn optimize(&mut self, ir: IR) -> IR {
        self.copies.clear();
        self.visit(ir)
    }
}

impl CopyPropagationPass {
    fn visit(&mut self, ir: IR) -> IR {
        // Replace Ref with source, following copy chain
        if let IR::Ref(ref name, start, end) = ir {
            let mut current = name.clone();
            while let Some(source) = self.copies.get(&current) {
                current = source.clone();
            }
            if current != *name {
                return IR::Ref(current, start, end);
            }
            return ir;
        }

        // Visit children
        let visited = map_children(ir, &mut |child| self.visit(child));

        // Track copies: SetLocals(dest, Ref(source))
        if let IR::Op {
            opcode: IROpCode::SetLocals,
            ref args,
            ..
        } = visited
        {
            if args.len() >= 2 {
                if let IR::Identifier(ref dest) = args[0] {
                    if let IR::Ref(ref source, _, _) = args[1] {
                        self.copies.insert(dest.clone(), source.clone());
                    } else {
                        // Non-copy assignment invalidates
                        self.copies.remove(dest);
                    }
                }
            }
        }

        visited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(ir: IR) -> IR {
        CopyPropagationPass::new().optimize(ir)
    }

    #[test]
    fn test_propagate_copy() {
        // x = y; z = x + 1 → x = y; z = y + 1
        let assign = IR::op(
            IROpCode::SetLocals,
            vec![IR::Identifier("x".into()), IR::Ref("y".into(), 0, 1)],
        );
        let use_x = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 5, 6), IR::Int(1)]);
        let program = IR::Program(vec![assign, use_x]);
        let result = opt(program);

        if let IR::Program(items) = result {
            if let IR::Op { args, .. } = &items[1] {
                assert!(matches!(&args[0], IR::Ref(name, _, _) if name == "y"));
            }
        }
    }

    #[test]
    fn test_copy_chain() {
        // x = y; z = x → z uses y (following chain)
        let assign1 = IR::op(
            IROpCode::SetLocals,
            vec![IR::Identifier("x".into()), IR::Ref("y".into(), 0, 1)],
        );
        let assign2 = IR::op(
            IROpCode::SetLocals,
            vec![IR::Identifier("z".into()), IR::Ref("x".into(), 5, 6)],
        );
        let use_z = IR::Ref("z".into(), 10, 11);
        let program = IR::Program(vec![assign1, assign2, use_z]);
        let result = opt(program);

        if let IR::Program(items) = result {
            assert!(matches!(&items[2], IR::Ref(name, _, _) if name == "y"));
        }
    }
}
