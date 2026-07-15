// FILE: catnip_core/src/parser/pure_transforms/definitions.rs
use super::*;

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

/// Extract `(name, type annotation, default)` from a field node
/// (`struct_field`, trait field, or `union_field`).
fn parse_struct_field(node: &Node, source: &str) -> Result<(Option<String>, IR, Option<IR>), String> {
    let mut field_name: Option<String> = None;
    let mut type_ann: IR = IR::None;
    let mut default: Option<IR> = None;
    for grandchild in named_children(node) {
        match grandchild.kind() {
            "identifier" if field_name.is_none() => {
                field_name = Some(node_text(&grandchild, source).to_string());
            }
            "type_expr" => {
                type_ann = IR::String(node_text(&grandchild, source).to_string());
            }
            _ if field_name.is_some() => {
                default = Some(transform(grandchild, source)?);
            }
            _ => {}
        }
    }
    Ok((field_name, type_ann, default))
}

pub(crate) fn transform_struct_stmt(node: Node, source: &str) -> TransformResult {
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
                let (field_name, type_ann, default) = parse_struct_field(child, source)?;
                let fname = field_name.unwrap_or_default();
                if default.is_some() {
                    seen_default = true;
                } else if seen_default {
                    return Err(format!("non-default field '{}' follows default field", fname));
                }
                let has_default = default.is_some();
                let default_val = default.unwrap_or(IR::None);
                fields.push(IR::Tuple(vec![
                    IR::String(fname),
                    IR::Bool(has_default),
                    default_val,
                    type_ann,
                ]));
            }
            "struct_method" => {
                let mut method_name: Option<String> = None;
                let mut operator_symbol: Option<String> = None;
                let mut params = Vec::new();
                let mut body: Option<IR> = None;
                let mut decorators: Vec<String> = Vec::new();
                let mut return_type: Option<IR> = None;

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
                        "type_expr" => {
                            return_type = Some(IR::String(node_text(&method_child, source).to_string()));
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
                        // Abstract method: (name, None, is_static, signature). No body =>
                        // no OpLambda to carry the parsed signature, so the param tuple
                        // (with its type slots) and the return type ride in a 4th element
                        // for SÉ3. Abstract is still detected via the None body slot;
                        // index 3 is ignored by the struct/trait compilers.
                        let signature = IR::Tuple(vec![IR::Tuple(params), return_type.unwrap_or(IR::None)]);
                        methods.push(IR::Tuple(vec![IR::String(mname), IR::None, is_static_ir, signature]));
                    } else {
                        let mbody = body.unwrap(); // safe: validated above
                        let mut lambda_args = vec![IR::Tuple(params), mbody];
                        if let Some(rt) = return_type {
                            lambda_args.push(rt);
                        }
                        let lambda_ir =
                            IR::op_with_pos(IROpCode::OpLambda, lambda_args, child.start_byte(), child.end_byte());
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

pub(crate) fn transform_trait_stmt(node: Node, source: &str) -> TransformResult {
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
                let (field_name, type_ann, default) = parse_struct_field(child, source)?;
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
                    type_ann,
                ]));
            }
            "struct_method" => {
                let mut method_name: Option<String> = None;
                let mut operator_symbol: Option<String> = None;
                let mut params = Vec::new();
                let mut body: Option<IR> = None;
                let mut decorators: Vec<String> = Vec::new();
                let mut return_type: Option<IR> = None;

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
                        "type_expr" => {
                            return_type = Some(IR::String(node_text(&grandchild, source).to_string()));
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
                        // Abstract method: (name, None, is_static, signature). No body =>
                        // no OpLambda to carry the parsed signature, so the param list
                        // (with its type slots) and the return type ride in a 4th element
                        // for SÉ3. Abstract is still detected via the None body slot;
                        // index 3 is ignored by the struct/trait compilers.
                        let signature = IR::Tuple(vec![IR::List(params), return_type.unwrap_or(IR::None)]);
                        methods.push(IR::Tuple(vec![IR::String(mname), IR::None, is_static_ir, signature]));
                    } else {
                        let mbody = body.unwrap(); // safe: validated above
                        let mut lambda_args = vec![IR::List(params), mbody];
                        if let Some(rt) = return_type {
                            lambda_args.push(rt);
                        }
                        let lambda = IR::op(IROpCode::OpLambda, lambda_args);
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
pub(crate) fn transform_enum_stmt(node: Node, source: &str) -> TransformResult {
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

/// Transform union definition: `union Name[T, ...] { Variant(field: T); ... }`
///
/// Produced IR: `UnionDef(name, type_params, variants)`
///
/// - `name` : `IR::String`
/// - `type_params` : `IR::List([IR::String])` -- empty if no generics
/// - `variants` : `IR::List([(variant_name, fields)])`
///   where each variant field is `(field_name, type_text_or_none)`
///
/// Type annotations are kept as raw source text for the MVP. They are
/// stored but not yet used by the semantic analyzer.
pub(crate) fn transform_union_stmt(node: Node, source: &str) -> TransformResult {
    let mut name: Option<String> = None;
    let mut type_params: Vec<IR> = Vec::new();
    let mut variants: Vec<IR> = Vec::new();
    let mut methods: Vec<IR> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut seen_methods = std::collections::HashSet::new();

    for child in named_children(&node) {
        match child.kind() {
            "identifier" if name.is_none() => {
                name = Some(node_text(&child, source).to_string());
            }
            "type_params" => {
                for grandchild in named_children(&child) {
                    if grandchild.kind() == "identifier" {
                        type_params.push(IR::String(node_text(&grandchild, source).to_string()));
                    }
                }
            }
            "union_variant" => {
                let mut vname: Option<String> = None;
                let mut fields: Vec<IR> = Vec::new();
                let mut seen_fields = std::collections::HashSet::new();

                for grandchild in named_children(&child) {
                    match grandchild.kind() {
                        "identifier" if vname.is_none() => {
                            vname = Some(node_text(&grandchild, source).to_string());
                        }
                        "union_field" => {
                            // union_field is name + optional type, never a default
                            let (fname, ftype, _) = parse_struct_field(&grandchild, source)?;
                            let fname_str = fname.ok_or("union_field: missing name")?;
                            if !seen_fields.insert(fname_str.clone()) {
                                return Err(format!(
                                    "union '{}' variant '{}': duplicate field '{}'",
                                    name.as_deref().unwrap_or("?"),
                                    vname.as_deref().unwrap_or("?"),
                                    fname_str
                                ));
                            }
                            fields.push(IR::Tuple(vec![IR::String(fname_str), ftype]));
                        }
                        _ => {}
                    }
                }

                let vname_str = vname.ok_or("union_variant: missing name")?;
                if !seen.insert(vname_str.clone()) {
                    return Err(format!(
                        "union '{}': duplicate variant '{}'",
                        name.as_deref().unwrap_or("?"),
                        vname_str
                    ));
                }
                variants.push(IR::Tuple(vec![IR::String(vname_str), IR::List(fields)]));
            }
            "union_method" => {
                let mut method_name: Option<String> = None;
                let mut params = Vec::new();
                let mut body: Option<IR> = None;
                let mut return_type: Option<IR> = None;

                for method_child in named_children(&child) {
                    match method_child.kind() {
                        "identifier" if method_name.is_none() => {
                            method_name = Some(node_text(&method_child, source).to_string());
                        }
                        "lambda_params" => {
                            params = parse_lambda_params(&method_child, source)?;
                        }
                        "type_expr" => {
                            return_type = Some(IR::String(node_text(&method_child, source).to_string()));
                        }
                        "block" => {
                            body = Some(transform(method_child, source)?);
                        }
                        _ => {}
                    }
                }

                let mname = method_name.ok_or("union_method: missing name")?;
                let union_name = name.as_deref().unwrap_or("?");
                if mname == "init" {
                    return Err(format!(
                        "union '{}': no init method -- the variants are the constructors",
                        union_name
                    ));
                }
                // Methods and variants share the `Union.name` namespace.
                if seen.contains(&mname) {
                    return Err(format!(
                        "union '{}': method '{}' collides with a variant of the same name",
                        union_name, mname
                    ));
                }
                if !seen_methods.insert(mname.clone()) {
                    return Err(format!("union '{}': duplicate method '{}'", union_name, mname));
                }
                let mbody = body.ok_or_else(|| format!("union '{}': method '{}' has no body", union_name, mname))?;
                let mut lambda_args = vec![IR::Tuple(params), mbody];
                if let Some(rt) = return_type {
                    lambda_args.push(rt);
                }
                let lambda_ir = IR::op_with_pos(IROpCode::OpLambda, lambda_args, child.start_byte(), child.end_byte());
                methods.push(IR::Tuple(vec![IR::String(mname), lambda_ir]));
            }
            _ => {}
        }
    }

    let name_str = name.ok_or("union_stmt: missing name")?;

    if variants.is_empty() {
        return Err(format!("union '{}' must have at least one variant", name_str));
    }

    let mut args = vec![IR::String(name_str), IR::List(type_params), IR::List(variants)];
    if !methods.is_empty() {
        args.push(IR::List(methods));
    }

    Ok(IR::op_with_pos(
        IROpCode::UnionDef,
        args,
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
            let mut type_ann: IR = IR::None;
            let mut default: Option<IR> = None;

            for grandchild in &param_children {
                match grandchild.kind() {
                    "identifier" if name.is_none() => {
                        name = Some(node_text(grandchild, source).to_string());
                    }
                    "type_expr" => {
                        type_ann = IR::String(node_text(grandchild, source).to_string());
                    }
                    _ => {
                        default = Some(transform(*grandchild, source)?);
                    }
                }
            }

            let name_str = name.unwrap_or_default();
            let default_val = default.unwrap_or(IR::None);
            params.push(IR::Tuple(vec![IR::String(name_str), default_val, type_ann]));
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

pub(crate) fn transform_lambda_expr(node: Node, source: &str) -> TransformResult {
    let mut params = Vec::new();
    let mut body: Option<IR> = None;
    let mut return_type: Option<IR> = None;

    for child in named_children(&node) {
        match child.kind() {
            "lambda_params" => params = parse_lambda_params(&child, source)?,
            "type_expr" => return_type = Some(IR::String(node_text(&child, source).to_string())),
            "block" => body = Some(transform(child, source)?),
            _ => {}
        }
    }

    let body_node = body.ok_or_else(|| "Lambda without body".to_string())?;

    let mut lambda_args = vec![IR::Tuple(params), body_node];
    if let Some(rt) = return_type {
        lambda_args.push(rt);
    }

    Ok(IR::op_with_pos(
        IROpCode::OpLambda,
        lambda_args,
        node.start_byte(),
        node.end_byte(),
    ))
}
