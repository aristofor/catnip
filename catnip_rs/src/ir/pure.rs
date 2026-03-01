// FILE: catnip_rs/src/ir/pure.rs
//! Pure Rust IR types - no Python dependencies
//!
//! Standalone representation of the IR graph for the full Rust pipeline.
//! Used by the parser, semantic analyzer and standalone compiler.

use super::opcode::IROpCode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Pure Rust IR node (no PyO3 dependency)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IRPure {
    /// Integer literal (smallint compatible: -2^47 to 2^47-1)
    Int(i64),

    /// Float literal
    Float(f64),

    /// String literal
    String(String),

    /// Bytes literal
    Bytes(Vec<u8>),

    /// Boolean literal
    Bool(bool),

    /// None/Nil literal
    None,

    /// Decimal literal (base-10 exact, stored as text for deferred parsing)
    Decimal(String),

    /// Imaginary literal (pure imaginary part, stored as text for deferred parsing)
    Imaginary(String),

    /// Operation node (generic for all opcodes)
    Op {
        opcode: IROpCode,
        args: Vec<IRPure>,
        kwargs: HashMap<String, IRPure>,
        tail: bool,
        start_byte: usize,
        end_byte: usize,
    },

    /// Identifier (variable name)
    Identifier(String),

    /// Reference (variable reference), with optional source position
    Ref(String, isize, isize),

    /// List literal
    List(Vec<IRPure>),

    /// Top-level statement sequence (returned by transform_source_file)
    Program(Vec<IRPure>),

    /// Tuple literal
    Tuple(Vec<IRPure>),

    /// Dict literal
    Dict(
        #[serde(
            serialize_with = "serialize_dict_entries",
            deserialize_with = "deserialize_dict_entries"
        )]
        Vec<(IRPure, IRPure)>,
    ),

    /// Set literal
    Set(Vec<IRPure>),

    /// Function call
    Call {
        func: Box<IRPure>,
        args: Vec<IRPure>,
        kwargs: HashMap<String, IRPure>,
        start_byte: usize,
        end_byte: usize,
    },

    /// Pattern matching variants
    PatternLiteral(Box<IRPure>),
    PatternVar(String),
    PatternWildcard,
    PatternOr(Vec<IRPure>),
    PatternTuple(Vec<IRPure>),
    PatternStruct {
        name: String,
        fields: Vec<String>,
    },

    /// Slice object (for array/list slicing)
    Slice {
        start: Box<IRPure>,
        stop: Box<IRPure>,
        step: Box<IRPure>,
    },

    /// Broadcasting operation (vectorized operations)
    Broadcast {
        target: Option<Box<IRPure>>,
        operator: Box<IRPure>,
        operand: Option<Box<IRPure>>,
        broadcast_type: BroadcastType,
    },
}

/// Custom serializer for Dict entries (non-string keys)
fn serialize_dict_entries<S>(entries: &[(IRPure, IRPure)], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(entries.len()))?;
    for (k, v) in entries {
        seq.serialize_element(&vec![k, v])?;
    }
    seq.end()
}

/// Custom deserializer for Dict entries
fn deserialize_dict_entries<'de, D>(deserializer: D) -> Result<Vec<(IRPure, IRPure)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let pairs: Vec<Vec<IRPure>> = Deserialize::deserialize(deserializer)?;
    pairs
        .into_iter()
        .map(|pair| {
            if pair.len() == 2 {
                Ok((pair[0].clone(), pair[1].clone()))
            } else {
                Err(D::Error::custom("Dict entry must be [key, value]"))
            }
        })
        .collect()
}

/// Broadcasting type
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BroadcastType {
    Binary,      // .[+ 1]
    Unary,       // .[- ]
    If,          // .[if x > 0]
    Lambda,      // .[lambda]
    NDMap,       // .[NDMap]
    NDRecursion, // .[NDRecur]
}

impl IRPure {
    /// Create an operation node with default metadata
    pub fn op(opcode: IROpCode, args: Vec<IRPure>) -> Self {
        Self::Op {
            opcode,
            args,
            kwargs: HashMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        }
    }

    /// Create a function call
    pub fn call(func: IRPure, args: Vec<IRPure>) -> Self {
        Self::Call {
            func: Box::new(func),
            args,
            kwargs: HashMap::new(),
            start_byte: 0,
            end_byte: 0,
        }
    }

    /// Create an operation with kwargs
    pub fn op_with_kwargs(
        opcode: IROpCode,
        args: Vec<IRPure>,
        kwargs: HashMap<String, IRPure>,
    ) -> Self {
        Self::Op {
            opcode,
            args,
            kwargs,
            tail: false,
            start_byte: 0,
            end_byte: 0,
        }
    }

