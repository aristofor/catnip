// FILE: catnip_rs/src/semantic/tests/test_blunt_code.rs
//! Unit tests for BluntCodePass optimization.
//!
//! These tests validate that the BluntCodePass correctly simplifies
//! inefficient or redundant patterns in the IR.

use super::helpers::*;
use crate::ir::IROpCode;
use pyo3::prelude::*;

#[test]
fn test_double_not_simplification() {
    // not not x → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, true);
        let not_x = not_op(py, &x);
        let not_not_x = not_op(py, &not_x);

        let result = run_blunt_code_pass(py, &not_not_x).expect("BluntCodePass failed");

        // Should simplify to just x (the literal True)
        assert!(
            is_value(py, &result, true),
            "not not True should simplify to True"
        );
    });
}

#[test]
fn test_triple_not_simplification() {
    // not not not x → not x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, true);
        let not_x = not_op(py, &x);
        let not_not_x = not_op(py, &not_x);
        let not_not_not_x = not_op(py, &not_not_x);

        let result = run_blunt_code_pass(py, &not_not_not_x).expect("BluntCodePass failed");

        // Should simplify to not x (which evaluates to False for True)
        assert!(
            has_opcode(py, &result, IROpCode::Not) || is_value(py, &result, false),
            "Triple negation should simplify to single NOT or False"
        );
    });
}

#[test]
fn test_x_equals_true_simplification() {
    // x == True → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, true);
        let true_lit = literal(py, true);
        let eq = eq_op(py, &x, &true_lit);

        let result = run_blunt_code_pass(py, &eq).expect("BluntCodePass failed");

        // Should simplify to just x
        assert!(
            is_value(py, &result, true),
            "x == True should simplify to x"
        );
    });
}

#[test]
fn test_true_equals_x_simplification() {
    // True == x → x
    Python::initialize();
    Python::attach(|py| {
        let true_lit = literal(py, true);
        let x = literal(py, false);
        let eq = eq_op(py, &true_lit, &x);

        let result = run_blunt_code_pass(py, &eq).expect("BluntCodePass failed");

        // Should simplify to just x (False)
        assert!(
            is_value(py, &result, false),
            "True == False should simplify to False"
        );
    });
}

#[test]
fn test_x_equals_false_simplification() {
    // x == False → not x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, true);
        let false_lit = literal(py, false);
        let eq = eq_op(py, &x, &false_lit);

        let result = run_blunt_code_pass(py, &eq).expect("BluntCodePass failed");

        // Should simplify to not x (not True = False)
        assert!(
            is_value(py, &result, false) || has_opcode(py, &result, IROpCode::Not),
            "True == False should simplify to False or not True"
        );
    });
}

#[test]
fn test_false_equals_x_simplification() {
    // False == x → not x
    Python::initialize();
    Python::attach(|py| {
        let false_lit = literal(py, false);
        let x = literal(py, true);
        let eq = eq_op(py, &false_lit, &x);

        let result = run_blunt_code_pass(py, &eq).expect("BluntCodePass failed");

        // Should simplify to not x (not True = False)
        assert!(
            is_value(py, &result, false) || has_opcode(py, &result, IROpCode::Not),
            "False == True should simplify to False or not True"
        );
    });
}

#[test]
fn test_true_and_x_simplification() {
    // True and x → x
    Python::initialize();
    Python::attach(|py| {
        let true_lit = literal(py, true);
        let x = literal(py, 42);
        let and_expr = and_op(py, &true_lit, &x);

        let result = run_blunt_code_pass(py, &and_expr).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "True and x should simplify to x");
    });
}

#[test]
fn test_false_and_x_simplification() {
    // False and x → False
    Python::initialize();
    Python::attach(|py| {
        let false_lit = literal(py, false);
        let x = literal(py, 42);
        let and_expr = and_op(py, &false_lit, &x);

        let result = run_blunt_code_pass(py, &and_expr).expect("BluntCodePass failed");

        // Should simplify to False
        assert!(
            is_value(py, &result, false),
            "False and x should simplify to False"
        );
    });
}

#[test]
fn test_true_or_x_simplification() {
    // True or x → True
    Python::initialize();
    Python::attach(|py| {
        let true_lit = literal(py, true);
        let x = literal(py, 42);
        let or_expr = or_op(py, &true_lit, &x);

        let result = run_blunt_code_pass(py, &or_expr).expect("BluntCodePass failed");

        // Should simplify to True
        assert!(
            is_value(py, &result, true),
            "True or x should simplify to True"
        );
    });
}

