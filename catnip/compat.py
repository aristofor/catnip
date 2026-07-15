# FILE: catnip/compat.py
"""Wrapper making Pipeline API-compatible with Catnip for tests."""

import re

from catnip._error_strings import (
    ATTRIBUTE_ERROR_PREFIX,
    CATNIP_TYPE_ERROR_PREFIX,
    COMPILE_SYNTAX_PREFIX,
    INDEX_ERROR_PREFIX,
    KEY_ERROR_PREFIX,
    PY_TYPE_ERROR_PREFIX,
    RUNTIME_ERROR_PREFIX,
    RUNTIME_WRAPPED_PREFIXES,
    SYNTAX_PREFIXES,
    TYPE_ERROR_PREFIX,
    VALUE_ERROR_PREFIX,
    WEIRD_ERROR_PREFIX,
    ZERO_DIVISION_ERROR_PREFIX,
)
from catnip._rs import CatnipRuntime, Pipeline, SourceMap, extract_name_from_error
from catnip.exc import (
    CatnipNameError,
    CatnipPragmaError,
    CatnipRuntimeError,
    CatnipSemanticError,
    CatnipTypeError,
    CatnipWeirdError,
)
from catnip.pragma import PragmaContext

# Pattern: "@pragma:BYTE message" (pragma/semantic errors with byte position)
_PRAGMA_RE = re.compile(r"^@pragma:(\d+) (.+)")

# Python's "'X' object is not iterable", normalized to Catnip's unpack message
_NOT_ITERABLE_RE = re.compile(r"'(\w+)' object is not iterable")


def _byte_to_line_col(source_map, byte_offset):
    """(1-based line, 0-based column) of a byte offset, or (None, None) without a SourceMap."""
    if source_map is None or byte_offset is None:
        return None, None
    line, col = source_map.byte_to_line_col(byte_offset)
    return line, col - 1


def _type_error(detail):
    """Build a CatnipTypeError, normalizing Python's iterable message to Catnip's."""
    m = _NOT_ITERABLE_RE.match(detail)
    if m:
        return CatnipTypeError(f"Cannot unpack non-iterable {m.group(1)}")
    return CatnipTypeError(detail)


def _map_wrapped(msg):
    """Map a "Prefix: detail" wrapped Python exception message, or None."""
    for prefix in RUNTIME_WRAPPED_PREFIXES:
        if not msg.startswith(prefix):
            continue
        inner = msg[len(prefix) :]
        if prefix in (PY_TYPE_ERROR_PREFIX, CATNIP_TYPE_ERROR_PREFIX):
            return _type_error(inner)
        if prefix == INDEX_ERROR_PREFIX:
            return IndexError(inner)
        if prefix == KEY_ERROR_PREFIX:
            # Rust wraps the repr ("KeyError: 'foo'"): strip the quotes or
            # Python's own repr would double them.
            return KeyError(inner.strip("'\""))
        if prefix == VALUE_ERROR_PREFIX:
            return ValueError(inner)
        if prefix == ATTRIBUTE_ERROR_PREFIX:
            return AttributeError(inner)
        if prefix == ZERO_DIVISION_ERROR_PREFIX:
            return CatnipRuntimeError(inner)
        break  # RUNTIME_EXCEPTION_PREFIX: fall through to the caller
    return None


