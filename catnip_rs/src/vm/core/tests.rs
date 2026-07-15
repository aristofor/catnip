//! Unit tests for the VM dispatch loop and native struct operations.

use super::*;
use crate::vm::opcode::Instruction;

#[test]
fn test_simple_add() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test");
        code.constants = vec![Value::from_int(2), Value::from_int(3)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::Add),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(5));
    });
}

#[test]
fn test_comparison() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test");
        code.constants = vec![Value::from_int(5), Value::from_int(3)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::Gt),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    });
}

#[test]
fn test_jump() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test");
        code.constants = vec![Value::from_int(10), Value::from_int(20)];
        code.instructions = vec![
            Instruction::new(OpCode::Jump, 2),      // Jump to LoadConst 20
            Instruction::new(OpCode::LoadConst, 0), // Skip this
            Instruction::new(OpCode::LoadConst, 1), // Load 20
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(20));
    });
}

#[test]
fn test_arithmetic_sub() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_sub");
        code.constants = vec![Value::from_int(10), Value::from_int(3)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0), // 10
            Instruction::new(OpCode::LoadConst, 1), // 3
            Instruction::simple(OpCode::Sub),       // 10 - 3
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(7));
    });
}

#[test]
fn test_arithmetic_mul() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_mul");
        code.constants = vec![Value::from_int(4), Value::from_int(5)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::Mul),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(20));
    });
}

#[test]
fn test_arithmetic_floordiv() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_floordiv");
        code.constants = vec![Value::from_int(23), Value::from_int(5)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::FloorDiv),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(4));
    });
}

#[test]
fn test_arithmetic_mod() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_mod");
        code.constants = vec![Value::from_int(23), Value::from_int(5)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::Mod),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(3));
    });
}

#[test]
fn test_stack_dup_top() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_dup");
        code.constants = vec![Value::from_int(42)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0), // Push 42
            Instruction::simple(OpCode::DupTop),    // Duplicate
            Instruction::simple(OpCode::Add),       // 42 + 42
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(84));
    });
}

#[test]
fn test_stack_pop_top() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_pop");
        code.constants = vec![Value::from_int(10), Value::from_int(20)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0), // Push 10
            Instruction::new(OpCode::LoadConst, 1), // Push 20
            Instruction::simple(OpCode::PopTop),    // Pop 20
            Instruction::simple(OpCode::Halt),      // Return 10
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(10));
    });
}

#[test]
fn test_locals_store_load() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_locals");
        code.nlocals = 2; // 2 local slots
        code.constants = vec![Value::from_int(100), Value::from_int(200)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),  // 100
            Instruction::new(OpCode::StoreLocal, 0), // local[0] = 100
            Instruction::new(OpCode::LoadConst, 1),  // 200
            Instruction::new(OpCode::StoreLocal, 1), // local[1] = 200
            Instruction::new(OpCode::LoadLocal, 0),  // Load local[0]
            Instruction::new(OpCode::LoadLocal, 1),  // Load local[1]
            Instruction::simple(OpCode::Add),        // 100 + 200
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(300));
    });
}

#[test]
fn test_conditional_jump_taken() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_jump");
        code.constants = vec![
            Value::from_int(5),
            Value::from_int(3),
            Value::from_int(100), // Value if jump taken
            Value::from_int(999), // Value if jump not taken
        ];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),   // 5
            Instruction::new(OpCode::LoadConst, 1),   // 3
            Instruction::simple(OpCode::Gt),          // 5 > 3 = True
            Instruction::new(OpCode::JumpIfFalse, 6), // Skip if False
            Instruction::new(OpCode::LoadConst, 2),   // 100 (taken)
            Instruction::new(OpCode::Jump, 7),        // Skip else
            Instruction::new(OpCode::LoadConst, 3),   // 999 (not taken)
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(100)); // Jump was taken
    });
}

