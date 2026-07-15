//! Unit tests for the pure VM dispatch loop and native operations.

use super::*;
use crate::compiler::code_object::CodeObject;
use crate::host::PureHost;
use catnip_core::vm::opcode::Instruction;

fn make_code(instructions: Vec<Instruction>, constants: Vec<Value>) -> Arc<CodeObject> {
    Arc::new(CodeObject {
        instructions,
        constants,
        names: vec![],
        nlocals: 0,
        varnames: vec![],
        slotmap: Default::default(),
        nargs: 0,
        defaults: vec![],
        name: "test".into(),
        freevars: vec![],
        vararg_idx: -1,
        is_pure: false,
        complexity: 0,
        line_table: vec![],
        patterns: vec![],
        union_checks: vec![],
        composite_checks: vec![],
        generic_checks: vec![],
        encoded_ir: None,
    })
}

fn make_code_with_locals(
    instructions: Vec<Instruction>,
    constants: Vec<Value>,
    nlocals: usize,
    varnames: Vec<String>,
) -> Arc<CodeObject> {
    let slotmap = varnames.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
    Arc::new(CodeObject {
        instructions,
        constants,
        names: vec![],
        nlocals,
        varnames,
        slotmap,
        nargs: 0,
        defaults: vec![],
        name: "test".into(),
        freevars: vec![],
        vararg_idx: -1,
        is_pure: false,
        complexity: 0,
        line_table: vec![],
        patterns: vec![],
        union_checks: vec![],
        composite_checks: vec![],
        generic_checks: vec![],
        encoded_ir: None,
    })
}

fn make_code_named(instructions: Vec<Instruction>, constants: Vec<Value>, names: Vec<String>) -> Arc<CodeObject> {
    Arc::new(CodeObject {
        instructions,
        constants,
        names,
        nlocals: 0,
        varnames: vec![],
        slotmap: Default::default(),
        nargs: 0,
        defaults: vec![],
        name: "test".into(),
        freevars: vec![],
        vararg_idx: -1,
        is_pure: false,
        complexity: 0,
        line_table: vec![],
        patterns: vec![],
        union_checks: vec![],
        composite_checks: vec![],
        generic_checks: vec![],
        encoded_ir: None,
    })
}

// ---- Operand-refcount oracle ----
//
// Creates a string value, keeps a witness `Arc` to it, and hands it to
// `build` (which places it as the constant operand the opcode under test
// consumes). After the program runs and its result is released, the
// witnessed string must be back to exactly two strong references: the code's
// constant slot and the witness. A stack-owned operand an opcode forgot to
// release shows up as a third reference. Strings cover every operand path.
fn assert_operand_released(build: impl FnOnce(Value, Value) -> Arc<CodeObject>, label: &str) {
    let payload = Value::from_str("witness_payload"); // strong=1 (-> constant slot)
    let witness = match payload.to_key().unwrap() {
        ValueKey::Str(a) => a, // strong=2 (constant slot + witness)
        _ => unreachable!(),
    };
    assert_eq!(Arc::strong_count(&witness), 2, "{label}: witness setup");
    let code = build(payload, Value::from_int(0));
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let r = vm.execute(code.clone(), &[], &host).unwrap();
    r.decref();
    assert_eq!(
        Arc::strong_count(&witness),
        2,
        "{label}: opcode leaked its stack-owned operand"
    );
    drop(code);
}

// Control: BuildList consumes operands via `from_list`, so it must not leak.
#[test]
fn oracle_buildlist_releases_operand() {
    assert_operand_released(
        |payload, halt| {
            make_code(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0),
                    Instruction::new(VMOpCode::BuildList, 1),
                    Instruction::simple(VMOpCode::PopTop),
                    Instruction::new(VMOpCode::LoadConst, 1),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![payload, halt],
            )
        },
        "BuildList",
    );
}

#[test]
fn oracle_builddict_releases_value() {
    assert_operand_released(
        |payload, halt| {
            make_code(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0), // key
                    Instruction::new(VMOpCode::LoadConst, 1), // value (payload)
                    Instruction::new(VMOpCode::BuildDict, 1),
                    Instruction::simple(VMOpCode::PopTop),
                    Instruction::new(VMOpCode::LoadConst, 2),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![Value::from_str("k"), payload, halt],
            )
        },
        "BuildDict",
    );
}

#[test]
fn oracle_binop_releases_operand() {
    assert_operand_released(
        |payload, halt| {
            make_code(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0), // payload
                    Instruction::new(VMOpCode::LoadConst, 1), // other
                    Instruction::new(VMOpCode::Add, 0),
                    Instruction::simple(VMOpCode::PopTop),
                    Instruction::new(VMOpCode::LoadConst, 2),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![payload, Value::from_str("x"), halt],
            )
        },
        "Add",
    );
}

// CheckType pops its operand then may bail with a TypeError; the popped,
// stack-owned operand must be released on that error path, not leaked. The
// oracle above assumes success (`unwrap`), so the failing path needs its own.
#[test]
fn oracle_checktype_releases_operand_on_error() {
    use catnip_core::vm::opcode::type_code;
    let payload = Value::from_str("witness_payload"); // strong=1 (-> constant slot)
    let witness = match payload.to_key().unwrap() {
        ValueKey::Str(a) => a, // strong=2 (constant slot + witness)
        _ => unreachable!(),
    };
    assert_eq!(Arc::strong_count(&witness), 2, "checktype: witness setup");
    // CheckType(INT) on a string operand fails (str is not int).
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::CheckType, type_code::INT as u32),
        ],
        vec![payload],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code.clone(), &[], &host);
    assert!(result.is_err(), "CheckType(INT) on a str must raise");
    assert_eq!(
        Arc::strong_count(&witness),
        2,
        "CheckType leaked its stack-owned operand on the type-error path"
    );
    drop(code);
}

#[test]
fn oracle_callmethod_releases_receiver() {
    assert_operand_released(
        |payload, halt| {
            make_code_named(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0),  // payload
                    Instruction::new(VMOpCode::CallMethod, 0), // upper, 0 args
                    Instruction::simple(VMOpCode::PopTop),
                    Instruction::new(VMOpCode::LoadConst, 1),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![payload, halt],
                vec!["upper".into()],
            )
        },
        "CallMethod",
    );
}

