# FILE: tests/language/test_memoization.py
"""
Tests for the memoization system.
"""

import pytest

from catnip import Catnip
from catnip.cachesys import Memoization, MemoryCache
from catnip.context import Context


def test_function_cache_basic():
    """Test basique du cache de fonctions"""
    cache = Memoization()

    # No cached result
    assert cache.get("test_func", (1, 2), {}) is None

    # Store a result
    cache.set("test_func", (1, 2), {}, 42)

    # Retrieve the result
    assert cache.get("test_func", (1, 2), {}) == 42


def test_function_cache_different_args():
    """Ensure different arguments produce different keys."""
    cache = Memoization()

    cache.set("func", (1,), {}, "result1")
    cache.set("func", (2,), {}, "result2")

    assert cache.get("func", (1,), {}) == "result1"
    assert cache.get("func", (2,), {}) == "result2"


def test_function_cache_kwargs():
    """Test with named arguments."""
    cache = Memoization()

    cache.set("func", (), {"x": 1}, "result1")
    cache.set("func", (), {"x": 2}, "result2")

    assert cache.get("func", (), {"x": 1}) == "result1"
    assert cache.get("func", (), {"x": 2}) == "result2"


def test_function_cache_invalidate():
    """Test de l'invalidation du cache"""
    cache = Memoization()

    cache.set("func1", (1,), {}, "result1")
    cache.set("func2", (2,), {}, "result2")

    # Invalidate the entire cache
    count = cache.invalidate()
    assert count >= 2

    # Cache is empty
    assert cache.get("func1", (1,), {}) is None
    assert cache.get("func2", (2,), {}) is None


def test_function_cache_stats():
    """Test des statistiques du cache"""
    cache = Memoization()

    stats = cache.stats()
    assert 'size' in stats
    assert 'hits' in stats
    assert 'misses' in stats


def test_cached_builtin_basic():
    """Test de la fonction builtin cached()"""
    cat = Catnip()

    code = """
    counter = 0

    expensive = (x) => {
        counter = counter + 1
        x * x
    }

    # Cache wrapper
    cached_expensive = cached(expensive, "expensive")

    # First call: cache miss
    r1 = cached_expensive(5)
    count1 = counter

    # Second call: cache hit
    r2 = cached_expensive(5)
    count2 = counter

    # Result
    list(r1, r2, count1, count2)
    """

    cat.parse(code)
    result = cat.execute()

    # r1 and r2 should match (25)
    # count1 = 1 (one execution)
    # count2 = 1 (no second execution thanks to cache)
    assert result == [25, 25, 1, 1]


def test_cached_builtin_different_args():
    """Ensure cached() distinguishes different arguments."""
    cat = Catnip()

    code = """
    square = cached((x) => { x * x }, "square")

    r1 = square(3)
    r2 = square(4)
    r3 = square(3)

    list(r1, r2, r3)
    """

    cat.parse(code)
    result = cat.execute()

    assert result == [9, 16, 9]


def test_cache_invalidate_builtin():
    """Test de cache.invalidate()"""
    cat = Catnip()

    code = """
    counter = 0

    func = cached((x) => {
        counter = counter + 1
        x + 10
    }, "func")

    # First call
    r1 = func(5)
    count1 = counter

    # Second call (cache hit)
    r2 = func(5)
    count2 = counter

    # Invalidate the cache
    _cache.invalidate("func")

    # Third call (cache miss after invalidation)
    r3 = func(5)
    count3 = counter

    list(r1, r2, r3, count1, count2, count3)
    """

    cat.parse(code)
    result = cat.execute()

    # r1, r2, r3 = 15
    # count1 = 1 (first execution)
    # count2 = 1 (no execution, cache hit)
    # count3 = 2 (second execution after invalidation)
    assert result == [15, 15, 15, 1, 1, 2]


def test_cache_stats_builtin():
    """Test de cache.stats()"""
    cat = Catnip()

    code = """
    func = cached((x) => { x * 2 }, "func")

    func(1)
    func(2)
    func(1)  # Cache hit

    _cache.stats()
    """

    cat.parse(code)
    stats = cat.execute()

    assert 'size' in stats
    assert 'hits' in stats
    assert 'misses' in stats
    assert stats['hits'] >= 1  # At least one hit
    assert stats['misses'] >= 2  # At least two misses


def test_cached_wrapper_caches_none_results():
    ctx = Context()
    calls = {'n': 0}

    def returns_none(x):
        calls['n'] += 1
        return None

    wrapped = ctx.globals['cached'](returns_none)

    assert wrapped(1) is None
    assert wrapped(1) is None
    assert calls['n'] == 1


def test_cached_wrapper_normalizes_kwargs_order():
    ctx = Context()
    calls = {'n': 0}

    def build_result(**kwargs):
        calls['n'] += 1
        return calls['n']

    wrapped = ctx.globals['cached'](build_result)

    assert wrapped(a=1, b=2) == 1
    assert wrapped(b=2, a=1) == 1
    assert calls['n'] == 1


def test_cache_enable_disable():
    """Test de cache.enable() et cache.disable()"""
    cat = Catnip()

    code = """
    counter = 0

    func = cached((x) => {
        counter = counter + 1
        x * 10
    }, "func")

    # With cache enabled
    r1 = func(5)
    r2 = func(5)
    count1 = counter

    # Disable the cache
    _cache.disable()
    r3 = func(5)
    count2 = counter

    # Re-enable the cache
    _cache.enable()
    r4 = func(5)
    count3 = counter

    list(count1, count2, count3)
    """

    cat.parse(code)
    result = cat.execute()

    # count1 = 1 (single execution with cache)
    # count2 = 2 (second execution without cache)
    # count3 = 2 (cache hit, result still cached)
    assert result == [1, 2, 2]


def test_function_cache_with_custom_backend():
    """Test with a custom backend."""
    backend = MemoryCache(max_size=2)
    cache = Memoization(backend=backend)

    # Fill beyond the limit
    cache.set("func1", (1,), {}, "r1")
    cache.set("func2", (2,), {}, "r2")
    cache.set("func3", (3,), {}, "r3")  # Should evict func1

    # func1 was evicted
    assert cache.get("func1", (1,), {}) is None
    # func2 and func3 remain
    assert cache.get("func2", (2,), {}) == "r2"
    assert cache.get("func3", (3,), {}) == "r3"


def test_cached_with_complex_types():
    """Test avec des types complexes (listes, dicts)"""
    cat = Catnip()

    code = """
    process = cached((data) => {
        # Simulate processing
        len(data)
    }, "process")

    r1 = process(list(1, 2, 3))
    r2 = process(list(1, 2, 3))  # Cache hit
    r3 = process(list(4, 5, 6))  # Cache miss (different data)

    list(r1, r2, r3)
    """

    cat.parse(code)
    result = cat.execute()

    assert result == [3, 3, 3]
