// FILE: catnip_rs/src/ir/tests.rs
//! Tests for IR JSON serialization

#[cfg(test)]
mod tests {
    use crate::ir::opcode::IROpCode;
    use crate::ir::pure::{BroadcastType, IRPure};
    use std::collections::HashMap;

    #[test]
    fn test_irpure_int_json() {
        let ir = IRPure::Int(42);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Int""#));
        assert!(json.contains("42"));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_float_json() {
        let ir = IRPure::Float(3.14);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Float""#));
        assert!(json.contains("3.14"));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_string_json() {
        let ir = IRPure::String("hello".into());
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""String""#));
        assert!(json.contains("hello"));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_bool_json() {
        let ir = IRPure::Bool(true);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Bool""#));
        assert!(json.contains("true"));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_none_json() {
        let ir = IRPure::None;
        let json = ir.to_json().unwrap();
        // None avec le rename devient {"type":"None"} au lieu de juste "None"
        assert!(json.contains("None"));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_list_json() {
        let ir = IRPure::List(vec![IRPure::Int(1), IRPure::Int(2), IRPure::Int(3)]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""List""#));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_tuple_json() {
        let ir = IRPure::Tuple(vec![IRPure::Int(1), IRPure::String("a".into())]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Tuple""#));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_dict_json() {
        // Dict avec clés non-string
        let ir = IRPure::Dict(vec![
            (IRPure::Int(1), IRPure::String("a".into())),
            (IRPure::Int(2), IRPure::String("b".into())),
        ]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Dict""#));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_set_json() {
        let ir = IRPure::Set(vec![IRPure::Int(1), IRPure::Int(2)]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Set""#));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_op_json() {
        let ir = IRPure::op(IROpCode::Add, vec![IRPure::Int(1), IRPure::Int(2)]);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Op""#));
        assert!(json.contains(r#""opcode":"Add""#));
        assert!(json.contains(r#""tail":false"#));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_identifier_json() {
        let ir = IRPure::Identifier("x".into());
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Identifier""#));
        assert!(json.contains("x"));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_ref_json() {
        let ir = IRPure::Ref("x".into(), -1, -1);
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Ref""#));
        assert!(json.contains("x"));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_broadcast_with_none_json() {
        // Test explicite pour Option<T> → null
        let ir = IRPure::Broadcast {
            target: None,
            operator: Box::new(IRPure::Identifier("+".into())),
            operand: Some(Box::new(IRPure::Int(5))),
            broadcast_type: BroadcastType::Binary,
        };
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Broadcast""#));
        assert!(json.contains(r#""target":null"#)); // IMPORTANT: null explicite
        assert!(json.contains(r#""broadcast_type":"Binary""#));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_slice_json() {
        let ir = IRPure::Slice {
            start: Box::new(IRPure::Int(0)),
            stop: Box::new(IRPure::Int(10)),
            step: Box::new(IRPure::Int(1)),
        };
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Slice""#));

        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }

    #[test]
    fn test_irpure_call_json() {
        let ir = IRPure::Call {
            func: Box::new(IRPure::Identifier("print".into())),
            args: vec![IRPure::String("hello".into())],
            kwargs: HashMap::new(),
            start_byte: 0,
            end_byte: 0,
        };
        let json = ir.to_json().unwrap();
        assert!(json.contains(r#""Call""#));

        let roundtrip = IRPure::from_json(&json).unwrap();
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
        let ir = IRPure::Op {
            opcode: IROpCode::OpIf,
            args: vec![
                IRPure::op(
                    IROpCode::Eq,
                    vec![IRPure::Identifier("x".into()), IRPure::Int(5)],
                ),
                IRPure::op(
                    IROpCode::Add,
                    vec![IRPure::Identifier("x".into()), IRPure::Int(1)],
                ),
                IRPure::Int(0),
            ],
            kwargs: HashMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        };

        let json = ir.to_json_pretty().unwrap();
        let roundtrip = IRPure::from_json(&json).unwrap();
        assert_eq!(ir, roundtrip);
    }
}
