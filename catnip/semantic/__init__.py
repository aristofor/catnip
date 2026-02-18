# FILE: catnip/semantic/__init__.py
"""
Semantic analysis and optimization module for Catnip.

This module provides:
- Semantic analysis: transforms IR to executable Op nodes
- Optimization: IR-level optimizations before semantic analysis
- OpCodes: integer-based operation codes for efficiency
"""

# Import opcodes first (no dependencies)
# Import Semantic analyzer and optimization passes from Rust
from catnip._rs import (
    BlockFlatteningPass,
    BluntCodePass,
    CommonSubexpressionEliminationPass,
    ConstantFoldingPass,
    ConstantPropagationPass,
    CopyPropagationPass,
    DeadCodeEliminationPass,
    DeadStoreEliminationPass,
    FunctionInliningPass,
    Optimizer,
    Semantic,
    StrengthReductionPass,
    TailRecursionToLoopPass,
)
from catnip._rs import (
    OptimizationPassBase as OptimizationPass,
)

from .opcode import ASSOCIATIVE_OPS, COMMUTATIVE_OPS, CONTROL_FLOW_OPS, OpCode

__all__ = (
    # Semantic analysis
    'Semantic',
    # Optimization
    'Optimizer',
    'OptimizationPass',
    'BluntCodePass',
    'ConstantPropagationPass',
    'ConstantFoldingPass',
    'CopyPropagationPass',
    'FunctionInliningPass',
    'DeadStoreEliminationPass',
    'StrengthReductionPass',
    'CommonSubexpressionEliminationPass',
    'BlockFlatteningPass',
    'DeadCodeEliminationPass',
    'TailRecursionToLoopPass',
    # OpCodes
    'OpCode',
    'CONTROL_FLOW_OPS',
    'COMMUTATIVE_OPS',
    'ASSOCIATIVE_OPS',
)