#[test]
fn oracle_getiter_releases_iterable() {
    assert_operand_released(
        |payload, halt| {
            // GetIter over the string iterable, then drop the handle.
            make_code(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0), // payload (iterable)
                    Instruction::new(VMOpCode::GetIter, 0),
                    Instruction::simple(VMOpCode::PopTop), // drop the iterator handle
                    Instruction::new(VMOpCode::LoadConst, 1),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![payload, halt],
            )
        },
        "GetIter",
    );
}

#[test]
fn oracle_getiter_drop_releases_items() {
    assert_operand_released(
        |payload, halt| {
            // Build [payload], get an iterator, but never consume it: the
            // iterator retains payload and must release it when dropped at
            // end of execution (early-exit path). Also checks GetIter
            // releases the list operand.
            make_code(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0), // payload
                    Instruction::new(VMOpCode::BuildList, 1), // [payload]
                    Instruction::new(VMOpCode::GetIter, 0),
                    Instruction::simple(VMOpCode::PopTop), // drop the handle, leave iter unconsumed
                    Instruction::new(VMOpCode::LoadConst, 1),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![payload, halt],
            )
        },
        "GetIter+Drop",
    );
}

#[test]
fn oracle_call_releases_args() {
    assert_operand_released(
        |payload, halt| {
            // len(payload)  -> builtin via host.call_function (call_non_vmfunc)
            make_code(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0), // func "len"
                    Instruction::new(VMOpCode::LoadConst, 1), // payload (arg)
                    Instruction::new(VMOpCode::Call, 1),
                    Instruction::simple(VMOpCode::PopTop),
                    Instruction::new(VMOpCode::LoadConst, 2),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![Value::from_str("len"), payload, halt],
            )
        },
        "Call",
    );
}

#[test]
fn oracle_getitem_releases_container() {
    assert_operand_released(
        |payload, halt| {
            // payload[0]  (index a string container; result is a fresh char)
            make_code(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0), // payload (container)
                    Instruction::new(VMOpCode::LoadConst, 1), // index 0
                    Instruction::new(VMOpCode::GetItem, 0),
                    Instruction::simple(VMOpCode::PopTop),
                    Instruction::new(VMOpCode::LoadConst, 2),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![payload, Value::from_int(0), halt],
            )
        },
        "GetItem",
    );
}

#[test]
fn oracle_in_releases_item() {
    assert_operand_released(
        |payload, halt| {
            // payload in ["other"]   (membership scan, result is a bool)
            make_code(
                vec![
                    Instruction::new(VMOpCode::LoadConst, 0), // payload (item)
                    Instruction::new(VMOpCode::LoadConst, 1), // "other"
                    Instruction::new(VMOpCode::BuildList, 1), // container
                    Instruction::new(VMOpCode::In, 0),
                    Instruction::simple(VMOpCode::PopTop),
                    Instruction::new(VMOpCode::LoadConst, 2),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![payload, Value::from_str("other"), halt],
            )
        },
        "In",
    );
}

#[test]
fn oracle_setitem_releases_value() {
    assert_operand_released(
        |payload, _halt| {
            // d = {}; d["k"] = payload; d   (dict returned, then released)
            make_code_with_locals(
                vec![
                    Instruction::new(VMOpCode::BuildDict, 0),
                    Instruction::new(VMOpCode::StoreLocal, 0),
                    Instruction::new(VMOpCode::LoadLocal, 0),
                    Instruction::new(VMOpCode::LoadConst, 0), // key
                    Instruction::new(VMOpCode::LoadConst, 1), // payload
                    Instruction::new(VMOpCode::SetItem, 0),
                    Instruction::new(VMOpCode::LoadLocal, 0),
                    Instruction::simple(VMOpCode::Halt),
                ],
                vec![Value::from_str("k"), payload],
                1,
                vec!["d".into()],
            )
        },
        "SetItem",
    );
}

// Excess positional args on a non-variadic call are consumed but unbound:
// bind_args must release them. Uses the witness pattern directly because the
// payload must travel through a real Call (not a constant operand).
#[test]
fn oracle_call_excess_arg_released() {
    let payload = Value::from_str("excess_witness");
    let witness = match payload.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(Arc::strong_count(&witness), 2, "excess: witness setup");

    // f(x) => x   (nargs = 1)
    let func_code = Arc::new(CodeObject {
        instructions: vec![
            Instruction::new(VMOpCode::LoadLocal, 0),
            Instruction::simple(VMOpCode::Return),
        ],
        constants: vec![],
        names: vec![],
        nlocals: 1,
        varnames: vec!["x".into()],
        slotmap: [("x".into(), 0)].into(),
        nargs: 1,
        defaults: vec![],
        name: "f".into(),
        freevars: vec![],
        vararg_idx: -1,
        is_pure: false,
        complexity: 2,
        line_table: vec![],
        patterns: vec![],
        union_checks: vec![],
        composite_checks: vec![],
        generic_checks: vec![],
        encoded_ir: None,
    });

    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let func_idx = vm.register_function(func_code);

    // f(0, payload): x binds 0, payload is the excess positional arg.
    let main_code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // func
            Instruction::new(VMOpCode::LoadConst, 1), // 0  (bound)
            Instruction::new(VMOpCode::LoadConst, 2), // payload (excess)
            Instruction::new(VMOpCode::Call, 2),
            Instruction::simple(VMOpCode::PopTop),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_vmfunc(func_idx), Value::from_int(0), payload],
    );

    let r = vm.execute(Arc::clone(&main_code), &[], &host).unwrap();
    r.decref();
    assert_eq!(Arc::strong_count(&witness), 2, "excess positional arg leaked");

    drop(vm);
    drop(main_code);
    assert_eq!(Arc::strong_count(&witness), 1, "pool released on code death");
}

