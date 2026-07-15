#!/usr/bin/env python3
# FILE: catnip_rs/gen_opcodes.py
"""Generate Python opcode enums from Rust source files.

Flow: Rust (source of truth) → Python (generated)

Generates:
- catnip/semantic/opcode.py from catnip_core/src/ir/opcode.rs

The Rust files define the opcodes with explicit values.
Python files are generated with the same explicit values (no auto()).
"""

import re
from pathlib import Path
from typing import List, Tuple


def parse_rust_enum(path: Path, enum_name: str) -> List[Tuple[str, int]]:
    """Parse a Rust enum to extract variant names and values.

    Args:
        path: Path to the Rust file
        enum_name: Name of the enum to extract (e.g., 'IROpCode')

    Returns:
        [(variant_name, value), ...]
    """
    content = path.read_text()

    # Find the enum block
    enum_pattern = rf'pub enum {enum_name}\s*\{{\s*(.*?)\s*\}}'
    match = re.search(enum_pattern, content, re.DOTALL)
    if not match:
        raise ValueError(f"Enum {enum_name} not found in {path}")

    enum_body = match.group(1)

    # Extract variants: "VariantName = value,"
    variant_pattern = r'(\w+)\s*=\s*(\d+)'
    variants = re.findall(variant_pattern, enum_body)

    return [(name, int(value)) for name, value in variants]


# Explicit mapping for names that don't follow simple PascalCase → SNAKE_CASE
SNAKE_CASE_OVERRIDES = {
    # Compound words without underscore
    'GetAttr': 'GETATTR',
    'SetAttr': 'SETATTR',
    'GetItem': 'GETITEM',
    'SetItem': 'SETITEM',
    'TrueDiv': 'TRUEDIV',
    'FloorDiv': 'FLOORDIV',
    # Bitwise ops: single letter prefix
    'BAnd': 'BAND',
    'BOr': 'BOR',
    'BXor': 'BXOR',
    'BNot': 'BNOT',
    'LShift': 'LSHIFT',
    'RShift': 'RSHIFT',
    # VM-specific
    'TailCall': 'TAILCALL',
    'CallKw': 'CALL_KW',
    'PopTop': 'POP_TOP',
    'DupTop': 'DUP_TOP',
    'RotTwo': 'ROT_TWO',
    'GetIter': 'GET_ITER',
    'ForIter': 'FOR_ITER',
    'ForRangeInt': 'FOR_RANGE_INT',
    'MakeFunction': 'MAKE_FUNCTION',
    'BuildList': 'BUILD_LIST',
    'BuildTuple': 'BUILD_TUPLE',
    'BuildSet': 'BUILD_SET',
    'BuildDict': 'BUILD_DICT',
    'BuildSlice': 'BUILD_SLICE',
    'PushBlock': 'PUSH_BLOCK',
    'PopBlock': 'POP_BLOCK',
    'MatchPattern': 'MATCH_PATTERN',
    'BindMatch': 'BIND_MATCH',
    'JumpIfNone': 'JUMP_IF_NONE',
    'JumpIfFalse': 'JUMP_IF_FALSE',
    'JumpIfTrue': 'JUMP_IF_TRUE',
    'JumpIfFalseOrPop': 'JUMP_IF_FALSE_OR_POP',
    'JumpIfTrueOrPop': 'JUMP_IF_TRUE_OR_POP',
    'JumpIfNotNoneOrPop': 'JUMP_IF_NOT_NONE_OR_POP',
    'UnpackSequence': 'UNPACK_SEQUENCE',
    'UnpackEx': 'UNPACK_EX',
    'LoadConst': 'LOAD_CONST',
    'LoadLocal': 'LOAD_LOCAL',
    'StoreLocal': 'STORE_LOCAL',
    'LoadScope': 'LOAD_SCOPE',
    'StoreScope': 'STORE_SCOPE',
    'LoadGlobal': 'LOAD_GLOBAL',
    # IR-specific
    'ListLiteral': 'LIST_LITERAL',
    'TupleLiteral': 'TUPLE_LITERAL',
    'SetLiteral': 'SET_LITERAL',
    'DictLiteral': 'DICT_LITERAL',
    'SetLocals': 'SET_LOCALS',
    'FnDef': 'FN_DEF',
    'PushPeek': 'PUSH_PEEK',
    'NdRecursion': 'ND_RECURSION',
    'NdMap': 'ND_MAP',
    'NdEmptyTopos': 'ND_EMPTY_TOPOS',
    'OpStruct': 'OP_STRUCT',
    'MatchPatternVM': 'MATCH_PATTERN_VM',
    'MatchAssignPatternVM': 'MATCH_ASSIGN_PATTERN_VM',
    'TraitDef': 'TRAIT_DEF',
    'CallMethod': 'CALL_METHOD',
    'MatchFail': 'MATCH_FAIL',
}


def pascal_to_snake(name: str) -> str:
    """Convert PascalCase to SNAKE_CASE.

    Uses explicit overrides for compound words and edge cases.
    """
    # Check explicit overrides first
    if name in SNAKE_CASE_OVERRIDES:
        return SNAKE_CASE_OVERRIDES[name]

    # Standard conversion for simple cases
    result = [name[0]]
    for char in name[1:]:
        if char.isupper():
            result.append('_')
        result.append(char)
    return ''.join(result).upper()


