use tree_sitter::Node;

/// Find syntax errors in the parse tree
pub fn find_errors(node: Node, source: &str) -> Option<String> {
    if node.kind() == "ERROR" {
        let text = &source[node.byte_range()];
        let (line, col) = (
            node.start_position().row + 1,
            node.start_position().column + 1,
        );

        if !text.trim().is_empty() {
            let snippet = if text.len() > 20 { &text[..20] } else { text };
            return Some(format!(
                "Unexpected token '{}' at line {}, column {}",
                snippet, line, col
            ));
        } else {
            return Some(format!("Syntax error at line {}, column {}", line, col));
        }
    }

    if node.is_missing() {
        let (line, col) = (
            node.start_position().row + 1,
            node.start_position().column + 1,
        );
        let expected = node.kind().replace('_', " ");
        return Some(format!(
            "Expected {} at line {}, column {}",
            expected, line, col
        ));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(error) = find_errors(child, source) {
            return Some(error);
        }
    }

    None
}
