// FILE: catnip_core/src/semantic/passes/strength_reduction.rs
//! Strength reduction pass (pure Rust).
//!
//! Replaces expensive operations with cheaper equivalents:
//! - True && False → False, ... (both operands Bool literals)
//!
//! Arithmetic rewrites (x ** 2 → x * x, x * 0, x * 1, x + 0, x / 1,
//! x // 1, x ** 0, x ** 1) are intentionally absent: without type
//! information they change observable values ("abc" * 0 is "", 7.5 // 1 is
//! 7.0) and `**`/`*` dispatch to distinct overloads (`__pow__` vs `__mul__`
//! on Python objects, struct operator methods), so one cannot be rewritten
//! into the other. All-literal cases are handled by constant folding;
//! type-aware reductions belong to the JIT where runtime types are guarded.

use super::{PurePass, map_children};
use crate::ir::{IR, IROpCode};

pub struct StrengthReductionPass;

impl PurePass for StrengthReductionPass {
    fn name(&self) -> &str {
        "strength_reduction"
    }

    fn optimize(&mut self, ir: IR) -> IR {
        let visited = map_children(ir, &mut |child| self.optimize(child));
        reduce(visited)
    }
}

fn reduce(ir: IR) -> IR {
    let result = match &ir {
        IR::Op { opcode, args, .. } if args.len() == 2 => reduce_binary(*opcode, args),
        _ => None,
    };
    result.unwrap_or(ir)
}

fn reduce_binary(opcode: IROpCode, args: &[IR]) -> Option<IR> {
    let (left, right) = (&args[0], &args[1]);
    match opcode {
        // And/Or: only simplify when both operands are Bool (preserves return type)
        IROpCode::And if matches!((left, right), (IR::Bool(_), IR::Bool(_))) => {
            if matches!(right, IR::Bool(true)) {
                return Some(left.clone());
            }
            if matches!(left, IR::Bool(true)) {
                return Some(right.clone());
            }
            if matches!(right, IR::Bool(false)) || matches!(left, IR::Bool(false)) {
                return Some(IR::Bool(false));
            }
            None
        }
        IROpCode::Or if matches!((left, right), (IR::Bool(_), IR::Bool(_))) => {
            if matches!(right, IR::Bool(false)) {
                return Some(left.clone());
            }
            if matches!(left, IR::Bool(false)) {
                return Some(right.clone());
            }
            if matches!(right, IR::Bool(true)) || matches!(left, IR::Bool(true)) {
                return Some(IR::Bool(true));
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    fn opt(ir: IR) -> IR {
        StrengthReductionPass.optimize(ir)
    }

    #[test]
    fn test_pow_preserved() {
        // x ** n must never be rewritten: `**` and `*` dispatch to distinct
        // overloads (__pow__ vs __mul__), and ** 0 / ** 1 change types or
        // drop effects
        let x = IR::Ref("x".into(), 0, 1);
        for ir in [
            IR::op(IROpCode::Pow, vec![x.clone(), IR::Int(2)]),
            IR::op(IROpCode::Pow, vec![x.clone(), IR::Int(1)]),
            IR::op(IROpCode::Pow, vec![x.clone(), IR::Int(0)]),
        ] {
            assert_eq!(opt(ir.clone()), ir);
        }
    }

    #[test]
    fn test_pow_2_call_not_duplicated() {
        // f() ** 2 must NOT become f() * f(): side effects would run twice
        let call = IR::Call {
            func: Box::new(IR::Ref("f".into(), 0, 1)),
            args: vec![],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        };
        let ir = IR::op(IROpCode::Pow, vec![call, IR::Int(2)]);
        assert_eq!(opt(ir.clone()), ir);
    }

    #[test]
    fn test_mul_identities_preserved() {
        // x * 1 aliases lists, x * 0 is wrong for str/list/NaN
        let x = IR::Ref("x".into(), 0, 1);
        for ir in [
            IR::op(IROpCode::Mul, vec![x.clone(), IR::Int(1)]),
            IR::op(IROpCode::Mul, vec![x.clone(), IR::Int(0)]),
            IR::op(IROpCode::TrueDiv, vec![x.clone(), IR::Int(1)]),
            IR::op(IROpCode::FloorDiv, vec![x.clone(), IR::Int(1)]),
        ] {
            assert_eq!(opt(ir.clone()), ir);
        }
    }
}
