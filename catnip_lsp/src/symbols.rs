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

/// Node kinds that delimit a lexical scope in Catnip.
///
/// A `lambda_expr` owns both its parameters and its body block, and a `for_stmt`
/// owns its loop variable and body block. Treating those nodes (rather than the
/// inner `block`) as scopes keeps a binding and the code that sees it inside one
/// subtree. `source_file` is the module scope and the fallback binding scope.
fn is_scope(kind: &str) -> bool {
    matches!(
        kind,
        NK::SOURCE_FILE | NK::BLOCK | NK::LAMBDA_EXPR | NK::FOR_STMT | NK::WHILE_STMT
    )
}

/// Check if a node is an identifier with the given name.
fn is_identifier(node: &Node, source: &[u8], name: &str) -> bool {
    node.kind() == NK::IDENTIFIER && node.utf8_text(source) == Ok(name)
}

/// Field name connecting `node` to its parent, e.g. `attribute` for the
/// identifier in `obj.attribute`. None for the root or unnamed positions.
fn field_in_parent(node: &Node) -> Option<&'static str> {
    let parent = node.parent()?;
    let mut cursor = parent.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().id() == node.id() {
                return cursor.field_name();
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// True when an identifier is NOT a variable reference but a member name, a
/// keyword-argument key, or the declared name of a struct/trait/enum/method.
///
/// These live in unrelated namespaces (attributes, kwargs, type members), so
/// renaming a variable must never touch them. The discriminator is the
/// parent kind plus the tree-sitter field linking the identifier to its parent.
fn is_non_variable_position(node: &Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        // `obj.attribute` / `obj.method(...)`: the trailing identifier is a member.
        NK::GETATTR | NK::CALLATTR => true,
        // `f(key=value)`: only the `key` side is a name, not the value subtree.
        NK::KWARG | NK::DICT_KWARG => field_in_parent(node) == Some("key"),
        // Declared names of type-level constructs (not value variables).
        NK::STRUCT_STMT | NK::TRAIT_STMT | NK::ENUM_STMT => field_in_parent(node) == Some("name"),
        NK::STRUCT_METHOD => field_in_parent(node) == Some("method_name"),
        _ => false,
    }
}

/// Nearest enclosing scope node, starting from `node`'s parent.
fn enclosing_scope<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent()?;
    loop {
        if is_scope(current.kind()) {
            return Some(current);
        }
        current = current.parent()?;
    }
}

/// True when an identifier introduces a binding for its name (and thus opens or
/// belongs to a scope): an assignment target, a lambda parameter, a for-loop
/// variable, an `except` binding, or a `with` binding.
///
/// Used to locate the scope that owns a name and to detect shadowing.
fn is_binding_occurrence(node: &Node) -> bool {
    let mut current = *node;
    // Walk up through the lvalue / unpack wrappers that hold assignment targets.
    while let Some(parent) = current.parent() {
        match parent.kind() {
            // Assignment targets sit under lvalue > unpack_target (possibly nested
            // in unpack tuples/sequences). The RHS expression is a sibling that is
            // not an lvalue, so reaching `assignment` through lvalue wrappers means
            // this identifier is being bound.
            NK::LVALUE => return true,
            NK::UNPACK_TARGET | NK::UNPACK_TUPLE | NK::UNPACK_SEQUENCE | NK::UNPACK_ITEMS => {
                current = parent;
            }
            // Lambda parameter / variadic parameter names.
            NK::LAMBDA_PARAM | NK::VARIADIC_PARAM => return field_in_parent(node) == Some("name"),
            // for <target> in ...: the target is a direct unpack_target child.
            NK::FOR_STMT => return true,
            // except <Type> as binding: ... and with binding = ...
            NK::EXCEPT_CLAUSE => return field_in_parent(node) == Some("binding"),
            // `with_binding` has no node_kinds constant; match by rule name.
            "with_binding" => return field_in_parent(node) == Some("name"),
            _ => return false,
        }
    }
    false
}

/// True if `scope` directly binds `name`: there is a binding occurrence of `name`
/// whose nearest enclosing scope is exactly `scope` (bindings in deeper nested
/// scopes do not count).
fn scope_binds(scope: &Node, source: &[u8], name: &str) -> bool {
    let mut found = false;
    walk_until_scope(scope, &mut |node| {
        if is_identifier(node, source, name) && is_binding_occurrence(node) {
            found = true;
        }
    });
    found
}

