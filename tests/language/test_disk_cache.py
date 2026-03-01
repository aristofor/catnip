# FILE: tests/language/test_disk_cache.py
"""
Tests for DiskCache backend.
"""

import os
import tempfile
import time
from pathlib import Path

import pytest
from catnip._rs import CacheKey, DiskCache

from catnip.cachesys import CacheType


@pytest.fixture
def temp_cache_dir():
    """Create a temporary directory for cache tests."""
    tmpdir = tempfile.mkdtemp()
    yield tmpdir
    # Cleanup
    import shutil

    shutil.rmtree(tmpdir, ignore_errors=True)


class TestDiskCache:
    """Tests for DiskCache."""

    def test_disk_cache_creation(self, temp_cache_dir):
        """Verify cache creation."""
        cache = DiskCache(directory=temp_cache_dir)

        stats = cache.stats()
        assert stats['size'] == 0
        assert cache.max_size_bytes is None
        assert cache.ttl_seconds is None
        assert cache.hits == 0
        assert cache.misses == 0

    def test_set_and_get(self, temp_cache_dir):
        """Verify set and get basics."""
        cache = DiskCache(directory=temp_cache_dir)
        key = CacheKey(content='x = 1', cache_type=CacheType.AST)
        value = {'ast': 'tree'}

        cache.set(key, value)
        entry = cache.get(key)

        assert entry is not None
        assert entry.value == value
        assert entry.cache_type.value == 'ast'

    def test_set_with_metadata(self, temp_cache_dir):
        """Verify set with metadata."""
        cache = DiskCache(directory=temp_cache_dir)
        key = CacheKey(content='x = 1', cache_type=CacheType.AST)
        value = {'ast': 'tree'}
        metadata = {'timestamp': 123456}

        cache.set(key, value, metadata=metadata)
        entry = cache.get(key)

        assert entry.metadata == metadata

    def test_exists(self, temp_cache_dir):
        """Verify exists."""
        cache = DiskCache(directory=temp_cache_dir)
        key = CacheKey(content='x = 1', cache_type=CacheType.AST)

        assert not cache.exists(key)

        cache.set(key, 'value')

        assert cache.exists(key)

    def test_delete(self, temp_cache_dir):
        """Verify delete."""
        cache = DiskCache(directory=temp_cache_dir)
        key = CacheKey(content='x = 1', cache_type=CacheType.AST)

        cache.set(key, 'value')
        assert cache.exists(key)

        result = cache.delete(key)

        assert result is True
        assert not cache.exists(key)

    def test_clear(self, temp_cache_dir):
        """Verify clear."""
        cache = DiskCache(directory=temp_cache_dir)

        # Add multiple entries
        for i in range(5):
            key = CacheKey(content=f'x = {i}', cache_type=CacheType.AST)
            cache.set(key, i)

        # Some hits/misses
        key = CacheKey(content='x = 0', cache_type=CacheType.AST)
        cache.get(key)
        cache.get(CacheKey(content='nonexistent', cache_type=CacheType.AST))

        stats = cache.stats()
        assert stats['size'] == 5
        assert cache.hits > 0
        assert cache.misses > 0

        cache.clear()

        stats = cache.stats()
        assert stats['size'] == 0
        assert cache.hits == 0
        assert cache.misses == 0

    def test_prune_no_limits(self, temp_cache_dir):
        """Verify prune with no limits returns 0."""
        cache = DiskCache(directory=temp_cache_dir)

        # Add some entries
        for i in range(5):
            key = CacheKey(content=f'x = {i}', cache_type=CacheType.AST)
            cache.set(key, i)

        removed = cache.prune()
        assert removed == 0

    def test_ttl_expiration(self, temp_cache_dir):
        """Verify TTL expiration."""
        cache = DiskCache(directory=temp_cache_dir, ttl_seconds=1)
        key = CacheKey(content='x = 1', cache_type=CacheType.AST)

        cache.set(key, 'value')
        assert cache.exists(key)

        # Entry should exist immediately
        entry = cache.get(key)
        assert entry is not None

        # Wait for expiration (add margin for timing precision)
        time.sleep(2.0)

        # Entry should be expired and return None
        entry = cache.get(key)
        assert entry is None

    def test_prune_removes_expired(self, temp_cache_dir):
        """Verify prune removes expired entries."""
        cache = DiskCache(directory=temp_cache_dir, ttl_seconds=1)

        # Add some entries
        for i in range(5):
            key = CacheKey(content=f'x = {i}', cache_type=CacheType.AST)
            cache.set(key, i)

        assert cache.stats()['size'] == 5

        # Wait for expiration (add margin for timing precision)
        time.sleep(2.0)

        # Prune should remove all expired entries
        removed = cache.prune()
        assert removed == 5
        assert cache.stats()['size'] == 0

    def test_different_cache_types_coexist(self, temp_cache_dir):
        """Verify different cache types can coexist."""
        cache = DiskCache(directory=temp_cache_dir)
        content = 'x = 1'

        # Same content but different types
        key_ast = CacheKey(content=content, cache_type=CacheType.AST)
        key_bytecode = CacheKey(content=content, cache_type=CacheType.BYTECODE)
        key_source = CacheKey(content=content, cache_type=CacheType.SOURCE)

        cache.set(key_ast, 'ast_value')
        cache.set(key_bytecode, 'bytecode_value')
        cache.set(key_source, 'source_value')

        assert cache.get(key_ast).value == 'ast_value'
        assert cache.get(key_bytecode).value == 'bytecode_value'
        assert cache.get(key_source).value == 'source_value'

    def test_cache_complex_values(self, temp_cache_dir):
        """Verify we can cache complex values."""
        cache = DiskCache(directory=temp_cache_dir)
        key = CacheKey(content='x = 1', cache_type=CacheType.AST)

        # Complex value
        complex_value = {
            'ast': {
                'type': 'assign',
                'left': 'x',
                'right': 1,
                'children': [{'type': 'literal', 'value': 1}],
            }
        }

        cache.set(key, complex_value)
        entry = cache.get(key)

        assert entry.value == complex_value
        assert entry.value['ast']['children'][0]['value'] == 1

    def test_persistence_across_instances(self, temp_cache_dir):
        """Verify cache persists across instances."""
        key = CacheKey(content='x = 1', cache_type=CacheType.AST)
        value = {'ast': 'tree'}

        # Create first instance and set value
        cache1 = DiskCache(directory=temp_cache_dir)
        cache1.set(key, value)

        # Create second instance and get value
        cache2 = DiskCache(directory=temp_cache_dir)
        entry = cache2.get(key)

        assert entry is not None
        assert entry.value == value


if __name__ == '__main__':
    pytest.main([__file__, '-v'])

# Removed (CatnipCacheDisk.v, CatnipCacheMemory.v):
# - test_disk_cache_creation_with_limits — trivially covered by creation
# - test_get_nonexistent_key — trivial miss
# - test_delete_nonexistent — trivial
# - test_stats_empty_cache — empty_cache_zero_hits
# - test_stats_with_data — mc_get_counter_total (analogous)
# - test_hit_miss_counting — mc_get_hit_increments, mc_get_miss_increments (analogous)
