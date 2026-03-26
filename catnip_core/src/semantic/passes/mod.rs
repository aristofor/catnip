// FILE: catnip_core/src/semantic/passes/mod.rs
//! Pure optimization passes for IR - no PyO3 dependency.
//!
//! Each pass implements `PurePass` and transforms IR trees.
//! The `PureOptimizer` applies all passes iteratively until fixpoint.

mod block_flattening;
mod blunt_code;
mod constant_folding;
mod constant_propagation;
mod copy_propagation;
mod dead_code_elimination;
mod dead_store_elimination;
mod strength_reduction;

pub use block_flattening::BlockFlatteningPass;
pub use blunt_code::BluntCodePass;
pub use constant_folding::ConstantFoldingPass;
pub use constant_propagation::ConstantPropagationPass;
pub use copy_propagation::CopyPropagationPass;
pub use dead_code_elimination::DeadCodeEliminationPass;
pub use dead_store_elimination::DeadStoreEliminationPass;
pub use strength_reduction::StrengthReductionPass;

use crate::ir::{IR, IROpCode};

/// Trait for pure optimization passes operating on IR.
pub trait PurePass {
    /// Pass name for debugging
    fn name(&self) -> &str;

    /// Optimize an IR tree
    fn optimize(&mut self, ir: IR) -> IR;
}

/// Optimizer applying a sequence of passes iteratively until fixpoint.
pub struct PureOptimizer {
    passes: Vec<Box<dyn PurePass>>,
    max_iterations: usize,
}

impl PureOptimizer {
    /// Create optimizer with default pass order
    pub fn new() -> Self {
        Self {
            passes: vec![
                Box::new(BluntCodePass),
                Box::new(ConstantPropagationPass::new()),
                Box::new(ConstantFoldingPass),
                Box::new(CopyPropagationPass::new()),
                // FunctionInlining: not ported (legacy only)
                Box::new(DeadStoreEliminationPass),
                Box::new(StrengthReductionPass),
                Box::new(BlockFlatteningPass),
                Box::new(DeadCodeEliminationPass),
                // CSE: not yet ported (complex, 667 lines)
            ],
            max_iterations: 10,
        }
    }

    /// Optimize an IR tree through all passes until fixpoint or max iterations
    pub fn optimize(&mut self, ir: IR) -> IR {
        let mut result = ir;
        for _ in 0..self.max_iterations {
            let before = result.clone();
            for pass in &mut self.passes {
                result = pass.optimize(result);
            }
            if result == before {
                break;
            }
        }
        result
    }
}

impl Default for PureOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively apply a transformation to all children of an IR node.
pub fn map_children(ir: IR, mut f: &mut dyn FnMut(IR) -> IR) -> IR {
    match ir {
        IR::Op {
            opcode,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        } => IR::Op {
            opcode,
            args: args.into_iter().map(&mut f).collect(),
            kwargs: kwargs.into_iter().map(|(k, v)| (k, f(v))).collect(),
            tail,
            start_byte,
            end_byte,
        },
        IR::Program(items) => IR::Program(items.into_iter().map(&mut f).collect()),
        IR::List(items) => IR::List(items.into_iter().map(&mut f).collect()),
        IR::Tuple(items) => IR::Tuple(items.into_iter().map(&mut f).collect()),
        IR::Set(items) => IR::Set(items.into_iter().map(&mut f).collect()),
        IR::Dict(pairs) => IR::Dict(pairs.into_iter().map(|(k, v)| (f(k), f(v))).collect()),
        IR::Call {
            func,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        } => IR::Call {
            func: Box::new(f(*func)),
            args: args.into_iter().map(&mut f).collect(),
            kwargs: kwargs.into_iter().map(|(k, v)| (k, f(v))).collect(),
            tail,
            start_byte,
            end_byte,
        },
        IR::PatternLiteral(v) => IR::PatternLiteral(Box::new(f(*v))),
        IR::PatternOr(ps) => IR::PatternOr(ps.into_iter().map(&mut f).collect()),
        IR::PatternTuple(ps) => IR::PatternTuple(ps.into_iter().map(&mut f).collect()),
        IR::Slice { start, stop, step } => IR::Slice {
            start: Box::new(f(*start)),
            stop: Box::new(f(*stop)),
            step: Box::new(f(*step)),
        },
        IR::Broadcast {
            target,
            operator,
            operand,
            broadcast_type,
        } => IR::Broadcast {
            target: target.map(|t| Box::new(f(*t))),
            operator: Box::new(f(*operator)),
            operand: operand.map(|o| Box::new(f(*o))),
            broadcast_type,
        },
        // Leaf nodes: no children
        other => other,
    }
}

