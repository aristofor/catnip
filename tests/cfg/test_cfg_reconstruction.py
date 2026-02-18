# FILE: tests/cfg/test_cfg_reconstruction.py
"""Tests for CFG reconstruction."""

import catnip._rs as rs
import pytest

from catnip import Catnip
from catnip.semantic.opcode import OpCode


@pytest.fixture
def catnip():
    return Catnip()


def test_reconstruct_while_loop(catnip):
    """Test reconstruction of while loop."""
    code = '''
x = 0
while x < 10 {
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'while')
    cfg.compute_dominators()

    # Reconstruct from CFG
    reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

    # Should have at least 2 operations (assignment and while)
    assert len(reconstructed) >= 2

    # Find the while operation
    while_ops = [op for op in reconstructed if op.ident == OpCode.OP_WHILE]
    assert len(while_ops) == 1, "Should have exactly one while operation"


def test_reconstruct_if_else(catnip):
    """Test reconstruction of if/else."""
    code = '''
if x > 0 {
    y = 1
} else {
    y = 2
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'if_else')
    cfg.compute_dominators()

    # Reconstruct from CFG
    reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

    # Should have at least one if operation
    assert len(reconstructed) >= 1

    # Find the if operation
    if_ops = [op for op in reconstructed if op.ident == OpCode.OP_IF]
    assert len(if_ops) == 1, "Should have exactly one if operation"


def test_reconstruct_nested_while(catnip):
    """Test reconstruction of nested while loops."""
    code = '''
x = 0
while x < 10 {
    y = 0
    while y < 5 {
        y = y + 1
    }
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'nested')
    cfg.compute_dominators()

    # Reconstruct from CFG
    reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

    # Should have multiple operations including 2 while loops
    assert len(reconstructed) >= 2

    # Find while operations
    while_ops = [op for op in reconstructed if op.ident == OpCode.OP_WHILE]
    # Note: nested while might be inside the outer while's body
    assert len(while_ops) >= 1, "Should have at least one while operation"


def test_reconstruct_while_with_break(catnip):
    """Test reconstruction of while loop with break."""
    code = '''
x = 0
while x < 10 {
    if x == 5 {
        break
    }
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'break')
    cfg.compute_dominators()

    # Reconstruct from CFG
    reconstructed = rs.cfg.py_reconstruct_from_cfg(cfg)

    # Should have operations including while and if
    while_ops = [op for op in reconstructed if op.ident == OpCode.OP_WHILE]
    assert len(while_ops) == 1, "Should have exactly one while operation"
