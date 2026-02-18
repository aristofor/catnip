# FILE: catnip/semantic/opcode.py
# GENERATED FROM catnip_rs/src/ir/opcode.rs
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

    NOP = 1
    OP_IF = 2
    OP_WHILE = 3
    OP_FOR = 4
    OP_MATCH = 5
    OP_BLOCK = 6
    OP_RETURN = 7
    OP_BREAK = 8
    OP_CONTINUE = 9
    CALL = 10
    OP_LAMBDA = 11
    FN_DEF = 12
    SET_LOCALS = 13
    GETATTR = 14
    SETATTR = 15
    GETITEM = 16
    SETITEM = 17
    SLICE = 18
    ADD = 19
    SUB = 20
    MUL = 21
    DIV = 22
    TRUEDIV = 23
    FLOORDIV = 24
    MOD = 25
    POW = 26
    NEG = 27
    POS = 28
    EQ = 29
    NE = 30
    LT = 31
    LE = 32
    GT = 33
    GE = 34
    AND = 35
    OR = 36
    NOT = 37
    BAND = 38
    BOR = 39
    BXOR = 40
    BNOT = 41
    LSHIFT = 42
    RSHIFT = 43
    BROADCAST = 44
    LIST_LITERAL = 45
    TUPLE_LITERAL = 46
    SET_LITERAL = 47
    DICT_LITERAL = 48
    PUSH = 49
    POP = 50
    PUSH_PEEK = 51
    FSTRING = 52
    PRAGMA = 53
    ND_RECURSION = 54
    ND_MAP = 55
    ND_EMPTY_TOPOS = 56
    BREAKPOINT = 57
    OP_STRUCT = 58
    TRAIT_DEF = 59


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
