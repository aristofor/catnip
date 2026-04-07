// FILE: catnip_tools/src/pretty/convert_decl.rs
//! Declaration converters: lambda, struct, trait.

use tree_sitter::Node;

use super::convert::{BodyItem, build_body_doc, convert, node_text};
use super::doc::{Arena, Doc};

/// `(params) => { body }`
pub(crate) fn convert_lambda(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();
    parts.push(arena.text("("));

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "lambda_params" {
            parts.push(convert_lambda_params(arena, child, source, indent));
        }
    }

    parts.push(arena.text(") => "));

    let mut cursor2 = node.walk();
    for child in node.named_children(&mut cursor2) {
        if child.kind() == "block" {
            parts.push(convert(arena, child, source, indent));
        }
    }

    arena.concat_many(&parts)
}

/// Comma-separated lambda params.
pub(crate) fn convert_lambda_params(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut params = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "lambda_param" => params.push(convert_lambda_param(arena, child, source, indent)),
            "variadic_param" => {
                let star = arena.text("*");
                let name = child.child_by_field_name("name").unwrap();
                let name_doc = arena.text(node_text(name, source));
                params.push(arena.concat(star, name_doc));
            }
            _ => {}
        }
    }
    let sep = arena.text(", ");
    arena.intersperse(&params, sep)
}

fn convert_lambda_param(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let name = node.child_by_field_name("name").unwrap();
    let name_doc = arena.text(node_text(name, source));

    if let Some(default) = node.child_by_field_name("default") {
        let eq = arena.text("=");
        let val = convert(arena, default, source, indent);
        arena.concat_many(&[name_doc, eq, val])
    } else {
        name_doc
    }
}

/// Collect struct/trait body items, merging inline items on the same row.
fn collect_decl_body(arena: &mut Arena, node: Node, source: &[u8], indent: i32, skip_id: usize) -> Vec<BodyItem> {
    let mut body_items: Vec<BodyItem> = Vec::new();

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.id() == skip_id {
            continue;
        }
        match child.kind() {
            "struct_field" | "struct_method" => {
                let doc = if child.kind() == "struct_field" {
                    convert_struct_field(arena, child, source, indent)
                } else {
                    convert_struct_method(arena, child, source, indent)
                };
                let start_row = child.start_position().row;
                // Inline items on same source line: merge with space separator
                if let Some(last) = body_items.last_mut() {
                    if start_row == last.end_row {
                        let sp = arena.text(" ");
                        let combined = arena.concat_many(&[last.doc, sp, doc]);
                        last.doc = combined;
                        last.end_row = child.end_position().row;
                        continue;
                    }
                }
                body_items.push(BodyItem {
                    doc,
                    start_row,
                    end_row: child.end_position().row,
                    semi_before: false,
                });
            }
            "comment" => {
                let doc = arena.text(node_text(child, source).trim_end());
                let start_row = child.start_position().row;
                let end_row = child.end_position().row;
                if let Some(last) = body_items.last_mut() {
                    if start_row == last.end_row {
                        let sp = arena.text("  ");
                        let combined = arena.concat_many(&[last.doc, sp, doc]);
                        last.doc = combined;
                        last.end_row = end_row;
                        continue;
                    }
                }
                body_items.push(BodyItem {
                    doc,
                    start_row,
                    end_row,
                    semi_before: false,
                });
            }
            _ => {}
        }
    }

    body_items
}

/// `struct Name [extends(...)] [implements(...)] { fields; methods }`
pub(crate) fn convert_struct(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let name = node.child_by_field_name("name").unwrap();
    let name_id = name.id();

    let mut header = vec![arena.text("struct"), arena.space(), arena.text(node_text(name, source))];

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.id() == name_id {
            continue;
        }
        match child.kind() {
            "struct_extends" => {
                let sp = arena.space();
                header.push(sp);
                header.push(convert_extends(arena, child, source));
            }
            "struct_implements" => {
                let sp = arena.space();
                header.push(sp);
                header.push(convert_implements(arena, child, source));
            }
            _ => {}
        }
    }

    let body_items = collect_decl_body(arena, node, source, indent, name_id);
    let sp = arena.space();
    let block = if body_items.is_empty() {
        arena.text("{}")
    } else {
        let body = build_body_doc(arena, &body_items);
        // Preserve source intent: multiline structs stay multiline
        if node.start_position().row != node.end_position().row {
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
    };
    header.push(sp);
    header.push(block);
    arena.concat_many(&header)
}

