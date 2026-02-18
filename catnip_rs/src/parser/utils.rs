// FILE: catnip_rs/src/parser/utils.rs
use tree_sitter::Node;

/// Extract text from node
pub fn node_text<'a>(node: &Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Get child by field name
pub fn get_child_by_field<'a>(node: &'a Node, field: &str) -> Option<Node<'a>> {
    node.child_by_field_name(field)
}

/// Check if node is named (not punctuation)
pub fn is_named(node: &Node) -> bool {
    node.is_named()
}

/// Iterate over named children (excluding comments)
pub fn named_children<'a>(node: &Node<'a>) -> Vec<Node<'a>> {
    let mut result = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() && child.kind() != "comment" {
            result.push(child);
        }
    }
    result
}

/// Unescape string literal (handles \n, \t, \r, \\, \", \')
pub fn unescape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Extract operator text from node children (searches for *_op nodes or unnamed children)
pub fn extract_operator<'a>(node: &Node, source: &'a str) -> Option<String> {
    let mut cursor = node.walk();

    // First pass: look for nodes ending with "_op"
    for child in node.children(&mut cursor) {
        if child.kind().ends_with("_op") {
            let mut op_cursor = child.walk();
            for op_child in child.children(&mut op_cursor) {
                if !op_child.is_named() {
                    let text = node_text(&op_child, source).trim();
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
        }
    }

    // Second pass: look for unnamed children (excluding parens)
    cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            let text = node_text(&child, source).trim();
            if !text.is_empty() && text != "(" && text != ")" {
                return Some(text.to_string());
            }
        }
    }

    None
}
