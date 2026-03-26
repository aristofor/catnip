// FILE: catnip_core/src/semantic/passes/constant_folding.rs
//! Constant folding pass (pure Rust).
//!
//! Evaluates constant expressions at compile time:
//! - 2 + 3 → 5
//! - "hello" + " world" → "hello world"
//! - not True → False
//! - 3 * 2 ** 4 → 48

use super::{PurePass, is_truthy_constant, map_children};
use crate::ir::{IR, IROpCode};

pub struct ConstantFoldingPass;

impl PurePass for ConstantFoldingPass {
    fn name(&self) -> &str {
        "constant_folding"
    }

    fn optimize(&mut self, ir: IR) -> IR {
        let visited = map_children(ir, &mut |child| self.optimize(child));
        fold(visited)
    }
}

fn fold(ir: IR) -> IR {
    match &ir {
        IR::Op { opcode, args, .. } => {
            if !args.iter().all(|a| a.is_literal()) {
                return ir;
            }
            fold_op(*opcode, args).unwrap_or(ir)
        }
        _ => ir,
    }
}

fn fold_op(opcode: IROpCode, args: &[IR]) -> Option<IR> {
    match opcode {
        // Arithmetic
        IROpCode::Add => fold_add(args),
        IROpCode::Sub => fold_binary_num(args, |a, b| a - b, |a, b| a - b),
        IROpCode::Mul => fold_mul(args),
        IROpCode::TrueDiv | IROpCode::Div => fold_div(args),
        IROpCode::FloorDiv => fold_floordiv(args),
        IROpCode::Mod => fold_mod(args),
        IROpCode::Pow => fold_pow(args),
        IROpCode::Neg => fold_unary_num(args, |a| -a, |a| -a),
        IROpCode::Pos if args.len() == 1 => Some(args[0].clone()),

        // Comparison
        IROpCode::Eq => fold_cmp(args, |ord| ord == std::cmp::Ordering::Equal),
        IROpCode::Ne => fold_cmp(args, |ord| ord != std::cmp::Ordering::Equal),
        IROpCode::Lt => fold_cmp(args, |ord| ord == std::cmp::Ordering::Less),
        IROpCode::Le => fold_cmp(args, |ord| ord != std::cmp::Ordering::Greater),
        IROpCode::Gt => fold_cmp(args, |ord| ord == std::cmp::Ordering::Greater),
        IROpCode::Ge => fold_cmp(args, |ord| ord != std::cmp::Ordering::Less),

        // Logical
        IROpCode::And if args.len() >= 2 => {
            let a = is_truthy_constant(&args[0])?;
            let b = is_truthy_constant(&args[1])?;
            Some(IR::Bool(a && b))
        }
        IROpCode::Or if args.len() >= 2 => {
            let a = is_truthy_constant(&args[0])?;
            let b = is_truthy_constant(&args[1])?;
            Some(IR::Bool(a || b))
        }
        IROpCode::Not if args.len() == 1 => {
            let v = is_truthy_constant(&args[0])?;
            Some(IR::Bool(!v))
        }

        // Bitwise
        IROpCode::BAnd => fold_bitwise(args, |a, b| a & b),
        IROpCode::BOr => fold_bitwise(args, |a, b| a | b),
        IROpCode::BXor => fold_bitwise(args, |a, b| a ^ b),
        IROpCode::BNot if args.len() == 1 => match &args[0] {
            IR::Int(v) => Some(IR::Int(!v)),
            _ => None,
        },
        IROpCode::LShift => fold_bitwise(args, |a, b| a << b),
        IROpCode::RShift => fold_bitwise(args, |a, b| a >> b),

        _ => None,
    }
}

// --- Helpers ---

