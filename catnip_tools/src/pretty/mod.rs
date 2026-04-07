// FILE: catnip_tools/src/pretty/mod.rs
//! Wadler-Leijen pretty-printer for Catnip.

pub(crate) mod align;
pub(crate) mod combinators;
pub(crate) mod convert;
pub(crate) mod convert_decl;
pub(crate) mod convert_expr;
pub(crate) mod convert_stmt;
pub(crate) mod doc;
pub(crate) mod layout;

use crate::config::FormatConfig;
use align::{align_columns, normalize_newlines};
use convert::convert;
use doc::Arena;
use layout::layout;

/// Format Catnip source using the Wadler-Leijen pretty-printer.
pub fn format_code_pretty(source: &str, config: &FormatConfig) -> Result<String, String> {
    // Preserve shebang
    let (shebang, code) = if source.starts_with("#!") {
        if let Some(pos) = source.find('\n') {
            (Some(&source[..=pos]), &source[pos + 1..])
        } else {
            (Some(source), "")
        }
    } else {
        (None, source)
    };

    // Parse
    let language = crate::get_language();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| format!("Failed to set language: {e}"))?;
    let tree = parser.parse(code, None).ok_or("Parse failed")?;

    // Convert CST -> Doc
    let mut arena = Arena::new();
    let doc = convert(&mut arena, tree.root_node(), code.as_bytes(), config.indent_size as i32);

    // Layout
    let (formatted, line_map) = layout(&arena, doc, config.line_length);

    // Align columns
    let formatted = align_columns(&formatted, code, &line_map, config);

    // Normalize newlines (max 2 consecutive) and strip trailing whitespace per line
    let formatted = normalize_newlines(&formatted);
    let formatted: String = formatted.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");

    // Trailing newline
    let formatted = formatted.trim_end_matches('\n').to_string() + "\n";

    if let Some(shebang) = shebang {
        Ok(format!("{shebang}{formatted}"))
    } else {
        Ok(formatted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(source: &str) -> String {
        let config = FormatConfig::default();
        format_code_pretty(source, &config).unwrap()
    }

    // -- Basic expressions --

    #[test]
    fn test_simple_assignment() {
        assert_eq!(fmt("x=1+2"), "x = 1 + 2\n");
    }

    #[test]
    fn test_binary_ops() {
        assert_eq!(fmt("a+b-c*d/e"), "a + b - c * d / e\n");
    }

    #[test]
    fn test_unary_minus() {
        assert_eq!(fmt("x = -1"), "x = -1\n");
    }

    #[test]
    fn test_unary_in_expr() {
        assert_eq!(fmt("y = a + -b"), "y = a + -b\n");
    }

    #[test]
    fn test_bool_not() {
        assert_eq!(fmt("not x"), "not x\n");
    }

    #[test]
    fn test_bool_ops() {
        assert_eq!(fmt("a and b or c"), "a and b or c\n");
    }

    #[test]
    fn test_comparison() {
        assert_eq!(fmt("a < b"), "a < b\n");
    }

    #[test]
    fn test_null_coalesce() {
        assert_eq!(fmt("a ?? b"), "a ?? b\n");
    }

    // -- Calls and kwargs --

    #[test]
    fn test_call_simple() {
        assert_eq!(fmt("f(a, b)"), "f(a, b)\n");
    }

    #[test]
    fn test_call_kwargs() {
        assert_eq!(fmt("f(a=1, b=2)"), "f(a=1, b=2)\n");
    }

    #[test]
    fn test_call_no_args() {
        assert_eq!(fmt("f()"), "f()\n");
    }

    // -- Blocks --

    #[test]
    fn test_block_inline() {
        assert_eq!(fmt("{ 1 }"), "{ 1 }\n");
    }

    #[test]
    fn test_empty_block() {
        assert_eq!(fmt("{}"), "{}\n");
    }

    // -- Comments --

    #[test]
    fn test_standalone_comment() {
        assert_eq!(fmt("# hello"), "# hello\n");
    }

    #[test]
    fn test_trailing_comment() {
        assert_eq!(fmt("x = 1  # comment"), "x = 1  # comment\n");
    }

    // -- Strings --

    #[test]
    fn test_string_literal() {
        assert_eq!(fmt("x = \"hello\""), "x = \"hello\"\n");
    }

    // -- Misc --

    #[test]
    fn test_empty_source() {
        assert_eq!(fmt(""), "\n");
    }

    #[test]
    fn test_shebang() {
        let result = fmt("#!/usr/bin/env catnip\nx=1");
        assert!(result.starts_with("#!/usr/bin/env catnip\n"));
        assert!(result.contains("x = 1"));
    }

    #[test]
    fn test_parens() {
        assert_eq!(fmt("(a + b)"), "(a + b)\n");
    }

    #[test]
    fn test_return_stmt() {
        assert_eq!(fmt("return 42"), "return 42\n");
    }

    #[test]
    fn test_blank_line_preserved() {
        assert_eq!(fmt("x = 1\n\ny = 2"), "x = 1\n\ny = 2\n");
    }

    #[test]
    fn test_multiple_statements() {
        assert_eq!(fmt("x = 1\ny = 2\nz = 3"), "x = 1\ny = 2\nz = 3\n");
    }

    // -- Control flow --

    #[test]
    fn test_if_simple() {
        assert_eq!(fmt("if x { 1 }"), "if x { 1 }\n");
    }

    #[test]
    fn test_if_multiline() {
        let source = "if x {\n    y = 1\n    z = 2\n}";
        let result = fmt(source);
        assert!(result.contains("if x {"));
        assert!(result.contains("    y = 1"));
        assert!(result.contains("    z = 2"));
    }

    #[test]
    fn test_if_else() {
        let source = "if x {\n    1\n}\nelse {\n    2\n}";
        let result = fmt(source);
        assert!(result.contains("if x {"));
        assert!(result.contains("else {"));
    }

    #[test]
    fn test_if_elif_else() {
        let source = "if a {\n    1\n}\nelif b {\n    2\n}\nelse {\n    3\n}";
        let result = fmt(source);
        assert!(result.contains("if a {"));
        assert!(result.contains("elif b {"));
        assert!(result.contains("else {"));
    }

    #[test]
    fn test_while() {
        assert_eq!(fmt("while x { 1 }"), "while x { 1 }\n");
    }

    #[test]
    fn test_for() {
        assert_eq!(fmt("for i in items { i }"), "for i in items { i }\n");
    }

    #[test]
    fn test_match_simple() {
        let source = "match x {\n    1 => { \"one\" }\n    _ => { \"other\" }\n}";
        let result = fmt(source);
        assert!(result.contains("match x {"));
        assert!(result.contains("1 => "));
        assert!(result.contains("_ => "));
    }

    #[test]
    fn test_match_or_pattern() {
        let source = "match x {\n    1 | 2 | 3 => { \"small\" }\n    _ => { \"big\" }\n}";
        let result = fmt(source);
        assert!(result.contains("1 | 2 | 3 => "));
    }

    #[test]
    fn test_match_guard() {
        let source = "match x {\n    n if n > 0 => { n }\n    _ => { 0 }\n}";
        let result = fmt(source);
        assert!(result.contains("n if n > 0 => "));
    }

    #[test]
    fn test_pragma() {
        assert_eq!(fmt("pragma(\"tco\", True)"), "pragma(\"tco\", True)\n");
    }

    // -- Lambda --

    #[test]
    fn test_lambda_no_params() {
        assert_eq!(fmt("f = () => { 1 }"), "f = () => { 1 }\n");
    }

    #[test]
    fn test_lambda_with_params() {
        assert_eq!(fmt("f = (x, y) => { x + y }"), "f = (x, y) => { x + y }\n");
    }

    #[test]
    fn test_lambda_default_param() {
        assert_eq!(fmt("f = (x, y=0) => { x + y }"), "f = (x, y=0) => { x + y }\n");
    }

    // -- Chained --

    #[test]
    fn test_chained_getattr() {
        assert_eq!(fmt("x.y"), "x.y\n");
    }

    #[test]
    fn test_chained_callattr() {
        assert_eq!(fmt("x.foo(1)"), "x.foo(1)\n");
    }

    #[test]
    fn test_chained_index() {
        assert_eq!(fmt("x[0]"), "x[0]\n");
    }

    #[test]
    fn test_chained_multi() {
        assert_eq!(fmt("x.foo().bar(1).baz"), "x.foo().bar(1).baz\n");
    }

    // -- Collections --

    #[test]
    fn test_list_literal() {
        assert_eq!(fmt("list(1, 2, 3)"), "list(1, 2, 3)\n");
    }

    #[test]
    fn test_tuple_literal() {
        assert_eq!(fmt("tuple(1, 2)"), "tuple(1, 2)\n");
    }

    #[test]
    fn test_set_literal() {
        assert_eq!(fmt("set(1, 2, 3)"), "set(1, 2, 3)\n");
    }

    #[test]
    fn test_dict_literal() {
        assert_eq!(fmt("dict(x=1, y=2)"), "dict(x=1, y=2)\n");
    }

    // -- Struct --

    #[test]
    fn test_struct_simple() {
        let source = "struct Point {\n    x;\n    y;\n}";
        let result = fmt(source);
        assert!(result.contains("struct Point {"));
        assert!(result.contains("x;"));
        assert!(result.contains("y;"));
    }

    #[test]
    fn test_struct_blank_line_no_trailing_ws() {
        let source = "struct Foo {\n    x\n\n    bar(self) => { self.x }\n}";
        let result = fmt(source);
        // Blank line between fields and methods should not have trailing whitespace
        for line in result.lines() {
            assert!(
                line.trim_end() == line || !line.trim().is_empty(),
                "trailing whitespace on blank line: {result:?}"
            );
        }
        assert!(result.contains("\n\n"), "blank line should be preserved: {result:?}");
    }

    #[test]
    fn test_struct_with_method() {
        let source = "struct Point {\n    x;\n    y;\n    init(self) => {\n        self\n    }\n}";
        let result = fmt(source);
        assert!(result.contains("struct Point {"));
        assert!(result.contains("init(self) => {"));
    }

    #[test]
    fn test_struct_extends() {
        let source = "struct Child extends(Base) {\n    z;\n}";
        let result = fmt(source);
        assert!(result.contains("struct Child extends(Base) {"));
    }

    // -- Trait --

    #[test]
    fn test_trait_simple() {
        let source = "trait Printable {\n    print(self)\n}";
        let result = fmt(source);
        assert!(result.contains("trait Printable {"));
        assert!(result.contains("print(self)"));
    }

    // -- Phase 5: Source intent preservation --

    #[test]
    fn test_trailing_comma_forces_break() {
        // Trailing comma before ) forces multiline
        let source = "x = list(\n    a,\n    b,\n)";
        let result = fmt(source);
        assert!(
            result.contains("\n    a,\n"),
            "trailing comma should force multiline: {result}"
        );
    }

    #[test]
    fn test_no_trailing_comma_can_flatten() {
        // No trailing comma: can flatten if it fits
        assert_eq!(fmt("f(a, b)"), "f(a, b)\n");
    }

    #[test]
    fn test_no_trailing_comma_allows_inline_dict() {
        // Inline dict without trailing comma stays on one line
        assert_eq!(fmt("x = dict(a=1, b=2)"), "x = dict(a=1, b=2)\n");
        assert_eq!(fmt("f(dict(x=1))"), "f(dict(x=1))\n");
        // Inline list without trailing comma stays on one line
        assert_eq!(fmt("list(1, 2, 3)"), "list(1, 2, 3)\n");
    }

    #[test]
    fn test_source_multiline_preserved_without_trailing_comma() {
        // First arg on next line -> preserve multiline layout
        let result = fmt("list(\n    a,\n    b\n)");
        assert!(result.contains("\n    a,\n"), "multiline preserved: {result}");
    }

    #[test]
    fn test_source_inline_stays_inline() {
        // First arg on same line as ( -> keep inline
        assert_eq!(fmt("f(a, b)"), "f(a, b)\n");
        assert_eq!(fmt("list(1, 2, 3)"), "list(1, 2, 3)\n");
    }

    #[test]
    fn test_string_concat_multiline_preserved() {
        let source = "msg = \"hello \" +\n    \"world\"";
        let result = fmt(source);
        assert!(result.contains("+\n"), "string concat should stay multiline: {result}");
    }

    #[test]
    fn test_non_string_concat_can_flatten() {
        assert_eq!(fmt("x = a + b"), "x = a + b\n");
    }

    // -- Idempotence --

    #[test]
    fn test_idempotent_simple() {
        let source = "x = 1 + 2\n";
        let first = fmt(source);
        let second = fmt(&first);
        assert_eq!(first, second);
    }

    #[test]
    fn test_idempotent_if() {
        let source = "if x {\n    y = 1\n}\n";
        let first = fmt(source);
        let second = fmt(&first);
        assert_eq!(first, second);
    }

    // -- Error handling --

    #[test]
    fn test_try_finally_inline() {
        assert_eq!(fmt("try { 1 } finally { 2 }"), "try { 1 } finally { 2 }\n");
    }

    #[test]
    fn test_try_except_inline() {
        assert_eq!(
            fmt("try { 1 } except { _ => { 2 } }"),
            "try { 1 } except { _ => { 2 } }\n"
        );
    }

    #[test]
    fn test_try_except_typed() {
        assert_eq!(
            fmt("try { f() } except { e: TypeError => { g(e) } }"),
            "try { f() } except { e: TypeError => { g(e) } }\n"
        );
    }

    #[test]
    fn test_try_except_multi_types() {
        assert_eq!(
            fmt("try { f() } except { e: ValueError | KeyError => { g(e) } }"),
            "try { f() } except { e: ValueError | KeyError => { g(e) } }\n"
        );
    }

    #[test]
    fn test_raise_bare() {
        assert_eq!(fmt("raise"), "raise\n");
    }

    #[test]
    fn test_raise_expr() {
        assert_eq!(fmt("raise ValueError(\"msg\")"), "raise ValueError(\"msg\")\n");
    }

    #[test]
    fn test_try_except_finally_multiline() {
        let source = "try {\n    f()\n} except {\n    e: TypeError => { g(e) }\n    _ => { h() }\n} finally {\n    cleanup()\n}\n";
        let first = fmt(source);
        assert_eq!(first, source);
    }

    #[test]
    fn test_try_comment_between_except_finally() {
        // Comment between } except and finally on the same try_stmt
        // (scanner treats the newline before except/finally as continuation)
        let source = "try { 1 } except {\n    _ => { 2 }\n}  # handled\nfinally { 3 }\n";
        let first = fmt(source);
        assert_eq!(first, source);
    }

    #[test]
    fn test_try_trailing_comment() {
        let source = "try { 1 }  # after try\nexcept { _ => { 2 } }\n";
        let first = fmt(source);
        assert_eq!(first, source);
    }
}
