// FILE: catnip_tools/src/pretty/convert.rs
//! CST -> Doc conversion dispatch.

use tree_sitter::Node;

use super::convert_decl;
use super::convert_expr;
use super::convert_stmt;
use super::doc::{Arena, Doc};

pub(crate) fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap()
}

/// Convert a tree-sitter node to a Doc.
pub(crate) fn convert(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    match node.kind() {
        "source_file" => convert_source_file(arena, node, source, indent),

        // Transparent wrappers (single named child)
        "statement" | "lvalue" | "unpack_target" | "number" | "literal" => {
            if let Some(child) = node.named_child(0) {
                convert(arena, child, source, indent)
            } else {
                arena.text(node_text(node, source))
            }
        }

        // Leaves
        "identifier" | "integer" | "float" | "decimal" | "imaginary" | "true" | "false" | "none" => {
            arena.text(node_text(node, source))
        }

        // Strings: multiline -> verbatim
        "string" | "fstring" | "bstring" => {
            let text = node_text(node, source);
            if text.contains('\n') {
                arena.verbatim(text)
            } else {
                arena.text(text)
            }
        }

        // Comments
        "comment" => arena.text(node_text(node, source).trim_end()),

        // Binary expressions
        "additive" | "multiplicative" | "exponent" | "shift" | "bool_or" | "bool_and" | "bit_or" | "bit_xor"
        | "bit_and" | "null_coalesce" => convert_expr::convert_binary(arena, node, source, indent),
        "comparison" => convert_expr::convert_comparison(arena, node, source, indent),

        // Unary
        "unary" => convert_expr::convert_unary(arena, node, source, indent),
        "bool_not" => convert_expr::convert_bool_not(arena, node, source, indent),

        // Structures
        "assignment" => convert_expr::convert_assignment(arena, node, source, indent),
        "block" => convert_expr::convert_block(arena, node, source, indent),
        "group" => convert_expr::convert_paren(arena, node, source, indent),

        // Control flow
        "if_expr" => convert_stmt::convert_if(arena, node, source, indent),
        "while_stmt" => convert_stmt::convert_while(arena, node, source, indent),
        "for_stmt" => convert_stmt::convert_for(arena, node, source, indent),
        "match_expr" => convert_stmt::convert_match(arena, node, source, indent),
        "try_stmt" => convert_stmt::convert_try(arena, node, source, indent),
        "with_stmt" => convert_stmt::convert_with(arena, node, source, indent),
        "pragma_stmt" => convert_stmt::convert_pragma(arena, node, source, indent),

        // Call & chained
        "call" => convert_expr::convert_call(arena, node, source, indent),
        "arguments" => convert_expr::convert_arguments(arena, node, source, indent),
        "kwarg" => convert_expr::convert_kwarg(arena, node, source, indent),
        "chained" => convert_expr::convert_chained(arena, node, source, indent),

        // Collections
        "bracket_list" => convert_expr::convert_bracket_list(arena, node, source, indent),
        "bracket_dict" => convert_expr::convert_bracket_dict(arena, node, source, indent),
        "list_literal" => convert_expr::convert_collection(arena, "list", node, source, indent),
        "tuple_literal" => convert_expr::convert_collection(arena, "tuple", node, source, indent),
        "set_literal" => convert_expr::convert_collection(arena, "set", node, source, indent),
        "dict_literal" => convert_expr::convert_dict(arena, node, source, indent),

        // ND operators
        "nd_recursion" => convert_expr::convert_nd_recursion(arena, node, source, indent),
        "nd_map" => convert_expr::convert_nd_map(arena, node, source, indent),

        // Broadcast internals (used inside .[...])
        "broadcast_op" => {
            if let Some(child) = node.named_child(0) {
                convert(arena, child, source, indent)
            } else {
                arena.text(node_text(node, source))
            }
        }
        "broadcast_binary" => {
            let op = node.child(0);
            let expr = node.child(1);
            match (op, expr) {
                (Some(op), Some(expr)) => {
                    let op_doc = arena.text(node_text(op, source));
                    let sp = arena.space();
                    let expr_doc = convert(arena, expr, source, indent);
                    arena.concat_many(&[op_doc, sp, expr_doc])
                }
                _ => arena.text(node_text(node, source)),
            }
        }
        "broadcast_if" => {
            let kw = arena.text("if ");
            if let Some(inner) = node.named_child(0) {
                let inner_doc = convert(arena, inner, source, indent);
                arena.concat(kw, inner_doc)
            } else {
                arena.text(node_text(node, source))
            }
        }
        "broadcast_nd_recursion" => {
            if let Some(expr) = node.named_child(0) {
                let expr_doc = convert(arena, expr, source, indent);
                let starts_paren =
                    matches!(expr.kind(), "group" | "arguments") || expr.child(0).is_some_and(|c| c.kind() == "(");
                let prefix = if starts_paren {
                    arena.text("~~")
                } else {
                    arena.text("~~ ")
                };
                arena.concat(prefix, expr_doc)
            } else {
                arena.text(node_text(node, source))
            }
        }
        "broadcast_nd_map" => {
            if let Some(expr) = node.named_child(0) {
                let expr_doc = convert(arena, expr, source, indent);
                let starts_paren =
                    matches!(expr.kind(), "group" | "arguments") || expr.child(0).is_some_and(|c| c.kind() == "(");
                let prefix = if starts_paren {
                    arena.text("~>")
                } else {
                    arena.text("~> ")
                };
                arena.concat(prefix, expr_doc)
            } else {
                arena.text(node_text(node, source))
            }
        }
        "broadcast_unary" | "bcast_unary_op" | "bcast_op" => arena.text(node_text(node, source)),
        // Slice: normalize `1 : 3 : 2` → `1:3:2`
        "slice_range" => {
            let mut parts = Vec::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    parts.push(convert(arena, child, source, indent));
                } else {
                    let text = node_text(child, source).trim();
                    if text == ":" {
                        parts.push(arena.text(":"));
                    }
                }
            }
            arena.concat_many(&parts)
        }

        // Unpacking
        "unpack_tuple" => convert_expr::convert_unpack_tuple(arena, node, source, indent),
        "unpack_sequence" => convert_expr::convert_unpack_sequence(arena, node, source, indent),

        // Declarations
        "lambda_expr" => convert_decl::convert_lambda(arena, node, source, indent),
        "struct_stmt" => convert_decl::convert_struct(arena, node, source, indent),
        "trait_stmt" => convert_decl::convert_trait(arena, node, source, indent),
        "decorator" => {
            let text = node_text(node, source);
            arena.text(text)
        }

        // Return/break/continue
        "return_stmt" => convert_expr::convert_return(arena, node, source, indent),
        "raise_stmt" => convert_expr::convert_raise(arena, node, source, indent),
        "break_stmt" => arena.text("break"),
        "continue_stmt" => arena.text("continue"),

        // Fallback: preserve source text
        _ => {
            let text = node_text(node, source);
            if text.contains('\n') {
                arena.verbatim(text)
            } else {
                arena.text(text)
            }
        }
    }
}

