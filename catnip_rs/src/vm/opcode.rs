// FILE: catnip_rs/src/vm/opcode.rs
//! VM OpCode enumeration - SOURCE OF TRUTH
//!
//! This file defines the VM bytecode opcodes. Python bindings are generated from here.
//! Run `python catnip_rs/gen_opcodes.py` to regenerate Python files.

#![allow(dead_code)]

/// VMOpCode enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum VMOpCode {
    LoadConst = 1,
    LoadLocal = 2,
    StoreLocal = 3,
    LoadScope = 4,
    StoreScope = 5,
    LoadGlobal = 6,
    PopTop = 7,
    DupTop = 8,
    RotTwo = 9,
    Add = 10,
    Sub = 11,
    Mul = 12,
    Div = 13,
    FloorDiv = 14,
    Mod = 15,
    Pow = 16,
    Neg = 17,
    Pos = 18,
    BOr = 19,
    BXor = 20,
    BAnd = 21,
    BNot = 22,
    LShift = 23,
    RShift = 24,
    Lt = 25,
    Le = 26,
    Gt = 27,
    Ge = 28,
    Eq = 29,
    Ne = 30,
    Not = 31,
    Jump = 32,
    JumpIfFalse = 33,
    JumpIfTrue = 34,
    JumpIfFalseOrPop = 35,
    JumpIfTrueOrPop = 36,
    GetIter = 37,
    ForIter = 38,
    ForRangeInt = 39,
    Call = 40,
    CallKw = 41,
    TailCall = 42,
    Return = 43,
    MakeFunction = 44,
    BuildList = 45,
    BuildTuple = 46,
    BuildSet = 47,
    BuildDict = 48,
    BuildSlice = 49,
    GetAttr = 50,
    SetAttr = 51,
    GetItem = 52,
    SetItem = 53,
    PushBlock = 54,
    PopBlock = 55,
    Break = 56,
    Continue = 57,
    Broadcast = 58,
    MatchPattern = 59,
    BindMatch = 60,
    JumpIfNone = 61,
    UnpackSequence = 62,
    UnpackEx = 63,
    Nop = 64,
    Halt = 65,
    NdEmptyTopos = 66,
    NdRecursion = 67,
    NdMap = 68,
    ForRangeStep = 69,
    MatchPatternVM = 70,
    Breakpoint = 71,
    MakeStruct = 72,
    MakeTrait = 73,
}

impl VMOpCode {
    /// Convert from u8, returning None for invalid values.
    #[inline]
    pub fn from_u8(v: u8) -> Option<Self> {
        const MAX_OPCODE: u8 = VMOpCode::MakeTrait as u8;
        if (1..=MAX_OPCODE).contains(&v) {
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
        )
    }

