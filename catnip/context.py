# FILE: catnip/context.py
import builtins as _builtins
import logging
import sys

from catnip import _rs
from catnip._rs import JIT_PURE_BUILTINS, CatnipMeta, ContextBase

from .exc import CatnipTypeError


class _LiveStderrHandler(logging.StreamHandler):
    """StreamHandler resolving sys.stderr at emit time.

    configure_logging() runs once per process; a handler that captured
    sys.stderr at creation would keep writing to that stream forever,
    even after a redirection (pytest capture, Click CliRunner). Same
    pattern as the stdlib's logging._StderrHandler.
    """

    def __init__(self):
        logging.Handler.__init__(self)

    @property
    def stream(self):
        return sys.stderr


def configure_logging():
    """Attach catnip's CLI log format to the 'catnip' logger hierarchy.

    Called from application entry points only, never at import time: a
    library must not alter the host's logging (root basicConfig would turn
    user basicConfig calls into silent no-ops; an import-time handler with
    propagate=False would prevent the host from routing catnip messages).
    Idempotent.
    """
    logger = logging.getLogger('catnip')
    if logger.handlers:
        return
    handler = _LiveStderrHandler()
    handler.setFormatter(logging.Formatter('%(asctime)s.%(msecs)03d %(message)s', datefmt='%F %T'))
    logger.addHandler(handler)
    logger.setLevel(logging.WARNING)
    logger.propagate = False


# Module-level helper functions (VM-compatible, not closures)


def _list_ctor(*args):
    """Variadic list constructor: list(a, b, c) -> [a, b, c]"""
    if len(args) == 0:
        return []
    else:
        return _builtins.list(args)


def _set_ctor(*args):
    """Variadic set constructor: set(a, b, c) -> {a, b, c}"""
    if len(args) == 0:
        return set()
    else:
        return _builtins.set(args)


def _tuple_ctor(*args):
    """Variadic tuple constructor: tuple(a, b, c) -> (a, b, c)"""
    if len(args) == 0:
        return ()
    else:
        return _builtins.tuple(args)


# Pickle-safe dummy functions for ND parallel mode
def _jit_dummy(func):
    return func


def _pure_dummy(func):
    return func


def _cached_dummy(func, name=None, key_func=None, validator=None):
    return func


def _fold(xs, init, f):
    acc = init
    for x in xs:
        acc = f(acc, x)
    return acc


def _reduce(xs, f):
    it = iter(xs)
    try:
        acc = next(it)
    except StopIteration:
        raise ValueError("reduce() of empty sequence with no initial value")
    for x in it:
        acc = f(acc, x)
    return acc


def _import_dummy(spec, *names):
    raise RuntimeError("import() not available in this context")


def _parse_import_name(raw):
    if not isinstance(raw, str):
        raise CatnipTypeError(f"import name must be a string, got {type(raw).__name__}")
    if not raw:
        raise ValueError("import name cannot be empty")
    name, _, alias = raw.partition(':')
    if not name:
        raise ValueError(f"empty name in import spec '{raw}'")
    if _ and not alias:
        raise ValueError(f"empty alias in import spec '{raw}'")
    return (name, alias) if alias else (name, name)


class _CacheManagerDummy:
    def invalidate(self, func_name=None):
        pass

    def stats(self):
        return {}

    def enable(self):
        pass

    def disable(self):
        pass


def _debug_dummy(*args, sep=' '):
    pass


# VM-compatible wrappers (not closures)
class _JitWrapper:
    def __init__(self, ctx):
        self.ctx = ctx

    def __call__(self, func):
        from ._rs import Function, Lambda

        if hasattr(func, 'vm_code'):
            return func

        if not isinstance(func, (Function, Lambda)):
            raise TypeError(f"jit() expects a Catnip function, got {type(func).__name__}")

        if not self.ctx.jit_enabled:
            self.ctx.jit_enabled = True

        # No per-function compilation in AST mode: the JIT is a VM feature (hot
        # loop detection compiles in the VM). Marking jit_enabled is enough; the
        # wrapper just returns the callable.
        return func

    def __reduce__(self):
        return (_jit_dummy, ())


class _PureWrapper:
    def __init__(self, ctx):
        self.ctx = ctx

    def __call__(self, func):
        return self.ctx.mark_pure(func)

    def __reduce__(self):
        return (_pure_dummy, ())


