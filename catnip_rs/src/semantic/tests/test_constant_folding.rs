// FILE: catnip_rs/src/semantic/tests/test_constant_folding.rs
//! Unit tests for ConstantFolding optimization.
//!
//! These tests validate that the ConstantFolding pass correctly evaluates
//! constant expressions at compile time.

use super::helpers::*;
use crate::ir::IROpCode;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

#[test]
fn test_fold_addition() {
    // 2 + 3 → 5
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let add = add_op(py, &two, &three);

        let result = run_constant_folding_pass(py, &add).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 5), "2 + 3 should fold to 5");
    });
}

#[test]
fn test_fold_subtraction() {
    // 10 - 5 → 5
    Python::initialize();
    Python::attach(|py| {
        let ten = literal(py, 10);
        let five = literal(py, 5);
        let sub = sub_op(py, &ten, &five);

        let result = run_constant_folding_pass(py, &sub).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 5), "10 - 5 should fold to 5");
    });
}

#[test]
fn test_fold_multiplication() {
    // 4 * 5 → 20
    Python::initialize();
    Python::attach(|py| {
        let four = literal(py, 4);
        let five = literal(py, 5);
        let mul = mul_op(py, &four, &five);

        let result = run_constant_folding_pass(py, &mul).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 20), "4 * 5 should fold to 20");
    });
}

#[test]
fn test_fold_division() {
    // 20 / 4 → 5.0
    Python::initialize();
    Python::attach(|py| {
        let twenty = literal(py, 20);
        let four = literal(py, 4);
        let div = div_op(py, &twenty, &four);

        let result = run_constant_folding_pass(py, &div).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 5.0), "20 / 4 should fold to 5.0");
    });
}

#[test]
fn test_fold_floor_division() {
    // 23 // 5 → 4
    Python::initialize();
    Python::attach(|py| {
        let twenty_three = literal(py, 23);
        let five = literal(py, 5);
        let floordiv = floordiv_op(py, &twenty_three, &five);

        let result = run_constant_folding_pass(py, &floordiv).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 4), "23 // 5 should fold to 4");
    });
}

#[test]
fn test_fold_modulo() {
    // 23 % 5 → 3
    Python::initialize();
    Python::attach(|py| {
        let twenty_three = literal(py, 23);
        let five = literal(py, 5);
        let mod_op_node = mod_op(py, &twenty_three, &five);

        let result = run_constant_folding_pass(py, &mod_op_node).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 3), "23 % 5 should fold to 3");
    });
}

#[test]
fn test_fold_power() {
    // 2 ** 3 → 8
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let pow_node = pow_op(py, &two, &three);

        let result = run_constant_folding_pass(py, &pow_node).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 8), "2 ** 3 should fold to 8");
    });
}

#[test]
fn test_fold_nested_operations() {
    // (2 + 3) * 4 → 5 * 4 → 20
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let four = literal(py, 4);

        let add = add_op(py, &two, &three);
        let mul = mul_op(py, &add, &four);

        let result = run_constant_folding_pass(py, &mul).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 20), "(2 + 3) * 4 should fold to 20");
    });
}

#[test]
fn test_fold_complex_expression() {
    // 10 + 20 * 3 - 5 → 10 + 60 - 5 → 65
    Python::initialize();
    Python::attach(|py| {
        let ten = literal(py, 10);
        let twenty = literal(py, 20);
        let three = literal(py, 3);
        let five = literal(py, 5);

        let mul = mul_op(py, &twenty, &three); // 20 * 3 = 60
        let add = add_op(py, &ten, &mul); // 10 + 60 = 70
        let sub = sub_op(py, &add, &five); // 70 - 5 = 65

        let result = run_constant_folding_pass(py, &sub).expect("ConstantFolding failed");

        assert!(
            is_value(py, &result, 65),
            "10 + 20 * 3 - 5 should fold to 65"
        );
    });
}

