# FILE: catnip/compat.py
"""Wrapper making Pipeline API-compatible with Catnip for tests."""

import re

from catnip._rs import extract_name_from_error
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
    ZERO_DIVISION_ERROR_PREFIX,
)
from catnip._rs import CatnipRuntime, Pipeline
from catnip.exc import (
    CatnipNameError,
    CatnipPragmaError,
    CatnipRuntimeError,
    CatnipSemanticError,
    CatnipTypeError,
)
from catnip.pragma import PRAGMA_ATTRS, PragmaContext

# Pattern: "@pragma:BYTE message" (pragma/semantic errors with byte position)
_PRAGMA_RE = re.compile(r"^@pragma:(\d+) (.+)")


def _map_exception(exc, source_text=None):
    """Map a RuntimeError from standalone to the appropriate Catnip exception."""
    msg = str(exc)

    # Pragma/semantic errors: "@pragma:BYTE message"
    m = _PRAGMA_RE.match(msg)
    if m:
        start_byte = int(m.group(1))
        detail = m.group(2)
        line = col = None
        if source_text is not None:
            line = source_text[:start_byte].count('\n') + 1
            last_nl = source_text.rfind('\n', 0, start_byte)
            col = start_byte - last_nl - 1 if last_nl >= 0 else start_byte
        if detail.startswith('Unknown pragma directive'):
            return CatnipSemanticError(detail, line=line, column=col)
        return CatnipPragmaError(detail, line=line, column=col)

    # NameError: "name 'x' is not defined" or "NameError: name 'x' is not defined"
    name_msg = msg.removeprefix('NameError: ')
    name = extract_name_from_error(name_msg)
    if name is not None:
        return CatnipNameError(name)

    # SyntaxError from compilation: "Compilation error: SyntaxError: ..."
    if msg.startswith(COMPILE_SYNTAX_PREFIX):
        detail = msg[len(COMPILE_SYNTAX_PREFIX) :].strip()
        return SyntaxError(detail)

    # Syntax errors from parser
    if msg.startswith(SYNTAX_PREFIXES):
        return SyntaxError(msg)

    # TypeError: "TypeError: ..." or "type error: ..."
    if msg.startswith(PY_TYPE_ERROR_PREFIX):
        return CatnipTypeError(msg[len(PY_TYPE_ERROR_PREFIX) :])
    if msg.startswith(CATNIP_TYPE_ERROR_PREFIX):
        return CatnipTypeError(msg[len(CATNIP_TYPE_ERROR_PREFIX) :])
    if msg.startswith(TYPE_ERROR_PREFIX):
        detail = msg[len(TYPE_ERROR_PREFIX) :].strip()
        # Strip redundant "TypeError: " prefix
        if detail.startswith(PY_TYPE_ERROR_PREFIX):
            detail = detail[len(PY_TYPE_ERROR_PREFIX) :]
        return CatnipTypeError(detail)

    # Direct wrapped Python exceptions (e.g. "IndexError: ...", "ValueError: ...")
    for prefix in RUNTIME_WRAPPED_PREFIXES:
        if msg.startswith(prefix):
            inner = msg[len(prefix) :]
            if prefix in (PY_TYPE_ERROR_PREFIX, CATNIP_TYPE_ERROR_PREFIX):
                return CatnipTypeError(inner)
            if prefix == INDEX_ERROR_PREFIX:
                return IndexError(inner)
            if prefix == KEY_ERROR_PREFIX:
                return KeyError(inner)
            if prefix == VALUE_ERROR_PREFIX:
                return ValueError(inner)
            if prefix == ATTRIBUTE_ERROR_PREFIX:
                return AttributeError(inner)
            if prefix == ZERO_DIVISION_ERROR_PREFIX:
                return CatnipRuntimeError(inner)
            break

    # RuntimeError: "runtime error: ..." (legacy format)
    if msg.startswith(RUNTIME_ERROR_PREFIX.strip()):
        detail = msg[len(RUNTIME_ERROR_PREFIX) :].strip()
        for prefix in RUNTIME_WRAPPED_PREFIXES:
            if detail.startswith(prefix):
                inner = detail[len(prefix) :]
                if prefix in (PY_TYPE_ERROR_PREFIX, CATNIP_TYPE_ERROR_PREFIX):
                    return CatnipTypeError(inner)
                if prefix == INDEX_ERROR_PREFIX:
                    return IndexError(inner)
                if prefix == KEY_ERROR_PREFIX:
                    return KeyError(inner)
                if prefix == VALUE_ERROR_PREFIX:
                    return ValueError(inner)
                if prefix == ATTRIBUTE_ERROR_PREFIX:
                    return AttributeError(inner)
                break
        return CatnipRuntimeError(detail)

    # Division by zero (direct)
    if 'division by zero' in msg.lower() or 'DivisionByZero' in msg:
        return CatnipRuntimeError(msg)

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
        if 0 <= sb <= len(raw) and hasattr(exc, 'line'):
            exc.line = raw[:sb].count(b'\n') + 1
            last_nl = raw.rfind(b'\n', 0, sb)
            exc.column = sb - last_nl - 1 if last_nl >= 0 else sb
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
        from .cachesys.memoization import Memoization

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
        except RuntimeError as e:
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
        for node in nodes:
            if node.kind == 'Op' and node.opcode == 'Pragma':
                args = node.args
                if not args:
                    continue
                directive = args[0].value
                value = args[1].value if len(args) > 1 else True
                mapping = PRAGMA_ATTRS.get(directive)
                if mapping is None:
                    if directive in ('warning', 'inline', 'pure'):
                        continue
                    from .exc import CatnipSemanticError

                    raise CatnipSemanticError(f"Unknown pragma directive: '{directive}'")
                attr, typ = mapping
                if directive == 'jit' and value == 'all':
                    self.pragma_context.jit_enabled = True
                    self.pragma_context.jit_all = True
                    continue
                try:
                    if typ is bool and not isinstance(value, bool):
                        continue
                    setattr(self.pragma_context, attr, typ(value))
                except (ValueError, TypeError):
                    continue
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
