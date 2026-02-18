# FILE: catnip/context.py
import builtins as _builtins
import logging
import sys

# Always use Rust Scope
from catnip._rs import Scope

try:
    from catnip._rs import JIT_PURE_BUILTINS as _RUST_JIT_PURE_BUILTINS
except ImportError:
    _RUST_JIT_PURE_BUILTINS = None

from .exc import CatnipInternalError

# Configure logger format
logging.basicConfig(format="%(asctime)s.%(msecs)03d %(message)s", datefmt="%F %T", level=logging.WARNING)


# Module-level helper functions (VM-compatible, not closures)


def _list_ctor(*args):
    """Variadic list constructor: list(a, b, c) -> [a, b, c]"""
    if len(args) == 0:
        return []
    else:
        # Multiple args: always variadic behavior
        # This matches the grammar's list_literal behavior
        return _builtins.list(args)


def _set_ctor(*args):
    """Variadic set constructor: set(a, b, c) -> {a, b, c}"""
    if len(args) == 0:
        return set()
    else:
        # Multiple args: always variadic behavior
        # This matches the grammar's set_literal behavior
        return _builtins.set(args)


def _tuple_ctor(*args):
    """Variadic tuple constructor: tuple(a, b, c) -> (a, b, c)"""
    if len(args) == 0:
        return ()
    else:
        # Multiple args: always variadic behavior
        return _builtins.tuple(args)


def _write(*args):
    """Low-level write to stdout (no separator, no newline)."""
    message = "".join(str(arg) for arg in args)
    sys.stdout.write(message)
    sys.stdout.flush()


def _write_err(*args):
    """Low-level write to stderr (no separator, no newline)."""
    message = "".join(str(arg) for arg in args)
    sys.stderr.write(message)
    sys.stderr.flush()


def _print(*args):
    """High-level print (space separator, newline at end)."""
    if args:
        message = " ".join(str(arg) for arg in args)
        sys.stdout.write(message + "\n")
    else:
        sys.stdout.write("\n")
    sys.stdout.flush()


# Pickle-safe dummy functions for ND parallel mode
def _jit_dummy(func):
    """Dummy JIT wrapper for pickle (returns function unchanged)."""
    return func


def _pure_dummy(func):
    """Dummy pure wrapper for pickle (returns function unchanged)."""
    return func


def _cached_dummy(func, name=None, key_func=None, validator=None):
    """Dummy cached wrapper for pickle (returns function unchanged)."""
    return func


def _import_dummy(spec):
    """Dummy import wrapper for pickle."""
    raise RuntimeError("import() not available in this context")


class _CacheManagerDummy:
    """Dummy cache manager for pickle (no-op methods)."""

    def invalidate(self, func_name=None):
        pass

    def stats(self):
        return {}

    def enable(self):
        pass

    def disable(self):
        pass


def _debug_dummy(*args, sep=" "):
    """Dummy debug function for pickle (no-op)."""
    pass


# VM-compatible wrappers (not closures)
class _JitWrapper:
    """JIT wrapper that captures context."""

    def __init__(self, ctx):
        self.ctx = ctx

    def __call__(self, func):
        """Force immediate JIT compilation of a function."""
        from ._rs import Function, Lambda

        # Accept VMFunction in VM mode (already compiled to bytecode)
        if hasattr(func, 'vm_code'):
            # VMFunction: already compiled, return as-is
            return func

        if not isinstance(func, (Function, Lambda)):
            raise TypeError(f"jit() expects a Catnip function, got {type(func).__name__}")

        # Ensure JIT subsystem is initialized
        if not self.ctx.jit_enabled:
            self.ctx.jit_enabled = True
        self.ctx._init_jit()

        # Force compilation attempt
        func._try_jit_compile()

        return func

    def __reduce__(self):
        """Pickle support: don't pickle context, just return a callable that does nothing."""
        # In ND parallel mode, we can't pickle this - return a dummy function
        return (_jit_dummy, ())


class _PureWrapper:
    """Pure function marker that captures context."""

    def __init__(self, ctx):
        self.ctx = ctx

    def __call__(self, func):
        """Marque une fonction comme pure pour optimisation JIT inline."""
        return self.ctx.mark_pure(func)

    def __reduce__(self):
        """Pickle support: don't pickle context, just return a callable that does nothing."""
        return (_pure_dummy, ())


class _ImportWrapper:
    """Import wrapper that captures context for module loading."""

    def __init__(self, ctx):
        self.ctx = ctx
        self._loader = None

    def _get_loader(self):
        if self._loader is None:
            from .loader import ModuleLoader

            self._loader = ModuleLoader(self.ctx)
        return self._loader

    def __call__(self, spec):
        return self._get_loader().import_module(spec)

    def __reduce__(self):
        return (_import_dummy, ())