#[test]
fn test_no_fold_with_variables() {
    // x + 2 should NOT fold (x is not a constant)
    Python::initialize();
    Python::attach(|py| {
        // Create a Ref node for variable x
        let nodes_mod = py
            .import("catnip.nodes")
            .expect("Failed to import catnip.nodes");
        let ref_class = nodes_mod.getattr("Ref").expect("Failed to get Ref class");
        let x = ref_class
            .call1(("x",))
            .expect("Failed to create Ref")
            .unbind();

        let two = literal(py, 2);
        let add = add_op(py, &x, &two);

        let result = run_constant_folding_pass(py, &add).expect("ConstantFolding failed");

        // Should NOT be folded - should still be an ADD operation
        let opcode = get_opcode(py, &result).expect("Failed to get opcode");
        assert_eq!(
            opcode,
            crate::ir::IROpCode::Add as i32,
            "x + 2 should not fold (x is a variable)"
        );
    });
}

#[test]
fn test_unary_negation() {
    // -42 → -42
    Python::initialize();
    Python::attach(|py| {
        let forty_two = literal(py, 42);
        let neg = neg_op(py, &forty_two);

        let result = run_constant_folding_pass(py, &neg).expect("ConstantFolding failed");

        assert!(is_value(py, &result, -42), "-42 should fold to -42");
    });
}

#[test]
fn test_unary_positive() {
    // +42 → 42
    Python::initialize();
    Python::attach(|py| {
        let forty_two = literal(py, 42);
        let pos = pos_op(py, &forty_two);

        let result = run_constant_folding_pass(py, &pos).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 42), "+42 should fold to 42");
    });
}

#[test]
fn test_comparison_equal() {
    // 5 == 5 → True
    Python::initialize();
    Python::attach(|py| {
        let five1 = literal(py, 5);
        let five2 = literal(py, 5);
        let eq = eq_op(py, &five1, &five2);

        let result = run_constant_folding_pass(py, &eq).expect("ConstantFolding failed");

        assert!(is_value(py, &result, true), "5 == 5 should fold to True");
    });
}

#[test]
fn test_comparison_not_equal() {
    // 5 != 3 → True
    Python::initialize();
    Python::attach(|py| {
        let five = literal(py, 5);
        let three = literal(py, 3);
        let ne = ne_op(py, &five, &three);

        let result = run_constant_folding_pass(py, &ne).expect("ConstantFolding failed");

        assert!(is_value(py, &result, true), "5 != 3 should fold to True");
    });
}

#[test]
fn test_comparison_less_than() {
    // 3 < 5 → True
    Python::initialize();
    Python::attach(|py| {
        let three = literal(py, 3);
        let five = literal(py, 5);
        let lt = lt_op(py, &three, &five);

        let result = run_constant_folding_pass(py, &lt).expect("ConstantFolding failed");

        assert!(is_value(py, &result, true), "3 < 5 should fold to True");
    });
}

#[test]
fn test_comparison_less_equal() {
    // 5 <= 5 → True
    Python::initialize();
    Python::attach(|py| {
        let five1 = literal(py, 5);
        let five2 = literal(py, 5);
        let le = le_op(py, &five1, &five2);

        let result = run_constant_folding_pass(py, &le).expect("ConstantFolding failed");

        assert!(is_value(py, &result, true), "5 <= 5 should fold to True");
    });
}

#[test]
fn test_comparison_greater_than() {
    // 5 > 3 → True
    Python::initialize();
    Python::attach(|py| {
        let five = literal(py, 5);
        let three = literal(py, 3);
        let gt = gt_op(py, &five, &three);

        let result = run_constant_folding_pass(py, &gt).expect("ConstantFolding failed");

        assert!(is_value(py, &result, true), "5 > 3 should fold to True");
    });
}

#[test]
fn test_comparison_greater_equal() {
    // 5 >= 5 → True
    Python::initialize();
    Python::attach(|py| {
        let five1 = literal(py, 5);
        let five2 = literal(py, 5);
        let ge = ge_op(py, &five1, &five2);

        let result = run_constant_folding_pass(py, &ge).expect("ConstantFolding failed");

        assert!(is_value(py, &result, true), "5 >= 5 should fold to True");
    });
}

