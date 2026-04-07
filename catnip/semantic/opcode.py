# FILE: catnip/semantic/opcode.py
# GENERATED FROM catnip_core/src/ir/opcode.rs
# Do not edit the enum manually. Run: python catnip_rs/gen_opcodes.py
"""
OpCode enumeration for efficient integer-based operation representation.

Using integer opcodes instead of strings provides:
- Faster comparison and hashing (integer vs string)
- Lower memory usage
- Better performance in tight loops
- Type safety with enum
"""

from enum import IntEnum


class OpCode(IntEnum):
    """
    Enumeration of all operation codes in Catnip.

    Categories:
    - Other: NOP (no operation)
    - Control flow: IF, WHILE, FOR, MATCH, BLOCK, RETURN, BREAK, CONTINUE
    - Functions: CALL, LAMBDA, FN_DEF
    - Variables: SET_LOCALS, GETATTR
    - Arithmetic: ADD, SUB, MUL, DIV, FLOORDIV, MOD, POW
    - Comparison: EQ, NE, LT, LE, GT, GE
    - Logical: AND, OR, NOT
    - Bitwise: BAND, BOR, BXOR, BNOT, LSHIFT, RSHIFT
    - Collections: BROADCAST
    """

    ADD = 1
    SUB = 2
    MUL = 3
    FLOORDIV = 4
    MOD = 5
    POW = 6
    NEG = 7
    POS = 8
    EQ = 9
    NE = 10
    LT = 11
    LE = 12
    GT = 13
    GE = 14
    NOT = 15
    BAND = 16
    BOR = 17
    BXOR = 18
    BNOT = 19
    LSHIFT = 20
    RSHIFT = 21
    GETATTR = 22
    SETATTR = 23
    GETITEM = 24
    SETITEM = 25
    BROADCAST = 26
    ND_RECURSION = 27
    ND_MAP = 28
    ND_EMPTY_TOPOS = 29
    NOP = 30
    BREAKPOINT = 31
    DIV = 32
    TRUEDIV = 33
    AND = 34
    OR = 35
    IN = 36
    NOT_IN = 37
    IS = 38
    IS_NOT = 39
    NULL_COALESCE = 40
    SLICE = 41
    LIST_LITERAL = 42
    TUPLE_LITERAL = 43
    SET_LITERAL = 44
    DICT_LITERAL = 45
    FSTRING = 46
    PUSH = 47
    POP = 48
    PUSH_PEEK = 49
    OP_IF = 50
    OP_WHILE = 51
    OP_FOR = 52
    OP_MATCH = 53
    OP_BLOCK = 54
    OP_RETURN = 55
    OP_BREAK = 56
    OP_CONTINUE = 57
    OP_TRY = 58
    OP_RAISE = 59
    EXC_INFO = 60
    CALL = 61
    OP_LAMBDA = 62
    FN_DEF = 63
    SET_LOCALS = 64
    OP_STRUCT = 65
    TRAIT_DEF = 66
    ENUM_DEF = 67
    PRAGMA = 68
    TYPE_OF = 69
    GLOBALS = 70
    LOCALS = 71


# Set of opcodes where arguments should not be evaluated immediately
# CALL is included to allow passing the node for tail-call optimization
CONTROL_FLOW_OPS = frozenset(
    {
        OpCode.OP_IF,
        OpCode.OP_WHILE,
        OpCode.OP_FOR,
        OpCode.OP_MATCH,
        OpCode.OP_BLOCK,
        OpCode.CALL,  # Needs unevaluated args to pass node for tail-call check
        OpCode.OP_LAMBDA,
        OpCode.FN_DEF,
        OpCode.SET_LOCALS,
        OpCode.ND_RECURSION,  # Lambda received unevaluated
        OpCode.ND_MAP,  # Function received unevaluated
        OpCode.OP_STRUCT,  # Body not pre-evaluated
        OpCode.TRAIT_DEF,  # Method lambdas not pre-evaluated
        OpCode.OP_TRY,  # Body/handlers/finally not pre-evaluated
    }
)

# Set of opcodes that are commutative (a op b == b op a)
COMMUTATIVE_OPS = frozenset(
    {
        OpCode.ADD,
        OpCode.MUL,
        OpCode.EQ,
        OpCode.NE,
        OpCode.BAND,
        OpCode.BOR,
        OpCode.BXOR,
        OpCode.AND,
        OpCode.OR,
    }
)

# Set of opcodes that are associative ((a op b) op c == a op (b op c))
ASSOCIATIVE_OPS = frozenset(
    {
        OpCode.ADD,
        OpCode.MUL,
        OpCode.BAND,
        OpCode.BOR,
        OpCode.BXOR,
        OpCode.AND,
        OpCode.OR,
    }
)
