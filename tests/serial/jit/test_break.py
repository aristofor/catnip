# FILE: tests/serial/jit/test_break.py
"""Tests for JIT compilation of loops with break statements."""

import pytest


def test_simple_loop_without_break(catnip_with_jit):
    """Test baseline: simple loop without break compiles correctly."""
    # First verify JIT works on simple loops (regression test)
    code = '''
    total = 0
    for i in range(1000) {
        total = total + i
    }
    total
    '''

    result = catnip_with_jit.eval(code)
    expected = sum(range(1000))
    assert result == expected, f"Expected {expected}, got {result}"

    # Verify it compiled
    stats = catnip_with_jit.vm.get_jit_stats()
    assert stats['compiled_loops'] > 0, f"Simple loop should have been compiled (stats: {stats})"


def test_unconditional_break_fallback(catnip_with_jit):
    """Test that conditional break falls back to interpreter (Phase 1 limitation)."""
    # Conditional breaks are not yet supported in JIT
    # They should fallback gracefully to interpreter
    code = '''
    total = 0
    for i in range(1000) {
        if i >= 500 {
            break
        }
        total = total + i
    }
    total
    '''

    result = catnip_with_jit.eval(code)
    expected = sum(range(500))
    assert result == expected, f"Expected {expected}, got {result}"

    # Note: This will NOT compile in Phase 1 (conditional break not supported)
    # It fallbacks to interpreter, which is correct behavior


def test_break_with_conditional_correctness(catnip_with_jit):
    """Test that conditional break executes correctly (interpreter fallback)."""
    code = '''
    total = 0
    for i in range(200) {
        if i >= 150 {
            break
        }
        total = total + i
    }
    total
    '''

    result = catnip_with_jit.eval(code)
    expected = sum(range(150))
    assert result == expected, f"Expected {expected}, got {result}"


def test_break_in_nested_calculation(catnip_with_jit):
    """Test break with nested calculations (interpreter fallback)."""
    code = '''
    result = 0
    for i in range(100) {
        x = i * 2
        y = x + 1
        if y >= 50 {
            break
        }
        result = result + y
    }
    result
    '''

    # Loop runs while y < 50
    # y = i*2 + 1, so y < 50 when i < 24.5, i.e., i <= 24
    result = catnip_with_jit.eval(code)
    expected = sum(i * 2 + 1 for i in range(25))  # i from 0 to 24
    assert result == expected, f"Expected {expected}, got {result}"


def test_early_break(catnip_with_jit):
    """Test early break (first few iterations)."""
    code = '''
    total = 0
    for i in range(1000) {
        total = total + i
        if i >= 5 {
            break
        }
    }
    total
    '''

    result = catnip_with_jit.eval(code)
    expected = sum(range(6))  # 0 + 1 + ... + 5
    assert result == expected, f"Expected {expected}, got {result}"
