# FILE: catnip/runtime.py
"""
Runtime introspection for Catnip.

Provides the `catnip` builtin namespace for inspecting interpreter state.
"""

from ._version import __version__


class CatnipRuntime:
    """
    Runtime introspection object exposed as `catnip` builtin.

    Provides read-only access to interpreter configuration and state.

    Usage in Catnip code:
        catnip.version      # "0.1.0"
        catnip.tco          # True/False
        catnip.optimize     # 0-3
        catnip.debug        # True/False
        catnip.modules      # ["math", "numpy"]
    """

    def __init__(self, pragma_context=None):
        """
        Initialize runtime introspection.

        :param pragma_context: PragmaContext instance (can be set later)
        """
        self._pragma_context = pragma_context
        self._modules = []  # Loaded Python modules

    def _set_pragma_context(self, pragma_context):
        """Set the pragma context (called after Catnip init)."""
        self._pragma_context = pragma_context

    def _add_module(self, name):
        """Register a loaded module."""
        if name not in self._modules:
            self._modules.append(name)

    # --- Read-only properties ---

    @property
    def version(self):
        """Catnip version string."""
        return __version__

    @property
    def tco(self):
        """True if tail-call optimization is enabled."""
        if self._pragma_context is None:
            return True  # Default
        return self._pragma_context.tco_enabled

    @property
    def optimize(self):
        """Optimization level (0-3)."""
        if self._pragma_context is None:
            return 3  # Default
        return self._pragma_context.optimize_level

    @property
    def debug(self):
        """True if debug mode is enabled."""
        if self._pragma_context is None:
            return False  # Default
        return self._pragma_context.debug_mode

    @property
    def jit(self):
        """True if JIT compilation is enabled."""
        if self._pragma_context is None:
            return True  # Default
        return self._pragma_context.jit_enabled

    @property
    def cache(self):
        """True if caching is enabled."""
        if self._pragma_context is None:
            return True  # Default
        return self._pragma_context.cache_enabled

    @property
    def modules(self):
        """List of loaded Python modules."""
        return list(self._modules)

    def __repr__(self):
        return "<CatnipRuntime>"
