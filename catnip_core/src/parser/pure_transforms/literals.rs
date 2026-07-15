// FILE: catnip_core/src/parser/pure_transforms/literals.rs
use super::*;

// ============================================================================
// Literals
// ============================================================================

pub(crate) fn transform_number(node: Node, source: &str) -> TransformResult {
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

pub(crate) fn transform_string(node: Node, source: &str) -> TransformResult {
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

pub(crate) fn transform_bstring(node: Node, source: &str) -> TransformResult {
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

pub(crate) fn transform_fstring(node: Node, source: &str) -> TransformResult {
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

pub(crate) fn transform_bool(node: Node, source: &str) -> TransformResult {
    let text = node_text(&node, source);
    match text {
        "True" => Ok(IR::Bool(true)),
        "False" => Ok(IR::Bool(false)),
        _ => Err(format!("Invalid bool literal: {}", text)),
    }
}

pub(crate) fn transform_nd_empty_topos(_node: Node, _source: &str) -> TransformResult {
    // ~[] - empty non-deterministic topos
    Ok(IR::op(IROpCode::NdEmptyTopos, vec![]))
}

pub(crate) fn transform_nd_recursion(node: Node, source: &str) -> TransformResult {
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

pub(crate) fn transform_nd_map(node: Node, source: &str) -> TransformResult {
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
