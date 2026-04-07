// FILE: catnip_tools/src/pretty/convert_stmt.rs
//! Control flow and statement converters.

use tree_sitter::Node;

use super::convert::{BodyItem, convert, node_text};
use super::doc::{Arena, Doc};

/// `if cond { } elif cond { } else { }`
pub(crate) fn convert_if(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();

    // "if" condition consequence
    let kw = arena.text("if");
    let sp = arena.space();
    let cond = node.child_by_field_name("condition").unwrap();
    let cond_doc = convert(arena, cond, source, indent);
    let consequence = node.child_by_field_name("consequence").unwrap();
    let block_doc = convert(arena, consequence, source, indent);
    parts.push(kw);
    parts.push(sp);
    parts.push(cond_doc);
    let sp2 = arena.space();
    parts.push(sp2);
    parts.push(block_doc);

    // elif / else clauses: preserve source choice of `} else` vs `}\nelse`
    let mut prev_end_row = consequence.end_position().row;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "elif_clause" => {
                let join = if child.start_position().row == prev_end_row {
                    arena.space()
                } else {
                    arena.hardline()
                };
                parts.push(join);
                parts.push(convert_elif(arena, child, source, indent));
                prev_end_row = child.end_position().row;
            }
            "else_clause" => {
                let join = if child.start_position().row == prev_end_row {
                    arena.space()
                } else {
                    arena.hardline()
                };
                parts.push(join);
                parts.push(convert_else(arena, child, source, indent));
            }
            _ => {}
        }
    }

    arena.concat_many(&parts)
}

fn convert_elif(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let kw = arena.text("elif");
    let sp = arena.space();
    let cond = node.child_by_field_name("condition").unwrap();
    let cond_doc = convert(arena, cond, source, indent);
    let consequence = node.child_by_field_name("consequence").unwrap();
    let block_doc = convert(arena, consequence, source, indent);
    let sp2 = arena.space();
    arena.concat_many(&[kw, sp, cond_doc, sp2, block_doc])
}

fn convert_else(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let kw = arena.text("else");
    let sp = arena.space();
    let body = node.child_by_field_name("body").unwrap();
    let block_doc = convert(arena, body, source, indent);
    arena.concat_many(&[kw, sp, block_doc])
}

/// `while cond { body }`
pub(crate) fn convert_while(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    // children: "while" (anon), expression (named), block (named)
    let mut expr_doc = arena.nil();
    let mut block_doc = arena.text("{}");

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "block" {
            block_doc = convert(arena, child, source, indent);
        } else if child.kind() != "comment" {
            expr_doc = convert(arena, child, source, indent);
        }
    }

    let kw = arena.text("while");
    let sp = arena.space();
    let sp2 = arena.space();
    arena.concat_many(&[kw, sp, expr_doc, sp2, block_doc])
}

/// `for target in iterable { body }`
pub(crate) fn convert_for(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    // children: "for" (anon), unpack_target (named), "in" (anon), expression (named), block (named)
    let mut target_doc = arena.nil();
    let mut iter_doc = arena.nil();
    let mut block_doc = arena.text("{}");

    let mut named_idx = 0;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "comment" {
            continue;
        }
        match named_idx {
            0 => target_doc = convert(arena, child, source, indent),
            1 => iter_doc = convert(arena, child, source, indent),
            2 => block_doc = convert(arena, child, source, indent),
            _ => {}
        }
        named_idx += 1;
    }

    let for_kw = arena.text("for");
    let sp1 = arena.space();
    let in_kw = arena.text(" in ");
    let sp2 = arena.space();
    arena.concat_many(&[for_kw, sp1, target_doc, in_kw, iter_doc, sp2, block_doc])
}

