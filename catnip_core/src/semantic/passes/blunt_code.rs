// FILE: catnip_core/src/semantic/passes/blunt_code.rs
//! Blunt code simplification pass (pure Rust).
//!
//! Simplifies patterns:
//! - not not x → x
//! - not (a == b) → a != b
//! - x == True → x, x == False → not x
//! - x + 0 → x, x * 1 → x, x * 0 → 0
//! - x and False → False, x or True → True
//! - x and x → x, x or x → x (idempotence)
//! - x and (not x) → False, x or (not x) → True (complement)

use super::{PurePass, invert_comparison, is_truthy_constant, map_children};
use crate::ir::{IR, IROpCode};

pub struct BluntCodePass;

impl PurePass for BluntCodePass {
    fn name(&self) -> &str {
        "blunt_code"
    }

    fn optimize(&mut self, ir: IR) -> IR {
        let visited = map_children(ir, &mut |child| self.optimize(child));
        simplify(visited)
    }
}

fn simplify(ir: IR) -> IR {
    let result = match &ir {
        IR::Op { opcode, args, .. } => simplify_op(*opcode, args),
        _ => None,
    };
    result.unwrap_or(ir)
}

fn simplify_op(opcode: IROpCode, args: &[IR]) -> Option<IR> {
    match opcode {
        // --- NOT ---
        IROpCode::Not if !args.is_empty() => {
            // not not x → x
            if let IR::Op {
                opcode: IROpCode::Not,
                args: inner_args,
                ..
            } = &args[0]
            {
                return inner_args.first().cloned();
            }
            // not (a cmp b) → a inv_cmp b
            if let IR::Op {
                opcode: inner_op,
                args: inner_args,
                kwargs: inner_kwargs,
                tail: inner_tail,
                start_byte: isb,
                end_byte: ieb,
            } = &args[0]
            {
                if let Some(inverted) = invert_comparison(*inner_op) {
                    return Some(IR::Op {
                        opcode: inverted,
                        args: inner_args.clone(),
                        kwargs: inner_kwargs.clone(),
                        tail: *inner_tail,
                        start_byte: *isb,
                        end_byte: *ieb,
                    });
                }
            }
            None
        }

        // --- EQ with True/False ---
        IROpCode::Eq if args.len() >= 2 => {
            let (left, right) = (&args[0], &args[1]);
            if matches!(right, IR::Bool(true)) {
                return Some(left.clone());
            }
            if matches!(left, IR::Bool(true)) {
                return Some(right.clone());
            }
            if matches!(right, IR::Bool(false)) {
                return Some(IR::op(IROpCode::Not, vec![left.clone()]));
            }
            if matches!(left, IR::Bool(false)) {
                return Some(IR::op(IROpCode::Not, vec![right.clone()]));
            }
            None
        }

        // --- ADD ---
        IROpCode::Add if args.len() >= 2 => {
            if is_zero(&args[1]) {
                return Some(args[0].clone());
            }
            if is_zero(&args[0]) {
                return Some(args[1].clone());
            }
            None
        }

        // --- SUB ---
        IROpCode::Sub if args.len() >= 2 => {
            if is_zero(&args[1]) {
                return Some(args[0].clone());
            }
            None
        }

        // --- MUL ---
        IROpCode::Mul if args.len() >= 2 => {
            if is_zero(&args[0]) || is_zero(&args[1]) {
                return Some(IR::Int(0));
            }
            if is_one(&args[1]) {
                return Some(args[0].clone());
            }
            if is_one(&args[0]) {
                return Some(args[1].clone());
            }
            None
        }

        // --- DIV / TRUEDIV ---
        IROpCode::Div | IROpCode::TrueDiv if args.len() >= 2 => {
            if is_one(&args[1]) {
                return Some(args[0].clone());
            }
            None
        }

        // --- FLOORDIV ---
        IROpCode::FloorDiv if args.len() >= 2 => {
            if is_one(&args[1]) {
                return Some(args[0].clone());
            }
            None
        }

        // --- AND (only when both operands are Bool to preserve return type) ---
        IROpCode::And if args.len() >= 2 && matches!((&args[0], &args[1]), (IR::Bool(_), IR::Bool(_))) => {
            let (left, right) = (&args[0], &args[1]);
            if matches!(right, IR::Bool(false)) || matches!(left, IR::Bool(false)) {
                return Some(IR::Bool(false));
            }
            if matches!(right, IR::Bool(true)) {
                return Some(left.clone());
            }
            if matches!(left, IR::Bool(true)) {
                return Some(right.clone());
            }
            None
        }
        IROpCode::And if args.len() >= 2 => {
            let (left, right) = (&args[0], &args[1]);
            if is_negation_of(left, right) || is_negation_of(right, left) {
                return Some(IR::Bool(false));
            }
            None
        }

        // --- OR (only when both operands are Bool to preserve return type) ---
        IROpCode::Or if args.len() >= 2 && matches!((&args[0], &args[1]), (IR::Bool(_), IR::Bool(_))) => {
            let (left, right) = (&args[0], &args[1]);
            if matches!(right, IR::Bool(true)) || matches!(left, IR::Bool(true)) {
                return Some(IR::Bool(true));
            }
            if matches!(right, IR::Bool(false)) {
                return Some(left.clone());
            }
            if matches!(left, IR::Bool(false)) {
                return Some(right.clone());
            }
            None
        }
        IROpCode::Or if args.len() >= 2 => {
            let (left, right) = (&args[0], &args[1]);
            if is_negation_of(left, right) || is_negation_of(right, left) {
                return Some(IR::Bool(true));
            }
            None
        }

        // --- IF with constant condition ---
        IROpCode::OpIf if !args.is_empty() => simplify_if(args),

        _ => None,
    }
}

