// FILE: catnip_rs/src/parser/transforms.rs
//! PyO3 wrapper around pure_transforms
//!
//! Delegates transformation to pure_transforms (Rust) and converts
//! the result to Python objects for compatibility with existing API.

use pyo3::prelude::*;
use pyo3::types::PyList;
use tree_sitter::Node;

use crate::ir::ir_pure_to_python;
use crate::parser::pure_transforms;

/// Main transform dispatcher (PyO3 wrapper)
pub fn transform(py: Python, node: Node, source: &str, _level: i32) -> PyResult<Py<PyAny>> {
    // Special handling for source_file to maintain list return type
    if node.kind() == "source_file" {
        return transform_source_file(py, node, source);
    }

    // Delegate to pure_transforms
    let ir_pure = pure_transforms::transform(node, source).map_err(pyo3::exceptions::PyValueError::new_err)?;

    // Convert IR → Python object
    ir_pure_to_python(py, ir_pure)
}

/// Transform source file (root node) - maintains list return type
fn transform_source_file(py: Python, node: Node, source: &str) -> PyResult<Py<PyAny>> {
    let mut results = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        // Skip unnamed nodes
        if !child.is_named() {
            continue;
        }

        // Transform child using pure_transforms
        let ir_pure = pure_transforms::transform(child, source).map_err(pyo3::exceptions::PyValueError::new_err)?;

        // Convert to Python
        let py_result = ir_pure_to_python(py, ir_pure)?;

        // Filter out None results EXCEPT for 'none' literals
        if !py_result.is_none(py) || contains_none_literal(&child) {
            results.push(py_result);
        }
    }

    // Always return as list for consistency with old parser behavior
    Ok(PyList::new(py, &results)?.into())
}

/// Check if a node or its descendants contain a 'none' literal node
fn contains_none_literal(node: &Node) -> bool {
    if node.kind() == "none" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_none_literal(&child) {
            return true;
        }
    }
    false
}

// Re-export utils helpers
pub use super::utils::node_text;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_simple() {
        Python::initialize();
        Python::attach(|_py| ());
    }
}
