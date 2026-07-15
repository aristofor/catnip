// FILE: catnip_core/src/freeze/struct_type.rs
//! Serializable struct type definitions for the ND process worker pool.
//!
//! A worker reconstructs a struct type from these, keyed by name, so a
//! broadcast/ND over struct instances runs on the native parallel path
//! instead of the Python fallback.
//!
//! v1 frontier: flat structs only. The *absence* of parent/trait/abstract
//! fields on [`FrozenStructType`] is the gate -- the parent-side collector
//! (freeze step C) cannot represent an `extends`/`implements`/abstract type
//! and falls back to the Python path for it.

use super::FrozenValue;
use crate::vm::opcode::ParamCheck;
use serde::{Deserialize, Serialize};

/// A struct field carried to a worker: name, optional default value, and the
/// boundary check its type annotation compiles to (preserved verbatim, so the
/// worker enforces the same field contract as the parent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenField {
    pub name: String,
    pub has_default: bool,
    pub default: FrozenValue,
    pub check: ParamCheck,
}

/// A struct method carried to a worker as its frozen IR body -- the same
/// `encoded_ir` channel a lambda travels through (a method *is* a lambda).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenMethod {
    pub name: String,
    /// Postcard-encoded lambda IR (`Vec<IR>`, no `.catf` header).
    pub encoded_ir: Vec<u8>,
    pub param_names: Vec<String>,
    pub is_static: bool,
}

/// A serializable, nominal struct type definition. Fields are keyed by name so
/// the worker's field order need not match the parent's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenStructType {
    pub name: String,
    pub fields: Vec<FrozenField>,
    pub methods: Vec<FrozenMethod>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frozen_struct_type_roundtrip_postcard() {
        let ty = FrozenStructType {
            name: "Point".into(),
            fields: vec![
                FrozenField {
                    name: "x".into(),
                    has_default: false,
                    default: FrozenValue::None,
                    check: ParamCheck::Primitive(3),
                },
                FrozenField {
                    name: "label".into(),
                    has_default: true,
                    default: FrozenValue::String("origin".into()),
                    check: ParamCheck::Nominal("str".into()),
                },
            ],
            methods: vec![FrozenMethod {
                name: "norm".into(),
                encoded_ir: vec![1, 2, 3, 4],
                param_names: vec!["self".into()],
                is_static: false,
            }],
        };
        let encoded = postcard::to_allocvec(&ty).unwrap();
        let decoded: FrozenStructType = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded, ty);
    }

    #[test]
    fn test_frozen_field_check_variants_roundtrip() {
        // ParamCheck must survive the wire, including its recursive variants.
        let checks = vec![
            ParamCheck::None,
            ParamCheck::Primitive(1),
            ParamCheck::Nominal("Foo".into()),
            ParamCheck::Union(Box::from([ParamCheck::Primitive(3), ParamCheck::Nominal("Bar".into())])),
            ParamCheck::Composite {
                head: 7,
                params: Box::from([ParamCheck::Primitive(3)]),
            },
            ParamCheck::Generic {
                name: "Option".into(),
                args: Box::from([ParamCheck::Primitive(3)]),
            },
            ParamCheck::Callable { arity: 2 },
        ];
        for check in checks {
            let field = FrozenField {
                name: "f".into(),
                has_default: false,
                default: FrozenValue::None,
                check: check.clone(),
            };
            let encoded = postcard::to_allocvec(&field).unwrap();
            let decoded: FrozenField = postcard::from_bytes(&encoded).unwrap();
            assert_eq!(decoded.check, check);
        }
    }
}