#[test]
fn test_false_or_x_simplification() {
    // False or x → x
    Python::initialize();
    Python::attach(|py| {
        let false_lit = literal(py, false);
        let x = literal(py, 42);
        let or_expr = or_op(py, &false_lit, &x);

        let result = run_blunt_code_pass(py, &or_expr).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "False or x should simplify to x");
    });
}

#[test]
fn test_x_and_true_simplification() {
    // x and True → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let true_lit = literal(py, true);
        let and_expr = and_op(py, &x, &true_lit);

        let result = run_blunt_code_pass(py, &and_expr).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "x and True should simplify to x");
    });
}

#[test]
fn test_x_and_false_simplification() {
    // x and False → False
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let false_lit = literal(py, false);
        let and_expr = and_op(py, &x, &false_lit);

        let result = run_blunt_code_pass(py, &and_expr).expect("BluntCodePass failed");

        // Should simplify to False
        assert!(
            is_value(py, &result, false),
            "x and False should simplify to False"
        );
    });
}

#[test]
fn test_x_or_true_simplification() {
    // x or True → True
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let true_lit = literal(py, true);
        let or_expr = or_op(py, &x, &true_lit);

        let result = run_blunt_code_pass(py, &or_expr).expect("BluntCodePass failed");

        // Should simplify to True
        assert!(
            is_value(py, &result, true),
            "x or True should simplify to True"
        );
    });
}

#[test]
fn test_x_or_false_simplification() {
    // x or False → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let false_lit = literal(py, false);
        let or_expr = or_op(py, &x, &false_lit);

        let result = run_blunt_code_pass(py, &or_expr).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "x or False should simplify to x");
    });
}

#[test]
fn test_double_not_with_expression() {
    // not not (x > 3) → x > 3
    Python::initialize();
    Python::attach(|py| {
        // Create: 5 > 3 (which is True)
        let five = literal(py, 5);
        let three = literal(py, 3);
        let gt = create_ir(py, IROpCode::Gt, vec![five, three]);

        let not_gt = not_op(py, &gt);
        let not_not_gt = not_op(py, &not_gt);

        let result = run_blunt_code_pass(py, &not_not_gt).expect("BluntCodePass failed");

        // Should simplify back to the GT operation
        assert!(
            has_opcode(py, &result, IROpCode::Gt),
            "not not (5 > 3) should simplify to (5 > 3)"
        );
    });
}

#[test]
fn test_add_zero_right() {
    // x + 0 → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let zero = literal(py, 0);
        let add = add_op(py, &x, &zero);

        let result = run_blunt_code_pass(py, &add).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "42 + 0 should simplify to 42");
    });
}

#[test]
fn test_add_zero_left() {
    // 0 + x → x
    Python::initialize();
    Python::attach(|py| {
        let zero = literal(py, 0);
        let x = literal(py, 42);
        let add = add_op(py, &zero, &x);

        let result = run_blunt_code_pass(py, &add).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "0 + 42 should simplify to 42");
    });
}

#[test]
fn test_subtract_zero() {
    // x - 0 → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let zero = literal(py, 0);
        let sub = sub_op(py, &x, &zero);

        let result = run_blunt_code_pass(py, &sub).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "42 - 0 should simplify to 42");
    });
}

#[test]
fn test_multiply_one_right() {
    // x * 1 → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let one = literal(py, 1);
        let mul = mul_op(py, &x, &one);

        let result = run_blunt_code_pass(py, &mul).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "42 * 1 should simplify to 42");
    });
}

#[test]
fn test_multiply_one_left() {
    // 1 * x → x
    Python::initialize();
    Python::attach(|py| {
        let one = literal(py, 1);
        let x = literal(py, 42);
        let mul = mul_op(py, &one, &x);

        let result = run_blunt_code_pass(py, &mul).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "1 * 42 should simplify to 42");
    });
}

#[test]
fn test_divide_one() {
    // x / 1 → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let one = literal(py, 1);
        let div = div_op(py, &x, &one);

        let result = run_blunt_code_pass(py, &div).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "42 / 1 should simplify to 42");
    });
}

