// FILE: catnip_rs/src/parser/core.rs
use catnip_tools::errors::find_errors;
use pyo3::prelude::*;
use tree_sitter::Parser;

#[pyclass(name = "TreeSitterParser")]
pub struct TreeSitterParser {
    parser: Parser,
}

#[pymethods]
impl TreeSitterParser {
    #[new]
    fn new() -> PyResult<Self> {
        let language = crate::get_tree_sitter_language();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Failed to set language: {}", e)))?;
        Ok(Self { parser })
    }

    #[pyo3(signature = (source, level=3))]
    fn parse(&mut self, py: Python, source: &str, level: i32) -> PyResult<Py<PyAny>> {
        // Parse with tree-sitter
        let tree = self
            .parser
            .parse(source, None)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PySyntaxError, _>("Parse failed"))?;

        // Check for syntax errors
        if let Some(error_msg) = find_errors(tree.root_node(), source) {
            return Err(PyErr::new::<pyo3::exceptions::PySyntaxError, _>(error_msg));
        }

        // Level 0: return parse tree
        if level == 0 {
            let tree_node = crate::parser::tree_node::TreeNode::from_node(py, tree.root_node(), source)?;
            return Ok(tree_node.into_bound(py).into_any().unbind());
        }

        // Transform to IR
        crate::parser::transforms::transform(py, tree.root_node(), source, level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_creation() {
        Python::initialize();
        Python::attach(|_py| {
            let parser = TreeSitterParser::new();
            assert!(parser.is_ok());
        });
    }

    #[test]
    fn test_parse_simple() {
        Python::initialize();
        Python::attach(|py| {
            let mut parser = TreeSitterParser::new().unwrap();
            let result = parser.parse(py, "1 + 2", 3);
            // Should not error on valid syntax
            assert!(result.is_ok());
        });
    }

    #[test]
    fn test_parse_syntax_error() {
        Python::initialize();
        Python::attach(|py| {
            let mut parser = TreeSitterParser::new().unwrap();
            let result = parser.parse(py, "if {", 3);
            // Should error on invalid syntax
            assert!(result.is_err());
        });
    }
}
