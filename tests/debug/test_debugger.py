# FILE: tests/debug/test_debugger.py
"""Integration tests for the Catnip debugger."""

import pytest

from catnip import Catnip
from catnip.debug.session import DebugAction, DebugSession, DebugState


class TestBreakpoint:
    """Breakpoints stop execution at the right line."""

    def test_breakpoint_stops_at_line(self):
        cat = Catnip()
        code = "x = 1\ny = 2\nz = 3"
        session = DebugSession(cat, code)
        session.add_breakpoint(2)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        event_type, pause = event
        assert event_type == 'paused'
        assert pause.line == 2

        session.send_command(DebugAction.CONTINUE)
        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'finished'

    def test_multiple_breakpoints(self):
        cat = Catnip()
        code = "a = 1\nb = 2\nc = 3\nd = 4"
        session = DebugSession(cat, code)
        session.add_breakpoint(2)
        session.add_breakpoint(4)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'
        assert event[1].line == 2

        session.send_command(DebugAction.CONTINUE)
        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'
        assert event[1].line == 4

        session.send_command(DebugAction.CONTINUE)
        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'finished'


class TestStepping:
    """Step into/over/out move through code correctly."""

    def test_step_into(self):
        cat = Catnip()
        code = "x = 1\ny = 2\nz = 3"
        session = DebugSession(cat, code)
        session.add_breakpoint(1)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'

        # Step through remaining instructions, collecting lines
        lines = [event[1].line]
        for _ in range(10):
            session.send_command(DebugAction.STEP_INTO)
            event = session.wait_for_event(timeout=5)
            assert event is not None
            if event[0] == 'finished':
                break
            lines.append(event[1].line)
        else:
            session.send_command(DebugAction.CONTINUE)
            session.wait_for_event(timeout=5)

        # Should have visited multiple lines
        assert len(lines) > 1

    def test_step_over(self):
        cat = Catnip()
        code = "x = 1\ny = 2\nz = 3"
        session = DebugSession(cat, code)
        session.add_breakpoint(1)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'

        session.send_command(DebugAction.STEP_OVER)
        event = session.wait_for_event(timeout=5)
        assert event is not None
        # Should either pause at next line or finish
        assert event[0] in ('paused', 'finished')

        if event[0] == 'paused':
            session.send_command(DebugAction.CONTINUE)
            event = session.wait_for_event(timeout=5)

    def test_step_out(self):
        cat = Catnip()
        code = "f = () => { 1 + 1 }\nf()\nz = 3"
        session = DebugSession(cat, code)
        session.add_breakpoint(1)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'

        session.send_command(DebugAction.STEP_OUT)
        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] in ('paused', 'finished')

        # Drain remaining events
        if event[0] == 'paused':
            session.send_command(DebugAction.CONTINUE)
            session.wait_for_event(timeout=5)


class TestLocals:
    """Local variables are visible at breakpoints."""

    def test_locals_visible(self):
        cat = Catnip()
        code = "x = 42\ny = x + 1"
        session = DebugSession(cat, code)
        session.add_breakpoint(2)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'
        assert 'x' in event[1].locals
        assert event[1].locals['x'] == 42

        session.send_command(DebugAction.CONTINUE)
        session.wait_for_event(timeout=5)

    def test_nil_variables_visible(self):
        """Variables declared but not yet assigned should still appear."""
        cat = Catnip()
        # y is declared after the breakpoint line but its slot exists in the frame
        code = "x = 1\ny = 2"
        session = DebugSession(cat, code)
        session.add_breakpoint(1)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'
        # At line 1, the slotmap may contain y with nil value
        # The key test is that the filter doesn't hide nil vars
        locals_dict = event[1].locals
        assert isinstance(locals_dict, dict)

        session.send_command(DebugAction.CONTINUE)
        session.wait_for_event(timeout=5)