#[test]
fn test_floordiv_one() {
    // x // 1 → x
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let one = literal(py, 1);
        let floordiv = floordiv_op(py, &x, &one);

        let result = run_blunt_code_pass(py, &floordiv).expect("BluntCodePass failed");

        // Should simplify to x
        assert!(is_value(py, &result, 42), "42 // 1 should simplify to 42");
    });
}

#[test]
fn test_multiply_zero_right() {
    // x * 0 → 0
    Python::initialize();
    Python::attach(|py| {
        let x = literal(py, 42);
        let zero = literal(py, 0);
        let mul = mul_op(py, &x, &zero);

        let result = run_blunt_code_pass(py, &mul).expect("BluntCodePass failed");

        // Should simplify to 0
        assert!(is_value(py, &result, 0), "42 * 0 should simplify to 0");
    });
}

#[test]
fn test_multiply_zero_left() {
    // 0 * x → 0
    Python::initialize();
    Python::attach(|py| {
        let zero = literal(py, 0);
        let x = literal(py, 42);
        let mul = mul_op(py, &zero, &x);

        let result = run_blunt_code_pass(py, &mul).expect("BluntCodePass failed");

        // Should simplify to 0
        assert!(is_value(py, &result, 0), "0 * 42 should simplify to 0");
    });
}

#[test]
fn test_multiply_zero_with_expression() {
    // (10 + 20 + 30) * 0 → 0
    Python::initialize();
    Python::attach(|py| {
        let ten = literal(py, 10);
        let twenty = literal(py, 20);
        let thirty = literal(py, 30);

        // Build (10 + 20) + 30
        let add1 = add_op(py, &ten, &twenty);
        let add2 = add_op(py, &add1, &thirty);

        let zero = literal(py, 0);
        let mul = mul_op(py, &add2, &zero);

        let result = run_blunt_code_pass(py, &mul).expect("BluntCodePass failed");

        // Should simplify to 0
        assert!(
            is_value(py, &result, 0),
            "(10 + 20 + 30) * 0 should simplify to 0"
        );
    });
}

#[test]
fn test_multiple_simplifications() {
    // (5 + 0) * 1 - 0 → 5
    Python::initialize();
    Python::attach(|py| {
        let five = literal(py, 5);
        let zero = literal(py, 0);
        let one = literal(py, 1);

        // Build (5 + 0)
        let add = add_op(py, &five, &zero);
        // Build (5 + 0) * 1
        let mul = mul_op(py, &add, &one);
        // Build ((5 + 0) * 1) - 0
        let sub = sub_op(py, &mul, &zero);

        let result = run_blunt_code_pass(py, &sub).expect("BluntCodePass failed");

        // Should simplify to 5
        assert!(
            is_value(py, &result, 5),
            "(5 + 0) * 1 - 0 should simplify to 5"
        );
    });
}

#[test]
fn test_nested_simplifications() {
    // not not (True and (5 > 3)) → True
    Python::initialize();
    Python::attach(|py| {
        // Build 5 > 3
        let five = literal(py, 5);
        let three = literal(py, 3);
        let gt = create_ir(py, IROpCode::Gt, vec![five, three]);

        // Build True and (5 > 3)
        let true_lit = literal(py, true);
        let and = and_op(py, &true_lit, &gt);

        // Build not not (True and (5 > 3))
        let not1 = not_op(py, &and);
        let not2 = not_op(py, &not1);

        let result = run_blunt_code_pass(py, &not2).expect("BluntCodePass failed");

        // Should simplify - the inner expression evaluates to True,
        // True and True is True, not not True is True
        assert!(
            is_value(py, &result, true) || has_opcode(py, &result, IROpCode::Gt),
            "not not (True and (5 > 3)) should simplify"
        );
    });
}

#[test]
fn test_empty_expression() {
    // None literal - should pass through unchanged
    Python::initialize();
    Python::attach(|py| {
        let none = py.None();

        let result = run_blunt_code_pass(py, &none).expect("BluntCodePass failed");

        // Should remain None
        assert!(result.is_none(py), "None should pass through unchanged");
    });
}

