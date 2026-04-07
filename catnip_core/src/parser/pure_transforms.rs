// FILE: catnip_core/src/parser/pure_transforms.rs
//! Pure Rust transformers - Tree-sitter → IR (no Python dependencies)
//!
//! Port of the existing transformers for the standalone pipeline.

use super::utils::{named_children, node_text, unescape_bytes, unescape_string};
use crate::ir::{BroadcastType, IR, IROpCode};
use indexmap::IndexMap;
use tree_sitter::Node;

/// Result type for transform operations
pub type TransformResult = Result<IR, String>;

/// Parse arguments from an `arguments` tree-sitter node into (args, kwargs).
///
/// Handles all grammar node types: args, kwargs, args_kwargs, kwarg, keyword_argument.
fn parse_arguments(arguments_node: &Node, source: &str) -> Result<(Vec<IR>, IndexMap<String, IR>), String> {
    let mut args = Vec::new();
    let mut kwargs = IndexMap::new();

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
        "number" | "integer" => transform_number(node, source),
        "string" => transform_string(node, source),
        "bstring" => transform_bstring(node, source),
        "fstring" => transform_fstring(node, source),
        "true" | "false" => transform_bool(node, source),
        "none" => Ok(IR::None),
        "nd_empty_topos" => transform_nd_empty_topos(node, source),

        // Operators MVP
        "additive" => transform_additive(node, source),
        "multiplicative" => transform_multiplicative(node, source),
        "exponent" => transform_exponent(node, source),
        "unary" => transform_unary(node, source),
        "comparison" => transform_comparison(node, source),
        "bool_and" => transform_bool_and(node, source),
        "bool_or" => transform_bool_or(node, source),
        "null_coalesce" => transform_null_coalesce(node, source),
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

        // Error handling
        "try_stmt" => transform_try_stmt(node, source),
        "raise_stmt" => transform_raise_stmt(node, source),

        // Context managers
        "with_stmt" => transform_with_stmt(node, source),

        // Pattern matching
        "pattern" => transform_pattern(node, source),
        "pattern_literal" => transform_pattern_literal(node, source),
        "pattern_var" => transform_pattern_var(node, source),
        "pattern_wildcard" => transform_pattern_wildcard(node, source),
        "pattern_or" => transform_pattern_or(node, source),
        "pattern_tuple" => transform_pattern_tuple(node, source),
        "pattern_star" => transform_pattern_star(node, source),
        "pattern_struct" => transform_pattern_struct(node, source),
        "pattern_enum" => transform_pattern_enum(node, source),

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
        "bracket_list" => transform_collection_ir(node, source, IROpCode::ListLiteral, "__catnip_spread_list"),
        "bracket_dict" => transform_bracket_dict(node, source),
        "list_literal" => transform_collection_ir(node, source, IROpCode::ListLiteral, "__catnip_spread_list"),
        "tuple_literal" => transform_collection_ir(node, source, IROpCode::TupleLiteral, "__catnip_spread_tuple"),
        "dict_literal" => transform_dict_literal(node, source),
        "set_literal" => transform_collection_ir(node, source, IROpCode::SetLiteral, "__catnip_spread_set"),

        // Function call & Lambda
        "call" => transform_call(node, source),
        "lambda_expr" => transform_lambda_expr(node, source),

        // Pragma directive
        "pragma_stmt" => transform_pragma_stmt(node, source),
        "pragma_qualified" => transform_pragma_qualified(node, source),

        // Struct
        "struct_stmt" => transform_struct_stmt(node, source),

        // Trait
        "trait_stmt" => transform_trait_stmt(node, source),

        // Enum
        "enum_stmt" => transform_enum_stmt(node, source),

        // Block
        "block" => transform_block(node, source),

        // Access & Broadcasting
        "index" => transform_index(node, source),
        "slice_range" => transform_slice_range(node, source),
        "fullslice" => transform_fullslice(node, source),
        "broadcast" => transform_broadcast(node, source),

        // Chained operations (call_member, getattr, index, broadcast)
        "chained" => transform_chained(node, source),

        // Statement (detect import statement pattern)
        "statement" => transform_statement(node, source),

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

    // Imaginary literal: suffix j/J
    if text.ends_with('j') || text.ends_with('J') {
        let digits = &text[..text.len() - 1];
        return Ok(IR::Imaginary(digits.to_string()));
    }

    // Decimal literal: suffix d/D
    if text.ends_with('d') || text.ends_with('D') {
        let digits = &text[..text.len() - 1];
        return Ok(IR::Decimal(digits.to_string()));
    }

    // Try int with base prefixes (0x, 0b, 0o)
    if text.len() > 2 {
        let (prefix, digits) = text.split_at(2);
        let radix = match prefix {
            "0x" | "0X" => Some(16),
            "0b" | "0B" => Some(2),
            "0o" | "0O" => Some(8),
            _ => None,
        };
        if let Some(r) = radix {
            return i64::from_str_radix(digits, r)
                .map(IR::Int)
                .map_err(|e| format!("Invalid base-{} number: {} ({})", r, text, e));
        }
    }

    // Try int (decimal)
    if let Ok(val) = text.parse::<i64>() {
        return Ok(IR::Int(val));
    }

    // Try float
    if let Ok(val) = text.parse::<f64>() {
        return Ok(IR::Float(val));
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

    Ok(IR::String(unescaped))
}

fn transform_bstring(node: Node, source: &str) -> TransformResult {
    let text = node_text(&node, source);

    // Detect prefix: b""" / b''' (offset 4) vs b" / b' (offset 2)
    let (start_offset, end_offset) = if text.starts_with("b\"\"\"") || text.starts_with("b'''") {
        if text.len() < 8 {
            return Err("Triple-quoted byte string literal too short".into());
        }
        (4, 3)
    } else {
        if text.len() < 4 {
            return Err("Byte string literal too short".into());
        }
        (2, 1)
    };

    let content = &text[start_offset..text.len() - end_offset];
    let bytes = unescape_bytes(content);

    Ok(IR::Bytes(bytes))
}

fn transform_fstring(node: Node, source: &str) -> TransformResult {
    let mut parts = Vec::new();
    let children = named_children(&node);

    // Collect all parts (literal text and interpolations)
    for child in &children {
        match child.kind() {
            // Literal text nodes → IR::String
            "fstring_text_double" | "fstring_text_single" | "fstring_text_long_double" | "fstring_text_long_single" => {
                let text = node_text(child, source);
                let unescaped = unescape_string(text);
                parts.push(IR::String(unescaped));
            }
            // Interpolation → IR::Tuple([expr, Int(conv), spec])
            // conv: 0=none, 1=str(!s), 2=repr(!r), 3=ascii(!a)
            // spec: String(spec) or None
            "interpolation" => {
                let interp_children = named_children(child);
                let mut expr_node = None;
                let mut format_spec = None;
                let mut conversion: i64 = 0;
                let mut is_debug = false;

                for ic in &interp_children {
                    match ic.kind() {
                        "fstring_debug" => is_debug = true,
                        "fstring_conversion" => {
                            let conv_text = node_text(ic, source);
                            conversion = match conv_text {
                                "s" => 1,
                                "r" => 2,
                                "a" => 3,
                                _ => 0,
                            };
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

                    // Debug: prepend "expr_source=" as text part
                    if is_debug {
                        let expr_text = &source[child.start_byte() + 1..expr_n.end_byte()];
                        parts.push(IR::String(format!("{}=", expr_text)));
                    }

                    let spec = match format_spec {
                        Some(s) => IR::String(s),
                        None => IR::None,
                    };

                    parts.push(IR::Tuple(vec![expr, IR::Int(conversion), spec]));
                }
            }
            "escape_sequence" => {
                let text = node_text(child, source);
                let unescaped = unescape_string(text);
                parts.push(IR::String(unescaped));
            }
            _ => {}
        }
    }

    // Check for trailing text after last child (before closing delimiter)
    if let Some(last_child) = children.last() {
        let last_child_end = last_child.end_byte();
        let node_end = node.end_byte();

        if last_child_end < node_end {
            let trailing = &source[last_child_end..node_end];
            let trimmed = trailing
                .trim_end_matches("\"\"\"")
                .trim_end_matches("'''")
                .trim_end_matches('"')
                .trim_end_matches('\'');

            if !trimmed.is_empty() {
                let unescaped = unescape_string(trimmed);
                parts.push(IR::String(unescaped));
            }
        }
    }

    // 0 parts → empty string
    if parts.is_empty() {
        return Ok(IR::String(String::new()));
    }

    // 1 text-only part → unwrap directly
    if parts.len() == 1 {
        if let IR::String(_) = &parts[0] {
            return Ok(parts.into_iter().next().unwrap());
        }
    }

    Ok(IR::op(IROpCode::Fstring, parts))
}

fn transform_bool(node: Node, source: &str) -> TransformResult {
    let text = node_text(&node, source);
    match text {
        "True" => Ok(IR::Bool(true)),
        "False" => Ok(IR::Bool(false)),
        _ => Err(format!("Invalid bool literal: {}", text)),
    }
}

fn transform_nd_empty_topos(_node: Node, _source: &str) -> TransformResult {
    // ~[] - empty non-deterministic topos
    Ok(IR::op(IROpCode::NdEmptyTopos, vec![]))
}

fn transform_nd_recursion(node: Node, source: &str) -> TransformResult {
    // ND-recursion: ~~(seed, lambda) or ~~ lambda
    // Registry expects: (data_or_seed, lambda_node)
    // Forms:
    // - ~~(seed, lambda): Combinator form - (seed, lambda)
    // - ~~ lambda: Declaration form - (lambda, None)
    let children = named_children(&node);

    for child in children {
        let kind = child.kind();
        if kind == "arguments" {
            // ~~(seed, lambda) or ~~(lambda)
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
            // Single arg in parens: declaration form ~~(lambda) → (lambda, None)
            if args.len() == 1 {
                args.push(IR::None);
            }
            return Ok(IR::op(IROpCode::NdRecursion, args));
        } else if kind == "lambda_expr" {
            // Declaration form: ~~ lambda → (lambda, None)
            let lambda = transform(child, source)?;
            return Ok(IR::op(IROpCode::NdRecursion, vec![lambda, IR::None]));
        }
    }

    Err("nd_recursion: no arguments or lambda found".into())
}

fn transform_nd_map(node: Node, source: &str) -> TransformResult {
    // ND-map: ~>(data, f) or ~> f
    // Registry expects: (data_or_func, func_node)
    // Forms:
    // - ~>(data, f): Applicative form - (data, f)
    // - ~> f: Lift form - (f, None)
    let children = named_children(&node);

    for child in children {
        let kind = child.kind();
        if kind == "arguments" {
            // ~>(data, f) or ~>(f)
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
            // Single arg in parens: lift form ~>(f) → (f, None)
            if args.len() == 1 {
                args.push(IR::None);
            }
            return Ok(IR::op(IROpCode::NdMap, args));
        } else if kind == "lambda_expr" || kind.starts_with("_") || kind == "identifier" {
            // Lift form: ~> f → (f, None)
            let func = transform(child, source)?;
            return Ok(IR::op(IROpCode::NdMap, vec![func, IR::None]));
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

fn transform_comparison(node: Node, source: &str) -> TransformResult {
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
fn transform_bool_and(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::And)
}

fn transform_bool_or(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::Or)
}

fn transform_null_coalesce(node: Node, source: &str) -> TransformResult {
    binary_logical(node, source, IROpCode::NullCoalesce)
}

fn transform_bool_not(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if !children.is_empty() {
        let operand = transform(children[children.len() - 1], source)?;
        Ok(IR::op(IROpCode::Not, vec![operand]))
    } else {
        Ok(IR::None)
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
        Ok(IR::op(IROpCode::BNot, vec![operand]))
    } else {
        Ok(IR::None)
    }
}

fn transform_shift(node: Node, source: &str) -> TransformResult {
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

fn transform_unary(node: Node, source: &str) -> TransformResult {
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

// ============================================================================
// Control Flow
// ============================================================================

fn transform_if_expr(node: Node, source: &str) -> TransformResult {
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

fn transform_elif_clause(node: Node, source: &str) -> TransformResult {
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

fn transform_else_clause(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    // else_clause has one child: the block
    if !children.is_empty() {
        transform(children[0], source)
    } else {
        Ok(IR::None)
    }
}

fn transform_while_stmt(node: Node, source: &str) -> TransformResult {
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

fn transform_for_stmt(node: Node, source: &str) -> TransformResult {
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

fn transform_return_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);

    let value = if !children.is_empty() {
        transform(children[0], source)?
    } else {
        IR::None
    };

    Ok(IR::op(IROpCode::OpReturn, vec![value]))
}

fn transform_break_stmt(_node: Node, _source: &str) -> TransformResult {
    Ok(IR::op(IROpCode::OpBreak, vec![]))
}

fn transform_continue_stmt(_node: Node, _source: &str) -> TransformResult {
    Ok(IR::op(IROpCode::OpContinue, vec![]))
}

// -- Error handling ----------------------------------------------------------

fn transform_raise_stmt(node: Node, source: &str) -> TransformResult {
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

fn transform_try_stmt(node: Node, source: &str) -> TransformResult {
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

// ============================================================================
// Context Managers (with)
// ============================================================================

/// Desugar `with a=e1, b=e2 { body }` into nested try/except/finally IR.
///
/// Each binding `name = expr` produces (N = globally unique counter):
///   __with_cm_N = expr
///   name = __with_cm_N.__enter__()
///   __with_exc_N = False
///   try {
///     try { <inner> }
///     except { __with_e_N => {
///       __with_exc_N = True
///       __with_ei_N = ExcInfo()       // (type_class, instance, None)
///       if not __with_cm_N.__exit__(__with_ei_N[0], __with_ei_N[1], None) { raise }
///     }}
///   } finally {
///     if not __with_exc_N { __with_cm_N.__exit__(None, None, None) }
///   }
///
/// Multiple bindings nest right-to-left so cleanup runs in reverse order.
fn transform_with_stmt(node: Node, source: &str) -> TransformResult {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static WITH_COUNTER: AtomicUsize = AtomicUsize::new(0);

    let mut bindings: Vec<(String, IR)> = Vec::new();
    let mut body_ir: Option<IR> = None;

    let children = named_children(&node);
    for child in &children {
        match child.kind() {
            "with_binding" => {
                let name_node = child.child_by_field_name("name").ok_or("with_binding: missing name")?;
                let value_node = child
                    .child_by_field_name("value")
                    .ok_or("with_binding: missing value")?;
                let name = node_text(&name_node, source).to_string();
                let value = transform(value_node, source)?;
                bindings.push((name, value));
            }
            "block" => {
                body_ir = Some(transform(*child, source)?);
            }
            _ => {}
        }
    }

    if bindings.is_empty() {
        return Err("with_stmt: requires at least one binding".into());
    }
    let body = body_ir.ok_or("with_stmt: missing body block")?;
    let (sb, eb) = (node.start_byte(), node.end_byte());

    // Fold right: innermost binding wraps the body, outermost is the root.
    let mut result = body;
    for (name, cm_expr) in bindings.into_iter().rev() {
        let uid = WITH_COUNTER.fetch_add(1, Ordering::Relaxed);
        result = desugar_with_single(uid, &name, cm_expr, result, sb, eb);
    }

    Ok(result)
}

/// Produce the desugared IR for a single `with` binding wrapping an inner body.
/// `uid` is a globally unique counter to avoid name collisions in nested with blocks.
fn desugar_with_single(uid: usize, name: &str, cm_expr: IR, inner_body: IR, sb: usize, eb: usize) -> IR {
    let cm_var = format!("__with_cm_{uid}");
    let exc_var = format!("__with_exc_{uid}");
    let ei_var = format!("__with_ei_{uid}");

    // Helper: make a simple ref
    let r = |n: &str| IR::Ref(n.to_string(), -1, -1);
    // Helper: set_locals(name, value) -- same format as transform_assignment
    let set = |n: &str, v: IR| {
        IR::op_with_pos(
            IROpCode::SetLocals,
            vec![IR::Tuple(vec![IR::Ref(n.to_string(), -1, -1)]), v, IR::Bool(false)],
            sb,
            eb,
        )
    };
    // Helper: method call obj.method(args...)
    let method_call = |obj: &str, method: &str, args: Vec<IR>| IR::Call {
        func: Box::new(IR::op_with_pos(
            IROpCode::GetAttr,
            vec![r(obj), IR::String(method.to_string())],
            sb,
            eb,
        )),
        args,
        kwargs: indexmap::IndexMap::new(),
        tail: false,
        start_byte: sb,
        end_byte: eb,
    };
    // Helper: index access obj[idx]
    let index = |obj: IR, idx: i64| IR::op_with_pos(IROpCode::GetItem, vec![obj, IR::Int(idx)], sb, eb);

    // __with_cm_N = expr
    let assign_cm = set(&cm_var, cm_expr);

    // name = __with_cm_N.__enter__()
    let assign_val = set(name, method_call(&cm_var, "__enter__", vec![]));

    // __with_exc_N = False
    let assign_exc = set(&exc_var, IR::Bool(false));

    // --- except handler ---
    // __with_exc_N = True
    let set_exc_true = set(&exc_var, IR::Bool(true));
    // __with_ei_N = ExcInfo()  -- pushes (exc_type_class, exc_instance, None)
    let assign_ei = set(&ei_var, IR::op_with_pos(IROpCode::ExcInfo, vec![], sb, eb));
    // __with_cm_N.__exit__(__with_ei_N[0], __with_ei_N[1], None)
    let exit_with_exc = method_call(
        &cm_var,
        "__exit__",
        vec![index(r(&ei_var), 0), index(r(&ei_var), 1), IR::None],
    );
    // not __exit__(...)
    let not_exit = IR::op_with_pos(IROpCode::Not, vec![exit_with_exc], sb, eb);
    // if not __exit__(...) { raise }
    // OpIf takes args[0] = Tuple of (condition, body) branch pairs
    let raise_block = IR::op_with_pos(
        IROpCode::OpBlock,
        vec![IR::op_with_pos(IROpCode::OpRaise, vec![], sb, eb)],
        sb,
        eb,
    );
    let conditional_raise = IR::op_with_pos(
        IROpCode::OpIf,
        vec![IR::Tuple(vec![IR::Tuple(vec![not_exit, raise_block])])],
        sb,
        eb,
    );
    // handler body: block { set_exc_true; assign_ei; conditional_raise }
    let handler_body = IR::op_with_pos(
        IROpCode::OpBlock,
        vec![set_exc_true, assign_ei, conditional_raise],
        sb,
        eb,
    );

    // except clause: Tuple([types=[], binding=None (wildcard, no binding needed), handler_body])
    // We use ExcInfo to get exception info, no need for the except binding.
    let except_clause = IR::Tuple(vec![IR::List(vec![]), IR::None, handler_body]);
    let handlers = IR::List(vec![except_clause]);

    // inner try/except (no finally)
    let inner_try = IR::op_with_pos(IROpCode::OpTry, vec![inner_body, handlers, IR::None], sb, eb);

    // --- finally block ---
    // if not __with_exc_N { __with_cm_N.__exit__(None, None, None) }
    let not_exc = IR::op_with_pos(IROpCode::Not, vec![r(&exc_var)], sb, eb);
    let exit_clean = method_call(&cm_var, "__exit__", vec![IR::None, IR::None, IR::None]);
    let exit_clean_block = IR::op_with_pos(IROpCode::OpBlock, vec![exit_clean], sb, eb);
    let finally_body = IR::op_with_pos(
        IROpCode::OpIf,
        vec![IR::Tuple(vec![IR::Tuple(vec![not_exc, exit_clean_block])])],
        sb,
        eb,
    );

    // outer try/finally (no except)
    let outer_try = IR::op_with_pos(IROpCode::OpTry, vec![inner_try, IR::List(vec![]), finally_body], sb, eb);

    // Wrap in a block (expression-valued, returns last value)
    IR::op_with_pos(
        IROpCode::OpBlock,
        vec![assign_cm, assign_val, assign_exc, outer_try],
        sb,
        eb,
    )
}

fn transform_block(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();

    Ok(IR::op_with_pos(
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
    let start_byte = node.start_byte() as isize;
    let end_byte = node.end_byte() as isize;
    Ok(IR::Ref(text.to_string(), start_byte, end_byte))
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
                result = IR::op(IROpCode::GetAttr, vec![result, IR::String(attr_name)]);
            }
        } else if child.kind() == "index" {
            let index_children = named_children(child);
            if !index_children.is_empty() {
                let index = transform(index_children[0], source)?;
                result = IR::op(IROpCode::GetItem, vec![result, index]);
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

fn transform_assignment(node: Node, source: &str) -> TransformResult {
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

// ============================================================================
// Collections
// ============================================================================

/// Shared helper for list/tuple/set literals (both bracket and function-call forms).
/// Iterates collection_items, handles spreads, emits the given opcode.
fn transform_collection_ir(node: Node, source: &str, opcode: IROpCode, spread_fn: &str) -> TransformResult {
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

fn transform_bracket_dict(node: Node, source: &str) -> TransformResult {
    // Extract colon_pair and dict_spread entries from bracket_dict_items.
    let mut pair_tuples = Vec::new();
    let mut spread_entries = Vec::new();
    let mut has_spread = false;

    for child in named_children(&node) {
        if child.kind() == "bracket_dict_items" {
            for item_child in named_children(&child) {
                let current = item_child;

                if current.kind() == "colon_pair" {
                    let pair_children = named_children(&current);
                    if pair_children.len() >= 2 {
                        let key = transform(pair_children[0], source)?;
                        let value = transform(pair_children[1], source)?;
                        pair_tuples.push(IR::Tuple(vec![key, value]));
                        spread_entries.push(IR::op_with_pos(
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
                } else if current.kind() == "dict_spread" {
                    let spread_children = named_children(&current);
                    if spread_children.is_empty() {
                        return Err("dict_spread: missing value".into());
                    }
                    let mapping = transform(spread_children[0], source)?;
                    spread_entries.push(IR::op_with_pos(
                        IROpCode::TupleLiteral,
                        vec![IR::Bool(true), mapping],
                        current.start_byte(),
                        current.end_byte(),
                    ));
                    has_spread = true;
                }
            }
        }
    }

    if has_spread {
        return Ok(IR::Call {
            func: Box::new(IR::Ref("__catnip_spread_dict".to_string(), -1, -1)),
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

    Ok(IR::op_with_pos(
        IROpCode::DictLiteral,
        pair_tuples,
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_dict_literal(node: Node, source: &str) -> TransformResult {
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

        return Ok(IR::Call {
            func: Box::new(IR::Ref("__catnip_spread_dict".to_string(), -1, -1)),
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

    // Extract pairs/spreads from dict_items node.
    // If any **spread is present, lower to __catnip_spread_dict(entries).
    let mut pair_tuples = Vec::new();
    let mut spread_entries = Vec::new();
    let mut has_spread = false;

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

                if current.kind() == "dict_pair" {
                    let pair_children = named_children(&current);
                    if pair_children.len() >= 2 {
                        let key = transform(pair_children[0], source)?;
                        let value = transform(pair_children[1], source)?;
                        pair_tuples.push(IR::Tuple(vec![key, value]));
                        spread_entries.push(IR::op_with_pos(
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
                } else if current.kind() == "dict_kwarg" {
                    let kwarg_children = named_children(&current);
                    if kwarg_children.len() >= 2 {
                        let key_name = node_text(&kwarg_children[0], source);
                        let value = transform(kwarg_children[1], source)?;
                        pair_tuples.push(IR::Tuple(vec![IR::String(key_name.to_string()), value]));
                        spread_entries.push(IR::op_with_pos(
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
                } else if current.kind() == "dict_spread" {
                    let spread_children = named_children(&current);
                    if spread_children.is_empty() {
                        return Err("dict_spread: missing value".into());
                    }
                    let mapping = transform(spread_children[0], source)?;
                    spread_entries.push(IR::op_with_pos(
                        IROpCode::TupleLiteral,
                        vec![IR::Bool(true), mapping],
                        current.start_byte(),
                        current.end_byte(),
                    ));
                    has_spread = true;
                }
            }
        }
    }

    if has_spread {
        return Ok(IR::Call {
            func: Box::new(IR::Ref("__catnip_spread_dict".to_string(), -1, -1)),
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

    // Use DICT_LITERAL opcode to ensure proper evaluation
    Ok(IR::op_with_pos(
        IROpCode::DictLiteral,
        pair_tuples,
        node.start_byte(),
        node.end_byte(),
    ))
}

fn transform_call(node: Node, source: &str) -> TransformResult {
    let mut func: Option<IR> = None;
    let mut func_name: Option<String> = None;
    let mut args = Vec::new();
    let mut kwargs = IndexMap::new();

    for child in named_children(&node) {
        if child.kind() == "identifier" {
            let name = node_text(&child, source);
            func_name = Some(name.to_string());
            func = Some(IR::Ref(
                name.to_string(),
                child.start_byte() as isize,
                child.end_byte() as isize,
            ));
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
            return Ok(IR::Op {
                opcode: IROpCode::Pragma,
                args,
                kwargs,
                tail: false,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            });
        }
    }

    Ok(IR::Call {
        func: Box::new(func_node),
        args,
        kwargs,
        tail: false,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    })
}

/// Resolve ND.thread → "thread" (pragma_qualified is just a qualified constant lookup)
fn transform_pragma_qualified(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    // children[0] = namespace identifier, children[1] = attr identifier
    if children.len() >= 2 {
        let attr = node_text(&children[1], source);
        Ok(IR::String(attr.to_string()))
    } else {
        Err("pragma_qualified: expected namespace.attr".to_string())
    }
}

fn transform_pragma_stmt(node: Node, source: &str) -> TransformResult {
    // pragma_stmt node has children that are the arguments to pragma()
    // Transform them into an Op(Pragma, args)
    let mut args = Vec::new();

    for child in named_children(&node) {
        args.push(transform(child, source)?);
    }

    Ok(IR::Op {
        opcode: IROpCode::Pragma,
        args,
        kwargs: IndexMap::new(),
        tail: false,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    })
}

// -- import statement --------------------------------------------------------

/// Check if a call node is `import("string_literal")` -- the statement form.
/// Returns true only when: function is `import`, exactly one positional arg
/// that is a string literal (not a computed expression), no kwargs.
fn is_import_call_string_literal(node: &Node, source: &str) -> bool {
    if node.kind() != "call" {
        return false;
    }
    let children = named_children(node);
    let func = children.iter().find(|c| c.kind() == "identifier");
    let args_node = children.iter().find(|c| c.kind() == "arguments");

    let (Some(f), Some(a)) = (func, args_node) else {
        return false;
    };
    if node_text(f, source) != "import" {
        return false;
    }

    // Collect positional args (unwrapping the `args` wrapper node)
    let mut positional: Vec<Node> = Vec::new();
    let mut has_kwargs = false;
    for child in named_children(a) {
        match child.kind() {
            "args" => positional.extend(named_children(&child)),
            "kwargs" | "kwarg" | "keyword_argument" | "args_kwargs" => has_kwargs = true,
            _ => positional.push(child),
        }
    }
    if positional.len() != 1 || has_kwargs {
        return false;
    }

    // The single arg must be a literal > string (not a call, identifier, etc.)
    let arg = &positional[0];
    if arg.kind() == "literal" {
        let lit_children = named_children(arg);
        return lit_children.len() == 1 && lit_children[0].kind() == "string";
    }
    false
}

fn derive_import_name(spec: &str) -> Result<String, String> {
    let stripped = spec.trim_start_matches('.');
    if stripped.contains('.') {
        return Err(format!(
            "ambiguous module name '{}', use: name = import(\"{}\")",
            spec, spec
        ));
    }
    if stripped.is_empty() {
        return Err("import: empty module spec".to_string());
    }
    Ok(stripped.to_string())
}

/// Extract the string node from the single argument of an import call.
/// Expects the CST shape: call > arguments > [args >] literal > string.
fn extract_import_string_node<'a>(call_node: &Node<'a>) -> Option<Node<'a>> {
    let children = named_children(call_node);
    let args_node = children.iter().find(|c| c.kind() == "arguments")?;
    // Unwrap the optional `args` wrapper
    let mut positional: Vec<Node<'a>> = Vec::new();
    for child in named_children(args_node) {
        if child.kind() == "args" {
            positional.extend(named_children(&child));
        } else if !matches!(child.kind(), "kwargs" | "kwarg" | "keyword_argument" | "args_kwargs") {
            positional.push(child);
        }
    }
    let literal = positional.first().filter(|n| n.kind() == "literal")?;
    let lit_children = named_children(literal);
    lit_children.first().filter(|n| n.kind() == "string").copied()
}

/// Desugar `import('spec')` in statement position to `spec_name = import('spec')`.
fn desugar_import_statement(call_node: Node, source: &str) -> TransformResult {
    // Extract the string literal from the single argument
    let string_node = extract_import_string_node(&call_node)
        .ok_or_else(|| "import statement: missing string literal argument".to_string())?;

    let raw = node_text(&string_node, source);
    let spec_text = raw.trim_matches(|c: char| c == '"' || c == '\'');
    let binding_name = derive_import_name(spec_text)?;

    // Transform the call normally first
    let call_ir = transform_call(call_node, source)?;

    let start = call_node.start_byte();
    let end = call_node.end_byte();

    // Wrap in SetLocals: name = import(spec)
    Ok(IR::op_with_pos(
        IROpCode::SetLocals,
        vec![IR::Tuple(vec![IR::Identifier(binding_name)]), call_ir, IR::Bool(false)],
        start,
        end,
    ))
}

/// Transform a statement node. Detects import('spec') calls for auto-binding.
/// Falls back to normal call transform if the spec can't derive a valid name.
fn transform_statement(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.len() == 1 && is_import_call_string_literal(&children[0], source) {
        if let Ok(ir) = desugar_import_statement(children[0], source) {
            return Ok(ir);
        }
    }
    // Default: transparent, transform the child
    transform_children(node, source)
}

/// Map operator symbol + param count to internal method name.
/// Disambiguation: `-`/`+` with 1 param = unary, 2 params = binary.
fn operator_symbol_to_method_name(sym: &str, param_count: usize) -> Option<&'static str> {
    // Normalize multi-word operators (tree-sitter seq may include variable whitespace)
    let normalized: String;
    let sym = if sym.contains(' ') {
        normalized = sym.split_whitespace().collect::<Vec<_>>().join(" ");
        normalized.as_str()
    } else {
        sym
    };
    match (sym, param_count) {
        // Binary arithmetic
        ("+", 2) => Some("op_add"),
        ("-", 2) => Some("op_sub"),
        ("*", 2) => Some("op_mul"),
        ("/", 2) => Some("op_div"),
        ("//", 2) => Some("op_floordiv"),
        ("%", 2) => Some("op_mod"),
        ("**", 2) => Some("op_pow"),
        // Binary comparison
        ("==", 2) => Some("op_eq"),
        ("!=", 2) => Some("op_ne"),
        ("<", 2) => Some("op_lt"),
        ("<=", 2) => Some("op_le"),
        (">", 2) => Some("op_gt"),
        (">=", 2) => Some("op_ge"),
        // Binary bitwise
        ("&", 2) => Some("op_band"),
        ("|", 2) => Some("op_bor"),
        ("^", 2) => Some("op_bxor"),
        ("<<", 2) => Some("op_lshift"),
        (">>", 2) => Some("op_rshift"),
        // Membership
        ("in", 2) => Some("op_in"),
        ("not in", 2) => Some("op_not_in"),
        // Unary
        ("-", 1) => Some("op_neg"),
        ("+", 1) => Some("op_pos"),
        ("~", 1) => Some("op_bnot"),
        _ => None,
    }
}

fn transform_struct_stmt(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let mut name: Option<String> = None;
    let mut implements: Vec<String> = Vec::new();
    let mut bases: Vec<String> = Vec::new();
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut seen_default = false;

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
                        bases.push(node_text(&grandchild, source).to_string());
                    }
                }
            }
            "struct_field" => {
                let mut field_name: Option<String> = None;
                let mut default: Option<IR> = None;
                for grandchild in named_children(child) {
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
                    return Err(format!("non-default field '{}' follows default field", fname));
                }
                let has_default = default.is_some();
                let default_val = default.unwrap_or(IR::None);
                fields.push(IR::Tuple(vec![IR::String(fname), IR::Bool(has_default), default_val]));
            }
            "struct_method" => {
                let mut method_name: Option<String> = None;
                let mut operator_symbol: Option<String> = None;
                let mut params = Vec::new();
                let mut body: Option<IR> = None;
                let mut decorators: Vec<String> = Vec::new();

                for method_child in named_children(child) {
                    match method_child.kind() {
                        "decorator" => {
                            for d_child in named_children(&method_child) {
                                if d_child.kind() == "identifier" {
                                    decorators.push(node_text(&d_child, source).to_string());
                                }
                            }
                        }
                        "operator_symbol" => {
                            operator_symbol = Some(node_text(&method_child, source).to_string());
                        }
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

                // Resolve operator symbol to method name
                if let Some(sym) = operator_symbol {
                    let param_count = params.len();
                    method_name = Some(
                        operator_symbol_to_method_name(&sym, param_count)
                            .ok_or_else(|| format!("invalid operator '{}' with {} param(s)", sym, param_count))?
                            .to_string(),
                    );
                }

                let has_abstract = decorators.iter().any(|d| d == "abstract");
                let has_static = decorators.iter().any(|d| d == "static");

                if let Some(ref mname) = method_name {
                    if has_abstract && mname == "init" {
                        return Err("init cannot be abstract".into());
                    }
                    if has_static && mname == "init" {
                        return Err("init cannot be static".into());
                    }
                    if !has_abstract && body.is_none() {
                        return Err(format!("method '{}' has no body (add @abstract or => {{...}})", mname));
                    }
                    // Validate: @static method must not have self as first param
                    if has_static && !params.is_empty() {
                        if let IR::Tuple(ref pair) = params[0] {
                            if let Some(IR::String(ref pname)) = pair.first() {
                                if pname == "self" {
                                    return Err(format!("@static method '{}' must not have 'self' parameter", mname));
                                }
                            }
                        }
                    }
                }

                if let Some(mname) = method_name {
                    let is_static_ir = IR::Bool(has_static);
                    if has_abstract {
                        // Abstract method: (name, None, is_static)
                        methods.push(IR::Tuple(vec![IR::String(mname), IR::None, is_static_ir]));
                    } else {
                        let mbody = body.unwrap(); // safe: validated above
                        let lambda_ir = IR::op_with_pos(
                            IROpCode::OpLambda,
                            vec![IR::Tuple(params), mbody],
                            child.start_byte(),
                            child.end_byte(),
                        );
                        methods.push(IR::Tuple(vec![IR::String(mname), lambda_ir, is_static_ir]));
                    }
                }
            }
            _ => {}
        }
    }

    let name_str = name.ok_or("struct_stmt: missing name")?;

    // Build args: (name, fields, [implements], [base], [methods])
    let mut args = vec![IR::String(name_str), IR::Tuple(fields)];

    // Add implements list if present
    if !implements.is_empty() {
        args.push(IR::List(implements.into_iter().map(IR::String).collect()));
    } else if !bases.is_empty() || !methods.is_empty() {
        // Placeholder if bases or methods follow
        args.push(IR::List(Vec::new()));
    }

    // Add extends bases if present (as a list of strings)
    if !bases.is_empty() {
        args.push(IR::List(bases.into_iter().map(IR::String).collect()));
    } else if !methods.is_empty() {
        // Placeholder if methods follow
        args.push(IR::None);
    }

    // Add methods if present
    if !methods.is_empty() {
        args.push(IR::List(methods));
    }

    Ok(IR::op_with_pos(
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
    let mut fields: Vec<IR> = Vec::new();
    let mut methods: Vec<IR> = Vec::new();
    let mut seen_default = false;

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
            "struct_field" => {
                let mut field_name: Option<String> = None;
                let mut default: Option<IR> = None;
                for grandchild in named_children(child) {
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
                fields.push(IR::Tuple(vec![
                    IR::String(field_name.unwrap_or_default()),
                    IR::Bool(has_default),
                    default.unwrap_or(IR::None),
                ]));
            }
            "struct_method" => {
                let mut method_name: Option<String> = None;
                let mut operator_symbol: Option<String> = None;
                let mut params = Vec::new();
                let mut body: Option<IR> = None;
                let mut decorators: Vec<String> = Vec::new();

                for grandchild in named_children(child) {
                    match grandchild.kind() {
                        "decorator" => {
                            for d_child in named_children(&grandchild) {
                                if d_child.kind() == "identifier" {
                                    decorators.push(node_text(&d_child, source).to_string());
                                }
                            }
                        }
                        "operator_symbol" => {
                            operator_symbol = Some(node_text(&grandchild, source).to_string());
                        }
                        "identifier" if method_name.is_none() => {
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

                // Resolve operator symbol to method name
                if let Some(sym) = operator_symbol {
                    let param_count = params.len();
                    method_name = Some(
                        operator_symbol_to_method_name(&sym, param_count)
                            .ok_or_else(|| format!("invalid operator '{}' with {} param(s)", sym, param_count))?
                            .to_string(),
                    );
                }

                let has_abstract = decorators.iter().any(|d| d == "abstract");
                let has_static = decorators.iter().any(|d| d == "static");

                if let Some(ref mname) = method_name {
                    if has_abstract && mname == "init" {
                        return Err("init cannot be abstract".into());
                    }
                    if has_static && mname == "init" {
                        return Err("init cannot be static".into());
                    }
                    if !has_abstract && body.is_none() {
                        return Err(format!(
                            "trait method '{}' has no body (add @abstract or => {{...}})",
                            mname
                        ));
                    }
                    // Validate: @static method must not have self as first param
                    if has_static && !params.is_empty() {
                        if let IR::Tuple(ref pair) = params[0] {
                            if let Some(IR::String(ref pname)) = pair.first() {
                                if pname == "self" {
                                    return Err(format!("@static method '{}' must not have 'self' parameter", mname));
                                }
                            }
                        }
                    }
                }

                if let Some(mname) = method_name {
                    let is_static_ir = IR::Bool(has_static);
                    if has_abstract {
                        // Abstract method: (name, None, is_static)
                        methods.push(IR::Tuple(vec![IR::String(mname), IR::None, is_static_ir]));
                    } else {
                        let mbody = body.unwrap(); // safe: validated above
                        let lambda = IR::op(IROpCode::OpLambda, vec![IR::List(params), mbody]);
                        methods.push(IR::Tuple(vec![IR::String(mname), lambda, is_static_ir]));
                    }
                }
            }
            _ => {}
        }
    }

    let name_str = name.ok_or("trait_stmt: missing name")?;

    let mut args = vec![
        IR::String(name_str),
        IR::List(extends.into_iter().map(IR::String).collect()),
        IR::Tuple(fields),
    ];

    if !methods.is_empty() {
        args.push(IR::List(methods));
    }

    Ok(IR::op_with_pos(
        IROpCode::TraitDef,
        args,
        node.start_byte(),
        node.end_byte(),
    ))
}

/// Transform enum definition: `enum Name { variant1; variant2; ... }`
fn transform_enum_stmt(node: Node, source: &str) -> TransformResult {
    let mut name: Option<String> = None;
    let mut variants: Vec<IR> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for child in named_children(&node) {
        match child.kind() {
            "identifier" if name.is_none() => {
                name = Some(node_text(&child, source).to_string());
            }
            "enum_variant" => {
                // The variant has a single named child: the identifier
                let vname = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source).to_string())
                    .ok_or_else(|| "enum_variant: missing name".to_string())?;
                if !seen.insert(vname.clone()) {
                    return Err(format!(
                        "enum '{}': duplicate variant '{}'",
                        name.as_deref().unwrap_or("?"),
                        vname
                    ));
                }
                variants.push(IR::String(vname));
            }
            _ => {}
        }
    }

    let name_str = name.ok_or("enum_stmt: missing name")?;

    if variants.is_empty() {
        return Err(format!("enum '{}' must have at least one variant", name_str));
    }

    Ok(IR::op_with_pos(
        IROpCode::EnumDef,
        vec![IR::String(name_str), IR::Tuple(variants)],
        node.start_byte(),
        node.end_byte(),
    ))
}

/// Parse lambda_params node into a Vec of IR param tuples.
fn parse_lambda_params(node: &Node, source: &str) -> Result<Vec<IR>, String> {
    let mut params = Vec::new();
    for param_child in named_children(node) {
        if param_child.kind() == "lambda_param" {
            let param_children = named_children(&param_child);
            let mut name: Option<String> = None;
            let mut default: Option<IR> = None;

            for grandchild in &param_children {
                if grandchild.kind() == "identifier" {
                    name = Some(node_text(grandchild, source).to_string());
                } else {
                    default = Some(transform(*grandchild, source)?);
                }
            }

            let name_str = name.unwrap_or_default();
            let default_val = default.unwrap_or(IR::None);
            params.push(IR::Tuple(vec![IR::String(name_str), default_val]));
        } else if param_child.kind() == "variadic_param" {
            for grandchild in named_children(&param_child) {
                if grandchild.kind() == "identifier" {
                    let name_str = node_text(&grandchild, source).to_string();
                    params.push(IR::Tuple(vec![IR::String("*".to_string()), IR::String(name_str)]));
                }
            }
        }
    }
    Ok(params)
}

fn transform_lambda_expr(node: Node, source: &str) -> TransformResult {
    let mut params = Vec::new();
    let mut body: Option<IR> = None;

    for child in named_children(&node) {
        if child.kind() == "lambda_params" {
            params = parse_lambda_params(&child, source)?;
        } else if child.kind() == "block" {
            body = Some(transform(child, source)?);
        }
    }

    let body_node = body.ok_or_else(|| "Lambda without body".to_string())?;

    Ok(IR::op_with_pos(
        IROpCode::OpLambda,
        vec![IR::Tuple(params), body_node],
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
    Ok(IR::Program(transformed?))
}

// ============================================================================
// Helpers
// ============================================================================

fn transform_children(node: Node, source: &str) -> TransformResult {
    // named_children already filters out comments
    let children = named_children(&node);

    if children.is_empty() {
        Ok(IR::None)
    } else if children.len() == 1 {
        transform(children[0], source)
    } else {
        let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();
        Ok(IR::Tuple(transformed?))
    }
}

// ============================================================================
// Broadcasting & Chained Access
// ============================================================================

fn transform_chained(node: Node, source: &str) -> TransformResult {
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
                // GetItem: extract index and create GetItem op
                let index_children = named_children(child);
                if !index_children.is_empty() {
                    let index = transform(index_children[0], source)?;
                    result = IR::op(IROpCode::GetItem, vec![result, index]);
                }
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
        "broadcast_nd_map" => {
            // Function map: .[~> func]
            // Registry expects ND_MAP with (data_or_func, func_node)
            // In broadcast form: (func, None) for lift-like form
            let bc_children = named_children(&final_node);
            if bc_children.is_empty() {
                return Err("broadcast_nd_map: no function".into());
            }

            let func_node = bc_children[0];
            let func = transform(func_node, source)?;

            // Use IR::Broadcast structure with NDMap type
            Ok(IR::Broadcast {
                target: Some(Box::new(target)),
                operator: Box::new(IR::op(IROpCode::NdMap, vec![func, IR::None])),
                operand: None,
                broadcast_type: BroadcastType::NDMap,
            })
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
    let cases_tuple = IR::Tuple(cases);
    Ok(IR::op_with_pos(
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
    Ok(IR::PatternLiteral(Box::new(value)))
}

fn transform_pattern_var(node: Node, source: &str) -> TransformResult {
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

fn transform_pattern_wildcard(_node: Node, _source: &str) -> TransformResult {
    Ok(IR::PatternWildcard)
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
        ));
    }

    let name = node_text(&identifier_node, source).to_string();
    // Retourner tuple ('*', name)
    Ok(IR::Tuple(vec![IR::String("*".to_string()), IR::String(name)]))
}

fn transform_variadic_param(node: Node, source: &str) -> TransformResult {
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

    Ok(IR::PatternOr(patterns))
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

    Ok(IR::PatternTuple(patterns))
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
    Ok(IR::PatternStruct { name, fields })
}

fn transform_pattern_enum(node: Node, source: &str) -> TransformResult {
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
        "broadcast_nd_map" => {
            let children = named_children(&broadcast_op_node);
            if children.is_empty() {
                return Err("broadcast_nd_map: no expression".into());
            }
            let expr = transform(children[0], source)?;
            Ok(IR::Broadcast {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_transform(source: &str) -> TransformResult {
        use tree_sitter::Parser;

        let language = catnip_grammar::get_language();
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
        assert_eq!(result, IR::Int(42));
    }

    #[test]
    fn test_transform_addition() {
        let result = parse_and_transform("2 + 3").unwrap();

        match result {
            IR::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::Add);
                assert_eq!(args.len(), 2);
                assert_eq!(args[0], IR::Int(2));
                assert_eq!(args[1], IR::Int(3));
            }
            _ => panic!("Expected Op node"),
        }
    }

    #[test]
    fn test_transform_string() {
        let result = parse_and_transform("\"hello\"").unwrap();
        assert_eq!(result, IR::String("hello".into()));
    }

    #[test]
    fn test_transform_bool() {
        let result = parse_and_transform("True").unwrap();
        assert_eq!(result, IR::Bool(true));
    }

    #[test]
    fn test_transform_raise_bare() {
        let result = parse_and_transform("raise").unwrap();
        match result {
            IR::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::OpRaise);
                assert!(args.is_empty());
            }
            _ => panic!("Expected Op node"),
        }
    }

    #[test]
    fn test_transform_raise_expr() {
        let result = parse_and_transform("raise ValueError(\"msg\")").unwrap();
        match result {
            IR::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::OpRaise);
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected Op node"),
        }
    }

    #[test]
    fn test_transform_try_except_wildcard() {
        let result = parse_and_transform("try { 1 } except { _ => { 2 } }").unwrap();
        match result {
            IR::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::OpTry);
                assert_eq!(args.len(), 3);
                // args[2] = None (no finally)
                assert_eq!(args[2], IR::None);
                if let IR::List(handlers) = &args[1] {
                    assert_eq!(handlers.len(), 1);
                    if let IR::Tuple(clause) = &handlers[0] {
                        assert_eq!(clause[0], IR::List(vec![])); // wildcard = empty types
                        assert_eq!(clause[1], IR::None); // no binding
                    } else {
                        panic!("Expected Tuple clause");
                    }
                } else {
                    panic!("Expected List handlers");
                }
            }
            _ => panic!("Expected Op node"),
        }
    }

    #[test]
    fn test_transform_try_except_typed_with_binding() {
        let result = parse_and_transform("try { 1 } except { e: TypeError => { 2 } }").unwrap();
        match result {
            IR::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::OpTry);
                if let IR::List(handlers) = &args[1] {
                    assert_eq!(handlers.len(), 1);
                    if let IR::Tuple(clause) = &handlers[0] {
                        assert_eq!(clause[0], IR::List(vec![IR::String("TypeError".into())]));
                        assert_eq!(clause[1], IR::String("e".into()));
                    } else {
                        panic!("Expected Tuple clause");
                    }
                } else {
                    panic!("Expected List handlers");
                }
            }
            _ => panic!("Expected Op node"),
        }
    }

    #[test]
    fn test_transform_try_finally() {
        let result = parse_and_transform("try { 1 } finally { 2 }").unwrap();
        match result {
            IR::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::OpTry);
                assert_eq!(args.len(), 3);
                assert_eq!(args[1], IR::List(vec![])); // no handlers
                assert_ne!(args[2], IR::None); // finally present
            }
            _ => panic!("Expected Op node"),
        }
    }
}