def _map_exception(exc, source_text=None):
    """Map a VM exception to the appropriate Catnip exception.

    Single mapping for both the pipeline (Catnip.execute) and the VM executor
    (debugger): keep the two paths producing identical exceptions.
    """
    # Native Python exceptions from aligned VMError -> PyErr boundary:
    # pass through directly (they already have the right type)
    if isinstance(exc, (ValueError, IndexError, KeyError, AttributeError, MemoryError)):
        return exc
    if isinstance(exc, ZeroDivisionError):
        return CatnipRuntimeError(str(exc))
    if isinstance(exc, TypeError):
        return _type_error(str(exc))
    if isinstance(exc, NameError):
        name = extract_name_from_error(str(exc))
        if name is not None:
            return CatnipNameError(name)
        return CatnipNameError(str(exc))

    msg = str(exc)

    # Pragma/semantic errors: "@pragma:BYTE message"
    m = _PRAGMA_RE.match(msg)
    if m:
        start_byte = int(m.group(1))
        detail = m.group(2)
        line = col = None
        if source_text is not None:
            line, col = _byte_to_line_col(SourceMap(source_text.encode('utf-8'), '<input>'), start_byte)
        if detail.startswith('Unknown pragma directive'):
            return CatnipSemanticError(detail, line=line, column=col)
        return CatnipPragmaError(detail, line=line, column=col)

    # NameError: "name 'x' is not defined" or "NameError: name 'x' is not defined"
    name_msg = msg.removeprefix('NameError: ')
    name = extract_name_from_error(name_msg)
    if name is not None:
        return CatnipNameError(name)

    # Internal VM errors (stack underflow, frame overflow)
    if msg.startswith(WEIRD_ERROR_PREFIX):
        return CatnipWeirdError(msg[len(WEIRD_ERROR_PREFIX) :], cause='vm')

    # SyntaxError from compilation: "Compilation error: SyntaxError: ..."
    if msg.startswith(COMPILE_SYNTAX_PREFIX):
        detail = msg[len(COMPILE_SYNTAX_PREFIX) :].strip()
        return SyntaxError(detail)

    # Syntax errors from parser
    if msg.startswith(SYNTAX_PREFIXES):
        return SyntaxError(msg)

    # TypeError: "type error: ..." (the "TypeError:"/"CatnipTypeError:" forms
    # are handled by _map_wrapped below)
    if msg.startswith(TYPE_ERROR_PREFIX):
        detail = msg[len(TYPE_ERROR_PREFIX) :].strip()
        # Strip redundant "TypeError: " prefix
        if detail.startswith(PY_TYPE_ERROR_PREFIX):
            detail = detail[len(PY_TYPE_ERROR_PREFIX) :]
        return _type_error(detail)

    # Direct wrapped Python exceptions (e.g. "IndexError: ...", "ValueError: ...")
    mapped = _map_wrapped(msg)
    if mapped is not None:
        return mapped

    # RuntimeError: "runtime error: ..." (legacy format)
    if msg.startswith(RUNTIME_ERROR_PREFIX.strip()):
        detail = msg[len(RUNTIME_ERROR_PREFIX) :].strip()
        if detail.startswith(WEIRD_ERROR_PREFIX):
            return CatnipWeirdError(detail[len(WEIRD_ERROR_PREFIX) :], cause='vm')
        mapped = _map_wrapped(detail)
        if mapped is not None:
            return mapped
        return CatnipRuntimeError(detail)

    # Division by zero (direct)
    if 'division by zero' in msg.lower() or 'DivisionByZero' in msg:
        return CatnipRuntimeError(msg)

    # Bare type errors (no prefix)
    if 'unsupported operand' in msg or 'not iterable' in msg:
        return CatnipTypeError(msg)

    # Fallback: CatnipRuntimeError
    return CatnipRuntimeError(msg)


def _enrich_with_position(exc, pipeline, source):
    """Add line/column and suggestions to a mapped exception using VM error context."""
    ctx = pipeline.get_last_error_context()
    if source is None:
        return
    if ctx is not None:
        sb = ctx.get('start_byte', -1)
        raw = source.encode('utf-8')
        if 0 <= sb <= len(raw):
            exc.line, exc.column = _byte_to_line_col(SourceMap(raw, '<input>'), sb)
    # Add "Did you mean?" suggestions for NameError
    if type(exc) is CatnipNameError:
        suggestions = exc.suggestions
        if not suggestions:
            from .suggest import suggest_name

            available = list(pipeline.globals().keys())
            suggestions = suggest_name(exc.name, available) or []
        # Always reconstruct message with position (even without suggestions)
        if exc.line is not None or suggestions:
            exc.suggestions = suggestions
            exc.__init__(exc.name, suggestions=suggestions, line=exc.line, column=exc.column)


