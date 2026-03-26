# FILE: catnip/transformer.py
"""
Transformer data structures for AST intermediate representation.

The transformation from parse tree to IR is done in Rust via Tree-sitter.
This module re-exports the Rust data structures.
"""

from catnip._rs import (
    Call,
    CallMember,
    Identifier,
    Lvalue,
    Member,
    Op,
    Params,
    SetAttrTarget,
)
from catnip.semantic.opcode import OpCode

# Alias: legacy semantic passes (Rust PyO3) create nodes via transformer.IR
IR = Op

__all__ = (
    'Call',
    'CallMember',
    'IR',
    'Identifier',
    'Lvalue',
    'Member',
    'OpCode',
    'Params',
    'SetAttrTarget',
)
