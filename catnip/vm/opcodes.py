# FILE: catnip/vm/opcodes.py
# GENERATED FROM catnip_rs/src/vm/opcode.rs
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

    LOAD_CONST = 1
    LOAD_LOCAL = 2
    STORE_LOCAL = 3
    LOAD_SCOPE = 4
    STORE_SCOPE = 5
    LOAD_GLOBAL = 6
    POP_TOP = 7
    DUP_TOP = 8
    ROT_TWO = 9
    ADD = 10
    SUB = 11
    MUL = 12
    DIV = 13
    FLOORDIV = 14
    MOD = 15
    POW = 16
    NEG = 17
    POS = 18
    BOR = 19
    BXOR = 20
    BAND = 21
    BNOT = 22
    LSHIFT = 23
    RSHIFT = 24
    LT = 25
    LE = 26
    GT = 27
    GE = 28
    EQ = 29
    NE = 30
    NOT = 31
    JUMP = 32
    JUMP_IF_FALSE = 33
    JUMP_IF_TRUE = 34
    JUMP_IF_FALSE_OR_POP = 35
    JUMP_IF_TRUE_OR_POP = 36
    GET_ITER = 37
    FOR_ITER = 38
    FOR_RANGE_INT = 39
    CALL = 40
    CALL_KW = 41
    TAILCALL = 42
    RETURN = 43
    MAKE_FUNCTION = 44
    BUILD_LIST = 45
    BUILD_TUPLE = 46
    BUILD_SET = 47
    BUILD_DICT = 48
    BUILD_SLICE = 49
    GETATTR = 50
    SETATTR = 51
    GETITEM = 52
    SETITEM = 53
    PUSH_BLOCK = 54
    POP_BLOCK = 55
    BREAK = 56
    CONTINUE = 57
    BROADCAST = 58
    MATCH_PATTERN = 59
    BIND_MATCH = 60
    JUMP_IF_NONE = 61
    UNPACK_SEQUENCE = 62
    UNPACK_EX = 63
    NOP = 64
    HALT = 65
    ND_EMPTY_TOPOS = 66
    ND_RECURSION = 67
    ND_MAP = 68
    FOR_RANGE_STEP = 69
    MATCH_PATTERN_VM = 70
    BREAKPOINT = 71
    MAKE_STRUCT = 72
    MAKE_TRAIT = 73


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
    VMOp.NOT: (1, 1),
    VMOp.JUMP: (0, 0),
    VMOp.JUMP_IF_FALSE: (1, 0),
    VMOp.JUMP_IF_TRUE: (1, 0),
    VMOp.JUMP_IF_FALSE_OR_POP: (1, 0),
    VMOp.JUMP_IF_TRUE_OR_POP: (1, 0),
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