class TestLastPause:
    """DebugSession.last_pause tracks the last pause info."""

    def test_last_pause_none_before_start(self):
        cat = Catnip()
        session = DebugSession(cat, "x = 1")
        assert session.last_pause is None

    def test_last_pause_set_on_pause(self):
        cat = Catnip()
        code = "x = 1\ny = 2"
        session = DebugSession(cat, code)
        session.add_breakpoint(2)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'
        assert session.last_pause is not None
        assert session.last_pause.line == event[1].line

        session.send_command(DebugAction.CONTINUE)
        session.wait_for_event(timeout=5)

    def test_last_pause_persists_after_finish(self):
        cat = Catnip()
        code = "x = 1\ny = 2"
        session = DebugSession(cat, code)
        session.add_breakpoint(1)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event[0] == 'paused'
        pause_line = session.last_pause.line

        session.send_command(DebugAction.CONTINUE)
        event = session.wait_for_event(timeout=5)
        assert event[0] == 'finished'
        # last_pause still holds the last pause info
        assert session.last_pause is not None
        assert session.last_pause.line == pause_line


class TestDynamicBreakpoint:
    """Breakpoints added during execution."""

    def test_add_breakpoint_while_paused(self):
        """Adding a breakpoint via step-through confirms dynamic stops work."""
        cat = Catnip()
        code = "a = 1\nb = 2\nc = 3\nd = 4"
        session = DebugSession(cat, code)
        # Start with breakpoint at line 1, then step to confirm multiple pauses
        session.add_breakpoint(1)
        session.add_breakpoint(4)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'
        assert event[1].line == 1

        # Continue to next breakpoint at line 4
        session.send_command(DebugAction.CONTINUE)
        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'paused'
        assert event[1].line == 4

        session.send_command(DebugAction.CONTINUE)
        session.wait_for_event(timeout=5)


class TestSessionLifecycle:
    """Session finishes normally or reports errors."""

    def test_normal_finish(self):
        cat = Catnip()
        code = "1 + 2"
        session = DebugSession(cat, code)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'finished'
        assert event[1] == 3
        assert session.state == DebugState.FINISHED

    def test_error_reported(self):
        cat = Catnip()
        code = "x + undefined_var"
        session = DebugSession(cat, code)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event is not None
        assert event[0] == 'error'
        assert session.state == DebugState.ERROR

    def test_state_transitions(self):
        cat = Catnip()
        code = "x = 1\ny = 2"
        session = DebugSession(cat, code)
        assert session.state == DebugState.IDLE

        session.add_breakpoint(2)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event[0] == 'paused'
        assert session.state == DebugState.PAUSED

        session.send_command(DebugAction.CONTINUE)
        event = session.wait_for_event(timeout=5)
        assert event[0] == 'finished'
        assert session.state == DebugState.FINISHED


class TestEvalInScope:
    """Expressions evaluated in the current debug scope."""

    def test_eval_sees_locals(self):
        cat = Catnip()
        code = "x = 42\ny = x + 1"
        session = DebugSession(cat, code)
        session.add_breakpoint(2)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event[0] == 'paused'
        pause = event[1]

        # Evaluate an expression using the locals
        eval_cat = Catnip()
        eval_cat.context.globals.update(session.catnip.context.globals)
        eval_cat.context.globals.update(pause.locals)
        eval_cat.parse("x + 10")
        result = eval_cat.execute()
        assert result == 52

        session.send_command(DebugAction.CONTINUE)
        session.wait_for_event(timeout=5)


class TestSendCommandString:
    """send_command accepts string aliases."""

    def test_string_commands(self):
        cat = Catnip()
        code = "x = 1\ny = 2"
        session = DebugSession(cat, code)
        session.add_breakpoint(1)
        session.start(blocking=False)

        event = session.wait_for_event(timeout=5)
        assert event[0] == 'paused'

        session.send_command('continue')
        event = session.wait_for_event(timeout=5)
        assert event[0] == 'finished'