#[test]
fn test_conditional_jump_not_taken() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_jump");
        code.constants = vec![
            Value::from_int(3),
            Value::from_int(5),
            Value::from_int(100),
            Value::from_int(999),
        ];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),   // 3
            Instruction::new(OpCode::LoadConst, 1),   // 5
            Instruction::simple(OpCode::Gt),          // 3 > 5 = False
            Instruction::new(OpCode::JumpIfFalse, 6), // Jump to else
            Instruction::new(OpCode::LoadConst, 2),   // 100 (not taken)
            Instruction::new(OpCode::Jump, 7),
            Instruction::new(OpCode::LoadConst, 3), // 999 (taken)
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(999)); // Else branch taken
    });
}

#[test]
fn test_bitwise_and() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_band");
        code.constants = vec![Value::from_int(12), Value::from_int(10)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::BAnd), // 12 & 10 = 8
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(8));
    });
}

#[test]
fn test_bitwise_or() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_bor");
        code.constants = vec![Value::from_int(12), Value::from_int(10)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::BOr), // 12 | 10 = 14
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(14));
    });
}

#[test]
fn test_bitwise_xor() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_bxor");
        code.constants = vec![Value::from_int(12), Value::from_int(10)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::BXor), // 12 ^ 10 = 6
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(6));
    });
}

#[test]
fn test_unary_neg() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_neg");
        code.constants = vec![Value::from_int(42)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::simple(OpCode::Neg),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(-42));
    });
}

#[test]
fn test_loop_simple() {
    // Simple counting loop: sum = 0; for i in 0..5 { sum += i }
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_loop");
        code.nlocals = 2; // i, sum
        code.constants = vec![
            Value::from_int(0), // Initial i
            Value::from_int(5), // Limit
            Value::from_int(1), // Increment
        ];
        code.instructions = vec![
            // sum = 0
            Instruction::new(OpCode::LoadConst, 0),  // 0
            Instruction::new(OpCode::StoreLocal, 1), // 1
            // i = 0
            Instruction::new(OpCode::LoadConst, 0),  // 2
            Instruction::new(OpCode::StoreLocal, 0), // 3
            // loop start (ip=4)
            Instruction::new(OpCode::LoadLocal, 0),    // 4: i
            Instruction::new(OpCode::LoadConst, 1),    // 5: limit
            Instruction::simple(OpCode::Lt),           // 6: i < 5
            Instruction::new(OpCode::JumpIfFalse, 17), // 7: Exit to ip=17 if false
            // sum = sum + i
            Instruction::new(OpCode::LoadLocal, 1),  // 8: sum
            Instruction::new(OpCode::LoadLocal, 0),  // 9: i
            Instruction::simple(OpCode::Add),        // 10: sum + i
            Instruction::new(OpCode::StoreLocal, 1), // 11: Store sum
            // i = i + 1
            Instruction::new(OpCode::LoadLocal, 0),  // 12: i
            Instruction::new(OpCode::LoadConst, 2),  // 13: 1
            Instruction::simple(OpCode::Add),        // 14: i + 1
            Instruction::new(OpCode::StoreLocal, 0), // 15: Store i
            Instruction::new(OpCode::Jump, 4),       // 16: Loop back to ip=4
            // exit (ip=17)
            Instruction::new(OpCode::LoadLocal, 1), // 17: Return sum
            Instruction::simple(OpCode::Halt),      // 18: Halt
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(10)); // 0+1+2+3+4
    });
}

#[test]
fn test_float_arithmetic() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_float");
        code.constants = vec![Value::from_float(1.5), Value::from_float(2.5)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::Add), // 1.5 + 2.5
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_float(), Some(4.0));
    });
}

#[test]
fn test_comparison_chains() {
    // Test multiple comparisons
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_cmp");
        code.constants = vec![Value::from_int(5), Value::from_int(5)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::Eq), // 5 == 5
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    });
}

#[test]
fn test_nested_arithmetic() {
    // (2 + 3) * 4 = 20
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut code = CodeObject::new("test_nested");
        code.constants = vec![Value::from_int(2), Value::from_int(3), Value::from_int(4)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0), // 2
            Instruction::new(OpCode::LoadConst, 1), // 3
            Instruction::simple(OpCode::Add),       // 5
            Instruction::new(OpCode::LoadConst, 2), // 4
            Instruction::simple(OpCode::Mul),       // 20
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(20));
    });
}

