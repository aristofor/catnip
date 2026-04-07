// FILE: catnip_tools/src/pretty/convert_expr.rs
//! Expression and statement converters.

use tree_sitter::Node;

use super::convert::{build_body_doc, collect_body_items, convert, node_text};
use super::doc::{Arena, Doc};

/// Binary expression: `left op right`.
/// Operator stays at end of line in break mode.
/// String concat multiline: if both sides are strings and source is multiline, force break.
pub(crate) fn convert_binary(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let left = node.child(0).unwrap();
    let op = node.child(1).unwrap();
    let right = node.child(2).unwrap();

    let left_doc = convert(arena, left, source, indent);
    let op_doc = arena.text(node_text(op, source));
    let right_doc = convert(arena, right, source, indent);
    let sp = arena.space();

    // String concat multiline: both operands are strings on different lines -> force hardline
    let is_string_concat = is_string_node(&left) && is_string_node(&right);
    let source_multiline = left.end_position().row != right.start_position().row;

    if is_string_concat && source_multiline {
        let hl = arena.hardline();
        let right_break = arena.concat(hl, right_doc);
        let right_nested = arena.nest(indent, right_break);
        arena.concat_many(&[left_doc, sp, op_doc, right_nested])
    } else {
        let ln = arena.line();
        let right_break = arena.concat(ln, right_doc);
        let right_nested = arena.nest(indent, right_break);
        let doc = arena.concat_many(&[left_doc, sp, op_doc, right_nested]);
        arena.group(doc)
    }
}

/// Chained comparison: `a < b < c`.
pub(crate) fn convert_comparison(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();
    let count = node.child_count();

    for i in 0..count {
        let child = node.child(i as u32).unwrap();
        if i > 0 {
            let sp = arena.space();
            parts.push(sp);
        }
        if child.kind() == "comp_op" {
            parts.push(arena.text(node_text(child, source)));
        } else {
            parts.push(convert(arena, child, source, indent));
        }
    }

    arena.concat_many(&parts)
}

/// Unary expression: `-x`, `+x`, `~x` (no space).
pub(crate) fn convert_unary(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let op = node.child(0).unwrap();
    let operand = node.child(1).unwrap();

    let op_doc = arena.text(node_text(op, source));
    let operand_doc = convert(arena, operand, source, indent);
    arena.concat(op_doc, operand_doc)
}

/// `not expr` - no space before `(`, space otherwise.
pub(crate) fn convert_bool_not(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let operand = node.child(1).unwrap();
    let not_doc = arena.text("not");
    let operand_doc = convert(arena, operand, source, indent);
    if operand.kind() == "group" {
        arena.concat(not_doc, operand_doc)
    } else {
        let sp = arena.space();
        arena.concat_many(&[not_doc, sp, operand_doc])
    }
}

/// Assignment: `[decorators] lvalue = expr`.
pub(crate) fn convert_assignment(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();
    let count = node.child_count();

    for i in 0..count {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "=" => {
                let eq = arena.text(" = ");
                parts.push(eq);
            }
            "decorator" => {
                let deco_doc = arena.text(node_text(child, source));
                let hl = arena.hardline();
                parts.push(deco_doc);
                parts.push(hl);
            }
            _ => {
                parts.push(convert(arena, child, source, indent));
            }
        }
    }

    arena.concat_many(&parts)
}

/// Block: `{ statements }`.
/// Source-multiline blocks stay multiline (no flattening).
pub(crate) fn convert_block(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let items = collect_body_items(arena, node, source, indent);
    if items.is_empty() {
        return arena.text("{}");
    }
    let body = build_body_doc(arena, &items);

    // Preserve source intent: multiline blocks stay multiline
    let source_multiline = node.start_position().row != node.end_position().row;
    if source_multiline {
        let open = arena.text("{");
        let close = arena.text("}");
        let hl = arena.hardline();
        let inner = arena.concat(hl, body);
        let nested = arena.nest(indent, inner);
        let hl2 = arena.hardline();
        arena.concat_many(&[open, nested, hl2, close])
    } else {
        arena.bracket("{", body, "}", indent)
    }
}

/// Parenthesized expression: `(expr)`.
pub(crate) fn convert_paren(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    if let Some(expr) = node.named_child(0) {
        let expr_doc = convert(arena, expr, source, indent);
        arena.surround("(", expr_doc, ")")
    } else {
        arena.text("()")
    }
}

