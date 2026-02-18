# FILE: catnip/jit.py
"""
JIT compilation system for Catnip.

Implemented in Rust (catnip_rs/src/jit/).
"""

from catnip._rs import HotLoopDetector

__all__ = ('HotLoopDetector',)
