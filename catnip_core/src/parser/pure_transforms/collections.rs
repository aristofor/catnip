// FILE: catnip_core/src/parser/pure_transforms/collections.rs
use super::*;

// ============================================================================
// Collections
// ============================================================================

/// Shared helper for list/tuple/set literals (both bracket and function-call forms).
/// Iterates collection_items, handles spreads, emits the given opcode.
pub(crate) fn transform_collection_ir(node: Node, source: &str, opcode: IROpCode, spread_fn: &str) -> TransformResult {
    let mut items = Vec::new();
    let mut spread_entries = Vec::new();
    let mut has_spread = false;

    for child in named_children(&node) {
        if child.kind() == "collection_items" {
            for item_child in named_children(&child) {
                let mut current = item_child;
                if current.kind() == "collection_item" {
                    let inner = named_children(&current);
                    if let Some(first) = inner.first() {
                        current = *first;
                    } else {
                        continue;
                    }
                }

                if current.kind() == "collection_spread" {
                    let spread_children = named_children(&current);
                    if spread_children.is_empty() {
                        return Err("collection_spread: missing value".into());
                    }
                    let value = transform(spread_children[0], source)?;
                    spread_entries.push(IR::op_with_pos(
                        IROpCode::TupleLiteral,
                        vec![IR::Bool(true), value],
                        current.start_byte(),
                        current.end_byte(),
                    ));
                    has_spread = true;
                } else {
                    let value = transform(current, source)?;
                    items.push(value.clone());
                    spread_entries.push(IR::op_with_pos(
                        IROpCode::TupleLiteral,
                        vec![IR::Bool(false), value],
                        current.start_byte(),
                        current.end_byte(),
                    ));
                }
            }
        }
    }

    if has_spread {
        return Ok(IR::Call {
            func: Box::new(IR::Ref(spread_fn.to_string(), -1, -1)),
            args: vec![IR::op_with_pos(
                IROpCode::TupleLiteral,
                spread_entries,
                node.start_byte(),
                node.end_byte(),
            )],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
        });
    }

    Ok(IR::op_with_pos(opcode, items, node.start_byte(), node.end_byte()))
}

/// Accumulates dict entries in both encodings: literal `(key, value)` pairs
/// and the spread-form tuples used when a `**spread` is present.
#[derive(Default)]
struct DictEntries {
    pair_tuples: Vec<IR>,
    spread_entries: Vec<IR>,
    has_spread: bool,
}

/// Route one dict entry node (pair, kwarg, or `**spread`) into both encodings.
fn handle_dict_entry(current: Node, source: &str, entries: &mut DictEntries) -> Result<(), String> {
    match current.kind() {
        "dict_pair" | "colon_pair" => {
            let pair_children = named_children(&current);
            if pair_children.len() >= 2 {
                let key = transform(pair_children[0], source)?;
                let value = transform(pair_children[1], source)?;
                entries.pair_tuples.push(IR::Tuple(vec![key, value]));
                entries.spread_entries.push(IR::op_with_pos(
                    IROpCode::TupleLiteral,
                    vec![
                        IR::Bool(false),
                        transform(pair_children[0], source)?,
                        transform(pair_children[1], source)?,
                    ],
                    current.start_byte(),
                    current.end_byte(),
                ));
            }
        }
        "dict_kwarg" => {
            let kwarg_children = named_children(&current);
            if kwarg_children.len() >= 2 {
                let key_name = node_text(&kwarg_children[0], source);
                let value = transform(kwarg_children[1], source)?;
                entries
                    .pair_tuples
                    .push(IR::Tuple(vec![IR::String(key_name.to_string()), value]));
                entries.spread_entries.push(IR::op_with_pos(
                    IROpCode::TupleLiteral,
                    vec![
                        IR::Bool(false),
                        IR::String(key_name.to_string()),
                        transform(kwarg_children[1], source)?,
                    ],
                    current.start_byte(),
                    current.end_byte(),
                ));
            }
        }
        "dict_spread" => {
            let spread_children = named_children(&current);
            if spread_children.is_empty() {
                return Err("dict_spread: missing value".into());
            }
            let mapping = transform(spread_children[0], source)?;
            entries.spread_entries.push(IR::op_with_pos(
                IROpCode::TupleLiteral,
                vec![IR::Bool(true), mapping],
                current.start_byte(),
                current.end_byte(),
            ));
            entries.has_spread = true;
        }
        _ => {}
    }
    Ok(())
}