/// Function call: `func(args)`.
pub(crate) fn convert_call(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut func_doc = arena.nil();
    let mut args_doc = arena.text("()");

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "arguments" {
            args_doc = convert_arguments(arena, child, source, indent);
        } else if child.kind() != "comment" {
            func_doc = convert(arena, child, source, indent);
        }
    }

    arena.concat(func_doc, args_doc)
}

struct ArgItem {
    doc: Doc,
    end_row: usize,
    /// Trailing comment text (e.g. "# doublon"), already includes the comma before it.
    trailing_comment: Option<Doc>,
}

/// Recursively collect argument expressions from args/kwargs/args_kwargs.
/// Trailing comments are tracked so the comma can be placed before them.
fn collect_args(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Vec<ArgItem> {
    collect_args_inner(arena, node, source, indent)
}

fn collect_args_inner(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Vec<ArgItem> {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "args" | "kwargs" | "args_kwargs" => {
                items.extend(collect_args_inner(arena, child, source, indent));
            }
            "comment" => {
                let start_row = child.start_position().row;
                if let Some(last) = items.last_mut() {
                    if start_row == last.end_row {
                        last.trailing_comment = Some(arena.text(node_text(child, source).trim_end()));
                        last.end_row = child.end_position().row;
                        continue;
                    }
                }
                // Standalone comment -- skip in arg lists
            }
            _ => {
                items.push(ArgItem {
                    doc: convert(arena, child, source, indent),
                    end_row: child.end_position().row,
                    trailing_comment: None,
                });
            }
        }
    }
    items
}

/// Build multiline `(items)` from ArgItems, preserving trailing comments.
fn build_args_parens(arena: &mut Arena, items: &[ArgItem], indent: i32, trailing_comma: bool) -> Doc {
    let comma_hl = arena.text(",");
    let hl = arena.hardline();
    let sep = arena.concat(comma_hl, hl);
    let args_doc = build_args_doc(arena, items, sep);
    let with_trailing = append_last_comment(arena, args_doc, items, trailing_comma);
    let hl1 = arena.hardline();
    let inner = arena.concat(hl1, with_trailing);
    let nested = arena.nest(indent, inner);
    let hl2 = arena.hardline();
    let open = arena.text("(");
    let close = arena.text(")");
    arena.concat_many(&[open, nested, hl2, close])
}

/// Build a Doc from arg items, placing commas before trailing comments.
fn build_args_doc(arena: &mut Arena, items: &[ArgItem], sep: Doc) -> Doc {
    if items.is_empty() {
        return arena.nil();
    }
    let mut acc = items[0].doc;
    // First item's trailing comment is handled in the loop below

    for i in 1..items.len() {
        // Previous item: add comma + trailing comment if present, then separator
        if let Some(tc) = items[i - 1].trailing_comment {
            let comma = arena.text(",");
            let sp = arena.text("  ");
            acc = arena.concat_many(&[acc, comma, sp, tc]);
            // Then line break (sep without comma since we already added one)
            let hl = arena.hardline();
            acc = arena.concat(acc, hl);
        } else {
            acc = arena.concat(acc, sep);
        }
        acc = arena.concat(acc, items[i].doc);
    }
    acc
}

/// Append trailing comment of the last item, with comma before it if needed.
fn append_last_comment(arena: &mut Arena, doc: Doc, items: &[ArgItem], trailing_comma: bool) -> Doc {
    if let Some(tc) = items.last().and_then(|it| it.trailing_comment) {
        if trailing_comma {
            let comma = arena.text(",");
            let sp = arena.text("  ");
            arena.concat_many(&[doc, comma, sp, tc])
        } else {
            let sp = arena.text("  ");
            arena.concat_many(&[doc, sp, tc])
        }
    } else if trailing_comma {
        let comma = arena.text(",");
        arena.concat(doc, comma)
    } else {
        doc
    }
}

