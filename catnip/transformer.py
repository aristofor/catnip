# FILE: catnip/transformer.py
"""
Transformer data structures for AST intermediate representation.

The transformation from parse tree to IR is done in Rust via Tree-sitter.
This module provides the core data structures.
"""

from catnip._rs import (
    Call,
    CallMember,
    Identifier,
    Lvalue,
    Member,
    Params,
    SetAttrTarget,
)
from catnip.nodes import Op
from catnip.semantic.opcode import OpCode


class IR(Op):
    """
    Intermediate Representation node.

    Represents an operation in the AST before semantic analysis.
    Uses OpCode enum for operation identification.
    """

    def __init__(self, ident, args=None, kwargs=None, start_byte=-1, end_byte=-1):
        self.ident = ident
        self.args = tuple(args) if args else ()
        self.kwargs = kwargs if kwargs else {}
        self.tail = False
        self.start_byte = start_byte
        self.end_byte = end_byte

    def __hash__(self):
        return hash((self.ident, self.args, tuple(self.kwargs.items())))

    def __repr__(self):
        if hasattr(self.ident, 'name'):
            ident_repr = self.ident.name
        else:
            from catnip.semantic.opcode import OpCode

            try:
                ident_repr = OpCode(self.ident).name
            except ValueError:
                ident_repr = self.ident
        trailer = f' {self.kwargs!r}' if self.kwargs else ''
        return f'<IR {ident_repr} {self.args!r}{trailer}>'


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
