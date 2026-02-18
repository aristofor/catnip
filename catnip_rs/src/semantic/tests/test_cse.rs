// FILE: catnip_rs/src/semantic/tests/test_cse.rs
//! Tests for Common Subexpression Elimination (CSE) pass.
//!
//! CSE detects repeated subexpressions and extracts them into temporaries
//! to avoid recomputation.

use super::helpers::*;
use crate::ir::IROpCode;
use crate::semantic::opcode::OpCode;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

/// Helper to create a block IR node
fn block_node(py: Python, stmts: Vec<Py<PyAny>>) -> Py<PyAny> {
    create_ir(py, IROpCode::OpBlock, stmts)
}

/// Helper to run CSE pass on an IR node
fn run_cse_pass(py: Python, ir: &Py<PyAny>) -> PyResult<Py<PyAny>> {
    // Instantiate via Python to access the constructor
    let cse_module = py.import("catnip._rs")?;
    let cse_class = cse_module.getattr("CommonSubexpressionEliminationPass")?;
    let pass_instance = cse_class.call0()?;
    let ir_bound = ir.bind(py);
    let result = pass_instance.call_method1("visit", (ir_bound,))?;
    Ok(result.unbind())
}

/// Helper to create a SET_LOCALS node (assignment)
fn set_locals_node(py: Python, names: Vec<&str>, values: Vec<Py<PyAny>>) -> Py<PyAny> {
    let names_tuple = PyTuple::new(py, names).unwrap().unbind();
    let values_args: Vec<Py<PyAny>> = values.into_iter().collect();
    create_ir(
        py,
        IROpCode::SetLocals,
        vec![names_tuple.into(), create_list(py, values_args)],
    )
}

/// Helper to create a Python list from Py<PyAny> items
fn create_list(py: Python, items: Vec<Py<PyAny>>) -> Py<PyAny> {
    py.import("builtins")
        .unwrap()
        .getattr("list")
        .unwrap()
        .call1((items,))
        .unwrap()
        .unbind()
}