/// Arguments list: `(a, b, c)`.
/// Layout follows source intent:
/// - Trailing comma → force multiline
/// - First arg on same line as `(` → keep inline (breaks happen inside inner groups)
/// - First arg on next line → force multiline
pub(crate) fn convert_arguments(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut items = collect_args(arena, node, source, indent);

    if items.is_empty() {
        return arena.text("()");
    }

    recover_trailing_comment(arena, node, source, &mut items);
    let has_comments = items.iter().any(|it| it.trailing_comment.is_some());
    let trailing_comma = has_trailing_comma(node);
    let first_arg_inline = first_arg_on_same_line(node);

    if trailing_comma || has_comments || !first_arg_inline {
        // Multiline: each arg on its own line
        build_args_parens(arena, &items, indent, trailing_comma)
    } else {
        // Inline: fill mode -- pack items, each separator decides independently.
        // Append ")" to the last item so the fill accounts for the closing paren.
        let mut plain: Vec<Doc> = items.iter().map(|it| it.doc).collect();
        let close = arena.text(")");
        if let Some(last) = plain.last_mut() {
            *last = arena.concat(*last, close);
        }
        let sep_flat = arena.text(", ");
        let hl = arena.hardline();
        let args_doc = arena.fill_sep(&plain, sep_flat, hl);
        let nested = arena.nest(indent, args_doc);
        let open = arena.text("(");
        arena.concat(open, nested)
    }
}

/// Keyword argument: `key=value` (no spaces).
pub(crate) fn convert_kwarg(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let key = node.child_by_field_name("key").unwrap();
    let value = node.child_by_field_name("value").unwrap();

    let key_doc = arena.text(node_text(key, source));
    let eq = arena.text("=");
    let val_doc = convert(arena, value, source, indent);
    arena.concat_many(&[key_doc, eq, val_doc])
}

/// Return statement: `return [expr]`.
pub(crate) fn convert_return(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    if let Some(expr) = node.named_child(0) {
        let ret = arena.text("return");
        let sp = arena.space();
        let expr_doc = convert(arena, expr, source, indent);
        arena.concat_many(&[ret, sp, expr_doc])
    } else {
        arena.text("return")
    }
}

pub(crate) fn convert_raise(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    if let Some(expr) = node.child_by_field_name("value") {
        let kw = arena.text("raise");
        let sp = arena.space();
        let expr_doc = convert(arena, expr, source, indent);
        arena.concat_many(&[kw, sp, expr_doc])
    } else {
        arena.text("raise")
    }
}

// -- Chained member access --

fn is_member(kind: &str) -> bool {
    matches!(
        kind,
        "getattr" | "callattr" | "call_member" | "broadcast" | "index" | "fullslice"
    )
}

/// `base.attr.method(args)[idx]` - source-aware method chain.
/// Members on the same source line as their predecessor are concatenated directly.
/// Members on a different source line get a hardline break.
pub(crate) fn convert_chained(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut base_doc = arena.nil();
    let mut base_end_row: usize = 0;
    let mut members: Vec<(Doc, usize, usize)> = Vec::new(); // (doc, start_row, end_row)

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if is_member(child.kind()) {
            members.push((
                convert_member(arena, child, source, indent),
                child.start_position().row,
                child.end_position().row,
            ));
        } else if child.kind() != "comment" {
            base_doc = convert(arena, child, source, indent);
            base_end_row = child.end_position().row;
        }
    }

    if members.is_empty() {
        return base_doc;
    }

    // Check if any member starts on a different line than the previous
    let any_break = members.iter().any(|(_, sr, _)| *sr != base_end_row);

    if any_break {
        // Source had line breaks: preserve them with nesting
        let mut rest_parts = Vec::new();
        let mut prev_row = base_end_row;
        for (doc, start_row, end_row) in &members {
            if *start_row != prev_row {
                let hl = arena.hardline();
                rest_parts.push(hl);
            }
            rest_parts.push(*doc);
            prev_row = *end_row;
        }
        let rest = arena.concat_many(&rest_parts);
        let nested = arena.nest(indent, rest);
        arena.concat(base_doc, nested)
    } else {
        // All on one line: direct concatenation, no break points
        let mut parts = vec![base_doc];
        for (doc, _, _) in &members {
            parts.push(*doc);
        }
        arena.concat_many(&parts)
    }
}

