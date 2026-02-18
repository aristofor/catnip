// FILE: catnip_rs/src/parser/pure_transforms.rs
//! Pure Rust transformers - Tree-sitter → IRPure (no Python dependencies)
//!
//! Port of the existing transformers for the standalone pipeline.

use super::utils::{named_children, node_text, unescape_string};
use crate::ir::{BroadcastType, IROpCode, IRPure};
use std::collections::HashMap;
use tree_sitter::Node;

/// Result type for transform operations
pub type TransformResult = Result<IRPure, String>;

/// Parse arguments from an `arguments` tree-sitter node into (args, kwargs).
///
/// Handles all grammar node types: args, kwargs, args_kwargs, kwarg, keyword_argument.
fn parse_arguments(
    arguments_node: &Node,
    source: &str,
) -> Result<(Vec<IRPure>, HashMap<String, IRPure>), String> {
    let mut args = Vec::new();
    let mut kwargs = HashMap::new();

    for arg_child in named_children(arguments_node) {
        let arg_kind = arg_child.kind();

        if arg_kind == "args_kwargs" {
            for item in named_children(&arg_child) {
                let item_kind = item.kind();
                if item_kind == "kwarg" || item_kind == "keyword_argument" {
                    let grandchildren: Vec<_> = named_children(&item);
                    if grandchildren.len() == 2 {
                        let key = node_text(&grandchildren[0], source).to_string();
                        let value = transform(grandchildren[1], source)?;
                        kwargs.insert(key, value);
                    }
                } else {
                    args.push(transform(item, source)?);
                }
            }
        } else if arg_kind == "kwargs" {
            for kwarg_item in named_children(&arg_child) {
                if kwarg_item.kind() == "kwarg" || kwarg_item.kind() == "keyword_argument" {
                    let grandchildren: Vec<_> = named_children(&kwarg_item);
                    if grandchildren.len() == 2 {
                        let key = node_text(&grandchildren[0], source).to_string();
                        let value = transform(grandchildren[1], source)?;
                        kwargs.insert(key, value);
                    }
                }
            }
        } else if arg_kind == "kwarg" || arg_kind == "keyword_argument" {
            let grandchildren: Vec<_> = named_children(&arg_child);
            if grandchildren.len() == 2 {
                let key = node_text(&grandchildren[0], source).to_string();
                let value = transform(grandchildren[1], source)?;
                kwargs.insert(key, value);
            }
        } else if arg_kind == "args" {
            for inner_arg in named_children(&arg_child) {
                args.push(transform(inner_arg, source)?);
            }
        } else {
            args.push(transform(arg_child, source)?);
        }
    }

    Ok((args, kwargs))
}

/// Main transform dispatcher (standalone)
pub fn transform(node: Node, source: &str) -> TransformResult {
    let kind = node.kind();

    match kind {
        // Literals
        "number" => transform_number(node, source),
        "string" => transform_string(node, source),
        "fstring" => transform_fstring(node, source),
        "true" | "false" => transform_bool(node, source),
        "none" => Ok(IRPure::None),
        "nd_empty_topos" => transform_nd_empty_topos(node, source),

        // Operators MVP
        "additive" => transform_additive(node, source),
        "multiplicative" => transform_multiplicative(node, source),
        "exponent" => transform_exponent(node, source),
        "unary" => transform_unary(node, source),
        "comparison" => transform_comparison(node, source),
        "bool_and" => transform_bool_and(node, source),
        "bool_or" => transform_bool_or(node, source),
        "bool_not" => transform_bool_not(node, source),
        "bit_and" => transform_bit_and(node, source),
        "bit_or" => transform_bit_or(node, source),
        "bit_xor" => transform_bit_xor(node, source),
        "bit_not" => transform_bit_not(node, source),
        "shift" => transform_shift(node, source),

        // Control flow MVP
        "if_expr" => transform_if_expr(node, source),
        "elif_clause" => transform_elif_clause(node, source),
        "else_clause" => transform_else_clause(node, source),
        "while_stmt" => transform_while_stmt(node, source),
        "for_stmt" => transform_for_stmt(node, source),
        "return_stmt" => transform_return_stmt(node, source),
        "break_stmt" => transform_break_stmt(node, source),
        "continue_stmt" => transform_continue_stmt(node, source),
        "match_expr" => transform_match_expr(node, source),
        "match_case" => transform_match_case(node, source),

        // Pattern matching
        "pattern" => transform_pattern(node, source),
        "pattern_literal" => transform_pattern_literal(node, source),
        "pattern_var" => transform_pattern_var(node, source),
        "pattern_wildcard" => transform_pattern_wildcard(node, source),
        "pattern_or" => transform_pattern_or(node, source),
        "pattern_tuple" => transform_pattern_tuple(node, source),
        "pattern_star" => transform_pattern_star(node, source),
        "pattern_struct" => transform_pattern_struct(node, source),

        // Variables
        "identifier" => transform_identifier(node, source),
        "assignment" => transform_assignment(node, source),
        "setattr" => transform_setattr_lvalue(node, source),
        "variadic_param" => transform_variadic_param(node, source),
        "unpack_items" => transform_unpack_items(node, source),

        // ND operations
        "nd_recursion" => transform_nd_recursion(node, source),
        "nd_map" => transform_nd_map(node, source),

        // Collections
        "list_literal" => transform_list_literal(node, source),
        "tuple_literal" => transform_tuple_literal(node, source),
        "dict_literal" => transform_dict_literal(node, source),
        "set_literal" => transform_set_literal(node, source),

        // Function call & Lambda
        "call" => transform_call(node, source),
        "lambda_expr" => transform_lambda_expr(node, source),

        // Pragma directive
        "pragma_stmt" => transform_pragma_stmt(node, source),

        // Struct
        "struct_stmt" => transform_struct_stmt(node, source),

        // Trait
        "trait_stmt" => transform_trait_stmt(node, source),

        // Block
        "block" => transform_block(node, source),

        // Access & Broadcasting
        "index" => transform_index(node, source),
        "slice_range" => transform_slice_range(node, source),
        "fullslice" => transform_fullslice(node, source),
        "broadcast" => transform_broadcast(node, source),

        // Chained operations (call_member, getattr, index, broadcast)
        "chained" => transform_chained(node, source),

        // Source file
        "source_file" => transform_source_file(node, source),

        // Fallback: transform children
        _ => transform_children(node, source),
    }
}

// ============================================================================
// Literals
// ============================================================================

fn transform_number(node: Node, source: &str) -> TransformResult {
    let text = node_text(&node, source);

    // Try int first
    if let Ok(val) = text.parse::<i64>() {
        return Ok(IRPure::Int(val));
    }

    // Try float
    if let Ok(val) = text.parse::<f64>() {
        return Ok(IRPure::Float(val));
    }

    Err(format!("Invalid number: {}", text))
}

