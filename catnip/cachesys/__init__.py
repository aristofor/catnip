# FILE: catnip/cachesys/__init__.py
"""
Cache system for Catnip - Fully migrated to Rust.

All cache functionality is now implemented in Rust (catnip._rs).
This module provides backward-compatible re-exports.

Host applications can implement custom cache backends by implementing
the cache protocol (get, set, delete, clear, exists, stats methods).
"""

# Import all cache classes from Rust
from catnip._rs import (
    CacheEntry,
    CacheKey,
    CatnipCache,
    DiskCache,
    Memoization,
    MemoryCache,
)
from catnip._rs import CacheType as _RustCacheType


class _CacheTypeMeta(type):
    """Metaclass to make CacheType class iterable."""

    def __iter__(cls):
        """Iterate over all cache type variants."""
        return iter(
            [
                _RustCacheType.SOURCE,
                _RustCacheType.AST,
                _RustCacheType.BYTECODE,
                _RustCacheType.RESULT,
            ]
        )


class CacheType(metaclass=_CacheTypeMeta):
    """
    Cache type enum wrapper for iteration support.

    This wrapper allows 'for cache_type in CacheType:' pattern
    by implementing iteration at the class level via metaclass.
    """

    SOURCE = _RustCacheType.SOURCE
    AST = _RustCacheType.AST
    BYTECODE = _RustCacheType.BYTECODE
    RESULT = _RustCacheType.RESULT


# Protocol for custom cache backends
from .base import CacheBackend  # noqa: E402

# Legacy Python imports (deprecated, kept for backward compatibility)
from .memoization import CachedWrapper  # noqa: E402

__all__ = (
    'CacheBackend',
    'CachedWrapper',
    'CacheEntry',
    'CacheKey',
    'CacheType',
    'CatnipCache',
    'DiskCache',
    'Memoization',
    'MemoryCache',
)