fn convert_member(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    match node.kind() {
        "getattr" => {
            let attr = node.child_by_field_name("attribute").unwrap();
            let s = format!(".{}", node_text(attr, source));
            arena.text(s)
        }
        "callattr" => {
            let method = node.child_by_field_name("method").unwrap();
            let s = format!(".{}", node_text(method, source));
            let name_doc = arena.text(s);
            let mut args_doc = arena.text("()");
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "arguments" {
                    args_doc = convert_arguments(arena, child, source, indent);
                }
            }
            arena.concat(name_doc, args_doc)
        }
        "call_member" => {
            if let Some(args) = node.named_child(0) {
                if args.kind() == "arguments" {
                    return convert_arguments(arena, args, source, indent);
                }
            }
            // call_member might itself act as arguments
            convert_arguments(arena, node, source, indent)
        }
        "index" => {
            if let Some(expr) = node.named_child(0) {
                let expr_doc = convert(arena, expr, source, indent);
                arena.surround("[", expr_doc, "]")
            } else {
                arena.text(node_text(node, source))
            }
        }
        "broadcast" | "fullslice" => {
            if let Some(inner) = node.named_child(0) {
                let inner_doc = convert(arena, inner, source, indent);
                arena.surround(".[", inner_doc, "]")
            } else {
                arena.text(node_text(node, source))
            }
        }
        _ => arena.text(node_text(node, source)),
    }
}

// -- Collections --

/// Generic collection: `keyword(item1, item2, ...)`.
fn convert_collection_items(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Vec<ArgItem> {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "comment" {
            let start_row = child.start_position().row;
            if let Some(last) = items.last_mut() {
                let last: &mut ArgItem = last;
                if start_row == last.end_row {
                    last.trailing_comment = Some(arena.text(node_text(child, source).trim_end()));
                    last.end_row = child.end_position().row;
                    continue;
                }
            }
            continue;
        }
        items.push(ArgItem {
            doc: convert(arena, child, source, indent),
            end_row: child.end_position().row,
            trailing_comment: None,
        });
    }
    items
}

/// `list(a, b, c)`, `tuple(a, b)`, `set(a, b)`.
pub(crate) fn convert_collection(arena: &mut Arena, keyword: &str, node: Node, source: &[u8], indent: i32) -> Doc {
    let kw = arena.text(keyword);

    let (mut items, trailing, inline) = if let Some(items_node) = node.named_child(0) {
        let first_inline = find_first_arg_row(items_node).is_none_or(|row| row == node.start_position().row);
        (
            convert_collection_items(arena, items_node, source, indent),
            has_trailing_comma(items_node),
            first_inline,
        )
    } else {
        (Vec::new(), false, true)
    };

    if items.is_empty() {
        let empty = arena.text("()");
        return arena.concat(kw, empty);
    }

    // Recover trailing comment from parent node (tree-sitter extras)
    recover_trailing_comment(arena, node, source, &mut items);

    let has_comments = items.iter().any(|it| it.trailing_comment.is_some());
    let parens = if has_comments || trailing || !inline {
        build_args_parens(arena, &items, indent, trailing)
    } else {
        let plain: Vec<Doc> = items.iter().map(|it| it.doc).collect();
        format_parens_list(arena, &plain, indent, trailing)
    };
    arena.concat(kw, parens)
}

/// `dict((k, v), name=val, **spread)` or `dict(iterable)`.
pub(crate) fn convert_dict(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let kw = arena.text("dict");

    // dict(iterable) - single expression argument
    if let Some(child) = node.named_child(0) {
        if child.kind() == "dict_from_iterable" {
            let inner = if let Some(c) = child.named_child(0) { c } else { child };
            let expr = super::convert::convert(arena, inner, source, indent);
            let open = arena.text("(");
            let close = arena.text(")");
            let body = arena.concat(open, expr);
            let body = arena.concat(body, close);
            return arena.concat(kw, body);
        }
    }

    let (mut items, trailing, inline) = if let Some(items_node) = node.named_child(0) {
        let first_inline = find_first_arg_row(items_node).is_none_or(|row| row == node.start_position().row);
        (
            convert_dict_items(arena, items_node, source, indent),
            has_trailing_comma(items_node),
            first_inline,
        )
    } else {
        (Vec::new(), false, true)
    };

    if items.is_empty() {
        let empty = arena.text("()");
        return arena.concat(kw, empty);
    }

    recover_trailing_comment(arena, node, source, &mut items);
    let has_comments = items.iter().any(|it| it.trailing_comment.is_some());
    let parens = if has_comments || trailing || !inline {
        build_args_parens(arena, &items, indent, trailing)
    } else {
        let plain: Vec<Doc> = items.iter().map(|it| it.doc).collect();
        format_parens_list(arena, &plain, indent, trailing)
    };
    arena.concat(kw, parens)
}