    /// Get stack effect: (pops, pushes). -1 means depends on arg.
    #[inline]
    pub fn stack_effect(self) -> (i8, i8) {
        match self {
            VMOpCode::LoadConst => (0, 1),
            VMOpCode::LoadLocal => (0, 1),
            VMOpCode::StoreLocal => (1, 0),
            VMOpCode::LoadScope => (0, 1),
            VMOpCode::StoreScope => (1, 0),
            VMOpCode::LoadGlobal => (0, 1),
            VMOpCode::PopTop => (1, 0),
            VMOpCode::DupTop => (1, 2),
            VMOpCode::RotTwo => (2, 2),
            VMOpCode::Add => (2, 1),
            VMOpCode::Sub => (2, 1),
            VMOpCode::Mul => (2, 1),
            VMOpCode::Div => (2, 1),
            VMOpCode::FloorDiv => (2, 1),
            VMOpCode::Mod => (2, 1),
            VMOpCode::Pow => (2, 1),
            VMOpCode::Neg => (1, 1),
            VMOpCode::Pos => (1, 1),
            VMOpCode::BOr => (2, 1),
            VMOpCode::BXor => (2, 1),
            VMOpCode::BAnd => (2, 1),
            VMOpCode::BNot => (1, 1),
            VMOpCode::LShift => (2, 1),
            VMOpCode::RShift => (2, 1),
            VMOpCode::Lt => (2, 1),
            VMOpCode::Le => (2, 1),
            VMOpCode::Gt => (2, 1),
            VMOpCode::Ge => (2, 1),
            VMOpCode::Eq => (2, 1),
            VMOpCode::Ne => (2, 1),
            VMOpCode::Not => (1, 1),
            VMOpCode::Jump => (0, 0),
            VMOpCode::JumpIfFalse => (1, 0),
            VMOpCode::JumpIfTrue => (1, 0),
            VMOpCode::JumpIfFalseOrPop => (1, 0),
            VMOpCode::JumpIfTrueOrPop => (1, 0),
            VMOpCode::GetIter => (1, 1),
            VMOpCode::ForIter => (0, 1),
            VMOpCode::ForRangeInt => (0, 0),
            VMOpCode::Call => (-1, 1),
            VMOpCode::CallKw => (-1, 1),
            VMOpCode::TailCall => (-1, 0),
            VMOpCode::Return => (1, 0),
            VMOpCode::MakeFunction => (1, 1),
            VMOpCode::BuildList => (-1, 1),
            VMOpCode::BuildTuple => (-1, 1),
            VMOpCode::BuildSet => (-1, 1),
            VMOpCode::BuildDict => (-1, 1),
            VMOpCode::BuildSlice => (-1, 1),
            VMOpCode::GetAttr => (1, 1),
            VMOpCode::SetAttr => (2, 0),
            VMOpCode::GetItem => (2, 1),
            VMOpCode::SetItem => (3, 0),
            VMOpCode::PushBlock => (0, 0),
            VMOpCode::PopBlock => (0, 0),
            VMOpCode::Break => (0, 0),
            VMOpCode::Continue => (0, 0),
            VMOpCode::Broadcast => (-1, 1),
            VMOpCode::MatchPattern => (1, 1),
            VMOpCode::BindMatch => (1, 0),
            VMOpCode::JumpIfNone => (1, 0),
            VMOpCode::UnpackSequence => (1, -1),
            VMOpCode::UnpackEx => (1, -1),
            VMOpCode::Nop => (0, 0),
            VMOpCode::Halt => (0, 0),
            VMOpCode::NdEmptyTopos => (0, 1),
            VMOpCode::NdRecursion => (-1, 1),
            VMOpCode::NdMap => (-1, 1),
            VMOpCode::ForRangeStep => (0, 0),
            VMOpCode::MatchPatternVM => (1, 1),
            VMOpCode::Breakpoint => (0, 0),
            VMOpCode::MakeStruct => (0, 0),
            VMOpCode::MakeTrait => (0, 0),
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
        // Premier opcode valide = 1
        assert_eq!(VMOpCode::LoadConst as u8, 1);
        // Dernier opcode valide
        let max_opcode = VMOpCode::MakeTrait as u8;
        assert!(max_opcode >= 1);
    }

    #[test]
    fn test_from_u8_roundtrip() {
        // Propriété : from_u8(opcode as u8) == Some(opcode) pour tout opcode valide
        let opcodes = [
            VMOpCode::LoadConst,
            VMOpCode::Add,
            VMOpCode::Jump,
            VMOpCode::Call,
            VMOpCode::Halt,
            VMOpCode::NdMap,
            VMOpCode::ForRangeStep,
            VMOpCode::MatchPatternVM,
            VMOpCode::MakeStruct,
            VMOpCode::MakeTrait,
        ];
        for opcode in opcodes {
            assert_eq!(VMOpCode::from_u8(opcode as u8), Some(opcode));
        }
    }

    #[test]
    fn test_from_u8_invalid() {
        // 0 n'est pas un opcode valide
        assert_eq!(VMOpCode::from_u8(0), None);
        // MAX + 1 n'est pas un opcode valide
        let max_plus_one = (VMOpCode::MakeTrait as u8) + 1;
        assert_eq!(VMOpCode::from_u8(max_plus_one), None);
        // Valeur arbitrairement grande
        assert_eq!(VMOpCode::from_u8(255), None);
    }
}