#[test]
fn test_chained_comparisons() {
    // (1 < 2) == True → True
    Python::initialize();
    Python::attach(|py| {
        // Build 1 < 2
        let one = literal(py, 1);
        let two = literal(py, 2);
        let lt = create_ir(py, IROpCode::Lt, vec![one, two]);

        // Build (1 < 2) == True
        let true_lit = literal(py, true);
        let eq = eq_op(py, &lt, &true_lit);

        let result = run_blunt_code_pass(py, &eq).expect("BluntCodePass failed");

        // The comparison (1 < 2) evaluates to True,
        // so True == True simplifies to True
        assert!(
            is_value(py, &result, true) || has_opcode(py, &result, IROpCode::Lt),
            "(1 < 2) == True should simplify"
        );
    });
}

#[test]
fn test_variable_operations_not_simplified() {
    // x + zero where BOTH are variables should NOT be simplified
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let zero = ref_node(py, "zero"); // Variable, not literal
        let add = add_op(py, &x, &zero);

        let result = run_blunt_code_pass(py, &add).expect("BluntCodePass failed");

        // Should NOT be simplified because both operands are variables
        // The result should still be an ADD operation
        assert!(
            has_opcode(py, &result, IROpCode::Add),
            "x + zero should NOT be simplified when both are variables"
        );
    });
}

#[test]
fn test_variable_multiply_constant_not_folded() {
    // x * one where both are variables should NOT be simplified
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let one = ref_node(py, "one"); // Variable, not literal
        let mul = mul_op(py, &x, &one);

        let result = run_blunt_code_pass(py, &mul).expect("BluntCodePass failed");

        // Should NOT be simplified because both operands are variables
        assert!(
            has_opcode(py, &result, IROpCode::Mul),
            "x * one should NOT be simplified when both are variables"
        );
    });
}

#[test]
fn test_variable_and_variable_not_simplified() {
    // x and y where both are variables should NOT be simplified
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let y = ref_node(py, "y"); // Variable, not literal
        let and = and_op(py, &x, &y);

        let result = run_blunt_code_pass(py, &and).expect("BluntCodePass failed");

        // Should NOT be simplified because both operands are variables
        assert!(
            has_opcode(py, &result, IROpCode::And),
            "x and y should NOT be simplified when both are variables"
        );
    });
}

#[test]
fn test_division_by_zero_not_optimized() {
    // x / 0 must NOT be optimized (would cause runtime error)
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let zero = literal(py, 0);
        let div = div_op(py, &x, &zero);

        let result = run_blunt_code_pass(py, &div).expect("BluntCodePass failed");

        // Should NOT be optimized - division by zero must be preserved
        // so it raises at runtime
        assert!(
            has_opcode(py, &result, IROpCode::TrueDiv),
            "x / 0 should NOT be optimized (must raise at runtime)"
        );
    });
}

// --- NOT inversion on comparisons ---

#[test]
fn test_not_eq_inversion() {
    // not (a == b) → a != b
    Python::initialize();
    Python::attach(|py| {
        let a = ref_node(py, "a");
        let b = ref_node(py, "b");
        let eq = eq_op(py, &a, &b);
        let not_eq = not_op(py, &eq);

        let result = run_blunt_code_pass(py, &not_eq).expect("BluntCodePass failed");
        assert!(
            has_opcode(py, &result, IROpCode::Ne),
            "not (a == b) should simplify to a != b"
        );
    });
}

#[test]
fn test_not_ne_inversion() {
    // not (a != b) → a == b
    Python::initialize();
    Python::attach(|py| {
        let a = ref_node(py, "a");
        let b = ref_node(py, "b");
        let ne = ne_op(py, &a, &b);
        let not_ne = not_op(py, &ne);

        let result = run_blunt_code_pass(py, &not_ne).expect("BluntCodePass failed");
        assert!(
            has_opcode(py, &result, IROpCode::Eq),
            "not (a != b) should simplify to a == b"
        );
    });
}

#[test]
fn test_not_lt_inversion() {
    // not (a < b) → a >= b
    Python::initialize();
    Python::attach(|py| {
        let a = ref_node(py, "a");
        let b = ref_node(py, "b");
        let lt = lt_op(py, &a, &b);
        let not_lt = not_op(py, &lt);

        let result = run_blunt_code_pass(py, &not_lt).expect("BluntCodePass failed");
        assert!(
            has_opcode(py, &result, IROpCode::Ge),
            "not (a < b) should simplify to a >= b"
        );
    });
}

