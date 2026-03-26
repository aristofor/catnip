# FILE: catnip/vm/executor.py
"""
VM-based executor for Catnip code.

Compiles IR to bytecode and executes via the Rust VM.
"""

import logging

from catnip._rs import Compiler

from .rust_bridge import VMExecutor as RustVM


class VMExecutor:
    """
    Execute Catnip code via bytecode compilation and Rust VM.

    Usage:
        executor = VMExecutor(registry, context)
        result = executor.execute(statements)
    """

    def __init__(self, registry, context, logger=None):
        self.registry = registry
        self.context = context
        self.logger = logger or context.logger or logging.getLogger(__name__)

        self.compiler = Compiler()
        self.vm = RustVM(registry, context)

        # Enable JIT if requested via pragma
        if hasattr(context, 'jit_enabled') and context.jit_enabled:
            self.vm._vm.enable_jit()
            self.logger.debug("JIT compilation enabled")

        # Apply memory limit from config
        memory_limit = getattr(context, 'memory_limit', None)
        if memory_limit is not None:
            self.vm._vm.set_memory_limit(memory_limit)

    def execute(self, statements, trace=False):
        """
        Execute statements via VM.

        :param statements: List of Op nodes to execute
        :param trace: Enable execution tracing
        :return: Result of the last statement
        """
        self.context.result = None

        if statements is None:
            return None

        if isinstance(statements, list):
            if not statements:
                return None
            root = statements
        else:
            root = statements

        # Compile to bytecode - fallback to AST for unsupported features
        try:
            code = self.compiler.compile(root, "<module>")
        except NotImplementedError as e:
            from ..executor import Executor

            self.logger.debug(f"VM fallback to AST: {e}")
            return Executor(self.registry, self.context, self.logger).execute(statements, trace=trace)

        if trace:
            self.logger.debug("Compiled bytecode:")
            code.disassemble()

        # Pass source to VM for error reporting
        sourcemap = getattr(self.context, 'sourcemap', None)
        if sourcemap is not None:
            self.vm.set_source(sourcemap.source, sourcemap.filename)

        # Enable VM tracing if requested
        self.vm.set_trace(trace)

        # Execute
        try:
            self.context.result = self.vm.execute(code)
        except Exception as e:
            self.logger.debug(f"{type(e).__name__}: {e}")
            raise

        return self.context.result