fn transform_string(node: Node, source: &str) -> TransformResult {
    let text = node_text(&node, source);

    // Check for triple-quoted strings
    let (start_offset, end_offset) = if text.starts_with("\"\"\"") || text.starts_with("'''") {
        if text.len() < 6 {
            return Err("Triple-quoted string literal too short".into());
        }
        (3, 3)
    } else {
        if text.len() < 2 {
            return Err("String literal too short".into());
        }
        (1, 1)
    };

    let content = &text[start_offset..text.len() - end_offset];
    let unescaped = unescape_string(content);

    Ok(IRPure::String(unescaped))
}

fn transform_fstring(node: Node, source: &str) -> TransformResult {
    let mut parts = Vec::new();
    let children = named_children(&node);

    // Collect all parts (literal text and interpolations)
    for child in &children {
        match child.kind() {
            // Literal text nodes
            "fstring_text_double"
            | "fstring_text_single"
            | "fstring_text_long_double"
            | "fstring_text_long_single" => {
                let text = node_text(child, source);
                let unescaped = unescape_string(text);
                parts.push(IRPure::String(unescaped));
            }
            // Interpolation nodes {expr}, {expr:spec}, {expr!r}, {expr=}
            "interpolation" => {
                let interp_children = named_children(child);
                let mut expr_node = None;
                let mut format_spec = None;
                let mut conversion = None;
                let mut is_debug = false;

                for ic in &interp_children {
                    match ic.kind() {
                        "fstring_debug" => is_debug = true,
                        "fstring_conversion" => {
                            conversion = Some(node_text(ic, source).to_string());
                        }
                        "format_spec" => {
                            format_spec = Some(node_text(ic, source).to_string());
                        }
                        _ => {
                            if expr_node.is_none() {
                                expr_node = Some(*ic);
                            }
                        }
                    }
                }

                if let Some(expr_n) = expr_node {
                    let expr = transform(expr_n, source)?;

                    // Apply conversion flag (repr/str/ascii)
                    let has_conversion = conversion.is_some();
                    let value = if let Some(ref conv) = conversion {
                        let func_name = match conv.as_str() {
                            "r" => "repr",
                            "a" => "ascii",
                            _ => "str",
                        };
                        IRPure::Call {
                            func: Box::new(IRPure::Ref(func_name.to_string())),
                            args: vec![expr],
                            kwargs: HashMap::new(),
                            start_byte: 0,
                            end_byte: 0,
                        }
                    } else {
                        expr
                    };

                    // Apply format spec or default str()
                    let result = if let Some(ref spec) = format_spec {
                        IRPure::Call {
                            func: Box::new(IRPure::Ref("format".to_string())),
                            args: vec![value, IRPure::String(spec.clone())],
                            kwargs: HashMap::new(),
                            start_byte: 0,
                            end_byte: 0,
                        }
                    } else if !has_conversion {
                        IRPure::Call {
                            func: Box::new(IRPure::Ref("str".to_string())),
                            args: vec![value],
                            kwargs: HashMap::new(),
                            start_byte: 0,
                            end_byte: 0,
                        }
                    } else {
                        // Conversion already returns a string
                        value
                    };

                    // Debug: prepend "expr_source="
                    if is_debug {
                        let expr_text = &source[child.start_byte() + 1..expr_n.end_byte()];
                        parts.push(IRPure::String(format!("{}=", expr_text)));
                    }

                    parts.push(result);
                }
            }
            "escape_sequence" => {
                let text = node_text(child, source);
                let unescaped = unescape_string(text);
                parts.push(IRPure::String(unescaped));
            }
            _ => {}
        }
    }

    // Check for trailing text after last child (before closing delimiter)
    // This handles cases like f"""{x}\n""" where \n is part of the closing token
    if let Some(last_child) = children.last() {
        let last_child_end = last_child.end_byte();
        let node_end = node.end_byte();

        // Extract text between last child and node end
        if last_child_end < node_end {
            let trailing = &source[last_child_end..node_end];
            // Remove closing delimiter (""" or ''' or " or ')
            let trimmed = trailing
                .trim_end_matches("\"\"\"")
                .trim_end_matches("'''")
                .trim_end_matches('"')
                .trim_end_matches('\'');

            if !trimmed.is_empty() {
                let unescaped = unescape_string(trimmed);
                parts.push(IRPure::String(unescaped));
            }
        }
    }

    // If empty, return empty string
    if parts.is_empty() {
        return Ok(IRPure::String(String::new()));
    }

    // If single part, return it directly
    if parts.len() == 1 {
        return Ok(parts.into_iter().next().unwrap());
    }

    // Concatenate all parts with Add operations (left-associative)
    let mut result = parts[0].clone();
    for part in &parts[1..] {
        result = IRPure::op(IROpCode::Add, vec![result, part.clone()]);
    }

    Ok(result)
}

fn transform_bool(node: Node, source: &str) -> TransformResult {
    let text = node_text(&node, source);
    match text {
        "True" => Ok(IRPure::Bool(true)),
        "False" => Ok(IRPure::Bool(false)),
        _ => Err(format!("Invalid bool literal: {}", text)),
    }
}

fn transform_nd_empty_topos(_node: Node, _source: &str) -> TransformResult {
    // @[] - empty non-deterministic topos
    Ok(IRPure::op(IROpCode::NdEmptyTopos, vec![]))
}

fn transform_nd_recursion(node: Node, source: &str) -> TransformResult {
    // ND-recursion: @@(seed, lambda) or @@ lambda
    // Registry expects: (data_or_seed, lambda_node)
    // Forms:
    // - @@(seed, lambda): Combinator form - (seed, lambda)
    // - @@ lambda: Declaration form - (lambda, None)
    let children = named_children(&node);

    for child in children {
        let kind = child.kind();
        if kind == "arguments" {
            // Combinator form: @@(seed, lambda)
            // Extract args from arguments node
            let mut args = Vec::new();
            for arg_child in named_children(&child) {
                if arg_child.kind() == "args" {
                    for item in named_children(&arg_child) {
                        args.push(transform(item, source)?);
                    }
                } else if arg_child.kind() != "(" && arg_child.kind() != ")" {
                    args.push(transform(arg_child, source)?);
                }
            }
            return Ok(IRPure::op(IROpCode::NdRecursion, args));
        } else if kind == "lambda_expr" {
            // Declaration form: @@ lambda → (lambda, None)
            let lambda = transform(child, source)?;
            return Ok(IRPure::op(
                IROpCode::NdRecursion,
                vec![lambda, IRPure::None],
            ));
        }
    }

    Err("nd_recursion: no arguments or lambda found".into())
}