// --- Native struct tests ---

/// Helper: register a struct type and map a Python object's pointer to it.
/// Returns (py_obj_value, type_id) where py_obj_value can be used as the callable.
/// Install struct registry thread-local for a test VM.
fn install_test_tables(vm: &mut VM) {
    crate::vm::value::set_struct_registry(&vm.struct_registry as *const _);
    crate::vm::value::set_func_table(&vm.func_table as *const _);
}

fn register_test_struct(py: Python<'_>, vm: &mut VM, name: &str, fields: Vec<StructField>) -> (Value, StructTypeId) {
    install_test_tables(vm);
    let type_id = vm.struct_registry.register_type(
        name.into(),
        fields,
        IndexMap::new(),
        vec![],                 // implements
        vec![name.to_string()], // mro
    );
    // Use a simple Python object as stand-in for the dataclass
    let marker = py.eval(c"type('_Marker', (), {})", None, None).unwrap();
    let ptr = marker.as_ptr() as usize;
    vm.struct_type_map.insert(ptr, type_id);
    let val = Value::from_pyobject(py, &marker).unwrap();
    (val, type_id)
}

#[test]
fn test_native_struct_call() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let (struct_val, type_id) = register_test_struct(
            py,
            &mut vm,
            "Point",
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        // Bytecode: load struct type, load args, call
        let mut code = CodeObject::new("test_struct_call");
        code.constants = vec![struct_val, Value::from_int(10), Value::from_int(20)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0), // struct type
            Instruction::new(OpCode::LoadConst, 1), // x=10
            Instruction::new(OpCode::LoadConst, 2), // y=20
            Instruction::new(OpCode::Call, 2),      // Point(10, 20)
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert!(result.is_struct_instance());
        let idx = result.as_struct_instance_idx().unwrap();
        let (inst_type_id, fields) = vm
            .struct_registry
            .with_instance(idx, |i| (i.type_id, i.fields.clone()))
            .unwrap();
        assert_eq!(inst_type_id, type_id);
        assert_eq!(fields[0].as_int(), Some(10));
        assert_eq!(fields[1].as_int(), Some(20));
    });
}

#[test]
fn test_native_struct_call_with_defaults() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let (struct_val, _type_id) = register_test_struct(
            py,
            &mut vm,
            "Config",
            vec![
                StructField {
                    name: "name".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "debug".into(),
                    has_default: true,
                    default: Value::FALSE,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "level".into(),
                    has_default: true,
                    default: Value::from_int(1),
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        // Call with only required arg: Config("test")
        let mut code = CodeObject::new("test_struct_defaults");
        code.constants = vec![
            struct_val,
            Value::from_pyobject(py, &"test".into_pyobject(py).unwrap()).unwrap(),
        ];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0), // struct type
            Instruction::new(OpCode::LoadConst, 1), // name="test"
            Instruction::new(OpCode::Call, 1),      // Config("test")
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert!(result.is_struct_instance());
        let idx = result.as_struct_instance_idx().unwrap();
        let fields = vm.struct_registry.with_instance(idx, |i| i.fields.clone()).unwrap();
        assert_eq!(fields[1].as_bool(), Some(false)); // debug default
        assert_eq!(fields[2].as_int(), Some(1)); // level default
    });
}

#[test]
fn test_native_struct_call_too_few_args() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let (struct_val, _) = register_test_struct(
            py,
            &mut vm,
            "Point",
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        let mut code = CodeObject::new("test_too_few");
        code.constants = vec![struct_val, Value::from_int(10)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::new(OpCode::Call, 1), // Only 1 arg for 2 required
            Instruction::simple(OpCode::Halt),
        ];

        let err = vm.execute(py, Arc::new(code), &[]).unwrap_err();
        match err {
            VMError::TypeError(msg) => assert!(msg.contains("missing"), "got: {msg}"),
            other => panic!("expected TypeError, got {other:?}"),
        }
    });
}

