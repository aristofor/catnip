// FILE: catnip_rs/src/semantic/tests/test_optimizer.rs
//! Unit tests for the overall Optimizer pipeline.
//!
//! Tests validate that the Optimizer correctly creates default passes,
//! accepts custom passes, respects max_iterations, and handles various
//! IR node types (literals, lists, tuples, IR nodes).
//!
//! Integration tests (strength reduction, dead code elimination, block
//! flattening, chained comparisons, optimization levels) are covered by
//! the Python test suite: tests/optimization/test_optimizer.py

use super::helpers::*;
use pyo3::prelude::*;
use pyo3::types::PyList;

#[test]
fn test_optimizer_creates_with_default_passes() {
    // Optimizer() should create 10 default passes
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let passes = optimizer.getattr("passes").expect("get passes");
        let passes_list = passes.cast::<PyList>().expect("passes is a list");

        assert_eq!(
            passes_list.len(),
            10,
            "Default optimizer should have 10 passes"
        );
    });
}

#[test]
fn test_optimizer_with_custom_passes() {
    // Optimizer(passes=[ConstantFoldingPass()]) should have exactly 1 pass
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let cf_pass = rs
            .getattr("ConstantFoldingPass")
            .expect("get ConstantFoldingPass")
            .call0()
            .expect("create pass");

        let passes = PyList::new(py, &[cf_pass]).expect("create list");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call1((passes,))
            .expect("create Optimizer with custom passes");

        let result_passes = optimizer.getattr("passes").expect("get passes");
        let result_list = result_passes.cast::<PyList>().expect("passes is a list");

        assert_eq!(
            result_list.len(),
            1,
            "Custom optimizer should have exactly 1 pass"
        );
    });
}

#[test]
fn test_optimizer_optimize_literal_int() {
    // Optimizer.optimize(42) should return 42 unchanged
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let result = optimizer.call_method1("optimize", (42,)).expect("optimize");
        let val: i64 = result.extract().expect("extract int");
        assert_eq!(val, 42);
    });
}

#[test]
fn test_optimizer_optimize_literal_string() {
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let result = optimizer
            .call_method1("optimize", ("hello",))
            .expect("optimize");
        let val: String = result.extract().expect("extract str");
        assert_eq!(val, "hello");
    });
}

#[test]
fn test_optimizer_optimize_literal_none() {
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let result = optimizer
            .call_method1("optimize", (py.None(),))
            .expect("optimize");
        assert!(result.is_none(), "None should pass through unchanged");
    });
}

#[test]
fn test_optimizer_optimize_list() {
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let input = vec![1, 2, 3];
        let result = optimizer
            .call_method1("optimize", (input.clone(),))
            .expect("optimize");
        let val: Vec<i64> = result.extract().expect("extract list");
        assert_eq!(val, vec![1, 2, 3]);
    });
}

#[test]
fn test_optimizer_optimize_tuple() {
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let input = (1, 2, 3);
        let result = optimizer
            .call_method1("optimize", (input,))
            .expect("optimize");
        let val: (i64, i64, i64) = result.extract().expect("extract tuple");
        assert_eq!(val, (1, 2, 3));
    });
}

#[test]
fn test_optimizer_max_iterations() {
    // optimize(42, max_iterations=1) should not crash
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("max_iterations", 1).unwrap();
        let result = optimizer
            .call_method("optimize", (42,), Some(&kwargs))
            .expect("optimize with max_iterations=1");
        let val: i64 = result.extract().expect("extract int");
        assert_eq!(val, 42);
    });
}

#[test]
fn test_optimizer_folds_constant_addition() {
    // Full pipeline: Optimizer.optimize(IR(Add, 2, 3)) should fold to 5
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let add = add_op(py, &two, &three);

        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let result = optimizer
            .call_method1("optimize", (add,))
            .expect("optimize");
        assert!(is_value(py, &result.unbind(), 5), "2 + 3 should fold to 5");
    });
}

#[test]
fn test_optimizer_simplifies_blunt_code() {
    // Full pipeline: not not True should simplify to True
    Python::initialize();
    Python::attach(|py| {
        let t = literal(py, true);
        let not1 = not_op(py, &t);
        let not2 = not_op(py, &not1);

        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let result = optimizer
            .call_method1("optimize", (not2,))
            .expect("optimize");
        assert!(
            is_value(py, &result.unbind(), true),
            "not not True should optimize to True"
        );
    });
}

#[test]
fn test_optimizer_composes_passes() {
    // BluntCode (x + 0 → x) + ConstantFolding should compose:
    // (2 + 3) + 0 → 5 + 0 → 5
    Python::initialize();
    Python::attach(|py| {
        let two = literal(py, 2);
        let three = literal(py, 3);
        let zero = literal(py, 0);

        let add_inner = add_op(py, &two, &three);
        let add_outer = add_op(py, &add_inner, &zero);

        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let result = optimizer
            .call_method1("optimize", (add_outer,))
            .expect("optimize");
        assert!(
            is_value(py, &result.unbind(), 5),
            "(2 + 3) + 0 should optimize to 5"
        );
    });
}

#[test]
fn test_optimizer_repr() {
    Python::initialize();
    Python::attach(|py| {
        let rs = py.import("catnip._rs").expect("import catnip._rs");
        let optimizer = rs
            .getattr("Optimizer")
            .expect("get Optimizer")
            .call0()
            .expect("create Optimizer");

        let repr: String = optimizer
            .call_method0("__repr__")
            .expect("repr")
            .extract()
            .expect("extract str");
        assert!(
            repr.contains("10 passes"),
            "repr should mention 10 passes, got: {}",
            repr
        );
    });
}

// Remaining ~40 optimizer tests are integration tests requiring the full
// pipeline (parser + semantic + executor). They test:
// - Strength reduction (x*1→x, x**0→1, etc.) within executed programs
// - Dead code elimination (if True/False branch removal)
// - Block flattening (nested blocks)
// - Pipeline composition (multiple passes enabling each other)
// - Optimization level skipping (level 0 vs 2 vs 3)
// - Chained comparisons
// See tests/optimization/test_optimizer.py for complete coverage.
