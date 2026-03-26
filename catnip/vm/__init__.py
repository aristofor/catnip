# FILE: catnip/vm/__init__.py
"""
Catnip Virtual Machine package.

Provides a stack-based VM that eliminates Python stack growth
during deep recursion by using an explicit operand stack.
"""

from .opcodes import STACK_EFFECT, VMOp

__all__ = ('STACK_EFFECT', 'VMOp')


# Lazy imports (used by Rust debug modules via py.import)
def __getattr__(name):
    if name == 'CodeObject':
        from catnip._rs import CodeObject

        return CodeObject
    if name == 'VMFunction':
        from catnip._rs import VMFunction

        return VMFunction
    if name == 'ClosureScope':
        from catnip._rs import ClosureScope

        return ClosureScope
    if name == 'Compiler':
        from catnip._rs import Compiler

        return Compiler
    if name == 'VMExecutor':
        from .executor import VMExecutor

        return VMExecutor
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
