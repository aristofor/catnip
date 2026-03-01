# FILE: catnip/cachesys/base.py
"""
Cache backend protocol for host applications.

IMPORTANT: All core cache functionality is in Rust (catnip._rs).
This module defines the protocol that custom cache backends must implement.
"""

from typing import Any, Optional, Protocol

from catnip._rs import CacheEntry, CacheKey


class CacheBackend(Protocol):
    """
    Protocol for cache backend implementations.

    Host applications can implement this protocol to provide custom caching.
    All methods must match these signatures.

    Built-in implementations (in Rust):
    - MemoryCache: In-memory FIFO cache with hit/miss stats
    - DiskCache: Persistent disk cache with TTL and size management

    Example custom implementation:
        class MyRedisCache:
            def get(self, key: CacheKey) -> Optional[CacheEntry]: ...
            def set(self, key: CacheKey, value: Any, metadata: dict = None) -> None: ...
            def delete(self, key: CacheKey) -> bool: ...
            def clear(self) -> None: ...
            def exists(self, key: CacheKey) -> bool: ...
            def stats(self) -> dict: ...
    """

    def get(self, key: CacheKey) -> Optional[CacheEntry]:
        """
        Retrieve an entry from the cache.

        Args:
            key: Cache key with content hash and options

        Returns:
            CacheEntry if found, None if cache miss
        """
        ...

    def set(self, key: CacheKey, value: Any, metadata: dict = None) -> None:
        """
        Store an entry in the cache.

        Args:
            key: Cache key with content hash and options
            value: Value to cache (usually AST or bytecode)
            metadata: Optional metadata dict
        """
        ...

    def delete(self, key: CacheKey) -> bool:
        """
        Delete an entry from the cache.

        Args:
            key: Cache key to delete

        Returns:
            True if entry existed and was deleted, False otherwise
        """
        ...

    def clear(self) -> None:
        """
        Clear the entire cache.

        Removes all entries and resets statistics.
        """
        ...

    def exists(self, key: CacheKey) -> bool:
        """
        Check if a key exists in the cache.

        Args:
            key: Cache key to check

        Returns:
            True if key exists, False otherwise
        """
        ...

    def stats(self) -> dict:
        """
        Return cache statistics.

        Should include at least:
        - 'backend': str (backend name)
        - 'size': int (number of entries)
        - 'hits': int (cache hits)
        - 'misses': int (cache misses)

        Optional:
        - 'max_size': int (maximum entries)
        - 'hit_rate': str (hit rate percentage)
        - 'ttl_seconds': int (TTL in seconds)
        - 'cache_dir': str (cache directory path)

        Returns:
            dict with cache statistics
        """
        ...


__all__ = ('CacheBackend',)
