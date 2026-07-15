// FILE: catnip_core/src/parser/pure_transforms/broadcast.rs
use super::*;

// ============================================================================
// Broadcasting & Chained Access
// ============================================================================

pub(crate) fn transform_chained(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.is_empty() {
        return Ok(IR::None);
    }

    // Transform first child as base
    let mut result = transform(children[0], source)?;

    // Process remaining children as chained operations
    for child in &children[1..] {
        match child.kind() {
            "call_member" => {
                // call_member: direct call on previous result
                let mut args = Vec::new();
                let mut kwargs = IndexMap::new();

                for arg_node in named_children(child) {
                    if arg_node.kind() == "arguments" {
                        let (parsed_args, parsed_kwargs) = parse_arguments(&arg_node, source)?;
                        args = parsed_args;
                        kwargs = parsed_kwargs;
                    }
                }

                result = IR::Call {
                    func: Box::new(result),
                    args,
                    kwargs,
                    tail: false,
                    start_byte: child.start_byte(),
                    end_byte: child.end_byte(),
                };
            }
            "callattr" => {
                // callattr: .method(args) - extract method name and call it
                let mut method_name = String::new();
                let mut args = Vec::new();
                let mut kwargs = IndexMap::new();

                for callattr_child in named_children(child) {
                    match callattr_child.kind() {
                        "identifier" => {
                            method_name = node_text(&callattr_child, source).to_string();
                        }
                        "arguments" => {
                            let (parsed_args, parsed_kwargs) = parse_arguments(&callattr_child, source)?;
                            args = parsed_args;
                            kwargs = parsed_kwargs;
                        }
                        _ => {}
                    }
                }

                // Create GetAttr to get the method
                let method = IR::op(IROpCode::GetAttr, vec![result, IR::String(method_name)]);

                // Create Call with method as function
                result = IR::Call {
                    func: Box::new(method),
                    args,
                    kwargs,
                    tail: false,
                    start_byte: child.start_byte(),
                    end_byte: child.end_byte(),
                };
            }
            "broadcast" => {
                result = apply_broadcast(result, *child, source)?;
            }
            "getattr" => {
                // getattr: .attribute - extract attribute name
                let children = named_children(child);
                if !children.is_empty() && children[0].kind() == "identifier" {
                    let attr_name = node_text(&children[0], source).to_string();
                    result = IR::op(IROpCode::GetAttr, vec![result, IR::String(attr_name)]);
                }
            }
            "index" => {
                // GetItem: transform_index handles single and tuple subscripts
                // (arr[y, x] -> tuple key), so route through it rather than
                // reading only the first index child.
                let index = transform_index(*child, source)?;
                result = IR::op(IROpCode::GetItem, vec![result, index]);
            }
            "fullslice" => {
                // fullslice: .[slice_range] -> GetItem with slice
                let fullslice_children = named_children(child);
                if !fullslice_children.is_empty() {
                    let slice = transform(fullslice_children[0], source)?;
                    result = IR::op(IROpCode::GetItem, vec![result, slice]);
                }
            }
            _ => {
                // Unknown chained operation, skip
            }
        }
    }

    Ok(result)
}

