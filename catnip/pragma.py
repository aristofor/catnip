# FILE: catnip/pragma.py
"""Pragma system for Catnip. All components implemented in Rust."""

from catnip._rs import Pragma, PragmaContext, PragmaType, pragma_directives

# Canonical directive names from Rust
PRAGMA_DIRECTIVES = frozenset(pragma_directives())

# Pragma directive -> (PragmaContext attr_name, Python type)
PRAGMA_ATTRS = {
    'tco': ('tco_enabled', bool),
    'debug': ('debug_mode', bool),
    'cache': ('cache_enabled', bool),
    'optimize': ('optimize_level', int),
    'jit': ('jit_enabled', bool),
    'jit_all': ('jit_all', bool),
    'nd_mode': ('nd_mode', str),
    'nd_workers': ('nd_workers', int),
    'nd_memoize': ('nd_memoize', bool),
    'nd_batch_size': ('nd_batch_size', int),
    'batch_size': ('nd_batch_size', int),
}

__all__ = ('Pragma', 'PragmaContext', 'PragmaType', 'PRAGMA_ATTRS', 'PRAGMA_DIRECTIVES')
