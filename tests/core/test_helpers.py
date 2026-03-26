# FILE: tests/core/test_helpers.py
"""
Shared test helper functions for Catnip tests.

Provides utilities to avoid repetition in test files.
"""

import os

from catnip import Catnip


def exec_catnip(code, vm_mode=None):
    """
    Helper to execute Catnip code (parse + execute).

    Avoids repetition of the 3-line pattern in simple tests.

    Usage:
        from test_helpers import exec_catnip

        def test_something():
            result = exec_catnip("1 + 2")
            assert result == 3
    """
    if vm_mode is None:
        executor = os.environ.get('CATNIP_EXECUTOR', 'ast')
        vm_mode = 'on' if executor == 'vm' else 'off' if executor == 'ast' else executor
    c = Catnip(vm_mode=vm_mode)
    c.parse(code)
    return c.execute()


def parse_ir(code, vm_mode=None):
    """
    Helper to get IR at level 1 (after transformation, before semantic analysis).

    Usage:
        from test_helpers import parse_ir

        def test_transform():
            ir = parse_ir("x + 1")
            assert ir[0].ident == OpCode.ADD
    """
    if vm_mode is None:
        executor = os.environ.get('CATNIP_EXECUTOR', 'ast')
        vm_mode = 'on' if executor == 'vm' else 'off' if executor == 'ast' else executor
    c = Catnip(vm_mode=vm_mode)
    return c.parse(code, semantic=False)