/// Force multiline `(items)` with hardlines (for plain Doc items without comments).
fn format_parens_list_forced(arena: &mut Arena, items: &[Doc], indent: i32, trailing_comma: bool) -> Doc {
    let comma_hl = arena.text(",");
    let hl = arena.hardline();
    let sep = arena.concat(comma_hl, hl);
    let items_doc = arena.intersperse(items, sep);
    let with_trailing = if trailing_comma {
        let tc = arena.text(",");
        arena.concat(items_doc, tc)
    } else {
        items_doc
    };
    let hl1 = arena.hardline();
    let inner = arena.concat(hl1, with_trailing);
    let nested = arena.nest(indent, inner);
    let hl2 = arena.hardline();
    let open = arena.text("(");
    let close = arena.text(")");
    arena.concat_many(&[open, nested, hl2, close])
}

/// Format `(item1, item2, ...)` -- fill mode (pack items, break per separator).
fn format_parens_list(arena: &mut Arena, items: &[Doc], indent: i32, trailing_comma: bool) -> Doc {
    if trailing_comma {
        format_parens_list_forced(arena, items, indent, true)
    } else {
        let mut items_vec = items.to_vec();
        let close = arena.text(")");
        if let Some(last) = items_vec.last_mut() {
            *last = arena.concat(*last, close);
        }
        let sep_flat = arena.text(", ");
        let hl = arena.hardline();
        let items_doc = arena.fill_sep(&items_vec, sep_flat, hl);
        let nested = arena.nest(indent, items_doc);
        let open = arena.text("(");
        arena.concat(open, nested)
    }
}

fn convert_dict_items(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Vec<ArgItem> {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "comment" {
            let start_row = child.start_position().row;
            if let Some(last) = items.last_mut() {
                let last: &mut ArgItem = last;
                if start_row == last.end_row {
                    last.trailing_comment = Some(arena.text(node_text(child, source).trim_end()));
                    last.end_row = child.end_position().row;
                    continue;
                }
            }
            continue;
        }
        let doc = match child.kind() {
            "dict_pair" => convert_dict_pair(arena, child, source, indent),
            "dict_kwarg" => convert_dict_kwarg(arena, child, source, indent),
            "dict_spread" => convert_dict_spread(arena, child, source, indent),
            _ => convert(arena, child, source, indent),
        };
        items.push(ArgItem {
            doc,
            end_row: child.end_position().row,
            trailing_comment: None,
        });
    }
    items
}

fn convert_dict_pair(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let key = node.child_by_field_name("key").unwrap();
    let value = node.child_by_field_name("value").unwrap();
    let key_doc = convert(arena, key, source, indent);
    let value_doc = convert(arena, value, source, indent);
    let sep = arena.text(", ");
    let open = arena.text("(");
    let close = arena.text(")");
    let inner = arena.concat_many(&[key_doc, sep, value_doc]);
    arena.concat_many(&[open, inner, close])
}

fn convert_dict_kwarg(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let key = node.child_by_field_name("key").unwrap();
    let value = node.child_by_field_name("value").unwrap();
    let key_doc = arena.text(node_text(key, source));
    let eq = arena.text("=");
    let val_doc = convert(arena, value, source, indent);
    arena.concat_many(&[key_doc, eq, val_doc])
}

fn convert_dict_spread(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let value = node.child_by_field_name("value").unwrap();
    let stars = arena.text("**");
    let val_doc = convert(arena, value, source, indent);
    arena.concat(stars, val_doc)
}

// -- Bracket collections --

/// `[a, b, c]`
pub(crate) fn convert_bracket_list(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let (mut items, trailing, inline) = if let Some(items_node) = node.named_child(0) {
        let first_inline = find_first_arg_row(items_node).is_none_or(|row| row == node.start_position().row);
        (
            convert_collection_items(arena, items_node, source, indent),
            has_trailing_comma(items_node),
            first_inline,
        )
    } else {
        (Vec::new(), false, true)
    };

    if items.is_empty() {
        return arena.text("[]");
    }

    recover_trailing_comment(arena, node, source, &mut items);

    let has_comments = items.iter().any(|it| it.trailing_comment.is_some());
    if has_comments || trailing || !inline {
        format_bracket_list_forced(arena, &items, indent, trailing)
    } else {
        let plain: Vec<Doc> = items.iter().map(|it| it.doc).collect();
        format_bracket_list(arena, &plain, indent)
    }
}

