// FILE: catnip_core/src/ir/tests.rs
//! Tests for IR JSON serialization

#[cfg(test)]
mod json_tests {
    use crate::ir::opcode::IROpCode;
    use crate::ir::pure::{BroadcastType, IR};
    use indexmap::IndexMap;

    #[test]
    fn test_irpure_int_json() {
        let ir = IR::Int(42);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Int""#));
        assert!(json.contains("42"));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_float_json() {
        let ir = IR::Float(2.5);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Float""#));
        assert!(json.contains("2.5"));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_string_json() {
        let ir = IR::String("hello".into());
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""String""#));
        assert!(json.contains("hello"));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_bool_json() {
        let ir = IR::Bool(true);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Bool""#));
        assert!(json.contains("true"));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_none_json() {
        let ir = IR::None;
        let json = ir.to_json().unwrap();
        // None avec le rename devient {"type":"None"} au lieu de juste "None"
        assert!(json.contains("None"));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_list_json() {
        let ir = IR::List(vec![IR::Int(1), IR::Int(2), IR::Int(3)]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""List""#));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_tuple_json() {
        let ir = IR::Tuple(vec![IR::Int(1), IR::String("a".into())]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Tuple""#));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_dict_json() {
        // Dict avec clés non-string
        let ir = IR::Dict(vec![
            (IR::Int(1), IR::String("a".into())),
            (IR::Int(2), IR::String("b".into())),
        ]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Dict""#));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_set_json() {
        let ir = IR::Set(vec![IR::Int(1), IR::Int(2)]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Set""#));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_op_json() {
        let ir = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Op""#));
        assert!(json.contains(r#""opcode":"Add""#));
        assert!(json.contains(r#""tail":false"#));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_identifier_json() {
        let ir = IR::Identifier("x".into());
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Identifier""#));
        assert!(json.contains("x"));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_ref_json() {
        let ir = IR::Ref("x".into(), -1, -1);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Ref""#));
        assert!(json.contains("x"));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_broadcast_with_none_json() {
        // Test explicite pour Option<T> → null
        let ir = IR::Broadcast {
            target: None,
            operator: Box::new(IR::Identifier("+".into())),
            operand: Some(Box::new(IR::Int(5))),
            broadcast_type: BroadcastType::Binary,
        };
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Broadcast""#));
        assert!(json.contains(r#""target":null"#)); // IMPORTANT: null explicite
        assert!(json.contains(r#""broadcast_type":"Binary""#));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_slice_json() {
        let ir = IR::Slice {
            start: Box::new(IR::Int(0)),
            stop: Box::new(IR::Int(10)),
            step: Box::new(IR::Int(1)),
        };
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Slice""#));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_call_json() {
        let ir = IR::Call {
            func: Box::new(IR::Identifier("print".into())),
            args: vec![IR::String("hello".into())],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        };
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Call""#));

        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_iropdode_json() {
        let opcode = IROpCode::Add;
        let json = opcode.to_json().unwrap();
        assert_eq!(json, r#""Add""#);

        let roundtrip = IROpCode::from_json(&json).unwrap();
        assert_eq!(opcode, roundtrip);
    }

    #[test]
    fn test_complex_nested_structure() {
        // Structure imbriquée complexe
        let ir = IR::Op {
            opcode: IROpCode::OpIf,
            args: vec![
                IR::op(IROpCode::Eq, vec![IR::Identifier("x".into()), IR::Int(5)]),
                IR::op(IROpCode::Add, vec![IR::Identifier("x".into()), IR::Int(1)]),
                IR::Int(0),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        };

        let json = ir.to_json_pretty().unwrap();
        let roundtrip = IR::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }
}
