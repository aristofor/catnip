// FILE: catnip_core/src/parser/pure_transforms/access.rs
use super::*;

// ============================================================================
// Access & Indexing
// ============================================================================

pub(crate) fn transform_index(node: Node, source: &str) -> TransformResult {
    // index: [expr], [slice_range], or [expr, expr, ...] (tuple subscript).
    // Returns the index expression (or slice) for a GetItem operation.
    let children = named_children(&node);

    if children.is_empty() {
        return Err("index: no children".into());
    }

    // Multi-index subscript `arr[y, x]` -> tuple key `(y, x)`, matching Python
    // and numpy `__getitem__((y, x))`. Each element may itself be a slice.
    if children.len() > 1 {
        let mut items = Vec::with_capacity(children.len());
        for child in &children {
            items.push(transform_index_elem(*child, source)?);
        }
        return Ok(IR::Tuple(items));
    }

    transform_index_elem(children[0], source)
}

fn transform_index_elem(child: Node, source: &str) -> TransformResult {
    if child.kind() == "slice_range" {
        transform_slice_range(child, source)
    } else {
        transform(child, source)
    }
}

pub(crate) fn transform_slice_range(node: Node, source: &str) -> TransformResult {
    // slice_range: start:stop:step (all parts optional)
    // Returns IR::Slice variant

    let node_source = node_text(&node, source);
    let mut start = IR::None;
    let mut stop = IR::None;
    let mut step = IR::None;

    // Count colons to determine structure
    let colon_count = node_source.matches(':').count();

    if colon_count == 0 {
        return Err("slice_range: no colons found".into());
    }

    // Get all children (named nodes)
    let children = named_children(&node);

    // Parse slice by examining structure:
    // - Find positions of colons to determine which part each child represents
    // - Children appear in document order (start, stop, step) if present

    // Split the source text by colons to identify empty positions
    let parts: Vec<&str> = node_source.split(':').collect();

    // Determine which positions have values based on non-empty parts
    let mut child_idx = 0;
    for (part_idx, part) in parts.iter().enumerate() {
        if !part.trim().is_empty() && child_idx < children.len() {
            let mut expr = transform(children[child_idx], source)?;

            // Special handling for negative numbers in slices:
            // Convert unary negation of literals to negative literals
            if let IR::Op {
                opcode: IROpCode::Neg,
                args,
                ..
            } = &expr
            {
                if args.len() == 1 {
                    match &args[0] {
                        IR::Int(n) => expr = IR::Int(-n),
                        IR::Float(f) => expr = IR::Float(-f),
                        _ => {}
                    }
                }
            }

            match part_idx {
                0 => start = expr,
                1 => stop = expr,
                2 => step = expr,
                _ => {}
            }
            child_idx += 1;
        }
    }

    // Create Slice variant
    Ok(IR::Slice {
        start: Box::new(start),
        stop: Box::new(stop),
        step: Box::new(step),
    })
}

pub(crate) fn transform_fullslice(node: Node, source: &str) -> TransformResult {
    // fullslice: .[slice_range]
    // This is a broadcast operation on a slice (e.g., x.[1:3])
    let children = named_children(&node);

    if children.is_empty() {
        return Err("fullslice: no children".into());
    }

    if children[0].kind() == "slice_range" {
        // Transform the slice_range to a Slice IR node
        transform_slice_range(children[0], source)
    } else {
        Err("fullslice: expected slice_range".into())
    }
}

pub(crate) fn transform_broadcast(node: Node, source: &str) -> TransformResult {
    // broadcast: .[broadcast_op]
    // Returns IR::Broadcast variant

    let children = named_children(&node);
    if children.is_empty() {
        return Err("broadcast: no children".into());
    }

    let first_child = children[0];

    // If first child is broadcast_op wrapper, descend to actual operation
    let (broadcast_op_node, actual_kind) = if first_child.kind() == "broadcast_op" {
        let op_children = named_children(&first_child);
        if !op_children.is_empty() {
            let inner = op_children[0];
            (inner, inner.kind())
        } else {
            (first_child, first_child.kind())
        }
    } else {
        (first_child, first_child.kind())
    };

    use crate::ir::BroadcastType;

    match actual_kind {
        "broadcast_binary" => transform_broadcast_binary(broadcast_op_node, source),
        "broadcast_unary" => transform_broadcast_unary(broadcast_op_node, source),
        "broadcast_if" => transform_broadcast_if(broadcast_op_node, source),
        "broadcast_nd_recursion" => {
            let children = named_children(&broadcast_op_node);
            if children.is_empty() {
                return Err("broadcast_nd_recursion: no expression".into());
            }
            let expr = transform(children[0], source)?;
            Ok(IR::Broadcast {
                target: None,
                operator: Box::new(expr),
                operand: None,
                broadcast_type: BroadcastType::NDRecursion,
            })
        }
        _ => {
            // Default: lambda or expression
            let expr = transform(broadcast_op_node, source)?;
            Ok(IR::Broadcast {
                target: None,
                operator: Box::new(expr),
                operand: None,
                broadcast_type: BroadcastType::Lambda,
            })
        }
    }
}

fn transform_broadcast_binary(node: Node, source: &str) -> TransformResult {
    // broadcast_binary: operator expression
    let children = named_children(&node);

    if children.len() < 2 {
        return Err("broadcast_binary: expected operator and expression".into());
    }

    // First child is the operator token
    let operator_str = node_text(&children[0], source).to_string();
    let operand = transform(children[1], source)?;

    use crate::ir::BroadcastType;
    Ok(IR::Broadcast {
        target: None,
        operator: Box::new(IR::String(operator_str)),
        operand: Some(Box::new(operand)),
        broadcast_type: BroadcastType::Binary,
    })
}

fn transform_broadcast_unary(node: Node, source: &str) -> TransformResult {
    // broadcast_unary: unary_operator
    let operator_str = node_text(&node, source).to_string();

    use crate::ir::BroadcastType;
    Ok(IR::Broadcast {
        target: None,
        operator: Box::new(IR::String(operator_str)),
        operand: None,
        broadcast_type: BroadcastType::Unary,
    })
}

fn transform_broadcast_if(node: Node, source: &str) -> TransformResult {
    // broadcast_if: if/elif/else clauses
    // We encode the clauses as a list expression
    let children = named_children(&node);

    let mut clauses = Vec::new();

    for child in children {
        match child.kind() {
            "if_broadcast_clause" | "elif_broadcast_clause" => {
                let clause_children = named_children(&child);
                if clause_children.len() >= 2 {
                    let condition = transform(clause_children[0], source)?;
                    let body = transform(clause_children[1], source)?;
                    clauses.push(IR::Tuple(vec![condition, body]));
                }
            }
            "else_broadcast_clause" => {
                let clause_children = named_children(&child);
                if !clause_children.is_empty() {
                    let body = transform(clause_children[0], source)?;
                    clauses.push(IR::Tuple(vec![IR::Bool(true), body]));
                }
            }
            _ => {}
        }
    }

    use crate::ir::BroadcastType;
    Ok(IR::Broadcast {
        target: None,
        operator: Box::new(IR::List(clauses)),
        operand: None,
        broadcast_type: BroadcastType::If,
    })
}