/// Simplify if/elif/else with constant conditions.
/// Removes false branches, collapses true branches.
fn simplify_if(args: &[IR]) -> Option<IR> {
    let branch_items = match &args[0] {
        IR::List(items) | IR::Tuple(items) => items.as_slice(),
        _ => return None,
    };

    if branch_items.is_empty() {
        return None;
    }

    // Check first branch
    if let Some(IR::Tuple(parts)) = branch_items.first() {
        if parts.len() >= 2 {
            // First branch is always true: return its body
            if is_truthy_constant(&parts[0]) == Some(true) {
                return Some(parts[1].clone());
            }
            // First branch is always false: remove it
            if is_truthy_constant(&parts[0]) == Some(false) {
                let remaining: Vec<_> = branch_items[1..].to_vec();
                if remaining.is_empty() {
                    // No more branches: return else or None
                    return Some(args.get(1).cloned().unwrap_or(IR::None));
                }
                // Rebuild OpIf with remaining branches
                let else_clause = args.get(1).cloned();
                let mut new_args = vec![IR::Tuple(remaining)];
                if let Some(el) = else_clause {
                    new_args.push(el);
                }
                return Some(IR::op(IROpCode::OpIf, new_args));
            }
        }
    }

    None
}

fn is_zero(ir: &IR) -> bool {
    match ir {
        IR::Int(0) => true,
        IR::Float(v) => *v == 0.0,
        _ => false,
    }
}

fn is_one(ir: &IR) -> bool {
    match ir {
        IR::Int(1) => true,
        IR::Float(v) => *v == 1.0,
        _ => false,
    }
}

