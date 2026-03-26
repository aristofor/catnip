// FILE: catnip_core/src/ir/opcode.rs
//! IR OpCode enumeration - SOURCE OF TRUTH
//!
//! This file defines the IR opcodes. Python bindings are generated from here.
//! Run `python catnip_rs/gen_opcodes.py` to regenerate Python files.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// IROpCode enumeration.
///
/// Layout: shared zone (1..=SHARED_MAX) has identical values to VMOpCode,
/// followed by IR-only zone (SHARED_MAX+1..=MAX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[repr(u8)]
pub enum IROpCode {
    // === Shared zone (1..=31) - same values as VMOpCode ===

    // -- Arithmetic (1-8) --
    Add = 1,
    Sub = 2,
    Mul = 3,
    FloorDiv = 4,
    Mod = 5,
    Pow = 6,
    Neg = 7,
    Pos = 8,

    // -- Comparison (9-14) --
    Eq = 9,
    Ne = 10,
    Lt = 11,
    Le = 12,
    Gt = 13,
    Ge = 14,

    // -- Unary logic (15) --
    Not = 15,

    // -- Bitwise (16-21) --
    BAnd = 16,
    BOr = 17,
    BXor = 18,
    BNot = 19,
    LShift = 20,
    RShift = 21,

    // -- Access (22-25) --
    GetAttr = 22,
    SetAttr = 23,
    GetItem = 24,
    SetItem = 25,

    // -- Broadcasting & ND (26-29) --
    Broadcast = 26,
    NdRecursion = 27,
    NdMap = 28,
    NdEmptyTopos = 29,

    // -- Meta (30-31) --
    Nop = 30,
    Breakpoint = 31,

    // === IR-only zone (32..=65) ===

    // -- Arithmetic (extends shared 1-8) --
    Div = 32,
    TrueDiv = 33,

    // -- Logic (extends shared 15: Not) --
    And = 34,
    Or = 35,

    // -- Comparison (extends shared 9-14) --
    In = 36,
    NotIn = 37,
    Is = 38,
    IsNot = 39,
    NullCoalesce = 40,

    // -- Collections (41-44) --
    ListLiteral = 41,
    TupleLiteral = 42,
    SetLiteral = 43,
    DictLiteral = 44,

    // -- Stack (45-47) --
    Push = 45,
    Pop = 46,
    PushPeek = 47,

    // -- String (48) --
    Fstring = 48,

    // -- Access (extends shared 22-25) --
    Slice = 49,

    // -- Control flow (50-57) --
    OpIf = 50,
    OpWhile = 51,
    OpFor = 52,
    OpMatch = 53,
    OpBlock = 54,
    OpReturn = 55,
    OpBreak = 56,
    OpContinue = 57,

    // -- Functions (58-60) --
    Call = 58,
    OpLambda = 59,
    FnDef = 60,

    // -- Assignment (61) --
    SetLocals = 61,

    // -- Structures (62-63) --
    OpStruct = 62,
    TraitDef = 63,

    // -- Directives (64) --
    Pragma = 64,

    // -- Intrinsics (65-67) --
    TypeOf = 65,
    Globals = 66,
    Locals = 67,
}

impl IROpCode {
    /// Highest opcode value. Used for range checks.
    pub const MAX: u8 = IROpCode::Locals as u8;

    /// Highest shared opcode value (same values as VMOpCode).
    pub const SHARED_MAX: u8 = IROpCode::Breakpoint as u8;

