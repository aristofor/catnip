# FILE: catnip/debug/session.py
"""Debug session: thin Python wrapper around RustDebugSession."""

import enum
from dataclasses import dataclass

from .._rs import RustDebugSession


class DebugAction(enum.IntEnum):
    """Actions sent from debugger to VM via callback return value."""

    DISABLED = 0
    CONTINUE = 1
    STEP_INTO = 2
    STEP_OVER = 3
    STEP_OUT = 4


class DebugState(enum.Enum):
    """Session lifecycle states."""

    IDLE = "idle"
    RUNNING = "running"
    PAUSED = "paused"
    FINISHED = "finished"
    ERROR = "error"


@dataclass
class PauseInfo:
    """Snapshot of VM state at a breakpoint or step stop."""

    line: int
    col: int
    locals: dict
    snippet: str
    call_stack: list[tuple[str, int]]
    start_byte: int = 0


# Map string commands to DebugAction
_ACTION_MAP = {
    'continue': DebugAction.CONTINUE,
    'step_into': DebugAction.STEP_INTO,
    'step': DebugAction.STEP_INTO,
    'step_over': DebugAction.STEP_OVER,
    'next': DebugAction.STEP_OVER,
    'step_out': DebugAction.STEP_OUT,
    'out': DebugAction.STEP_OUT,
}

# Map string names to DebugAction command strings for Rust
_RUST_CMD_MAP = {
    'continue': 'continue',
    'step_into': 'step_into',
    'step': 'step_into',
    'step_over': 'step_over',
    'next': 'step_over',
    'step_out': 'step_out',
    'out': 'step_out',
}


class DebugSession:
    """
    Manages a debug session between the VM and a debugger frontend.

    Delegates to RustDebugSession for channel-based communication.
    Preserves the existing Python API for tests and MCP.
    """

    def __init__(self, catnip_instance, source_text):
        self.catnip = catnip_instance
        self.source_text = source_text
        self._rust_session = RustDebugSession(source_text)
        self._state = DebugState.IDLE
        self._last_pause = None
        self._breakpoints = set()

    @property
    def last_pause(self):
        """Last PauseInfo snapshot, or None if never paused."""
        return self._last_pause

    @property
    def state(self):
        return self._state

    def add_breakpoint(self, line: int):
        """Add a breakpoint at a line number (1-indexed)."""
        self._breakpoints.add(line)
        self._rust_session.add_breakpoint(line)

    def remove_breakpoint(self, line: int):
        """Remove a breakpoint at a line number."""
        self._breakpoints.discard(line)
        self._rust_session.remove_breakpoint(line)

    def start(self, blocking=True):
        """
        Start the debug session.

        If blocking=False, the VM runs in a background thread (always the case
        with the Rust backend). blocking=True is kept for API compat but also
        uses background thread + immediate wait.
        """
        self._state = DebugState.RUNNING
        self._rust_session.start(self.catnip)

        if blocking:
            # Wait for completion (for testing)
            while True:
                event = self.wait_for_event(timeout=30)
                if event is None:
                    break
                event_type, _ = event
                if event_type in ('finished', 'error'):
                    break
                # Auto-continue on pauses in blocking mode
                self.send_command(DebugAction.CONTINUE)

    def send_command(self, action):
        """Send a debug action to the paused VM."""
        if isinstance(action, str):
            cmd_str = _RUST_CMD_MAP.get(action, 'continue')
        elif isinstance(action, DebugAction):
            cmd_str = {
                DebugAction.CONTINUE: 'continue',
                DebugAction.STEP_INTO: 'step_into',
                DebugAction.STEP_OVER: 'step_over',
                DebugAction.STEP_OUT: 'step_out',
            }.get(action, 'continue')
        else:
            cmd_str = 'continue'
        self._rust_session.send_command(cmd_str)

    def wait_for_event(self, timeout=None):
        """
        Wait for the next debug event.

        Returns: (event_type, data)
          - ('paused', PauseInfo)
          - ('finished', result)
          - ('error', exception)
        """
        result = self._rust_session.wait_for_event(timeout=timeout)
        if result is None:
            return None

        event_type, data = result[0], result[1]

        if event_type == 'paused':
            # data is a tuple: (line, col, locals_repr, snippet, call_stack, start_byte, locals_py)
            line, col, locals_repr, snippet, call_stack, start_byte, locals_py = data

            # Build locals dict from the actual Python dict
            locals_dict = dict(locals_py) if locals_py else {}

            info = PauseInfo(
                line=line,
                col=col,
                locals=locals_dict,
                snippet=snippet,
                call_stack=list(call_stack) if call_stack else [],
                start_byte=start_byte,
            )
            self._last_pause = info
            self._state = DebugState.PAUSED
            return ('paused', info)

        elif event_type == 'finished':
            self._state = DebugState.FINISHED
            # data is a repr string — try to parse if simple
            return ('finished', data)

        elif event_type == 'error':
            self._state = DebugState.ERROR
            return ('error', data)

        return None