// A heap default bound to an unbound param must be incref'd: the slot is
// decref'd at frame teardown, so without the incref the shared default
// constant is over-released (use-after-free under reuse).
#[test]
fn oracle_default_no_overdecref() {
    let payload = Value::from_str("default_witness");
    let witness = match payload.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };
    // strong = 2: the func's defaults slot owns one ref, witness the other.
    assert_eq!(Arc::strong_count(&witness), 2, "default: witness setup");

    // f(x, y = payload) => y   (nargs = 2, one default)
    let func_code = Arc::new(CodeObject {
        instructions: vec![
            Instruction::new(VMOpCode::LoadLocal, 1), // y
            Instruction::simple(VMOpCode::Return),
        ],
        constants: vec![],
        names: vec![],
        nlocals: 2,
        varnames: vec!["x".into(), "y".into()],
        slotmap: [("x".into(), 0), ("y".into(), 1)].into(),
        nargs: 2,
        defaults: vec![payload],
        name: "f".into(),
        freevars: vec![],
        vararg_idx: -1,
        is_pure: false,
        complexity: 2,
        line_table: vec![],
        patterns: vec![],
        union_checks: vec![],
        composite_checks: vec![],
        generic_checks: vec![],
        encoded_ir: None,
    });

    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let func_idx = vm.register_function(func_code);

    // f(0): y falls back to the default, which is returned then dropped.
    let main_code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // func
            Instruction::new(VMOpCode::LoadConst, 1), // x = 0
            Instruction::new(VMOpCode::Call, 1),
            Instruction::simple(VMOpCode::PopTop),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_vmfunc(func_idx), Value::from_int(0)],
    );

    let r = vm.execute(main_code, &[], &host).unwrap();
    r.decref();
    assert_eq!(
        Arc::strong_count(&witness),
        2,
        "default value over- or under-released across a call"
    );
}

/// Register a struct type with all-required (no-default) fields. Returns its
/// type_id. Tests use it to drive the struct-method and construct paths.
fn register_struct(vm: &mut PureVM, name: &str, fields: &[&str]) -> u32 {
    use crate::vm::structs::{PureStructField, PureStructType};
    let field_defs = fields
        .iter()
        .map(|n| PureStructField {
            name: (*n).to_string(),
            has_default: false,
            default_slot: None,
            check: catnip_core::vm::opcode::ParamCheck::None,
        })
        .collect();
    vm.struct_registry.register_type(PureStructType {
        id: 0,
        name: name.to_string(),
        fields: field_defs,
        defaults: vec![],
        methods: indexmap::IndexMap::new(),
        static_methods: indexmap::IndexMap::new(),
        init_fn: None,
        implements: vec![],
        mro: vec![name.to_string()],
        mro_ids: vec![],
        parent_names: vec![],
        abstract_methods: std::collections::HashSet::new(),
    })
}

// An unknown method on a struct receiver returns an error after the args are
// popped (owned). The error path must release them or every argument routed
// through a failing method call leaks for the VM's lifetime.
#[test]
fn oracle_callmethod_unknown_method_releases_args() {
    let payload = Value::from_str("method_witness");
    let witness = match payload.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(Arc::strong_count(&witness), 2, "method: witness setup");

    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let type_id = register_struct(&mut vm, "P", &["x"]);
    let obj = Value::from_struct_instance(StructCell::new(type_id, vec![Value::from_int(7)]));

    // p.nope(payload) -> no such method; payload must be released on the error.
    let main_code = make_code_named(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),  // obj
            Instruction::new(VMOpCode::LoadConst, 1),  // payload (arg)
            Instruction::new(VMOpCode::CallMethod, 1), // name_idx 0 ("nope"), 1 arg
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![obj, payload],
        vec!["nope".into()],
    );

    let r = vm.execute(Arc::clone(&main_code), &[], &host);
    assert!(r.is_err(), "expected unknown-method error");
    assert_eq!(
        Arc::strong_count(&witness),
        2,
        "CallMethod unknown-method leaked its stack-owned arg"
    );

    drop(vm);
    drop(main_code);
    assert_eq!(Arc::strong_count(&witness), 1, "pool released on code death");
}

// CallKw on a non-function pops func/args/kwargs (owned) then errors. The
// kwarg value (a heap operand) must be released on that error path.
#[test]
fn oracle_callkw_non_function_releases_args() {
    let payload = Value::from_str("callkw_witness");
    let witness = match payload.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(Arc::strong_count(&witness), 2, "callkw: witness setup");

    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();

    // (42)(k = payload) -> 42 is not callable. nargs=0, nkwargs=1.
    let kw_names = Value::from_tuple(vec![Value::from_str("k")]);
    let main_code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // func = 42 (non-function)
            Instruction::new(VMOpCode::LoadConst, 1), // payload (kwarg value)
            Instruction::new(VMOpCode::LoadConst, 2), // kw_names tuple ("k",)
            Instruction::new(VMOpCode::CallKw, 1),    // (0 << 8) | 1 kwarg
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(42), payload, kw_names],
    );

    let r = vm.execute(Arc::clone(&main_code), &[], &host);
    assert!(r.is_err(), "expected non-function call error");
    assert_eq!(
        Arc::strong_count(&witness),
        2,
        "CallKw non-function leaked its stack-owned kwarg value"
    );

    drop(vm);
    drop(main_code);
    assert_eq!(Arc::strong_count(&witness), 1, "pool released on code death");
}