fn transform_nd_map(node: Node, source: &str) -> TransformResult {
    // ND-map: @>(data, f) or @> f
    // Registry expects: (data_or_func, func_node)
    // Forms:
    // - @>(data, f): Applicative form - (data, f)
    // - @> f: Lift form - (f, None)
    let children = named_children(&node);

    for child in children {
        let kind = child.kind();
        if kind == "arguments" {
            // Applicative form: @>(data, f)
            let mut args = Vec::new();
            for arg_child in named_children(&child) {
                if arg_child.kind() == "args" {
                    for item in named_children(&arg_child) {
                        args.push(transform(item, source)?);
                    }
                } else if arg_child.kind() != "(" && arg_child.kind() != ")" {
                    args.push(transform(arg_child, source)?);
                }
            }
            return Ok(IRPure::op(IROpCode::NdMap, args));
        } else if kind == "lambda_expr" || kind.starts_with("_") || kind == "identifier" {
            // Lift form: @> f → (f, None)
            let func = transform(child, source)?;
            return Ok(IRPure::op(IROpCode::NdMap, vec![func, IRPure::None]));
        }
    }

    Err("nd_map: no arguments or function found".into())
}

// ============================================================================
// Operators
// ============================================================================

fn transform_additive(node: Node, source: &str) -> TransformResult {
    binary_op(node, source, &[("+", IROpCode::Add), ("-", IROpCode::Sub)])
}

fn transform_multiplicative(node: Node, source: &str) -> TransformResult {
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

fn transform_exponent(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.len() >= 2 {
        let left = transform(children[0], source)?;
        let right = transform(children[children.len() - 1], source)?;
        Ok(IRPure::op_with_pos(
            IROpCode::Pow,
            vec![IRPure::Tuple(vec![left, right])],
            node.start_byte(),
            node.end_byte(),
        ))
    } else if !children.is_empty() {
        transform(children[0], source)
    } else {
        Ok(IRPure::None)
    }
}

fn transform_comparison(node: Node, source: &str) -> TransformResult {
    // New grammar: comparison has alternating operands and comp_ops
    // Structure: operand (comp_op operand)+
    // Example: 1 < 2 < 3 -> operands=[1,2,3], ops=["<","<"]

    let mut operands = Vec::new();
    let mut operators = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "comp_op" {
            // Extract operator from comp_op node
            let mut op_cursor = child.walk();
            for op_child in child.children(&mut op_cursor) {
                if !op_child.is_named() {
                    let op_text = node_text(&op_child, source).trim();
                    if !op_text.is_empty() {
                        operators.push(op_text.to_string());
                        break;
                    }
                }
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
            _ => return Err(format!("Unknown comparison operator: {}", operators[0])),
        };

        return Ok(IRPure::op_with_pos(
            opcode,
            vec![IRPure::Tuple(vec![
                operands[0].clone(),
                operands[1].clone(),
            ])],
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
            _ => return Err(format!("Unknown comparison operator: {}", operators[i])),
        };

        let comp = IRPure::op(
            opcode,
            vec![IRPure::Tuple(vec![
                operands[i].clone(),
                operands[i + 1].clone(),
            ])],
        );
        comparisons.push(comp);
    }

    // Combine with AND
    let mut result = comparisons[0].clone();
    for comp in &comparisons[1..] {
        result = IRPure::op(
            IROpCode::And,
            vec![IRPure::Tuple(vec![result, comp.clone()])],
        );
    }

    Ok(result)
}

// Logical operators
fn transform_bool_and(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::And)
}

fn transform_bool_or(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::Or)
}

fn transform_bool_not(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if !children.is_empty() {
        let operand = transform(children[children.len() - 1], source)?;
        Ok(IRPure::op(IROpCode::Not, vec![operand]))
    } else {
        Ok(IRPure::None)
    }
}

// Bitwise operators
fn transform_bit_and(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::BAnd)
}

fn transform_bit_or(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::BOr)
}

fn transform_bit_xor(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::BXor)
}

fn transform_bit_not(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if !children.is_empty() {
        let operand = transform(children[children.len() - 1], source)?;
        Ok(IRPure::op(IROpCode::BNot, vec![operand]))
    } else {
        Ok(IRPure::None)
    }
}

