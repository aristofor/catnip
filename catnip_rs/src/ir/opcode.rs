// FILE: catnip_rs/src/ir/opcode.rs
//! IR OpCode enumeration - SOURCE OF TRUTH
//!
//! This file defines the IR opcodes. Python bindings are generated from here.
//! Run `python catnip_rs/gen_opcodes.py` to regenerate Python files.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// IROpCode enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[repr(u8)]
pub enum IROpCode {
    Nop = 1,
    OpIf = 2,
    OpWhile = 3,
    OpFor = 4,
    OpMatch = 5,
    OpBlock = 6,
    OpReturn = 7,
    OpBreak = 8,
    OpContinue = 9,
    Call = 10,
    OpLambda = 11,
    FnDef = 12,
    SetLocals = 13,
    GetAttr = 14,
    SetAttr = 15,
    GetItem = 16,
    SetItem = 17,
    Slice = 18,
    Add = 19,
    Sub = 20,
    Mul = 21,
    Div = 22,
    TrueDiv = 23,
    FloorDiv = 24,
    Mod = 25,
    Pow = 26,
    Neg = 27,
    Pos = 28,
    Eq = 29,
    Ne = 30,
    Lt = 31,
    Le = 32,
    Gt = 33,
    Ge = 34,
    And = 35,
    Or = 36,
    Not = 37,
    BAnd = 38,
    BOr = 39,
    BXor = 40,
    BNot = 41,
    LShift = 42,
    RShift = 43,
    Broadcast = 44,
    ListLiteral = 45,
    TupleLiteral = 46,
    SetLiteral = 47,
    DictLiteral = 48,
    Push = 49,
    Pop = 50,
    PushPeek = 51,
    Fstring = 52,
    Pragma = 53,
    NdRecursion = 54,
    NdMap = 55,
    NdEmptyTopos = 56,
    Breakpoint = 57,
    OpStruct = 58,
    TraitDef = 59,
}

impl IROpCode {
    /// Convert from u8, returning None for invalid values.
    #[inline]
    pub fn from_u8(v: u8) -> Option<Self> {
        const MAX_OPCODE: u8 = IROpCode::TraitDef as u8;
        if (1..=MAX_OPCODE).contains(&v) {
            // SAFETY: We verified the range matches our enum
            Some(unsafe { std::mem::transmute(v) })
        } else {
            None
        }
    }

    /// Serialize to JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize from JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

impl std::fmt::Display for IROpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_values() {
        assert_eq!(IROpCode::Nop as u8, 1);
        assert_eq!(IROpCode::NdEmptyTopos as u8, 56);
        assert_eq!(IROpCode::Breakpoint as u8, 57);
        assert_eq!(IROpCode::OpStruct as u8, 58);
        assert_eq!(IROpCode::TraitDef as u8, 59);
    }

    #[test]
    fn test_from_u8() {
        assert_eq!(IROpCode::from_u8(1), Some(IROpCode::Nop));
        assert_eq!(IROpCode::from_u8(57), Some(IROpCode::Breakpoint));
        assert_eq!(IROpCode::from_u8(58), Some(IROpCode::OpStruct));
        assert_eq!(IROpCode::from_u8(59), Some(IROpCode::TraitDef));
        assert_eq!(IROpCode::from_u8(0), None);
        assert_eq!(IROpCode::from_u8(60), None);
    }
}
