# FILE: catnip/__init__.py
import os

from catnip._rs import Scope

from ._rs import SourceMap
from ._version import __build_date__, __lang_id__, __version__
from .cachesys import CatnipCache
from .context import Context
from .exc import CatnipSemanticError
from .executor import Executor
from .parser import Parser
from .pragma import PragmaContext
from .registry import Registry
from .runtime import CatnipRuntime
from .semantic import Semantic


class Catnip:
    """
    Combines the Parser, Semantic and Executor to run a complete script.

    Supports pragma directives for controlling compilation and execution.
    """

    def __init__(self, **kwargs):
        # Initialize: set up context and classes.
        self.context = kwargs.get("context", None)
        if self.context is None:
            self.context = kwargs.get("context_class", Context)()
        self.parser = kwargs.get("parser", None)
        self.parser_class = kwargs.get("parser_class", Parser) if self.parser is None else None
        self.semantic_class = kwargs.get("semantic_class", Semantic)
        self.registry_class = kwargs.get("registry_class", Registry)
        self.executor_class = kwargs.get("executor_class", Executor)
        self.registry = self.registry_class(context=self.context)
        self.code = None

        # Pragma support
        self.pragma_context = PragmaContext()
        self.use_pragmas = kwargs.get("use_pragmas", True)

        # Apply optimization level from kwargs if provided
        if "optimize" in kwargs:
            self.pragma_context.optimize_level = kwargs["optimize"]

        # VM execution mode: "on" (default), "off"
        # Can be set via CATNIP_EXECUTOR env var or -x/--executor CLI flag
        # Map executor values: vm→on, ast→off
        default_mode = os.environ.get("CATNIP_EXECUTOR") or "vm"
        if default_mode == "vm":
            default_mode = "on"
        elif default_mode == "ast":
            default_mode = "off"
        self.vm_mode = kwargs.get("vm_mode", default_mode)

        # Runtime introspection - create and inject into context
        self.runtime = CatnipRuntime(pragma_context=self.pragma_context)
        self.context.globals['catnip'] = self.runtime

        # Module policy
        if "module_policy" in kwargs:
            self.context.module_policy = kwargs["module_policy"]

        # Cache support
        self.cache = kwargs.get("cache", None)
        if self.cache is None and kwargs.get("enable_cache", False):
            self.cache = CatnipCache()

    def parse(self, text, semantic=True):
        """
        Parse input text into executable code.

        Uses cache if enabled.

        :param text: Source code
        :param semantic: Whether to perform semantic analysis
        :return: Parsed code (AST or Op nodes)
        """
        # Determine cache options
        optimize = self.pragma_context.optimize_level > 0 if self.use_pragmas else True
        tco_enabled = self.pragma_context.tco_enabled if self.use_pragmas else True

        # Check cache if enabled
        # Always create sourcemap for error reporting (even with cache)
        source_bytes = text.encode('utf-8') if isinstance(text, str) else text
        self.context.sourcemap = SourceMap(source_bytes, filename='<input>')

        if self.cache is not None and semantic:
            cached_code = self.cache.get_parsed(text, optimize=optimize, tco_enabled=tco_enabled)
            if cached_code is not None:
                self.code = cached_code
                return self.code

        # Parse: transform input text into executable code.
        parser = self.parser if self.parser is not None else self.parser_class()
        ast = parser.parse(text)

        if semantic:
            semantic_analyzer = self.semantic_class(self.registry, self.context, optimize=optimize)

            # Pass pragma context to semantic analyzer
            if hasattr(semantic_analyzer, "pragma_context"):
                semantic_analyzer.pragma_context = self.pragma_context

            try:
                self.code = semantic_analyzer.analyze(ast)
            except CatnipSemanticError as e:
                if e.start_byte is not None and e.start_byte >= 0 and e.line is None:
                    sm = self.context.sourcemap
                    if sm is not None:
                        line, col = sm.byte_to_line_col(e.start_byte)
                        e.line = line
                        e.column = col
                        e.context = sm.get_snippet(e.start_byte, (e.end_byte or e.start_byte) + 1)
                        Exception.__init__(e, e._format_message())
                raise

            # Cache if enabled
            if self.cache is not None:
                self.cache.set_parsed(text, self.code, optimize=optimize, tco_enabled=tco_enabled)

            return self.code
        else:
            return ast

    def execute(self, trace=False):
        """
        Execute prepared code.

        :param trace: Enable execution tracing
        :return: Execution result
        """
        if self.code is None:
            raise RuntimeError("No code to execute.")

        # Apply pragma-controlled caching
        if hasattr(self.registry, "enable_cache"):
            self.registry.enable_cache(self.pragma_context.cache_enabled)

        # Apply pragma settings to context
        if self.use_pragmas:
            self.context.tco_enabled = self.pragma_context.tco_enabled
            self.context.jit_enabled = self.pragma_context.jit_enabled
            self.context.jit_all = self.pragma_context.jit_all
            if self.context.jit_enabled:
                # Initialize JIT subsystem if enabled
                self.context._init_jit()

            # Apply ND-recursion pragmas
            self.context.nd_mode = self.pragma_context.nd_mode
            self.context.nd_workers = self.pragma_context.nd_workers
            self.context.nd_memoize = self.pragma_context.nd_memoize
            self.context.nd_batch_size = self.pragma_context.nd_batch_size

            # Force ND mode to 'thread' in VM mode (multiprocessing requires pickle)
            if self.vm_mode in ('on', 'rust') and self.context.nd_mode == 'process':
                self.context.nd_mode = 'thread'

        # Create executor based on vm_mode
        if self.vm_mode == "off":
            self.executor = self.executor_class(self.registry, self.context)
        elif self.vm_mode in ("on", "rust"):
            from .vm.executor import VMExecutor

            self.executor = VMExecutor(self.registry, self.context)
        else:
            raise ValueError(f"Unknown vm_mode: {self.vm_mode!r}")

        return self.executor.execute(self.code, trace=trace)


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
