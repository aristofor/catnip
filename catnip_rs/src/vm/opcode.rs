// FILE: catnip_rs/src/vm/opcode.rs
//! VM OpCode enumeration - SOURCE OF TRUTH
//!
//! This file defines the VM bytecode opcodes. Python bindings are generated from here.
//! Run `python catnip_rs/gen_opcodes.py` to regenerate Python files.

#![allow(dead_code)]

/// VMOpCode enumeration.
///
/// Layout: shared zone (1..=SHARED_MAX) has identical values to IROpCode,
/// followed by VM-only zone (SHARED_MAX+1..=MAX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum VMOpCode {
    // === Shared zone (1..=31) — same values as IROpCode ===

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

    // === VM-only zone (32..=76) ===

    // -- VM-only arithmetic (32) --
    Div = 32,

    // -- Load/Store (33-38) --
    LoadConst = 33,
    LoadLocal = 34,
    StoreLocal = 35,
    LoadScope = 36,
    StoreScope = 37,
    LoadGlobal = 38,

    // -- Stack (39-41) --
    PopTop = 39,
    DupTop = 40,
    RotTwo = 41,

    // -- Jumps (42-47) --
    Jump = 42,
    JumpIfFalse = 43,
    JumpIfTrue = 44,
    JumpIfFalseOrPop = 45,
    JumpIfTrueOrPop = 46,
    JumpIfNone = 47,

    // -- Iteration (48-51) --
    GetIter = 48,
    ForIter = 49,
    ForRangeInt = 50,
    ForRangeStep = 51,

    // -- Functions (52-57) --
    Call = 52,
    CallKw = 53,
    TailCall = 54,
    Return = 55,
    MakeFunction = 56,
    CallMethod = 57,

    // -- Collections (58-62) --
    BuildList = 58,
    BuildTuple = 59,
    BuildSet = 60,
    BuildDict = 61,
    BuildSlice = 62,

    // -- Blocks (63-66) --
    PushBlock = 63,
    PopBlock = 64,
    Break = 65,
    Continue = 66,

    // -- Match (67-71) --
    MatchPattern = 67,
    BindMatch = 68,
    MatchPatternVM = 69,
    MatchAssignPatternVM = 70,
    MatchFail = 71,

    // -- Unpack (72-73) --
    UnpackSequence = 72,
    UnpackEx = 73,

    // -- Structures (74-75) --
    MakeStruct = 74,
    MakeTrait = 75,

    // -- Halt (76) --
    Halt = 76,
}

impl VMOpCode {
    /// Highest opcode value. Used for range checks and cache invalidation.
    pub const MAX: u8 = VMOpCode::Halt as u8;

    /// Highest shared opcode value (same values as IROpCode).
    pub const SHARED_MAX: u8 = VMOpCode::Breakpoint as u8;

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

    /// Check if this opcode has an argument.
    #[inline]
    pub fn has_arg(self) -> bool {
        matches!(
            self,
            VMOpCode::Broadcast
                | VMOpCode::BuildDict
                | VMOpCode::BuildList
                | VMOpCode::BuildSet
                | VMOpCode::BuildSlice
                | VMOpCode::BuildTuple
                | VMOpCode::Call
                | VMOpCode::CallKw
                | VMOpCode::ForIter
                | VMOpCode::ForRangeInt
                | VMOpCode::GetAttr
                | VMOpCode::Jump
                | VMOpCode::JumpIfFalse
                | VMOpCode::JumpIfFalseOrPop
                | VMOpCode::JumpIfNone
                | VMOpCode::JumpIfTrue
                | VMOpCode::JumpIfTrueOrPop
                | VMOpCode::LoadConst
                | VMOpCode::LoadGlobal
                | VMOpCode::LoadLocal
                | VMOpCode::LoadScope
                | VMOpCode::MakeFunction
                | VMOpCode::MatchPattern
                | VMOpCode::SetAttr
                | VMOpCode::StoreLocal
                | VMOpCode::StoreScope
                | VMOpCode::TailCall
                | VMOpCode::UnpackEx
                | VMOpCode::UnpackSequence
                | VMOpCode::NdRecursion
                | VMOpCode::NdMap
                | VMOpCode::ForRangeStep
                | VMOpCode::MatchPatternVM
                | VMOpCode::MakeStruct
                | VMOpCode::MakeTrait
                | VMOpCode::CallMethod
                | VMOpCode::MatchFail
                | VMOpCode::MatchAssignPatternVM
        )
    }