/// Visit every descendant of `scope` without descending into nested scopes.
fn walk_until_scope(scope: &Node, visit: &mut impl FnMut(&Node)) {
    let mut cursor = scope.walk();
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        let child = cursor.node();
        visit(&child);
        if !is_scope(child.kind()) {
            walk_until_scope(&child, visit);
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Resolve the scope that binds `name` for the occurrence at `node`: the nearest
/// enclosing scope that binds `name`, walking outward. Falls back to the module
/// scope (`source_file`) when no scope binds the name.
fn binding_scope<'a>(node: &Node<'a>, root: &Node<'a>, source: &[u8], name: &str) -> Node<'a> {
    let mut scope = enclosing_scope(node);
    while let Some(s) = scope {
        if scope_binds(&s, source, name) {
            return s;
        }
        scope = enclosing_scope(&s);
    }
    *root
}

/// Collect every variable occurrence of `name` within `scope`, pruning any nested
/// scope that re-binds `name` (shadowing). Member names, kwarg keys, and type
/// declaration names are excluded.
fn collect_refs(scope: &Node, source: &[u8], name: &str, refs: &mut Vec<SymbolRef>) {
    let mut cursor = scope.walk();
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        let child = cursor.node();

        if is_identifier(&child, source, name) && !is_non_variable_position(&child) {
            refs.push(SymbolRef {
                start_line: child.start_position().row as u32,
                start_col: child.start_position().column as u32,
                end_line: child.end_position().row as u32,
                end_col: child.end_position().column as u32,
            });
        }

        // Recurse, but stop at a nested scope that re-binds `name` (shadowing):
        // those occurrences belong to a different variable.
        let shadows = is_scope(child.kind()) && scope_binds(&child, source, name);
        if !shadows {
            collect_refs(&child, source, name, refs);
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Parse source with the Catnip tree-sitter grammar.
pub fn parse(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&catnip_grammar::get_language()).ok()?;
    parser.parse(source, None)
}

/// Locate the identifier node under the cursor.
///
/// VS Code can invoke rename with the caret at the end of a symbol. Tree-sitter
/// point lookups are stricter at token boundaries, so we retry one column to the
/// left on the same line.
fn identifier_node_near<'a>(tree: &'a Tree, line: u32, col: u32) -> Option<Node<'a>> {
    identifier_node_at(tree, line, col)
        .or_else(|| col.checked_sub(1).and_then(|left| identifier_node_at(tree, line, left)))
}

/// Identifier node containing the given (0-indexed) line and column, walking up
/// from the descendant at that point to the enclosing identifier.
fn identifier_node_at<'a>(tree: &'a Tree, line: u32, col: u32) -> Option<Node<'a>> {
    let point = tree_sitter::Point::new(line as usize, col as usize);
    let mut current = tree.root_node().descendant_for_point_range(point, point)?;
    loop {
        if current.kind() == NK::IDENTIFIER {
            return Some(current);
        }
        current = current.parent()?;
    }
}

