// FILE: catnip_rs/src/semantic/opcode.rs
//! OpCode enumeration for efficient integer-based operation representation.
//!
//! Mirror of IROpCode with same values. Uses SCREAMING_SNAKE_CASE naming.

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum OpCode {
    // === Shared zone (1..=31) — same values as IROpCode/VMOpCode ===

    // -- Arithmetic (1-8) --
    ADD = 1,
    SUB = 2,
    MUL = 3,
    FLOORDIV = 4,
    MOD = 5,
    POW = 6,
    NEG = 7,
    POS = 8,

    // -- Comparison (9-14) --
    EQ = 9,
    NE = 10,
    LT = 11,
    LE = 12,
    GT = 13,
    GE = 14,

    // -- Unary logic (15) --
    NOT = 15,

    // -- Bitwise (16-21) --
    BAND = 16,
    BOR = 17,
    BXOR = 18,
    BNOT = 19,
    LSHIFT = 20,
    RSHIFT = 21,

    // -- Access (22-25) --
    GETATTR = 22,
    SETATTR = 23,
    GETITEM = 24,
    SETITEM = 25,

    // -- Broadcasting & ND (26-29) --
    BROADCAST = 26,
    ND_RECURSION = 27,
    ND_MAP = 28,
    ND_EMPTY_TOPOS = 29,

    // -- Meta (30-31) --
    NOP = 30,
    BREAKPOINT = 31,

    // === IR-only zone (32..=59) ===

    // -- Control flow (32-39) --
    OP_IF = 32,
    OP_WHILE = 33,
    OP_FOR = 34,
    OP_MATCH = 35,
    OP_BLOCK = 36,
    OP_RETURN = 37,
    OP_BREAK = 38,
    OP_CONTINUE = 39,

    // -- Functions (40-42) --
    CALL = 40,
    OP_LAMBDA = 41,
    FN_DEF = 42,

    // -- Assignment (43-44) --
    SET_LOCALS = 43,
    SLICE = 44,

    // -- Special arithmetic (45-46) --
    DIV = 45,
    TRUEDIV = 46,

    // -- Short-circuit logic (47-48) --
    AND = 47,
    OR = 48,

    // -- Collections (49-52) --
    LIST_LITERAL = 49,
    TUPLE_LITERAL = 50,
    SET_LITERAL = 51,
    DICT_LITERAL = 52,

    // -- Stack (53-55) --
    PUSH = 53,
    POP = 54,
    PUSH_PEEK = 55,

    // -- String (56) --
    FSTRING = 56,

    // -- Directives (57) --
    PRAGMA = 57,

    // -- Structures (58-59) --
    OP_STRUCT = 58,
    TRAIT_DEF = 59,
}

impl OpCode {
    /// Highest opcode value.
    pub const MAX: i32 = OpCode::TRAIT_DEF as i32;

    /// Check if this opcode is a control flow operation where arguments should not be evaluated
    pub fn is_control_flow(self) -> bool {
        matches!(
            self,
            OpCode::OP_IF
                | OpCode::OP_WHILE
                | OpCode::OP_FOR
                | OpCode::OP_MATCH
                | OpCode::OP_BLOCK
                | OpCode::CALL
                | OpCode::OP_LAMBDA
                | OpCode::FN_DEF
                | OpCode::SET_LOCALS
                | OpCode::ND_RECURSION
                | OpCode::ND_MAP
                | OpCode::OP_STRUCT
                | OpCode::TRAIT_DEF
        )
    }

    /// Check if this opcode is commutative (a op b == b op a)
    pub fn is_commutative(self) -> bool {
        matches!(
            self,
            OpCode::ADD
                | OpCode::MUL
                | OpCode::EQ
                | OpCode::NE
                | OpCode::BAND
                | OpCode::BOR
                | OpCode::BXOR
                | OpCode::AND
                | OpCode::OR
        )
    }

    /// Check if this opcode is associative ((a op b) op c == a op (b op c))
    pub fn is_associative(self) -> bool {
        matches!(
            self,
            OpCode::ADD
                | OpCode::MUL
                | OpCode::BAND
                | OpCode::BOR
                | OpCode::BXOR
                | OpCode::AND
                | OpCode::OR
        )
    }

    /// Try to convert from an i32 value
    pub fn from_i32(value: i32) -> Option<Self> {
        if (1..=Self::MAX).contains(&value) {
            // SAFETY: We've checked the value is in valid range
            Some(unsafe { std::mem::transmute(value) })
        } else {
            None
        }
    }
}

impl From<OpCode> for i32 {
    fn from(op: OpCode) -> Self {
        op as i32
    }
}
