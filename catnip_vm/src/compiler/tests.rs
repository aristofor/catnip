//! Tests for the bytecode compiler.

use super::*;
use catnip_core::ir::pure::IR;

#[test]
fn test_compile_int_literal() {
    let mut compiler = PureCompiler::new();
    let ir = IR::Program(vec![IR::Int(42)]);
    let output = compiler.compile(&ir).unwrap();
    assert!(!output.code.instructions.is_empty());
    assert_eq!(output.code.constants.len(), 1);
}

#[test]
fn test_compile_string_literal() {
    let mut compiler = PureCompiler::new();
    let ir = IR::Program(vec![IR::String("hello".to_string())]);
    let output = compiler.compile(&ir).unwrap();
    assert_eq!(output.code.constants.len(), 1);
    assert!(output.code.constants[0].is_native_str());
}

#[test]
fn test_compile_binary_add() {
    let mut compiler = PureCompiler::new();
    let ir = IR::Program(vec![IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)])]);
    let output = compiler.compile(&ir).unwrap();
    // Should have: LoadConst(1), LoadConst(2), Add, Halt
    assert!(output.code.instructions.len() >= 3);
}

#[test]
fn test_compile_and_rejects_extra_operands() {
    // unwrap_binary_args takes exactly two operands; a third must error, not be dropped.
    let mut compiler = PureCompiler::new();
    let ir = IR::Program(vec![IR::op(
        IROpCode::And,
        vec![IR::Bool(true), IR::Bool(false), IR::Bool(true)],
    )]);
    let result = compiler.compile(&ir);
    assert!(matches!(result, Err(CompileError::ValueError(_))));
}

#[test]
fn test_compile_decimal_unsupported() {
    let mut compiler = PureCompiler::new();
    let ir = IR::Program(vec![IR::Decimal("1.5".to_string())]);
    let result = compiler.compile(&ir);
    assert!(result.is_err());
    if let Err(CompileError::UnsupportedLiteral(_)) = result {
    } else {
        panic!("expected UnsupportedLiteral");
    }
}

#[test]
fn test_compile_lambda() {
    let mut compiler = PureCompiler::new();
    let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
    let params = IR::List(vec![IR::Identifier("n".into())]);
    let lambda_ir = IR::op(IROpCode::OpLambda, vec![params, body]);
    let ir = IR::Program(vec![lambda_ir]);
    let output = compiler.compile(&ir).unwrap();
    assert_eq!(output.functions.len(), 1);
    assert_eq!(output.functions[0].name, "<lambda>");
}

/// Build `name = <decorator>(lambda)` as IR (the shape `@decorator` lowers to),
/// or a plain `name = lambda` when `decorator` is None.
fn assign_lambda(decorator: Option<&str>) -> IR {
    let body = IR::op(
        IROpCode::Mul,
        vec![IR::Identifier("x".into()), IR::Identifier("x".into())],
    );
    let params = IR::List(vec![IR::Identifier("x".into())]);
    let lambda = IR::op(IROpCode::OpLambda, vec![params, body]);
    let value = match decorator {
        Some(name) => IR::call(IR::Ref(name.into(), 0, 0), vec![lambda]),
        None => lambda,
    };
    IR::Program(vec![IR::op(
        IROpCode::SetLocals,
        vec![IR::Tuple(vec![IR::Ref("square".into(), 0, 0)]), value],
    )])
}

#[test]
fn test_pure_decorator_marks_codeobject() {
    // `@pure square = (x) => x * x` must set is_pure on the lambda's CodeObject
    // statically, so the JIT records calls to it as CallPure.
    let mut compiler = PureCompiler::new();
    let output = compiler.compile(&assign_lambda(Some("pure"))).unwrap();
    assert_eq!(output.functions.len(), 1);
    assert!(output.functions[0].is_pure, "@pure should mark the lambda pure");
}

#[test]
fn test_plain_lambda_not_pure() {
    let mut compiler = PureCompiler::new();
    let output = compiler.compile(&assign_lambda(None)).unwrap();
    assert_eq!(output.functions.len(), 1);
    assert!(!output.functions[0].is_pure);
}