class _CachedWrapper:
    """Cached wrapper that captures context."""

    def __init__(self, ctx):
        self.ctx = ctx

    def __call__(self, func, name=None, key_func=None, validator=None):
        """Wrap a function with memoization support."""
        from .cachesys import CachedWrapper

        func_name = name or getattr(func, "__name__", "anonymous")
        return CachedWrapper(
            func,
            self.ctx.memoization,
            func_name,
            key_func=key_func,
            validator=validator,
        )

    def __reduce__(self):
        """Pickle support: return dummy function."""
        return (_cached_dummy, ())


class _CacheManager:
    """Memoization management interface accessible from Catnip code."""

    def __init__(self, ctx):
        self.ctx = ctx

    def invalidate(self, func_name=None):
        """Invalidate memoization entries for a function (or all if None)."""
        return self.ctx.memoization.invalidate(func_name)

    def stats(self):
        """Get memoization statistics."""
        return self.ctx.memoization.stats()

    def enable(self):
        """Enable memoization."""
        self.ctx.memoization.enable()

    def disable(self):
        """Disable memoization."""
        self.ctx.memoization.disable()

    def __reduce__(self):
        """Pickle support: return dummy object."""
        return (_CacheManagerDummy, ())


class _DebugWrapper:
    """Debug wrapper that captures logger."""

    def __init__(self, logger):
        self.logger = logger

    def __call__(self, *args, sep=" "):
        """Debug function accessible from Catnip code."""
        return self.logger.debug(*args, sep=sep)

    def __reduce__(self):
        """Pickle support: return dummy function."""
        return (_debug_dummy, ())


class MinimalLogger:
    """
    Minimal default logger for Catnip.
    Provides the basic interface (debug, info, warning, error, critical) via logging.
    Respects the logging level configured via logging.basicConfig.
    """

    def __init__(self):
        self._logger = logging.getLogger('catnip')

    def debug(self, *args, sep=" "):
        msg = sep.join(str(arg) for arg in args)
        self._logger.debug(msg)

    def info(self, *args, sep=" "):
        msg = sep.join(str(arg) for arg in args)
        self._logger.info(msg)

    def warning(self, *args, sep=" "):
        msg = sep.join(str(arg) for arg in args)
        self._logger.warning(msg)

    def error(self, *args, sep=" "):
        msg = sep.join(str(arg) for arg in args)
        self._logger.error(msg)

    def critical(self, *args, sep=" "):
        msg = sep.join(str(arg) for arg in args)
        self._logger.critical(msg)