/// Emit `__catnip_spread_dict(entries)` when a spread is present, otherwise a
/// plain DictLiteral from the collected pairs.
fn emit_dict_ir(node: &Node, entries: DictEntries) -> TransformResult {
    if entries.has_spread {
        return Ok(IR::Call {
            func: Box::new(IR::Ref("__catnip_spread_dict".to_string(), -1, -1)),
            args: vec![IR::op_with_pos(
                IROpCode::TupleLiteral,
                entries.spread_entries,
                node.start_byte(),
                node.end_byte(),
            )],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
        });
    }

    Ok(IR::op_with_pos(
        IROpCode::DictLiteral,
        entries.pair_tuples,
        node.start_byte(),
        node.end_byte(),
    ))
}

pub(crate) fn transform_bracket_dict(node: Node, source: &str) -> TransformResult {
    // Extract colon_pair and dict_spread entries from bracket_dict_items.
    let mut entries = DictEntries::default();

    for child in named_children(&node) {
        if child.kind() == "bracket_dict_items" {
            for item_child in named_children(&child) {
                handle_dict_entry(item_child, source, &mut entries)?;
            }
        }
    }

    emit_dict_ir(&node, entries)
}

pub(crate) fn transform_dict_literal(node: Node, source: &str) -> TransformResult {
    // dict(iterable) or dict(iterable, key=val, **spread)
    let children = named_children(&node);
    let has_iterable = children.iter().any(|c| c.kind() == "dict_from_iterable");
    if has_iterable {
        let mut iterable_ir = None;
        let mut has_extras = false;

        for child in &children {
            if child.kind() == "dict_from_iterable" {
                let inner = named_children(child);
                iterable_ir = Some(if !inner.is_empty() {
                    transform(inner[0], source)?
                } else {
                    transform(*child, source)?
                });
            } else if child.kind() == "dict_items" {
                has_extras = true;
            }
        }

        let iterable_expr = iterable_ir.unwrap();

        if !has_extras {
            // Simple: dict(iterable) → Call to builtin dict
            return Ok(IR::Call {
                func: Box::new(IR::Ref("dict".to_string(), -1, -1)),
                args: vec![iterable_expr],
                kwargs: IndexMap::new(),
                tail: false,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            });
        }

        // dict(iterable, key=val, **spread) → __catnip_spread_dict
        // First entry: spread the iterable-built dict
        let dict_call = IR::Call {
            func: Box::new(IR::Ref("dict".to_string(), -1, -1)),
            args: vec![iterable_expr],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
        };
        let mut spread_entries = vec![IR::op_with_pos(
            IROpCode::TupleLiteral,
            vec![IR::Bool(true), dict_call],
            node.start_byte(),
            node.end_byte(),
        )];

        // Remaining entries from dict_items
        for child in &children {
            if child.kind() == "dict_items" {
                for item_child in named_children(child) {
                    let mut current = item_child;
                    if current.kind() == "_dict_entry" {
                        if let Some(first) = named_children(&current).first() {
                            current = *first;
                        } else {
                            continue;
                        }
                    }
                    if current.kind() == "dict_kwarg" {
                        let kc = named_children(&current);
                        if kc.len() >= 2 {
                            let key_name = node_text(&kc[0], source);
                            spread_entries.push(IR::op_with_pos(
                                IROpCode::TupleLiteral,
                                vec![
                                    IR::Bool(false),
                                    IR::String(key_name.to_string()),
                                    transform(kc[1], source)?,
                                ],
                                current.start_byte(),
                                current.end_byte(),
                            ));
                        }
                    } else if current.kind() == "dict_spread" {
                        let sc = named_children(&current);
                        if !sc.is_empty() {
                            spread_entries.push(IR::op_with_pos(
                                IROpCode::TupleLiteral,
                                vec![IR::Bool(true), transform(sc[0], source)?],
                                current.start_byte(),
                                current.end_byte(),
                            ));
                        }
                    } else if current.kind() == "dict_pair" {
                        let pc = named_children(&current);
                        if pc.len() >= 2 {
                            spread_entries.push(IR::op_with_pos(
                                IROpCode::TupleLiteral,
                                vec![IR::Bool(false), transform(pc[0], source)?, transform(pc[1], source)?],
                                current.start_byte(),
                                current.end_byte(),
                            ));
                        }
                    }
                }
            }
        }

        return emit_dict_ir(
            &node,
            DictEntries {
                pair_tuples: Vec::new(),
                spread_entries,
                has_spread: true,
            },
        );
    }

    // Extract pairs/spreads from dict_items node.
    // If any **spread is present, lower to __catnip_spread_dict(entries).
    let mut entries = DictEntries::default();

    for child in named_children(&node) {
        if child.kind() == "dict_items" {
            for item_child in named_children(&child) {
                let mut current = item_child;
                if current.kind() == "_dict_entry" {
                    let inner = named_children(&current);
                    if let Some(first) = inner.first() {
                        current = *first;
                    } else {
                        continue;
                    }
                }
                handle_dict_entry(current, source, &mut entries)?;
            }
        }
    }

    emit_dict_ir(&node, entries)
}