/// `{"key": value, ...}`
pub(crate) fn convert_bracket_dict(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let (mut items, trailing, inline) = if let Some(items_node) = node.named_child(0) {
        let first_inline = find_first_arg_row(items_node).is_none_or(|row| row == node.start_position().row);
        (
            convert_bracket_dict_items(arena, items_node, source, indent),
            has_trailing_comma(items_node),
            first_inline,
        )
    } else {
        return arena.text("{}");
    };

    if items.is_empty() {
        return arena.text("{}");
    }

    recover_trailing_comment(arena, node, source, &mut items);

    let has_comments = items.iter().any(|it| it.trailing_comment.is_some());
    if has_comments || trailing || !inline {
        format_brace_list_forced(arena, &items, indent, trailing)
    } else {
        let plain: Vec<Doc> = items.iter().map(|it| it.doc).collect();
        format_brace_list(arena, &plain, indent)
    }
}

fn convert_bracket_dict_items(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Vec<ArgItem> {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "comment" {
            let start_row = child.start_position().row;
            if let Some(last) = items.last_mut() {
                let last: &mut ArgItem = last;
                if start_row == last.end_row {
                    last.trailing_comment = Some(arena.text(node_text(child, source).trim_end()));
                    last.end_row = child.end_position().row;
                    continue;
                }
            }
            continue;
        }
        let doc = match child.kind() {
            "colon_pair" => convert_colon_pair(arena, child, source, indent),
            "dict_spread" => convert_dict_spread(arena, child, source, indent),
            _ => convert(arena, child, source, indent),
        };
        items.push(ArgItem {
            doc,
            end_row: child.end_position().row,
            trailing_comment: None,
        });
    }
    items
}

fn convert_colon_pair(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let key = node.child_by_field_name("key").unwrap();
    let value = node.child_by_field_name("value").unwrap();
    let key_doc = convert(arena, key, source, indent);
    let value_doc = convert(arena, value, source, indent);
    let sep = arena.text(": ");
    arena.concat_many(&[key_doc, sep, value_doc])
}

fn format_bracket_list(arena: &mut Arena, items: &[Doc], indent: i32) -> Doc {
    let mut items_vec = items.to_vec();
    let close = arena.text("]");
    if let Some(last) = items_vec.last_mut() {
        *last = arena.concat(*last, close);
    }
    let sep_flat = arena.text(", ");
    let hl = arena.hardline();
    let items_doc = arena.fill_sep(&items_vec, sep_flat, hl);
    let nested = arena.nest(indent, items_doc);
    let open = arena.text("[");
    arena.concat(open, nested)
}

fn format_bracket_list_forced(arena: &mut Arena, items: &[ArgItem], indent: i32, trailing_comma: bool) -> Doc {
    let comma_hl = arena.text(",");
    let hl = arena.hardline();
    let sep = arena.concat(comma_hl, hl);
    let args_doc = build_args_doc(arena, items, sep);
    let with_trailing = append_last_comment(arena, args_doc, items, trailing_comma);
    let hl1 = arena.hardline();
    let inner = arena.concat(hl1, with_trailing);
    let nested = arena.nest(indent, inner);
    let hl2 = arena.hardline();
    let open = arena.text("[");
    let close = arena.text("]");
    arena.concat_many(&[open, nested, hl2, close])
}

fn format_brace_list(arena: &mut Arena, items: &[Doc], indent: i32) -> Doc {
    let mut items_vec = items.to_vec();
    let close = arena.text("}");
    if let Some(last) = items_vec.last_mut() {
        *last = arena.concat(*last, close);
    }
    let sep_flat = arena.text(", ");
    let hl = arena.hardline();
    let items_doc = arena.fill_sep(&items_vec, sep_flat, hl);
    let nested = arena.nest(indent, items_doc);
    let open = arena.text("{");
    arena.concat(open, nested)
}

fn format_brace_list_forced(arena: &mut Arena, items: &[ArgItem], indent: i32, trailing_comma: bool) -> Doc {
    let comma_hl = arena.text(",");
    let hl = arena.hardline();
    let sep = arena.concat(comma_hl, hl);
    let args_doc = build_args_doc(arena, items, sep);
    let with_trailing = append_last_comment(arena, args_doc, items, trailing_comma);
    let hl1 = arena.hardline();
    let inner = arena.concat(hl1, with_trailing);
    let nested = arena.nest(indent, inner);
    let hl2 = arena.hardline();
    let open = arena.text("{");
    let close = arena.text("}");
    arena.concat_many(&[open, nested, hl2, close])
}

// -- ND operators --

