// FILE: catnip_core/src/parser/pure_transforms/control_flow.rs
use super::*;

// ============================================================================
// Control Flow
// ============================================================================

pub(crate) fn transform_if_expr(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.is_empty() {
        return Ok(IR::None);
    }

    let mut condition: Option<IR> = None;
    let mut consequence: Option<IR> = None;
    let mut elif_clauses: Vec<IR> = Vec::new();
    let mut else_block: Option<IR> = None;

    for child in &children {
        match child.kind() {
            "block" => {
                if condition.is_some() && consequence.is_none() {
                    consequence = Some(transform(*child, source)?);
                }
            }
            "elif_clause" => {
                elif_clauses.push(transform(*child, source)?);
            }
            "else_clause" => {
                else_block = Some(transform(*child, source)?);
            }
            _ => {
                if condition.is_none() {
                    condition = Some(transform(*child, source)?);
                }
            }
        }
    }

    // Build branches: [(condition, block), ...]
    let mut branches: Vec<IR> = Vec::new();
    if let (Some(cond), Some(cons)) = (condition, consequence) {
        branches.push(IR::Tuple(vec![cond, cons]));
    }
    branches.extend(elif_clauses);

    // OpIf args: [Tuple(branches), optional_else]
    let branches_tuple = IR::Tuple(branches);
    let args = if let Some(else_val) = else_block {
        vec![branches_tuple, else_val]
    } else {
        vec![branches_tuple]
    };

    Ok(IR::op_with_pos(
        IROpCode::OpIf,
        args,
        node.start_byte(),
        node.end_byte(),
    ))
}

pub(crate) fn transform_elif_clause(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    let mut condition: Option<IR> = None;
    let mut block: Option<IR> = None;

    for child in &children {
        if child.kind() == "block" {
            block = Some(transform(*child, source)?);
        } else if condition.is_none() {
            condition = Some(transform(*child, source)?);
        }
    }

    // Return tuple (condition, block)
    let condition_val = condition.unwrap_or(IR::None);
    let block_val = block.unwrap_or(IR::None);

    Ok(IR::Tuple(vec![condition_val, block_val]))
}

pub(crate) fn transform_else_clause(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // else_clause has one child: the block
    if !children.is_empty() {
        transform(children[0], source)
    } else {
        Ok(IR::None)
    }
}

pub(crate) fn transform_while_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.len() < 2 {
        return Err("while_stmt requires condition and body".into());
    }

    let condition = transform(children[0], source)?;
    let body = transform(children[1], source)?;

    Ok(IR::op_with_pos(
        IROpCode::OpWhile,
        vec![condition, body],
        node.start_byte(),
        node.end_byte(),
    ))
}

pub(crate) fn transform_for_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.len() < 3 {
        return Err("for_stmt requires var, iterable, and body".into());
    }

    let var = transform(children[0], source)?;
    let iterable = transform(children[1], source)?;
    let body = transform(children[2], source)?;

    Ok(IR::op_with_pos(
        IROpCode::OpFor,
        vec![var, iterable, body],
        node.start_byte(),
        node.end_byte(),
    ))
}

pub(crate) fn transform_return_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    let value = if !children.is_empty() {
        transform(children[0], source)?
    } else {
        IR::None
    };

    Ok(IR::op(IROpCode::OpReturn, vec![value]))
}

pub(crate) fn transform_break_stmt(_node: Node, _source: &str) -> TransformResult {
    Ok(IR::op(IROpCode::OpBreak, vec![]))
}

pub(crate) fn transform_continue_stmt(_node: Node, _source: &str) -> TransformResult {
    Ok(IR::op(IROpCode::OpContinue, vec![]))
}

// -- Error handling ----------------------------------------------------------

pub(crate) fn transform_raise_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.is_empty() {
        // bare raise
        Ok(IR::op_with_pos(
            IROpCode::OpRaise,
            vec![],
            node.start_byte(),
            node.end_byte(),
        ))
    } else {
        // raise <expr>
        let value = transform(children[0], source)?;
        Ok(IR::op_with_pos(
            IROpCode::OpRaise,
            vec![value],
            node.start_byte(),
            node.end_byte(),
        ))
    }
}

pub(crate) fn transform_try_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    let mut body: Option<IR> = None;
    let mut handlers = IR::List(vec![]);
    let mut finally_block = IR::None;

    for child in &children {
        match child.kind() {
            "block" if body.is_none() => {
                body = Some(transform(*child, source)?);
            }
            "except_block" => {
                handlers = transform_except_block(*child, source)?;
            }
            "finally_clause" => {
                finally_block = transform_finally_clause(*child, source)?;
            }
            _ => {}
        }
    }

    let body_ir = body.ok_or_else(|| "try_stmt: missing body block".to_string())?;

    Ok(IR::op_with_pos(
        IROpCode::OpTry,
        vec![body_ir, handlers, finally_block],
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_except_block(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let mut clauses = Vec::new();

    for child in &children {
        if child.kind() == "except_clause" {
            clauses.push(transform_except_clause(*child, source)?);
        }
    }

    Ok(IR::List(clauses))
}

fn transform_except_clause(node: Node, source: &str) -> TransformResult {
    // Extract binding (optional field)
    let binding = if let Some(binding_node) = node.child_by_field_name("binding") {
        IR::String(node_text(&binding_node, source).to_string())
    } else {
        IR::None
    };

    // Extract exception types from except_pattern
    let mut types = Vec::new();
    let children = named_children(&node);
    for child in &children {
        if child.kind() == "except_pattern" {
            let pattern_children = named_children(child);
            for pc in &pattern_children {
                match pc.kind() {
                    "except_types" => {
                        let type_children = named_children(pc);
                        for tc in &type_children {
                            if tc.kind() == "identifier" {
                                types.push(IR::String(node_text(tc, source).to_string()));
                            }
                        }
                    }
                    "pattern_wildcard" => {
                        // wildcard = catch-all, types stays empty
                    }
                    _ => {}
                }
            }
        }
    }

    // Extract handler block (field)
    let handler = if let Some(handler_node) = node.child_by_field_name("handler") {
        transform(handler_node, source)?
    } else {
        return Err("except_clause: missing handler block".into());
    };

    // Tuple(types_list, binding_or_nil, handler_block)
    Ok(IR::Tuple(vec![IR::List(types), binding, handler]))
}

fn transform_finally_clause(node: Node, source: &str) -> TransformResult {
    if let Some(body_node) = node.child_by_field_name("body") {
        transform(body_node, source)
    } else {
        Err("finally_clause: missing body block".into())
    }
}
