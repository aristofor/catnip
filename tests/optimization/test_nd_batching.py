# FILE: tests/optimization/test_nd_batching.py
"""Tests for ND-recursion batching optimization."""

import pytest

from catnip import Catnip


def exec_catnip(code: str):
    """Helper to execute Catnip code."""
    catnip = Catnip()
    catnip.parse(code)
    return catnip.execute()


def test_pragma_nd_batch_size_auto():
    """Test auto batch size calculation (default)."""
    code = '''
    pragma("nd_mode", "process")
    pragma("nd_workers", "4")
    # Auto batch size (default 0)

    list(3, 4, 5, 6).[~~ (n, recur) => {
        if n <= 1 { 1 }
        else { n * recur(n - 1) }
    }]
    '''
    result = exec_catnip(code)
    assert result == [6, 24, 120, 720], f"Expected [6, 24, 120, 720], got {result}"


def test_pragma_nd_batch_size_explicit():
    """Test explicit batch size."""
    code = '''
    pragma("nd_mode", "process")
    pragma("nd_workers", "4")
    pragma("nd_batch_size", "2")  # Process 2 items per batch

    list(3, 4, 5, 6, 7, 8).[~~ (n, recur) => {
        if n <= 1 { 1 }
        else { n * recur(n - 1) }
    }]
    '''
    result = exec_catnip(code)
    assert result == [6, 24, 120, 720, 5040, 40320], f"Expected factorials, got {result}"


def test_pragma_nd_batch_size_large_collection():
    """Test batching with large collection."""
    code = '''
    pragma("nd_mode", "process")
    pragma("nd_workers", "4")
    pragma("nd_batch_size", "5")

    # Large collection: 20 items
    range(1, 21).[~~ (n, recur) => {
        if n <= 0 { 0 }
        else { 2 + recur(n - 1) }
    }]
    '''
    result = exec_catnip(code)
    # double(n) returns 2*n via recursion
    expected = [2 * i for i in range(1, 21)]
    assert result == expected, f"Expected {expected}, got {result}"


def test_nd_map_batching():
    """Test batching works with ND-map broadcast."""
    code = '''
    pragma("nd_mode", "process")
    pragma("nd_workers", "4")
    pragma("nd_batch_size", "3")

    list(-1, -2, -3, -4, -5, -6, -7, -8, -9).[~> abs]
    '''
    result = exec_catnip(code)
    assert result == [1, 2, 3, 4, 5, 6, 7, 8, 9], f"Expected [1,2,...,9], got {result}"


def test_batching_small_collection_no_batching():
    """Test that small collections skip batching overhead."""
    code = '''
    pragma("nd_mode", "process")
    pragma("nd_workers", "8")  # More workers than items
    pragma("nd_batch_size", "10")

    # Only 4 items, less than n_workers*2 (16)
    # Should skip batching and use direct executor.map
    list(1, 2, 3, 4).[~> (x) => { x * 2 }]
    '''
    result = exec_catnip(code)
    assert result == [2, 4, 6, 8], f"Expected [2,4,6,8], got {result}"


def test_batching_shorthand_pragma():
    """Test batch_size shorthand pragma."""
    code = '''
    pragma("nd_mode", "process")
    pragma("batch_size", "2")  # Shorthand

    list(1, 2, 3, 4, 5).[~> (x) => { x + 10 }]
    '''
    result = exec_catnip(code)
    assert result == [11, 12, 13, 14, 15], f"Expected [11-15], got {result}"


def test_batching_with_memoization():
    """Test batching combined with memoization."""
    code = '''
    pragma("nd_mode", "process")
    pragma("nd_memoize", "on")
    pragma("nd_batch_size", "3")

    # List with duplicates - memoization should help
    list(5, 6, 5, 7, 6, 8).[~~ (n, recur) => {
        if n <= 1 { n }
        else { recur(n - 1) + recur(n - 2) }
    }]
    '''
    result = exec_catnip(code)
    # fib: 0,1,1,2,3,5,8,13,21,34...
    expected = [5, 8, 5, 13, 8, 21]
    assert result == expected, f"Expected {expected}, got {result}"


def test_batching_sequential_mode_ignored():
    """Test that batch_size is ignored in sequential mode."""
    code = '''
    pragma("nd_mode", "sequential")
    pragma("nd_batch_size", "100")  # Should be ignored

    list(1, 2, 3).[~> (x) => { x * 3 }]
    '''
    result = exec_catnip(code)
    assert result == [3, 6, 9], f"Expected [3,6,9], got {result}"


def test_batching_preserves_tuple_type():
    """Test that batching preserves tuple type."""
    code = '''
    pragma("nd_mode", "process")
    pragma("nd_batch_size", "2")

    tuple(3, 4, 5, 6).[~~ (n, recur) => {
        if n <= 1 { 1 }
        else { n * recur(n - 1) }
    }]
    '''
    result = exec_catnip(code)
    assert isinstance(result, tuple), f"Expected tuple, got {type(result)}"
    assert result == (6, 24, 120, 720), f"Expected (6,24,120,720), got {result}"