/// Find all references to the symbol at the given position.
///
/// Resolves the occurrence under the cursor to its binding scope (lexical scope
/// resolution), then collects every variable occurrence of the name reachable
/// from that scope, excluding member/kwarg/declaration positions and nested
/// scopes that shadow the name. Returns None if there's no identifier there.
pub fn find_references(source: &str, line: u32, col: u32) -> Option<(String, Vec<SymbolRef>)> {
    let tree = parse(source)?;
    let source_bytes = source.as_bytes();

    let ident = identifier_node_near(&tree, line, col)?;
    // The cursor may land on a member/kwarg/declaration name, which is not a
    // renameable variable reference.
    if is_non_variable_position(&ident) {
        return None;
    }
    let name = ident.utf8_text(source_bytes).ok()?.to_string();

    let root = tree.root_node();
    let scope = binding_scope(&ident, &root, source_bytes, &name);

    let mut refs = Vec::new();
    // Collect occurrences directly bound in this scope, then descend.
    if is_identifier(&scope, source_bytes, &name) && !is_non_variable_position(&scope) {
        refs.push(SymbolRef {
            start_line: scope.start_position().row as u32,
            start_col: scope.start_position().column as u32,
            end_line: scope.end_position().row as u32,
            end_col: scope.end_position().column as u32,
        });
    }
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

    // --- F2: member / kwarg / declaration names are not variable references ---

    #[test]
    fn test_rename_skips_attribute_access() {
        // Renaming the variable `x` must not touch the attribute `obj.x`.
        let src = "x = 1\nobj.x";
        let (name, refs) = find_references(src, 0, 0).expect("should find x");
        assert_eq!(name, "x");
        assert_eq!(refs.len(), 1, "only the binding `x`, not `obj.x`");
        assert_eq!(refs[0].start_line, 0);
    }

    #[test]
    fn test_rename_skips_method_call() {
        // `arr.len()` is a method name, not a reference to the variable `len`.
        let src = "len = 3\narr.len()";
        let (name, refs) = find_references(src, 0, 0).expect("should find len");
        assert_eq!(name, "len");
        assert_eq!(refs.len(), 1, "only the binding `len`, not `arr.len()`");
    }

    #[test]
    fn test_rename_skips_kwarg_key() {
        // In `f(x=2)`, the `x` key is a keyword-argument name, not the variable.
        let src = "x = 1\nf(x=2)";
        let (name, refs) = find_references(src, 0, 0).expect("should find x");
        assert_eq!(name, "x");
        assert_eq!(refs.len(), 1, "only the binding `x`, not the kwarg key");
    }

    #[test]
    fn test_kwarg_value_is_a_reference() {
        // The value side of a kwarg IS a variable reference.
        let src = "x = 1\nf(k=x)";
        let (name, refs) = find_references(src, 0, 0).expect("should find x");
        assert_eq!(name, "x");
        assert_eq!(refs.len(), 2, "binding + kwarg value reference");
    }

    #[test]
    fn test_cursor_on_attribute_is_none() {
        // Cursor on the attribute name itself is not a renameable variable.
        let src = "obj.field";
        assert!(find_references(src, 0, 4).is_none());
    }

    // --- F3: lexical scope resolution (partial rename + shadowing) ---

    #[test]
    fn test_rename_from_inner_usage_finds_outer_def() {
        // Rename triggered from a usage inside a nested block must still rename
        // the outer definition and the sibling usage (no partial rename).
        let src = "x = 1\n{\n  x + 1\n}\nx + 2";
        let (name, refs) = find_references(src, 2, 2).expect("should find x inside block");
        assert_eq!(name, "x");
        assert_eq!(refs.len(), 3, "def + inner usage + sibling usage");
    }

    #[test]
    fn test_rename_global_used_in_lambda() {
        // A global referenced inside a lambda body resolves to the module scope.
        let src = "g = 10\nf = (a) => { g + a }\ng";
        let (name, refs) = find_references(src, 1, 13).expect("should find g inside lambda");
        assert_eq!(name, "g");
        assert_eq!(refs.len(), 3, "def + lambda-body usage + trailing usage");
    }

    #[test]
    fn test_shadowing_outer_var_not_renamed_in_inner_scope() {
        // Renaming the outer `x` must NOT touch the lambda param `x` that shadows
        // it, nor the lambda body that refers to the param.
        let src = "x = 1\nf = (x) => { x + 1 }\nx + 2";
        let (name, refs) = find_references(src, 0, 0).expect("should find outer x");
        assert_eq!(name, "x");
        assert_eq!(refs.len(), 2, "outer def + trailing usage only (not the shadow)");
        // Both refs are on the module-scope lines, never line 1 (the lambda).
        assert!(
            refs.iter().all(|r| r.start_line != 1),
            "must not touch shadowing param/body"
        );
    }

    #[test]
    fn test_shadowing_inner_var_isolated() {
        // Renaming from the inner (shadowing) param renames only the inner scope.
        let src = "x = 1\nf = (x) => { x + 1 }\nx + 2";
        let (name, refs) = find_references(src, 1, 5).expect("should find inner x (param)");
        assert_eq!(name, "x");
        assert_eq!(refs.len(), 2, "param + body usage only");
        assert!(refs.iter().all(|r| r.start_line == 1), "stays inside the lambda");
    }

    #[test]
    fn test_for_loop_variable_scope() {
        // The loop variable is bound by the for-loop; its uses inside the body
        // are collected, and an unrelated outer name is not pulled in.
        let src = "for i in range(3) { i + i }";
        let (name, refs) = find_references(src, 0, 4).expect("should find loop var i");
        assert_eq!(name, "i");
        assert_eq!(refs.len(), 3, "loop var + two body usages");
    }
}
