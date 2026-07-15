// FILE: catnip_core/src/cfg/ssa_cse.rs
//! Shared helpers for the value-numbering passes (GVN and friends).
//!
//! Holds the purity table (`pure_opcodes`) and the RHS-operand extraction
//! (`extract_rhs_opcode`, `rhs_operands`) used to build expression keys. The
//! inter-block redundancy pass itself is Global Value Numbering (`ssa_gvn.rs`),
//! which subsumes plain syntactic CSE.

use super::ssa::{ExprOperand, SSAValue};
use crate::ir::{IR, IROpCode};
use std::collections::HashSet;

/// Pure operations safe to value-number (no side effects, deterministic).
pub fn pure_opcodes() -> HashSet<i32> {
    let mut ops = HashSet::new();
    // Arithmetic
    ops.insert(IROpCode::Add as i32);
    ops.insert(IROpCode::Sub as i32);
    ops.insert(IROpCode::Mul as i32);
    ops.insert(IROpCode::Div as i32);
    ops.insert(IROpCode::TrueDiv as i32);
    ops.insert(IROpCode::FloorDiv as i32);
    ops.insert(IROpCode::Mod as i32);
    ops.insert(IROpCode::Pow as i32);
    ops.insert(IROpCode::Neg as i32);
    ops.insert(IROpCode::Pos as i32);
    // Comparison
    ops.insert(IROpCode::Eq as i32);
    ops.insert(IROpCode::Ne as i32);
    ops.insert(IROpCode::Lt as i32);
    ops.insert(IROpCode::Le as i32);
    ops.insert(IROpCode::Gt as i32);
    ops.insert(IROpCode::Ge as i32);
    // Logical
    ops.insert(IROpCode::And as i32);
    ops.insert(IROpCode::Or as i32);
    ops.insert(IROpCode::Not as i32);
    // Bitwise
    ops.insert(IROpCode::BAnd as i32);
    ops.insert(IROpCode::BOr as i32);
    ops.insert(IROpCode::BXor as i32);
    ops.insert(IROpCode::BNot as i32);
    ops.insert(IROpCode::LShift as i32);
    ops.insert(IROpCode::RShift as i32);
    // GetAttr/GetItem are deliberately NOT here: they read mutable state and
    // dispatch user `__getattr__`/`__getitem__` (the docs use `__getitem__` for
    // dynamic method dispatch), so `x = obj.f; obj.f = 2; y = obj.f` must not
    // collapse to `y = x` (Codex adversarial review, 2026-06-18).
    //
    // The arithmetic/comparison/etc. above can dispatch struct operator methods
    // AND allocate mutable results (list/set/str concat). This set only lists
    // value-numbering *candidates*; soundness comes from GVN materializing an
    // alias only for proven immutable scalars (`operands_all_scalar` in
    // ssa_gvn.rs), so a non-scalar (overloaded or mutable) `a + b` is never
    // aliased -- it is recomputed.
    ops
}

/// Extract the opcode of the RHS of a SetLocals instruction.
///
/// SetLocals args = (names, rhs, unpack).
/// Returns Some(rhs_opcode) if the RHS is an Op, None if it's a literal or Ref.
pub fn extract_rhs_opcode(op: &IR) -> Option<i32> {
    let IR::Op { opcode, args, .. } = op else {
        return None;
    };
    if *opcode != IROpCode::SetLocals {
        return None;
    }
    match args.get(1) {
        Some(IR::Op { opcode, .. }) => Some(*opcode as i32),
        _ => None,
    }
}

/// Whether a `SetLocals` stores the whole RHS value in a single target (`x = e`),
/// as opposed to destructuring (`(a, b) = e`, even `(x,) = e`) where each target
/// is an element of the RHS. The parser sets args[2] = `Bool(true)` exactly when
/// the lvalue was a tuple (unpacking); a falsy flag means one target holds the
/// entire RHS. Anything unexpected is treated as not-single (conservative).
pub fn is_single_target(set_locals: &IR) -> bool {
    let IR::Op { args, .. } = set_locals else {
        return false;
    };
    matches!(args.get(2), Some(IR::Bool(false)))
}

