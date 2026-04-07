# FILE: catnip/vm/opcodes.py
# GENERATED FROM catnip_core/src/vm/opcode.rs
# Do not edit the enum manually. Run: python catnip_rs/gen_opcodes.py
"""
VM opcodes and metadata.

Defines the instruction set for the Catnip VM bytecode.
Each opcode has associated metadata for stack effect calculation
and disassembly.
"""

from enum import IntEnum


class VMOp(IntEnum):
    """VM bytecode opcodes."""

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
    IN = 33
    NOT_IN = 34
    IS = 35
    IS_NOT = 36
    TO_BOOL = 37
    TYPE_OF = 38
    LOAD_CONST = 39
    LOAD_LOCAL = 40
    STORE_LOCAL = 41
    LOAD_SCOPE = 42
    STORE_SCOPE = 43
    LOAD_GLOBAL = 44
    POP_TOP = 45
    DUP_TOP = 46
    ROT_TWO = 47
    JUMP = 48
    JUMP_IF_FALSE = 49
    JUMP_IF_TRUE = 50
    JUMP_IF_FALSE_OR_POP = 51
    JUMP_IF_TRUE_OR_POP = 52
    JUMP_IF_NONE = 53
    JUMP_IF_NOT_NONE_OR_POP = 54
    GET_ITER = 55
    FOR_ITER = 56
    FOR_RANGE_INT = 57
    FOR_RANGE_STEP = 58
    CALL = 59
    CALL_KW = 60
    CALL_METHOD = 61
    TAILCALL = 62
    RETURN = 63
    MAKE_FUNCTION = 64
    BUILD_LIST = 65
    BUILD_TUPLE = 66
    BUILD_SET = 67
    BUILD_DICT = 68
    BUILD_SLICE = 69
    FORMAT_VALUE = 70
    BUILD_STRING = 71
    PUSH_BLOCK = 72
    POP_BLOCK = 73
    BREAK = 74
    CONTINUE = 75
    MATCH_PATTERN = 76
    MATCH_PATTERN_VM = 77
    MATCH_ASSIGN_PATTERN_VM = 78
    BIND_MATCH = 79
    MATCH_FAIL = 80
    UNPACK_SEQUENCE = 81
    UNPACK_EX = 82
    MAKE_STRUCT = 83
    MAKE_TRAIT = 84
    MAKE_ENUM = 85
    HALT = 86
    EXIT = 87
    GLOBALS = 88
    LOCALS = 89
    SETUP_EXCEPT = 90
    SETUP_FINALLY = 91
    POP_HANDLER = 92
    RAISE = 93
    CHECK_EXC_MATCH = 94
    LOAD_EXCEPTION = 95
    RESUME_UNWIND = 96
    CLEAR_EXCEPTION = 97


