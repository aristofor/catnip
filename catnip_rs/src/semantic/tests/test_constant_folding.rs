// FILE: catnip_rs/src/semantic/tests/test_constant_folding.rs
//! Unit tests for ConstantFolding optimization.
//!
//! Tests that verify the pass doesn't fold what it shouldn't (negative tests),
//! handles edge cases, and works on types outside the Coq model (strings, floats).
//!
//! Pure constant-folding identities (2+3→5, True and False→False, etc.) are
//! formally proved in proof/CatnipConstFoldProof.v and removed from here.

use super::helpers::*;
use crate::ir::IROpCode;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

// Pure constant-folding identities are formally proved in proof/CatnipConstFoldProof.v:
// - Nested folding: cf_add_fold + cf_mul_fold (+ semantic versions *_sem)
// - Boolean folding: cf_and_bool_fold, cf_or_bool_fold, cf_not_fold
// - Bitwise folding: cf_band_fold, cf_bor_fold, cf_bxor_fold, cf_lshift_fold, cf_rshift_fold

// --- Negative tests: must NOT fold ---

#[test]
fn test_no_fold_with_variables() {
    // x + 2 should NOT fold (x is not a constant)
    Python::initialize();
    Python::attach(|py| {
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

        let opcode = get_opcode(py, &result).expect("Failed to get opcode");
        assert_eq!(
            opcode,
            crate::ir::IROpCode::Add as i32,
            "x + 2 should not fold (x is a variable)"
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

        assert!(
            has_opcode(py, &result, IROpCode::Add),
            "x + 3 should NOT be folded when x is a variable"
        );
    });
}

#[test]
fn test_does_not_fold_function_calls() {
    // Call(lambda, args) should not be folded even if args are constants
    Python::initialize();
    Python::attach(|py| {
        let five = literal(py, 5);
        let f = ref_node(py, "f");
        let call = create_ir(py, IROpCode::Call, vec![f, five]);

        let result = run_constant_folding_pass(py, &call).expect("ConstantFolding failed");

        assert!(
            has_opcode(py, &result, IROpCode::Call),
            "Function calls should not be folded"
        );
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

#[test]
fn test_division_by_zero_not_folded() {
    // 1 / 0 must not be folded at compile time (should error at runtime)
    Python::initialize();
    Python::attach(|py| {
        let one = literal(py, 1);
        let zero = literal(py, 0);
        let div = div_op(py, &one, &zero);

        let result = run_constant_folding_pass(py, &div);

        match result {
            Ok(node) => {
                let is_div = has_opcode(py, &node, IROpCode::TrueDiv);
                assert!(
                    is_div,
                    "Division by zero should not be folded at compile time"
                );
            }
            Err(e) => {
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

// --- Partial folding ---

#[test]
fn test_fold_with_mixed_constants_and_variables() {
    // (2 + 3) + x: the constant part (2 + 3) should fold to 5,
    // result should be 5 + x (still an Add with a Ref)
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let x = ref_node(py, "x");

        let const_add = add_op(py, &two, &three);
        let mixed_add = add_op(py, &const_add, &x);

        let result = run_constant_folding_pass(py, &mixed_add).expect("ConstantFolding failed");

        assert!(
            has_opcode(py, &result, IROpCode::Add),
            "(2 + 3) + x should remain an Add"
        );

        let result_bound = result.bind(py);
        let args = result_bound.getattr("args").expect("get args");
        let args_tuple = args.cast::<pyo3::types::PyTuple>().expect("args is tuple");
        let left = args_tuple.get_item(0).expect("get left");
        let left_val: i64 = left.extract().expect("left should be int 5");
        assert_eq!(left_val, 5, "Constant part (2 + 3) should fold to 5");
    });
}

// --- Types outside the Coq Z/Q model ---

#[test]
fn test_string_multiplication() {
    // "hello" * 3 → "hellohellohello"
    Python::initialize();
    Python::attach(|py| {
        let hello = literal(py, "hello");
        let three = literal(py, 3);
        let mul = mul_op(py, &hello, &three);

        let result = run_constant_folding_pass(py, &mul).expect("ConstantFolding failed");

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
fn test_float_arithmetic() {
    // 1.5 + 2.5 → 4.0
    Python::initialize();
    Python::attach(|py| {
        let one_point_five = literal(py, 1.5);
        let two_point_five = literal(py, 2.5);
        let add = add_op(py, &one_point_five, &two_point_five);

        let result = run_constant_folding_pass(py, &add).expect("ConstantFolding failed");

        let val = result.bind(py).extract::<f64>().expect("Should be float");
        assert!(
            (val - 4.0).abs() < 1e-10,
            "1.5 + 2.5 should fold to 4.0, got {}",
            val
        );
    });
}

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

// Combined strength reduction + folding tests are integration tests
// covered by tests/optimization/test_constant_folding.py::TestConstantFoldingWithStrengthReduction