class _GlobalsView:
    """Dict-like view over Pipeline globals via GlobalsProxy."""

    __slots__ = ('_proxy',)

    def __init__(self, proxy):
        self._proxy = proxy

    def __getitem__(self, key):
        return self._proxy[key]

    def __setitem__(self, key, value):
        self._proxy[key] = value

    def __contains__(self, key):
        return key in self._proxy

    def get(self, key, default=None):
        return self._proxy.get(key, default)

    def __len__(self):
        return len(self._proxy)

    def __iter__(self):
        return iter(self._proxy.keys())

    def keys(self):
        return self._proxy.keys()

    def items(self):
        return self._proxy.items()

    def values(self):
        return [v for _, v in self._proxy.items()]

    def update(self, other):
        for k, v in (other.items() if hasattr(other, 'items') else other):
            self._proxy[k] = v


class _ContextShim:
    """Shim exposing .globals, .result, and optional attributes for test compatibility."""

    def __init__(self, globals_view):
        self.globals = globals_view
        self.result = None
        self._module_policy = None
        self.memoization = None
        self._extensions = {}
        self._loader_ns = None

    def pop_scope(self):
        from .exc import CatnipWeirdError

        raise CatnipWeirdError("Cannot pop global scope")

    @property
    def module_policy(self):
        return self._module_policy

    @module_policy.setter
    def module_policy(self, value):
        self._module_policy = value
        if self._loader_ns is not None:
            if value is not None:
                self._loader_ns.module_policy = value
            elif hasattr(self._loader_ns, 'module_policy'):
                del self._loader_ns.module_policy


class _CacheManagerStandalone:
    """Standalone _cache builtin backed by a Memoization instance."""

    def __init__(self, memo):
        self._memo = memo

    def invalidate(self, func_name=None):
        return self._memo.invalidate(func_name)

    def stats(self):
        return self._memo.stats()

    def enable(self):
        self._memo.enable()

    def disable(self):
        self._memo.disable()


class _CachedStandalone:
    """Standalone cached() builtin backed by its own Memoization instance."""

    def __init__(self, memo):
        self._memo = memo

    def __call__(self, func, name=None, key_func=None, validator=None):
        from .cachesys.memoization import CachedWrapper

        func_name = name or getattr(func, '__name__', 'anonymous')
        return CachedWrapper(func, self._memo, func_name, key_func=key_func, validator=validator)