/// Check if an IR node is a constant with a known truthiness.
pub fn is_truthy_constant(ir: &IR) -> Option<bool> {
    match ir {
        IR::Bool(v) => Some(*v),
        IR::Int(v) => Some(*v != 0),
        IR::Float(v) => Some(*v != 0.0),
        IR::String(v) => Some(!v.is_empty()),
        IR::None => Some(false),
        _ => None,
    }
}

/// Invert a comparison opcode (Eq↔Ne, Lt↔Ge, Le↔Gt).
pub fn invert_comparison(opcode: IROpCode) -> Option<IROpCode> {
    match opcode {
        IROpCode::Eq => Some(IROpCode::Ne),
        IROpCode::Ne => Some(IROpCode::Eq),
        IROpCode::Lt => Some(IROpCode::Ge),
        IROpCode::Le => Some(IROpCode::Gt),
        IROpCode::Gt => Some(IROpCode::Le),
        IROpCode::Ge => Some(IROpCode::Lt),
        _ => None,
    }
}

/// Collect all Ref names in an IR tree.
pub fn collect_refs(ir: &IR, f: &mut dyn FnMut(String)) {
    match ir {
        IR::Ref(name, _, _) => f(name.clone()),
        IR::Op { args, kwargs, .. } => {
            for arg in args {
                collect_refs(arg, f);
            }
            for (_, v) in kwargs {
                collect_refs(v, f);
            }
        }
        IR::Call { func, args, kwargs, .. } => {
            collect_refs(func, f);
            for arg in args {
                collect_refs(arg, f);
            }
            for (_, v) in kwargs {
                collect_refs(v, f);
            }
        }
        IR::Program(items) | IR::List(items) | IR::Tuple(items) | IR::Set(items) => {
            for item in items {
                collect_refs(item, f);
            }
        }
        IR::Dict(pairs) => {
            for (k, v) in pairs {
                collect_refs(k, f);
                collect_refs(v, f);
            }
        }
        IR::PatternLiteral(v) => collect_refs(v, f),
        IR::PatternOr(ps) | IR::PatternTuple(ps) => {
            for p in ps {
                collect_refs(p, f);
            }
        }
        IR::Slice { start, stop, step } => {
            collect_refs(start, f);
            collect_refs(stop, f);
            collect_refs(step, f);
        }
        IR::Broadcast {
            target,
            operator,
            operand,
            ..
        } => {
            if let Some(t) = target {
                collect_refs(t, f);
            }
            collect_refs(operator, f);
            if let Some(o) = operand {
                collect_refs(o, f);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_children_leaf() {
        let ir = IR::Int(42);
        let result = map_children(ir.clone(), &mut |x| x);
        assert_eq!(result, ir);
    }

    #[test]
    fn test_map_children_op() {
        let ir = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        let result = map_children(ir, &mut |x| {
            if let IR::Int(n) = x { IR::Int(n * 10) } else { x }
        });
        assert_eq!(result, IR::op(IROpCode::Add, vec![IR::Int(10), IR::Int(20)]));
    }

    #[test]
    fn test_truthy_constants() {
        assert_eq!(is_truthy_constant(&IR::Bool(true)), Some(true));
        assert_eq!(is_truthy_constant(&IR::Bool(false)), Some(false));
        assert_eq!(is_truthy_constant(&IR::Int(0)), Some(false));
        assert_eq!(is_truthy_constant(&IR::Int(42)), Some(true));
        assert_eq!(is_truthy_constant(&IR::None), Some(false));
        assert_eq!(is_truthy_constant(&IR::Identifier("x".into())), None);
    }

    #[test]
    fn test_invert_comparison() {
        assert_eq!(invert_comparison(IROpCode::Eq), Some(IROpCode::Ne));
        assert_eq!(invert_comparison(IROpCode::Lt), Some(IROpCode::Ge));
        assert_eq!(invert_comparison(IROpCode::Add), None);
    }

    #[test]
    fn test_optimizer_fixpoint() {
        let mut opt = PureOptimizer::new();
        let ir = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(0)]);
        let result = opt.optimize(ir);
        // BluntCode: x + 0 → x
        assert_eq!(result, IR::Int(1));
    }

    #[test]
    fn test_collect_refs() {
        let ir = IR::op(
            IROpCode::Add,
            vec![IR::Ref("x".into(), 0, 1), IR::Ref("y".into(), 2, 3)],
        );
        let mut refs = Vec::new();
        collect_refs(&ir, &mut |name| refs.push(name));
        assert_eq!(refs, vec!["x", "y"]);
    }
}