class Context:
    """
    Stores the execution context, including locals, globals, and logger.
    """

    # Set of known pure builtins (no side effects, deterministic)
    # Synchronized with Rust JIT list when available.
    KNOWN_PURE_FUNCTIONS = frozenset(
        _RUST_JIT_PURE_BUILTINS
        or (
            'abs',
            'all',
            'any',
            'bool',
            'dict',
            'enumerate',
            'filter',
            'float',
            'int',
            'len',
            'list',
            'map',
            'max',
            'min',
            'range',
            'round',
            'set',
            'sorted',
            'str',
            'sum',
            'tuple',
            'zip',
        )
    )

    def __init__(self, globals=None, locals=None, logger=None, memoization=None):
        # Use provided logger or create minimal default
        if logger:
            self.logger = logger
        else:
            self.logger = MinimalLogger()

        # Memoization system for function execution results
        from .cachesys import Memoization

        self.memoization = memoization or Memoization()

        # Create debug wrapper (pickle-safe, available for all code paths)
        _debug = _DebugWrapper(self.logger)

        # Initialize globals with Python builtins if not provided
        if globals is None:
            import builtins

            self.globals = {
                "range": builtins.range,
                "len": builtins.len,
                "str": builtins.str,
                "int": builtins.int,
                "float": builtins.float,
                "list": _list_ctor,
                "dict": builtins.dict,
                "tuple": _tuple_ctor,
                "set": _set_ctor,
                "write": _write,
                "write_err": _write_err,
                "print": _print,
                "sum": builtins.sum,
                "min": builtins.min,
                "max": builtins.max,
                "abs": builtins.abs,
                "bool": builtins.bool,
                "round": builtins.round,
                "sorted": builtins.sorted,
                "reversed": builtins.reversed,
                "enumerate": builtins.enumerate,
                "zip": builtins.zip,
                "map": builtins.map,
                "filter": builtins.filter,
                "format": builtins.format,
                "repr": builtins.repr,
                "ascii": builtins.ascii,
                "cached": _CachedWrapper(self),
                "_cache": _CacheManager(self),
                "debug": _debug,
                "import": _ImportWrapper(self),
                "jit": _JitWrapper(self),
                "pure": _PureWrapper(self),
            }
        else:
            self.globals = dict(globals)

        # Always expose logger and debug in globals (even if custom globals provided)
        self.globals["logger"] = self.logger
        self.globals["debug"] = _debug

        self.locals = Scope()
        if locals:
            for k, v in locals.items():
                self.locals._set(k, v)

        self.result = None  # Last operation result

        # Track pure functions by name for optimization
        # Initialize with known pure builtins
        self.pure_functions = set(self.KNOWN_PURE_FUNCTIONS)

        # Tail-call optimization
        self.tco_enabled = True  # Enable/disable TCO (controlled by pragma)

        # JIT compilation system
        self.jit_enabled = False  # Enable via pragma("jit", True)
        self.jit_all = False  # Compile ALL functions immediately via pragma("jit", "all")
        self.jit_detector = None
        self.jit_executor = None

        self.jit_matcher = None
        self.jit_codegen = None

        # ND-recursion (concurrence structurelle)
        self.nd_scheduler = None  # Lazy init: NDScheduler instance
        self.nd_workers = 0  # 0 = auto-detect, controlled by pragma
        self.nd_mode = 'sequential'  # 'sequential' ou 'parallel'

        # Error context for stack traces
        self.sourcemap = None  # SourceMap instance (set by parser)
        self.call_stack = []  # List of CatnipFrame for traceback

    def _init_jit(self):
        """Lazy initialization of JIT subsystem (only when enabled)."""
        if self.jit_detector is None:
            try:
                from .jit import HotLoopDetector

                self.jit_detector = HotLoopDetector(threshold=100)
                # Note: Full JIT (codegen, executor) is now in Rust (catnip_rs/src/jit/)
                # These fields are kept for future integration
            except (ImportError, RuntimeError) as e:
                # JIT not available (wrong platform or dependencies missing)
                self.logger.warning(f"JIT compilation unavailable: {e}")
                self.jit_enabled = False

    def push_scope(self, scope=None, parent=None):
        """
        Push a new scope on the stack.

        :param scope: Initial symbols (dict or Scope) for the new scope
        :param parent: Explicit parent scope (defaults to current scope)
        """
        # push a frame and init symbols
        self.locals.push_frame()
        if isinstance(scope, dict):
            for k, v in scope.items():
                self.locals._set(k, v)
        elif hasattr(scope, '_symbols'):
            for k, v in scope._symbols.items():
                self.locals._set(k, v)

    def pop_scope(self):
        """
        Pop the top scope from the stack.
        """
        if self.locals.depth() > 1:
            self.locals.pop_frame()
            return None
        else:
            raise CatnipInternalError("Cannot pop the global scope.")

    def capture_scope(self):
        """
        Capture current scope for closure creation.

        Returns:
            A snapshot dict of current variables
        """
        return self.locals.snapshot()

    def push_scope_with_capture(self, captured):
        """
        Push a new scope with captured variables from a closure.

        Args:
            captured: A snapshot dict from capture_scope()
        """
        # push frame with captured variables
        self.locals.push_frame_with_captures(captured)

    def sync_captures(self, captured):
        """
        Sync modified variables back to the captured dict.

        Call this before pop_scope() to persist closure state.

        Args:
            captured: The captured dict from capture_scope()
        """
        self.locals.sync_to_captures(captured)

    def pop_scope_with_sync(self, captured):
        """
        Sync captures and pop scope in one operation.

        Args:
            captured: The captured dict from closure_scope (for sync)
        """
        before = None
        before = dict(captured)
        self.sync_captures(captured)
        result = self.pop_scope()
        if captured and before is not None:
            missing = object()
            for name, value in captured.items():
                if before.get(name, missing) != value:
                    self.locals._set(name, value)
        return result

    def mark_pure(self, func):
        """
        Marque une fonction comme pure pour optimisation JIT.

        Une fonction pure :
        - N'a pas d'effets de bord
        - Retourne toujours le même résultat pour les mêmes arguments
        - Peut être inlinée dans le JIT pour meilleures performances

        Args:
            func: Fonction Catnip (Function ou Lambda) à marquer comme pure

        Returns:
            func: La fonction inchangée (pour usage comme décorateur)
        """
        # Marquer dans le set de fonctions pures (pour info/debug)
        # Note: __name__ peut ne pas exister sur fonctions Catnip
        if hasattr(func, '__name__'):
            self.pure_functions.add(func.__name__)

        # Marquer le CodeObject si disponible
        # Catnip functions have vm_code attribute (not code)
        code_obj = None
        if hasattr(func, 'vm_code'):
            code_obj = func.vm_code
        elif hasattr(func, 'code'):
            code_obj = func.code

        if code_obj is not None:
            try:
                code_obj.is_pure = True
            except AttributeError:
                # CodeObject n'a pas is_pure (ancienne version)
                pass

        return func