    /// Create an operation with position metadata
    pub fn op_with_pos(
        opcode: IROpCode,
        args: Vec<IRPure>,
        start_byte: usize,
        end_byte: usize,
    ) -> Self {
        Self::Op {
            opcode,
            args,
            kwargs: HashMap::new(),
            tail: false,
            start_byte,
            end_byte,
        }
    }

    /// Check if this is a literal value
    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            IRPure::Int(_)
                | IRPure::Float(_)
                | IRPure::String(_)
                | IRPure::Bytes(_)
                | IRPure::Bool(_)
                | IRPure::None
                | IRPure::Decimal(_)
                | IRPure::Imaginary(_)
        )
    }

    /// Check if this is an operation
    pub fn is_op(&self) -> bool {
        matches!(self, IRPure::Op { .. })
    }

    /// Get opcode if this is an operation
    pub fn opcode(&self) -> Option<IROpCode> {
        match self {
            IRPure::Op { opcode, .. } => Some(*opcode),
            _ => None,
        }
    }

    /// Get args if this is an operation
    pub fn args(&self) -> Option<&[IRPure]> {
        match self {
            IRPure::Op { args, .. } => Some(args),
            _ => None,
        }
    }

    /// Check if this is a tail call
    pub fn is_tail(&self) -> bool {
        match self {
            IRPure::Op { tail, .. } => *tail,
            _ => false,
        }
    }

    /// Mark this operation as a tail call
    pub fn mark_tail(mut self) -> Self {
        if let IRPure::Op { ref mut tail, .. } = self {
            *tail = true;
        }
        self
    }

    /// Serialize to JSON string (compact).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serialize to JSON string (pretty-printed).
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Compact JSON representation — debug-friendly, no serde tags.
    ///
    /// Primitives aplaties (42, "hello", true, null), Op/Call compacts
    /// with args/kwargs always present but tail/pos omitted when default.
    pub fn to_compact_value(&self) -> serde_json::Value {
        use serde_json::{json, Map, Value};

        match self {
            IRPure::Int(n) => json!(n),
            IRPure::Float(f) => json!(f),
            IRPure::String(s) => json!(s),
            IRPure::Bytes(v) => {
                let hex: String = v.iter().map(|b| format!("{:02x}", b)).collect();
                json!({"bytes": hex})
            }
            IRPure::Bool(b) => json!(b),
            IRPure::None => Value::Null,
            IRPure::Decimal(s) => json!({"decimal": s}),
            IRPure::Imaginary(s) => json!({"imaginary": s}),

            IRPure::Ref(name, _, _) => json!({"ref": name}),
            IRPure::Identifier(name) => json!({"id": name}),

            IRPure::Tuple(items) => {
                Value::Array(items.iter().map(|i| i.to_compact_value()).collect())
            }
            IRPure::List(items) => {
                json!({"list": items.iter().map(|i| i.to_compact_value()).collect::<Vec<_>>()})
            }
            IRPure::Program(items) => {
                json!({"program": items.iter().map(|i| i.to_compact_value()).collect::<Vec<_>>()})
            }
            IRPure::Set(items) => {
                json!({"set": items.iter().map(|i| i.to_compact_value()).collect::<Vec<_>>()})
            }
            IRPure::Dict(entries) => {
                let pairs: Vec<Value> = entries
                    .iter()
                    .map(|(k, v)| json!([k.to_compact_value(), v.to_compact_value()]))
                    .collect();
                json!({"dict": pairs})
            }

            IRPure::PatternWildcard => json!("_"),
            IRPure::PatternVar(name) => json!({"pat_var": name}),
            IRPure::PatternLiteral(inner) => json!({"pat_lit": inner.to_compact_value()}),
            IRPure::PatternOr(pats) => {
                json!({"pat_or": pats.iter().map(|p| p.to_compact_value()).collect::<Vec<_>>()})
            }
            IRPure::PatternTuple(pats) => {
                json!({"pat_tuple": pats.iter().map(|p| p.to_compact_value()).collect::<Vec<_>>()})
            }
            IRPure::PatternStruct { name, fields } => {
                json!({"pat_struct": {"name": name, "fields": fields}})
            }

            IRPure::Slice { start, stop, step } => {
                json!({"slice": [start.to_compact_value(), stop.to_compact_value(), step.to_compact_value()]})
            }

            IRPure::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => {
                // Opcode name via serde (PascalCase)
                let opcode_name =
                    serde_json::to_value(opcode).unwrap_or_else(|_| json!(format!("{:?}", opcode)));

                let mut map = Map::new();
                map.insert("op".into(), opcode_name);
                map.insert(
                    "args".into(),
                    Value::Array(args.iter().map(|a| a.to_compact_value()).collect()),
                );
                map.insert(
                    "kwargs".into(),
                    Value::Object(
                        kwargs
                            .iter()
                            .map(|(k, v)| (k.clone(), v.to_compact_value()))
                            .collect(),
                    ),
                );
                if *tail {
                    map.insert("tail".into(), json!(true));
                }
                if *start_byte != 0 || *end_byte != 0 {
                    map.insert("pos".into(), json!([start_byte, end_byte]));
                }
                Value::Object(map)
            }

            IRPure::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
            } => {
                let mut map = Map::new();
                map.insert("call".into(), func.to_compact_value());
                map.insert(
                    "args".into(),
                    Value::Array(args.iter().map(|a| a.to_compact_value()).collect()),
                );
                map.insert(
                    "kwargs".into(),
                    Value::Object(
                        kwargs
                            .iter()
                            .map(|(k, v)| (k.clone(), v.to_compact_value()))
                            .collect(),
                    ),
                );
                if *start_byte != 0 || *end_byte != 0 {
                    map.insert("pos".into(), json!([start_byte, end_byte]));
                }
                Value::Object(map)
            }

            IRPure::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => {
                let type_name = serde_json::to_value(broadcast_type)
                    .unwrap_or_else(|_| json!(format!("{:?}", broadcast_type)));

                let mut map = Map::new();
                map.insert("broadcast".into(), type_name);
                if let Some(t) = target {
                    map.insert("target".into(), t.to_compact_value());
                }
                map.insert("operator".into(), operator.to_compact_value());
                if let Some(o) = operand {
                    map.insert("operand".into(), o.to_compact_value());
                }
                Value::Object(map)
            }
        }
    }

    /// Compact JSON string (minified)
    pub fn to_compact_json(&self) -> String {
        self.to_compact_value().to_string()
    }

    /// Compact JSON string (pretty-printed)
    pub fn to_compact_json_pretty(&self) -> String {
        serde_json::to_string_pretty(&self.to_compact_value())
            .unwrap_or_else(|_| self.to_compact_json())
    }
}