#[test]
fn test_logical_and_true() {
    // True and True → True
    Python::initialize();
    Python::attach(|py| {
        let true1 = literal(py, true);
        let true2 = literal(py, true);
        let and = and_op(py, &true1, &true2);

        let result = run_constant_folding_pass(py, &and).expect("ConstantFolding failed");

        assert!(
            is_value(py, &result, true),
            "True and True should fold to True"
        );
    });
}

#[test]
fn test_logical_and_false() {
    // True and False → False
    Python::initialize();
    Python::attach(|py| {
        let true_lit = literal(py, true);
        let false_lit = literal(py, false);
        let and = and_op(py, &true_lit, &false_lit);

        let result = run_constant_folding_pass(py, &and).expect("ConstantFolding failed");

        assert!(
            is_value(py, &result, false),
            "True and False should fold to False"
        );
    });
}

#[test]
fn test_logical_or_true() {
    // False or True → True
    Python::initialize();
    Python::attach(|py| {
        let false_lit = literal(py, false);
        let true_lit = literal(py, true);
        let or = or_op(py, &false_lit, &true_lit);

        let result = run_constant_folding_pass(py, &or).expect("ConstantFolding failed");

        assert!(
            is_value(py, &result, true),
            "False or True should fold to True"
        );
    });
}

#[test]
fn test_logical_or_false() {
    // False or False → False
    Python::initialize();
    Python::attach(|py| {
        let false1 = literal(py, false);
        let false2 = literal(py, false);
        let or = or_op(py, &false1, &false2);

        let result = run_constant_folding_pass(py, &or).expect("ConstantFolding failed");

        assert!(
            is_value(py, &result, false),
            "False or False should fold to False"
        );
    });
}

#[test]
fn test_logical_not() {
    // not False → True
    Python::initialize();
    Python::attach(|py| {
        let false_lit = literal(py, false);
        let not = not_op(py, &false_lit);

        let result = run_constant_folding_pass(py, &not).expect("ConstantFolding failed");

        assert!(is_value(py, &result, true), "not False should fold to True");
    });
}

#[test]
fn test_bitwise_and() {
    // 12 & 10 → 8 (1100 & 1010 = 1000)
    Python::initialize();
    Python::attach(|py| {
        let twelve = literal(py, 12);
        let ten = literal(py, 10);
        let bitand = bitand_op(py, &twelve, &ten);

        let result = run_constant_folding_pass(py, &bitand).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 8), "12 & 10 should fold to 8");
    });
}

#[test]
fn test_bitwise_or() {
    // 12 | 10 → 14 (1100 | 1010 = 1110)
    Python::initialize();
    Python::attach(|py| {
        let twelve = literal(py, 12);
        let ten = literal(py, 10);
        let bitor = bitor_op(py, &twelve, &ten);

        let result = run_constant_folding_pass(py, &bitor).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 14), "12 | 10 should fold to 14");
    });
}

#[test]
fn test_bitwise_xor() {
    // 12 ^ 10 → 6 (1100 ^ 1010 = 0110)
    Python::initialize();
    Python::attach(|py| {
        let twelve = literal(py, 12);
        let ten = literal(py, 10);
        let bitxor = bitxor_op(py, &twelve, &ten);

        let result = run_constant_folding_pass(py, &bitxor).expect("ConstantFolding failed");

        assert!(is_value(py, &result, 6), "12 ^ 10 should fold to 6");
    });
}

#[test]
fn test_bitwise_not() {
    // ~5 → -6
    Python::initialize();
    Python::attach(|py| {
        let five = literal(py, 5);
        let bitnot = bitnot_op(py, &five);

        let result = run_constant_folding_pass(py, &bitnot).expect("ConstantFolding failed");

        assert!(is_value(py, &result, -6), "~5 should fold to -6");
    });
}

#[test]
fn test_string_multiplication() {
    // "hello" * 3 → "hellohellohello"
    Python::initialize();
    Python::attach(|py| {
        let hello = literal(py, "hello");
        let three = literal(py, 3);
        let mul = mul_op(py, &hello, &three);

        let result = run_constant_folding_pass(py, &mul).expect("ConstantFolding failed");

        // Extract as String and compare
        let result_str = result
            .bind(py)
            .extract::<String>()
            .expect("Should be string");
        assert_eq!(
            result_str, "hellohellohello",
            "\"hello\" * 3 should fold to \"hellohellohello\""
        );
    });
}