#[test]
fn test_not_gt_inversion() {
    // not (a > b) → a <= b
    Python::initialize();
    Python::attach(|py| {
        let a = ref_node(py, "a");
        let b = ref_node(py, "b");
        let gt = gt_op(py, &a, &b);
        let not_gt = not_op(py, &gt);

        let result = run_blunt_code_pass(py, &not_gt).expect("BluntCodePass failed");
        assert!(
            has_opcode(py, &result, IROpCode::Le),
            "not (a > b) should simplify to a <= b"
        );
    });
}

#[test]
fn test_not_le_inversion() {
    // not (a <= b) → a > b
    Python::initialize();
    Python::attach(|py| {
        let a = ref_node(py, "a");
        let b = ref_node(py, "b");
        let le = le_op(py, &a, &b);
        let not_le = not_op(py, &le);

        let result = run_blunt_code_pass(py, &not_le).expect("BluntCodePass failed");
        assert!(
            has_opcode(py, &result, IROpCode::Gt),
            "not (a <= b) should simplify to a > b"
        );
    });
}

#[test]
fn test_not_ge_inversion() {
    // not (a >= b) → a < b
    Python::initialize();
    Python::attach(|py| {
        let a = ref_node(py, "a");
        let b = ref_node(py, "b");
        let ge = ge_op(py, &a, &b);
        let not_ge = not_op(py, &ge);

        let result = run_blunt_code_pass(py, &not_ge).expect("BluntCodePass failed");
        assert!(
            has_opcode(py, &result, IROpCode::Lt),
            "not (a >= b) should simplify to a < b"
        );
    });
}

// --- Idempotence ---

#[test]
fn test_and_idempotent() {
    // x and x → x
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let and_expr = and_op(py, &x, &x);

        let result = run_blunt_code_pass(py, &and_expr).expect("BluntCodePass failed");
        // Should simplify to just the ref "x"
        let result_bound = result.bind(py);
        let type_name = result_bound
            .get_type()
            .name()
            .expect("get type name")
            .to_string();
        assert_eq!(type_name, "Ref", "x and x should simplify to x (Ref node)");
    });
}

#[test]
fn test_or_idempotent() {
    // x or x → x
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let or_expr = or_op(py, &x, &x);

        let result = run_blunt_code_pass(py, &or_expr).expect("BluntCodePass failed");
        let result_bound = result.bind(py);
        let type_name = result_bound
            .get_type()
            .name()
            .expect("get type name")
            .to_string();
        assert_eq!(type_name, "Ref", "x or x should simplify to x (Ref node)");
    });
}

// --- Complement ---

#[test]
fn test_and_complement() {
    // x and (not x) → False
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let not_x = not_op(py, &x);
        let and_expr = and_op(py, &x, &not_x);

        let result = run_blunt_code_pass(py, &and_expr).expect("BluntCodePass failed");
        assert!(
            is_value(py, &result, false),
            "x and (not x) should simplify to False"
        );
    });
}

#[test]
fn test_or_complement() {
    // x or (not x) → True
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let not_x = not_op(py, &x);
        let or_expr = or_op(py, &x, &not_x);

        let result = run_blunt_code_pass(py, &or_expr).expect("BluntCodePass failed");
        assert!(
            is_value(py, &result, true),
            "x or (not x) should simplify to True"
        );
    });
}

#[test]
fn test_and_complement_reversed() {
    // (not x) and x → False
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let not_x = not_op(py, &x);
        let and_expr = and_op(py, &not_x, &x);

        let result = run_blunt_code_pass(py, &and_expr).expect("BluntCodePass failed");
        assert!(
            is_value(py, &result, false),
            "(not x) and x should simplify to False"
        );
    });
}

#[test]
fn test_or_complement_reversed() {
    // (not x) or x → True
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let not_x = not_op(py, &x);
        let or_expr = or_op(py, &not_x, &x);

        let result = run_blunt_code_pass(py, &or_expr).expect("BluntCodePass failed");
        assert!(
            is_value(py, &result, true),
            "(not x) or x should simplify to True"
        );
    });
}

// 6 remaining tests from test_blunt_code.py are integration tests that require
// the full pipeline (parser + semantic + executor) to test end-to-end behavior
// with control structures. They are covered by the Python test suite:
// - TestConstantConditions (3 tests) - if True/False branch elimination
// - TestComplexCases::test_blunt_code_in_function - simplifications inside lambdas
// - TestComplexCases::test_blunt_code_in_loop - simplifications inside for loops
// - TestBooleanComparisons::test_comparison_in_condition - flag == True in if