#[test]
fn test_native_struct_call_too_many_args() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let (struct_val, _) = register_test_struct(
            py,
            &mut vm,
            "Pair",
            vec![
                StructField {
                    name: "a".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "b".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        let mut code = CodeObject::new("test_too_many");
        code.constants = vec![struct_val, Value::from_int(1), Value::from_int(2), Value::from_int(3)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::new(OpCode::LoadConst, 2),
            Instruction::new(OpCode::LoadConst, 3),
            Instruction::new(OpCode::Call, 3), // 3 args for 2 fields
            Instruction::simple(OpCode::Halt),
        ];

        let err = vm.execute(py, Arc::new(code), &[]).unwrap_err();
        match err {
            VMError::TypeError(msg) => assert!(msg.contains("takes"), "got: {msg}"),
            other => panic!("expected TypeError, got {other:?}"),
        }
    });
}

#[test]
fn test_native_struct_callkw() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let (struct_val, type_id) = register_test_struct(
            py,
            &mut vm,
            "Point",
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        // CallKw encoding: (nargs << 8) | nkw
        // Point(10, y=20) -> nargs=1, nkw=1
        let kw_names = PyTuple::new(py, ["y"]).unwrap();
        let kw_names_val = Value::from_pyobject(py, kw_names.as_any()).unwrap();

        let mut code = CodeObject::new("test_struct_callkw");
        code.constants = vec![struct_val, Value::from_int(10), Value::from_int(20), kw_names_val];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),         // struct type
            Instruction::new(OpCode::LoadConst, 1),         // x=10 (positional)
            Instruction::new(OpCode::LoadConst, 2),         // y=20 (kw value)
            Instruction::new(OpCode::LoadConst, 3),         // kw_names ("y",)
            Instruction::new(OpCode::CallKw, (1 << 8) | 1), // nargs=1, nkw=1
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert!(result.is_struct_instance());
        let idx = result.as_struct_instance_idx().unwrap();
        let (inst_type_id, fields) = vm
            .struct_registry
            .with_instance(idx, |i| (i.type_id, i.fields.clone()))
            .unwrap();
        assert_eq!(inst_type_id, type_id);
        assert_eq!(fields[0].as_int(), Some(10));
        assert_eq!(fields[1].as_int(), Some(20));
    });
}

/// Helper: create a Point(x, y) instance and return (vm, instance_value).
fn make_point_instance(py: Python<'_>, x: i64, y: i64) -> (VM, Value) {
    let mut vm = VM::new();

    let (struct_val, _) = register_test_struct(
        py,
        &mut vm,
        "Point",
        vec![
            StructField {
                name: "x".into(),
                has_default: false,
                default: Value::NIL,
                check: catnip_core::vm::opcode::ParamCheck::None,
            },
            StructField {
                name: "y".into(),
                has_default: false,
                default: Value::NIL,
                check: catnip_core::vm::opcode::ParamCheck::None,
            },
        ],
    );

    let mut code = CodeObject::new("make_point");
    code.constants = vec![struct_val, Value::from_int(x), Value::from_int(y)];
    code.instructions = vec![
        Instruction::new(OpCode::LoadConst, 0),
        Instruction::new(OpCode::LoadConst, 1),
        Instruction::new(OpCode::LoadConst, 2),
        Instruction::new(OpCode::Call, 2),
        Instruction::simple(OpCode::Halt),
    ];

    let result = vm.execute(py, Arc::new(code), &[]).unwrap();
    (vm, result)
}

#[test]
fn test_native_struct_getattr() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let (mut vm, instance) = make_point_instance(py, 10, 20);

        // GetAttr for "x" (names[0]) then "y" (names[1])
        let mut code = CodeObject::new("test_getattr");
        code.constants = vec![instance];
        code.names = vec!["x".into(), "y".into()];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::GetAttr, 0), // .x
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(10));

        let mut code2 = CodeObject::new("test_getattr_y");
        code2.constants = vec![instance];
        code2.names = vec!["y".into()];
        code2.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::GetAttr, 0), // .y
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code2), &[]).unwrap();
        assert_eq!(result.as_int(), Some(20));
    });
}

