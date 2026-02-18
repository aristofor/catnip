// FILE: catnip_rs/src/semantic/opcode.rs
//! OpCode enumeration for efficient integer-based operation representation.
//!
//! Port of catnip/semantic/opcode.py

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum OpCode {
    // Special operations
    NOP = 1,

    // Control flow operations (prefixed with OP_ to avoid keyword conflicts)
    OP_IF = 2,
    OP_WHILE = 3,
    OP_FOR = 4,
    OP_MATCH = 5,
    OP_BLOCK = 6,
    OP_RETURN = 7,
    OP_BREAK = 8,
    OP_CONTINUE = 9,

    // Function operations
    CALL = 10,
    OP_LAMBDA = 11,
    FN_DEF = 12,

    // Variable operations
    SET_LOCALS = 13,
    GETATTR = 14,
    SETATTR = 15,
    GETITEM = 16,
    SETITEM = 17,
    SLICE = 18,

    // Arithmetic operations
    ADD = 19,
    SUB = 20,
    MUL = 21,
    DIV = 22,
    TRUEDIV = 23,
    FLOORDIV = 24,
    MOD = 25,
    POW = 26,
    NEG = 27,
    POS = 28,

    // Comparison operations
    EQ = 29,
    NE = 30,
    LT = 31,
    LE = 32,
    GT = 33,
    GE = 34,

    // Logical operations
    AND = 35,
    OR = 36,
    NOT = 37,

    // Bitwise operations
    BAND = 38,
    BOR = 39,
    BXOR = 40,
    BNOT = 41,
    LSHIFT = 42,
    RSHIFT = 43,

    // Collection operations
    BROADCAST = 44,
    LIST_LITERAL = 45,
    TUPLE_LITERAL = 46,
    SET_LITERAL = 47,
    DICT_LITERAL = 48,

    // Stack operations
    PUSH = 49,
    POP = 50,
    PUSH_PEEK = 51,

    // String operations
    FSTRING = 52,

    // Directives
    PRAGMA = 53,

    // ND operations (non-deterministic recursion)
    ND_RECURSION = 54,
    ND_MAP = 55,
    ND_EMPTY_TOPOS = 56,
}

impl OpCode {
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
        if (1..=56).contains(&value) {
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