fn fold_add(args: &[IR]) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    match (&args[0], &args[1]) {
        (IR::Int(a), IR::Int(b)) => a.checked_add(*b).map(IR::Int),
        (IR::Float(a), IR::Float(b)) => Some(IR::Float(a + b)),
        (IR::Int(a), IR::Float(b)) => Some(IR::Float(*a as f64 + b)),
        (IR::Float(a), IR::Int(b)) => Some(IR::Float(a + *b as f64)),
        (IR::String(a), IR::String(b)) => {
            let mut s = String::with_capacity(a.len() + b.len());
            s.push_str(a);
            s.push_str(b);
            Some(IR::String(s))
        }
        _ => None,
    }
}

fn fold_mul(args: &[IR]) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    match (&args[0], &args[1]) {
        (IR::Int(a), IR::Int(b)) => a.checked_mul(*b).map(IR::Int),
        (IR::Float(a), IR::Float(b)) => Some(IR::Float(a * b)),
        (IR::Int(a), IR::Float(b)) => Some(IR::Float(*a as f64 * b)),
        (IR::Float(a), IR::Int(b)) => Some(IR::Float(a * *b as f64)),
        (IR::String(s), IR::Int(n)) if *n >= 0 => Some(IR::String(s.repeat(*n as usize))),
        (IR::Int(n), IR::String(s)) if *n >= 0 => Some(IR::String(s.repeat(*n as usize))),
        _ => None,
    }
}

fn fold_binary_num(args: &[IR], int_op: fn(i64, i64) -> i64, float_op: fn(f64, f64) -> f64) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    match (&args[0], &args[1]) {
        (IR::Int(a), IR::Int(b)) => Some(IR::Int(int_op(*a, *b))),
        (IR::Float(a), IR::Float(b)) => Some(IR::Float(float_op(*a, *b))),
        (IR::Int(a), IR::Float(b)) => Some(IR::Float(float_op(*a as f64, *b))),
        (IR::Float(a), IR::Int(b)) => Some(IR::Float(float_op(*a, *b as f64))),
        _ => None,
    }
}

fn fold_div(args: &[IR]) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    // Check division by zero
    match &args[1] {
        IR::Int(0) => return None,
        IR::Float(v) if *v == 0.0 => return None,
        _ => {}
    }
    match (&args[0], &args[1]) {
        (IR::Int(a), IR::Int(b)) => Some(IR::Float(*a as f64 / *b as f64)),
        (IR::Float(a), IR::Float(b)) => Some(IR::Float(a / b)),
        (IR::Int(a), IR::Float(b)) => Some(IR::Float(*a as f64 / b)),
        (IR::Float(a), IR::Int(b)) => Some(IR::Float(a / *b as f64)),
        _ => None,
    }
}

fn fold_floordiv(args: &[IR]) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    match &args[1] {
        IR::Int(0) => return None,
        IR::Float(v) if *v == 0.0 => return None,
        _ => {}
    }
    match (&args[0], &args[1]) {
        (IR::Int(a), IR::Int(b)) => Some(IR::Int(python_floordiv(*a, *b))),
        (IR::Float(a), IR::Float(b)) => Some(IR::Float((a / b).floor())),
        (IR::Int(a), IR::Float(b)) => Some(IR::Float((*a as f64 / b).floor())),
        (IR::Float(a), IR::Int(b)) => Some(IR::Float((a / *b as f64).floor())),
        _ => None,
    }
}

fn fold_mod(args: &[IR]) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    match &args[1] {
        IR::Int(0) => return None,
        IR::Float(v) if *v == 0.0 => return None,
        _ => {}
    }
    match (&args[0], &args[1]) {
        (IR::Int(a), IR::Int(b)) => Some(IR::Int(python_mod(*a, *b))),
        // Python floored modulo: a - b * floor(a / b)
        (IR::Float(a), IR::Float(b)) => Some(IR::Float(a - b * (a / b).floor())),
        (IR::Int(a), IR::Float(b)) => {
            let a = *a as f64;
            Some(IR::Float(a - b * (a / b).floor()))
        }
        (IR::Float(a), IR::Int(b)) => {
            let b = *b as f64;
            Some(IR::Float(a - b * (a / b).floor()))
        }
        _ => None,
    }
}