/// `match value { cases }`
pub(crate) fn convert_match(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let value = node.child_by_field_name("value").unwrap();
    let value_doc = convert(arena, value, source, indent);

    // Collect match_case children as body items for proper source_line tracking
    let mut case_items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "match_case" {
            case_items.push(BodyItem {
                doc: convert_match_case(arena, child, source, indent),
                start_row: child.start_position().row,
                end_row: child.end_position().row,
                semi_before: false,
            });
        }
    }

    let cases_doc = super::convert::build_body_doc(arena, &case_items);

    // Build { body } with source_line tracking for arrow alignment.
    // Preserve source multiline: if source is multiline, force hardlines.
    let source_multiline = node.start_position().row != node.end_position().row;
    let open = arena.text("{");
    let close = arena.text("}");

    let body = if source_multiline && !case_items.is_empty() {
        let hl = arena.hardline();
        let hl = if let Some(first) = case_items.first() {
            arena.source_line(first.start_row, hl)
        } else {
            hl
        };
        let inner = arena.concat(hl, cases_doc);
        let nested = arena.nest(indent, inner);
        let hl2 = arena.hardline();
        arena.concat_many(&[open, nested, hl2, close])
    } else {
        let ln = arena.line();
        let ln = if let Some(first) = case_items.first() {
            arena.source_line(first.start_row, ln)
        } else {
            ln
        };
        let inner = arena.concat(ln, cases_doc);
        let nested = arena.nest(indent, inner);
        let ln2 = arena.line();
        let body = arena.concat_many(&[open, nested, ln2, close]);
        arena.group(body)
    };

    let kw = arena.text("match");
    let sp = arena.space();
    let sp2 = arena.space();
    arena.concat_many(&[kw, sp, value_doc, sp2, body])
}

fn convert_match_case(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();

    let count = node.child_count();
    for i in 0..count {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "pattern" => {
                parts.push(convert_pattern(arena, child, source, indent));
            }
            "if" => {
                // Guard: "if" keyword + guard expression
                let guard = node.child_by_field_name("guard").unwrap();
                let if_kw = arena.text(" if ");
                let guard_doc = convert(arena, guard, source, indent);
                parts.push(if_kw);
                parts.push(guard_doc);
            }
            "=>" => {
                let arrow = arena.text(" => ");
                parts.push(arrow);
            }
            "block" => {
                parts.push(convert(arena, child, source, indent));
            }
            _ => {}
        }
    }

    arena.concat_many(&parts)
}

fn convert_pattern(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    // pattern -> pattern_or -> _pattern_primary (| _pattern_primary)*
    if let Some(child) = node.named_child(0) {
        convert_pattern_inner(arena, child, source, indent)
    } else {
        arena.text(node_text(node, source))
    }
}

fn convert_pattern_inner(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    match node.kind() {
        "pattern_or" => {
            // children: pattern1, "|", pattern2, "|", pattern3, ...
            let mut parts = Vec::new();
            let count = node.child_count();
            for i in 0..count {
                let child = node.child(i as u32).unwrap();
                if child.kind() == "|" {
                    let pipe = arena.text(" | ");
                    parts.push(pipe);
                } else {
                    parts.push(convert_pattern_inner(arena, child, source, indent));
                }
            }
            arena.concat_many(&parts)
        }
        "pattern_literal" | "pattern_var" | "pattern_wildcard" => {
            // Transparent: convert inner child or text
            if let Some(child) = node.named_child(0) {
                convert(arena, child, source, indent)
            } else {
                arena.text(node_text(node, source))
            }
        }
        "pattern_struct" => {
            let name = node.child_by_field_name("struct_name").unwrap();
            let name_doc = arena.text(node_text(name, source));
            // Collect field identifiers
            let mut fields = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "identifier" && child.id() != name.id() {
                    fields.push(arena.text(node_text(child, source)));
                }
            }
            let sep = arena.text(", ");
            let fields_doc = arena.intersperse(&fields, sep);
            let open = arena.text("{");
            let close = arena.text("}");
            arena.concat_many(&[name_doc, open, fields_doc, close])
        }
        "pattern_tuple" => {
            // (items)
            if let Some(items) = node.named_child(0) {
                let items_doc = convert_pattern_items(arena, items, source, indent);
                arena.surround("(", items_doc, ")")
            } else {
                arena.text("()")
            }
        }
        "pattern_star" => {
            // *name
            let star = arena.text("*");
            if let Some(id) = node.named_child(0) {
                let id_doc = arena.text(node_text(id, source));
                arena.concat(star, id_doc)
            } else {
                star
            }
        }
        _ => arena.text(node_text(node, source)),
    }
}

