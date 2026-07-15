// FILE: catnip_core/src/parser/pure_transforms/patterns.rs
use super::*;

// ============================================================================
// Pattern Matching
// ============================================================================

pub(crate) fn transform_match_expr(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // Structure: match_expr(value, match_case+)
    if children.is_empty() {
        return Err("match_expr: no value".into());
    }

    let value = transform(children[0], source)?;

    // Collecter tous les match_case
    let mut cases = Vec::new();
    for child in &children[1..] {
        if child.kind() == "match_case" {
            cases.push(transform(*child, source)?);
        }
    }

    // OpMatch(value, Tuple(cases))
    let cases_tuple = IR::Tuple(cases);
    Ok(IR::op_with_pos(
        IROpCode::OpMatch,
        vec![value, cases_tuple],
        node.start_byte(),
        node.end_byte(),
    ))
}

pub(crate) fn transform_match_case(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // Structure: match_case(pattern, guard?, block)
    // guard est un field tree-sitter, pas un kind
    let mut pattern: Option<IR> = None;
    let mut block: Option<IR> = None;

    for child in children {
        match child.kind() {
            "pattern" | "pattern_literal" | "pattern_var" | "pattern_wildcard" | "pattern_or" | "pattern_tuple"
            | "pattern_struct" => {
                pattern = Some(transform(child, source)?);
            }
            "block" => {
                block = Some(transform(child, source)?);
            }
            _ => {}
        }
    }

    // Récupérer le guard via field au lieu de kind
    let guard = if let Some(guard_node) = node.child_by_field_name("guard") {
        Some(transform(guard_node, source)?)
    } else {
        None
    };

    // Retourner Tuple(pattern, guard, block)
    let pattern_val = pattern.ok_or("match_case: missing pattern")?;
    let guard_val = guard.unwrap_or(IR::None);
    let block_val = block.ok_or("match_case: missing block")?;

    Ok(IR::Tuple(vec![pattern_val, guard_val, block_val]))
}

// ============================================================================
// Patterns
// ============================================================================

pub(crate) fn transform_pattern(node: Node, source: &str) -> TransformResult {
    // Pattern est un wrapper, dispatcher vers le type réel
    let children = named_children(&node);
    if children.is_empty() {
        return Err("pattern: no children".into());
    }

    transform(children[0], source)
}

pub(crate) fn transform_pattern_literal(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.is_empty() {
        return Err("pattern_literal: no value".into());
    }

    let value = transform(children[0], source)?;
    Ok(IR::PatternLiteral(Box::new(value)))
}

pub(crate) fn transform_pattern_var(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.is_empty() {
        return Err("pattern_var: no identifier".into());
    }

    let name_node = children[0];
    if name_node.kind() != "identifier" {
        return Err(format!("pattern_var: expected identifier, got {}", name_node.kind()));
    }

    let name = node_text(&name_node, source).to_string();
    Ok(IR::PatternVar(name))
}

pub(crate) fn transform_pattern_wildcard(_node: Node, _source: &str) -> TransformResult {
    Ok(IR::PatternWildcard)
}

pub(crate) fn transform_pattern_star(node: Node, source: &str) -> TransformResult {
    // pattern_star: seq('*', $.identifier)
    // Retourne un tuple ('*', name) pour le registry
    let children = named_children(&node);
    if children.is_empty() {
        return Err("pattern_star: no identifier".into());
    }

    let identifier_node = children[0];
    if identifier_node.kind() != "identifier" {
        return Err(format!(
            "pattern_star: expected identifier, got {}",
            identifier_node.kind()
        ));
    }

    let name = node_text(&identifier_node, source).to_string();
    // Retourner tuple ('*', name)
    Ok(IR::Tuple(vec![IR::String("*".to_string()), IR::String(name)]))
}

pub(crate) fn transform_variadic_param(node: Node, source: &str) -> TransformResult {
    // Variadic parameter in unpacking: *rest
    // Expected format: ("*", "varname")
    let children = named_children(&node);
    for child in children {
        if child.kind() == "identifier" {
            let name = node_text(&child, source).to_string();
            return Ok(IR::Tuple(vec![IR::String("*".to_string()), IR::String(name)]));
        }
    }
    Err("variadic_param: no identifier found".into())
}

pub(crate) fn transform_pattern_or(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // Si un seul enfant, retourner directement le pattern sans PatternOr
    if children.len() == 1 {
        return transform(children[0], source);
    }

    // Plusieurs patterns séparés par |
    let mut patterns = Vec::new();
    for child in children {
        patterns.push(transform(child, source)?);
    }

    if patterns.is_empty() {
        return Err("pattern_or: no patterns".into());
    }

    Ok(IR::PatternOr(patterns))
}

pub(crate) fn transform_pattern_tuple(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // pattern_tuple contient un seul enfant: pattern_items
    // Il faut descendre dans pattern_items pour récupérer les vrais patterns
    let mut patterns = Vec::new();
    for child in children {
        if child.kind() == "pattern_items" {
            // Descendre dans pattern_items pour récupérer les patterns individuels
            for pattern_child in named_children(&child) {
                patterns.push(transform(pattern_child, source)?);
            }
        } else if child.kind().starts_with("pattern") {
            // Fallback pour d'autres types de patterns
            patterns.push(transform(child, source)?);
        }
    }

    Ok(IR::PatternTuple(patterns))
}

pub(crate) fn transform_pattern_struct(node: Node, source: &str) -> TransformResult {
    let name = node
        .child_by_field_name("struct_name")
        .map(|n| node_text(&n, source).to_string())
        .ok_or("pattern_struct: missing struct name")?;

    let variant = node
        .child_by_field_name("variant_name")
        .map(|n| node_text(&n, source).to_string());

    // Field nodes are tagged with the `fields` field name. Multiple
    // identifiers may share that tag for `Point{x, y}` style patterns.
    let mut cursor = node.walk();
    let fields: Vec<String> = node
        .children_by_field_name("fields", &mut cursor)
        .map(|n| node_text(&n, source).to_string())
        .collect();

    Ok(IR::PatternStruct { name, variant, fields })
}

pub(crate) fn transform_pattern_enum(node: Node, source: &str) -> TransformResult {
    let enum_name = node
        .child_by_field_name("enum_name")
        .ok_or("pattern_enum: missing enum_name")?;
    let variant_name = node
        .child_by_field_name("variant_name")
        .ok_or("pattern_enum: missing variant_name")?;
    Ok(IR::PatternEnum {
        enum_name: node_text(&enum_name, source).to_string(),
        variant_name: node_text(&variant_name, source).to_string(),
    })
}