#[test]
fn test_native_struct_getattr_unknown_field() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let (mut vm, instance) = make_point_instance(py, 10, 20);

        let mut code = CodeObject::new("test_getattr_bad");
        code.constants = vec![instance];
        code.names = vec!["z".into()];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::GetAttr, 0), // .z (doesn't exist)
            Instruction::simple(OpCode::Halt),
        ];

        let err = vm.execute(py, Arc::new(code), &[]).unwrap_err();
        match err {
            VMError::RuntimeError(msg) => {
                assert!(msg.contains("no attribute"), "got: {msg}");
                assert!(msg.contains("'z'"), "got: {msg}");
            }
            other => panic!("expected RuntimeError, got {other:?}"),
        }
    });
}

#[test]
fn test_native_struct_setattr() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let (mut vm, instance) = make_point_instance(py, 10, 20);

        // SetAttr: set x = 42, then GetAttr to verify
        let mut code = CodeObject::new("test_setattr");
        code.constants = vec![instance, Value::from_int(42)];
        code.names = vec!["x".into()];
        code.instructions = vec![
            // SetAttr: pop value, pop obj -> obj.x = 42
            Instruction::new(OpCode::LoadConst, 0), // obj
            Instruction::new(OpCode::LoadConst, 1), // 42
            Instruction::new(OpCode::SetAttr, 0),   // obj.x = 42
            // GetAttr: verify
            Instruction::new(OpCode::LoadConst, 0), // obj
            Instruction::new(OpCode::GetAttr, 0),   // obj.x
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(42));
    });
}

#[test]
fn test_native_struct_eq_same() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let (struct_val, _) = register_test_struct(
            py,
            &mut vm,
            "Point",
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        // Create two identical instances and compare
        let mut code = CodeObject::new("test_eq_same");
        code.constants = vec![struct_val, Value::from_int(10), Value::from_int(20)];
        code.instructions = vec![
            // Point(10, 20)
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::new(OpCode::LoadConst, 2),
            Instruction::new(OpCode::Call, 2),
            // Point(10, 20)
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::new(OpCode::LoadConst, 2),
            Instruction::new(OpCode::Call, 2),
            // ==
            Instruction::simple(OpCode::Eq),
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    });
}

#[test]
fn test_native_struct_eq_different_values() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let (struct_val, _) = register_test_struct(
            py,
            &mut vm,
            "Point",
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        // Point(10, 20) == Point(10, 99) -> false
        let mut code = CodeObject::new("test_eq_diff_vals");
        code.constants = vec![
            struct_val,
            Value::from_int(10),
            Value::from_int(20),
            Value::from_int(99),
        ];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::new(OpCode::LoadConst, 2),
            Instruction::new(OpCode::Call, 2),
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::new(OpCode::LoadConst, 3), // y=99
            Instruction::new(OpCode::Call, 2),
            Instruction::simple(OpCode::Eq),
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    });
}

#[test]
fn test_native_struct_eq_different_types() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let (point_val, _) = register_test_struct(
            py,
            &mut vm,
            "Point",
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        let (vec_val, _) = register_test_struct(
            py,
            &mut vm,
            "Vec2",
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
        );

        // Point(10, 20) == Vec2(10, 20) -> false (different types)
        let mut code = CodeObject::new("test_eq_diff_types");
        code.constants = vec![point_val, vec_val, Value::from_int(10), Value::from_int(20)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0), // Point
            Instruction::new(OpCode::LoadConst, 2),
            Instruction::new(OpCode::LoadConst, 3),
            Instruction::new(OpCode::Call, 2),
            Instruction::new(OpCode::LoadConst, 1), // Vec2
            Instruction::new(OpCode::LoadConst, 2),
            Instruction::new(OpCode::LoadConst, 3),
            Instruction::new(OpCode::Call, 2),
            Instruction::simple(OpCode::Eq),
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    });
}

