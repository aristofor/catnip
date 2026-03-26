// FILE: catnip_core/src/semantic/passes/strength_reduction.rs
//! Strength reduction pass (pure Rust).
//!
//! Replaces expensive operations with cheaper equivalents:
//! - x ** 2 → x * x
//! - x ** 1 → x, x ** 0 → 1
//! - x * 0 → 0, x * 1 → x
//! - x + 0 → x, x - 0 → x
//! - x / 1 → x, x // 1 → x
//! - x && True → x, x && False → False
//! - x || False → x, x || True → True

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
        IROpCode::Mul => {
            if is_int_val(right, 1) {
                return Some(left.clone());
            }
            if is_int_val(left, 1) {
                return Some(right.clone());
            }
            if is_int_val(right, 0) || is_int_val(left, 0) {
                return Some(IR::Int(0));
            }
            None
        }
        IROpCode::Pow => {
            if is_int_val(right, 2) {
                return Some(IR::op(IROpCode::Mul, vec![left.clone(), left.clone()]));
            }
            if is_int_val(right, 1) {
                return Some(left.clone());
            }
            if is_int_val(right, 0) {
                return Some(IR::Int(1));
            }
            None
        }
        IROpCode::Add => {
            if is_int_val(right, 0) {
                return Some(left.clone());
            }
            if is_int_val(left, 0) {
                return Some(right.clone());
            }
            None
        }
        IROpCode::Sub => {
            if is_int_val(right, 0) {
                return Some(left.clone());
            }
            None
        }
        IROpCode::TrueDiv | IROpCode::Div => {
            if is_int_val(right, 1) {
                return Some(left.clone());
            }
            None
        }
        IROpCode::FloorDiv => {
            if is_int_val(right, 1) {
                return Some(left.clone());
            }
            None
        }
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

fn is_int_val(ir: &IR, val: i64) -> bool {
    match ir {
        IR::Int(v) => *v == val,
        IR::Float(v) => *v == val as f64,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(ir: IR) -> IR {
        StrengthReductionPass.optimize(ir)
    }

    #[test]
    fn test_pow_2_to_mul() {
        let x = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::Pow, vec![x.clone(), IR::Int(2)]);
        let result = opt(ir);
        assert_eq!(result.opcode(), Some(IROpCode::Mul));
        assert_eq!(result.args().unwrap(), &[x.clone(), x]);
    }

    #[test]
    fn test_pow_1() {
        let x = IR::Ref("x".into(), 0, 1);
        assert_eq!(opt(IR::op(IROpCode::Pow, vec![x.clone(), IR::Int(1)])), x);
    }

    #[test]
    fn test_pow_0() {
        let x = IR::Ref("x".into(), 0, 1);
        assert_eq!(opt(IR::op(IROpCode::Pow, vec![x, IR::Int(0)])), IR::Int(1));
    }

    #[test]
    fn test_mul_identity() {
        let x = IR::Ref("x".into(), 0, 1);
        assert_eq!(opt(IR::op(IROpCode::Mul, vec![x.clone(), IR::Int(1)])), x);
    }
}
