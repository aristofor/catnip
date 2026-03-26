// FILE: catnip_core/src/semantic/passes/constant_propagation.rs
//! Constant propagation pass (pure Rust).
//!
//! Propagates constant values through variable references:
//! - x = 42; y = x + 1 → x = 42; y = 42 + 1

use super::{PurePass, map_children};
use crate::ir::{IR, IROpCode};
use std::collections::HashMap;

pub struct ConstantPropagationPass {
    constants: HashMap<String, IR>,
}

impl ConstantPropagationPass {
    pub fn new() -> Self {
        Self {
            constants: HashMap::new(),
        }
    }
}

impl Default for ConstantPropagationPass {
    fn default() -> Self {
        Self::new()
    }
}

impl PurePass for ConstantPropagationPass {
    fn name(&self) -> &str {
        "constant_propagation"
    }

    fn optimize(&mut self, ir: IR) -> IR {
        self.constants.clear();
        self.visit(ir)
    }
}

impl ConstantPropagationPass {
    fn visit(&mut self, ir: IR) -> IR {
        // Replace Ref with known constant
        if let IR::Ref(ref name, _, _) = ir {
            if let Some(constant) = self.constants.get(name) {
                return constant.clone();
            }
            return ir;
        }

        // Visit children
        let visited = map_children(ir, &mut |child| self.visit(child));

        // Track constant assignments: SetLocals(name, literal)
        if let IR::Op {
            opcode: IROpCode::SetLocals,
            ref args,
            ..
        } = visited
        {
            if args.len() >= 2 {
                if let IR::Identifier(ref name) = args[0] {
                    if args[1].is_literal() {
                        self.constants.insert(name.clone(), args[1].clone());
                    } else {
                        // Non-constant assignment invalidates previous constant
                        self.constants.remove(name);
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
        ConstantPropagationPass::new().optimize(ir)
    }

    #[test]
    fn test_propagate_constant() {
        // x = 42; y = x + 1 → x = 42; y = 42 + 1
        let assign_x = IR::op(IROpCode::SetLocals, vec![IR::Identifier("x".into()), IR::Int(42)]);
        let use_x = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 0, 1), IR::Int(1)]);
        let program = IR::Program(vec![assign_x, use_x]);
        let result = opt(program);

        if let IR::Program(items) = result {
            // Second statement should have 42 instead of Ref("x")
            if let IR::Op { args, .. } = &items[1] {
                assert_eq!(args[0], IR::Int(42));
            } else {
                panic!("Expected Op");
            }
        } else {
            panic!("Expected Program");
        }
    }

    #[test]
    fn test_no_propagate_non_literal() {
        // x = f(); y = x → x stays as Ref
        let assign_x = IR::op(
            IROpCode::SetLocals,
            vec![IR::Identifier("x".into()), IR::call(IR::Ref("f".into(), 0, 1), vec![])],
        );
        let use_x = IR::Ref("x".into(), 5, 6);
        let program = IR::Program(vec![assign_x, use_x]);
        let result = opt(program);

        if let IR::Program(items) = result {
            assert!(matches!(&items[1], IR::Ref(name, _, _) if name == "x"));
        } else {
            panic!("Expected Program");
        }
    }

    #[test]
    fn test_invalidate_on_reassign() {
        // x = 42; x = f(); y = x → y stays as Ref
        let assign1 = IR::op(IROpCode::SetLocals, vec![IR::Identifier("x".into()), IR::Int(42)]);
        let assign2 = IR::op(
            IROpCode::SetLocals,
            vec![IR::Identifier("x".into()), IR::call(IR::Ref("f".into(), 0, 1), vec![])],
        );
        let use_x = IR::Ref("x".into(), 10, 11);
        let program = IR::Program(vec![assign1, assign2, use_x]);
        let result = opt(program);

        if let IR::Program(items) = result {
            assert!(matches!(&items[2], IR::Ref(name, _, _) if name == "x"));
        } else {
            panic!("Expected Program");
        }
    }
}
