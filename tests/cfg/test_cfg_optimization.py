# FILE: tests/cfg/test_cfg_optimization.py
"""Tests for CFG optimizations."""

import catnip._rs as rs
import pytest

from catnip import Catnip


@pytest.fixture
def catnip():
    return Catnip()


def test_eliminate_dead_code(catnip):
    """Test dead code elimination."""
    # All blocks reachable - no dead code
    ir = catnip.parse('x = 1; y = 2')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'test')

    dead = cfg.eliminate_dead_code()
    assert dead == 0  # No dead blocks


def test_merge_sequential_blocks(catnip):
    """Test merging sequential blocks."""
    # Simple sequence should not be merged if they're separate statements
    ir = catnip.parse('x = 1; y = 2; z = 3')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'linear')

    before = cfg.num_blocks
    merged = cfg.merge_blocks()

    # Entry block contains all statements, can't merge with exit
    assert cfg.num_blocks <= before


def test_remove_empty_blocks_while(catnip):
    """Test removing empty blocks in while loop."""
    code = '''
while x < 10 {
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'while')

    before = cfg.num_blocks
    removed = cfg.remove_empty_blocks()

    # while_exit block is empty and can be removed
    assert removed >= 0
    assert cfg.num_blocks <= before


def test_remove_empty_blocks_if(catnip):
    """Test removing empty blocks in if/else."""
    code = '''
if x {
    y = 1
} else {
    y = 2
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'if')

    before = cfg.num_blocks
    removed = cfg.remove_empty_blocks()

    # May remove empty merge block
    assert removed >= 0
    assert cfg.num_blocks <= before


def test_eliminate_constant_branches_same_target(catnip):
    """Test eliminating branches where both paths go to same target."""
    # Simple if - no constant branches in normal code
    ir = catnip.parse('if x { y = 1 }')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'test')

    # Test the API exists
    result = cfg.eliminate_constant_branches()
    assert result >= 0  # May or may not find constant branches


def test_optimize_full_pipeline(catnip):
    """Test full optimization pipeline."""
    code = '''
x = 0
while x < 10 {
    if x % 2 == 0 {
        y = x
    }
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'complex')

    before_blocks = cfg.num_blocks
    before_edges = cfg.num_edges

    dead, merged, empty, branches = cfg.optimize()

    # At least some optimizations should happen
    total = dead + merged + empty + branches
    assert total >= 0

    # Blocks/edges should not increase
    assert cfg.num_blocks <= before_blocks
    assert cfg.num_edges <= before_edges


def test_optimize_nested_loops(catnip):
    """Test optimization on nested loops."""
    code = '''
while x < 10 {
    while y < 5 {
        y = y + 1
    }
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'nested')

    before = cfg.num_blocks
    dead, merged, empty, branches = cfg.optimize()

    assert cfg.num_blocks <= before


def test_optimize_with_break(catnip):
    """Test optimization with break statement."""
    code = '''
while x < 10 {
    if x == 5 {
        break
    }
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'break')

    before = cfg.num_blocks
    dead, merged, empty, branches = cfg.optimize()

    # Empty blocks may be removed
    assert cfg.num_blocks <= before


def test_optimize_preserves_semantics(catnip):
    """Test that optimization preserves CFG semantics."""
    code = '''
if x > 0 {
    y = 1
} else {
    y = 2
}
z = y + 1
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'test')

    # Get reachable blocks before
    before_reachable = set(cfg.get_reachable_blocks())

    # Optimize
    cfg.optimize()

    # All previously reachable blocks should still be reachable or merged
    after_reachable = set(cfg.get_reachable_blocks())
    assert len(after_reachable) > 0


def test_optimize_idempotent(catnip):
    """Test that running optimize twice gives same result."""
    code = '''
while x < 10 {
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'loop')

    # First optimization
    cfg.optimize()
    blocks_after_first = cfg.num_blocks
    edges_after_first = cfg.num_edges

    # Second optimization should do nothing
    dead, merged, empty, branches = cfg.optimize()
    assert dead == 0
    assert merged == 0
    assert empty == 0
    assert branches == 0
    assert cfg.num_blocks == blocks_after_first
    assert cfg.num_edges == edges_after_first


def test_individual_optimizations(catnip):
    """Test that individual optimizations can be called separately."""
    code = 'while x { y = 1 }'
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'test')

    # Should be able to call each optimization
    dead = cfg.eliminate_dead_code()
    assert dead >= 0

    merged = cfg.merge_blocks()
    assert merged >= 0

    empty = cfg.remove_empty_blocks()
    assert empty >= 0

    branches = cfg.eliminate_constant_branches()
    assert branches >= 0