#[test]
fn test_native_struct_pattern_match() {
    // Point(10, 20) matches Point{x, y} -> bindings x=10, y=20
    crate::test_support::init_python();
    Python::attach(|py| {
        let (mut vm, instance) = make_point_instance(py, 10, 20);

        let mut code = CodeObject::new("test_pattern_match");
        code.constants = vec![instance];
        // slots 0=x, 1=y
        code.patterns = vec![VMPattern::Struct {
            name: "Point".into(),
            variant: None,
            field_slots: vec![("x".into(), 0), ("y".into(), 1)],
        }];
        code.nlocals = 2;
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),      // push instance
            Instruction::new(OpCode::MatchPatternVM, 0), // match against pattern 0
            Instruction::simple(OpCode::BindMatch),      // bind x=slot0, y=slot1
            Instruction::new(OpCode::LoadLocal, 0),      // push x
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(10));

        // Also check y
        let mut code2 = CodeObject::new("test_pattern_match_y");
        code2.constants = vec![instance];
        code2.patterns = vec![VMPattern::Struct {
            name: "Point".into(),
            variant: None,
            field_slots: vec![("x".into(), 0), ("y".into(), 1)],
        }];
        code2.nlocals = 2;
        code2.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::MatchPatternVM, 0),
            Instruction::simple(OpCode::BindMatch),
            Instruction::new(OpCode::LoadLocal, 1), // push y
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code2), &[]).unwrap();
        assert_eq!(result.as_int(), Some(20));
    });
}

#[test]
fn test_native_struct_pattern_mismatch_type() {
    // Point(10, 20) does NOT match Vec2{x, y}
    crate::test_support::init_python();
    Python::attach(|py| {
        let (mut vm, instance) = make_point_instance(py, 10, 20);

        let mut code = CodeObject::new("test_pattern_mismatch");
        code.constants = vec![instance];
        code.patterns = vec![VMPattern::Struct {
            name: "Vec2".into(),
            variant: None,
            field_slots: vec![("x".into(), 0), ("y".into(), 1)],
        }];
        code.nlocals = 2;
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::MatchPatternVM, 0),
            Instruction::simple(OpCode::Halt), // result is on stack: TRUE or NIL
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert!(result.is_nil(), "expected NIL for type mismatch, got {:?}", result);
    });
}

#[test]
fn test_native_struct_pattern_unknown_field() {
    // Point(10, 20) with pattern Point{x, z} -> no match (z doesn't exist)
    crate::test_support::init_python();
    Python::attach(|py| {
        let (mut vm, instance) = make_point_instance(py, 10, 20);

        let mut code = CodeObject::new("test_pattern_unknown_field");
        code.constants = vec![instance];
        code.patterns = vec![VMPattern::Struct {
            name: "Point".into(),
            variant: None,
            field_slots: vec![("x".into(), 0), ("z".into(), 1)],
        }];
        code.nlocals = 2;
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::MatchPatternVM, 0),
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert!(result.is_nil(), "expected NIL for unknown field, got {:?}", result);
    });
}

#[test]
fn test_bigint_eq_no_python_fallback() {
    crate::test_support::init_python();
    Python::attach(|py| {
        reset_vm_fallback_stats();

        let n = Integer::from(i64::MAX) * Integer::from(1000_u32);
        let mut code = CodeObject::new("test_bigint_eq_no_fallback");
        code.constants = vec![Value::from_bigint(n.clone()), Value::from_bigint(n)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::simple(OpCode::Eq),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_bool(), Some(true));

        let stats = get_vm_fallback_stats();
        assert_eq!(stats.py_compare_eq, 0);
    });
}

#[test]
fn test_match_pattern_literal_bigint_no_python_fallback() {
    crate::test_support::init_python();
    Python::attach(|py| {
        reset_vm_fallback_stats();

        let n = Integer::from(i64::MAX) * Integer::from(2000_u32);
        let mut code = CodeObject::new("test_match_bigint_literal");
        code.constants = vec![Value::from_bigint(n.clone())];
        code.patterns = vec![VMPattern::Literal(Value::from_bigint(n))];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::MatchPatternVM, 0),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_bool(), Some(true));

        let stats = get_vm_fallback_stats();
        assert_eq!(stats.py_pattern_literal_eq, 0);
    });
}

