# FILE: catnip/cachesys/memoization.py
"""
Memoization system for Catnip.

Stores function execution results based on their arguments.
Useful for build scripts and pure functions.
Note: This is memoization (storing function results), not compilation caching.
"""

from typing import Any, Callable, Optional

# Process-level Memoization backing the standalone `cached` builtin (lazy).
_STANDALONE_MEMO = None


def standalone_cached(func, name=None, key_func=None, validator=None):
    """`cached` builtin for the standalone VM host.

    VMHost looks this function up by name when seeding globals (vm/host.rs)
    and injects it as `cached`. Backed by one process-level Rust Memoization
    instance -- the standalone binary runs one VM per process.
    """
    from catnip._rs import Memoization

    global _STANDALONE_MEMO
    if _STANDALONE_MEMO is None:
        _STANDALONE_MEMO = Memoization()
    func_name = name or getattr(func, '__name__', 'anonymous')
    return CachedWrapper(func, _STANDALONE_MEMO, func_name, key_func=key_func, validator=validator)


class CachedWrapper:
    """
    Wrapper for a cached function.

    Intercepts calls and checks cache before executing.
    """

    def __init__(
        self,
        func: Any,
        cache: Any,  # catnip._rs.Memoization
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
            cache_args = cache_key if isinstance(cache_key, tuple) else (cache_key,)
            cache_kwargs = {}
        else:
            cache_args = args
            cache_kwargs = kwargs

        # Check cache - returns CacheEntry on hit, None on miss
        entry = self.cache.get_entry(self.func_name, cache_args, cache_kwargs)
        is_miss = entry is None

        # Validate cache if validator is provided
        if not is_miss and self.validator is not None:
            if not self.validator(entry.value, *args, **kwargs):
                self.cache.invalidate_key(self.func_name, cache_args, cache_kwargs)
                is_miss = True

        if not is_miss:
            return entry.value

        # Cache miss: execute and store
        result = self.func(*args, **kwargs)
        self.cache.set(self.func_name, cache_args, cache_kwargs, result)
        return result

    def __repr__(self):
        return f"<cached {self.func_name}>"