/// Helper to count CSE variables in a block
fn count_cse_vars(py: Python, block: &Py<PyAny>) -> PyResult<usize> {
    let block_bound = block.bind(py);
    let args = block_bound.getattr("args")?;
    let args_tuple = args.cast::<PyTuple>()?;

    let mut count = 0;
    for stmt in args_tuple.iter() {
        let stmt_type = stmt.get_type();
        let type_name_obj = stmt_type.name()?;
        let type_name = type_name_obj.to_str()?;

        if type_name == "IR" {
            let ident: i32 = stmt.getattr("ident")?.extract()?;
            if ident == OpCode::SET_LOCALS as i32 {
                let stmt_args = stmt.getattr("args")?;
                let stmt_args_tuple = stmt_args.cast::<PyTuple>()?;
                if stmt_args_tuple.len() >= 1 {
                    let names = stmt_args_tuple.get_item(0)?;
                    if let Ok(names_tuple) = names.cast::<PyTuple>() {
                        for name in names_tuple.iter() {
                            if let Ok(name_str) = name.extract::<String>() {
                                if name_str.starts_with("_cse_") {
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(count)
}

#[test]
fn test_cse_basic_duplicate() {
    // Test: b * 2 repeated twice
    // Block: result = a + b * 2 + b * 2
    // Expected: CSE extracts b * 2 into _cse_0
    Python::initialize();
    Python::attach(|py| {
        let b = ref_node(py, "b");
        let two = literal(py, 2);
        let a = ref_node(py, "a");

        // Create b * 2 (repeated twice)
        let b_times_2_1 = mul_op(py, &b, &two);
        let b_times_2_2 = mul_op(py, &b, &two);

        // Create a + b * 2 + b * 2
        let add1 = add_op(py, &a, &b_times_2_1);
        let add2 = add_op(py, &add1, &b_times_2_2);

        // Create block with the expression
        let result_assign = set_locals_node(py, vec!["result"], vec![add2]);
        let block = block_node(py, vec![result_assign]);

        // Run CSE pass
        let optimized = run_cse_pass(py, &block).expect("CSE failed");

        // Check that CSE variables were created
        let cse_count = count_cse_vars(py, &optimized).expect("Failed to count CSE vars");
        assert!(
            cse_count >= 1,
            "CSE should create at least one temporary for b * 2"
        );
    });
}

#[test]
fn test_cse_multiple_duplicates() {
    // Test: Multiple repeated subexpressions
    // r1 = x * y + x * y
    // r2 = x + y + x + y
    Python::initialize();
    Python::attach(|py| {
        let x = ref_node(py, "x");
        let y = ref_node(py, "y");

        // x * y repeated twice
        let x_times_y_1 = mul_op(py, &x, &y);
        let x_times_y_2 = mul_op(py, &x, &y);
        let r1 = add_op(py, &x_times_y_1, &x_times_y_2);

        // x + y repeated twice
        let x_plus_y_1 = add_op(py, &x, &y);
        let x_plus_y_2 = add_op(py, &x, &y);
        let r2 = add_op(py, &x_plus_y_1, &x_plus_y_2);

        // Create block
        let r1_assign = set_locals_node(py, vec!["r1"], vec![r1]);
        let r2_assign = set_locals_node(py, vec!["r2"], vec![r2]);
        let block = block_node(py, vec![r1_assign, r2_assign]);

        // Run CSE pass
        let optimized = run_cse_pass(py, &block).expect("CSE failed");

        // Check that CSE variables were created (at least one)
        let cse_count = count_cse_vars(py, &optimized).expect("Failed to count CSE vars");
        assert!(
            cse_count >= 1,
            "CSE should create temporary variables for repeated expressions"
        );
    });
}

#[test]
fn test_cse_no_duplicate() {
    // Test: No duplicates, CSE should not modify
    // result = a + b
    Python::initialize();
    Python::attach(|py| {
        let a = ref_node(py, "a");
        let b = ref_node(py, "b");
        let add = add_op(py, &a, &b);

        let result_assign = set_locals_node(py, vec!["result"], vec![add]);
        let block = block_node(py, vec![result_assign]);

        // Run CSE pass
        let optimized = run_cse_pass(py, &block).expect("CSE failed");

        // Check that NO CSE variables were created
        let cse_count = count_cse_vars(py, &optimized).expect("Failed to count CSE vars");
        assert_eq!(
            cse_count, 0,
            "CSE should not create variables without duplicates"
        );
    });
}

#[test]
fn test_cse_complex_expression() {
    // Test: (a + b) * 2 repeated
    // x = (a + b) * 2 + 10
    // y = (a + b) * 2 + 20
    Python::initialize();
    Python::attach(|py| {
        let a = ref_node(py, "a");
        let b = ref_node(py, "b");
        let two = literal(py, 2);
        let ten = literal(py, 10);
        let twenty = literal(py, 20);

        // (a + b) * 2 repeated twice
        let a_plus_b_1 = add_op(py, &a, &b);
        let complex_1 = mul_op(py, &a_plus_b_1, &two);
        let x = add_op(py, &complex_1, &ten);

        let a_plus_b_2 = add_op(py, &a, &b);
        let complex_2 = mul_op(py, &a_plus_b_2, &two);
        let y = add_op(py, &complex_2, &twenty);

        // Create block
        let x_assign = set_locals_node(py, vec!["x"], vec![x]);
        let y_assign = set_locals_node(py, vec!["y"], vec![y]);
        let block = block_node(py, vec![x_assign, y_assign]);

        // Run CSE pass
        let optimized = run_cse_pass(py, &block).expect("CSE failed");

        // Check that CSE variables were created
        let cse_count = count_cse_vars(py, &optimized).expect("Failed to count CSE vars");
        assert!(
            cse_count >= 1,
            "CSE should create temporaries for complex repeated expressions"
        );
    });
}
