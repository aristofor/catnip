// FILE: catnip_core/src/ir/pure.rs
//! Pure Rust IR types - no Python dependencies
//!
//! Standalone representation of the IR graph for the full Rust pipeline.
//! Used by the parser, semantic analyzer and standalone compiler.

use super::opcode::IROpCode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Pure Rust IR node (no PyO3 dependency)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IR {
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
        args: Vec<IR>,
        kwargs: IndexMap<String, IR>,
        tail: bool,
        start_byte: usize,
        end_byte: usize,
    },

    /// Identifier (variable name)
    Identifier(String),

    /// Reference (variable reference), with optional source position
    Ref(String, isize, isize),

    /// List literal
    List(Vec<IR>),

    /// Top-level statement sequence (returned by transform_source_file)
    Program(Vec<IR>),

    /// Tuple literal
    Tuple(Vec<IR>),

    /// Dict literal
    Dict(
        #[serde(
            serialize_with = "serialize_dict_entries",
            deserialize_with = "deserialize_dict_entries"
        )]
        Vec<(IR, IR)>,
    ),

    /// Set literal
    Set(Vec<IR>),

    /// Function call
    Call {
        func: Box<IR>,
        args: Vec<IR>,
        kwargs: IndexMap<String, IR>,
        start_byte: usize,
        end_byte: usize,
        /// Tail call flag (set by semantic analyzer for TCO)
        tail: bool,
    },

    /// Pattern matching variants
    PatternLiteral(Box<IR>),
    PatternVar(String),
    PatternWildcard,
    PatternOr(Vec<IR>),
    PatternTuple(Vec<IR>),
    PatternStruct {
        name: String,
        fields: Vec<String>,
    },
    PatternEnum {
        enum_name: String,
        variant_name: String,
    },

    /// Slice object (for array/list slicing)
    Slice {
        start: Box<IR>,
        stop: Box<IR>,
        step: Box<IR>,
    },

    /// Broadcasting operation (vectorized operations)
    Broadcast {
        target: Option<Box<IR>>,
        operator: Box<IR>,
        operand: Option<Box<IR>>,
        broadcast_type: BroadcastType,
    },
}

/// Custom serializer for Dict entries (non-string keys)
fn serialize_dict_entries<S>(entries: &[(IR, IR)], serializer: S) -> Result<S::Ok, S::Error>
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
fn deserialize_dict_entries<'de, D>(deserializer: D) -> Result<Vec<(IR, IR)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let pairs: Vec<Vec<IR>> = Deserialize::deserialize(deserializer)?;
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

impl IR {
    /// Create an operation node with default metadata
    pub fn op(opcode: IROpCode, args: Vec<IR>) -> Self {
        Self::Op {
            opcode,
            args,
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        }
    }

    /// Create a function call
    pub fn call(func: IR, args: Vec<IR>) -> Self {
        Self::Call {
            func: Box::new(func),
            args,
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        }
    }