// Constructing a struct with too many args fails the arity check inside
// construct_struct before any arg is moved into a field, so the caller still
// owns every arg and must release them on the error.
#[test]
fn oracle_construct_too_many_args_releases_args() {
    let payload = Value::from_str("construct_witness");
    let witness = match payload.to_key().unwrap() {
        ValueKey::Str(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(Arc::strong_count(&witness), 2, "construct: witness setup");

    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let type_id = register_struct(&mut vm, "P", &["x"]); // one field
    let ty_val = Value::from_struct_type(type_id);

    // P(0, payload) -> too many args (one field); payload released on error.
    let main_code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // type P
            Instruction::new(VMOpCode::LoadConst, 1), // 0
            Instruction::new(VMOpCode::LoadConst, 2), // payload (excess)
            Instruction::new(VMOpCode::Call, 2),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![ty_val, Value::from_int(0), payload],
    );

    let r = vm.execute(Arc::clone(&main_code), &[], &host);
    assert!(r.is_err(), "expected too-many-args error");
    assert_eq!(
        Arc::strong_count(&witness),
        2,
        "construct_struct error path leaked a positional arg"
    );

    drop(vm);
    drop(main_code);
    assert_eq!(Arc::strong_count(&witness), 1, "pool released on code death");
}

#[test]
fn test_load_const_halt() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(42)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_add_two_ints() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::new(VMOpCode::Add, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(2), Value::from_int(3)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_arithmetic_expression() {
    // 2 + 3 * 4 = 14
    // Bytecode: LoadConst(3), LoadConst(4), Mul, LoadConst(2), Add (wrong)
    // Actually: LoadConst(2), LoadConst(3), LoadConst(4), Mul, Add
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // 2
            Instruction::new(VMOpCode::LoadConst, 1), // 3
            Instruction::new(VMOpCode::LoadConst, 2), // 4
            Instruction::new(VMOpCode::Mul, 0),       // 3*4=12
            Instruction::new(VMOpCode::Add, 0),       // 2+12=14
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(2), Value::from_int(3), Value::from_int(4)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(14));
}

#[test]
fn test_comparison() {
    // 3 < 5 -> true
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::new(VMOpCode::Lt, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(3), Value::from_int(5)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_jump_if_false() {
    // if false: push 1 else push 2
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),   // false
            Instruction::new(VMOpCode::JumpIfFalse, 4), // jump to 4
            Instruction::new(VMOpCode::LoadConst, 1),   // 1 (skipped)
            Instruction::new(VMOpCode::Jump, 5),        // jump to halt
            Instruction::new(VMOpCode::LoadConst, 2),   // 2 (target)
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::FALSE, Value::from_int(1), Value::from_int(2)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_locals() {
    // x = 10; x + 5
    let code = make_code_with_locals(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),  // 10
            Instruction::new(VMOpCode::StoreLocal, 0), // x = 10
            Instruction::new(VMOpCode::LoadLocal, 0),  // x
            Instruction::new(VMOpCode::LoadConst, 1),  // 5
            Instruction::new(VMOpCode::Add, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(10), Value::from_int(5)],
        1,
        vec!["x".into()],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_not_and_tobool() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // 0
            Instruction::simple(VMOpCode::Not),       // !0 = true
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(0)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_dup_top() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // 42
            Instruction::simple(VMOpCode::DupTop),    // 42, 42
            Instruction::new(VMOpCode::Add, 0),       // 84
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(42)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(84));
}

#[test]
fn test_eq_ne() {
    // 5 == 5 -> true
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::Eq, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(5)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_build_list() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::new(VMOpCode::LoadConst, 2),
            Instruction::new(VMOpCode::BuildList, 3),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert!(result.is_native_list());
    let list = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    assert_eq!(list.get(0).unwrap(), Value::from_int(1));
    assert_eq!(list.get(2).unwrap(), Value::from_int(3));
    result.decref();
}

#[test]
fn test_build_tuple() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::new(VMOpCode::BuildTuple, 2),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(10), Value::from_int(20)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert!(result.is_native_tuple());
    let tuple = unsafe { result.as_native_tuple_ref().unwrap() };
    assert_eq!(tuple.len(), 2);
    result.decref();
}

#[test]
fn test_negation() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::simple(VMOpCode::Neg),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(42)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(-42));
}

#[test]
fn test_bitwise_and() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::new(VMOpCode::BAnd, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(0b1100), Value::from_int(0b1010)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(0b1000));
}

#[test]
fn test_return_from_implicit_end() {
    // Code that just falls off the end returns last stack value
    let code = make_code(
        vec![Instruction::new(VMOpCode::LoadConst, 0)],
        vec![Value::from_int(99)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(99));
}

#[test]
fn test_vm_match_pattern_literal() {
    let reg = PureStructRegistry::new();
    let pat = VMPattern::Literal(Value::from_int(42));
    assert!(
        vm_match_pattern(&pat, Value::from_int(42), &reg, &SymbolTable::new(), &|_| false)
            .unwrap()
            .is_some()
    );
    assert!(
        vm_match_pattern(&pat, Value::from_int(99), &reg, &SymbolTable::new(), &|_| false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_vm_match_pattern_var() {
    let reg = PureStructRegistry::new();
    let pat = VMPattern::Var(0);
    let result = vm_match_pattern(&pat, Value::from_int(42), &reg, &SymbolTable::new(), &|_| false).unwrap();
    assert!(result.is_some());
    let bindings = result.unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0], (0, Value::from_int(42)));
}

#[test]
fn test_vm_match_pattern_wildcard() {
    let reg = PureStructRegistry::new();
    let pat = VMPattern::Wildcard;
    assert!(
        vm_match_pattern(&pat, Value::from_int(42), &reg, &SymbolTable::new(), &|_| false)
            .unwrap()
            .is_some()
    );
    assert!(
        vm_match_pattern(&pat, Value::NIL, &reg, &SymbolTable::new(), &|_| false)
            .unwrap()
            .is_some()
    );
}

#[test]
fn test_vm_match_pattern_or() {
    let reg = PureStructRegistry::new();
    let pat = VMPattern::Or(vec![
        VMPattern::Literal(Value::from_int(1)),
        VMPattern::Literal(Value::from_int(2)),
    ]);
    assert!(
        vm_match_pattern(&pat, Value::from_int(1), &reg, &SymbolTable::new(), &|_| false)
            .unwrap()
            .is_some()
    );
    assert!(
        vm_match_pattern(&pat, Value::from_int(2), &reg, &SymbolTable::new(), &|_| false)
            .unwrap()
            .is_some()
    );
    assert!(
        vm_match_pattern(&pat, Value::from_int(3), &reg, &SymbolTable::new(), &|_| false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_format_value() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),   // 42
            Instruction::new(VMOpCode::FormatValue, 0), // no conv, no spec
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(42)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert!(result.is_native_str());
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("42"));
    result.decref();
}

#[test]
fn test_build_string() {
    let s1 = Value::from_str("hello ");
    let s2 = Value::from_str("world");
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::new(VMOpCode::BuildString, 2),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![s1, s2],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("hello world"));
    // s1/s2 are owned by the constant pool, released by CodeObject::drop.
    result.decref();
}

#[test]
fn test_function_call() {
    // Create a function that returns its argument + 1
    // func(x) => x + 1
    let func_code = Arc::new(CodeObject {
        instructions: vec![
            Instruction::new(VMOpCode::LoadLocal, 0), // x
            Instruction::new(VMOpCode::LoadConst, 0), // 1
            Instruction::new(VMOpCode::Add, 0),
            Instruction::simple(VMOpCode::Return),
        ],
        constants: vec![Value::from_int(1)],
        names: vec![],
        nlocals: 1,
        varnames: vec!["x".into()],
        slotmap: [("x".into(), 0)].into(),
        nargs: 1,
        defaults: vec![],
        name: "inc".into(),
        freevars: vec![],
        vararg_idx: -1,
        is_pure: false,
        complexity: 4,
        line_table: vec![],
        patterns: vec![],
        union_checks: vec![],
        composite_checks: vec![],
        generic_checks: vec![],
        encoded_ir: None,
    });

    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();

    // Register the function
    let func_idx = vm.register_function(func_code);

    // Main code: call func(10)
    let main_code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // func ref
            Instruction::new(VMOpCode::LoadConst, 1), // arg: 10
            Instruction::new(VMOpCode::Call, 1),      // call(1 arg)
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_vmfunc(func_idx), Value::from_int(10)],
    );

    let result = vm.execute(main_code, &[], &host).unwrap();
    assert_eq!(result.as_int(), Some(11));
}

