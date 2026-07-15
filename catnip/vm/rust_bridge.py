# FILE: catnip/vm/rust_bridge.py
"""
Bridge between Python Catnip and the Rust VM.

Provides VMExecutor that wraps the Rust VM with Python callbacks
for operations that need Python runtime support.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from .._rs import VM as _VM
from .._rs import SourceMap
from ..exc import CatnipNameError, CatnipRuntimeError
from ..suggest import suggest_name
from ..traceback import CatnipFrame, CatnipTraceback

if TYPE_CHECKING:
    from catnip._rs import CodeObject

    from ..context import Context


def _convert_rust_exception(exc: Exception) -> Exception:
    """Convert Rust VM exceptions to Catnip/Python exceptions.

    Delegates to compat._map_exception so the debugger path and the pipeline
    path produce identical exceptions for the same VM error.
    """
    from ..compat import _map_exception

    return _map_exception(exc)


class VMExecutor:
    """
    Executor that uses the Rust VM for bytecode execution.
    """

    def __init__(self, registry, context: Context) -> None:
        self.registry = registry
        self.context = context
        # Store registry and executor in context for VMFunction direct calls
        context._registry = registry
        context._vm_executor = self
        self._vm = _VM()
        self._vm.set_context(context)

    def set_source(self, source: bytes, filename: str = '<input>') -> None:
        """Set source code for error reporting."""
        self._source = source
        self._filename = filename
        self._vm.set_source(source, filename)

    def execute(
        self,
        code: CodeObject,
        args: tuple = (),
        kwargs: dict | None = None,
        sync_globals: bool = True,
        closure_scope: Any = None,
    ) -> Any:
        """Execute a code object using the Rust VM."""
        if kwargs is None:
            kwargs = {}

        # Execute via Rust VM
        try:
            result = self._vm.execute(code, args, kwargs, closure_scope)
        except MemoryError:
            raise
        except (
            RuntimeError,
            TypeError,
            NameError,
            IndexError,
            KeyError,
            AttributeError,
            ZeroDivisionError,
            ValueError,
        ) as e:
            enriched = self._enrich_error(e)
            raise enriched from None

        return result

    def _enrich_error(self, exc: Exception) -> Exception:
        """Enrich a Rust VM exception with source location and call stack."""
        ctx = self._vm.get_last_error_context()
        base_exc = _convert_rust_exception(exc)

        if ctx is None:
            return base_exc

        source = getattr(self, '_source', None)
        filename = getattr(self, '_filename', '<input>')

        # Build traceback from call stack
        tb = CatnipTraceback()
        for name, start_byte in ctx['call_stack']:
            tb.push(
                CatnipFrame(
                    name=name,
                    filename=filename,
                    start_byte=start_byte,
                    end_byte=start_byte,
                )
            )

        # Compute line/col from start_byte
        line = None
        col = None
        snippet = None
        if source and ctx['start_byte'] >= 0:
            sm = SourceMap(source, filename)
            line, col = sm.byte_to_line_col(ctx['start_byte'])
            snippet = sm.get_snippet(ctx['start_byte'], ctx['start_byte'] + 1)

        # Add "did you mean?" suggestions for NameError
        if isinstance(base_exc, CatnipNameError) and hasattr(base_exc, 'name'):
            available = list(self.context.locals.items()) + list(self.context.globals.keys())
            suggestions = suggest_name(base_exc.name, available)
            if suggestions:
                base_exc = CatnipNameError(base_exc.name, suggestions=suggestions)

        # Enrich the exception with location info if it's a CatnipError
        if isinstance(base_exc, CatnipRuntimeError):
            base_exc.filename = filename
            base_exc.line = line
            base_exc.column = col
            base_exc.context = snippet
            if tb:
                base_exc.traceback = tb
            # Re-format the message with location info
            Exception.__init__(base_exc, base_exc._format_message())
            return base_exc

        # Standard Python exceptions: inject location into the message
        if line is not None:
            loc = f"line {line}"
            if col is not None:
                loc += f", column {col}"
            if filename and filename != '<input>':
                loc = f"File {filename!r}, {loc}"
            # Rebuild exception with location prefix
            orig_args = base_exc.args
            if orig_args:
                enriched_msg = f"{loc}: {orig_args[0]}"
                base_exc.args = (enriched_msg, *orig_args[1:])
            else:
                base_exc.args = (loc,)

        return base_exc

    def set_trace(self, enabled: bool) -> None:
        """Enable/disable execution tracing."""
        self._vm.set_trace(enabled)

    def set_profile(self, enabled: bool) -> None:
        """Enable/disable profiling."""
        self._vm.set_profile(enabled)

    def get_profile_counts(self) -> dict:
        """Get opcode execution counts."""
        return self._vm.get_profile_counts()
