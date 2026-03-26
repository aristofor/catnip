// FILE: catnip_lsp/src/symbols.rs
use catnip_grammar::node_kinds as NK;
use tree_sitter::{Node, Parser, Tree};

/// A reference to a symbol: byte offset range + line/column.
#[derive(Debug, Clone)]
pub struct SymbolRef {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Node kinds that introduce a new scope in Catnip.
fn is_scope_boundary(kind: &str) -> bool {
    matches!(kind, NK::BLOCK | NK::LAMBDA_EXPR | NK::FOR_STMT | NK::WHILE_STMT)
}

/// Check if a node is an identifier with the given name.
fn is_identifier(node: &Node, source: &[u8], name: &str) -> bool {
    node.kind() == NK::IDENTIFIER && node.utf8_text(source) == Ok(name)
}

/// Parse source with the Catnip tree-sitter grammar.
pub fn parse(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&catnip_grammar::get_language()).ok()?;
    parser.parse(source, None)
}

/// Find the identifier node at the given (0-indexed) line and column.
pub fn find_identifier_at(tree: &Tree, source: &[u8], line: u32, col: u32) -> Option<String> {
    let point = tree_sitter::Point::new(line as usize, col as usize);
    let node = tree.root_node().descendant_for_point_range(point, point)?;

    // Walk up to find the identifier
    let mut current = node;
    loop {
        if current.kind() == NK::IDENTIFIER {
            return current.utf8_text(source).ok().map(|s| s.to_string());
        }
        current = current.parent()?;
    }
}

/// Resolve the identifier around a cursor position.
///
/// VS Code can invoke rename with the caret at the end of a symbol. Tree-sitter
/// point lookups are stricter at token boundaries, so we retry one column to the
/// left on the same line.
fn find_identifier_near(tree: &Tree, source: &[u8], line: u32, col: u32) -> Option<String> {
    find_identifier_at(tree, source, line, col).or_else(|| {
        col.checked_sub(1)
            .and_then(|left| find_identifier_at(tree, source, line, left))
    })
}

/// Find the smallest scope node containing the given point.
fn enclosing_scope<'a>(root: &'a Node<'a>, point: tree_sitter::Point) -> Node<'a> {
    let mut cursor = root.walk();
    let mut scope = *root;

    'outer: loop {
        let node = cursor.node();
        if node.start_position() <= point && point <= node.end_position() {
            if is_scope_boundary(node.kind()) {
                scope = node;
            }
            // Try to descend
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.start_position() <= point && point <= child.end_position() {
                        continue 'outer;
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
                cursor.goto_parent();
            }
        }
        break;
    }

    scope
}

/// Collect all identifier nodes with the given name within a subtree.
fn collect_refs(node: &Node, source: &[u8], name: &str, refs: &mut Vec<SymbolRef>) {
    if is_identifier(node, source, name) {
        refs.push(SymbolRef {
            start_line: node.start_position().row as u32,
            start_col: node.start_position().column as u32,
            end_line: node.end_position().row as u32,
            end_col: node.end_position().column as u32,
        });
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            // Don't cross into nested scopes - unless the name could be captured
            // For simplicity in v1, we collect all refs within the enclosing scope
            collect_refs(&child, source, name, refs);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Find all references to the symbol at the given position.
/// Returns None if there's no identifier at that position.
pub fn find_references(source: &str, line: u32, col: u32) -> Option<(String, Vec<SymbolRef>)> {
    let tree = parse(source)?;
    let source_bytes = source.as_bytes();

    let name = find_identifier_near(&tree, source_bytes, line, col)?;
    let point = tree_sitter::Point::new(line as usize, col as usize);

    let root = tree.root_node();
    let scope = enclosing_scope(&root, point);
    let mut refs = Vec::new();
    collect_refs(&scope, source_bytes, &name, &mut refs);

    if refs.is_empty() { None } else { Some((name, refs)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_refs_simple() {
        let src = "x = 10\ny = x + 1\nx";
        let (name, refs) = find_references(src, 0, 0).expect("should find x");
        assert_eq!(name, "x");
        assert_eq!(refs.len(), 3); // assignment + usage + final ref
    }

    #[test]
    fn test_find_refs_in_function() {
        let src = "f = (x) => { x + 1 }\nf(10)";
        let (name, refs) = find_references(src, 0, 5).expect("should find x");
        assert_eq!(name, "x");
        assert_eq!(refs.len(), 2); // param + body
    }

    #[test]
    fn test_no_identifier_on_operator() {
        let src = "x + y";
        assert!(find_references(src, 0, 2).is_none());
    }

    #[test]
    fn test_find_refs_with_cursor_at_identifier_end() {
        let src = "value = 1\nvalue";
        let (name, refs) = find_references(src, 0, 5).expect("should find value at end boundary");
        assert_eq!(name, "value");
        assert_eq!(refs.len(), 2);
    }
}
