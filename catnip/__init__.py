# FILE: catnip/__init__.py
import os

from ._rs import CatnipRuntime, Pipeline, Scope, SourceMap

# Backward compat alias
StandalonePipeline = Pipeline
from ._version import __build_date__, __commit__, __lang_id__, __version__
from .cachesys import CatnipCache
from .context import Context, _ImportWrapper
from .executor import Executor
from .pragma import PRAGMA_ATTRS, PragmaContext
from .registry import Registry
from .semantic.opcode import OpCode


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

        # Apply optimization level from kwargs if provided
        if 'optimize' in kwargs:
            self.pragma_context.optimize_level = kwargs['optimize']

        # VM execution mode: "on" (default), "off"
        # Can be set via CATNIP_EXECUTOR env var or -x/--executor CLI flag
        from .config import executor_to_vm_mode

        default_mode = executor_to_vm_mode(os.environ.get('CATNIP_EXECUTOR') or 'vm')
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
        # Fix import wrapper: must write to RustGlobals, not PyDict
        import types

        proxy = self._pipeline.globals()
        ns = types.SimpleNamespace(globals=proxy)
        if hasattr(self.context, 'module_policy'):
            ns.module_policy = self.context.module_policy
        if hasattr(self.context, '_extensions'):
            ns._extensions = self.context._extensions
        self._fixed_import = _ImportWrapper(ns)
        self._pipeline.set_global('import', self._fixed_import)

    def parse(self, text, semantic=True):
        """
        Parse input text into executable code.

        :param text: Source code
        :param semantic: Whether to perform semantic analysis
        :return: Parsed code (list of PyIRNode)
        """
        from .compat import _map_exception

        source_bytes = text.encode('utf-8') if isinstance(text, str) else text
        self.context.sourcemap = SourceMap(source_bytes, filename='<input>')
        self._source_text = text

        try:
            # Apply known pragma settings before parsing (optimize=0 disables passes)
            if self.use_pragmas:
                self._pipeline.set_optimize_enabled(self.pragma_context.optimize_level > 0)
                self._pipeline.set_tco_enabled(self.pragma_context.tco_enabled)

            if not semantic:
                return self._pipeline.parse_to_ir(text, False)

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

    def _line_from_byte(self, byte_offset):
        """Convert byte offset to 1-based line number."""
        src = self._source_text
        if src is None or byte_offset is None:
            return None
        raw = src.encode('utf-8')
        return raw[:byte_offset].count(b'\n') + 1

    def _col_from_byte(self, byte_offset):
        """Convert byte offset to 0-based column number."""
        src = self._source_text
        if src is None or byte_offset is None:
            return None
        raw = src.encode('utf-8')
        last_nl = raw.rfind(b'\n', 0, byte_offset)
        return byte_offset - last_nl - 1 if last_nl >= 0 else byte_offset

    def _sync_pragmas_from_ir(self):
        """Sync pragma directives from prepared IR to PragmaContext."""
        from .exc import CatnipPragmaError, CatnipSemanticError

        for node in self._pipeline.get_prepared_ir_nodes():
            if node.kind == 'Op' and node.opcode == 'Pragma':
                args = node.args
                if not args:
                    continue
                directive = args[0].value
                value = args[1].value if len(args) > 1 else True
                mapping = PRAGMA_ATTRS.get(directive)
                if mapping is None:
                    if directive == 'warning':
                        if not isinstance(value, bool):
                            exc = CatnipPragmaError("Pragma 'warning' requires True or False")
                            exc.line = self._line_from_byte(node.start_byte)
                            exc.column = self._col_from_byte(node.start_byte)
                            raise exc
                        continue
                    if directive in ('inline', 'pure'):
                        continue
                    exc = CatnipSemanticError(f"Unknown pragma directive: '{directive}'")
                    exc.line = self._line_from_byte(node.start_byte)
                    exc.column = self._col_from_byte(node.start_byte)
                    raise exc
                attr, typ = mapping
                try:
                    # jit accepts bool or 'all'
                    if directive == 'jit' and value == 'all':
                        self.pragma_context.jit_enabled = True
                        self.pragma_context.jit_all = True
                        continue
                    # bool() on strings silently returns True, check type
                    if typ is bool and not isinstance(value, bool):
                        raise ValueError(f"requires True or False, got {value!r}")
                    setattr(self.pragma_context, attr, typ(value))
                except (ValueError, TypeError) as e:
                    exc = CatnipPragmaError(f"Invalid value for pragma '{directive}': {e}")
                    exc.line = self._line_from_byte(node.start_byte)
                    exc.column = self._col_from_byte(node.start_byte)
                    raise exc from None

    def _apply_pragmas(self):
        """Apply pragma settings to context."""
        if hasattr(self.registry, 'enable_cache'):
            self.registry.enable_cache(self.pragma_context.cache_enabled)

        if self.use_pragmas:
            self.context.tco_enabled = self.pragma_context.tco_enabled
            self.context.jit_enabled = self.pragma_context.jit_enabled
            self.context.jit_all = self.pragma_context.jit_all
            if self.context.jit_enabled:
                self.context._init_jit()

            self.context.nd_mode = self.pragma_context.nd_mode
            self.context.nd_workers = self.pragma_context.nd_workers
            self.context.nd_memoize = self.pragma_context.nd_memoize
            self.context.nd_batch_size = self.pragma_context.nd_batch_size

            if self.vm_mode in ('on', 'rust') and self.context.nd_mode == 'process':
                self.context.nd_mode = 'thread'

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
        except RuntimeError as e:
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

        if not isinstance(exc, CatnipNameError):
            return
        msg = str(exc)
        if 'Did you mean' in msg:
            return
        import difflib
        import re

        m = re.search(r"Name '(\w+)' is not defined", msg)
        if not m:
            return
        name = m.group(1)
        candidates = list(self.context.globals.keys())
        # Also add pipeline globals
        try:
            pg = self._pipeline.globals()
            if pg:
                candidates.extend(pg.keys())
        except (AttributeError, TypeError):
            pass
        matches = difflib.get_close_matches(name, candidates, n=1, cutoff=0.6)
        if matches:
            exc.args = (f"{msg}. Did you mean '{matches[0]}'?",)

    def _enrich_from_vm_error_context(self, exc):
        """Enrich exception with position, snippet and traceback from VM's ErrorContext."""
        ctx = self._pipeline.get_last_error_context()
        if ctx is None:
            return
        byte_offset = ctx.get('start_byte')
        if byte_offset is not None:
            exc.line = self._line_from_byte(byte_offset)
            exc.column = self._col_from_byte(byte_offset)
            # Build code snippet with pointer
            src = self._source_text
            if src:
                source_bytes = src.encode('utf-8') if isinstance(src, str) else src
                sm = SourceMap(source_bytes, '<input>')
                exc.context = sm.get_snippet(byte_offset, byte_offset + 1)

        # Build traceback from call stack (deduplicate identical display lines)
        call_stack = ctx.get('call_stack')
        if call_stack:
            from .traceback import CatnipFrame, CatnipTraceback

            tb = CatnipTraceback()
            prev_key = None
            for name, sb in call_stack:
                line = self._line_from_byte(sb)
                key = (name, line)
                if key == prev_key:
                    continue
                prev_key = key
                frame = CatnipFrame(name=name, filename='<input>', start_byte=sb, end_byte=sb, line=line)
                tb.push(frame)
            exc.traceback = tb

    def _enrich_error_position(self, exc, statements):
        """Try to set .line/.column on an exception from the current statement."""
        stmt = getattr(self, 'executor', None) and getattr(self.executor, 'current_stmt', None)
        if stmt is None:
            return
        byte_offset = getattr(stmt, 'start_byte', None)
        if byte_offset is not None:
            exc.line = self._line_from_byte(byte_offset)
            exc.column = self._col_from_byte(byte_offset)

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