fn fold_pow(args: &[IR]) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    match (&args[0], &args[1]) {
        (IR::Int(base), IR::Int(exp)) => {
            if *exp < 0 {
                Some(IR::Float((*base as f64).powf(*exp as f64)))
            } else if *exp <= 20 {
                base.checked_pow(*exp as u32).map(IR::Int)
            } else {
                None
            }
        }
        (IR::Float(a), IR::Float(b)) => Some(IR::Float(a.powf(*b))),
        (IR::Int(a), IR::Float(b)) => Some(IR::Float((*a as f64).powf(*b))),
        (IR::Float(a), IR::Int(b)) => Some(IR::Float(a.powf(*b as f64))),
        _ => None,
    }
}

fn fold_unary_num(args: &[IR], int_op: fn(i64) -> i64, float_op: fn(f64) -> f64) -> Option<IR> {
    if args.len() != 1 {
        return None;
    }
    match &args[0] {
        IR::Int(v) => Some(IR::Int(int_op(*v))),
        IR::Float(v) => Some(IR::Float(float_op(*v))),
        _ => None,
    }
}

fn fold_cmp(args: &[IR], predicate: fn(std::cmp::Ordering) -> bool) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    let ord = compare_literals(&args[0], &args[1])?;
    Some(IR::Bool(predicate(ord)))
}