    /// Get stack effect: (pops, pushes). -1 means depends on arg.
    #[inline]
    pub fn stack_effect(self) -> (i8, i8) {
        match self {
            VMOpCode::Add => (2, 1),
            VMOpCode::Sub => (2, 1),
            VMOpCode::Mul => (2, 1),
            VMOpCode::FloorDiv => (2, 1),
            VMOpCode::Mod => (2, 1),
            VMOpCode::Pow => (2, 1),
            VMOpCode::Neg => (1, 1),
            VMOpCode::Pos => (1, 1),
            VMOpCode::Eq => (2, 1),
            VMOpCode::Ne => (2, 1),
            VMOpCode::Lt => (2, 1),
            VMOpCode::Le => (2, 1),
            VMOpCode::Gt => (2, 1),
            VMOpCode::Ge => (2, 1),
            VMOpCode::Not => (1, 1),
            VMOpCode::BAnd => (2, 1),
            VMOpCode::BOr => (2, 1),
            VMOpCode::BXor => (2, 1),
            VMOpCode::BNot => (1, 1),
            VMOpCode::LShift => (2, 1),
            VMOpCode::RShift => (2, 1),
            VMOpCode::GetAttr => (1, 1),
            VMOpCode::SetAttr => (2, 0),
            VMOpCode::GetItem => (2, 1),
            VMOpCode::SetItem => (3, 0),
            VMOpCode::Broadcast => (-1, 1),
            VMOpCode::NdRecursion => (-1, 1),
            VMOpCode::NdMap => (-1, 1),
            VMOpCode::NdEmptyTopos => (0, 1),
            VMOpCode::Nop => (0, 0),
            VMOpCode::Breakpoint => (0, 0),
            VMOpCode::Div => (2, 1),
            VMOpCode::LoadConst => (0, 1),
            VMOpCode::LoadLocal => (0, 1),
            VMOpCode::StoreLocal => (1, 0),
            VMOpCode::LoadScope => (0, 1),
            VMOpCode::StoreScope => (1, 0),
            VMOpCode::LoadGlobal => (0, 1),
            VMOpCode::PopTop => (1, 0),
            VMOpCode::DupTop => (1, 2),
            VMOpCode::RotTwo => (2, 2),
            VMOpCode::Jump => (0, 0),
            VMOpCode::JumpIfFalse => (1, 0),
            VMOpCode::JumpIfTrue => (1, 0),
            VMOpCode::JumpIfFalseOrPop => (1, 0),
            VMOpCode::JumpIfTrueOrPop => (1, 0),
            VMOpCode::JumpIfNone => (1, 0),
            VMOpCode::GetIter => (1, 1),
            VMOpCode::ForIter => (0, 1),
            VMOpCode::ForRangeInt => (0, 0),
            VMOpCode::ForRangeStep => (0, 0),
            VMOpCode::Call => (-1, 1),
            VMOpCode::CallKw => (-1, 1),
            VMOpCode::TailCall => (-1, 0),
            VMOpCode::Return => (1, 0),
            VMOpCode::MakeFunction => (1, 1),
            VMOpCode::CallMethod => (-1, 1),
            VMOpCode::BuildList => (-1, 1),
            VMOpCode::BuildTuple => (-1, 1),
            VMOpCode::BuildSet => (-1, 1),
            VMOpCode::BuildDict => (-1, 1),
            VMOpCode::BuildSlice => (-1, 1),
            VMOpCode::PushBlock => (0, 0),
            VMOpCode::PopBlock => (0, 0),
            VMOpCode::Break => (0, 0),
            VMOpCode::Continue => (0, 0),
            VMOpCode::MatchPattern => (1, 1),
            VMOpCode::BindMatch => (1, 0),
            VMOpCode::MatchPatternVM => (1, 1),
            VMOpCode::MatchAssignPatternVM => (1, 1),
            VMOpCode::MatchFail => (0, 0),
            VMOpCode::UnpackSequence => (1, -1),
            VMOpCode::UnpackEx => (1, -1),
            VMOpCode::MakeStruct => (0, 0),
            VMOpCode::MakeTrait => (0, 0),
            VMOpCode::Halt => (0, 0),
        }
    }
}

impl std::fmt::Display for VMOpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Instruction: opcode + optional argument
#[derive(Debug, Clone, Copy)]
pub struct Instruction {
    pub op: VMOpCode,
    pub arg: u32,
}

impl Instruction {
    #[inline]
    pub fn new(op: VMOpCode, arg: u32) -> Self {
        Self { op, arg }
    }

    #[inline]
    pub fn simple(op: VMOpCode) -> Self {
        Self { op, arg: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_bounds() {
        // Shared zone starts at 1
        assert_eq!(VMOpCode::Add as u8, 1);
        assert_eq!(VMOpCode::Breakpoint as u8, 31);
        assert_eq!(VMOpCode::SHARED_MAX, 31);

        // VM-only zone
        assert_eq!(VMOpCode::Div as u8, 32);
        assert_eq!(VMOpCode::Halt as u8, 76);
        assert_eq!(VMOpCode::MAX, 76);
    }

    #[test]
    fn test_from_u8_roundtrip() {
        let opcodes = [
            VMOpCode::Add,
            VMOpCode::Breakpoint,
            VMOpCode::Div,
            VMOpCode::Jump,
            VMOpCode::Call,
            VMOpCode::Halt,
            VMOpCode::NdMap,
            VMOpCode::ForRangeStep,
            VMOpCode::MatchPatternVM,
            VMOpCode::MakeStruct,
            VMOpCode::MakeTrait,
            VMOpCode::CallMethod,
            VMOpCode::MatchFail,
            VMOpCode::MatchAssignPatternVM,
        ];
        for opcode in opcodes {
            assert_eq!(VMOpCode::from_u8(opcode as u8), Some(opcode));
        }
    }

    #[test]
    fn test_from_u8_invalid() {
        assert_eq!(VMOpCode::from_u8(0), None);
        assert_eq!(VMOpCode::from_u8(VMOpCode::MAX + 1), None);
        assert_eq!(VMOpCode::from_u8(255), None);
    }
}