// -- Body handling (source_file, block) --

pub(crate) struct BodyItem {
    pub doc: Doc,
    pub start_row: usize,
    pub end_row: usize,
    pub semi_before: bool,
}

/// Collect statements and comments from a compound node's children.
/// Trailing comments are merged with the preceding item.
/// Semicolons between statements on the same line are tracked.
pub(crate) fn collect_body_items(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Vec<BodyItem> {
    let mut items: Vec<BodyItem> = Vec::new();
    let mut prev_semi = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Track semicolons between statements
        if !child.is_named() {
            if child.kind() == ";" {
                prev_semi = true;
            }
            continue;
        }

        let start_row = child.start_position().row;
        let end_row = child.end_position().row;

        if child.kind() == "comment" {
            let doc = arena.text(node_text(child, source).trim_end());

            // Trailing comment: same row as previous item's end
            if let Some(last) = items.last_mut() {
                if start_row == last.end_row {
                    let sp = arena.text("  ");
                    let combined = arena.concat_many(&[last.doc, sp, doc]);
                    last.doc = combined;
                    last.end_row = end_row;
                    prev_semi = false;
                    continue;
                }
            }

            items.push(BodyItem {
                doc,
                start_row,
                end_row,
                semi_before: false,
            });
        } else {
            items.push(BodyItem {
                doc: convert(arena, child, source, indent),
                start_row,
                end_row,
                semi_before: prev_semi,
            });
        }
        prev_semi = false;
    }

    items
}

/// Build a Doc from body items, inserting hardlines (+ extra for blank lines).
/// Semicolon-separated items stay on the same line.
pub(crate) fn build_body_doc(arena: &mut Arena, items: &[BodyItem]) -> Doc {
    if items.is_empty() {
        return arena.nil();
    }

    let first = arena.source_line(items[0].start_row, items[0].doc);
    let mut result = first;

    for i in 1..items.len() {
        let gap = items[i].start_row.saturating_sub(items[i - 1].end_row);

        let mut block = if items[i].semi_before && gap == 0 {
            // Semicolon-separated on same line
            let sep = arena.text("; ");
            arena.concat(sep, items[i].doc)
        } else if gap >= 2 {
            let hl = arena.hardline();
            let hl2 = arena.hardline();
            arena.concat_many(&[hl, hl2, items[i].doc])
        } else {
            let hl = arena.hardline();
            arena.concat(hl, items[i].doc)
        };
        block = arena.source_line(items[i].start_row, block);
        result = arena.concat(result, block);
    }

    result
}

fn convert_source_file(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let items = collect_body_items(arena, node, source, indent);
    build_body_doc(arena, &items)
}