fn compare_literals(a: &IR, b: &IR) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (IR::Int(a), IR::Int(b)) => Some(a.cmp(b)),
        (IR::Float(a), IR::Float(b)) => a.partial_cmp(b),
        (IR::Int(a), IR::Float(b)) => (*a as f64).partial_cmp(b),
        (IR::Float(a), IR::Int(b)) => a.partial_cmp(&(*b as f64)),
        (IR::String(a), IR::String(b)) => Some(a.cmp(b)),
        (IR::Bool(a), IR::Bool(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

fn fold_bitwise(args: &[IR], op: fn(i64, i64) -> i64) -> Option<IR> {
    if args.len() < 2 {
        return None;
    }
    match (&args[0], &args[1]) {
        (IR::Int(a), IR::Int(b)) => Some(IR::Int(op(*a, *b))),
        _ => None,
    }
}

/// Python-compatible floor division (rounds toward -infinity)
fn python_floordiv(a: i64, b: i64) -> i64 {
    let q = a / b;
    let r = a % b;
    if r != 0 && (r ^ b) < 0 { q - 1 } else { q }
}

/// Python-compatible modulo (result has sign of divisor)
fn python_mod(a: i64, b: i64) -> i64 {
    let r = a % b;
    if r != 0 && (r ^ b) < 0 { r + b } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(ir: IR) -> IR {
        ConstantFoldingPass.optimize(ir)
    }

    #[test]
    fn test_add_ints() {
        let ir = IR::op(IROpCode::Add, vec![IR::Int(2), IR::Int(3)]);
        assert_eq!(opt(ir), IR::Int(5));
    }

    #[test]
    fn test_add_floats() {
        let ir = IR::op(IROpCode::Add, vec![IR::Float(1.5), IR::Float(2.5)]);
        assert_eq!(opt(ir), IR::Float(4.0));
    }

    #[test]
    fn test_add_strings() {
        let ir = IR::op(
            IROpCode::Add,
            vec![IR::String("hello".into()), IR::String(" world".into())],
        );
        assert_eq!(opt(ir), IR::String("hello world".into()));
    }

    #[test]
    fn test_mul_string_int() {
        let ir = IR::op(IROpCode::Mul, vec![IR::String("ab".into()), IR::Int(3)]);
        assert_eq!(opt(ir), IR::String("ababab".into()));
    }

    #[test]
    fn test_sub() {
        let ir = IR::op(IROpCode::Sub, vec![IR::Int(10), IR::Int(3)]);
        assert_eq!(opt(ir), IR::Int(7));
    }

    #[test]
    fn test_mul() {
        let ir = IR::op(IROpCode::Mul, vec![IR::Int(4), IR::Int(5)]);
        assert_eq!(opt(ir), IR::Int(20));
    }

    #[test]
    fn test_truediv() {
        let ir = IR::op(IROpCode::TrueDiv, vec![IR::Int(7), IR::Int(2)]);
        assert_eq!(opt(ir), IR::Float(3.5));
    }

    #[test]
    fn test_floordiv() {
        let ir = IR::op(IROpCode::FloorDiv, vec![IR::Int(7), IR::Int(2)]);
        assert_eq!(opt(ir), IR::Int(3));
    }

    #[test]
    fn test_floordiv_negative() {
        let ir = IR::op(IROpCode::FloorDiv, vec![IR::Int(-7), IR::Int(2)]);
        assert_eq!(opt(ir), IR::Int(-4));
    }

    #[test]
    fn test_mod() {
        let ir = IR::op(IROpCode::Mod, vec![IR::Int(7), IR::Int(3)]);
        assert_eq!(opt(ir), IR::Int(1));
    }

    #[test]
    fn test_mod_negative() {
        // Python: -7 % 3 = 2 (not -1)
        let ir = IR::op(IROpCode::Mod, vec![IR::Int(-7), IR::Int(3)]);
        assert_eq!(opt(ir), IR::Int(2));
    }

    #[test]
    fn test_pow() {
        let ir = IR::op(IROpCode::Pow, vec![IR::Int(2), IR::Int(10)]);
        assert_eq!(opt(ir), IR::Int(1024));
    }

    #[test]
    fn test_neg() {
        let ir = IR::op(IROpCode::Neg, vec![IR::Int(42)]);
        assert_eq!(opt(ir), IR::Int(-42));
    }

    #[test]
    fn test_not() {
        let ir = IR::op(IROpCode::Not, vec![IR::Bool(true)]);
        assert_eq!(opt(ir), IR::Bool(false));
    }

    #[test]
    fn test_eq() {
        let ir = IR::op(IROpCode::Eq, vec![IR::Int(1), IR::Int(1)]);
        assert_eq!(opt(ir), IR::Bool(true));
    }

    #[test]
    fn test_lt() {
        let ir = IR::op(IROpCode::Lt, vec![IR::Int(1), IR::Int(2)]);
        assert_eq!(opt(ir), IR::Bool(true));
    }

    #[test]
    fn test_band() {
        let ir = IR::op(IROpCode::BAnd, vec![IR::Int(0b1010), IR::Int(0b1100)]);
        assert_eq!(opt(ir), IR::Int(0b1000));
    }

    #[test]
    fn test_bnot() {
        let ir = IR::op(IROpCode::BNot, vec![IR::Int(0)]);
        assert_eq!(opt(ir), IR::Int(-1));
    }

    #[test]
    fn test_no_fold_non_literal() {
        let ir = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 0, 1), IR::Int(1)]);
        let result = opt(ir.clone());
        assert_eq!(result, ir);
    }

    #[test]
    fn test_div_by_zero_no_fold() {
        let ir = IR::op(IROpCode::TrueDiv, vec![IR::Int(1), IR::Int(0)]);
        let result = opt(ir.clone());
        assert_eq!(result, ir);
    }

    #[test]
    fn test_nested_fold() {
        // (2 + 3) * 4 → 5 * 4 → 20
        let add = IR::op(IROpCode::Add, vec![IR::Int(2), IR::Int(3)]);
        let mul = IR::op(IROpCode::Mul, vec![add, IR::Int(4)]);
        assert_eq!(opt(mul), IR::Int(20));
    }
}