class _ImportWrapper:
    def __init__(self, ctx):
        self.ctx = ctx
        self._loader = None
        # Rust ImportLoader, wired by Catnip.__init__: namespace loading is
        # delegated to it (native plugins, rs:: cache isolation); the
        # selective/wild binding below stays here, in the Python globals
        self._rust_import = None

    def _get_loader(self):
        if self._loader is None:
            from .loader import ModuleLoader

            self._loader = ModuleLoader(self.ctx)
        return self._loader

    def __call__(self, spec, *names, wild=False, protocol=None):
        from pathlib import Path

        caller_dir = None
        meta = self.ctx.globals.get('META')
        if meta is not None:
            try:
                caller_dir = Path(meta.file).parent
            except AttributeError:
                pass
        if self._rust_import is not None:
            from .compat import _map_exception

            try:
                namespace = self._rust_import(
                    spec, protocol=protocol, caller_dir=str(caller_dir) if caller_dir is not None else None
                )
            except RuntimeError as e:
                # Same RuntimeError -> Catnip* mapping the VM applies
                raise _map_exception(e) from None
        else:
            namespace = self._get_loader().import_module(spec, caller_dir=caller_dir, protocol=protocol)
        if names and wild:
            raise CatnipTypeError("cannot combine selective names with wild=True")
        if names:
            resolved = []
            for raw in names:
                name, alias = _parse_import_name(raw)
                if not hasattr(namespace, name):
                    raise AttributeError(f"module '{spec}' has no attribute '{name}'")
                resolved.append((alias, getattr(namespace, name)))
            for alias, value in resolved:
                self.ctx.globals[alias] = value
            return None
        if wild:
            for name in dir(namespace):
                if name.startswith('_') or name == 'META':
                    continue
                self.ctx.globals[name] = getattr(namespace, name)
            return None
        return namespace

    def __reduce__(self):
        return (_import_dummy, ())


class _CachedWrapper:
    def __init__(self, ctx):
        self.ctx = ctx

    def __call__(self, func, name=None, key_func=None, validator=None):
        from .cachesys import CachedWrapper

        func_name = name or getattr(func, '__name__', 'anonymous')
        return CachedWrapper(
            func,
            self.ctx.memoization,
            func_name,
            key_func=key_func,
            validator=validator,
        )

    def __reduce__(self):
        return (_cached_dummy, ())


class _CacheManager:
    def __init__(self, ctx):
        self.ctx = ctx

    def invalidate(self, func_name=None):
        return self.ctx.memoization.invalidate(func_name)

    def stats(self):
        return self.ctx.memoization.stats()

    def enable(self):
        self.ctx.memoization.enable()

    def disable(self):
        self.ctx.memoization.disable()

    def __reduce__(self):
        return (_CacheManagerDummy, ())


class _DebugWrapper:
    def __init__(self, logger):
        self.logger = logger

    def __call__(self, *args, sep=' '):
        msg = sep.join(str(a) for a in args)
        return self.logger.debug(msg)

    def __reduce__(self):
        return (_debug_dummy, ())


