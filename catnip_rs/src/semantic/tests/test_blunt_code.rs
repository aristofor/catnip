// FILE: catnip_rs/src/semantic/tests/test_blunt_code.rs
//! Unit tests for BluntCodePass optimization.
//!
//! These tests validate that the BluntCodePass correctly simplifies
//! inefficient or redundant patterns in the IR.

use super::helpers::*;
use crate::ir::IROpCode;
use pyo3::prelude::*;

// Prudent: not not not x → not x (no explicit Coq theorem for triple negation)
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

// Prudent: not not expr where expr is non-literal (blunt_double_neg covers literals only)
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

// Removed: test_multiply_zero_with_expression - proven by sr_mul_zero_r (CatnipOptimProof.v:187)
// Removed: test_multiple_simplifications - proven by sr_add_zero_r + sr_mul_one_r + sr_sub_zero + compose_preserves_eval
// Removed: test_nested_simplifications - proven by sr_and_true_l + blunt_double_neg + compose_preserves_eval

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

// Removed: test_chained_comparisons - proven by cf_lt_fold + blunt_eq_true_r (CatnipOptimProof.v:494)

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

// 6 remaining tests from test_blunt_code.py are integration tests that require
// the full pipeline (parser + semantic + executor) to test end-to-end behavior
// with control structures. They are covered by the Python test suite:
// - TestConstantConditions (3 tests) - if True/False branch elimination
// - TestComplexCases::test_blunt_code_in_function - simplifications inside lambdas
// - TestComplexCases::test_blunt_code_in_loop - simplifications inside for loops
// - TestBooleanComparisons::test_comparison_in_condition - flag == True in if