class CatnipStandalone:
    """Drop-in replacement for Catnip using Pipeline."""

    def __init__(self, **kwargs):
        self._pipeline = Pipeline()
        self._pending_code = None
        self._prepared = False

        # Real PragmaContext + CatnipRuntime (same as DSL mode)
        self.pragma_context = PragmaContext()
        self.runtime = CatnipRuntime(pragma_context=self.pragma_context)

        # Expose globals view
        proxy = self._pipeline.globals()
        self._globals_view = _GlobalsView(proxy)
        self.context = _ContextShim(self._globals_view)

        # Inject catnip runtime + cached builtin into VM globals
        self.runtime._set_context(self.context)
        self._pipeline.set_global('catnip', self.runtime)
        from .cachesys import Memoization

        self._memoization = Memoization()
        self.context.memoization = self._memoization
        self._pipeline.set_global('cached', _CachedStandalone(self._memoization))
        self._pipeline.set_global('_cache', _CacheManagerStandalone(self._memoization))

        # Module policy + import wrapper
        if 'module_policy' in kwargs:
            self.context.module_policy = kwargs['module_policy']
        self._setup_import()

        # Store code for parse/execute pattern
        self.code = None

        # Auto-import modules
        auto = kwargs.get('auto', [])
        for mod_name in auto:
            import importlib

            try:
                mod = importlib.import_module(mod_name)
                self._globals_view[mod_name] = mod
            except ImportError:
                pass

    def _setup_import(self):
        """Wire up import() with the same _ImportWrapper as DSL mode."""
        import types

        from .context import _ImportWrapper

        ns = types.SimpleNamespace(globals=self._globals_view)
        if self.context.module_policy is not None:
            ns.module_policy = self.context.module_policy
        ns._extensions = self.context._extensions
        self._loader_ns = ns
        self.context._loader_ns = ns
        self._pipeline.set_global('import', _ImportWrapper(ns))

    def parse(self, text, semantic=True):
        self._pending_code = text
        self._prepared = False
        self.code = text
        if semantic:
            try:
                self._pipeline.prepare(text)
            except SyntaxError:
                raise
            except RuntimeError as e:
                raise _map_exception(e, source_text=text) from None
            self._prepared = True
            self._sync_pragmas()
        # Return IR nodes (same as DSL mode)
        try:
            if self._prepared:
                return self._pipeline.get_prepared_ir_nodes()
            return self._pipeline.parse_to_ir(text, semantic)
        except SyntaxError:
            raise
        except RuntimeError as e:
            raise _map_exception(e, source_text=text) from None

    def execute(self, trace=False):
        if self._pending_code is None:
            raise RuntimeError("No code to execute.")
        source = self._pending_code
        try:
            if self._prepared:
                result = self._pipeline.execute_prepared()
            else:
                result = self._pipeline.execute(source)
        except (
            RuntimeError,
            TypeError,
            ValueError,
            IndexError,
            KeyError,
            AttributeError,
            NameError,
            ZeroDivisionError,
            MemoryError,
        ) as e:
            mapped = _map_exception(e)
            _enrich_with_position(mapped, self._pipeline, source)
            raise mapped from None
        finally:
            self._pending_code = None
            self._prepared = False
        self.context.result = result
        return result

    def _sync_pragmas(self):
        """Extract pragma directives from prepared IR and apply to pragma_context + pipeline."""
        try:
            nodes = self._pipeline.get_prepared_ir_nodes()
        except RuntimeError:
            return
        from .exc import CatnipPragmaError, CatnipSemanticError
        from .pragma import sync_pragmas_from_nodes

        def on_error(kind, message, node):
            # Same strictness as the DSL path (no line/column: the standalone
            # wrapper does not keep the source text).
            raise (CatnipSemanticError if kind == 'semantic' else CatnipPragmaError)(message)

        sync_pragmas_from_nodes(nodes, self.pragma_context, on_error=on_error)
        self._apply_pragmas_to_pipeline()

    def _apply_pragmas_to_pipeline(self):
        """Push pragma_context settings into the Rust pipeline."""
        self._pipeline.set_tco_enabled(self.pragma_context.tco_enabled)
        self._pipeline.set_optimize_enabled(self.pragma_context.optimize_level > 0)
        if hasattr(self.pragma_context, 'jit_enabled'):
            self._pipeline.set_jit_enabled(self.pragma_context.jit_enabled)
        if hasattr(self.pragma_context, 'nd_mode') and self.pragma_context.nd_mode is not None:
            self._pipeline.set_nd_mode(self.pragma_context.nd_mode)
        if hasattr(self.pragma_context, 'nd_memoize'):
            self._pipeline.set_nd_memoize(self.pragma_context.nd_memoize)

    def reset(self):
        self._pipeline.reset()
        # Re-create globals view after reset
        proxy = self._pipeline.globals()
        self._globals_view = _GlobalsView(proxy)
        self.context = _ContextShim(self._globals_view)
        # Re-bind runtime to new context
        self.runtime._set_context(self.context)
        self._pipeline.set_global('catnip', self.runtime)
        # Re-inject memoization + cached builtins
        self.context.memoization = self._memoization
        self._pipeline.set_global('cached', _CachedStandalone(self._memoization))
        self._pipeline.set_global('_cache', _CacheManagerStandalone(self._memoization))
        # Re-wire import()
        self._setup_import()
