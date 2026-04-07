# FILE: tests/language/test_with.py
"""Tests for context manager (with statement)."""

import pytest

from catnip import Catnip


class MockCM:
    """Mock context manager that tracks enter/exit calls."""

    def __init__(self, value="cm_value", suppress=False):
        self.value = value
        self.suppress = suppress
        self.entered = False
        self.exited = False
        self.exit_args = None

    def __enter__(self):
        self.entered = True
        return self.value

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.exited = True
        self.exit_args = (exc_type, exc_val, exc_tb)
        return self.suppress


class FailEnterCM:
    """Context manager whose __enter__ raises."""

    def __enter__(self):
        raise RuntimeError("enter failed")

    def __exit__(self, *args):
        raise AssertionError("__exit__ should not be called")


def _make_cat(**kwargs):
    """Create a Catnip instance with mock CMs injected into globals."""
    cat = Catnip(**{k: v for k, v in kwargs.items() if k == 'vm_mode'})
    for k, v in kwargs.items():
        if k != 'vm_mode':
            cat.context.globals[k] = v
    return cat


class TestWithBasic:
    """Basic context manager functionality."""

    def test_simple_with(self):
        cm = MockCM("hello")
        cat = _make_cat(cm=cm)
        cat.parse('with a = cm { a }')
        result = cat.execute()
        assert result == "hello"
        assert cm.entered
        assert cm.exited
        assert cm.exit_args == (None, None, None)

    def test_with_body_result(self):
        cm = MockCM(42)
        cat = _make_cat(cm=cm)
        cat.parse('with x = cm { x + 1 }')
        assert cat.execute() == 43

    def test_with_binding_visible_in_body(self):
        cm = MockCM(10)
        cat = _make_cat(cm=cm)
        cat.parse('''
        with val = cm {
            result = val * 2
            result
        }
        ''')
        assert cat.execute() == 20

    def test_with_exit_called_on_normal_exit(self):
        cm = MockCM()
        cat = _make_cat(cm=cm)
        cat.parse('with a = cm { 1 + 1 }')
        cat.execute()
        assert cm.exited
        assert cm.exit_args[0] is None


class TestWithExceptions:
    """Exception handling in with statements."""

    def test_exception_propagates(self):
        cm = MockCM()
        cat = _make_cat(cm=cm)
        cat.parse('with a = cm { raise ValueError("boom") }')
        with pytest.raises(Exception):
            cat.execute()
        assert cm.exited
        assert cm.exit_args[0] is not None

    def test_exception_suppressed(self):
        cm = MockCM(suppress=True)
        cat = _make_cat(cm=cm)
        cat.parse('with a = cm { raise ValueError("boom") }')
        # Should NOT raise because __exit__ returns True
        cat.execute()
        assert cm.exited

    def test_enter_fails_no_exit(self):
        cm = FailEnterCM()
        cat = _make_cat(cm=cm)
        cat.parse('with a = cm { 42 }')
        with pytest.raises(Exception, match="enter failed"):
            cat.execute()


class TestWithMultiple:
    """Multiple context managers in a single with statement."""

    def test_two_bindings(self):
        cm1 = MockCM("first")
        cm2 = MockCM("second")
        cat = _make_cat(cm1=cm1, cm2=cm2)
        cat.parse('with a = cm1, b = cm2 { a + " " + b }')
        assert cat.execute() == "first second"
        assert cm1.entered and cm1.exited
        assert cm2.entered and cm2.exited

    def test_cleanup_reverse_order(self):
        order = []

        class OrderedCM:
            def __init__(self, name):
                self.name = name

            def __enter__(self):
                order.append(f"enter_{self.name}")
                return self.name

            def __exit__(self, *args):
                order.append(f"exit_{self.name}")
                return False

        cm1 = OrderedCM("a")
        cm2 = OrderedCM("b")
        cat = _make_cat(cm1=cm1, cm2=cm2)
        cat.parse('with x = cm1, y = cm2 { x + y }')
        cat.execute()
        assert order == ["enter_a", "enter_b", "exit_b", "exit_a"]

    def test_second_enter_fails_first_cleaned(self):
        cm1 = MockCM("ok")

        class FailSecond:
            def __enter__(self):
                raise RuntimeError("second failed")

            def __exit__(self, *args):
                return False

        cat = _make_cat(cm1=cm1, bad=FailSecond())
        cat.parse('with a = cm1, b = bad { 42 }')
        with pytest.raises(Exception, match="second failed"):
            cat.execute()
        assert cm1.entered
        assert cm1.exited