/// `~~(seed, lambda)` or `~~ lambda`.
pub(crate) fn convert_nd_recursion(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    if let Some(child) = node.named_child(0) {
        let inner = convert(arena, child, source, indent);
        let prefix = if starts_with_paren(child) {
            arena.text("~~")
        } else {
            arena.text("~~ ")
        };
        arena.concat(prefix, inner)
    } else {
        arena.text("~~")
    }
}

/// `~> abs`, `~>(data, f)`.
pub(crate) fn convert_nd_map(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    if let Some(child) = node.named_child(0) {
        let inner = convert(arena, child, source, indent);
        let prefix = if starts_with_paren(child) {
            arena.text("~>")
        } else {
            arena.text("~> ")
        };
        arena.concat(prefix, inner)
    } else {
        arena.text("~>")
    }
}

fn starts_with_paren(node: Node) -> bool {
    matches!(node.kind(), "group" | "arguments") || node.child(0).is_some_and(|c| c.kind() == "(")
}

// -- Unpacking --

/// `(a, b, c)` destructuring.
pub(crate) fn convert_unpack_tuple(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    if let Some(items) = node.named_child(0) {
        let items_doc = convert_unpack_items(arena, items, source, indent);
        arena.surround("(", items_doc, ")")
    } else {
        arena.text("()")
    }
}

fn convert_unpack_items(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        items.push(convert(arena, child, source, indent));
    }
    let sep = arena.text(", ");
    arena.intersperse(&items, sep)
}

/// `a, b, c` sequence destructuring (no parens).
pub(crate) fn convert_unpack_sequence(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        items.push(convert(arena, child, source, indent));
    }
    let sep = arena.text(", ");
    arena.intersperse(&items, sep)
}

// -- Helpers --

/// Find the source row of the first real argument (unwrapping wrapper nodes).
fn find_first_arg_row(node: Node) -> Option<usize> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "args" | "kwargs" | "args_kwargs" | "collection_items" | "dict_items" => {
                return find_first_arg_row(child);
            }
            "comment" => continue,
            _ => return Some(child.start_position().row),
        }
    }
    None
}

/// Recover a trailing comment from the parent node (tree-sitter extras).
/// Comments on the same line as the last item but placed as children of the parent
/// (not collection_items/args) are captured here.
fn recover_trailing_comment(arena: &mut Arena, parent: Node, source: &[u8], items: &mut [ArgItem]) {
    if let Some(last) = items.last_mut() {
        if last.trailing_comment.is_some() {
            return;
        }
        let mut cursor = parent.walk();
        for child in parent.named_children(&mut cursor) {
            if child.kind() == "comment" && child.start_position().row == last.end_row {
                last.trailing_comment = Some(arena.text(node_text(child, source).trim_end()));
                last.end_row = child.end_position().row;
                return;
            }
        }
    }
}

/// Check if the first real argument starts on the same line as the opening `(`.
fn first_arg_on_same_line(node: Node) -> bool {
    find_first_arg_row(node).is_none_or(|row| row == node.start_position().row)
}

/// Check if the last non-whitespace token before `)` is a comma.
/// Works on arguments, collection_items, and dict_items by scanning children.
fn has_trailing_comma(node: Node) -> bool {
    let count = node.child_count();
    if count < 2 {
        return false;
    }
    // Walk backwards from the closing paren to find last meaningful token
    let mut i = count as i32 - 1;
    while i >= 0 {
        let child = node.child(i as u32).unwrap();
        let kind = child.kind();
        if kind == ")" || kind == "]" || kind == "comment" {
            // skip the closing delimiter and comments
            i -= 1;
            continue;
        }
        // Check if this is a comma or if the inner args/kwargs/collection_items has trailing comma
        if kind == "," {
            return true;
        }
        // Recurse into args/kwargs/args_kwargs/collection_items/dict_items
        if matches!(
            kind,
            "args" | "kwargs" | "args_kwargs" | "collection_items" | "dict_items"
        ) {
            return has_trailing_comma(child);
        }
        return false;
    }
    false
}

/// Check if a node is a string literal, unwrapping transparent wrappers.
fn is_string_node(node: &Node) -> bool {
    match node.kind() {
        "string" | "fstring" | "bstring" => true,
        "literal" => node
            .named_child(0)
            .is_some_and(|c| matches!(c.kind(), "string" | "fstring" | "bstring")),
        _ => false,
    }
}