fn apply_broadcast(target: IR, node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.is_empty() {
        return Err("broadcast: no children".into());
    }

    let broadcast_op_node = children[0];
    let op_kind = broadcast_op_node.kind();

    // If op_kind is broadcast_op (wrapper), descend to actual operation
    let (final_node, final_kind) = if op_kind == "broadcast_op" {
        let op_children = named_children(&broadcast_op_node);
        if !op_children.is_empty() {
            let inner = op_children[0];
            (inner, inner.kind())
        } else {
            (broadcast_op_node, op_kind)
        }
    } else {
        (broadcast_op_node, op_kind)
    };

    match final_kind {
        "broadcast_binary" => {
            let bc_children = named_children(&final_node);
            if bc_children.len() < 2 {
                return Err("broadcast_binary: not enough children".into());
            }
            let op_node = bc_children[0];
            let operand_node = bc_children[1];
            let operator = node_text(&op_node, source);
            let operand = transform(operand_node, source)?;

            // Binary broadcast: target.[op operand]
            Ok(IR::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(IR::String(operator.to_string())),
                operand: Some(Box::new(operand)),
                broadcast_type: BroadcastType::Binary,
            })
        }
        "broadcast_unary" => {
            // Unary operators like .[!] or .[~]
            let bc_children = named_children(&final_node);
            if bc_children.is_empty() {
                return Err("broadcast_unary: no operator".into());
            }
            let op_node = bc_children[0];
            let operator = node_text(&op_node, source);

            // Unary broadcast: target.[op]
            Ok(IR::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(IR::String(operator.to_string())),
                operand: None,
                broadcast_type: BroadcastType::Unary,
            })
        }
        "broadcast_if" => {
            // Filter: .[if condition]
            let bc_children = named_children(&final_node);
            if bc_children.is_empty() {
                return Err("broadcast_if: no condition".into());
            }

            let condition_node = bc_children[0];
            let condition_kind = condition_node.kind();

            match condition_kind {
                "broadcast_binary" => {
                    // Extract operator and operand from binary condition
                    let binary_children = named_children(&condition_node);
                    if binary_children.len() < 2 {
                        return Err("broadcast_if binary: not enough children".into());
                    }

                    let op_node = binary_children[0];
                    let operand_node = binary_children[1];
                    let operator = node_text(&op_node, source);
                    let operand = transform(operand_node, source)?;

                    // Filter broadcast: target.[if op operand]
                    Ok(IR::Broadcast {
                        target: Some(Box::new(target)),
                        operator: Box::new(IR::String(operator.to_string())),
                        operand: Some(Box::new(operand)),
                        broadcast_type: BroadcastType::If,
                    })
                }
                "broadcast_unary" => {
                    // Unary condition
                    let operator = node_text(&condition_node, source);

                    // Filter broadcast: target.[if op]
                    Ok(IR::Broadcast {
                        target: Some(Box::new(target)),
                        operator: Box::new(IR::String(operator.to_string())),
                        operand: None,
                        broadcast_type: BroadcastType::If,
                    })
                }
                _ => {
                    // Expression (lambda or function)
                    let expr = transform(condition_node, source)?;

                    // Filter broadcast with lambda: target.[if lambda]
                    Ok(IR::Broadcast {
                        target: Some(Box::new(target)),
                        operator: Box::new(expr),
                        operand: None,
                        broadcast_type: BroadcastType::If,
                    })
                }
            }
        }
        "broadcast_nd_recursion" => {
            // ND recursion: .[~~lambda]
            // Registry expects ND_RECURSION with (data_or_seed, lambda_node)
            // In broadcast form: (lambda, None) for declaration-like form
            let bc_children = named_children(&final_node);
            if bc_children.is_empty() {
                return Err("broadcast_nd_recursion: no lambda".into());
            }

            let lambda_node = bc_children[0];
            let lambda = transform(lambda_node, source)?;

            // Use IR::Broadcast structure with NDRecursion type
            Ok(IR::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(IR::op(IROpCode::NdRecursion, vec![lambda, IR::None])),
                operand: None,
                broadcast_type: BroadcastType::NDRecursion,
            })
        }
        "broadcast" => {
            // Nested broadcast: target.[.[op]]
            // Desugar to lambda: target.[(x) => { x.[op] }]
            let param = "__bcast_x__";
            let inner = apply_broadcast(IR::Ref(param.to_string(), -1, -1), final_node, source)?;
            let lambda = IR::op(
                IROpCode::OpLambda,
                vec![
                    IR::Tuple(vec![IR::Tuple(vec![IR::String(param.to_string()), IR::None])]),
                    inner,
                ],
            );
            Ok(IR::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(lambda),
                operand: None,
                broadcast_type: BroadcastType::Lambda,
            })
        }
        _ => {
            // Default: treat as expression (mask, lambda, or other)
            let expr = transform(final_node, source)?;

            // Lambda broadcast: target.[lambda]
            Ok(IR::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(expr),
                operand: None,
                broadcast_type: BroadcastType::Lambda,
            })
        }
    }
}
