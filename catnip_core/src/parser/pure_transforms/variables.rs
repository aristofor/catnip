// FILE: catnip_core/src/parser/pure_transforms/variables.rs
use super::*;

// ============================================================================
// Variables
// ============================================================================

pub(crate) fn transform_identifier(node: Node, source: &str) -> TransformResult {
    let text = node_text(&node, source);
    let start_byte = node.start_byte() as isize;
    let end_byte = node.end_byte() as isize;
    Ok(IR::Ref(text.to_string(), start_byte, end_byte))
}

pub(crate) fn transform_setattr_lvalue(node: Node, source: &str) -> TransformResult {
    // setattr node: _atom followed by repeat1(_member)
    // Transform it into a GetAttr structure that transform_assignment can recognize
    let children = named_children(&node);

    if children.is_empty() {
        return Err("setattr: no children".into());
    }

    // First child is the base object
    let mut result = transform(children[0], source)?;

    // Process each member access (.attr or [index])
    for child in children.iter().skip(1) {
        if child.kind() == "getattr" {
            let attr_children = named_children(child);
            if !attr_children.is_empty() && attr_children[0].kind() == "identifier" {
                let attr_name = node_text(&attr_children[0], source).to_string();
                result = IR::op(IROpCode::GetAttr, vec![result, IR::String(attr_name)]);
            }
        } else if child.kind() == "index" {
            // transform_index handles tuple subscripts (arr[y, x]); build_assignment
            // turns the resulting GetItem into a SetItem for the write path.
            let index = transform_index(*child, source)?;
            result = IR::op(IROpCode::GetItem, vec![result, index]);
        }
    }

    Ok(result)
}

pub(crate) fn transform_unpack_items(node: Node, source: &str) -> TransformResult {
    // unpack_items always creates a tuple, even with one element
    // This preserves the explicit unpacking syntax (x,) vs x
    let children = named_children(&node);
    let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();
    Ok(IR::Tuple(transformed?))
}

/// Build a single assignment IR node from a transformed lvalue and rvalue.
fn build_assignment(lvalue: IR, rvalue: IR, start: usize, end: usize) -> IR {
    // SetAttr/SetItem for attribute/index targets
    if let IR::Op { opcode, args, .. } = &lvalue {
        if *opcode == IROpCode::GetAttr {
            let mut setattr_args = args.clone();
            setattr_args.push(rvalue);
            return IR::op_with_pos(IROpCode::SetAttr, setattr_args, start, end);
        }
        if *opcode == IROpCode::GetItem {
            let mut setitem_args = args.clone();
            setitem_args.push(rvalue);
            return IR::op_with_pos(IROpCode::SetItem, setitem_args, start, end);
        }
    }

    // SetLocals for identifiers and unpacking
    let (names_tuple, explicit_unpack) = match &lvalue {
        IR::Tuple(_) => (lvalue, IR::Bool(true)),
        _ => (IR::Tuple(vec![lvalue]), IR::Bool(false)),
    };
    IR::op_with_pos(
        IROpCode::SetLocals,
        vec![names_tuple, rvalue, explicit_unpack],
        start,
        end,
    )
}

pub(crate) fn transform_assignment(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    let num_decorators = children.iter().take_while(|c| c.kind() == "decorator").count();

    if children.len() < 2 {
        return Err("assignment requires at least lvalue and rvalue".into());
    }

    let non_dec = &children[num_decorators..];
    // Last child is always the rvalue, preceding children are lvalues
    let lvalue_nodes = &non_dec[..non_dec.len() - 1];
    let rvalue_node = *non_dec.last().unwrap();
    let mut rvalue = transform(rvalue_node, source)?;

    // Apply decorators bottom-up (only valid for single lvalue)
    if num_decorators > 0 {
        if lvalue_nodes.len() != 1 {
            return Err("decorators not supported on chained assignments".into());
        }
        for i in (0..num_decorators).rev() {
            let decorator = transform(children[i], source)?;
            rvalue = IR::call(decorator, vec![rvalue]);
        }
    }

    let start = node.start_byte();
    let end = node.end_byte();

    if lvalue_nodes.len() == 1 {
        // Simple assignment: x = value
        let lvalue = transform(lvalue_nodes[0], source)?;
        return Ok(build_assignment(lvalue, rvalue, start, end));
    }

    // Chained assignment: a = b = c = 42
    // Nest right-to-left: SET_LOCALS(a, SET_LOCALS(b, SET_LOCALS(c, 42)))
    // Each SET_LOCALS returns the assigned value, so it chains naturally.
    let mut expr = rvalue;
    for lv_node in lvalue_nodes.iter().rev() {
        let lvalue = transform(*lv_node, source)?;
        expr = build_assignment(lvalue, expr, start, end);
    }
    Ok(expr)
}
