// FILE: catnip_rs/src/ir/opcode.rs
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
    // === Shared zone (1..=31) — same values as VMOpCode ===

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

    // === IR-only zone (32..=59) ===

    // -- Control flow (32-39) --
    OpIf = 32,
    OpWhile = 33,
    OpFor = 34,
    OpMatch = 35,
    OpBlock = 36,
    OpReturn = 37,
    OpBreak = 38,
    OpContinue = 39,

    // -- Functions (40-42) --
    Call = 40,
    OpLambda = 41,
    FnDef = 42,

    // -- Assignment (43-44) --
    SetLocals = 43,
    Slice = 44,

    // -- Special arithmetic (45-46) --
    Div = 45,
    TrueDiv = 46,

    // -- Short-circuit logic (47-48) --
    And = 47,
    Or = 48,

    // -- Collections (49-52) --
    ListLiteral = 49,
    TupleLiteral = 50,
    SetLiteral = 51,
    DictLiteral = 52,

    // -- Stack (53-55) --
    Push = 53,
    Pop = 54,
    PushPeek = 55,

    // -- String (56) --
    Fstring = 56,

    // -- Directives (57) --
    Pragma = 57,

    // -- Structures (58-59) --
    OpStruct = 58,
    TraitDef = 59,
}

impl IROpCode {
    /// Highest opcode value. Used for range checks.
    pub const MAX: u8 = IROpCode::TraitDef as u8;

    /// Highest shared opcode value (same values as VMOpCode).
    pub const SHARED_MAX: u8 = IROpCode::Breakpoint as u8;

    /// Convert from u8, returning None for invalid values.
    #[inline]
    pub fn from_u8(v: u8) -> Option<Self> {
        if (1..=Self::MAX).contains(&v) {
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
        // Shared zone boundaries
        assert_eq!(IROpCode::Add as u8, 1);
        assert_eq!(IROpCode::Breakpoint as u8, 31);
        assert_eq!(IROpCode::SHARED_MAX, 31);

        // IR-only zone boundaries
        assert_eq!(IROpCode::OpIf as u8, 32);
        assert_eq!(IROpCode::TraitDef as u8, 59);
        assert_eq!(IROpCode::MAX, 59);
    }

    #[test]
    fn test_from_u8() {
        assert_eq!(IROpCode::from_u8(1), Some(IROpCode::Add));
        assert_eq!(IROpCode::from_u8(31), Some(IROpCode::Breakpoint));
        assert_eq!(IROpCode::from_u8(59), Some(IROpCode::TraitDef));
        assert_eq!(IROpCode::from_u8(0), None);
        assert_eq!(IROpCode::from_u8(60), None);
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