/// `trait Name [extends(...)] { fields; methods }`
pub(crate) fn convert_trait(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let name = node.child_by_field_name("name").unwrap();
    let name_id = name.id();

    let mut header = vec![arena.text("trait"), arena.space(), arena.text(node_text(name, source))];

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.id() == name_id {
            continue;
        }
        if child.kind() == "trait_extends" {
            let sp = arena.space();
            header.push(sp);
            header.push(convert_extends(arena, child, source));
        }
    }

    let body_items = collect_decl_body(arena, node, source, indent, name_id);
    let sp = arena.space();
    let block = if body_items.is_empty() {
        arena.text("{}")
    } else {
        let body = build_body_doc(arena, &body_items);
        if node.start_position().row != node.end_position().row {
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
    };
    header.push(sp);
    header.push(block);
    arena.concat_many(&header)
}

// -- Helpers --

fn convert_extends(arena: &mut Arena, node: Node, source: &[u8]) -> Doc {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            names.push(arena.text(node_text(child, source)));
        }
    }
    let kw = arena.text("extends(");
    let sep = arena.text(", ");
    let names_doc = arena.intersperse(&names, sep);
    let close = arena.text(")");
    arena.concat_many(&[kw, names_doc, close])
}

fn convert_implements(arena: &mut Arena, node: Node, source: &[u8]) -> Doc {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            names.push(arena.text(node_text(child, source)));
        }
    }
    let kw = arena.text("implements(");
    let sep = arena.text(", ");
    let names_doc = arena.intersperse(&names, sep);
    let close = arena.text(")");
    arena.concat_many(&[kw, names_doc, close])
}

fn convert_struct_field(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let name = node.child_by_field_name("name").unwrap();
    let name_doc = arena.text(node_text(name, source));

    // Preserve source semicolon: only add `;` if present in original
    let last_idx = node.child_count().saturating_sub(1) as u32;
    let has_semi = node.child(last_idx).is_some_and(|c| c.kind() == ";");

    if let Some(default) = node.child_by_field_name("default") {
        let eq = arena.text(" = ");
        let val = convert(arena, default, source, indent);
        if has_semi {
            let semi = arena.text(";");
            arena.concat_many(&[name_doc, eq, val, semi])
        } else {
            arena.concat_many(&[name_doc, eq, val])
        }
    } else if has_semi {
        let semi = arena.text(";");
        arena.concat(name_doc, semi)
    } else {
        name_doc
    }
}

fn convert_struct_method(arena: &mut Arena, node: Node, source: &[u8], indent: i32) -> Doc {
    let mut parts = Vec::new();
    let count = node.child_count();

    for i in 0..count {
        let child = node.child(i as u32).unwrap();
        match (child.kind(), child.is_named()) {
            ("decorator", true) => {
                parts.push(arena.text(node_text(child, source)));
                parts.push(arena.hardline());
            }
            ("op", false) => {
                parts.push(arena.text("op "));
            }
            ("operator_symbol", true) => {
                parts.push(arena.text(node_text(child, source)));
            }
            ("identifier", true) => {
                parts.push(arena.text(node_text(child, source)));
            }
            ("(", false) => {
                parts.push(arena.text("("));
            }
            ("lambda_params", true) => {
                parts.push(convert_lambda_params(arena, child, source, indent));
            }
            (")", false) => {
                parts.push(arena.text(")"));
            }
            ("=>", false) => {
                parts.push(arena.text(" => "));
            }
            ("block", true) => {
                parts.push(convert(arena, child, source, indent));
            }
            _ => {}
        }
    }

    arena.concat_many(&parts)
}