/// Extract the first-level operands of a `SetLocals`'s RHS op for value keying --
/// variables AND constants.
///
/// Keying on variable uses alone conflates `a + 4` with `a + 11`, or `d['x']`
/// with `d['y']` (same uses, different value). `collect_refs` (which produced
/// `uses`) walks `Ref` nodes only, so `uses` is consumed in order, one entry per
/// first-level `Ref`. Anything that is neither a `Ref` nor a scalar literal
/// (sub-expression, `Call`, container) makes the key un-buildable -- conservative
/// but sound. The final `use_idx` check rejects a hidden nested `Ref` (a
/// sub-expression sneaking extra uses in).
pub fn rhs_operands(set_locals: &IR, uses: &[SSAValue]) -> Option<Vec<ExprOperand>> {
    let IR::Op { args, .. } = set_locals else {
        return None;
    };
    let IR::Op {
        args: rhs_args, kwargs, ..
    } = args.get(1)?
    else {
        return None;
    };
    if !kwargs.is_empty() {
        return None;
    }

    let mut operands = Vec::with_capacity(rhs_args.len());
    let mut use_idx = 0usize;
    for arg in rhs_args {
        let operand = match arg {
            IR::Ref(..) => {
                let v = *uses.get(use_idx)?;
                use_idx += 1;
                ExprOperand::Var(v)
            }
            IR::Int(n) => ExprOperand::Int(*n),
            IR::Float(f) => ExprOperand::Float(f.to_bits()),
            IR::Bool(b) => ExprOperand::Bool(*b),
            IR::String(s) => ExprOperand::Str(s.clone()),
            IR::None => ExprOperand::None,
            _ => return None,
        };
        operands.push(operand);
    }
    if use_idx != uses.len() {
        return None;
    }

    Some(operands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pure_opcodes_complete() {
        let ops = pure_opcodes();
        assert!(ops.contains(&(IROpCode::Add as i32)));
        assert!(ops.contains(&(IROpCode::Eq as i32)));
        assert!(ops.contains(&(IROpCode::And as i32)));
        // GetAttr/GetItem are excluded: they read mutable state and dispatch
        // user code, so they are not safe to value-number without an effect model.
        assert!(!ops.contains(&(IROpCode::GetAttr as i32)));
        assert!(!ops.contains(&(IROpCode::GetItem as i32)));
        assert!(!ops.contains(&(IROpCode::OpIf as i32)));
        assert!(!ops.contains(&(IROpCode::SetLocals as i32)));
    }

    /// A literal operand must appear in the key, distinct per value -- so `a + 4`
    /// and `a + 11` do not collapse.
    #[test]
    fn test_rhs_operands_includes_literals() {
        let v = SSAValue::new(0, 0);
        let set_locals = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("x".into(), -1, -1)]),
                IR::op(IROpCode::Add, vec![IR::Ref("a".into(), -1, -1), IR::Int(4)]),
                IR::Bool(false),
            ],
        );
        let ops = rhs_operands(&set_locals, &[v]).expect("operands");
        assert_eq!(ops, vec![ExprOperand::Var(v), ExprOperand::Int(4)]);
    }

    /// The unpack flag (args[2]) decides single-target; anything malformed is
    /// not-single (conservative).
    #[test]
    fn test_is_single_target() {
        let single = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("x".into(), -1, -1)]),
                IR::Int(1),
                IR::Bool(false),
            ],
        );
        assert!(is_single_target(&single));

        let unpack = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("a".into(), -1, -1), IR::Ref("b".into(), -1, -1)]),
                IR::Ref("t".into(), -1, -1),
                IR::Bool(true),
            ],
        );
        assert!(!is_single_target(&unpack));

        let missing_flag = IR::op(
            IROpCode::SetLocals,
            vec![IR::Tuple(vec![IR::Ref("x".into(), -1, -1)]), IR::Int(1)],
        );
        assert!(!is_single_target(&missing_flag));
    }

    /// A nested sub-expression is not value-keyable (conservative).
    #[test]
    fn test_rhs_operands_rejects_subexpr() {
        let set_locals = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("x".into(), -1, -1)]),
                IR::op(
                    IROpCode::Add,
                    vec![IR::op(IROpCode::Mul, vec![IR::Int(2), IR::Int(3)]), IR::Int(4)],
                ),
                IR::Bool(false),
            ],
        );
        assert!(rhs_operands(&set_locals, &[]).is_none());
    }
}