/// Conversion helpers for standard type compatibility
impl From<i64> for IRPure {
    fn from(val: i64) -> Self {
        IRPure::Int(val)
    }
}

impl From<f64> for IRPure {
    fn from(val: f64) -> Self {
        IRPure::Float(val)
    }
}

impl From<bool> for IRPure {
    fn from(val: bool) -> Self {
        IRPure::Bool(val)
    }
}

impl From<String> for IRPure {
    fn from(val: String) -> Self {
        IRPure::String(val)
    }
}

impl From<&str> for IRPure {
    fn from(val: &str) -> Self {
        IRPure::String(val.to_string())
    }
}

impl From<Vec<u8>> for IRPure {
    fn from(val: Vec<u8>) -> Self {
        IRPure::Bytes(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_creation() {
        assert!(IRPure::Int(42).is_literal());
        assert!(IRPure::Float(3.14).is_literal());
        assert!(IRPure::String("hello".into()).is_literal());
        assert!(IRPure::Bytes(vec![1, 2, 3]).is_literal());
        assert!(IRPure::Bool(true).is_literal());
        assert!(IRPure::None.is_literal());
    }

    #[test]
    fn test_op_creation() {
        let op = IRPure::op(IROpCode::Add, vec![IRPure::Int(1), IRPure::Int(2)]);
        assert!(op.is_op());
        assert_eq!(op.opcode(), Some(IROpCode::Add));
        assert_eq!(op.args().unwrap().len(), 2);
    }

    #[test]
    fn test_tail_marking() {
        let op = IRPure::op(IROpCode::Call, vec![]).mark_tail();
        assert!(op.is_tail());
    }

    #[test]
    fn test_conversions() {
        let _: IRPure = 42.into();
        let _: IRPure = 3.14.into();
        let _: IRPure = true.into();
        let _: IRPure = "test".into();
    }

    // --- Compact JSON tests ---

    #[test]
    fn test_compact_primitives_flat() {
        assert_eq!(IRPure::Int(42).to_compact_json(), "42");
        assert_eq!(IRPure::Float(3.14).to_compact_json(), "3.14");
        assert_eq!(IRPure::Bool(true).to_compact_json(), "true");
        assert_eq!(IRPure::None.to_compact_json(), "null");
        assert_eq!(
            IRPure::String("hello".into()).to_compact_json(),
            r#""hello""#
        );
    }

    #[test]
    fn test_compact_ref_and_identifier() {
        let v: serde_json::Value =
            serde_json::from_str(&IRPure::Ref("x".into(), -1, -1).to_compact_json()).unwrap();
        assert_eq!(v["ref"], "x");

        let v: serde_json::Value =
            serde_json::from_str(&IRPure::Identifier("y".into()).to_compact_json()).unwrap();
        assert_eq!(v["id"], "y");
    }

    #[test]
    fn test_compact_op_args_kwargs_always_present() {
        let op = IRPure::op(IROpCode::Add, vec![IRPure::Int(1), IRPure::Int(2)]);
        let v: serde_json::Value = serde_json::from_str(&op.to_compact_json()).unwrap();

        assert_eq!(v["op"], "Add");
        assert!(v["args"].is_array());
        assert_eq!(v["args"].as_array().unwrap().len(), 2);
        assert!(v["kwargs"].is_object());
        // tail=false omitted
        assert!(v.get("tail").is_none());
        // pos=0,0 omitted
        assert!(v.get("pos").is_none());
    }

    #[test]
    fn test_compact_op_empty_args_present() {
        let op = IRPure::op(IROpCode::Nop, vec![]);
        let v: serde_json::Value = serde_json::from_str(&op.to_compact_json()).unwrap();

        assert_eq!(v["args"], serde_json::json!([]));
        assert_eq!(v["kwargs"], serde_json::json!({}));
    }

    #[test]
    fn test_compact_op_tail_shown_when_true() {
        let op = IRPure::op(IROpCode::Call, vec![]).mark_tail();
        let v: serde_json::Value = serde_json::from_str(&op.to_compact_json()).unwrap();

        assert_eq!(v["tail"], true);
    }

    #[test]
    fn test_compact_op_pos_shown_when_nonzero() {
        let op = IRPure::op_with_pos(IROpCode::Add, vec![], 10, 25);
        let v: serde_json::Value = serde_json::from_str(&op.to_compact_json()).unwrap();

        assert_eq!(v["pos"], serde_json::json!([10, 25]));
    }

    #[test]
    fn test_compact_call() {
        let call = IRPure::call(
            IRPure::Identifier("print".into()),
            vec![IRPure::String("hi".into())],
        );
        let v: serde_json::Value = serde_json::from_str(&call.to_compact_json()).unwrap();

        assert_eq!(v["call"]["id"], "print");
        assert!(v["args"].is_array());
        assert!(v["kwargs"].is_object());
    }

    #[test]
    fn test_compact_tuple_is_array() {
        let t = IRPure::Tuple(vec![IRPure::Int(1), IRPure::Int(2)]);
        let v: serde_json::Value = serde_json::from_str(&t.to_compact_json()).unwrap();
        assert!(v.is_array());
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_compact_list_tagged() {
        let l = IRPure::List(vec![IRPure::Int(1)]);
        let v: serde_json::Value = serde_json::from_str(&l.to_compact_json()).unwrap();
        assert!(v["list"].is_array());
    }

    #[test]
    fn test_compact_patterns() {
        assert_eq!(IRPure::PatternWildcard.to_compact_json(), r#""_""#);

        let v: serde_json::Value =
            serde_json::from_str(&IRPure::PatternVar("x".into()).to_compact_json()).unwrap();
        assert_eq!(v["pat_var"], "x");
    }

    #[test]
    fn test_compact_broadcast() {
        let b = IRPure::Broadcast {
            target: None,
            operator: Box::new(IRPure::Identifier("+".into())),
            operand: Some(Box::new(IRPure::Int(5))),
            broadcast_type: BroadcastType::Binary,
        };
        let v: serde_json::Value = serde_json::from_str(&b.to_compact_json()).unwrap();
        assert_eq!(v["broadcast"], "Binary");
        assert!(v.get("target").is_none()); // None omitted
        assert_eq!(v["operator"]["id"], "+");
        assert_eq!(v["operand"], 5);
    }

    #[test]
    fn test_compact_pretty_is_valid_json() {
        let op = IRPure::op(IROpCode::Add, vec![IRPure::Int(1), IRPure::Int(2)]);
        let pretty = op.to_compact_json_pretty();
        assert!(pretty.contains('\n'));
        let _: serde_json::Value = serde_json::from_str(&pretty).unwrap();
    }
}