#[test]
fn test_struct_instance_to_pyobject() {
    crate::test_support::init_python();
    Python::attach(|py| {
        let mut vm = VM::new();

        let type_id = vm.struct_registry.register_type(
            "Point".into(),
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            IndexMap::new(),
            vec![],               // implements
            vec!["Point".into()], // mro
        );

        // Create a native instance
        let idx = vm
            .struct_registry
            .create_instance(type_id, vec![Value::from_int(10), Value::from_int(20)]);
        let struct_val = Value::from_struct_instance(idx);

        // Install registries for to_pyobject
        crate::vm::value::set_struct_registry(&vm.struct_registry as *const _);
        crate::vm::value::set_func_table(&vm.func_table as *const _);

        let py_obj = struct_val.to_pyobject(py);
        let py_obj_bound = py_obj.bind(py);

        // Check it's a CatnipStructProxy with correct fields
        let x: i64 = py_obj_bound.getattr("x").unwrap().extract().unwrap();
        let y: i64 = py_obj_bound.getattr("y").unwrap().extract().unwrap();
        assert_eq!(x, 10);
        assert_eq!(y, 20);

        // Check repr
        let repr: String = py_obj_bound.repr().unwrap().extract().unwrap();
        assert!(repr.contains("Point"));
        assert!(repr.contains("x=10"));
        assert!(repr.contains("y=20"));

        crate::vm::value::clear_struct_registry();
        crate::vm::value::clear_symbol_table();
    });
}

// --- SmallInt overflow tests (regression: wrapping_add masked i64 overflow) ---

use catnip_core::nanbox::{SMALLINT_MAX as SMAX, SMALLINT_MIN as SMIN};

#[test]
fn test_add_smallint_overflow_to_bigint() {
    let result = binary_add(Value::from_int(SMAX), Value::from_int(1)).unwrap();
    assert!(result.is_bigint(), "SMAX + 1 must promote to BigInt");
    let expected = Integer::from(SMAX) + Integer::from(1);
    assert_eq!(unsafe { result.as_bigint_ref().unwrap() }, &expected);
    result.decref();
}

#[test]
fn test_add_i64_overflow_to_bigint() {
    // Sum overflows i64 -- checked_add returns None
    let result = binary_add(Value::from_int(SMAX), Value::from_int(SMAX)).unwrap();
    assert!(result.is_bigint(), "SMAX + SMAX must promote to BigInt");
    let expected = Integer::from(SMAX) + Integer::from(SMAX);
    assert_eq!(unsafe { result.as_bigint_ref().unwrap() }, &expected);
    result.decref();
}

#[test]
fn test_sub_smallint_overflow_to_bigint() {
    let result = binary_sub(Value::from_int(SMIN), Value::from_int(1)).unwrap();
    assert!(result.is_bigint(), "SMIN - 1 must promote to BigInt");
    result.decref();
}

#[test]
fn test_mul_smallint_overflow_to_bigint() {
    let result = binary_mul(Value::from_int(SMAX), Value::from_int(2)).unwrap();
    assert!(result.is_bigint(), "SMAX * 2 must promote to BigInt");
    let expected = Integer::from(SMAX) * Integer::from(2);
    assert_eq!(unsafe { result.as_bigint_ref().unwrap() }, &expected);
    result.decref();
}

#[test]
fn test_mul_i64_overflow_to_bigint() {
    // checked_mul returns None
    let result = binary_mul(Value::from_int(SMAX), Value::from_int(SMAX)).unwrap();
    assert!(result.is_bigint(), "SMAX^2 must promote to BigInt");
    let expected = Integer::from((1_i64 << 46) - 1) * Integer::from((1_i64 << 46) - 1);
    assert_eq!(unsafe { result.as_bigint_ref().unwrap() }, &expected);
    result.decref();
}

// --- CodeObject pool ownership (constants/defaults/pattern literals) ---
//
// The pools own one reference per slot; CodeObject::drop is the single
// release site. Oracle: the Python refcount of a witness object rises by
// one when pooled (ObjectTable slot holds a Py ref) and falls back when
// the CodeObject dies. Immune to test parallelism, unlike a global slot
// count.

fn py_refcnt(obj: &Bound<'_, PyAny>) -> isize {
    // SAFETY: obj wraps a live, GIL-bound object pointer.
    unsafe { pyo3::ffi::Py_REFCNT(obj.as_ptr()) }
}