    /// Convert from u8, returning None for invalid values.
    #[inline]
    pub fn from_u8(v: u8) -> Option<Self> {
        if (1..=Self::MAX).contains(&v) {
            // SAFETY: We verified the range matches our enum
            Some(unsafe { std::mem::transmute::<u8, Self>(v) })
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
        // Shared zone boundaries
        assert_eq!(IROpCode::Add as u8, 1);
        assert_eq!(IROpCode::Breakpoint as u8, 31);
        assert_eq!(IROpCode::SHARED_MAX, 31);

        // IR-only zone boundaries
        assert_eq!(IROpCode::Div as u8, 32);
        assert_eq!(IROpCode::Pragma as u8, 64);
        assert_eq!(IROpCode::TypeOf as u8, 65);
        assert_eq!(IROpCode::Globals as u8, 66);
        assert_eq!(IROpCode::Locals as u8, 67);
        assert_eq!(IROpCode::MAX, 67);
    }

    #[test]
    fn test_from_u8() {
        assert_eq!(IROpCode::from_u8(1), Some(IROpCode::Add));
        assert_eq!(IROpCode::from_u8(31), Some(IROpCode::Breakpoint));
        assert_eq!(IROpCode::from_u8(63), Some(IROpCode::TraitDef));
        assert_eq!(IROpCode::from_u8(37), Some(IROpCode::NotIn));
        assert_eq!(IROpCode::from_u8(0), None);
        assert_eq!(IROpCode::from_u8(64), Some(IROpCode::Pragma));
        assert_eq!(IROpCode::from_u8(65), Some(IROpCode::TypeOf));
        assert_eq!(IROpCode::from_u8(66), Some(IROpCode::Globals));
        assert_eq!(IROpCode::from_u8(67), Some(IROpCode::Locals));
        assert_eq!(IROpCode::from_u8(68), None);
    }

    #[test]
    fn test_shared_opcodes_bijection() {
        use crate::vm::opcode::VMOpCode;

        assert_eq!(IROpCode::Add as u8, VMOpCode::Add as u8);
        assert_eq!(IROpCode::Sub as u8, VMOpCode::Sub as u8);
        assert_eq!(IROpCode::Mul as u8, VMOpCode::Mul as u8);
        assert_eq!(IROpCode::FloorDiv as u8, VMOpCode::FloorDiv as u8);
        assert_eq!(IROpCode::Mod as u8, VMOpCode::Mod as u8);
        assert_eq!(IROpCode::Pow as u8, VMOpCode::Pow as u8);
        assert_eq!(IROpCode::Neg as u8, VMOpCode::Neg as u8);
        assert_eq!(IROpCode::Pos as u8, VMOpCode::Pos as u8);
        assert_eq!(IROpCode::Eq as u8, VMOpCode::Eq as u8);
        assert_eq!(IROpCode::Ne as u8, VMOpCode::Ne as u8);
        assert_eq!(IROpCode::Lt as u8, VMOpCode::Lt as u8);
        assert_eq!(IROpCode::Le as u8, VMOpCode::Le as u8);
        assert_eq!(IROpCode::Gt as u8, VMOpCode::Gt as u8);
        assert_eq!(IROpCode::Ge as u8, VMOpCode::Ge as u8);
        assert_eq!(IROpCode::Not as u8, VMOpCode::Not as u8);
        assert_eq!(IROpCode::BAnd as u8, VMOpCode::BAnd as u8);
        assert_eq!(IROpCode::BOr as u8, VMOpCode::BOr as u8);
        assert_eq!(IROpCode::BXor as u8, VMOpCode::BXor as u8);
        assert_eq!(IROpCode::BNot as u8, VMOpCode::BNot as u8);
        assert_eq!(IROpCode::LShift as u8, VMOpCode::LShift as u8);
        assert_eq!(IROpCode::RShift as u8, VMOpCode::RShift as u8);
        assert_eq!(IROpCode::GetAttr as u8, VMOpCode::GetAttr as u8);
        assert_eq!(IROpCode::SetAttr as u8, VMOpCode::SetAttr as u8);
        assert_eq!(IROpCode::GetItem as u8, VMOpCode::GetItem as u8);
        assert_eq!(IROpCode::SetItem as u8, VMOpCode::SetItem as u8);
        assert_eq!(IROpCode::Broadcast as u8, VMOpCode::Broadcast as u8);
        assert_eq!(IROpCode::NdRecursion as u8, VMOpCode::NdRecursion as u8);
        assert_eq!(IROpCode::NdMap as u8, VMOpCode::NdMap as u8);
        assert_eq!(IROpCode::NdEmptyTopos as u8, VMOpCode::NdEmptyTopos as u8);
        assert_eq!(IROpCode::Nop as u8, VMOpCode::Nop as u8);
        assert_eq!(IROpCode::Breakpoint as u8, VMOpCode::Breakpoint as u8);
    }
}
