// FILE: catnip_core/src/freeze/value.rs
//! Serializable value type for frozen data.
//!
//! Separate from VM's NaN-boxed Value (which holds PyObject handles).
//! Covers all primitive types a Catnip program can produce.
//! Functions and closures are not freezable; struct instances freeze
//! nominally (by type name + field values, see [`FrozenValue::Struct`]).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrozenValue {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    None,
    List(Vec<FrozenValue>),
    Tuple(Vec<FrozenValue>),
    Dict(Vec<(FrozenValue, FrozenValue)>),
    Set(Vec<FrozenValue>),
    Bytes(Vec<u8>),
    /// A struct instance carried by type name and field (name, value) pairs.
    /// The worker resolves the type by name in its own registry, so field
    /// order need not match the parent's -- fields are keyed by name.
    Struct {
        type_name: String,
        fields: Vec<(String, FrozenValue)>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frozen_value_roundtrip_postcard() {
        let values = vec![
            FrozenValue::Int(42),
            FrozenValue::Float(3.14),
            FrozenValue::String("hello".into()),
            FrozenValue::Bool(true),
            FrozenValue::None,
            FrozenValue::Bytes(vec![0xCA, 0xFE]),
            FrozenValue::List(vec![FrozenValue::Int(1), FrozenValue::Int(2)]),
            FrozenValue::Tuple(vec![FrozenValue::String("a".into()), FrozenValue::Bool(false)]),
            FrozenValue::Set(vec![FrozenValue::Int(10)]),
            FrozenValue::Dict(vec![(FrozenValue::String("key".into()), FrozenValue::Int(99))]),
        ];

        for val in &values {
            let encoded = postcard::to_allocvec(val).unwrap();
            let decoded: FrozenValue = postcard::from_bytes(&encoded).unwrap();
            assert_eq!(&decoded, val);
        }
    }

    #[test]
    fn test_frozen_struct_roundtrip_postcard() {
        // Nested struct: a field holds another struct, itself holding a list.
        let s = FrozenValue::Struct {
            type_name: "Point".into(),
            fields: vec![
                ("x".into(), FrozenValue::Int(1)),
                (
                    "meta".into(),
                    FrozenValue::Struct {
                        type_name: "Meta".into(),
                        fields: vec![(
                            "tags".into(),
                            FrozenValue::List(vec![FrozenValue::String("a".into()), FrozenValue::None]),
                        )],
                    },
                ),
            ],
        };
        let encoded = postcard::to_allocvec(&s).unwrap();
        let decoded: FrozenValue = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded, s);
    }

    #[test]
    fn test_nested_frozen_value() {
        let nested = FrozenValue::List(vec![FrozenValue::Dict(vec![(
            FrozenValue::String("data".into()),
            FrozenValue::Tuple(vec![
                FrozenValue::Int(1),
                FrozenValue::List(vec![FrozenValue::Float(2.5), FrozenValue::None]),
            ]),
        )])]);

        let encoded = postcard::to_allocvec(&nested).unwrap();
        let decoded: FrozenValue = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded, nested);
    }
}
