# FILE: catnip/semantic/__init__.py
"""
Semantic opcodes for Catnip.

OpCode is generated from Rust (catnip_core IROpCode) via `make gen-opcodes`.
Semantic analysis itself lives in Rust (catnip_core::pipeline::SemanticAnalyzer).
"""

from .opcode import ASSOCIATIVE_OPS, COMMUTATIVE_OPS, CONTROL_FLOW_OPS, OpCode

__all__ = (
    'OpCode',
    'CONTROL_FLOW_OPS',
    'COMMUTATIVE_OPS',
    'ASSOCIATIVE_OPS',
)