class TestWithAST:
    """Context managers in AST execution mode."""

    def test_simple_with_ast(self):
        cm = MockCM(99)
        cat = _make_cat(vm_mode='off', cm=cm)
        cat.parse('with x = cm { x }')
        assert cat.execute() == 99
        assert cm.entered and cm.exited

    def test_exception_suppressed_ast(self):
        cm = MockCM(suppress=True)
        cat = _make_cat(vm_mode='off', cm=cm)
        cat.parse('with x = cm { raise ValueError("ast boom") }')
        cat.execute()
        assert cm.exited


class TestWithNested:
    """Nested with statements use unique temporaries."""

    def test_nested_with_cleanup(self):
        """Inner and outer CMs both get __exit__ called correctly."""
        order = []

        class TrackedCM:
            def __init__(self, name):
                self.name = name

            def __enter__(self):
                order.append(f"enter_{self.name}")
                return self.name

            def __exit__(self, *args):
                order.append(f"exit_{self.name}")
                return False

        cat = _make_cat(cm1=TrackedCM("outer"), cm2=TrackedCM("inner"))
        cat.parse('''
        with a = cm1 {
            with b = cm2 {
                a + " " + b
            }
        }
        ''')
        assert cat.execute() == "outer inner"
        assert order == ["enter_outer", "enter_inner", "exit_inner", "exit_outer"]

    def test_nested_with_inner_exception(self):
        """Exception in inner with doesn't corrupt outer cleanup."""
        outer = MockCM("outer_val")
        inner = MockCM("inner_val", suppress=True)
        cat = _make_cat(cm1=outer, cm2=inner)
        cat.parse('''
        with a = cm1 {
            with b = cm2 {
                raise ValueError("inner boom")
            }
        }
        ''')
        cat.execute()
        assert outer.exited
        assert inner.exited
        assert outer.exit_args == (None, None, None)
        assert inner.exit_args[0] is not None


@pytest.mark.no_standalone
class TestWithExcInfo:
    """Exception type/value passed correctly to __exit__."""

    def test_exit_receives_exception_class(self):
        cm = MockCM()
        cat = _make_cat(cm=cm)
        cat.parse('with a = cm { raise ValueError("test") }')
        with pytest.raises(Exception):
            cat.execute()
        # __exit__ should receive the real exception class, not a string
        assert cm.exit_args[0] is ValueError

    def test_exit_receives_exception_instance(self):
        cm = MockCM()
        cat = _make_cat(cm=cm)
        cat.parse('with a = cm { raise TypeError("bad type") }')
        with pytest.raises(Exception):
            cat.execute()
        assert cm.exit_args[0] is TypeError
        assert isinstance(cm.exit_args[1], TypeError)
        assert str(cm.exit_args[1]) == "bad type"

    def test_contextlib_suppress(self):
        """contextlib.suppress works with real exception types."""
        import contextlib

        cat = _make_cat(cm=contextlib.suppress(ValueError))
        cat.parse('with a = cm { raise ValueError("suppressed") }')
        # Should NOT raise
        cat.execute()


class TestWithRealCM:
    """With statement with real Python context managers."""

    def test_open_devnull(self):
        cat = Catnip()
        cat.parse('''
        import('io')
        with f = io.open('/dev/null') {
            "opened"
        }
        ''')
        assert cat.execute() == "opened"