#[test]
fn test_for_range_loop() {
    // sum = 0; for i in range(5): sum = sum + i
    // Bytecode:
    //   0: LoadConst 0 (=0)     -> sum initial
    //   1: StoreLocal 0         -> sum = 0
    //   2: LoadConst 0 (=0)     -> i initial
    //   3: StoreLocal 1         -> i = 0
    //   4: LoadConst 1 (=5)     -> stop
    //   5: StoreLocal 2         -> stop = 5
    //   6: ForRangeInt(slot_i=1, slot_stop=2, step_pos=true, jump=5)
    //   7: LoadLocal 0          -> sum
    //   8: LoadLocal 1          -> i
    //   9: Add
    //  10: StoreLocal 0         -> sum = sum + i
    //  11: ForRangeStep(slot_i=1, step=1, jump_target=6)
    //  12: LoadLocal 0          -> result
    //  13: Halt

    // ForRangeInt arg: (1 << 24) | (2 << 16) | (0 << 15) | 5
    // explicit (0 << 15) documents the zero bit-field of the arg layout
    #[allow(clippy::identity_op)]
    let fri_arg = (1u32 << 24) | (2u32 << 16) | (0u32 << 15) | 5;
    // ForRangeStep arg: (1 << 24) | (1 << 16) | 6
    let frs_arg = (1u32 << 24) | (1u32 << 16) | 6;

    let code = make_code_with_locals(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),          // 0
            Instruction::new(VMOpCode::StoreLocal, 0),         // sum = 0
            Instruction::new(VMOpCode::LoadConst, 0),          // 0
            Instruction::new(VMOpCode::StoreLocal, 1),         // i = 0
            Instruction::new(VMOpCode::LoadConst, 1),          // 5
            Instruction::new(VMOpCode::StoreLocal, 2),         // stop = 5
            Instruction::new(VMOpCode::ForRangeInt, fri_arg),  // [6]
            Instruction::new(VMOpCode::LoadLocal, 0),          // [7] sum
            Instruction::new(VMOpCode::LoadLocal, 1),          // [8] i
            Instruction::new(VMOpCode::Add, 0),                // [9]
            Instruction::new(VMOpCode::StoreLocal, 0),         // [10] sum = sum + i
            Instruction::new(VMOpCode::ForRangeStep, frs_arg), // [11] i++, jump 6
            Instruction::new(VMOpCode::LoadLocal, 0),          // [12] sum
            Instruction::simple(VMOpCode::Halt),               // [13]
        ],
        vec![Value::from_int(0), Value::from_int(5)],
        3,
        vec!["sum".into(), "i".into(), "stop".into()],
    );

    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    // sum = 0+1+2+3+4 = 10
    assert_eq!(result.as_int(), Some(10));
}

#[test]
fn test_typeof() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::simple(VMOpCode::TypeOf),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(42)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("int"));
    result.decref();
}

#[test]
fn test_in_operator() {
    // 2 in [1, 2, 3] -> true
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // 2
            Instruction::new(VMOpCode::LoadConst, 1), // 1
            Instruction::new(VMOpCode::LoadConst, 0), // 2
            Instruction::new(VMOpCode::LoadConst, 2), // 3
            Instruction::new(VMOpCode::BuildList, 3), // [1, 2, 3]
            Instruction::new(VMOpCode::In, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(2), Value::from_int(1), Value::from_int(3)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_is_operator() {
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0), // None
            Instruction::new(VMOpCode::LoadConst, 0), // None
            Instruction::new(VMOpCode::Is, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::NIL],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_div_and_floordiv() {
    // 7 / 2 = 3.5
    let code = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::new(VMOpCode::Div, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(7), Value::from_int(2)],
    );
    let mut vm = PureVM::new();
    let host = PureHost::with_builtins();
    let result = vm.execute(code, &[], &host).unwrap();
    assert!((result.as_float().unwrap() - 3.5).abs() < 1e-10);

    // -7 // 2 = -4
    let code2 = make_code(
        vec![
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::LoadConst, 1),
            Instruction::new(VMOpCode::FloorDiv, 0),
            Instruction::simple(VMOpCode::Halt),
        ],
        vec![Value::from_int(-7), Value::from_int(2)],
    );
    let mut vm2 = PureVM::new();
    let result2 = vm2.execute(code2, &[], &host).unwrap();
    assert_eq!(result2.as_int(), Some(-4));
}

// =================================================================
// End-to-end: PureCompiler -> PureVM
// =================================================================

mod e2e {
    use super::*;
    use crate::compiler::PureCompiler;
    use catnip_core::ir::{IR, IROpCode};

    fn run(ir: IR) -> Value {
        let mut compiler = PureCompiler::new();
        let program = IR::Program(vec![ir]);
        let output = compiler.compile(&program).unwrap();
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        vm.execute_output(&output, &[], &host).unwrap()
    }

    #[test]
    fn test_e2e_arithmetic() {
        // 2 + 3 * 4 = 14
        let expr = IR::op(
            IROpCode::Add,
            vec![IR::Int(2), IR::op(IROpCode::Mul, vec![IR::Int(3), IR::Int(4)])],
        );
        assert_eq!(run(expr).as_int(), Some(14));
    }

    #[test]
    fn test_e2e_comparison() {
        let expr = IR::op(IROpCode::Lt, vec![IR::Int(3), IR::Int(5)]);
        assert_eq!(run(expr).as_bool(), Some(true));
    }

