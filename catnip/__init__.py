# FILE: catnip/__init__.py
import os
import sys as _sys

from ._rs import CatnipRuntime, Pipeline, Scope, SourceMap

# Backward compat alias
StandalonePipeline = Pipeline
from ._version import __build_date__, __commit__, __lang_id__, __version__
from .cachesys import CatnipCache
from .context import Context, _ImportWrapper, configure_logging
from .executor import Executor
from .pragma import PragmaContext
from .registry import Registry
from .semantic.opcode import OpCode

# Embedded binaries (catnip-run, catnip-repl) set this process-local marker
# before running user code: they are the application, so logging gets
# configured as soon as the package becomes live. Library hosts (plain
# `import catnip`) and child processes are untouched.
if getattr(_sys, '_catnip_embedded', False):
    configure_logging()


class Catnip:
    """
    Combines the Parser, Semantic and Executor to run a complete script.

    Supports pragma directives for controlling compilation and execution.
    """

    def __init__(self, **kwargs):
        # Initialize: set up context and classes.
        self.context = kwargs.get('context', None)
        if self.context is None:
            self.context = kwargs.get('context_class', Context)()
        self.registry_class = kwargs.get('registry_class', Registry)
        self.executor_class = kwargs.get('executor_class', Executor)
        self.registry = self.registry_class(context=self.context)
        self.context._registry = self.registry
        self.code = None
        self._source_text = None

        # Pragma support
        self.pragma_context = PragmaContext()
        self.use_pragmas = kwargs.get('use_pragmas', True)

        # Caller overrides (kwargs/CLI/env) win over in-file pragmas
        self._optimize_forced = 'optimize' in kwargs
        self._tco_forced = False
        if self._optimize_forced:
            self.pragma_context.optimize_level = kwargs['optimize']

        # VM execution mode: "on" (default), "off"
        # Can be set via CATNIP_EXECUTOR env var or -x/--executor CLI flag
        from .config import EXECUTOR_DEFAULT, executor_to_vm_mode

        default_mode = executor_to_vm_mode(os.environ.get('CATNIP_EXECUTOR') or EXECUTOR_DEFAULT)
        self.vm_mode = kwargs.get('vm_mode', default_mode)

        # Runtime introspection - create and inject into context
        self.runtime = CatnipRuntime(pragma_context=self.pragma_context)
        self.runtime._set_context(self.context)
        self.context.globals['catnip'] = self.runtime

        # Module policy
        if 'module_policy' in kwargs:
            self.context.module_policy = kwargs['module_policy']

        # Auto-import modules
        if 'auto' in kwargs:
            from .loader import ModuleLoader

            loader = ModuleLoader(self.context)
            loader.load_modules(kwargs['auto'])

        # Cache support
        self.cache = kwargs.get('cache', None)
        if self.cache is None and kwargs.get('enable_cache', False):
            self.cache = CatnipCache()

        # Standalone pipeline (unified parsing for all modes)
        self._pipeline = Pipeline()
        self._pipeline.set_context(self.context)
        self._pipeline.inject_globals(self.context.globals)
        # Import loader (Rust): resolves specs, loads modules, caches
        from ._rs import ImportLoader
        from .loader import ModuleLoader

        proxy = self._pipeline.globals()
        policy = getattr(self.context, 'module_policy', None)

        # .cat loading callback: delegates to Python ModuleLoader
        loader_ctx_ns = type('_Ctx', (), {'globals': proxy})()
        if policy is not None:
            loader_ctx_ns.module_policy = policy
        py_loader = ModuleLoader(loader_ctx_ns)

        def cat_loader(path, name):
            return py_loader.load_catnip_module(path, module_name=name)

        self._fixed_import = ImportLoader(proxy, policy=policy, cat_loader=cat_loader, context=self.context)
        self._pipeline.set_global('import', self._fixed_import)
        # AST mode resolves `import` from the Python context globals: route its
        # namespace loading through the same Rust loader (shared module cache)
        imp = self.context.globals.get('import')
        if isinstance(imp, _ImportWrapper):
            imp._rust_import = self._fixed_import

    def parse(self, text, semantic=True, filename=None):
        """
        Parse input text into executable code.

        :param text: Source code
        :param semantic: Whether to perform semantic analysis
        :param filename: Source file path (sets META.file for relative imports)
        :return: Parsed code (list of PyIRNode)
        """
        from .compat import _map_exception

        source_bytes = text.encode('utf-8') if isinstance(text, str) else text
        label = filename or '<input>'
        self.context.sourcemap = SourceMap(source_bytes, filename=label)
        self._source_text = text
        if filename:
            self._pipeline.set_source_path(filename)

        try:
            # Baseline before parsing; in-file pragmas can flip it during the
            # semantic pass, unless forced by the caller (kwargs/CLI/env)
            if self.use_pragmas:
                optimize = self.pragma_context.optimize_level > 0
                tco = self.pragma_context.tco_enabled
                self._pipeline.set_optimize_enabled(optimize)
                self._pipeline.set_tco_enabled(tco)
                self._pipeline.set_optimize_override(optimize if self._optimize_forced else None)
                self._pipeline.set_tco_override(tco if self._tco_forced else None)

            if not semantic:
                return self._pipeline.parse_to_ir(text, False)

            # A failed prepare() (now reachable via fatal E300) must not leave
            # the previous AST-mode code executable: drop it before re-preparing,
            # mirroring the Rust pipeline invalidating prepared_ir up front.
            self.code = None
            self._pipeline.prepare(text)
            self.code = self._pipeline.prepared_ir_to_op()
            self._sync_pragmas_from_ir()
            return self._pipeline.get_prepared_ir_nodes()
        except RuntimeError as e:
            raise _map_exception(e, source_text=self._source_text) from None

    def execute(self, trace=False):
        """
        Execute prepared code.

        :param trace: Enable execution tracing
        :return: Execution result
        """
        if self.code is None:
            raise RuntimeError("No code to execute.")

        self._apply_pragmas()

        if self.vm_mode == 'off':
            return self._execute_ast(trace)
        return self._execute_vm(trace)

    def _sourcemap(self):
        """SourceMap over the last parsed source, or None."""
        src = self._source_text
        if src is None:
            return None
        return SourceMap(src.encode('utf-8') if isinstance(src, str) else src, '<input>')

    def _line_col_from_byte(self, byte_offset):
        """Convert byte offset to (1-based line, 0-based column), or (None, None)."""
        from .compat import _byte_to_line_col

        return _byte_to_line_col(self._sourcemap(), byte_offset)

    def _sync_pragmas_from_ir(self):
        """Sync pragma directives from prepared IR to PragmaContext."""
        from .exc import CatnipPragmaError, CatnipSemanticError
        from .pragma import sync_pragmas_from_nodes

        def on_error(kind, message, node):
            exc = (CatnipSemanticError if kind == 'semantic' else CatnipPragmaError)(message)
            exc.line, exc.column = self._line_col_from_byte(node.start_byte)
            raise exc from None

        sync_pragmas_from_nodes(
            self._pipeline.get_prepared_ir_nodes(),
            self.pragma_context,
            on_error=on_error,
            # Caller overrides (kwargs/CLI/env) win over file pragmas:
            # pragma_context must keep the forced (effective) value,
            # both for the next parse and for catnip.optimize introspection
            skip_directive=lambda d: (d == 'optimize' and self._optimize_forced) or (d == 'tco' and self._tco_forced),
        )

    def _apply_pragmas(self):
        """Apply pragma settings to context."""
        if hasattr(self.registry, 'enable_cache'):
            self.registry.enable_cache(self.pragma_context.cache_enabled)

        if self.use_pragmas:
            self.context.tco_enabled = self.pragma_context.tco_enabled
            self.context.jit_enabled = self.pragma_context.jit_enabled
            self.context.jit_all = self.pragma_context.jit_all

            self.context.nd_mode = self.pragma_context.nd_mode
            self.context.nd_workers = self.pragma_context.nd_workers
            self.context.nd_memoize = self.pragma_context.nd_memoize
            self.context.nd_batch_size = self.pragma_context.nd_batch_size

    def _execute_vm(self, trace):
        """Execute via Pipeline delegation."""
        from .compat import _map_exception

        # Sync globals Python → Rust (but preserve the fixed import wrapper)
        self._pipeline.inject_globals(self.context.globals)
        self._pipeline.set_global('import', self._fixed_import)

        # Apply pragma settings to the running pipeline
        if self.use_pragmas:
            self._pipeline.set_tco_enabled(self.pragma_context.tco_enabled)
            self._pipeline.set_optimize_enabled(self.pragma_context.optimize_level > 0)
            self._pipeline.set_jit_enabled(self.pragma_context.jit_enabled)
            self._pipeline.set_nd_memoize(self.pragma_context.nd_memoize)
            nd_mode = self.pragma_context.nd_mode
            if nd_mode and nd_mode != 'sequential':
                self._pipeline.set_nd_mode(nd_mode)

        try:
            result = self._pipeline.execute_prepared()
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
            # Sync globals even on error (partial results must be visible)
            self._pipeline.export_globals(self.context.globals)
            exc = _map_exception(e)
            self._enrich_from_vm_error_context(exc)
            self._enrich_name_suggestion(exc)
            raise exc from None

        # Sync globals Rust → Python
        self._pipeline.export_globals(self.context.globals)

        # Track pure functions (normally done by Registry during broadcast)
        if hasattr(self.context, 'pure_functions'):
            for name, val in self.context.globals.items():
                if getattr(val, 'is_pure', False):
                    self.context.pure_functions.add(name)

        self.context.result = result
        return result

    def _enrich_name_suggestion(self, exc):
        """Add 'Did you mean' to CatnipNameError if a close match exists in globals."""
        from .exc import CatnipNameError
        from .suggest import suggest_name

        if not isinstance(exc, CatnipNameError) or exc.suggestions:
            return
        candidates = list(self.context.globals.keys())
        # Also add pipeline globals
        try:
            pg = self._pipeline.globals()
            if pg:
                candidates.extend(pg.keys())
        except (AttributeError, TypeError):
            pass
        matches = suggest_name(exc.name, candidates, max_suggestions=1)
        if matches:
            exc.suggestions = matches
            exc.args = (f"{exc}. Did you mean '{matches[0]}'?",)

    def _enrich_from_vm_error_context(self, exc):
        """Enrich exception with position, snippet and traceback from VM's ErrorContext."""
        from .compat import _byte_to_line_col

        ctx = self._pipeline.get_last_error_context()
        if ctx is None:
            return
        sm = self._sourcemap()
        byte_offset = ctx.get('start_byte')
        if byte_offset is not None:
            exc.line, exc.column = _byte_to_line_col(sm, byte_offset)
            # Build code snippet with pointer
            if self._source_text:
                exc.context = sm.get_snippet(byte_offset, byte_offset + 1)

        # Build traceback from call stack (deduplicate identical display lines)
        call_stack = ctx.get('call_stack')
        if call_stack:
            from .traceback import CatnipFrame, CatnipTraceback

            tb = CatnipTraceback()
            prev_key = None
            for name, sb in call_stack:
                line, _ = _byte_to_line_col(sm, sb)
                key = (name, line)
                if key == prev_key:
                    continue
                prev_key = key
                frame = CatnipFrame(name=name, filename='<input>', start_byte=sb, end_byte=sb, line=line)
                tb.push(frame)
            exc.traceback = tb

    def _enrich_error_position(self, exc, statements):
        """Try to set .line/.column on an exception from the failing node.

        Prefers the deepest failing node recorded by the Rust registry,
        falls back to the enclosing top-level statement.
        """
        take = getattr(self.registry, 'take_error_byte', None)
        byte_offset = take() if take is not None else -1
        if byte_offset < 0:
            stmt = getattr(self, 'executor', None) and getattr(self.executor, 'current_stmt', None)
            if stmt is None:
                return
            byte_offset = getattr(stmt, 'start_byte', None)
            if byte_offset is None:
                return
        exc.line, exc.column = self._line_col_from_byte(byte_offset)

    @staticmethod
    def _enrich_attribute_error(exc, msg):
        """Add 'Did you mean' suggestion to AttributeError if possible."""
        import difflib
        import re

        m = re.search(r"'(\w+)' object has no attribute '(\w+)'", msg)
        if not m:
            return msg
        attr_name = m.group(2)
        # Try to get the object's attributes from the exception
        obj = getattr(exc, 'obj', None)
        if obj is not None:
            candidates = dir(obj)
        else:
            # Infer type and check its dir
            type_map = dict(str='', list=[], dict={}, tuple=(), set=set())
            candidates = dir(type_map.get(m.group(1), object()))
        matches = difflib.get_close_matches(attr_name, candidates, n=1, cutoff=0.6)
        if matches:
            return f"{msg}. Did you mean: '{matches[0]}'?"
        return msg

    def _execute_ast(self, trace):
        """AST-based execution (validator mode)."""
        from ._rs import Op
        from .exc import CatnipRuntimeError, CatnipTypeError

        statements = self.code if isinstance(self.code, list) else [self.code]
        # Filter out Pragma ops (already processed by _sync_pragmas_from_ir)
        statements = [s for s in statements if not (type(s) is Op and s.ident == OpCode.PRAGMA)]
        self.executor = self.executor_class(self.registry, self.context)
        # Drop any position left over from a previous (handled) error
        take = getattr(self.registry, 'take_error_byte', None)
        if take is not None:
            take()
        try:
            return self.executor.execute(statements, trace=trace)
        except (CatnipRuntimeError, CatnipTypeError) as e:
            if not getattr(e, 'line', None):
                self._enrich_error_position(e, statements)
            raise
        except TypeError as e:
            exc = CatnipTypeError(str(e))
            self._enrich_error_position(exc, statements)
            raise exc from None
        except (ZeroDivisionError, ArithmeticError) as e:
            exc = CatnipRuntimeError(str(e))
            self._enrich_error_position(exc, statements)
            raise exc from None
        except AttributeError as e:
            msg = str(e)
            if 'Did you mean' not in msg:
                msg = self._enrich_attribute_error(e, msg)
            raise AttributeError(msg) from None


def pass_context(func):
    func.pass_context = True
    return func


def pure(func):
    """
    Decorator to mark an external function as pure (no side effects, deterministic).

    Pure functions are eligible for optimizations like:
    - Broadcast fusion (combining multiple operations into one)
    - Vectorization with numpy/pandas
    - Memoization/caching
    - Parallelization

    Example:
        @pure
        def square(x):
            return x ** 2

        ctx.globals['square'] = square

        # Can be optimized with broadcast fusion
        data.[square].[sqrt]  # -> data.[x => sqrt(square(x))]
    """
    func.is_pure = True
    return func