# Stack effect: (pops, pushes) for each opcode
# Negative value means "depends on instruction argument"
STACK_EFFECT = {
    VMOp.LOAD_CONST: (0, 1),
    VMOp.LOAD_LOCAL: (0, 1),
    VMOp.STORE_LOCAL: (1, 0),
    VMOp.LOAD_SCOPE: (0, 1),
    VMOp.STORE_SCOPE: (1, 0),
    VMOp.LOAD_GLOBAL: (0, 1),
    VMOp.POP_TOP: (1, 0),
    VMOp.DUP_TOP: (1, 2),
    VMOp.ROT_TWO: (2, 2),
    VMOp.ADD: (2, 1),
    VMOp.SUB: (2, 1),
    VMOp.MUL: (2, 1),
    VMOp.DIV: (2, 1),
    VMOp.FLOORDIV: (2, 1),
    VMOp.MOD: (2, 1),
    VMOp.POW: (2, 1),
    VMOp.NEG: (1, 1),
    VMOp.POS: (1, 1),
    VMOp.BOR: (2, 1),
    VMOp.BXOR: (2, 1),
    VMOp.BAND: (2, 1),
    VMOp.BNOT: (1, 1),
    VMOp.LSHIFT: (2, 1),
    VMOp.RSHIFT: (2, 1),
    VMOp.LT: (2, 1),
    VMOp.LE: (2, 1),
    VMOp.GT: (2, 1),
    VMOp.GE: (2, 1),
    VMOp.EQ: (2, 1),
    VMOp.NE: (2, 1),
    VMOp.IN: (2, 1),
    VMOp.NOT_IN: (2, 1),
    VMOp.IS: (2, 1),
    VMOp.IS_NOT: (2, 1),
    VMOp.NOT: (1, 1),
    VMOp.TO_BOOL: (1, 1),
    VMOp.JUMP: (0, 0),
    VMOp.JUMP_IF_FALSE: (1, 0),
    VMOp.JUMP_IF_TRUE: (1, 0),
    VMOp.JUMP_IF_FALSE_OR_POP: (1, 0),
    VMOp.JUMP_IF_TRUE_OR_POP: (1, 0),
    VMOp.JUMP_IF_NOT_NONE_OR_POP: (1, 0),
    VMOp.GET_ITER: (1, 1),
    VMOp.FOR_ITER: (0, 1),
    VMOp.FOR_RANGE_INT: (0, 0),
    VMOp.CALL: (-1, 1),
    VMOp.CALL_KW: (-1, 1),
    VMOp.TAILCALL: (-1, 0),
    VMOp.RETURN: (1, 0),
    VMOp.MAKE_FUNCTION: (1, 1),
    VMOp.BUILD_LIST: (-1, 1),
    VMOp.BUILD_TUPLE: (-1, 1),
    VMOp.BUILD_SET: (-1, 1),
    VMOp.BUILD_DICT: (-1, 1),
    VMOp.BUILD_SLICE: (-1, 1),
    VMOp.GETATTR: (1, 1),
    VMOp.SETATTR: (2, 0),
    VMOp.GETITEM: (2, 1),
    VMOp.SETITEM: (3, 0),
    VMOp.PUSH_BLOCK: (0, 0),
    VMOp.POP_BLOCK: (0, 0),
    VMOp.BREAK: (0, 0),
    VMOp.CONTINUE: (0, 0),
    VMOp.BROADCAST: (-1, 1),
    VMOp.MATCH_PATTERN: (1, 1),
    VMOp.MATCH_PATTERN_VM: (1, 1),
    VMOp.BIND_MATCH: (1, 0),
    VMOp.JUMP_IF_NONE: (1, 0),
    VMOp.UNPACK_SEQUENCE: (1, -1),
    VMOp.UNPACK_EX: (1, -1),
    VMOp.NOP: (0, 0),
    VMOp.HALT: (0, 0),
    VMOp.ND_EMPTY_TOPOS: (0, 1),
    VMOp.ND_RECURSION: (-1, 1),
    VMOp.ND_MAP: (-1, 1),
    VMOp.FOR_RANGE_STEP: (0, 0),
    VMOp.BREAKPOINT: (0, 0),
    VMOp.MAKE_STRUCT: (0, 0),
    VMOp.MAKE_TRAIT: (0, 0),
    VMOp.CALL_METHOD: (-1, 1),
    VMOp.MATCH_FAIL: (0, 0),
    VMOp.MATCH_ASSIGN_PATTERN_VM: (1, 1),
    VMOp.EXIT: (-1, 0),
    VMOp.SETUP_EXCEPT: (0, 0),
    VMOp.SETUP_FINALLY: (0, 0),
    VMOp.POP_HANDLER: (0, 0),
    VMOp.RAISE: (1, 0),
    VMOp.CHECK_EXC_MATCH: (0, 1),
    VMOp.LOAD_EXCEPTION: (0, 1),
    VMOp.RESUME_UNWIND: (0, 0),
}


# Opcodes that have an argument
HAS_ARG = frozenset(
    {
        VMOp.LOAD_CONST,
        VMOp.LOAD_LOCAL,
        VMOp.STORE_LOCAL,
        VMOp.LOAD_SCOPE,
        VMOp.STORE_SCOPE,
        VMOp.LOAD_GLOBAL,
        VMOp.JUMP,
        VMOp.JUMP_IF_FALSE,
        VMOp.JUMP_IF_TRUE,
        VMOp.JUMP_IF_FALSE_OR_POP,
        VMOp.JUMP_IF_TRUE_OR_POP,
        VMOp.JUMP_IF_NOT_NONE_OR_POP,
        VMOp.FOR_ITER,
        VMOp.FOR_RANGE_INT,
        VMOp.CALL,
        VMOp.CALL_KW,
        VMOp.TAILCALL,
        VMOp.BUILD_LIST,
        VMOp.BUILD_TUPLE,
        VMOp.BUILD_SET,
        VMOp.BUILD_DICT,
        VMOp.BUILD_SLICE,
        VMOp.GETATTR,
        VMOp.SETATTR,
        VMOp.MAKE_FUNCTION,
        VMOp.BROADCAST,
        VMOp.MATCH_PATTERN,
        VMOp.MATCH_PATTERN_VM,
        VMOp.JUMP_IF_NONE,
        VMOp.UNPACK_SEQUENCE,
        VMOp.UNPACK_EX,
        VMOp.ND_RECURSION,
        VMOp.ND_MAP,
        VMOp.FOR_RANGE_STEP,
        VMOp.MAKE_STRUCT,
        VMOp.MAKE_TRAIT,
        VMOp.CALL_METHOD,
        VMOp.MATCH_FAIL,
        VMOp.MATCH_ASSIGN_PATTERN_VM,
        VMOp.SETUP_EXCEPT,
        VMOp.SETUP_FINALLY,
        VMOp.RAISE,
        VMOp.CHECK_EXC_MATCH,
        VMOp.EXIT,
    }
)


def disassemble_instruction(opcode: int, arg: int = 0) -> str:
    """Format a single instruction for display."""
    try:
        op_name = VMOp(opcode).name
    except ValueError:
        op_name = f"UNKNOWN({opcode})"

    if opcode in HAS_ARG:
        return f"{op_name} {arg}"
    return op_name