#[test]
fn test_nested_arithmetic() {
    // (2 + 3) * (4 + 1) → 25
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let four = literal(py, 4);
        let one = literal(py, 1);

        // Build (2 + 3)
        let add1 = add_op(py, &two, &three);
        // Build (4 + 1)
        let add2 = add_op(py, &four, &one);
        // Build (2 + 3) * (4 + 1)
        let mul = mul_op(py, &add1, &add2);

        let result = run_constant_folding_pass(py, &mul).expect("ConstantFolding failed");

        assert!(
            is_value(py, &result, 25),
            "(2 + 3) * (4 + 1) should fold to 25"
        );
    });
}

#[test]
fn test_complex_expression() {
    // 2 ** 3 + 4 * 5 - 10 / 2 → 23.0
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let four = literal(py, 4);
        let five = literal(py, 5);
        let ten = literal(py, 10);

        // Build 2 ** 3
        let pow = pow_op(py, &two, &three);
        // Build 4 * 5
        let mul = mul_op(py, &four, &five);
        // Build 10 / 2
        let div = div_op(py, &ten, &two);
        // Build (2 ** 3) + (4 * 5)
        let add = add_op(py, &pow, &mul);
        // Build ((2 ** 3) + (4 * 5)) - (10 / 2)
        let sub = sub_op(py, &add, &div);

        let result = run_constant_folding_pass(py, &sub).expect("ConstantFolding failed");

        assert!(
            is_value(py, &result, 23.0),
            "2 ** 3 + 4 * 5 - 10 / 2 should fold to 23.0"
        );
    });
}

#[test]
fn test_does_not_fold_variables() {
    // x + 3 where x is a variable should NOT be folded
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let three = literal(py, 3);
        let add = add_op(py, &x, &three);

        let result = run_constant_folding_pass(py, &add).expect("ConstantFolding failed");

        // Should NOT be folded because x is a variable
        // The result should still be an ADD operation
        assert!(
            has_opcode(py, &result, IROpCode::Add),
            "x + 3 should NOT be folded when x is a variable"
        );
    });
}

/// Edge cases for constant folding

#[test]
fn test_very_large_numbers() {
    // 999999999999999999 + 1 → 1000000000000000000
    Python::initialize();
    Python::attach(|py| {
        let large = literal(py, 999999999999999999_i64);
        let one = literal(py, 1);
        let add = add_op(py, &large, &one);

        let result = run_constant_folding_pass(py, &add).expect("ConstantFolding failed");

        assert!(
            is_value(py, &result, 1000000000000000000_i64),
            "Very large numbers should be folded correctly"
        );
    });
}

#[test]
fn test_negative_numbers() {
    // -5 + (-3) → -8
    Python::initialize();
    Python::attach(|py| {
        let neg_five = neg_op(py, &literal(py, 5));
        let neg_three = neg_op(py, &literal(py, 3));
        let add = add_op(py, &neg_five, &neg_three);

        let result = run_constant_folding_pass(py, &add).expect("ConstantFolding failed");

        assert!(is_value(py, &result, -8), "-5 + (-3) should fold to -8");
    });
}

#[test]
fn test_float_arithmetic() {
    // 1.5 + 2.5 → 4.0
    Python::initialize();
    Python::attach(|py| {
        let one_point_five = literal(py, 1.5);
        let two_point_five = literal(py, 2.5);
        let add = add_op(py, &one_point_five, &two_point_five);

        let result = run_constant_folding_pass(py, &add).expect("ConstantFolding failed");

        // Check if close to 4.0 (floating point comparison)
        let val = result.bind(py).extract::<f64>().expect("Should be float");
        assert!(
            (val - 4.0).abs() < 1e-10,
            "1.5 + 2.5 should fold to 4.0, got {}",
            val
        );
    });
}