def generate_ir_opcode_py(opcodes: List[Tuple[str, int]], rust_path: str) -> str:
    """Generate catnip/semantic/opcode.py content."""
    lines = [
        '# FILE: catnip/semantic/opcode.py',
        f'# GENERATED FROM {rust_path}',
        '# Do not edit the enum manually. Run: python catnip_rs/gen_opcodes.py',
        '"""',
        'OpCode enumeration for efficient integer-based operation representation.',
        '',
        'Using integer opcodes instead of strings provides:',
        '- Faster comparison and hashing (integer vs string)',
        '- Lower memory usage',
        '- Better performance in tight loops',
        '- Type safety with enum',
        '"""',
        '',
        'from enum import IntEnum',
        '',
        '',
        'class OpCode(IntEnum):',
        '    """',
        '    Enumeration of all operation codes in Catnip.',
        '',
        '    Categories:',
        '    - Other: NOP (no operation)',
        '    - Control flow: IF, WHILE, FOR, MATCH, BLOCK, RETURN, BREAK, CONTINUE',
        '    - Functions: CALL, LAMBDA, FN_DEF',
        '    - Variables: SET_LOCALS, GETATTR',
        '    - Arithmetic: ADD, SUB, MUL, DIV, FLOORDIV, MOD, POW',
        '    - Comparison: EQ, NE, LT, LE, GT, GE',
        '    - Logical: AND, OR, NOT',
        '    - Bitwise: BAND, BOR, BXOR, BNOT, LSHIFT, RSHIFT',
        '    - Collections: BROADCAST',
        '    """',
        '',
    ]

    # Generate enum members with explicit values
    for rust_name, value in opcodes:
        py_name = pascal_to_snake(rust_name)
        lines.append(f'    {py_name} = {value}')

    lines.append('')
    lines.append('')

    # Add metadata constants (these are defined manually, not generated)
    lines.extend([
        '# Set of opcodes where arguments should not be evaluated immediately',
        '# CALL is included to allow passing the node for tail-call optimization',
        'CONTROL_FLOW_OPS = frozenset(',
        '    {',
        '        OpCode.OP_IF,',
        '        OpCode.OP_WHILE,',
        '        OpCode.OP_FOR,',
        '        OpCode.OP_MATCH,',
        '        OpCode.OP_BLOCK,',
        '        OpCode.CALL,  # Needs unevaluated args to pass node for tail-call check',
        '        OpCode.OP_LAMBDA,',
        '        OpCode.FN_DEF,',
        '        OpCode.SET_LOCALS,',
        '        OpCode.ND_RECURSION,  # Lambda received unevaluated',
        '        OpCode.ND_MAP,  # Function received unevaluated',
        '        OpCode.OP_STRUCT,  # Body not pre-evaluated',
        '        OpCode.TRAIT_DEF,  # Method lambdas not pre-evaluated',
        '        OpCode.OP_TRY,  # Body/handlers/finally not pre-evaluated',
        '    }',
        ')',
        '',
        '# Set of opcodes that are commutative (a op b == b op a)',
        'COMMUTATIVE_OPS = frozenset(',
        '    {',
        '        OpCode.ADD,',
        '        OpCode.MUL,',
        '        OpCode.EQ,',
        '        OpCode.NE,',
        '        OpCode.BAND,',
        '        OpCode.BOR,',
        '        OpCode.BXOR,',
        '        OpCode.AND,',
        '        OpCode.OR,',
        '    }',
        ')',
        '',
        '# Set of opcodes that are associative ((a op b) op c == a op (b op c))',
        'ASSOCIATIVE_OPS = frozenset(',
        '    {',
        '        OpCode.ADD,',
        '        OpCode.MUL,',
        '        OpCode.BAND,',
        '        OpCode.BOR,',
        '        OpCode.BXOR,',
        '        OpCode.AND,',
        '        OpCode.OR,',
        '    }',
        ')',
        '',
    ])

    return '\n'.join(lines)


def main():
    base_path = Path(__file__).parent
    catnip_path = base_path.parent / 'catnip'

    core_path = base_path.parent / 'catnip_core'

    # Parse IR opcodes from Rust (source of truth in catnip_core)
    print("Parsing IR opcodes from Rust...")
    ir_rust_path = core_path / 'src' / 'ir' / 'opcode.rs'
    ir_opcodes = parse_rust_enum(ir_rust_path, 'IROpCode')
    print(f"  Found {len(ir_opcodes)} IR opcodes")

    # Generate Python IR opcodes
    ir_py = generate_ir_opcode_py(ir_opcodes, 'catnip_core/src/ir/opcode.rs')
    ir_output = catnip_path / 'semantic' / 'opcode.py'
    ir_output.write_text(ir_py)
    print(f"  Generated {ir_output}")

    print("\nDone! Flow: Rust (source) -> Python (generated)")


if __name__ == '__main__':
    main()