#[test]
fn test_other_decorator_not_pure() {
    // Only `pure` marks; `jit` (or any other) must not.
    let mut compiler = PureCompiler::new();
    let output = compiler.compile(&assign_lambda(Some("jit"))).unwrap();
    assert_eq!(output.functions.len(), 1);
    assert!(!output.functions[0].is_pure);
}

#[test]
fn test_compile_if() {
    let mut compiler = PureCompiler::new();
    let ir = IR::op(
        IROpCode::OpIf,
        vec![IR::Tuple(vec![IR::Tuple(vec![IR::Bool(true), IR::Int(1)])]), IR::Int(2)],
    );
    let program = IR::Program(vec![ir]);
    let output = compiler.compile(&program).unwrap();
    assert!(!output.code.instructions.is_empty());
}

/// Regression: for + if + SetItem caused a stack imbalance.
/// compile_body must always leave exactly 1 value for compile_if symmetry.
/// Without the fix, SetItem (void op) left nothing, and the subsequent
/// PopTop in compile_body_void consumed the for-loop iterator.
#[test]
fn test_compile_for_if_setitem_stack_balance() {
    let mut compiler = PureCompiler::new();
    // for x in data { if (x != "b") { data[0] = x } }
    let setitem = IR::op(
        IROpCode::SetItem,
        vec![IR::Identifier("data".into()), IR::Int(0), IR::Identifier("x".into())],
    );
    let if_body = IR::op(IROpCode::OpBlock, vec![setitem]);
    let condition = IR::op(IROpCode::Ne, vec![IR::Identifier("x".into()), IR::String("b".into())]);
    let if_node = IR::op(
        IROpCode::OpIf,
        vec![IR::Tuple(vec![IR::Tuple(vec![condition, if_body])])],
    );
    let for_body = IR::op(IROpCode::OpBlock, vec![if_node]);
    let for_node = IR::op(
        IROpCode::OpFor,
        vec![IR::Identifier("x".into()), IR::Identifier("data".into()), for_body],
    );
    let program = IR::Program(vec![for_node]);
    let output = compiler.compile(&program).unwrap();
    // Verify compilation succeeds and produces instructions.
    // The real test is that this doesn't cause a segfault at runtime
    // (ForIter finding NoneType instead of iterator on the stack).
    assert!(!output.code.instructions.is_empty());

    // Verify stack balance: count pushes vs pops in the for-loop body
    // (between ForIter and Jump back). The body should be net-zero
    // on each iteration so the iterator stays on top.
    let instrs = &output.code.instructions;
    let for_iter_pos = instrs.iter().position(|i| i.op == VMOpCode::ForIter).unwrap();
    let jump_back_pos = instrs.iter().rposition(|i| i.op == VMOpCode::Jump).unwrap();

    let mut stack_delta: i32 = 0;
    for instr in &instrs[for_iter_pos + 1..jump_back_pos] {
        match instr.op {
            VMOpCode::LoadConst | VMOpCode::LoadLocal | VMOpCode::LoadGlobal => stack_delta += 1,
            VMOpCode::StoreLocal | VMOpCode::PopTop => stack_delta -= 1,
            VMOpCode::SetItem => stack_delta -= 3,
            VMOpCode::JumpIfFalse => stack_delta -= 1,
            _ => {}
        }
    }
    // ForIter pushes the next value (+1), StoreLocal pops it (-1),
    // then body should be net-zero. Overall delta = 0.
    assert_eq!(
        stack_delta, 0,
        "for-loop body has stack imbalance: delta = {stack_delta}"
    );
}

#[test]
fn test_compile_set_locals() {
    let mut compiler = PureCompiler::new();
    let ir = IR::op(
        IROpCode::SetLocals,
        vec![IR::Tuple(vec![IR::Identifier("x".into())]), IR::Int(42)],
    );
    let program = IR::Program(vec![ir]);
    let output = compiler.compile(&program).unwrap();
    assert!(!output.code.instructions.is_empty());
}

#[test]
fn test_try_compiles() {
    let mut compiler = PureCompiler::new();
    let ir = IR::op(
        IROpCode::OpTry,
        vec![IR::op(IROpCode::OpBlock, vec![IR::Int(1)]), IR::List(vec![]), IR::None],
    );
    let program = IR::Program(vec![ir]);
    let output = compiler.compile(&program);
    assert!(output.is_ok(), "try should compile: {:?}", output.err());
}