    #[test]
    fn test_e2e_negation() {
        let expr = IR::op(IROpCode::Neg, vec![IR::Int(42)]);
        assert_eq!(run(expr).as_int(), Some(-42));
    }

    #[test]
    fn test_e2e_lambda_call() {
        // ((n) => n * 2)(21)
        let params = IR::List(vec![IR::Identifier("n".into())]);
        let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
        let lambda = IR::op(IROpCode::OpLambda, vec![params, body]);
        let call = IR::op(IROpCode::Call, vec![lambda, IR::Int(21)]);
        assert_eq!(run(call).as_int(), Some(42));
    }

    #[test]
    fn test_e2e_fn_def_and_call() {
        // Use PurePipeline for proper semantic analysis
        let mut p = crate::pipeline::PurePipeline::new().unwrap();
        let result = p.execute("double = (n) => { n * 2 }; double(21)").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_e2e_list_operations() {
        let expr = IR::op(IROpCode::ListLiteral, vec![IR::Int(1), IR::Int(2), IR::Int(3)]);
        let result = run(expr);
        assert!(result.is_native_list());
        let list = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        result.decref();
    }

    #[test]
    fn test_e2e_string_literal() {
        let result = run(IR::String("hello".into()));
        assert!(result.is_native_str());
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("hello"));
        result.decref();
    }

    #[test]
    fn test_e2e_bool_not() {
        let expr = IR::op(IROpCode::Not, vec![IR::Bool(false)]);
        assert_eq!(run(expr).as_bool(), Some(true));
    }

    // TH2-B step 0b: boundary check + numeric-tower coercion (CheckType dispatch).

    #[test]
    fn test_boundary_coerce_numeric_tower() {
        use catnip_core::vm::opcode::type_code;
        // Exact types pass through.
        assert_eq!(
            boundary_coerce(Value::from_int(5), type_code::INT).unwrap().as_int(),
            Some(5)
        );
        assert_eq!(
            boundary_coerce(Value::from_float(2.5), type_code::FLOAT)
                .unwrap()
                .as_float(),
            Some(2.5)
        );
        assert_eq!(
            boundary_coerce(Value::from_bool(true), type_code::BOOL)
                .unwrap()
                .as_bool(),
            Some(true)
        );
        // Widening is coerced to the declared type.
        assert_eq!(
            boundary_coerce(Value::from_int(5), type_code::FLOAT)
                .unwrap()
                .as_float(),
            Some(5.0)
        );
        assert_eq!(
            boundary_coerce(Value::from_bool(true), type_code::INT)
                .unwrap()
                .as_int(),
            Some(1)
        );
        assert_eq!(
            boundary_coerce(Value::from_bool(true), type_code::FLOAT)
                .unwrap()
                .as_float(),
            Some(1.0)
        );
    }

    #[test]
    fn test_boundary_coerce_rejects_and_none() {
        use catnip_core::vm::opcode::type_code;
        // Narrowing (float -> int) and disjoint types are refused.
        assert!(boundary_coerce(Value::from_float(2.5), type_code::INT).is_err());
        assert!(boundary_coerce(Value::from_string("x".into()), type_code::INT).is_err());
        assert!(boundary_coerce(Value::from_int(0), type_code::BOOL).is_err());
        // None is an identity check.
        assert!(boundary_coerce(Value::NIL, type_code::NONE).is_ok());
        assert!(boundary_coerce(Value::from_int(0), type_code::NONE).is_err());
    }

    #[test]
    fn test_boundary_coerce_str_and_bigint() {
        use catnip_core::vm::opcode::type_code;
        // str passes the str boundary; a non-str is refused (no widening).
        assert!(boundary_coerce(Value::from_string("hi".into()), type_code::STR).is_ok());
        assert!(boundary_coerce(Value::from_int(5), type_code::STR).is_err());
        // bigint widens to float, like int.
        let big = Value::from_bigint(rug::Integer::from(42));
        assert_eq!(boundary_coerce(big, type_code::FLOAT).unwrap().as_float(), Some(42.0));
        // bigint too large for f64 is refused, not coerced to inf, and the
        // failure is a boundary TypeError (same class as other boundary errors).
        let huge = Value::from_bigint(rug::Integer::from_str_radix(&format!("1{}", "0".repeat(400)), 10).unwrap());
        assert!(matches!(
            boundary_coerce(huge, type_code::FLOAT),
            Err(VMError::TypeError(_))
        ));
        huge.decref_bigint(); // boundary_coerce does not consume val on the Err path
    }

    #[test]
    fn test_boundary_coerce_bigint_to_float_no_leak() {
        use catnip_core::vm::opcode::type_code;
        // The source bigint is replaced by a fresh float; its Arc must be
        // released, not leaked. Keep an extra ref to observe the count.
        let big = Value::from_bigint(rug::Integer::from(42));
        big.clone_refcount(); // strong = 2 (the keeper below + the one passed in)
        assert_eq!(big.bigint_strong_count(), 2);
        let coerced = boundary_coerce(big, type_code::FLOAT).unwrap();
        assert_eq!(coerced.as_float(), Some(42.0));
        assert_eq!(
            big.bigint_strong_count(),
            1,
            "bigint→float boundary must release the source Arc"
        );
        big.decref_bigint(); // release the keeper
    }

    // TH4 canal A: typed arithmetic opcodes (AddInt/AddFloat). Dispatch only --
    // these are inert until the compiler emits them from a proven `ty`.

