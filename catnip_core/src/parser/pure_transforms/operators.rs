// FILE: catnip_core/src/parser/pure_transforms/operators.rs
use super::*;

// ============================================================================
// Operators
// ============================================================================

pub(crate) fn transform_additive(node: Node, source: &str) -> TransformResult {
    binary_op(node, source, &[("+", IROpCode::Add), ("-", IROpCode::Sub)])
}

pub(crate) fn transform_multiplicative(node: Node, source: &str) -> TransformResult {
    binary_op(
        node,
        source,
        &[
            ("*", IROpCode::Mul),
            ("/", IROpCode::TrueDiv),
            ("//", IROpCode::FloorDiv),
            ("%", IROpCode::Mod),
        ],
    )
}

pub(crate) fn transform_exponent(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.len() >= 2 {
        let left = transform(children[0], source)?;
        let right = transform(children[children.len() - 1], source)?;
        Ok(IR::op_with_pos(
            IROpCode::Pow,
            vec![left, right],
            node.start_byte(),
            node.end_byte(),
        ))
    } else if !children.is_empty() {
        transform(children[0], source)
    } else {
        Ok(IR::None)
    }
}

pub(crate) fn transform_comparison(node: Node, source: &str) -> TransformResult {
    // New grammar: comparison has alternating operands and comp_ops
    // Structure: operand (comp_op operand)+
    // Example: 1 < 2 < 3 -> operands=[1,2,3], ops=["<","<"]

    let mut operands = Vec::new();
    let mut operators = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "comp_op" {
            // Extract operator from comp_op node (may be multi-token, e.g. "not in")
            let mut op_cursor = child.walk();
            let mut op_parts = Vec::new();
            for op_child in child.children(&mut op_cursor) {
                if !op_child.is_named() {
                    let text = node_text(&op_child, source).trim();
                    if !text.is_empty() {
                        op_parts.push(text.to_string());
                    }
                }
            }
            if !op_parts.is_empty() {
                operators.push(op_parts.join(" "));
            }
        } else if child.is_named() {
            // Other named children are operands (expressions)
            operands.push(transform(child, source)?);
        }
    }

    if operands.len() < 2 || operators.is_empty() {
        return Err("comparison: invalid structure".into());
    }

    // Single comparison: a < b
    if operators.len() == 1 {
        let opcode = match operators[0].as_str() {
            ">" => IROpCode::Gt,
            "<" => IROpCode::Lt,
            ">=" => IROpCode::Ge,
            "<=" => IROpCode::Le,
            "==" => IROpCode::Eq,
            "!=" => IROpCode::Ne,
            "in" => IROpCode::In,
            "not in" => IROpCode::NotIn,
            "is" => IROpCode::Is,
            "is not" => IROpCode::IsNot,
            _ => return Err(format!("Unknown comparison operator: {}", operators[0])),
        };

        return Ok(IR::op_with_pos(
            opcode,
            vec![operands[0].clone(), operands[1].clone()],
            node.start_byte(),
            node.end_byte(),
        ));
    }

    // Chained comparison: a < b < c -> (a < b) and (b < c)
    let mut comparisons = Vec::new();
    for i in 0..operators.len() {
        let opcode = match operators[i].as_str() {
            ">" => IROpCode::Gt,
            "<" => IROpCode::Lt,
            ">=" => IROpCode::Ge,
            "<=" => IROpCode::Le,
            "==" => IROpCode::Eq,
            "!=" => IROpCode::Ne,
            "in" => IROpCode::In,
            "not in" => IROpCode::NotIn,
            "is" => IROpCode::Is,
            "is not" => IROpCode::IsNot,
            _ => return Err(format!("Unknown comparison operator: {}", operators[i])),
        };

        let comp = IR::op(opcode, vec![operands[i].clone(), operands[i + 1].clone()]);
        comparisons.push(comp);
    }

    // Combine with AND
    let mut result = comparisons[0].clone();
    for comp in &comparisons[1..] {
        result = IR::op(IROpCode::And, vec![result, comp.clone()]);
    }

    Ok(result)
}

// Logical operators
pub(crate) fn transform_bool_and(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::And)
}

pub(crate) fn transform_bool_or(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::Or)
}

pub(crate) fn transform_null_coalesce(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::NullCoalesce)
}

pub(crate) fn transform_bool_not(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if !children.is_empty() {
        let operand = transform(children[children.len() - 1], source)?;
        Ok(IR::op(IROpCode::Not, vec![operand]))
    } else {
        Ok(IR::None)
    }
}

// Bitwise operators
pub(crate) fn transform_bit_and(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::BAnd)
}

pub(crate) fn transform_bit_or(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::BOr)
}