#[test]
fn test_raise_compiles() {
    let mut compiler = PureCompiler::new();
    let ir = IR::op(IROpCode::OpRaise, vec![]);
    let program = IR::Program(vec![ir]);
    let output = compiler.compile(&program);
    assert!(output.is_ok(), "bare raise should compile: {:?}", output.err());
}

// --- CodeObject pool ownership (constants/defaults/pattern literals) ---
//
// The pools own one reference per slot; Clone takes one per copy and Drop is
// the release site. Oracle: Arc::strong_count on a NativeString witness
// (same pattern as module_namespace_drop_releases_attrs).

#[test]
fn code_object_pools_balance_through_compile_clone_drop() {
    use crate::collections::ValueKey;
    use std::sync::Arc;

    let mut compiler = PureCompiler::new();
    let ir = IR::Program(vec![IR::String("pool-witness-xyz".into())]);
    let output = compiler.compile(&ir).unwrap();

    let sval = output
        .code
        .constants
        .iter()
        .find(|v| v.is_native_str())
        .copied()
        .expect("string literal pooled as a native str constant");
    let witness = match sval.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(Arc::strong_count(&witness), 2, "pool ref + witness");

    // Pools MOVED into the CodeObject: the compiler buffers hold nothing.
    drop(compiler);
    assert_eq!(
        Arc::strong_count(&witness),
        2,
        "compiler drop releases nothing (pools moved)"
    );

    // Clone takes its own reference per slot; Drop releases it.
    let dup = output.code.clone();
    assert_eq!(Arc::strong_count(&witness), 3, "clone owns its pool ref");
    drop(dup);
    assert_eq!(Arc::strong_count(&witness), 2, "clone released its ref");

    drop(output);
    assert_eq!(
        Arc::strong_count(&witness),
        1,
        "pool released on CodeObject drop (witness only)"
    );
}

#[test]
fn code_object_drop_releases_defaults_and_pattern_literals() {
    use crate::collections::ValueKey;
    use std::sync::Arc;

    let d = Value::from_str("default-witness");
    let p = Value::from_str("pattern-witness");
    let wd = match d.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };
    let wp = match p.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };

    let code = CodeObject {
        instructions: vec![],
        constants: vec![],
        names: vec![],
        nlocals: 0,
        varnames: vec![],
        slotmap: Default::default(),
        nargs: 1,
        defaults: vec![d],
        name: "pool_probe".into(),
        freevars: vec![],
        vararg_idx: -1,
        is_pure: false,
        complexity: 0,
        line_table: vec![],
        // Nested literal: exercises the recursive walk (Or > Tuple > Literal).
        patterns: vec![VMPattern::Or(vec![
            VMPattern::Tuple(vec![VMPatternElement::Pattern(VMPattern::Literal(p))]),
            VMPattern::Wildcard,
        ])],
        union_checks: vec![],
        composite_checks: vec![],
        generic_checks: vec![],
        encoded_ir: None,
    };
    assert_eq!(Arc::strong_count(&wd), 2, "default pooled");
    assert_eq!(Arc::strong_count(&wp), 2, "pattern literal pooled");

    drop(code);
    assert_eq!(Arc::strong_count(&wd), 1, "default released on drop");
    assert_eq!(Arc::strong_count(&wp), 1, "pattern literal released on drop");
}

#[test]
fn add_const_dedup_releases_candidate() {
    use crate::collections::ValueKey;
    use std::sync::Arc;

    let pooled = Value::from_str("dedup-witness");
    let candidate = Value::from_str("dedup-witness"); // equal by content, distinct allocation
    let witness = match candidate.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(Arc::strong_count(&witness), 2, "candidate + witness");

    let mut core = CompilerCore::new();
    let i1 = core.add_const(pooled);
    let i2 = core.add_const(candidate);
    assert_eq!(i1, i2, "native strings dedup by content");
    assert_eq!(
        Arc::strong_count(&witness),
        1,
        "deduped candidate released its ref (witness is the only holder)"
    );
    // `core` still owns `pooled`; released by CompilerCore::drop.
}
