// FILE: catnip_core/src/parser/pure_transforms/with_stmt.rs
use super::*;

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
pub(crate) fn transform_with_stmt(node: Node, source: &str) -> TransformResult {
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

pub(crate) fn transform_block(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();

    Ok(IR::op_with_pos(
        IROpCode::OpBlock,
        transformed?,
        node.start_byte(),
        node.end_byte(),
    ))
}