    /// Create an operation with kwargs
    pub fn op_with_kwargs(opcode: IROpCode, args: Vec<IR>, kwargs: IndexMap<String, IR>) -> Self {
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
    pub fn op_with_pos(opcode: IROpCode, args: Vec<IR>, start_byte: usize, end_byte: usize) -> Self {
        Self::Op {
            opcode,
            args,
            kwargs: IndexMap::new(),
            tail: false,
            start_byte,
            end_byte,
        }
    }

    /// Check if this is a literal value
    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            IR::Int(_)
                | IR::Float(_)
                | IR::String(_)
                | IR::Bytes(_)
                | IR::Bool(_)
                | IR::None
                | IR::Decimal(_)
                | IR::Imaginary(_)
        )
    }

    /// Check if this is an operation
    pub fn is_op(&self) -> bool {
        matches!(self, IR::Op { .. })
    }

    /// Get opcode if this is an operation
    pub fn opcode(&self) -> Option<IROpCode> {
        match self {
            IR::Op { opcode, .. } => Some(*opcode),
            _ => None,
        }
    }

    /// Get args if this is an operation
    pub fn args(&self) -> Option<&[IR]> {
        match self {
            IR::Op { args, .. } => Some(args),
            _ => None,
        }
    }

    /// Check if this is a tail call
    pub fn is_tail(&self) -> bool {
        match self {
            IR::Op { tail, .. } => *tail,
            _ => false,
        }
    }

    /// Mark this operation as a tail call
    pub fn mark_tail(mut self) -> Self {
        if let IR::Op { ref mut tail, .. } = self {
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

    /// Compact JSON representation - debug-friendly, no serde tags.
    ///
    /// Primitives aplaties (42, "hello", true, null), Op/Call compacts
    /// with args/kwargs always present but tail/pos omitted when default.
    pub fn to_compact_value(&self) -> serde_json::Value {
        use serde_json::{Map, Value, json};

        match self {
            IR::Int(n) => json!(n),
            IR::Float(f) => json!(f),
            IR::String(s) => json!(s),
            IR::Bytes(v) => {
                let hex: String = v.iter().map(|b| format!("{:02x}", b)).collect();
                json!({"bytes": hex})
            }
            IR::Bool(b) => json!(b),
            IR::None => Value::Null,
            IR::Decimal(s) => json!({"decimal": s}),
            IR::Imaginary(s) => json!({"imaginary": s}),

            IR::Ref(name, _, _) => json!({"ref": name}),
            IR::Identifier(name) => json!({"id": name}),

            IR::Tuple(items) => Value::Array(items.iter().map(|i| i.to_compact_value()).collect()),
            IR::List(items) => {
                json!({"list": items.iter().map(|i| i.to_compact_value()).collect::<Vec<_>>()})
            }
            IR::Program(items) => {
                json!({"program": items.iter().map(|i| i.to_compact_value()).collect::<Vec<_>>()})
            }
            IR::Set(items) => {
                json!({"set": items.iter().map(|i| i.to_compact_value()).collect::<Vec<_>>()})
            }
            IR::Dict(entries) => {
                let pairs: Vec<Value> = entries
                    .iter()
                    .map(|(k, v)| json!([k.to_compact_value(), v.to_compact_value()]))
                    .collect();
                json!({"dict": pairs})
            }

            IR::PatternWildcard => json!("_"),
            IR::PatternVar(name) => json!({"pat_var": name}),
            IR::PatternLiteral(inner) => json!({"pat_lit": inner.to_compact_value()}),
            IR::PatternOr(pats) => {
                json!({"pat_or": pats.iter().map(|p| p.to_compact_value()).collect::<Vec<_>>()})
            }
            IR::PatternTuple(pats) => {
                json!({"pat_tuple": pats.iter().map(|p| p.to_compact_value()).collect::<Vec<_>>()})
            }
            IR::PatternStruct { name, fields } => {
                json!({"pat_struct": {"name": name, "fields": fields}})
            }
            IR::PatternEnum {
                enum_name,
                variant_name,
            } => {
                json!({"pat_enum": {"enum": enum_name, "variant": variant_name}})
            }

            IR::Slice { start, stop, step } => {
                json!({"slice": [start.to_compact_value(), stop.to_compact_value(), step.to_compact_value()]})
            }

            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => {
                // Opcode name via serde (PascalCase)
                let opcode_name = serde_json::to_value(opcode).unwrap_or_else(|_| json!(format!("{:?}", opcode)));

                let mut map = Map::new();
                map.insert("op".into(), opcode_name);
                map.insert(
                    "args".into(),
                    Value::Array(args.iter().map(|a| a.to_compact_value()).collect()),
                );
                map.insert(
                    "kwargs".into(),
                    Value::Object(kwargs.iter().map(|(k, v)| (k.clone(), v.to_compact_value())).collect()),
                );
                if *tail {
                    map.insert("tail".into(), json!(true));
                }
                if *start_byte != 0 || *end_byte != 0 {
                    map.insert("pos".into(), json!([start_byte, end_byte]));
                }
                Value::Object(map)
            }

            IR::Call {
                func,
                args,
                kwargs,
                tail,
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
                    Value::Object(kwargs.iter().map(|(k, v)| (k.clone(), v.to_compact_value())).collect()),
                );
                if *tail {
                    map.insert("tail".into(), json!(true));
                }
                if *start_byte != 0 || *end_byte != 0 {
                    map.insert("pos".into(), json!([start_byte, end_byte]));
                }
                Value::Object(map)
            }

            IR::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => {
                let type_name =
                    serde_json::to_value(broadcast_type).unwrap_or_else(|_| json!(format!("{:?}", broadcast_type)));

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
        serde_json::to_string_pretty(&self.to_compact_value()).unwrap_or_else(|_| self.to_compact_json())
    }
}

/// Conversion helpers for standard type compatibility
impl From<i64> for IR {
    fn from(val: i64) -> Self {
        IR::Int(val)
    }
}

impl From<f64> for IR {
    fn from(val: f64) -> Self {
        IR::Float(val)
    }
}

impl From<bool> for IR {
    fn from(val: bool) -> Self {
        IR::Bool(val)
    }
}

impl From<String> for IR {
    fn from(val: String) -> Self {
        IR::String(val)
    }
}

impl From<&str> for IR {
    fn from(val: &str) -> Self {
        IR::String(val.to_string())
    }
}

impl From<Vec<u8>> for IR {
    fn from(val: Vec<u8>) -> Self {
        IR::Bytes(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_creation() {
        assert!(IR::Int(42).is_literal());
        assert!(IR::Float(3.14).is_literal());
        assert!(IR::String("hello".into()).is_literal());
        assert!(IR::Bytes(vec![1, 2, 3]).is_literal());
        assert!(IR::Bool(true).is_literal());
        assert!(IR::None.is_literal());
    }

    #[test]
    fn test_op_creation() {
        let op = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        assert!(op.is_op());
        assert_eq!(op.opcode(), Some(IROpCode::Add));
        assert_eq!(op.args().unwrap().len(), 2);
    }

    #[test]
    fn test_tail_marking() {
        let op = IR::op(IROpCode::Call, vec![]).mark_tail();
        assert!(op.is_tail());
    }

    #[test]
    fn test_conversions() {
        let _: IR = 42.into();
        let _: IR = 3.14.into();
        let _: IR = true.into();
        let _: IR = "test".into();
    }

    // --- Compact JSON tests ---

    #[test]
    fn test_compact_primitives_flat() {
        assert_eq!(IR::Int(42).to_compact_json(), "42");
        assert_eq!(IR::Float(3.14).to_compact_json(), "3.14");
        assert_eq!(IR::Bool(true).to_compact_json(), "true");
        assert_eq!(IR::None.to_compact_json(), "null");
        assert_eq!(IR::String("hello".into()).to_compact_json(), r#""hello""#);
    }

    #[test]
    fn test_compact_ref_and_identifier() {
        let v: serde_json::Value = serde_json::from_str(&IR::Ref("x".into(), -1, -1).to_compact_json()).unwrap();
        assert_eq!(v["ref"], "x");

        let v: serde_json::Value = serde_json::from_str(&IR::Identifier("y".into()).to_compact_json()).unwrap();
        assert_eq!(v["id"], "y");
    }

    #[test]
    fn test_compact_op_args_kwargs_always_present() {
        let op = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
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
        let op = IR::op(IROpCode::Nop, vec![]);
        let v: serde_json::Value = serde_json::from_str(&op.to_compact_json()).unwrap();

        assert_eq!(v["args"], serde_json::json!([]));
        assert_eq!(v["kwargs"], serde_json::json!({}));
    }

    #[test]
    fn test_compact_op_tail_shown_when_true() {
        let op = IR::op(IROpCode::Call, vec![]).mark_tail();
        let v: serde_json::Value = serde_json::from_str(&op.to_compact_json()).unwrap();

        assert_eq!(v["tail"], true);
    }

    #[test]
    fn test_compact_op_pos_shown_when_nonzero() {
        let op = IR::op_with_pos(IROpCode::Add, vec![], 10, 25);
        let v: serde_json::Value = serde_json::from_str(&op.to_compact_json()).unwrap();

        assert_eq!(v["pos"], serde_json::json!([10, 25]));
    }

    #[test]
    fn test_compact_call() {
        let call = IR::call(IR::Identifier("print".into()), vec![IR::String("hi".into())]);
        let v: serde_json::Value = serde_json::from_str(&call.to_compact_json()).unwrap();

        assert_eq!(v["call"]["id"], "print");
        assert!(v["args"].is_array());
        assert!(v["kwargs"].is_object());
    }

    #[test]
    fn test_compact_tuple_is_array() {
        let t = IR::Tuple(vec![IR::Int(1), IR::Int(2)]);
        let v: serde_json::Value = serde_json::from_str(&t.to_compact_json()).unwrap();
        assert!(v.is_array());
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_compact_list_tagged() {
        let l = IR::List(vec![IR::Int(1)]);
        let v: serde_json::Value = serde_json::from_str(&l.to_compact_json()).unwrap();
        assert!(v["list"].is_array());
    }

    #[test]
    fn test_compact_patterns() {
        assert_eq!(IR::PatternWildcard.to_compact_json(), r#""_""#);

        let v: serde_json::Value = serde_json::from_str(&IR::PatternVar("x".into()).to_compact_json()).unwrap();
        assert_eq!(v["pat_var"], "x");
    }

    #[test]
    fn test_compact_broadcast() {
        let b = IR::Broadcast {
            target: None,
            operator: Box::new(IR::Identifier("+".into())),
            operand: Some(Box::new(IR::Int(5))),
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
        let op = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        let pretty = op.to_compact_json_pretty();
        assert!(pretty.contains('\n'));
        let _: serde_json::Value = serde_json::from_str(&pretty).unwrap();
    }
}
