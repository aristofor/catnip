// FILE: catnip_rs/src/dispatch.rs
//! Pipeline orchestration - parse → semantic → execute
//!
//! Centralizes the Catnip pipeline logic in Rust to reduce Python overhead.
//! Parsing levels (0-3) are handled here; Python only handles display.

use crate::ir::PyIRNode;
use crate::parser::transform_pure;
use crate::pipeline::SemanticAnalyzer;
use pyo3::prelude::*;
use pyo3::types::PyList;
use tree_sitter::Parser;

/// Process input at different parsing levels
///
/// Parsing levels:
/// - 0: Parse tree only (tree-sitter AST)
/// - 1: IR only (after transformer, before semantic)
/// - 2: Executable IR (after semantic analysis)
/// - 3: Execute and return result (default)
#[pyfunction]
#[pyo3(signature = (catnip, text, level=3))]
pub fn process_input(py: Python<'_>, catnip: &Bound<'_, PyAny>, text: &str, level: u8) -> PyResult<Py<PyAny>> {
    match level {
        0 => process_level_0(py, catnip, text),
        1 => process_level_ir(py, text, false),
        2 => process_level_ir(py, text, true),
        3 => process_level_3(py, catnip, text),
        _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "Invalid parsing level: {}. Must be 0-3",
            level
        ))),
    }
}

/// Level 0: Parse tree only (no transformation)
fn process_level_0(py: Python<'_>, _catnip: &Bound<'_, PyAny>, text: &str) -> PyResult<Py<PyAny>> {
    let language = crate::get_tree_sitter_language();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Language error: {}", e)))?;

    let tree = parser
        .parse(text, None)
        .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Failed to parse source"))?;

    let root = tree.root_node();
    if let Some(error_msg) = catnip_tools::errors::find_errors(root, text) {
        return Err(pyo3::exceptions::PySyntaxError::new_err(error_msg));
    }

    let tree_node = crate::parser::tree_node::TreeNode::from_node(py, root, text)?;
    Ok(tree_node.into_bound(py).into_any().unbind())
}

/// Levels 1/2: Pure Rust path returning list of PyIRNode.
/// semantic=false → level 1 (raw IR), semantic=true → level 2 (after analysis).
fn process_level_ir(py: Python<'_>, text: &str, semantic: bool) -> PyResult<Py<PyAny>> {
    let language = crate::get_tree_sitter_language();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Language error: {}", e)))?;

    let tree = parser
        .parse(text, None)
        .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Failed to parse source"))?;

    let root = tree.root_node();
    if let Some(error_msg) = catnip_tools::errors::find_errors(root, text) {
        return Err(pyo3::exceptions::PySyntaxError::new_err(error_msg));
    }

    let ir = transform_pure(root, text).map_err(pyo3::exceptions::PySyntaxError::new_err)?;

    let result_ir = if semantic {
        let mut analyzer = SemanticAnalyzer::with_optimizer();
        analyzer
            .analyze(&ir)
            .map_err(pyo3::exceptions::PyRuntimeError::new_err)?
    } else {
        ir
    };

    let items = PyIRNode::unwrap_program(result_ir);
    let list = PyList::empty(py);
    for item in items {
        list.append(Py::new(py, item)?)?;
    }
    Ok(list.into_any().unbind())
}

/// Level 3: Execute and return result
fn process_level_3(_py: Python<'_>, catnip: &Bound<'_, PyAny>, text: &str) -> PyResult<Py<PyAny>> {
    // Parse
    catnip.call_method1("parse", (text,))?;

    // Execute and return result
    let result = catnip.call_method0("execute")?;
    Ok(result.unbind())
}

/// Pipeline module initialization
pub fn init_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(process_input, m)?)?;
    Ok(())
}