pub(crate) fn transform_bit_xor(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::BXor)
}

pub(crate) fn transform_bit_not(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if !children.is_empty() {
        let operand = transform(children[children.len() - 1], source)?;
        Ok(IR::op(IROpCode::BNot, vec![operand]))
    } else {
        Ok(IR::None)
    }
}

pub(crate) fn transform_shift(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.len() < 2 {
        return if !children.is_empty() {
            transform(children[0], source)
        } else {
            Ok(IR::None)
        };
    }

    // Extract operator
    let mut op_text = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "shift_op" {
            let mut op_cursor = child.walk();
            for op_child in child.children(&mut op_cursor) {
                if !op_child.is_named() {
                    let text = node_text(&op_child, source).trim();
                    if !text.is_empty() {
                        op_text = text.to_string();
                        break;
                    }
                }
            }
            if !op_text.is_empty() {
                break;
            }
        }
    }

    let opcode = match op_text.as_str() {
        "<<" => IROpCode::LShift,
        ">>" => IROpCode::RShift,
        _ => return Err(format!("Unknown shift operator: {}", op_text)),
    };

    let left = transform(children[0], source)?;
    let right = transform(children[children.len() - 1], source)?;

    Ok(IR::op_with_pos(
        opcode,
        vec![left, right],
        node.start_byte(),
        node.end_byte(),
    ))
}

// Helper for binary logical/bitwise operations
fn binary_logical(node: Node, source: &str, opcode: IROpCode) -> TransformResult {
    let children = named_children(&node);

    if children.len() >= 2 {
        let left = transform(children[0], source)?;
        let right = transform(children[children.len() - 1], source)?;
        Ok(IR::op_with_pos(
            opcode,
            vec![left, right],
            node.start_byte(),
            node.end_byte(),
        ))
    } else if !children.is_empty() {
        transform(children[0], source)
    } else {
        Ok(IR::None)
    }
}

pub(crate) fn transform_unary(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.is_empty() {
        return Ok(IR::None);
    }

    // Find operator - check both named (unary_op) and unnamed children
    let mut op_text = String::new();

    // First try to find unary_op child (named)
    for child in &children {
        if child.kind() == "unary_op" {
            // Extract operator from unary_op's unnamed children
            let mut op_cursor = child.walk();
            for op_child in child.children(&mut op_cursor) {
                if !op_child.is_named() {
                    let text = node_text(&op_child, source).trim();
                    if !text.is_empty() {
                        op_text = text.to_string();
                        break;
                    }
                }
            }
            if !op_text.is_empty() {
                break;
            }
        }
    }

    // Fallback: check unnamed children of current node
    if op_text.is_empty() {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                let text = node_text(&child, source).trim();
                if !text.is_empty() && text != "(" && text != ")" {
                    op_text = text.to_string();
                    break;
                }
            }
        }
    }

    // Map operator
    let opcode = match op_text.as_str() {
        "-" => IROpCode::Neg,
        "+" => IROpCode::Pos,
        "~" => IROpCode::BNot,
        _ => return transform(children[children.len() - 1], source),
    };

    // Transform operand (last named child)
    let operand = transform(children[children.len() - 1], source)?;

    Ok(IR::op_with_pos(
        opcode,
        vec![operand],
        node.start_byte(),
        node.end_byte(),
    ))
}

fn binary_op(node: Node, source: &str, ops_map: &[(&str, IROpCode)]) -> TransformResult {
    let children = named_children(&node);

    if children.len() < 2 {
        if children.is_empty() {
            return Ok(IR::None);
        }
        return transform(children[0], source);
    }

    // Find operator
    let mut op_text = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind.ends_with("_op") {
            let mut op_cursor = child.walk();
            for op_child in child.children(&mut op_cursor) {
                let text = node_text(&op_child, source);
                if !text.trim().is_empty() {
                    op_text = text.to_string();
                    break;
                }
            }
            if !op_text.is_empty() {
                break;
            }
        } else if !child.is_named() {
            let text = node_text(&child, source);
            if !text.trim().is_empty() && text != "(" && text != ")" {
                op_text = text.to_string();
                break;
            }
        }
    }

    // Map operator to opcode
    let opcode = ops_map
        .iter()
        .find(|(op_str, _)| *op_str == op_text)
        .map(|(_, op)| *op)
        .ok_or_else(|| format!("Unknown operator: {}", op_text))?;

    // Transform operands
    let left = transform(children[0], source)?;
    let right = transform(children[children.len() - 1], source)?;

    Ok(IR::op_with_pos(
        opcode,
        vec![left, right],
        node.start_byte(),
        node.end_byte(),
    ))
}
