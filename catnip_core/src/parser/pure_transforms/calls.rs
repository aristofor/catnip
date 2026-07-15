// FILE: catnip_core/src/parser/pure_transforms/calls.rs
use super::*;

pub(crate) fn transform_call(node: Node, source: &str) -> TransformResult {
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
pub(crate) fn transform_pragma_qualified(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    // children[0] = namespace identifier, children[1] = attr identifier
    if children.len() >= 2 {
        let attr = node_text(&children[1], source);
        Ok(IR::String(attr.to_string()))
    } else {
        Err("pragma_qualified: expected namespace.attr".to_string())
    }
}

pub(crate) fn transform_pragma_stmt(node: Node, source: &str) -> TransformResult {
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
pub(crate) fn transform_statement(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    if children.len() == 1 && is_import_call_string_literal(&children[0], source) {
        if let Ok(ir) = desugar_import_statement(children[0], source) {
            return Ok(ir);
        }
    }
    // Default: transparent, transform the child
    transform_children(node, source)
}