fn convert_pattern_items(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        items.push(convert_pattern_inner(arena, child, source, indent));
    }
    let sep = arena.text(", ");
    arena.intersperse(&items, sep)
}

/// `pragma("key", value)`
pub(crate) fn convert_pragma(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut args = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "pragma_arg" {
            args.push(convert_pragma_arg(arena, child, source, indent));
        }
    }

    let kw = arena.text("pragma");
    let open = arena.text("(");
    let sep = arena.text(", ");
    let args_doc = arena.intersperse(&args, sep);
    let close = arena.text(")");
    arena.concat_many(&[kw, open, args_doc, close])
}

fn convert_pragma_arg(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    if let Some(child) = node.named_child(0) {
        match child.kind() {
            "pragma_qualified" => {
                let ns = child.child_by_field_name("namespace").unwrap();
                let attr = child.child_by_field_name("attr").unwrap();
                let ns_doc = arena.text(node_text(ns, source));
                let dot = arena.text(".");
                let attr_doc = arena.text(node_text(attr, source));
                arena.concat_many(&[ns_doc, dot, attr_doc])
            }
            _ => convert(arena, child, source, indent),
        }
    } else {
        arena.text(node_text(node, source))
    }
}

// -- Error handling ----------------------------------------------------------

/// `try { body } except { clauses } finally { body }`
pub(crate) fn convert_try(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();

    // "try" body
    let kw = arena.text("try");
    let sp = arena.space();
    let body = node.child_by_field_name("body").unwrap();
    let body_doc = convert(arena, body, source, indent);
    parts.push(kw);
    parts.push(sp);
    parts.push(body_doc);

    // except block, finally clause, and comments: preserve source layout
    let mut prev_end_row = body.end_position().row;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "except_block" => {
                let join = if child.start_position().row == prev_end_row {
                    arena.space()
                } else {
                    arena.hardline()
                };
                parts.push(join);
                parts.push(convert_except_block(arena, child, source, indent));
                prev_end_row = child.end_position().row;
            }
            "finally_clause" => {
                let join = if child.start_position().row == prev_end_row {
                    arena.space()
                } else {
                    arena.hardline()
                };
                parts.push(join);
                parts.push(convert_finally(arena, child, source, indent));
                prev_end_row = child.end_position().row;
            }
            "comment" => {
                let comment_doc = arena.text(node_text(child, source).trim_end());
                if child.start_position().row == prev_end_row {
                    // Trailing comment on same line
                    let sp = arena.text("  ");
                    parts.push(sp);
                    parts.push(comment_doc);
                } else {
                    parts.push(arena.hardline());
                    parts.push(comment_doc);
                }
                prev_end_row = child.end_position().row;
            }
            _ => {}
        }
    }

    arena.concat_many(&parts)
}

