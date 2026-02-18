# FILE: catnip/registry.py
from ._rs import Registry as RegistryRust  # Rust implementation
from .context import Context
from .nodes import Broadcast
from .semantic.opcode import OpCode


class Registry(RegistryRust):
    """
    Main registry extending RegistryRust.

    All 52 operations implemented in Rust (100%):
    - Arithmetic, logical, bitwise, stack, literals, access
    - Control flow: if, while, for, block, return, break, continue, set_locals
    - Functions: lambda (factory), call (with TCO detection)
    - Pattern matching: match, match_pattern, match_tuple_pattern
    - ND operations: nd_empty_topos, nd_recursion, nd_map
    """

    def __init__(self, context: Context):
        # NOTE: Do NOT call super().__init__()
        # The Rust __new__ has already initialized the instance with context
        # PyO3 handles this automatically when Registry(context) is called

        # OP_LAMBDA is registered with True sentinel in Rust
        # Replace with the actual bound method
        self.internals[OpCode.OP_LAMBDA] = self._lambda

        # Register broadcast operation with string key for backward compatibility
        # Note: Broadcast nodes are handled directly by RegistryCore._handle_broadcast()
        operations = {
            "broadcast": self._broadcast,
        }

        self.internals.update(operations)

        # Also register with OpCode enum value
        self.internals.update(
            {
                OpCode.BROADCAST: self._broadcast,
            }
        )

    def _broadcast(self, target, operator, operand=None):
        """
        Stub for backward compatibility with explicit broadcast() calls.

        Creates a Broadcast node and delegates to _handle_broadcast.
        Most broadcast operations go directly through Broadcast nodes.
        """
        return self._handle_broadcast(Broadcast(target, operator, operand))
