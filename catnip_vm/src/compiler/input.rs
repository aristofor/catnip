// FILE: catnip_vm/src/compiler/input.rs
//! IR helper functions for the pure Rust compiler.
//!
//! Extracted from catnip_rs/src/vm/compiler_input.rs (Pure-only variants).

use catnip_core::ir::opcode::IROpCode;
use catnip_core::ir::pure::IR;

/// Convert an IR node to a variable name.
pub fn ir_to_name(node: &IR) -> Option<String> {
    match node {
        IR::Ref(name, _, _) | IR::Identifier(name) | IR::String(name) => Some(name.clone()),
        _ => None,
    }
}

/// Check if an IR pattern contains star or nested patterns.
pub fn has_complex_pattern_ir(pattern: &IR) -> bool {
    // Unwrap single-element tuple wrapper
    let actual = if let IR::Tuple(items) = pattern {
        if items.len() == 1 {
            if let IR::Tuple(_) = &items[0] {
                &items[0]
            } else {
                return false;
            }
        } else {
            pattern
        }
    } else {
        return false;
    };

    if let IR::Tuple(items) = actual {
        for item in items {
            if let IR::Tuple(pair) = item {
                // Star pattern: ("*", name)
                if pair.len() == 2 {
                    if let IR::String(s) = &pair[0] {
                        if s == "*" {
                            return true;
                        }
                    }
                }
                // Nested pattern: a tuple of Refs/Identifiers/Tuples
                if !pair.is_empty() {
                    let is_nested = pair
                        .iter()
                        .all(|nested| matches!(nested, IR::Ref(_, _, _) | IR::Identifier(_) | IR::Tuple(_)));
                    if is_nested {
                        return true;
                    }
                }
            } else if matches!(item, IR::List(_)) {
                return true;
            }
        }
    }
    false
}

/// Recursively extract variable names from an IR pattern.
fn extract_names_recursive_ir(pattern: &IR, names: &mut Vec<String>) {
    match pattern {
        IR::Tuple(items) => {
            for item in items {
                if matches!(item, IR::Tuple(_)) {
                    extract_names_recursive_ir(item, names);
                } else if let Some(name) = ir_to_name(item) {
                    names.push(name);
                }
            }
        }
        _ => {
            if let Some(name) = ir_to_name(pattern) {
                names.push(name);
            }
        }
    }
}

/// Extract flat variable names from a pattern.
pub fn extract_names_ir(pattern: &IR) -> Vec<String> {
    let mut names = Vec::new();
    extract_names_recursive_ir(pattern, &mut names);
    names
}

/// Unwrap a single-element tuple wrapper: ((items...),) -> (items...)
pub fn unwrap_single_tuple(ir: &IR) -> &IR {
    if let IR::Tuple(items) = ir {
        if items.len() == 1 {
            if let IR::Tuple(_) = &items[0] {
                return &items[0];
            }
        }
    }
    ir
}

/// Check if an IR node is None/nil.
pub fn is_none_ir(ir: &IR) -> bool {
    matches!(ir, IR::None)
}

/// Try to extract a negated literal: Op(NEG, [int]) -> -int.
pub fn try_extract_neg_literal_ir(ir: &IR) -> Option<i64> {
    if let IR::Op {
        opcode: IROpCode::Neg,
        args,
        ..
    } = ir
    {
        if args.len() == 1 {
            if let IR::Int(n) = &args[0] {
                return Some(-n);
            }
        }
    }
    None
}

/// Check if this is a call to `range()`.
pub fn is_range_call_ir(ir: &IR) -> bool {
    match ir {
        IR::Op {
            opcode: IROpCode::Call,
            args,
            ..
        } => {
            if args.len() >= 2 {
                if let IR::Ref(name, _, _) | IR::Identifier(name) = &args[0] {
                    return name == "range";
                }
            }
            false
        }
        IR::Call { func, .. } => {
            if let IR::Ref(name, _, _) | IR::Identifier(name) = func.as_ref() {
                return name == "range";
            }
            false
        }
        _ => false,
    }
}

/// Extract range call arguments (everything after the func ref).
pub fn range_call_args_ir(ir: &IR) -> Option<&[IR]> {
    match ir {
        IR::Op { args, .. } => {
            if args.len() >= 2 {
                Some(&args[1..])
            } else {
                None
            }
        }
        IR::Call { args, .. } => Some(args),
        _ => None,
    }
}

/// Check if an IR node is an Op with the given opcode.
pub fn is_op_ir(ir: &IR, opcode: IROpCode) -> bool {
    matches!(ir, IR::Op { opcode: op, .. } if *op == opcode)
}

/// If this is an OpBlock, return its inner statements.
pub fn as_block_contents_ir(ir: &IR) -> Option<&[IR]> {
    if let IR::Op {
        opcode: IROpCode::OpBlock,
        args,
        ..
    } = ir
    {
        Some(args)
    } else {
        None
    }
}

/// For GetAttr Op nodes: extract (object, method_name).
pub fn as_getattr_parts_ir(ir: &IR) -> Option<(&IR, String)> {
    if let IR::Op {
        opcode: IROpCode::GetAttr,
        args,
        ..
    } = ir
    {
        if args.len() >= 2 {
            let method_name = match &args[1] {
                IR::String(s) | IR::Identifier(s) | IR::Ref(s, _, _) => s.clone(),
                _ => return None,
            };
            return Some((&args[0], method_name));
        }
    }
    None
}
