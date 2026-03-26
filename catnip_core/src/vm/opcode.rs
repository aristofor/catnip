// FILE: catnip_core/src/vm/opcode.rs
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
    // === Shared zone (1..=31) - same values as IROpCode ===

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

    // === VM-only zone (32..=86) ===

    // -- Arithmetic (extends shared 1-8) --
    Div = 32,

    // -- Comparison (extends shared 9-14) --
    In = 33,
    NotIn = 34,
    Is = 35,
    IsNot = 36,

    // -- Conversion (37-38) --
    ToBool = 37,
    TypeOf = 38,

    // -- Load/Store (39-44) --
    LoadConst = 39,
    LoadLocal = 40,
    StoreLocal = 41,
    LoadScope = 42,
    StoreScope = 43,
    LoadGlobal = 44,

    // -- Stack (45-47) --
    PopTop = 45,
    DupTop = 46,
    RotTwo = 47,

    // -- Jumps (48-54) --
    Jump = 48,
    JumpIfFalse = 49,
    JumpIfTrue = 50,
    JumpIfFalseOrPop = 51,
    JumpIfTrueOrPop = 52,
    JumpIfNone = 53,
    JumpIfNotNoneOrPop = 54,

    // -- Iteration (55-58) --
    GetIter = 55,
    ForIter = 56,
    ForRangeInt = 57,
    ForRangeStep = 58,

    // -- Functions (59-64) --
    Call = 59,
    CallKw = 60,
    CallMethod = 61,
    TailCall = 62,
    Return = 63,
    MakeFunction = 64,

    // -- Collections (65-69) --
    BuildList = 65,
    BuildTuple = 66,
    BuildSet = 67,
    BuildDict = 68,
    BuildSlice = 69,

    // -- String formatting (70-71) --
    FormatValue = 70,
    BuildString = 71,

    // -- Blocks (72-75) --
    PushBlock = 72,
    PopBlock = 73,
    Break = 74,
    Continue = 75,

    // -- Match (76-80) --
    MatchPattern = 76,
    MatchPatternVM = 77,
    MatchAssignPatternVM = 78,
    BindMatch = 79,
    MatchFail = 80,

    // -- Unpack (81-82) --
    UnpackSequence = 81,
    UnpackEx = 82,

    // -- Structures (83-84) --
    MakeStruct = 83,
    MakeTrait = 84,

    // -- Control (85-86) --
    Halt = 85,
    Exit = 86,

    // -- Intrinsics (87-88) --
    Globals = 87,
    Locals = 88,
}

impl VMOpCode {
    /// Highest opcode value. Used for range checks and cache invalidation.
    pub const MAX: u8 = VMOpCode::Locals as u8;

    /// Highest shared opcode value (same values as IROpCode).
    pub const SHARED_MAX: u8 = VMOpCode::Breakpoint as u8;

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

    /// Check if this opcode has an argument.
    #[inline]
    pub fn has_arg(self) -> bool {
        matches!(
            self,
            // Shared
            VMOpCode::Broadcast
                | VMOpCode::GetAttr
                | VMOpCode::SetAttr
                | VMOpCode::NdRecursion
                | VMOpCode::NdMap
                // Load/Store
                | VMOpCode::LoadConst
                | VMOpCode::LoadLocal
                | VMOpCode::StoreLocal
                | VMOpCode::LoadScope
                | VMOpCode::StoreScope
                | VMOpCode::LoadGlobal
                // Jumps
                | VMOpCode::Jump
                | VMOpCode::JumpIfFalse
                | VMOpCode::JumpIfTrue
                | VMOpCode::JumpIfFalseOrPop
                | VMOpCode::JumpIfTrueOrPop
                | VMOpCode::JumpIfNone
                | VMOpCode::JumpIfNotNoneOrPop
                // Iteration
                | VMOpCode::ForIter
                | VMOpCode::ForRangeInt
                | VMOpCode::ForRangeStep
                // Functions
                | VMOpCode::Call
                | VMOpCode::CallKw
                | VMOpCode::CallMethod
                | VMOpCode::TailCall
                | VMOpCode::MakeFunction
                // Collections
                | VMOpCode::BuildList
                | VMOpCode::BuildTuple
                | VMOpCode::BuildSet
                | VMOpCode::BuildDict
                | VMOpCode::BuildSlice
                // Match
                | VMOpCode::MatchPatternVM
                | VMOpCode::MatchAssignPatternVM
                | VMOpCode::MatchFail
                // Unpack
                | VMOpCode::UnpackSequence
                | VMOpCode::UnpackEx
                // Structures
                | VMOpCode::MakeStruct
                | VMOpCode::MakeTrait
                // Control
                | VMOpCode::Exit
                // String formatting
                | VMOpCode::FormatValue
                | VMOpCode::BuildString
        )
    }