#[test]
fn code_object_drop_releases_pool_values() {
    use crate::vm::pattern::{VMPattern, VMPatternElement};
    use pyo3::types::PyString;

    crate::test_support::init_python();
    Python::attach(|py| {
        let c = PyString::new(py, "pool-const-witness").into_any();
        let d = PyString::new(py, "pool-default-witness").into_any();
        let p = PyString::new(py, "pool-pattern-witness").into_any();
        let (rc_c, rc_d, rc_p) = (py_refcnt(&c), py_refcnt(&d), py_refcnt(&p));

        let mut code = CodeObject::new("pool_probe");
        code.constants = vec![Value::from_pyobject(py, &c).unwrap()];
        code.defaults = vec![Value::from_pyobject(py, &d).unwrap()];
        // Nested literal: exercises the recursive walk (Or > Tuple > Literal).
        code.patterns = vec![VMPattern::Or(vec![
            VMPattern::Tuple(vec![VMPatternElement::Pattern(VMPattern::Literal(
                Value::from_pyobject(py, &p).unwrap(),
            ))]),
            VMPattern::Wildcard,
        ])];
        assert_eq!(py_refcnt(&c), rc_c + 1, "constant pooled: slot holds a ref");
        assert_eq!(py_refcnt(&d), rc_d + 1, "default pooled: slot holds a ref");
        assert_eq!(py_refcnt(&p), rc_p + 1, "pattern literal pooled: slot holds a ref");

        drop(code);
        assert_eq!(py_refcnt(&c), rc_c, "constant released on CodeObject drop");
        assert_eq!(py_refcnt(&d), rc_d, "default released on CodeObject drop");
        assert_eq!(py_refcnt(&p), rc_p, "pattern literal released on CodeObject drop");
    });
}

#[test]
fn compile_execute_drop_releases_const_pool() {
    use crate::vm::compiler_core::{CompilerCore, CompilerCoreExt};
    use pyo3::types::PyString;

    crate::test_support::init_python();
    Python::attach(|py| {
        let s = PyString::new(py, "exec-pool-witness").into_any();
        let rc0 = py_refcnt(&s);

        let mut core = CompilerCore::new();
        let sidx = core.add_const_py(py, &s).unwrap();
        let zidx = core.add_const(Value::from_int(0));
        core.emit(OpCode::LoadConst, sidx as u32);
        core.emit(OpCode::PopTop, 0);
        core.emit(OpCode::LoadConst, zidx as u32);
        core.emit(OpCode::Halt, 0);
        let code = core.build_code_object(py).unwrap();

        // Moved, not shared: exactly one pool ref, still held after the
        // compiler (whose buffers are now empty) goes away.
        drop(core);
        assert_eq!(py_refcnt(&s), rc0 + 1, "pool owns a single ref after build");

        let mut vm = VM::new();
        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        assert_eq!(result.as_int(), Some(0));
        drop(vm);
        assert_eq!(py_refcnt(&s), rc0, "pool ref released once the code dies");
    });
}

#[test]
fn add_const_dedup_releases_candidate() {
    use crate::vm::compiler_core::CompilerCore;
    use crate::vm::value::GmpInt;
    use catnip_core::nanbox::PAYLOAD_MASK;

    let n = Integer::from(i64::MAX) * Integer::from(9973_u32);
    let pooled = Value::from_bigint(n.clone());
    let candidate = Value::from_bigint(n); // equal by value, distinct allocation
    let ptr = (candidate.to_raw() & PAYLOAD_MASK) as *const GmpInt;
    // Witness ref so the allocation stays observable after the dedup decref.
    unsafe { Arc::increment_strong_count(ptr) };

    let mut core = CompilerCore::new();
    let i1 = core.add_const(pooled);
    let i2 = core.add_const(candidate);
    assert_eq!(i1, i2, "BigInt constants dedup by value");

    let witness = unsafe { Arc::from_raw(ptr) };
    assert_eq!(
        Arc::strong_count(&witness),
        1,
        "deduped candidate released its ref (witness is the only holder)"
    );
}