    fn run_binop(op: VMOpCode, a: Value, b: Value) -> Value {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::simple(op),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![a, b],
        );
        let host = PureHost::with_builtins();
        PureVM::new().execute(code, &[], &host).unwrap()
    }

    #[test]
    fn test_add_int_dispatch() {
        assert_eq!(
            run_binop(VMOpCode::AddInt, Value::from_int(2), Value::from_int(3)).as_int(),
            Some(5)
        );
        // Overflow keeps Add's semantics: a sum past the 47-bit smallint range
        // promotes to bigint (both operands are valid smallints).
        let big = 70_000_000_000_000; // < 2^46, so a valid smallint; 2*big is not
        let r = run_binop(VMOpCode::AddInt, Value::from_int(big), Value::from_int(big));
        assert!(r.is_bigint(), "sum past smallint range must promote to bigint");
    }

    #[test]
    fn test_add_float_dispatch() {
        assert_eq!(
            run_binop(VMOpCode::AddFloat, Value::from_float(1.5), Value::from_float(2.5)).as_float(),
            Some(4.0)
        );
    }

    #[test]
    fn test_typed_sub_mul_dispatch() {
        assert_eq!(
            run_binop(VMOpCode::SubInt, Value::from_int(10), Value::from_int(3)).as_int(),
            Some(7)
        );
        assert_eq!(
            run_binop(VMOpCode::SubFloat, Value::from_float(10.0), Value::from_float(3.0)).as_float(),
            Some(7.0)
        );
        assert_eq!(
            run_binop(VMOpCode::MulInt, Value::from_int(6), Value::from_int(7)).as_int(),
            Some(42)
        );
        assert_eq!(
            run_binop(VMOpCode::MulFloat, Value::from_float(1.5), Value::from_float(4.0)).as_float(),
            Some(6.0)
        );
    }

    #[test]
    fn test_div_float_dispatch() {
        assert_eq!(
            run_binop(VMOpCode::DivFloat, Value::from_float(7.0), Value::from_float(2.0)).as_float(),
            Some(3.5)
        );
        // Division by zero defers to numeric_div, which raises.
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::simple(VMOpCode::DivFloat),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_float(1.0), Value::from_float(0.0)],
        );
        let host = PureHost::with_builtins();
        assert!(PureVM::new().execute(code, &[], &host).is_err(), "x/0.0 must raise");
    }

    // Runtime enforcement of struct field type annotations (boundary at the
    // constructor). A statically-provable mismatch is E300 at compile time; these
    // route a dynamically-typed value (an unannotated function's return) into a
    // typed field so the runtime check fires.
    mod field_types {
        fn run(src: &str) -> crate::error::VMResult<crate::value::Value> {
            crate::pipeline::PurePipeline::new().unwrap().execute(src)
        }

        #[test]
        fn primitive_dynamic_mismatch_rejected() {
            let e = run("struct P { x: int }\nget = () => { \"h\" }\nP(get())").unwrap_err();
            let msg = format!("{}", e);
            assert!(msg.contains("field 'x' of 'P'"), "msg was: {msg}");
            assert!(msg.contains("expects 'int'"), "msg was: {msg}");
        }

        #[test]
        fn primitive_coerced_numeric_tower() {
            // int into a float field is coerced (parity with typed params).
            let r = run("struct P { x: float }\nget = () => { 3 }\nP(get()).x").unwrap();
            assert_eq!(r.as_float(), Some(3.0));
        }

        #[test]
        fn exact_type_passes() {
            let r = run("struct P { x: int }\nget = () => { 5 }\nP(get()).x").unwrap();
            assert_eq!(r.as_int(), Some(5));
        }

        #[test]
        fn unannotated_field_unchanged() {
            let r = run("struct P { x }\nget = () => { \"h\" }\nP(get()).x").unwrap();
            assert!(r.is_native_str());
            r.decref();
        }

        #[test]
        fn union_variant_payload_field_rejected() {
            // A concrete payload field is enforced at variant construction,
            // exactly like a struct field.
            let e = run("union U { A(x: int) }\nget = () => { 1.5 }\nU.A(get())").unwrap_err();
            let msg = format!("{}", e);
            assert!(msg.contains("field 'x' of 'U.A'"), "msg was: {msg}");
            assert!(msg.contains("expects 'int'"), "msg was: {msg}");
        }

        #[test]
        fn union_variant_payload_field_coerced() {
            // int into a float payload field is coerced (parity with structs).
            let r = run("union U { A(x: float) }\nget = () => { 3 }\nU.A(get()).x").unwrap();
            assert_eq!(r.as_float(), Some(3.0));
        }

        #[test]
        fn union_variant_payload_field_accepted() {
            let r = run("union U { A(x: int) }\nget = () => { 5 }\nU.A(get()).x").unwrap();
            assert_eq!(r.as_int(), Some(5));
        }

        #[test]
        fn union_generic_payload_field_not_checked() {
            // A type-parameter field (`Some(v: T)`) is not fixed at construction:
            // any argument is accepted (enforced at the use-site boundary instead).
            let r = run("union Opt[T] { Some(v: T)\n None }\nget = () => { 1.5 }\nOpt.Some(get()).v").unwrap();
            assert_eq!(r.as_float(), Some(1.5));
        }

        #[test]
        fn nominal_field_rejected() {
            let e = run("struct Pt { a }\nstruct Box { p: Pt }\nget = () => { 5 }\nBox(get())").unwrap_err();
            assert!(format!("{}", e).contains("field 'p' of 'Box'"));
        }

        #[test]
        fn nominal_field_accepted() {
            let r = run("struct Pt { a }\nstruct Box { p: Pt }\nget = () => { Pt(1) }\nBox(get()).p.a").unwrap();
            assert_eq!(r.as_int(), Some(1));
        }

        #[test]
        fn nominal_subtype_accepted() {
            let prog = "struct Base { a }\nstruct Child extends(Base) { b }\nstruct Box { p: Base }\nget = () => { Child(1, 2) }\nBox(get()).p.a";
            assert_eq!(run(prog).unwrap().as_int(), Some(1));
        }

        #[test]
        fn union_field_rejected() {
            let e = run("struct P { x: int | str }\nget = () => { 1.5 }\nP(get())").unwrap_err();
            assert!(format!("{}", e).contains("field 'x' of 'P'"));
        }

        #[test]
        fn union_field_accepted() {
            let r = run("struct P { x: int | str }\nget = () => { \"ok\" }\nP(get()).x").unwrap();
            assert!(r.is_native_str());
            r.decref();
        }

        #[test]
        fn composite_field_rejected() {
            let e = run("struct P { xs: list[int] }\nget = () => { [\"a\"] }\nP(get())").unwrap_err();
            assert!(format!("{}", e).contains("field 'xs' of 'P'"));
        }

        #[test]
        fn composite_field_accepted() {
            let r = run("struct P { xs: list[int] }\nget = () => { [1, 2] }\nlen(P(get()).xs)").unwrap();
            assert_eq!(r.as_int(), Some(2));
        }

        #[test]
        fn set_field_rejected() {
            let e = run("struct P { xs: set[int] }\nget = () => { set(\"a\") }\nP(get())").unwrap_err();
            assert!(format!("{}", e).contains("field 'xs' of 'P'"));
        }

        #[test]
        fn set_field_accepted() {
            let r = run("struct P { xs: set[int] }\nget = () => { set(1, 2) }\nlen(P(get()).xs)").unwrap();
            assert_eq!(r.as_int(), Some(2));
        }

        #[test]
        fn set_param_rejects_list_container() {
            // Distinct container: a dynamically-typed list does not satisfy `set`.
            let e = run("mk = (v) => { v }\nf = (xs: set) => { xs }\nf(mk([1, 2, 3]))").unwrap_err();
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn set_param_rejects_bad_element() {
            // Exercises the SET arm of check_composite: a dynamically-typed set
            // with a wrong element is rejected at the boundary.
            let e = run("mk = (v) => { v }\nf = (xs: set[int]) => { xs }\nf(mk(set(\"a\")))").unwrap_err();
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn set_param_accepts_good_dynamic() {
            // The SET arm iterates elements via keys_cloned + key_satisfies; a valid
            // dynamically-typed set passes and stays intact (len == 3).
            let r = run("mk = (v) => { v }\nf = (xs: set[int]) => { len(xs) }\nf(mk(set(1, 2, 3)))").unwrap();
            assert_eq!(r.as_int(), Some(3));
        }

        #[test]
        fn set_of_structs_element_check() {
            // A set of struct instances in a `set[Nominal]` slot: the element check
            // resolves the struct's type from its key snapshot without materializing
            // it (`to_value` cannot rebuild a struct key). Parity with `list[Point]`.
            let r = run("struct Point { x: int }\nf = (xs: set[Point]) => { len(xs) }\nf(set(Point(1), Point(2)))")
                .unwrap();
            assert_eq!(r.as_int(), Some(2));
            // A wrong element type is a clean TypeError, not a panic.
            let e = run("struct Point { x: int }\nf = (xs: set[Point]) => { xs }\nmk = (v) => { v }\nf(mk(set(1, 2)))")
                .unwrap_err();
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn dict_of_struct_keys_element_check() {
            // Pre-existing twin of the set case: a struct key in a `dict[Nominal, V]`
            // slot must resolve via its type_id, not panic in `to_value`.
            let r = run("struct Point { x: int }\nf = (d: dict[Point, int]) => { len(d) }\nf({Point(1): 9})").unwrap();
            assert_eq!(r.as_int(), Some(1));
        }

        #[test]
        fn tuple_field_rejected() {
            // A positional element of the wrong type is rejected at the constructor.
            let e = run("struct P { t: tuple[int, str] }\nget = () => { tuple(1, 2) }\nP(get())").unwrap_err();
            assert!(format!("{}", e).contains("field 't' of 'P'"));
        }

        #[test]
        fn tuple_field_accepted() {
            let r = run("struct P { t: tuple[int, str] }\nget = () => { tuple(1, \"a\") }\nlen(P(get()).t)").unwrap();
            assert_eq!(r.as_int(), Some(2));
        }

        #[test]
        fn tuple_param_rejects_list_container() {
            // Distinct container: a dynamically-typed list does not satisfy `tuple`.
            let e = run("mk = (v) => { v }\nf = (t: tuple) => { t }\nf(mk([1, 2, 3]))").unwrap_err();
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn tuple_param_rejects_wrong_arity() {
            // Arity is part of the contract: a 3-tuple does not satisfy tuple[int, str].
            let e = run("mk = (v) => { v }\nf = (t: tuple[int, str]) => { t }\nf(mk(tuple(1, \"a\", 3)))").unwrap_err();
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn tuple_param_rejects_bad_element() {
            // Position 1 must be a str: a dynamically-typed tuple with an int there
            // is rejected at the boundary (the positional element pass).
            let e = run("mk = (v) => { v }\nf = (t: tuple[int, str]) => { t }\nf(mk(tuple(1, 2)))").unwrap_err();
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn tuple_param_accepts_good_dynamic() {
            // A valid dynamically-typed tuple passes and stays intact (len == 2).
            let r = run("mk = (v) => { v }\nf = (t: tuple[int, str]) => { len(t) }\nf(mk(tuple(1, \"a\")))").unwrap();
            assert_eq!(r.as_int(), Some(2));
        }

        #[test]
        fn tuple_of_structs_element_check() {
            // Positional struct element: position 0 resolves the struct's type from
            // its value, accepting a match and rejecting a wrong type cleanly.
            let r = run("struct Point { x: int }\nf = (t: tuple[Point, int]) => { len(t) }\nf(tuple(Point(1), 9))")
                .unwrap();
            assert_eq!(r.as_int(), Some(2));
            let e = run(
                "struct Point { x: int }\nmk = (v) => { v }\nf = (t: tuple[Point, int]) => { t }\nf(mk(tuple(1, 9)))",
            )
            .unwrap_err();
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn inherited_field_enforced() {
            // Field `x: int` declared on the base must still be enforced on the
            // subtype's constructor (the check travels with the cloned field).
            let e =
                run("struct B { x: int }\nstruct C extends(B) { y }\nget = () => { \"h\" }\nC(get(), 1)").unwrap_err();
            assert!(format!("{}", e).contains("field 'x' of 'C'"));
        }

        #[test]
        fn bigint_overflow_into_float_field_rejected() {
            let e = run("struct P { x: float }\nget = () => { 2 ** 10000 }\nP(get())").unwrap_err();
            // Surfaces as a TypeError, like every other boundary failure.
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn coercion_before_later_field_failure_is_safe() {
            // x: float coerces a bigint (the one refcount-mutating coercion); y: int
            // then fails. The two-pass design validates every field before any
            // coercion, so this raises cleanly -- a crash here would mean the bigint
            // was decref'd then double-freed on the error path.
            let e =
                run("struct P { x: float; y: int }\ngb = () => { 2 ** 100 }\ngs = () => { \"bad\" }\nP(gb(), gs())")
                    .unwrap_err();
            assert!(matches!(e, crate::error::VMError::TypeError(_)), "got: {e:?}");
        }

        #[test]
        fn default_value_field_ok() {
            // A typed field with a default still constructs when the arg is omitted.
            let r = run("struct P { x: int = 7 }\nP().x").unwrap();
            assert_eq!(r.as_int(), Some(7));
        }
    }
}