fn transform_shift(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.len() < 2 {
        return if !children.is_empty() {
            transform(children[0], source)
        } else {
            Ok(IRPure::None)
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

    Ok(IRPure::op_with_pos(
        opcode,
        vec![IRPure::Tuple(vec![left, right])],
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
        Ok(IRPure::op_with_pos(
            opcode,
            vec![IRPure::Tuple(vec![left, right])],
            node.start_byte(),
            node.end_byte(),
        ))
    } else if !children.is_empty() {
        transform(children[0], source)
    } else {
        Ok(IRPure::None)
    }
}

fn transform_unary(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.is_empty() {
        return Ok(IRPure::None);
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

    Ok(IRPure::op_with_pos(
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
            return Ok(IRPure::None);
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

    Ok(IRPure::op_with_pos(
        opcode,
        vec![IRPure::Tuple(vec![left, right])],
        node.start_byte(),
        node.end_byte(),
    ))
}

// ============================================================================
// Control Flow
// ============================================================================

fn transform_if_expr(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.is_empty() {
        return Ok(IRPure::None);
    }

    let mut condition: Option<IRPure> = None;
    let mut consequence: Option<IRPure> = None;
    let mut elif_clauses: Vec<IRPure> = Vec::new();
    let mut else_block: Option<IRPure> = None;

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
    let mut branches: Vec<IRPure> = Vec::new();
    if let (Some(cond), Some(cons)) = (condition, consequence) {
        branches.push(IRPure::Tuple(vec![cond, cons]));
    }
    branches.extend(elif_clauses);

    // OpIf args: [Tuple(branches), optional_else]
    let branches_tuple = IRPure::Tuple(branches);
    let args = if let Some(else_val) = else_block {
        vec![branches_tuple, else_val]
    } else {
        vec![branches_tuple]
    };

    Ok(IRPure::op_with_pos(
        IROpCode::OpIf,
        args,
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_elif_clause(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    let mut condition: Option<IRPure> = None;
    let mut block: Option<IRPure> = None;

    for child in &children {
        if child.kind() == "block" {
            block = Some(transform(*child, source)?);
        } else if condition.is_none() {
            condition = Some(transform(*child, source)?);
        }
    }

    // Return tuple (condition, block)
    let condition_val = condition.unwrap_or(IRPure::None);
    let block_val = block.unwrap_or(IRPure::None);

    Ok(IRPure::Tuple(vec![condition_val, block_val]))
}

fn transform_else_clause(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // else_clause has one child: the block
    if !children.is_empty() {
        transform(children[0], source)
    } else {
        Ok(IRPure::None)
    }
}

fn transform_while_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.len() < 2 {
        return Err("while_stmt requires condition and body".into());
    }

    let condition = transform(children[0], source)?;
    let body = transform(children[1], source)?;

    Ok(IRPure::op_with_pos(
        IROpCode::OpWhile,
        vec![condition, body],
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_for_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    if children.len() < 3 {
        return Err("for_stmt requires var, iterable, and body".into());
    }

    let var = transform(children[0], source)?;
    let iterable = transform(children[1], source)?;
    let body = transform(children[2], source)?;

    Ok(IRPure::op_with_pos(
        IROpCode::OpFor,
        vec![var, iterable, body],
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_return_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    let value = if !children.is_empty() {
        transform(children[0], source)?
    } else {
        IRPure::None
    };

    Ok(IRPure::op(IROpCode::OpReturn, vec![value]))
}

fn transform_break_stmt(_node: Node, _source: &str) -> TransformResult {
    Ok(IRPure::op(IROpCode::OpBreak, vec![]))
}

fn transform_continue_stmt(_node: Node, _source: &str) -> TransformResult {
    Ok(IRPure::op(IROpCode::OpContinue, vec![]))
}

fn transform_block(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();

    Ok(IRPure::op_with_pos(
        IROpCode::OpBlock,
        transformed?,
        node.start_byte(),
        node.end_byte(),
    ))
}

// ============================================================================
// Variables
// ============================================================================

fn transform_identifier(node: Node, source: &str) -> TransformResult {
    let text = node_text(&node, source);
    // By default, identifiers are variable references (Ref)
    // Only use Identifier in specific contexts (attribute names, parameters, etc.)
    Ok(IRPure::Ref(text.to_string()))
}

fn transform_setattr_lvalue(node: Node, source: &str) -> TransformResult {
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
                result = IRPure::op(IROpCode::GetAttr, vec![result, IRPure::String(attr_name)]);
            }
        } else if child.kind() == "index" {
            let index_children = named_children(child);
            if !index_children.is_empty() {
                let index = transform(index_children[0], source)?;
                result = IRPure::op(IROpCode::GetItem, vec![result, index]);
            }
        }
    }

    Ok(result)
}

fn transform_unpack_items(node: Node, source: &str) -> TransformResult {
    // unpack_items always creates a tuple, even with one element
    // This preserves the explicit unpacking syntax (x,) vs x
    let children = named_children(&node);
    let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();
    Ok(IRPure::Tuple(transformed?))
}

fn transform_assignment(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // Handle decorated and undecorated assignments:
    // - Undecorated: [lvalue, rvalue]
    // - Decorated: [decorator+, lvalue, rvalue]
    // Count leading decorators
    let num_decorators = children
        .iter()
        .take_while(|c| c.kind() == "decorator")
        .count();

    if children.len() < 2 {
        return Err("assignment requires at least lvalue and rvalue".into());
    }

    let (lvalue, rvalue) = if num_decorators == 0 {
        // Normal assignment: x = value
        if children.len() != 2 {
            return Err(format!(
                "undecorated assignment expects 2 children, got {}",
                children.len()
            ));
        }
        (
            transform(children[0], source)?,
            transform(children[1], source)?,
        )
    } else {
        // Decorated assignment: @d1 @d2 ... x = value
        // Apply decorators bottom-up: x = d1(d2(...(value)))
        if children.len() < num_decorators + 2 {
            return Err("decorated assignment requires decorators, lvalue, and rvalue".into());
        }

        let lvalue = transform(children[num_decorators], source)?;
        let mut value = transform(children[num_decorators + 1], source)?;

        // Apply decorators in reverse order (bottom-up)
        for i in (0..num_decorators).rev() {
            let decorator = transform(children[i], source)?;
            value = IRPure::call(decorator, vec![value]);
        }

        (lvalue, value)
    };

    // Check if lvalue is an attribute access (GetAttr) or index access (GetItem)
    // If so, create SetAttr/SetItem instead of SetLocals
    if let IRPure::Op { opcode, args, .. } = &lvalue {
        if *opcode == IROpCode::GetAttr {
            let mut setattr_args = args.clone();
            setattr_args.push(rvalue);

            return Ok(IRPure::op_with_pos(
                IROpCode::SetAttr,
                setattr_args,
                node.start_byte(),
                node.end_byte(),
            ));
        }

        if *opcode == IROpCode::GetItem {
            let mut setitem_args = args.clone();
            setitem_args.push(rvalue);

            return Ok(IRPure::op_with_pos(
                IROpCode::SetItem,
                setitem_args,
                node.start_byte(),
                node.end_byte(),
            ));
        }
    }

    // Default: SetLocals expects args: [Tuple([names...]), value, explicit_unpack?]
    // If lvalue is already a Tuple, it's unpacking syntax like (x,) or (a, b)
    // Set explicit_unpack=True to enable single-element unwrapping
    let (names_tuple, explicit_unpack) = match &lvalue {
        IRPure::Tuple(_) => {
            // Already a tuple from parsing: (x,) or (a, b, ...)
            // This is explicit unpacking syntax
            (lvalue, IRPure::Bool(true))
        }
        _ => {
            // Simple identifier: x = value
            // Wrap in tuple for consistency
            (IRPure::Tuple(vec![lvalue]), IRPure::Bool(false))
        }
    };

    Ok(IRPure::op_with_pos(
        IROpCode::SetLocals,
        vec![names_tuple, rvalue, explicit_unpack],
        node.start_byte(),
        node.end_byte(),
    ))
}

// ============================================================================
// Collections
// ============================================================================

fn transform_list_literal(node: Node, source: &str) -> TransformResult {
    // Extract items from collection_items node
    let mut items = Vec::new();
    for child in named_children(&node) {
        if child.kind() == "collection_items" {
            for item_child in named_children(&child) {
                items.push(transform(item_child, source)?);
            }
        }
    }

    Ok(IRPure::op_with_pos(
        IROpCode::ListLiteral,
        items,
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_tuple_literal(node: Node, source: &str) -> TransformResult {
    // Extract items from collection_items node
    let mut items = Vec::new();
    for child in named_children(&node) {
        if child.kind() == "collection_items" {
            for item_child in named_children(&child) {
                items.push(transform(item_child, source)?);
            }
        }
    }

    // Create Op node like list_literal does, so it gets compiled properly
    Ok(IRPure::op_with_pos(
        IROpCode::TupleLiteral,
        items,
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_dict_literal(node: Node, source: &str) -> TransformResult {
    // Extract pairs from dict_items node
    let mut pair_tuples = Vec::new();
    for child in named_children(&node) {
        if child.kind() == "dict_items" {
            for item_child in named_children(&child) {
                if item_child.kind() == "dict_pair" {
                    let pair_children = named_children(&item_child);
                    if pair_children.len() >= 2 {
                        let key = transform(pair_children[0], source)?;
                        let value = transform(pair_children[1], source)?;
                        pair_tuples.push(IRPure::Tuple(vec![key, value]));
                    }
                } else if item_child.kind() == "dict_kwarg" {
                    let kwarg_children = named_children(&item_child);
                    if kwarg_children.len() >= 2 {
                        let key_name = node_text(&kwarg_children[0], source);
                        let value = transform(kwarg_children[1], source)?;
                        pair_tuples.push(IRPure::Tuple(vec![
                            IRPure::String(key_name.to_string()),
                            value,
                        ]));
                    }
                }
            }
        }
    }

    // Use DICT_LITERAL opcode to ensure proper evaluation
    Ok(IRPure::op_with_pos(
        IROpCode::DictLiteral,
        pair_tuples,
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_set_literal(node: Node, source: &str) -> TransformResult {
    // Extract items from collection_items node
    let mut items = Vec::new();
    for child in named_children(&node) {
        if child.kind() == "collection_items" {
            for item_child in named_children(&child) {
                items.push(transform(item_child, source)?);
            }
        }
    }

    Ok(IRPure::op_with_pos(
        IROpCode::SetLiteral,
        items,
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_call(node: Node, source: &str) -> TransformResult {
    let mut func: Option<IRPure> = None;
    let mut func_name: Option<String> = None;
    let mut args = Vec::new();
    let mut kwargs = HashMap::new();

    for child in named_children(&node) {
        if child.kind() == "identifier" {
            let name = node_text(&child, source);
            func_name = Some(name.to_string());
            func = Some(IRPure::Ref(name.to_string()));
        } else if child.kind() == "arguments" {
            let (parsed_args, parsed_kwargs) = parse_arguments(&child, source)?;
            args = parsed_args;
            kwargs = parsed_kwargs;
        }
    }

    let func_node = func.ok_or_else(|| "Call without function".to_string())?;

    // Special handling for pragma() calls
    if let Some(name) = &func_name {
        if name == "pragma" {
            // Transform pragma("directive", value, ...) into Op(Pragma, args)
            return Ok(IRPure::Op {
                opcode: IROpCode::Pragma,
                args,
                kwargs,
                tail: false,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            });
        }
    }

    Ok(IRPure::Call {
        func: Box::new(func_node),
        args,
        kwargs,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    })
}

fn transform_pragma_stmt(node: Node, source: &str) -> TransformResult {
    // pragma_stmt node has children that are the arguments to pragma()
    // Transform them into an Op(Pragma, args)
    let mut args = Vec::new();

    for child in named_children(&node) {
        args.push(transform(child, source)?);
    }

    Ok(IRPure::Op {
        opcode: IROpCode::Pragma,
        args,
        kwargs: HashMap::new(),
        tail: false,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    })
}

fn transform_struct_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let mut name: Option<String> = None;
    let mut implements: Vec<String> = Vec::new();
    let mut base: Option<String> = None;
    let mut fields = Vec::new();
    let mut methods = Vec::new();

    for child in &children {
        match child.kind() {
            "identifier" => {
                // First identifier is the struct name
                if name.is_none() {
                    name = Some(node_text(child, source).to_string());
                }
            }
            "struct_implements" => {
                for grandchild in named_children(child) {
                    if grandchild.kind() == "identifier" {
                        implements.push(node_text(&grandchild, source).to_string());
                    }
                }
            }
            "struct_extends" => {
                for grandchild in named_children(child) {
                    if grandchild.kind() == "identifier" {
                        base = Some(node_text(&grandchild, source).to_string());
                        break;
                    }
                }
            }
            "struct_fields" => {
                let mut seen_default = false;
                for field_child in named_children(child) {
                    if field_child.kind() == "struct_field" {
                        let mut field_name: Option<String> = None;
                        let mut default: Option<IRPure> = None;
                        for grandchild in named_children(&field_child) {
                            if grandchild.kind() == "identifier" && field_name.is_none() {
                                field_name = Some(node_text(&grandchild, source).to_string());
                            } else if field_name.is_some() {
                                default = Some(transform(grandchild, source)?);
                            }
                        }
                        let fname = field_name.unwrap_or_default();
                        if default.is_some() {
                            seen_default = true;
                        } else if seen_default {
                            return Err(format!(
                                "non-default field '{}' follows default field",
                                fname
                            ));
                        }
                        let default_val = default.unwrap_or(IRPure::None);
                        fields.push(IRPure::Tuple(vec![IRPure::String(fname), default_val]));
                    }
                }
            }
            "struct_method" => {
                let mut method_name: Option<String> = None;
                let mut params = Vec::new();
                let mut body: Option<IRPure> = None;

                for method_child in named_children(child) {
                    match method_child.kind() {
                        "identifier" if method_name.is_none() => {
                            method_name = Some(node_text(&method_child, source).to_string());
                        }
                        "lambda_params" => {
                            params = parse_lambda_params(&method_child, source)?;
                        }
                        "block" => {
                            body = Some(transform(method_child, source)?);
                        }
                        _ => {}
                    }
                }

                if let (Some(mname), Some(mbody)) = (method_name, body) {
                    // Method as (name, OpLambda(params, body))
                    let lambda_ir = IRPure::op_with_pos(
                        IROpCode::OpLambda,
                        vec![IRPure::Tuple(params), mbody],
                        child.start_byte(),
                        child.end_byte(),
                    );
                    methods.push(IRPure::Tuple(vec![IRPure::String(mname), lambda_ir]));
                }
            }
            _ => {}
        }
    }

    let name_str = name.ok_or("struct_stmt: missing name")?;

    // Build args: (name, fields, [implements], [base], [methods])
    let mut args = vec![IRPure::String(name_str), IRPure::Tuple(fields)];

    // Add implements list if present
    if !implements.is_empty() {
        args.push(IRPure::List(
            implements.into_iter().map(IRPure::String).collect(),
        ));
    } else if base.is_some() || !methods.is_empty() {
        // Placeholder if base or methods follow
        args.push(IRPure::List(Vec::new()));
    }

    // Add extends base if present
    if let Some(base_name) = base {
        args.push(IRPure::String(base_name));
    } else if !methods.is_empty() {
        // Placeholder if methods follow
        args.push(IRPure::None);
    }

    // Add methods if present
    if !methods.is_empty() {
        args.push(IRPure::List(methods));
    }

    Ok(IRPure::op_with_pos(
        IROpCode::OpStruct,
        args,
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_trait_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let mut name: Option<String> = None;
    let mut extends: Vec<String> = Vec::new();
    let mut fields: Vec<IRPure> = Vec::new();
    let mut methods: Vec<IRPure> = Vec::new();

    for child in &children {
        match child.kind() {
            "identifier" => {
                if name.is_none() {
                    name = Some(node_text(child, source).to_string());
                }
            }
            "trait_extends" => {
                for grandchild in named_children(child) {
                    if grandchild.kind() == "identifier" {
                        extends.push(node_text(&grandchild, source).to_string());
                    }
                }
            }
            "struct_fields" => {
                let mut seen_default = false;
                for field_child in named_children(child) {
                    if field_child.kind() == "struct_field" {
                        let mut field_name: Option<String> = None;
                        let mut default: Option<IRPure> = None;
                        for grandchild in named_children(&field_child) {
                            if grandchild.kind() == "identifier" {
                                field_name = Some(node_text(&grandchild, source).to_string());
                            } else {
                                default = Some(transform(grandchild, source)?);
                            }
                        }
                        let has_default = default.is_some();
                        if has_default {
                            seen_default = true;
                        } else if seen_default {
                            return Err("trait: non-default field after default field".into());
                        }
                        fields.push(IRPure::Tuple(vec![
                            IRPure::String(field_name.unwrap_or_default()),
                            default.unwrap_or(IRPure::None),
                        ]));
                    }
                }
            }
            "struct_method" => {
                let mut method_name: Option<String> = None;
                let mut params = Vec::new();
                let mut body: Option<IRPure> = None;
                for grandchild in named_children(child) {
                    match grandchild.kind() {
                        "identifier" => {
                            method_name = Some(node_text(&grandchild, source).to_string());
                        }
                        "lambda_params" => {
                            params = parse_lambda_params(&grandchild, source)?;
                        }
                        "block" => {
                            body = Some(transform(grandchild, source)?);
                        }
                        _ => {}
                    }
                }
                let lambda = IRPure::op(
                    IROpCode::OpLambda,
                    vec![IRPure::List(params), body.unwrap_or(IRPure::None)],
                );
                methods.push(IRPure::Tuple(vec![
                    IRPure::String(method_name.unwrap_or_default()),
                    lambda,
                ]));
            }
            _ => {}
        }
    }

    let name_str = name.ok_or("trait_stmt: missing name")?;

    let mut args = vec![
        IRPure::String(name_str),
        IRPure::List(extends.into_iter().map(IRPure::String).collect()),
        IRPure::Tuple(fields),
    ];

    if !methods.is_empty() {
        args.push(IRPure::List(methods));
    }

    Ok(IRPure::op_with_pos(
        IROpCode::TraitDef,
        args,
        node.start_byte(),
        node.end_byte(),
    ))
}

/// Parse lambda_params node into a Vec of IRPure param tuples.
fn parse_lambda_params(node: &Node, source: &str) -> Result<Vec<IRPure>, String> {
    let mut params = Vec::new();
    for param_child in named_children(node) {
        if param_child.kind() == "lambda_param" {
            let param_children = named_children(&param_child);
            let mut name: Option<String> = None;
            let mut default: Option<IRPure> = None;

            for grandchild in &param_children {
                if grandchild.kind() == "identifier" {
                    name = Some(node_text(grandchild, source).to_string());
                } else {
                    default = Some(transform(*grandchild, source)?);
                }
            }

            let name_str = name.unwrap_or_default();
            let default_val = default.unwrap_or(IRPure::None);
            params.push(IRPure::Tuple(vec![IRPure::String(name_str), default_val]));
        } else if param_child.kind() == "variadic_param" {
            for grandchild in named_children(&param_child) {
                if grandchild.kind() == "identifier" {
                    let name_str = node_text(&grandchild, source).to_string();
                    params.push(IRPure::Tuple(vec![
                        IRPure::String("*".to_string()),
                        IRPure::String(name_str),
                    ]));
                }
            }
        }
    }
    Ok(params)
}

fn transform_lambda_expr(node: Node, source: &str) -> TransformResult {
    let mut params = Vec::new();
    let mut body: Option<IRPure> = None;

    for child in named_children(&node) {
        if child.kind() == "lambda_params" {
            params = parse_lambda_params(&child, source)?;
        } else if child.kind() == "block" {
            body = Some(transform(child, source)?);
        }
    }

    let body_node = body.ok_or_else(|| "Lambda without body".to_string())?;

    Ok(IRPure::op_with_pos(
        IROpCode::OpLambda,
        vec![IRPure::Tuple(params), body_node],
        node.start_byte(),
        node.end_byte(),
    ))
}

// ============================================================================
// Source file
// ============================================================================

fn transform_source_file(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();

    // Return as Program (top-level statement sequence)
    Ok(IRPure::Program(transformed?))
}

// ============================================================================
// Helpers
// ============================================================================

fn transform_children(node: Node, source: &str) -> TransformResult {
    // named_children already filters out comments
    let children = named_children(&node);

    if children.is_empty() {
        Ok(IRPure::None)
    } else if children.len() == 1 {
        transform(children[0], source)
    } else {
        let transformed: Result<Vec<_>, _> =
            children.iter().map(|c| transform(*c, source)).collect();
        Ok(IRPure::Tuple(transformed?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_transform(source: &str) -> TransformResult {
        use tree_sitter::Parser;

        let language = crate::get_tree_sitter_language();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();

        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let children = named_children(&root);
        transform(children[0], source)
    }

    #[test]
    fn test_transform_number() {
        let result = parse_and_transform("42").unwrap();
        assert_eq!(result, IRPure::Int(42));
    }

    #[test]
    fn test_transform_addition() {
        let result = parse_and_transform("2 + 3").unwrap();

        match result {
            IRPure::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::Add);
                assert_eq!(args.len(), 1);
                match &args[0] {
                    IRPure::Tuple(operands) => {
                        assert_eq!(operands.len(), 2);
                        assert_eq!(operands[0], IRPure::Int(2));
                        assert_eq!(operands[1], IRPure::Int(3));
                    }
                    _ => panic!("Expected tuple of operands"),
                }
            }
            _ => panic!("Expected Op node"),
        }
    }

    #[test]
    fn test_transform_string() {
        let result = parse_and_transform("\"hello\"").unwrap();
        assert_eq!(result, IRPure::String("hello".into()));
    }

    #[test]
    fn test_transform_bool() {
        let result = parse_and_transform("True").unwrap();
        assert_eq!(result, IRPure::Bool(true));
    }
}

// ============================================================================
// Broadcasting & Chained Access
// ============================================================================

fn transform_chained(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.is_empty() {
        return Ok(IRPure::None);
    }

    // Transform first child as base
    let mut result = transform(children[0], source)?;

    // Process remaining children as chained operations
    for child in &children[1..] {
        match child.kind() {
            "call_member" => {
                // call_member: direct call on previous result
                let mut args = Vec::new();
                let mut kwargs = HashMap::new();

                for arg_node in named_children(child) {
                    if arg_node.kind() == "arguments" {
                        let (parsed_args, parsed_kwargs) = parse_arguments(&arg_node, source)?;
                        args = parsed_args;
                        kwargs = parsed_kwargs;
                    }
                }

                result = IRPure::Call {
                    func: Box::new(result),
                    args,
                    kwargs,
                    start_byte: child.start_byte(),
                    end_byte: child.end_byte(),
                };
            }
            "callattr" => {
                // callattr: .method(args) - extract method name and call it
                let mut method_name = String::new();
                let mut args = Vec::new();
                let mut kwargs = HashMap::new();

                for callattr_child in named_children(child) {
                    match callattr_child.kind() {
                        "identifier" => {
                            method_name = node_text(&callattr_child, source).to_string();
                        }
                        "arguments" => {
                            let (parsed_args, parsed_kwargs) =
                                parse_arguments(&callattr_child, source)?;
                            args = parsed_args;
                            kwargs = parsed_kwargs;
                        }
                        _ => {}
                    }
                }

                // Create GetAttr to get the method
                let method = IRPure::op(
                    IROpCode::GetAttr,
                    vec![result, IRPure::Identifier(method_name)],
                );

                // Create Call with method as function
                result = IRPure::Call {
                    func: Box::new(method),
                    args,
                    kwargs,
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
                    result = IRPure::op(
                        IROpCode::GetAttr,
                        vec![result, IRPure::Identifier(attr_name)],
                    );
                }
            }
            "index" => {
                // GetItem: extract index and create GetItem op
                let index_children = named_children(child);
                if !index_children.is_empty() {
                    let index = transform(index_children[0], source)?;
                    result = IRPure::op(IROpCode::GetItem, vec![result, index]);
                }
            }
            "fullslice" => {
                // fullslice: .[slice_range] -> GetItem with slice
                let fullslice_children = named_children(child);
                if !fullslice_children.is_empty() {
                    let slice = transform(fullslice_children[0], source)?;
                    result = IRPure::op(IROpCode::GetItem, vec![result, slice]);
                }
            }
            _ => {
                // Unknown chained operation, skip
            }
        }
    }

    Ok(result)
}

fn apply_broadcast(target: IRPure, node: Node, source: &str) -> TransformResult {
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
            Ok(IRPure::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(IRPure::String(operator.to_string())),
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
            Ok(IRPure::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(IRPure::String(operator.to_string())),
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
                    Ok(IRPure::Broadcast {
                        target: Some(Box::new(target)),
                        operator: Box::new(IRPure::String(operator.to_string())),
                        operand: Some(Box::new(operand)),
                        broadcast_type: BroadcastType::If,
                    })
                }
                "broadcast_unary" => {
                    // Unary condition
                    let operator = node_text(&condition_node, source);

                    // Filter broadcast: target.[if op]
                    Ok(IRPure::Broadcast {
                        target: Some(Box::new(target)),
                        operator: Box::new(IRPure::String(operator.to_string())),
                        operand: None,
                        broadcast_type: BroadcastType::If,
                    })
                }
                _ => {
                    // Expression (lambda or function)
                    let expr = transform(condition_node, source)?;

                    // Filter broadcast with lambda: target.[if lambda]
                    Ok(IRPure::Broadcast {
                        target: Some(Box::new(target)),
                        operator: Box::new(expr),
                        operand: None,
                        broadcast_type: BroadcastType::If,
                    })
                }
            }
        }
        "broadcast_nd_map" => {
            // Function map: .[@> func]
            // Registry expects ND_MAP with (data_or_func, func_node)
            // In broadcast form: (func, None) for lift-like form
            let bc_children = named_children(&final_node);
            if bc_children.is_empty() {
                return Err("broadcast_nd_map: no function".into());
            }

            let func_node = bc_children[0];
            let func = transform(func_node, source)?;

            // Use IRPure::Broadcast structure with NDMap type
            Ok(IRPure::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(IRPure::op(IROpCode::NdMap, vec![func, IRPure::None])),
                operand: None,
                broadcast_type: BroadcastType::NDMap,
            })
        }
        "broadcast_nd_recursion" => {
            // ND recursion: .[@@lambda]
            // Registry expects ND_RECURSION with (data_or_seed, lambda_node)
            // In broadcast form: (lambda, None) for declaration-like form
            let bc_children = named_children(&final_node);
            if bc_children.is_empty() {
                return Err("broadcast_nd_recursion: no lambda".into());
            }

            let lambda_node = bc_children[0];
            let lambda = transform(lambda_node, source)?;

            // Use IRPure::Broadcast structure with NDRecursion type
            Ok(IRPure::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(IRPure::op(
                    IROpCode::NdRecursion,
                    vec![lambda, IRPure::None],
                )),
                operand: None,
                broadcast_type: BroadcastType::NDRecursion,
            })
        }
        "broadcast" => {
            // Nested broadcast: target.[.[op]]
            // Desugar to lambda: target.[(x) => { x.[op] }]
            let param = "__bcast_x__";
            let inner = apply_broadcast(IRPure::Ref(param.to_string()), final_node, source)?;
            let lambda = IRPure::op(
                IROpCode::OpLambda,
                vec![
                    IRPure::Tuple(vec![IRPure::Tuple(vec![
                        IRPure::String(param.to_string()),
                        IRPure::None,
                    ])]),
                    inner,
                ],
            );
            Ok(IRPure::Broadcast {
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
            Ok(IRPure::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(expr),
                operand: None,
                broadcast_type: BroadcastType::Lambda,
            })
        }
    }
}

// ============================================================================
// Pattern Matching
// ============================================================================

fn transform_match_expr(node: Node, source: &str) -> TransformResult {
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
    let cases_tuple = IRPure::Tuple(cases);
    Ok(IRPure::op_with_pos(
        IROpCode::OpMatch,
        vec![value, cases_tuple],
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_match_case(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // Structure: match_case(pattern, guard?, block)
    // guard est un field tree-sitter, pas un kind
    let mut pattern: Option<IRPure> = None;
    let mut block: Option<IRPure> = None;

    for child in children {
        match child.kind() {
            "pattern" | "pattern_literal" | "pattern_var" | "pattern_wildcard" | "pattern_or"
            | "pattern_tuple" | "pattern_struct" => {
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
    let guard_val = guard.unwrap_or(IRPure::None);
    let block_val = block.ok_or("match_case: missing block")?;

    Ok(IRPure::Tuple(vec![pattern_val, guard_val, block_val]))
}

// ============================================================================
// Patterns
// ============================================================================

fn transform_pattern(node: Node, source: &str) -> TransformResult {
    // Pattern est un wrapper, dispatcher vers le type réel
    let children = named_children(&node);
    if children.is_empty() {
        return Err("pattern: no children".into());
    }

    transform(children[0], source)
}

fn transform_pattern_literal(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.is_empty() {
        return Err("pattern_literal: no value".into());
    }

    let value = transform(children[0], source)?;
    Ok(IRPure::PatternLiteral(Box::new(value)))
}

fn transform_pattern_var(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.is_empty() {
        return Err("pattern_var: no identifier".into());
    }

    let name_node = children[0];
    if name_node.kind() != "identifier" {
        return Err(format!(
            "pattern_var: expected identifier, got {}",
            name_node.kind()
        ));
    }

    let name = node_text(&name_node, source).to_string();
    Ok(IRPure::PatternVar(name))
}

fn transform_pattern_wildcard(_node: Node, _source: &str) -> TransformResult {
    Ok(IRPure::PatternWildcard)
}

fn transform_pattern_star(node: Node, source: &str) -> TransformResult {
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
        )
        .into());
    }

    let name = node_text(&identifier_node, source).to_string();
    // Retourner tuple ('*', name)
    Ok(IRPure::Tuple(vec![
        IRPure::String("*".to_string()),
        IRPure::String(name),
    ]))
}

fn transform_variadic_param(node: Node, source: &str) -> TransformResult {
    // Variadic parameter in unpacking: *rest
    // Expected format: ("*", "varname")
    let children = named_children(&node);
    for child in children {
        if child.kind() == "identifier" {
            let name = node_text(&child, source).to_string();
            return Ok(IRPure::Tuple(vec![
                IRPure::String("*".to_string()),
                IRPure::String(name),
            ]));
        }
    }
    Err("variadic_param: no identifier found".into())
}

fn transform_pattern_or(node: Node, source: &str) -> TransformResult {
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

    Ok(IRPure::PatternOr(patterns))
}

fn transform_pattern_tuple(node: Node, source: &str) -> TransformResult {
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

    Ok(IRPure::PatternTuple(patterns))
}

fn transform_pattern_struct(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let mut struct_name: Option<String> = None;
    let mut fields = Vec::new();

    for child in &children {
        if child.kind() == "identifier" {
            if struct_name.is_none() {
                struct_name = Some(node_text(child, source).to_string());
            } else {
                fields.push(node_text(child, source).to_string());
            }
        }
    }

    let name = struct_name.ok_or("pattern_struct: missing struct name")?;
    Ok(IRPure::PatternStruct { name, fields })
}

// ============================================================================
// Access & Indexing
// ============================================================================

fn transform_index(node: Node, source: &str) -> TransformResult {
    // index: [expression] or [slice_range]
    // Returns the index expression (or slice) for GetItem operation
    let children = named_children(&node);

    if children.is_empty() {
        return Err("index: no children".into());
    }

    // Check if this is a slice_range
    if children[0].kind() == "slice_range" {
        return transform_slice_range(children[0], source);
    }

    // Regular index expression
    transform(children[0], source)
}

fn transform_slice_range(node: Node, source: &str) -> TransformResult {
    // slice_range: start:stop:step (all parts optional)
    // Returns IRPure::Slice variant

    let node_source = node_text(&node, source);
    let mut start = IRPure::None;
    let mut stop = IRPure::None;
    let mut step = IRPure::None;

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
            if let IRPure::Op {
                opcode: IROpCode::Neg,
                args,
                ..
            } = &expr
            {
                if args.len() == 1 {
                    match &args[0] {
                        IRPure::Int(n) => expr = IRPure::Int(-n),
                        IRPure::Float(f) => expr = IRPure::Float(-f),
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
    Ok(IRPure::Slice {
        start: Box::new(start),
        stop: Box::new(stop),
        step: Box::new(step),
    })
}

fn transform_fullslice(node: Node, source: &str) -> TransformResult {
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

fn transform_broadcast(node: Node, source: &str) -> TransformResult {
    // broadcast: .[broadcast_op]
    // Returns IRPure::Broadcast variant

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
        "broadcast_nd_map" => {
            let children = named_children(&broadcast_op_node);
            if children.is_empty() {
                return Err("broadcast_nd_map: no expression".into());
            }
            let expr = transform(children[0], source)?;
            Ok(IRPure::Broadcast {
                target: None,
                operator: Box::new(expr),
                operand: None,
                broadcast_type: BroadcastType::NDMap,
            })
        }
        "broadcast_nd_recursion" => {
            let children = named_children(&broadcast_op_node);
            if children.is_empty() {
                return Err("broadcast_nd_recursion: no expression".into());
            }
            let expr = transform(children[0], source)?;
            Ok(IRPure::Broadcast {
                target: None,
                operator: Box::new(expr),
                operand: None,
                broadcast_type: BroadcastType::NDRecursion,
            })
        }
        _ => {
            // Default: lambda or expression
            let expr = transform(broadcast_op_node, source)?;
            Ok(IRPure::Broadcast {
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
    Ok(IRPure::Broadcast {
        target: None,
        operator: Box::new(IRPure::String(operator_str)),
        operand: Some(Box::new(operand)),
        broadcast_type: BroadcastType::Binary,
    })
}

fn transform_broadcast_unary(node: Node, source: &str) -> TransformResult {
    // broadcast_unary: unary_operator
    let operator_str = node_text(&node, source).to_string();

    use crate::ir::BroadcastType;
    Ok(IRPure::Broadcast {
        target: None,
        operator: Box::new(IRPure::String(operator_str)),
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
                    clauses.push(IRPure::Tuple(vec![condition, body]));
                }
            }
            "else_broadcast_clause" => {
                let clause_children = named_children(&child);
                if !clause_children.is_empty() {
                    let body = transform(clause_children[0], source)?;
                    clauses.push(IRPure::Tuple(vec![IRPure::Bool(true), body]));
                }
            }
            _ => {}
        }
    }

    use crate::ir::BroadcastType;
    Ok(IRPure::Broadcast {
        target: None,
        operator: Box::new(IRPure::List(clauses)),
        operand: None,
        broadcast_type: BroadcastType::If,
    })
}
