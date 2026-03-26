// FILE: catnip_rs/src/semantic/tests/test_constant_propagation.rs
//! Tests for ConstantPropagation optimization.
//!
//! These tests focus on state invalidation across reassignments.

use super::helpers::*;
use crate::ir::IROpCode;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

fn run_constant_propagation_pass(py: Python, ir: &Py<PyAny>) -> PyResult<Py<PyAny>> {
    use crate::semantic::constant_propagation::ConstantPropagationPass;
    use crate::semantic::optimizer::OptimizationPass;

    let pass = ConstantPropagationPass::new();
    pass.visit(py, ir.bind(py))
}

fn set_local_node(py: Python, name: &str, value: Py<PyAny>) -> Py<PyAny> {
    create_ir(py, IROpCode::SetLocals, vec![literal(py, name), value])
}

#[test]
fn test_reassignment_to_non_constant_invalidates_mapping() {
    Python::initialize();
    Python::attach(|py| {
        let assign_const = set_local_node(py, "x", literal(py, 1));
        let assign_non_const = set_local_node(py, "x", ref_node(py, "y"));
        let expr = add_op(py, &ref_node(py, "x"), &literal(py, 2));
        let program: Py<PyAny> = PyList::new(py, [assign_const, assign_non_const, expr])
            .expect("list")
            .unbind()
            .into_any();

        let optimized = run_constant_propagation_pass(py, &program).expect("ConstantPropagation failed");
        let optimized_list = optimized
            .bind(py)
            .cast::<PyList>()
            .expect("optimized program should be a list");
        let optimized_expr = optimized_list.get_item(2).expect("expression at index 2");

        assert!(
            has_opcode(py, &optimized_expr.clone().unbind(), IROpCode::Add),
            "x + 2 should remain an Add after x is reassigned to a non-constant"
        );

        let args = optimized_expr.getattr("args").expect("get args");
        let args_tuple = args.cast::<PyTuple>().expect("args should be a tuple");
        let left = args_tuple.get_item(0).expect("left operand");
        let left_type_name = left.get_type().name().expect("left type");
        let left_type = left_type_name.to_str().expect("left type str");
        assert_eq!(left_type, "Ref", "left operand should remain a Ref(x), not literal 1");
        let left_ident: String = left
            .getattr("ident")
            .expect("left ident")
            .extract()
            .expect("extract ident");
        assert_eq!(left_ident, "x");
    });
}