/// `except { clause1  clause2 ... }`
fn convert_except_block(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let kw = arena.text("except");
    let sp = arena.space();

    // Collect except_clause children (same pattern as match cases)
    let mut clause_items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "except_clause" {
            clause_items.push(BodyItem {
                doc: convert_except_clause(arena, child, source, indent),
                start_row: child.start_position().row,
                end_row: child.end_position().row,
                semi_before: false,
            });
        }
    }

    let clauses_doc = super::convert::build_body_doc(arena, &clause_items);

    let source_multiline = node.start_position().row != node.end_position().row;
    let open = arena.text("{");
    let close = arena.text("}");

    let body = if source_multiline && !clause_items.is_empty() {
        let hl = arena.hardline();
        let hl = if let Some(first) = clause_items.first() {
            arena.source_line(first.start_row, hl)
        } else {
            hl
        };
        let inner = arena.concat(hl, clauses_doc);
        let nested = arena.nest(indent, inner);
        let hl2 = arena.hardline();
        arena.concat_many(&[open, nested, hl2, close])
    } else {
        let ln = arena.line();
        let ln = if let Some(first) = clause_items.first() {
            arena.source_line(first.start_row, ln)
        } else {
            ln
        };
        let inner = arena.concat(ln, clauses_doc);
        let nested = arena.nest(indent, inner);
        let ln2 = arena.line();
        let body = arena.concat_many(&[open, nested, ln2, close]);
        arena.group(body)
    };

    arena.concat_many(&[kw, sp, body])
}

/// `e: TypeError | ValueError => { handler }`
fn convert_except_clause(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();

    let count = node.child_count();
    for i in 0..count {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "identifier"
                if node
                    .child_by_field_name("binding")
                    .is_some_and(|b| b.id() == child.id()) =>
            {
                parts.push(arena.text(node_text(child, source)));
                parts.push(arena.text(": "));
            }
            ":" => {} // consumed by binding
            "except_pattern" => {
                parts.push(convert_except_pattern(arena, child, source, indent));
            }
            "=>" => {
                parts.push(arena.text(" => "));
            }
            "block" => {
                parts.push(convert(arena, child, source, indent));
            }
            _ => {}
        }
    }

    arena.concat_many(&parts)
}

fn convert_except_pattern(arena: &mut Arena, node: Node, source: &[u8], _indent: i32) -> Doc {
    let mut cursor = node.walk();
    let children: Vec<_> = node.named_children(&mut cursor).collect();

    if children.len() == 1 {
        let child = children[0];
        if child.kind() == "pattern_wildcard" {
            return arena.text("_");
        }
        if child.kind() == "except_types" {
            return convert_except_types(arena, child, source);
        }
    }

    arena.text(node_text(node, source))
}

fn convert_except_types(arena: &mut Arena, node: Node, source: &[u8]) -> Doc {
    let mut parts = Vec::new();
    let count = node.child_count();
    for i in 0..count {
        let child = node.child(i as u32).unwrap();
        if child.kind() == "|" {
            parts.push(arena.text(" | "));
        } else if child.kind() == "identifier" {
            parts.push(arena.text(node_text(child, source)));
        }
    }
    arena.concat_many(&parts)
}

/// `finally { body }`
fn convert_finally(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let kw = arena.text("finally");
    let sp = arena.space();
    let body = node.child_by_field_name("body").unwrap();
    let body_doc = convert(arena, body, source, indent);
    arena.concat_many(&[kw, sp, body_doc])
}

// ---- with statement --------------------------------------------------------

pub(crate) fn convert_with(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();
    parts.push(arena.text("with"));
    parts.push(arena.space());

    // Collect with_binding nodes
    let mut bindings = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "with_binding" {
            bindings.push(child);
        }
    }

    for (i, binding) in bindings.iter().enumerate() {
        if i > 0 {
            parts.push(arena.text(","));
            parts.push(arena.space());
        }
        let name = binding.child_by_field_name("name").unwrap();
        let value = binding.child_by_field_name("value").unwrap();
        parts.push(arena.text(node_text(name, source)));
        parts.push(arena.text(" = "));
        parts.push(convert(arena, value, source, indent));
    }

    // Body block
    let body = node.child_by_field_name("body").unwrap();
    parts.push(arena.space());
    parts.push(convert(arena, body, source, indent));

    arena.concat_many(&parts)
}
