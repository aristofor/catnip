// FILE: catnip_core/src/semantic/opcode.rs
//! OpCode enumeration for efficient integer-based operation representation.
//!
//! Mirror of IROpCode with same values. Uses SCREAMING_SNAKE_CASE naming.

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum OpCode {
    // === Shared zone (1..=31) - same values as IROpCode/VMOpCode ===

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

    // === IR-only zone (32..=65) ===

    // -- Arithmetic (extends shared 1-8) --
    DIV = 32,
    TRUEDIV = 33,

    // -- Logic (extends shared 15: Not) --
    AND = 34,
    OR = 35,

    // -- Comparison (extends shared 9-14) --
    IN = 36,
    NOT_IN = 37,
    IS = 38,
    IS_NOT = 39,
    NULL_COALESCE = 40,

    // -- Collections (41-44) --
    LIST_LITERAL = 41,
    TUPLE_LITERAL = 42,
    SET_LITERAL = 43,
    DICT_LITERAL = 44,

    // -- Stack (45-47) --
    PUSH = 45,
    POP = 46,
    PUSH_PEEK = 47,

    // -- String (48) --
    FSTRING = 48,

    // -- Access (extends shared 22-25) --
    SLICE = 49,

    // -- Control flow (50-57) --
    OP_IF = 50,
    OP_WHILE = 51,
    OP_FOR = 52,
    OP_MATCH = 53,
    OP_BLOCK = 54,
    OP_RETURN = 55,
    OP_BREAK = 56,
    OP_CONTINUE = 57,

    // -- Functions (58-60) --
    CALL = 58,
    OP_LAMBDA = 59,
    FN_DEF = 60,

    // -- Assignment (61) --
    SET_LOCALS = 61,

    // -- Structures (62-63) --
    OP_STRUCT = 62,
    TRAIT_DEF = 63,

    // -- Directives (64) --
    PRAGMA = 64,

    // -- Intrinsics (65) --
    TYPE_OF = 65,
}

impl OpCode {
    /// Highest opcode value.
    pub const MAX: i32 = OpCode::TYPE_OF as i32;

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
            OpCode::ADD | OpCode::MUL | OpCode::BAND | OpCode::BOR | OpCode::BXOR | OpCode::AND | OpCode::OR
        )
    }

    /// Try to convert from an i32 value
    pub fn from_i32(value: i32) -> Option<Self> {
        if (1..=Self::MAX).contains(&value) {
            // SAFETY: We've checked the value is in valid range
            Some(unsafe { std::mem::transmute::<i32, Self>(value) })
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