    /// Get stack effect: (pops, pushes). -1 means depends on arg.
    #[inline]
    pub fn stack_effect(self) -> (i8, i8) {
        match self {
            // Shared: Arithmetic
            VMOpCode::Add => (2, 1),
            VMOpCode::Sub => (2, 1),
            VMOpCode::Mul => (2, 1),
            VMOpCode::FloorDiv => (2, 1),
            VMOpCode::Mod => (2, 1),
            VMOpCode::Pow => (2, 1),
            VMOpCode::Neg => (1, 1),
            VMOpCode::Pos => (1, 1),
            // Shared: Comparison
            VMOpCode::Eq => (2, 1),
            VMOpCode::Ne => (2, 1),
            VMOpCode::Lt => (2, 1),
            VMOpCode::Le => (2, 1),
            VMOpCode::Gt => (2, 1),
            VMOpCode::Ge => (2, 1),
            // Shared: Unary logic
            VMOpCode::Not => (1, 1),
            // Shared: Bitwise
            VMOpCode::BAnd => (2, 1),
            VMOpCode::BOr => (2, 1),
            VMOpCode::BXor => (2, 1),
            VMOpCode::BNot => (1, 1),
            VMOpCode::LShift => (2, 1),
            VMOpCode::RShift => (2, 1),
            // Shared: Access
            VMOpCode::GetAttr => (1, 1),
            VMOpCode::SetAttr => (2, 0),
            VMOpCode::GetItem => (2, 1),
            VMOpCode::SetItem => (3, 0),
            // Shared: Broadcasting & ND
            VMOpCode::Broadcast => (-1, 1),
            VMOpCode::NdRecursion => (-1, 1),
            VMOpCode::NdMap => (-1, 1),
            VMOpCode::NdEmptyTopos => (0, 1),
            // Shared: Meta
            VMOpCode::Nop => (0, 0),
            VMOpCode::Breakpoint => (0, 0),
            // VM: Arithmetic
            VMOpCode::Div => (2, 1),
            // VM: Comparison
            VMOpCode::In => (2, 1),
            VMOpCode::NotIn => (2, 1),
            VMOpCode::Is => (2, 1),
            VMOpCode::IsNot => (2, 1),
            // VM: Conversion
            VMOpCode::ToBool => (1, 1),
            VMOpCode::TypeOf => (1, 1),
            // VM: Load/Store
            VMOpCode::LoadConst => (0, 1),
            VMOpCode::LoadLocal => (0, 1),
            VMOpCode::StoreLocal => (1, 0),
            VMOpCode::LoadScope => (0, 1),
            VMOpCode::StoreScope => (1, 0),
            VMOpCode::LoadGlobal => (0, 1),
            // VM: Stack
            VMOpCode::PopTop => (1, 0),
            VMOpCode::DupTop => (1, 2),
            VMOpCode::RotTwo => (2, 2),
            // VM: Jumps
            VMOpCode::Jump => (0, 0),
            VMOpCode::JumpIfFalse => (1, 0),
            VMOpCode::JumpIfTrue => (1, 0),
            VMOpCode::JumpIfFalseOrPop => (1, 0),
            VMOpCode::JumpIfTrueOrPop => (1, 0),
            VMOpCode::JumpIfNone => (1, 0),
            VMOpCode::JumpIfNotNoneOrPop => (1, 0),
            // VM: Iteration
            VMOpCode::GetIter => (1, 1),
            VMOpCode::ForIter => (0, 1),
            VMOpCode::ForRangeInt => (0, 0),
            VMOpCode::ForRangeStep => (0, 0),
            // VM: Functions
            VMOpCode::Call => (-1, 1),
            VMOpCode::CallKw => (-1, 1),
            VMOpCode::CallMethod => (-1, 1),
            VMOpCode::TailCall => (-1, 0),
            VMOpCode::Return => (1, 0),
            VMOpCode::MakeFunction => (1, 1),
            // VM: Collections
            VMOpCode::BuildList => (-1, 1),
            VMOpCode::BuildTuple => (-1, 1),
            VMOpCode::BuildSet => (-1, 1),
            VMOpCode::BuildDict => (-1, 1),
            VMOpCode::BuildSlice => (-1, 1),
            // VM: String formatting
            VMOpCode::FormatValue => (-1, 1), // pops value [+spec], pushes string
            VMOpCode::BuildString => (-1, 1), // pops n strings, pushes concatenated
            // VM: Blocks
            VMOpCode::PushBlock => (0, 0),
            VMOpCode::PopBlock => (0, 0),
            VMOpCode::Break => (0, 0),
            VMOpCode::Continue => (0, 0),
            // VM: Match
            VMOpCode::MatchPattern => (1, 1),
            VMOpCode::MatchPatternVM => (1, 1),
            VMOpCode::MatchAssignPatternVM => (1, 1),
            VMOpCode::BindMatch => (1, 0),
            VMOpCode::MatchFail => (0, 0),
            // VM: Unpack
            VMOpCode::UnpackSequence => (1, -1),
            VMOpCode::UnpackEx => (1, -1),
            // VM: Structures
            VMOpCode::MakeStruct => (0, 0),
            VMOpCode::MakeTrait => (0, 0),
            // VM: Control
            VMOpCode::Halt => (0, 0),
            VMOpCode::Exit => (-1, 0),
            // VM: Intrinsics
            VMOpCode::Globals => (0, 1),
            VMOpCode::Locals => (0, 1),
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
        // Shared zone
        assert_eq!(VMOpCode::Add as u8, 1);
        assert_eq!(VMOpCode::Breakpoint as u8, 31);
        assert_eq!(VMOpCode::SHARED_MAX, 31);

        // VM-only zone boundaries
        assert_eq!(VMOpCode::Div as u8, 32);
        assert_eq!(VMOpCode::TypeOf as u8, 38);
        assert_eq!(VMOpCode::Halt as u8, 85);
        assert_eq!(VMOpCode::Exit as u8, 86);
        assert_eq!(VMOpCode::Globals as u8, 87);
        assert_eq!(VMOpCode::Locals as u8, 88);
        assert_eq!(VMOpCode::MAX, 88);
    }

    #[test]
    fn test_contiguous() {
        // Verify no gaps in the enum (required for transmute safety)
        for i in 1..=VMOpCode::MAX {
            assert!(VMOpCode::from_u8(i).is_some(), "gap at value {i}");
        }
    }

    #[test]
    fn test_from_u8_roundtrip() {
        // Spot-check each category
        let opcodes = [
            VMOpCode::Add,
            VMOpCode::Breakpoint,
            VMOpCode::Div,
            VMOpCode::In,
            VMOpCode::ToBool,
            VMOpCode::LoadConst,
            VMOpCode::Jump,
            VMOpCode::JumpIfNotNoneOrPop,
            VMOpCode::GetIter,
            VMOpCode::ForRangeStep,
            VMOpCode::Call,
            VMOpCode::CallMethod,
            VMOpCode::BuildList,
            VMOpCode::Break,
            VMOpCode::MatchPatternVM,
            VMOpCode::MatchAssignPatternVM,
            VMOpCode::UnpackSequence,
            VMOpCode::MakeStruct,
            VMOpCode::MakeTrait,
            VMOpCode::Halt,
            VMOpCode::Exit,
            VMOpCode::FormatValue,
            VMOpCode::BuildString,
            VMOpCode::TypeOf,
            VMOpCode::Globals,
            VMOpCode::Locals,
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
