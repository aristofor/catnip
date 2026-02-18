# FILE: catnip/repl/__init__.py
"""
Catnip REPL package.

Minimal REPL for Python module integration (-m/--module).
For pure Catnip code, use the Rust REPL (catnip-repl) instead.
"""

from catnip._rs import (
    parse_repl_command,
    preprocess_multiline,
    should_continue_multiline,
)

from .minimal import MinimalREPL

__all__ = (
    'MinimalREPL',
    'should_continue_multiline',
    'preprocess_multiline',
    'parse_repl_command',
)
