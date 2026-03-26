# FILE: catnip/utils.py
"""Utility functions for Catnip."""

import xxhash


def compute_signature(source: str | bytes) -> str:
    """Compute a signature for the source or bytecode of a script.

    Uses xxHash64 for optimal speed in caching context.
    """
    if isinstance(source, str):
        source = source.encode('utf-8')

    h = xxhash.xxh64()
    h.update(source)
    return h.hexdigest()
