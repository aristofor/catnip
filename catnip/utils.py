# FILE: catnip/utils.py
"""Utility functions for Catnip."""

import xxhash


def compute_signature(source: str | bytes) -> str:
    """Compute a signature for the source or bytecode of a script.

    Uses xxHash64 for optimal speed in caching context.
    Collision probability is negligible in practice (2^-64).

    Args:
        source: Source code (str) or bytecode (bytes) to sign

    Returns:
        Hexadecimal signature (16 characters)

    Note:
        Alternatives if more robust hashes are needed (at the cost of performance):
        - xxhash.xxh128() for 128 bits (collision ~2^-128, slightly slower)
        - blake3.blake3() for cryptographic hash (overkill for caching)
    """
    if isinstance(source, str):
        source = source.encode("utf-8")

    h = xxhash.xxh64()
    h.update(source)
    return h.hexdigest()
