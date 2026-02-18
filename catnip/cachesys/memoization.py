# FILE: catnip/cachesys/memoization.py
"""
Memoization system for Catnip.

Stores function execution results based on their arguments.
Useful for build scripts and pure functions.
Note: This is memoization (storing function results), not compilation caching.
"""

from typing import Any, Callable, Optional

from catnip._rs import CacheEntry, CacheKey, CacheType, MemoryCache

from .base import CacheBackend


class Memoization:
    """
    Memoization system for function execution results.

    Uses argument hash as cache key.
    Compatible with Python hooks for custom backends.
    """

    def __init__(self, backend: Optional[CacheBackend] = None):
        """
        :param backend: Cache backend to use (default: MemoryCache)
        """
        self.backend = backend or MemoryCache()
        self._enabled = True
        # Index: func_name -> list of cache keys
        self._func_index: dict[str, list[CacheKey]] = {}

    def get(self, func_name: str, args: tuple, kwargs: dict) -> Optional[Any]:
        """
        Retrieve result from cache.

        :param func_name: Function name
        :param args: Positional arguments
        :param kwargs: Keyword arguments
        :return: Cached result or None if cache miss
        """
        if not self._enabled:
            return None

        key = self._make_key(func_name, args, kwargs)
        entry: Optional[CacheEntry] = self.backend.get(key)
        return entry.value if entry else None

    def set(self, func_name: str, args: tuple, kwargs: dict, result: Any) -> None:
        """
        Store result in the cache.

        :param func_name: Function name
        :param args: Positional arguments
        :param kwargs: Keyword arguments
        :param result: Result to cache
        """
        if not self._enabled:
            return

        key = self._make_key(func_name, args, kwargs)
        metadata = dict(
            func_name=func_name,
            args_count=len(args),
            kwargs_keys=list(kwargs.keys()),
        )
        self.backend.set(key, result, metadata)

        # Update func_name index
        if func_name not in self._func_index:
            self._func_index[func_name] = []
        self._func_index[func_name].append(key)

    def invalidate(self, func_name: Optional[str] = None) -> int:
        """
        Invalidate cache entries.

        :param func_name: Function name to invalidate (None = invalidate all)
        :return: Number of entries invalidated
        """
        if func_name is None:
            # Invalidate entire cache
            count = self.backend.stats()['size']
            self.backend.clear()
            self._func_index.clear()
            return count
        else:
            # Invalidate only this function using index
            if func_name not in self._func_index:
                return 0

            keys = self._func_index[func_name]
            count = 0
            for key in keys:
                if self.backend.delete(key):
                    count += 1

            # Remove from index
            del self._func_index[func_name]
            return count

    def invalidate_key(self, func_name: str, args: tuple, kwargs: dict) -> bool:
        """
        Invalidate a specific cache entry.

        :param func_name: Function name
        :param args: Positional arguments
        :param kwargs: Keyword arguments
        :return: True if entry existed
        """
        key = self._make_key(func_name, args, kwargs)
        deleted = self.backend.delete(key)

        # Update index
        if deleted and func_name in self._func_index:
            try:
                self._func_index[func_name].remove(key)
            except ValueError:
                pass
            # Clean up empty lists
            if not self._func_index[func_name]:
                del self._func_index[func_name]

        return deleted

    def enable(self) -> None:
        """Enable the cache."""
        self._enabled = True

    def disable(self) -> None:
        """Disable the cache."""
        self._enabled = False

    def stats(self) -> dict:
        """Return cache statistics."""
        base_stats = self.backend.stats()
        base_stats['enabled'] = self._enabled
        return base_stats

    def _make_key(self, func_name: str, args: tuple, kwargs: dict) -> CacheKey:
        """
        Create a cache key from function name and arguments.

        :param func_name: Function name
        :param args: Positional arguments
        :param kwargs: Keyword arguments
        :return: CacheKey
        """
        # Serialize arguments to string for hashing
        # Note: we use repr() which works for most Python types
        args_str = ','.join(repr(arg) for arg in args)
        kwargs_str = ','.join(f"{k}={repr(v)}" for k, v in sorted(kwargs.items()))

        # Combine function + args in content
        content = f"{func_name}({args_str}{(',' if args_str and kwargs_str else '')}{kwargs_str})"

        return CacheKey(
            content=content,
            cache_type=CacheType.RESULT,
            optimize=False,
            tco_enabled=False,
        )

    def __repr__(self) -> str:
        return f"Memoization(backend={self.backend.__class__.__name__}, enabled={self._enabled})"


class CachedWrapper:
    """
    Wrapper for a cached function.

    Intercepts calls and checks cache before executing.
    """

    def __init__(
        self,
        func: Any,
        cache: "Memoization",
        func_name: str,
        key_func: Optional[Callable] = None,
        validator: Optional[Callable] = None,
    ):
        self.func = func
        self.cache = cache
        self.func_name = func_name
        self.key_func = key_func  # Function to generate custom key
        self.validator = validator  # Function to validate cache

    def __call__(self, *args, **kwargs):
        # Generate cache key (custom or standard)
        if self.key_func is not None:
            cache_key = self.key_func(*args, **kwargs)
        else:
            cache_key = (args, kwargs)

        # Check cache
        cached_result = self.cache.get(
            self.func_name,
            cache_key if isinstance(cache_key, tuple) else (cache_key,),
            {},
        )

        # Validate cache if validator is provided
        if cached_result is not None and self.validator is not None:
            if not self.validator(cached_result, *args, **kwargs):
                # Cache invalid, delete and recalculate
                self.cache.invalidate_key(
                    self.func_name,
                    cache_key if isinstance(cache_key, tuple) else (cache_key,),
                    {},
                )
                cached_result = None

        if cached_result is not None:
            return cached_result

        # Cache miss: execute and store
        result = self.func(*args, **kwargs)
        self.cache.set(
            self.func_name,
            cache_key if isinstance(cache_key, tuple) else (cache_key,),
            {},
            result,
        )
        return result

    def __repr__(self):
        return f"<cached {self.func_name}>"
