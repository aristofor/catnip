# FILE: catnip/debug/console.py
"""Interactive console debugger for Catnip — delegates to Rust."""

from .._rs import run_debugger
from .session import DebugSession


class ConsoleDebugger:
    """
    Interactive terminal debugger.

    Commands:
        c, continue   - Continue execution
        s, step       - Step into next instruction
        n, next       - Step over (stay at same depth)
        o, out        - Step out of current function
        b N           - Add breakpoint at line N
        rb N          - Remove breakpoint at line N
        p EXPR        - Evaluate expression in current scope
        v, vars       - Show all local variables
        l, list       - Show source around current position
        bt, backtrace - Show call stack
        q, quit       - Abort execution
        h, help       - Show help
    """

    def __init__(self, session: DebugSession, no_color: bool = False):
        self.session = session
        self.no_color = no_color

    def run(self):
        """Main debugger loop — delegated to Rust."""
        run_debugger(
            self.session.source_text,
            list(self.session._breakpoints),
            no_color=self.no_color,
        )