fn is_negation_of(a: &IR, b: &IR) -> bool {
    if let IR::Op {
        opcode: IROpCode::Not,
        args,
        ..
    } = a
    {
        if let Some(inner) = args.first() {
            return inner == b;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(ir: IR) -> IR {
        BluntCodePass.optimize(ir)
    }

    #[test]
    fn test_double_negation() {
        let x = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::Not, vec![IR::op(IROpCode::Not, vec![x.clone()])]);
        assert_eq!(opt(ir), x);
    }

    #[test]
    fn test_not_eq_to_ne() {
        let a = IR::Ref("a".into(), 0, 1);
        let b = IR::Ref("b".into(), 2, 3);
        let eq = IR::op(IROpCode::Eq, vec![a.clone(), b.clone()]);
        let ir = IR::op(IROpCode::Not, vec![eq]);
        let result = opt(ir);
        assert_eq!(result.opcode(), Some(IROpCode::Ne));
    }

    #[test]
    fn test_eq_true() {
        let x = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::Eq, vec![x.clone(), IR::Bool(true)]);
        assert_eq!(opt(ir), x);
    }

    #[test]
    fn test_eq_false() {
        let x = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::Eq, vec![x.clone(), IR::Bool(false)]);
        let result = opt(ir);
        assert_eq!(result.opcode(), Some(IROpCode::Not));
    }

    #[test]
    fn test_add_zero() {
        let x = IR::Ref("x".into(), 0, 1);
        assert_eq!(opt(IR::op(IROpCode::Add, vec![x.clone(), IR::Int(0)])), x);
        assert_eq!(opt(IR::op(IROpCode::Add, vec![IR::Int(0), x.clone()])), x);
    }

    #[test]
    fn test_mul_zero() {
        let x = IR::Ref("x".into(), 0, 1);
        assert_eq!(opt(IR::op(IROpCode::Mul, vec![x.clone(), IR::Int(0)])), IR::Int(0));
    }

    #[test]
    fn test_mul_one() {
        let x = IR::Ref("x".into(), 0, 1);
        assert_eq!(opt(IR::op(IROpCode::Mul, vec![x.clone(), IR::Int(1)])), x);
    }

    #[test]
    fn test_and_false() {
        // Bool/Bool: can simplify
        assert_eq!(
            opt(IR::op(IROpCode::And, vec![IR::Bool(true), IR::Bool(false)])),
            IR::Bool(false)
        );
        // Non-Bool operand: no simplification (and returns bool in Catnip)
        let x = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::And, vec![x.clone(), IR::Bool(false)]);
        assert_eq!(opt(ir.clone()), ir);
    }

    #[test]
    fn test_or_true() {
        // Bool/Bool: can simplify
        assert_eq!(
            opt(IR::op(IROpCode::Or, vec![IR::Bool(false), IR::Bool(true)])),
            IR::Bool(true)
        );
        // Non-Bool operand: no simplification
        let x = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::Or, vec![x.clone(), IR::Bool(true)]);
        assert_eq!(opt(ir.clone()), ir);
    }

    #[test]
    fn test_idempotence_and() {
        // Bool/Bool idempotence
        assert_eq!(
            opt(IR::op(IROpCode::And, vec![IR::Bool(true), IR::Bool(true)])),
            IR::Bool(true)
        );
    }

    #[test]
    fn test_complement_and() {
        let x = IR::Ref("x".into(), 0, 1);
        let not_x = IR::op(IROpCode::Not, vec![x.clone()]);
        assert_eq!(opt(IR::op(IROpCode::And, vec![x, not_x])), IR::Bool(false));
    }

    #[test]
    fn test_if_false_elif_true() {
        // if False { 1 } elif True { 2 } else { 3 }
        // → step 1: removes false branch → if True { 2 } else { 3 }
        // → step 2: if True → 2
        let branches = IR::Tuple(vec![
            IR::Tuple(vec![IR::Bool(false), IR::Int(1)]),
            IR::Tuple(vec![IR::Bool(true), IR::Int(2)]),
        ]);
        let ir = IR::op(IROpCode::OpIf, vec![branches, IR::Int(3)]);

        // After one pass: removes false branch
        let step1 = opt(ir);
        match &step1 {
            IR::Op {
                opcode: IROpCode::OpIf,
                args,
                ..
            } => {
                // Remaining branches should have just the True branch
                if let IR::Tuple(branches) = &args[0] {
                    assert_eq!(branches.len(), 1);
                }
            }
            _ => panic!("Expected OpIf after step 1, got {:?}", step1),
        }

        // After second pass: simplifies if True { 2 } → 2
        let step2 = opt(step1);
        assert_eq!(step2, IR::Int(2));
    }

    #[test]
    fn test_complement_or() {
        let x = IR::Ref("x".into(), 0, 1);
        let not_x = IR::op(IROpCode::Not, vec![x.clone()]);
        assert_eq!(opt(IR::op(IROpCode::Or, vec![x, not_x])), IR::Bool(true));
    }
}
