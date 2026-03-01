// FILE: catnip_rs/src/semantic/tests/helpers.rs
//! Test helpers for creating IR nodes and running semantic passes.

use crate::ir::IROpCode;
use pyo3::conversion::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

/// Create a literal value (just a Python object, NOT an IR node)
/// In Catnip, literals are direct Python values, not wrapped in IR.
#[allow(deprecated)]
pub fn literal<'py, T>(py: Python<'py>, value: T) -> Py<PyAny>
where
    T: IntoPyObjectExt<'py>,
{
    value.into_py_any(py).unwrap()
}

/// Create a NOT operation IR node
pub fn not_op(py: Python, operand: &Py<PyAny>) -> Py<PyAny> {
    create_ir(py, IROpCode::Not, vec![operand.clone_ref(py)])
}

/// Create an ADD operation IR node
pub fn add_op(py: Python, left: &Py<PyAny>, right: &Py<PyAny>) -> Py<PyAny> {
    create_ir(
        py,
        IROpCode::Add,
        vec![left.clone_ref(py), right.clone_ref(py)],
    )
}

/// Create a MUL operation IR node
pub fn mul_op(py: Python, left: &Py<PyAny>, right: &Py<PyAny>) -> Py<PyAny> {
    create_ir(
        py,
        IROpCode::Mul,
        vec![left.clone_ref(py), right.clone_ref(py)],
    )
}

/// Create a TRUEDIV operation IR node
pub fn div_op(py: Python, left: &Py<PyAny>, right: &Py<PyAny>) -> Py<PyAny> {
    create_ir(
        py,
        IROpCode::TrueDiv,
        vec![left.clone_ref(py), right.clone_ref(py)],
    )
}

/// Create an EQ (equals) operation IR node
pub fn eq_op(py: Python, left: &Py<PyAny>, right: &Py<PyAny>) -> Py<PyAny> {
    create_ir(
        py,
        IROpCode::Eq,
        vec![left.clone_ref(py), right.clone_ref(py)],
    )
}

/// Create an AND operation IR node
pub fn and_op(py: Python, left: &Py<PyAny>, right: &Py<PyAny>) -> Py<PyAny> {
    create_ir(
        py,
        IROpCode::And,
        vec![left.clone_ref(py), right.clone_ref(py)],
    )
}

/// Create a generic IR node with given opcode and args
pub fn create_ir(py: Python, opcode: IROpCode, args: Vec<Py<PyAny>>) -> Py<PyAny> {
    let ir_class = py
        .import("catnip.transformer")
        .expect("Failed to import catnip.transformer")
        .getattr("IR")
        .expect("Failed to get IR class");

    let args_tuple = PyTuple::new(py, &args)
        .expect("Failed to create tuple")
        .unbind();
    let kwargs = PyDict::new(py).unbind();

    ir_class
        .call((opcode as i32, args_tuple, kwargs), None)
        .expect("Failed to create IR node")
        .unbind()
}

/// Run the BluntCodePass on an IR node
pub fn run_blunt_code_pass(py: Python, ir: &Py<PyAny>) -> PyResult<Py<PyAny>> {
    use crate::semantic::blunt_code::BluntCodePass;
    use crate::semantic::optimizer::OptimizationPass;

    let pass = BluntCodePass::new();
    let ir_bound = ir.bind(py);
    pass.visit(py, ir_bound)
}

/// Run the ConstantFolding pass on an IR node
pub fn run_constant_folding_pass(py: Python, ir: &Py<PyAny>) -> PyResult<Py<PyAny>> {
    use crate::semantic::constant_folding::ConstantFoldingPass;
    use crate::semantic::optimizer::OptimizationPass;

    let pass = ConstantFoldingPass::new();
    let ir_bound = ir.bind(py);
    pass.visit(py, ir_bound)
}

/// Extract the opcode from an IR or Op node
pub fn get_opcode(py: Python, node: &Py<PyAny>) -> PyResult<i32> {
    let node_bound = node.bind(py);
    node_bound.getattr("ident")?.extract::<i32>()
}

/// Check if a node is a Python value with a specific value
pub fn is_value<T>(py: Python<'_>, node: &Py<PyAny>, expected: T) -> bool
where
    T: PartialEq + for<'py> pyo3::FromPyObject<'py, 'py>,
{
    node.bind(py)
        .extract::<T>()
        .map(|v| v == expected)
        .unwrap_or(false)
}

/// Check if a node is an IR node with a specific opcode
pub fn has_opcode(py: Python, node: &Py<PyAny>, expected_opcode: IROpCode) -> bool {
    match get_opcode(py, node) {
        Ok(opcode) => opcode == expected_opcode as i32,
        Err(_) => false,
    }
}

/// Create a Ref node (variable reference)
pub fn ref_node(py: Python, name: &str) -> Py<PyAny> {
    let nodes_mod = py
        .import("catnip.nodes")
        .expect("Failed to import catnip.nodes");
    let ref_class = nodes_mod.getattr("Ref").expect("Failed to get Ref class");
    ref_class
        .call1((name,))
        .expect("Failed to create Ref")
        .unbind()
}
