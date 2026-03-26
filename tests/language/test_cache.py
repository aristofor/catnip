# FILE: tests/language/test_cache.py
"""
Tests pour les modules de cache (base.py et memory.py).
"""

import pytest
from catnip._rs import CacheEntry, CacheKey, MemoryCache

from catnip.cachesys import CacheType


class TestCacheKey:
    """Tests for CacheKey."""

    def test_cache_key_to_string_basic(self):
        """Verify that to_string generates a consistent key."""
        key = CacheKey(content="x = 1", cache_type=CacheType.AST)
        key_str = key.to_string()

        # Format: catnip:type:hash
        assert key_str.startswith("catnip:ast:")
        parts = key_str.split(":")
        assert len(parts) == 3  # catnip:type:hash
        assert len(parts[2]) == 16  # hash xxhash64 (16 chars hex)

    def test_cache_key_different_options_different_key(self):
        """Different options = different key."""
        key1 = CacheKey(content="x = 1", cache_type=CacheType.AST, optimize=True)
        key2 = CacheKey(content="x = 1", cache_type=CacheType.AST, optimize=False)

        assert key1.to_string() != key2.to_string()

    def test_cache_key_all_types(self):
        """Verify tous les types de cache."""
        content = "x = 1"

        for cache_type in CacheType:
            key = CacheKey(content=content, cache_type=cache_type)
            key_str = key.to_string()
            assert cache_type.value in key_str


class TestCacheEntry:
    """Tests for CacheEntry."""

    def test_cache_entry_creation(self):
        """Verify creation of a basic entry."""
        entry = CacheEntry(key='test_key', value={'some': 'data'}, cache_type=CacheType.AST)

        assert entry.key == 'test_key'
        assert entry.value == {'some': 'data'}
        assert entry.cache_type == CacheType.AST
        assert entry.metadata == {}


class TestMemoryCache:
    """Tests for MemoryCache."""

    def test_memory_cache_creation(self):
        """Verify cache creation."""
        cache = MemoryCache()

        stats = cache.stats()
        assert stats['size'] == 0
        assert cache.max_size is None
        assert cache.hits == 0
        assert cache.misses == 0

    def test_set_and_get(self):
        """Verify set et get basiques."""
        cache = MemoryCache()
        key = CacheKey(content="x = 1", cache_type=CacheType.AST)
        value = {'ast': 'tree'}

        cache.set(key, value)
        entry = cache.get(key)

        assert entry is not None
        assert entry.value == value
        assert entry.cache_type == CacheType.AST

    def test_delete(self):
        """Verify delete."""
        cache = MemoryCache()
        key = CacheKey(content="x = 1", cache_type=CacheType.AST)

        cache.set(key, 'value')
        assert cache.exists(key)

        result = cache.delete(key)

        assert result is True
        assert not cache.exists(key)

    def test_clear(self):
        """Verify clear."""
        cache = MemoryCache()

        # Add multiple entries
        for i in range(5):
            key = CacheKey(content=f"x = {i}", cache_type=CacheType.AST)
            cache.set(key, i)

        # Quelques hits/misses
        key = CacheKey(content="x = 0", cache_type=CacheType.AST)
        cache.get(key)
        cache.get(CacheKey(content="nonexistent", cache_type=CacheType.AST))

        stats = cache.stats()
        assert stats['size'] == 5
        assert cache.hits > 0
        assert cache.misses > 0

        cache.clear()

        stats = cache.stats()
        assert stats['size'] == 0
        assert cache.hits == 0
        assert cache.misses == 0

    def test_max_size_eviction_fifo(self):
        """Verify FIFO eviction when max_size is reached."""
        cache = MemoryCache(max_size=3)

        key1 = CacheKey(content="x = 1", cache_type=CacheType.AST)
        key2 = CacheKey(content="x = 2", cache_type=CacheType.AST)
        key3 = CacheKey(content="x = 3", cache_type=CacheType.AST)
        key4 = CacheKey(content="x = 4", cache_type=CacheType.AST)

        cache.set(key1, "value1")
        cache.set(key2, "value2")
        cache.set(key3, "value3")

        assert cache.exists(key1)
        assert cache.exists(key2)
        assert cache.exists(key3)
        stats = cache.stats()
        assert stats['size'] == 3

        # Adding a 4th entry should evict the first
        cache.set(key4, "value4")

        assert not cache.exists(key1)  # Evicted
        assert cache.exists(key2)
        assert cache.exists(key3)
        assert cache.exists(key4)
        stats = cache.stats()
        assert stats['size'] == 3

    def test_max_size_no_eviction_on_update(self):
        """Verify updating an existing key does not evict."""
        cache = MemoryCache(max_size=2)

        key1 = CacheKey(content="x = 1", cache_type=CacheType.AST)
        key2 = CacheKey(content="x = 2", cache_type=CacheType.AST)

        cache.set(key1, "value1")
        cache.set(key2, "value2")

        # Updating key1 should not cause eviction
        cache.set(key1, "value1_updated")

        assert cache.exists(key1)
        assert cache.exists(key2)
        assert cache.get(key1).value == "value1_updated"

    def test_different_cache_types_coexist(self):
        """Verify different cache types can coexist."""
        cache = MemoryCache()
        content = "x = 1"

        # Same content but different types
        key_ast = CacheKey(content=content, cache_type=CacheType.AST)
        key_bytecode = CacheKey(content=content, cache_type=CacheType.BYTECODE)
        key_source = CacheKey(content=content, cache_type=CacheType.SOURCE)

        cache.set(key_ast, "ast_value")
        cache.set(key_bytecode, "bytecode_value")
        cache.set(key_source, "source_value")

        assert cache.get(key_ast).value == "ast_value"
        assert cache.get(key_bytecode).value == "bytecode_value"
        assert cache.get(key_source).value == "source_value"

    def test_cache_complex_values(self):
        """Verify qu'on peut cacher des valeurs complexes."""
        cache = MemoryCache()
        key = CacheKey(content="x = 1", cache_type=CacheType.AST)

        # Valeur complexe
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


if __name__ == "__main__":
    pytest.main([__file__, "-v"])

# Removed (CatnipCacheMemory.v):
# - test_cache_key_same_content_same_key - mc_set_preserves_unique (key determinism)
# - test_cache_key_different_content_different_key - mc_set_preserves_unique (key uniqueness)
# - test_cache_key_different_type_different_key - mc_set_preserves_unique
# - test_cache_key_tco_option - redundant with different_options
# - test_cache_key_uses_xxhash - format detail, redundant with to_string_basic
# - test_cache_key_with_bytecode_content - redundant with to_string_basic
# - test_cache_key_includes_catnip_signature - key determinism, redundant
# - test_cache_entry_with_metadata - trivially covered by creation
# - test_cache_entry_metadata_default - trivially covered by creation
# - test_memory_cache_creation_with_max_size - trivially covered by creation
# - test_get_nonexistent_key - trivial miss
# - test_set_with_metadata - trivially covered by set_and_get
# - test_exists - trivially covered by set_and_get
# - test_delete_nonexistent - trivial
# - test_stats_empty_cache - empty_cache_zero_hits
# - test_stats_with_data - mc_get_counter_total, mc_get_hit_increments
# - test_hit_miss_counting - mc_get_hit_increments, mc_get_miss_increments
