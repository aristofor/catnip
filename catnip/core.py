# FILE: catnip/core.py
"""
Core modules - 100% Rust.

All implemented in Rust (catnip_rs/src/core/):
- scope.rs: O(1) HashMap-based scope resolution
- op.rs: Optimized Op node structure
- registry/: 52/52 operations
- function.rs: Function/Lambda + TCO trampoline

Available via: from catnip._rs import Scope, Op, Registry, Function, Lambda
"""

__all__ = ()
