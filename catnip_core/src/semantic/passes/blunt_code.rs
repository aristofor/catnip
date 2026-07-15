// FILE: catnip_core/src/semantic/passes/blunt_code.rs
//! Blunt code simplification pass (pure Rust).
//!
//! Simplifies patterns:
//! - not (a == b) → a != b (Eq/Ne only: order inversions are unsound with NaN)
//! - True and False → False, ... (both operands Bool literals)
//! - x and (not x) → False, x or (not x) → True (complement, x a Ref or literal)
//! - if True { a } → a, if False { a } elif ... → elif ...
//!
//! Arithmetic identities (x + 0, x * 1, x * 0, x / 1, x // 1, x == True,
//! not not x) are intentionally absent: without type information they change
//! observable values in a dynamic language ("abc" * 0 is "", 7.5 // 1 is 7.0,
//! 5 == True is False, not not 5 is True). All-literal cases are handled by
//! constant folding.

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
            // not (a cmp b) → a inv_cmp b (Eq↔Ne only, see invert_comparison)
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

/// True when `a` is `not (inner)` and `inner` is structurally equal to `b`.
/// Gated on `inner` being a pure atom: matching arbitrary expressions would let
/// `f() and not f()` collapse to a constant, dropping both calls and their side
/// effects.
fn is_negation_of(a: &IR, b: &IR) -> bool {
    if let IR::Op {
        opcode: IROpCode::Not,
        args,
        ..
    } = a
    {
        if let Some(inner) = args.first() {
            return is_pure_atom(inner) && inner == b;
        }
    }
    false
}

/// A value whose evaluation has no side effects: scalar literals and variable
/// references. Collections and operations are excluded — their elements/operands
/// can be arbitrary expressions (`[f()]`, `a + b`). An unbound Ref still raises at
/// runtime; the complement reduction targets tautological dead code, so masking
/// that NameError is accepted, eliminating a side-effecting call is not.
fn is_pure_atom(ir: &IR) -> bool {
    matches!(
        ir,
        IR::Int(_)
            | IR::Float(_)
            | IR::String(_)
            | IR::Bytes(_)
            | IR::Bool(_)
            | IR::None
            | IR::Decimal(_)
            | IR::Imaginary(_)
            | IR::Ref(..)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(ir: IR) -> IR {
        BluntCodePass.optimize(ir)
    }

    #[test]
    fn test_double_negation_preserved() {
        // not not x must NOT become x: not not 5 is True, not 5
        let x = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::Not, vec![IR::op(IROpCode::Not, vec![x])]);
        assert_eq!(opt(ir.clone()), ir);
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
    fn test_not_lt_preserved() {
        // not (a < b) must NOT become a >= b: unsound with NaN operands
        let a = IR::Ref("a".into(), 0, 1);
        let b = IR::Ref("b".into(), 2, 3);
        let lt = IR::op(IROpCode::Lt, vec![a, b]);
        let ir = IR::op(IROpCode::Not, vec![lt]);
        assert_eq!(opt(ir.clone()), ir);
    }

    #[test]
    fn test_eq_true_preserved() {
        // x == True must NOT become x: 5 == True is False, not 5
        let x = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::Eq, vec![x, IR::Bool(true)]);
        assert_eq!(opt(ir.clone()), ir);
    }

    #[test]
    fn test_arithmetic_identities_preserved() {
        // x + 0, x * 1, x * 0: unsound without type info ("abc" * 0 is "")
        let x = IR::Ref("x".into(), 0, 1);
        for ir in [
            IR::op(IROpCode::Add, vec![x.clone(), IR::Int(0)]),
            IR::op(IROpCode::Mul, vec![x.clone(), IR::Int(1)]),
            IR::op(IROpCode::Mul, vec![x.clone(), IR::Int(0)]),
            IR::op(IROpCode::FloorDiv, vec![x.clone(), IR::Int(1)]),
            IR::op(IROpCode::TrueDiv, vec![x.clone(), IR::Int(1)]),
        ] {
            assert_eq!(opt(ir.clone()), ir);
        }
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

    #[test]
    fn test_complement_call_preserved() {
        // f() and not f() / f() or not f() must NOT collapse: structural equality
        // would drop both calls and their side effects. Only Refs/literals reduce.
        let call = IR::call(IR::Ref("f".into(), 0, 1), vec![]);
        let not_call = IR::op(IROpCode::Not, vec![call.clone()]);
        let and = IR::op(IROpCode::And, vec![call.clone(), not_call.clone()]);
        let or = IR::op(IROpCode::Or, vec![call, not_call]);
        assert_eq!(opt(and.clone()), and);
        assert_eq!(opt(or.clone()), or);
    }
}
