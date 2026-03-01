# FILE: tests/cfg/test_cfg_rust.py
"""Tests for Rust CFG implementation."""

import catnip._rs as rs
import pytest

from catnip import Catnip


@pytest.fixture
def catnip():
    return Catnip()


def test_cfg_while_loop(catnip):
    """Test CFG for while loop."""
    ir = catnip.parse('while x < 10 { x = x + 1 }')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'while')

    # entry, exit, while_header, while_body, while_exit
    assert cfg.num_blocks == 5

    # entry->header, header->body, header->exit, body->header, exit->exit
    assert cfg.num_edges == 5

    # All blocks reachable
    assert len(cfg.get_reachable_blocks()) == 5


def test_cfg_if_else(catnip):
    """Test CFG for if/else."""
    ir = catnip.parse('if x > 0 { y = 1 } else { y = 2 }')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'if_else')

    # entry, exit, if_then, if_else, if_merge
    assert cfg.num_blocks == 5

    # entry->then, entry->else, then->merge, else->merge, merge->exit
    assert cfg.num_edges == 5


def test_cfg_nested_break(catnip):
    """Test CFG with nested if and break."""
    code = '''
x = 0
while x < 10 {
    if x == 5 { break }
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'nested_break')

    # entry, exit, while_header, while_body, while_exit, if_then, if_else, if_merge
    assert cfg.num_blocks == 8
    assert cfg.num_edges == 9


def test_cfg_for_loop(catnip):
    """Test CFG for for loop."""
    ir = catnip.parse('for x in range(10) { y = x }')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'for')

    # entry, exit, for_header, for_body, for_exit
    assert cfg.num_blocks == 5


def test_cfg_continue(catnip):
    """Test CFG with continue."""
    code = '''
while x < 10 {
    if x % 2 == 0 { continue }
    x = x + 1
}
'''
    ir = catnip.parse(code)
    cfg = rs.cfg.build_cfg_from_ir(ir, 'continue')

    # Has continue edge to loop header
    assert cfg.num_blocks == 8


def test_cfg_dominance_diamond(catnip):
    """Test dominance analysis on diamond (if/else) CFG."""
    ir = catnip.parse('if x > 0 { y = 1 } else { y = 2 }')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'if_else')
    cfg.compute_dominators()

    blocks = cfg.get_reachable_blocks()
    assert len(blocks) == 5

    # entry should dominate all blocks
    entry = 0
    for block_id in blocks:
        if block_id != entry:
            assert entry in cfg.get_dominators(block_id)


def test_cfg_loop_detection(catnip):
    """Test loop detection."""
    ir = catnip.parse('while x < 10 { x = x + 1 }')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'loop')
    cfg.compute_dominators()

    loops = cfg.detect_loops()
    assert len(loops) == 1

    header, loop_blocks = loops[0]
    # Loop should contain at least header and body
    assert len(loop_blocks) >= 2
    assert header in loop_blocks


def test_cfg_nested_loops(catnip):
    """Test nested loops detection."""
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
    cfg.compute_dominators()

    loops = cfg.detect_loops()
    # Should detect 2 loops (outer and inner)
    assert len(loops) == 2


def test_cfg_to_dot(catnip):
    """Test DOT generation."""
    ir = catnip.parse('if x > 0 { y = 1 } else { y = 2 }')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'if_else')

    dot = cfg.to_dot()

    # Check basic DOT structure
    assert 'digraph "if_else"' in dot
    assert 'entry' in dot
    assert 'exit' in dot
    assert 'conditional_true' in dot
    assert 'conditional_false' in dot


def test_cfg_visualize(catnip, tmp_path):
    """Test visualize method."""
    ir = catnip.parse('while x < 10 { x = x + 1 }')
    cfg = rs.cfg.build_cfg_from_ir(ir, 'loop')

    dot_file = tmp_path / 'test.dot'
    cfg.visualize(str(dot_file))

    assert dot_file.exists()
    content = dot_file.read_text()
    assert 'digraph "loop"' in content