#[test]
fn test_division_by_zero_not_folded() {
    // 1 / 0 must not be folded at compile time (should error at runtime)
    Python::initialize();
    Python::attach(|py| {
        let one = literal(py, 1);
        let zero = literal(py, 0);
        let div = div_op(py, &one, &zero);

        // Attempt to fold should either:
        // 1. Return the original IR node (not folded), OR
        // 2. Raise ZeroDivisionError immediately
        let result = run_constant_folding_pass(py, &div);

        // If folding succeeds (returns node), it should be the original node or None
        // If it fails with ZeroDivisionError, that's also acceptable
        match result {
            Ok(node) => {
                // Check if it's still a DIV operation (not folded)
                let is_div = has_opcode(py, &node, IROpCode::TrueDiv);
                assert!(
                    is_div,
                    "Division by zero should not be folded at compile time"
                );
            }
            Err(e) => {
                // If it raised an error, verify it's ZeroDivisionError
                let err_str = e.to_string();
                assert!(
                    err_str.contains("ZeroDivisionError") || err_str.contains("division by zero"),
                    "Should raise ZeroDivisionError, got: {}",
                    err_str
                );
            }
        }
    });
}

#[test]
fn test_does_not_fold_function_calls() {
    // Call(lambda, args) should not be folded even if args are constants
    Python::initialize();
    Python::attach(|py| {
        let five = literal(py, 5);
        // Create a call node: Call(ref("f"), 5)
        let f = ref_node(py, "f");
        let call = create_ir(py, IROpCode::Call, vec![f, five]);

        let result = run_constant_folding_pass(py, &call).expect("ConstantFolding failed");

        // Should NOT be folded because it's a function call
        assert!(
            has_opcode(py, &result, IROpCode::Call),
            "Function calls should not be folded"
        );
    });
}

#[test]
fn test_fold_with_mixed_constants_and_variables() {
    // (2 + 3) + x: the constant part (2 + 3) should fold to 5,
    // result should be 5 + x (still an Add with a Ref)
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let x = ref_node(py, "x");

        // Build (2 + 3) + x
        let const_add = add_op(py, &two, &three);
        let mixed_add = add_op(py, &const_add, &x);

        let result = run_constant_folding_pass(py, &mixed_add).expect("ConstantFolding failed");

        // Should still be an ADD (not fully folded because x is a variable)
        assert!(
            has_opcode(py, &result, IROpCode::Add),
            "(2 + 3) + x should remain an Add"
        );

        // The left operand should have been folded to 5
        let result_bound = result.bind(py);
        let args = result_bound.getattr("args").expect("get args");
        let args_tuple = args.cast::<pyo3::types::PyTuple>().expect("args is tuple");
        let left = args_tuple.get_item(0).expect("get left");
        let left_val: i64 = left.extract().expect("left should be int 5");
        assert_eq!(left_val, 5, "Constant part (2 + 3) should fold to 5");
    });
}

#[test]
fn test_eq_with_call_object_not_folded() {
    // EQ(Call(len, Ref(x)), 0) must NOT be folded — Call is not a constant
    Python::initialize();
    Python::attach(|py| {
        let call_class = py
            .import("catnip.transformer")
            .expect("Failed to import")
            .getattr("Call")
            .expect("Failed to get Call");
        let len_ref = ref_node(py, "len");
        let x_ref = ref_node(py, "x");
        let args_tuple = PyTuple::new(py, &[x_ref]).expect("tuple").unbind();
        let kwargs = pyo3::types::PyDict::new(py).unbind();
        let call_obj = call_class
            .call1((&len_ref, &args_tuple, &kwargs))
            .expect("Failed to create Call")
            .unbind();

        let zero = literal(py, 0);
        let eq = eq_op(py, &call_obj, &zero);

        let result = run_constant_folding_pass(py, &eq).expect("ConstantFolding failed");

        assert!(
            has_opcode(py, &result, IROpCode::Eq),
            "EQ(Call(...), 0) should NOT be folded"
        );
    });
}

// Combined strength reduction + folding tests are integration tests
// covered by tests/optimization/test_constant_folding.py::TestConstantFoldingWithStrengthReduction
