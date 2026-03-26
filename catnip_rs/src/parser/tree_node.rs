// FILE: catnip_rs/src/parser/tree_node.rs
use pyo3::prelude::*;
use tree_sitter::Node;

/// A Python-accessible representation of a tree-sitter Node
#[pyclass(name = "TreeNode")]
pub struct TreeNode {
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    start_byte: usize,
    #[pyo3(get)]
    end_byte: usize,
    #[pyo3(get)]
    start_row: usize,
    #[pyo3(get)]
    start_col: usize,
    #[pyo3(get)]
    end_row: usize,
    #[pyo3(get)]
    end_col: usize,
    #[pyo3(get)]
    text: String,
    children: Vec<Py<TreeNode>>,
}

#[pymethods]
impl TreeNode {
    #[getter]
    fn children(&self, py: Python) -> Vec<Py<PyAny>> {
        self.children.iter().map(|c| c.clone_ref(py).into_any()).collect()
    }

    fn pretty(&self, py: Python) -> String {
        self._pretty(py, 0)
    }

    fn __repr__(&self) -> String {
        format!("<TreeNode {} {}..{}>", self.kind, self.start_byte, self.end_byte)
    }
}

impl TreeNode {
    /// Create a TreeNode from a tree-sitter Node
    pub fn from_node(py: Python, node: Node, source: &str) -> PyResult<Py<TreeNode>> {
        let text = node.utf8_text(source.as_bytes()).unwrap_or("").to_string();

        // Recursively create children
        let mut children = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            children.push(TreeNode::from_node(py, child, source)?);
        }

        Py::new(
            py,
            TreeNode {
                kind: node.kind().to_string(),
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                start_row: node.start_position().row,
                start_col: node.start_position().column,
                end_row: node.end_position().row,
                end_col: node.end_position().column,
                text,
                children,
            },
        )
    }

    fn _pretty(&self, py: Python, indent: usize) -> String {
        let indent_str = "  ".repeat(indent);
        let mut result = format!("{}({}", indent_str, self.kind);

        // Add text for leaf nodes
        if self.children.is_empty() && !self.text.is_empty() && self.text.len() < 40 {
            let text_repr = if self.text.contains('\n') {
                format!("{:?}", self.text)
            } else {
                self.text.clone()
            };
            result.push_str(&format!(" \"{}\"", text_repr));
        }

        // Add children
        if !self.children.is_empty() {
            for child_py in &self.children {
                let child = child_py.borrow(py);
                result.push('\n');
                result.push_str(&child._pretty(py, indent + 1));
            }
            result.push('\n');
            result.push_str(&indent_str);
        }

        result.push(')');
        result
    }
}