class MinimalLogger:
    def __init__(self):
        self._logger = logging.getLogger('catnip')

    def debug(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        self._logger.debug(msg)

    def info(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        self._logger.info(msg)

    def warning(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        self._logger.warning(msg)

    def error(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        self._logger.error(msg)

    def critical(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        self._logger.critical(msg)


class Context(ContextBase):
    """Execution context - inherits core fields and scope ops from Rust ContextBase."""

    KNOWN_PURE_FUNCTIONS = frozenset(JIT_PURE_BUILTINS)

    def __init__(self, globals=None, locals=None, logger=None, memoization=None):
        super().__init__()

        # Logger
        self.logger = logger if logger else MinimalLogger()

        # Memoization
        from .cachesys import Memoization

        self.memoization = memoization or Memoization()

        # Debug wrapper
        _debug = _DebugWrapper(self.logger)

        # Initialize globals with builtins
        if globals is None:
            import builtins

            self.globals.update(
                {
                    'range': builtins.range,
                    'len': builtins.len,
                    'str': builtins.str,
                    'int': builtins.int,
                    'float': builtins.float,
                    'list': _list_ctor,
                    'dict': builtins.dict,
                    'tuple': _tuple_ctor,
                    'set': _set_ctor,
                    'sum': builtins.sum,
                    'min': builtins.min,
                    'max': builtins.max,
                    'abs': builtins.abs,
                    'bool': builtins.bool,
                    'round': builtins.round,
                    'sorted': builtins.sorted,
                    'reversed': builtins.reversed,
                    'enumerate': builtins.enumerate,
                    'zip': builtins.zip,
                    'map': builtins.map,
                    'filter': builtins.filter,
                    'fold': _fold,
                    'reduce': _reduce,
                    'format': builtins.format,
                    'repr': builtins.repr,
                    'ascii': builtins.ascii,
                    'complex': builtins.complex,
                    'isinstance': builtins.isinstance,
                    'issubclass': builtins.issubclass,
                    'hasattr': builtins.hasattr,
                    'getattr': builtins.getattr,
                    'setattr': builtins.setattr,
                    'delattr': builtins.delattr,
                    'pow': builtins.pow,
                    'divmod': builtins.divmod,
                    'chr': builtins.chr,
                    'ord': builtins.ord,
                    'hex': builtins.hex,
                    'bin': builtins.bin,
                    'oct': builtins.oct,
                    'hash': builtins.hash,
                    'id': builtins.id,
                    'callable': builtins.callable,
                    'iter': builtins.iter,
                    'next': builtins.next,
                    'any': builtins.any,
                    'all': builtins.all,
                    'slice': builtins.slice,
                    'frozenset': builtins.frozenset,
                    'bytes': builtins.bytes,
                    'bytearray': builtins.bytearray,
                    'memoryview': builtins.memoryview,
                    'object': builtins.object,
                    'super': builtins.super,
                    'staticmethod': builtins.staticmethod,
                    'classmethod': builtins.classmethod,
                    'property': builtins.property,
                    'vars': builtins.vars,
                    'dir': builtins.dir,
                    'freeze': _rs.freeze,
                    'thaw': _rs.thaw,
                    'cached': _CachedWrapper(self),
                    '_cache': _CacheManager(self),
                    'debug': _debug,
                    'import': _ImportWrapper(self),
                    'jit': _JitWrapper(self),
                    'pure': _PureWrapper(self),
                    'META': CatnipMeta(),
                    'ND': _rs.build_nd(),
                    'RUNTIME': _rs.build_runtime(),
                    # Exception types (for raise expr)
                    'Exception': builtins.Exception,
                    'TypeError': builtins.TypeError,
                    'ValueError': builtins.ValueError,
                    'NameError': builtins.NameError,
                    'IndexError': builtins.IndexError,
                    'KeyError': builtins.KeyError,
                    'AttributeError': builtins.AttributeError,
                    'ZeroDivisionError': builtins.ZeroDivisionError,
                    'RuntimeError': builtins.RuntimeError,
                    'MemoryError': builtins.MemoryError,
                    'ArithmeticError': builtins.ArithmeticError,
                    'LookupError': builtins.LookupError,
                }
            )
        else:
            self.globals.update(globals)

        # META always available, with default attributes
        if 'META' not in self.globals:
            self.globals['META'] = CatnipMeta()
        meta = self.globals['META']
        if not hasattr(meta, 'main'):
            meta.main = True

        # Always expose logger and debug
        self.globals['logger'] = self.logger
        self.globals['debug'] = _debug

        # Init locals from dict
        if locals:
            for k, v in locals.items():
                self.locals._set(k, v)

        # Pure functions set
        for name in self.KNOWN_PURE_FUNCTIONS:
            self.pure_functions.add(name)

        # Snapshot of the builtin names seeded above, taken before any user
        # global is assigned at runtime. Drives has_shadowed_builtin_callable().
        self._builtin_names = frozenset(self.globals)

    def has_shadowed_builtin_callable(self):
        """True if a builtin name is currently bound to a Catnip callable.

        A user global that shadows a builtin (e.g. ``str = (x) => {...}``)
        can't ship to a fresh worker process: it isn't installed there, and the
        worker resolves the name to its own pre-seeded builtin instead -- a
        silent divergence, no error raised. The ND-process batch must fall back
        to the shared-memory thread path when this holds.
        """
        from ._rs import Function, Lambda, VMFunction

        callables = (Function, Lambda, VMFunction)
        g = self.globals
        return any(isinstance(g.get(name), callables) for name in self._builtin_names)
