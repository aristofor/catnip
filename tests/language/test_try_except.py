# FILE: tests/language/test_try_except.py
"""Tests for try/except/finally and raise."""

import pytest
from catnip import Catnip
from catnip.exc import CatnipRuntimeError


@pytest.fixture
def cat():
    return Catnip()


def run(cat, code):
    """Parse and execute code, return result."""
    cat.parse(code)
    return cat.execute()


# ============================================================================
# Basic try/except
# ============================================================================


class TestTryExceptBasic:
    def test_try_no_exception(self, cat):
        result = run(cat, "try { 42 } except { _ => { 0 } }")
        assert result == 42

    def test_try_catch_wildcard(self, cat):
        result = run(cat, "try { 1/0 } except { _ => { 99 } }")
        assert result == 99

    def test_try_catch_typed(self, cat):
        result = run(cat, 'try { 1/0 } except { e: ZeroDivisionError => { 42 } }')
        assert result == 42

    def test_try_catch_wrong_type_propagates(self, cat):
        with pytest.raises(Exception):
            run(cat, 'try { 1/0 } except { e: TypeError => { 42 } }')

    def test_try_multiple_clauses(self, cat):
        code = """
        try {
            1/0
        } except {
            e: TypeError => { 1 }
            e: ZeroDivisionError => { 2 }
            _ => { 3 }
        }
        """
        assert run(cat, code) == 2

    def test_try_wildcard_fallback(self, cat):
        code = """
        try {
            x = undefined_var
        } except {
            e: TypeError => { 1 }
            _ => { 99 }
        }
        """
        assert run(cat, code) == 99

    def test_try_binding_is_string(self, cat):
        """Bound exception is the error message string."""
        code = 'try { 1/0 } except { e: ZeroDivisionError => { e } }'
        result = run(cat, code)
        assert isinstance(result, str)
        assert "division" in result.lower() or "zero" in result.lower()


# ============================================================================
# Finally
# ============================================================================


class TestFinally:
    def test_try_finally_normal(self, cat):
        """Finally executes on normal exit."""
        code = """
        x = 0
        try {
            x = 1
        } finally {
            x = x + 10
        }
        x
        """
        assert run(cat, code) == 11

    def test_try_finally_on_exception(self, cat):
        """Finally executes even when exception is raised."""
        code = """
        x = 0
        try {
            x = 1
            1/0
        } except {
            _ => { x = x + 100 }
        } finally {
            x = x + 10
        }
        x
        """
        assert run(cat, code) == 111

    def test_try_finally_propagates_unhandled(self, cat):
        """Finally runs then unhandled exception propagates."""
        code = """
        x = 0
        try {
            1/0
        } finally {
            x = 42
        }
        """
        with pytest.raises(Exception):
            run(cat, code)

    def test_try_except_finally(self, cat):
        code = """
        try {
            1/0
        } except {
            e: ZeroDivisionError => { 42 }
        } finally {
            x = 1
        }
        """
        assert run(cat, code) == 42


# ============================================================================
# Raise
# ============================================================================


class TestRaise:
    def test_raise_string(self, cat):
        with pytest.raises(CatnipRuntimeError, match="boom"):
            run(cat, 'raise "boom"')

    def test_raise_caught_by_except(self, cat):
        code = """
        try {
            raise "oops"
        } except {
            _ => { 42 }
        }
        """
        assert run(cat, code) == 42

    def test_raise_builtin_exception(self, cat):
        """raise ValueError("msg") creates and raises a Python ValueError."""
        with pytest.raises(Exception, match="test error"):
            run(cat, 'raise ValueError("test error")')

    def test_raise_builtin_caught(self, cat):
        code = """
        try {
            raise ValueError("oops")
        } except {
            e: ValueError => { e }
        }
        """
        result = run(cat, code)
        assert "oops" in str(result)

    def test_bare_raise_in_except(self, cat):
        """Bare raise inside except re-raises the caught exception."""
        code = """
        try {
            try {
                1/0
            } except {
                _ => { raise }
            }
        } except {
            e: ZeroDivisionError => { 42 }
        }
        """
        assert run(cat, code) == 42

    def test_bare_raise_after_nested_try(self, cat):
        """Bare raise works after a nested try/except inside the handler."""
        code = """
        try {
            try {
                1/0
            } except {
                _ => {
                    try { x = undefined_var } except { _ => { 0 } }
                    raise
                }
            }
        } except {
            e: ZeroDivisionError => { 42 }
        }
        """
        assert run(cat, code) == 42

    def test_bare_raise_outside_except(self, cat):
        with pytest.raises(CatnipRuntimeError, match="no active exception"):
            run(cat, "raise")


# ============================================================================
# Control flow + finally
# ============================================================================


class TestControlFlowFinally:
    def test_return_through_finally(self, cat):
        """Finally executes before return propagates."""
        code = """
        x = 0
        f = () => {
            try {
                return 42
            } finally {
                x = 99
            }
        }
        f()
        x
        """
        assert run(cat, code) == 99

    def test_break_through_finally(self, cat):
        """Finally executes before break propagates."""
        code = """
        x = 0
        for i in range(3) {
            try {
                break
            } finally {
                x = x + 1
            }
        }
        x
        """
        assert run(cat, code) == 1

    def test_nested_try(self, cat):
        code = """
        try {
            try {
                1/0
            } except {
                e: ZeroDivisionError => { 42 }
            }
        } except {
            _ => { 0 }
        }
        """
        assert run(cat, code) == 42


# ============================================================================
# Exception hierarchy
# ============================================================================


class TestExceptionHierarchy:
    """except matches by MRO, not just exact type name."""

    def test_except_exception_catches_all(self, cat):
        """except Exception catches any built-in exception type."""
        for exc in [
            '1/0',  # ZeroDivisionError
            '1 + "x"',  # TypeError
            '[1,2,3][99]',  # IndexError
            'undefined_var',  # NameError
        ]:
            result = run(cat, f'try {{ {exc} }} except {{ e: Exception => {{ "caught" }} }}')
            assert result == "caught", f"Exception did not catch error from: {exc}"

    def test_except_arithmetic_catches_zero_div(self, cat):
        result = run(cat, 'try { 1/0 } except { e: ArithmeticError => { "caught" } }')
        assert result == "caught"

    def test_except_arithmetic_not_type_error(self, cat):
        """ArithmeticError does NOT catch TypeError."""
        with pytest.raises(Exception):
            run(cat, 'try { 1 + "x" } except { e: ArithmeticError => { "nope" } }')

    def test_except_lookup_catches_raised_index(self, cat):
        """LookupError catches a raised IndexError (VM-internal path)."""
        result = run(cat, 'try { raise IndexError("oob") } except { e: LookupError => { "caught" } }')
        assert result == "caught"

    def test_except_lookup_catches_raised_key(self, cat):
        result = run(cat, 'try { raise KeyError("missing") } except { e: LookupError => { "caught" } }')
        assert result == "caught"

    def test_except_lookup_not_type_error(self, cat):
        """LookupError does NOT catch TypeError."""
        with pytest.raises(Exception):
            run(cat, 'try { 1 + "x" } except { e: LookupError => { "nope" } }')

    def test_specific_before_general(self, cat):
        """First matching handler wins (specific before general)."""
        code = """
        try { 1/0 } except {
            e: ZeroDivisionError => { "specific" }
            e: ArithmeticError => { "general" }
            e: Exception => { "catchall" }
        }
        """
        assert run(cat, code) == "specific"

    def test_general_catches_when_specific_misses(self, cat):
        code = """
        try { 1/0 } except {
            e: TypeError => { "wrong" }
            e: ArithmeticError => { "right" }
        }
        """
        assert run(cat, code) == "right"

    def test_raise_exception_directly(self, cat):
        """Can raise and catch Exception directly."""
        code = 'try { raise Exception("boom") } except { e: Exception => { e } }'
        result = run(cat, code)
        assert "boom" in str(result)

    def test_raise_arithmetic_error(self, cat):
        """Can raise ArithmeticError and catch it."""
        code = 'try { raise ArithmeticError("math") } except { e: ArithmeticError => { e } }'
        result = run(cat, code)
        assert "math" in str(result)

    def test_raise_lookup_error(self, cat):
        code = 'try { raise LookupError("lookup") } except { e: LookupError => { e } }'
        result = run(cat, code)
        assert "lookup" in str(result)

    def test_bare_raise_preserves_grouped_type(self, cat):
        """Bare raise must preserve ArithmeticError identity through rethrow."""
        code = """
        try {
            try { raise ArithmeticError("x") } except { _ => { raise } }
        } except {
            e: ArithmeticError => { "caught" }
        }
        """
        assert run(cat, code) == "caught"

    def test_finally_unwind_preserves_grouped_type(self, cat):
        """Finally unwind must preserve Exception identity."""
        code = """
        try {
            try {
                raise LookupError("x")
            } finally {
                x = 1
            }
        } except {
            e: LookupError => { "caught" }
        }
        """
        assert run(cat, code) == "caught"

    def test_nested_rethrow_preserves_hierarchy(self, cat):
        """Double rethrow preserves the original exception type."""
        code = """
        try {
            try {
                try { 1/0 } except { _ => { raise } }
            } except { _ => { raise } }
        } except {
            e: ArithmeticError => { "caught" }
        }
        """
        assert run(cat, code) == "caught"


class TestUserExceptionHierarchy:
    """User-defined structs extending exception types inherit their MRO.

    The MRO resolver falls back to ExceptionKind for built-in exception types,
    so extends(RuntimeError) etc. works in both VM and AST modes.
    """

    def test_user_exception_caught_by_parent(self, cat):
        code = """
        struct AppError extends(RuntimeError) { message }
        try {
            raise AppError("internal")
        } except {
            e: RuntimeError => { e }
        }
        """
        result = run(cat, code)
        assert "internal" in str(result)

    def test_user_exception_caught_by_exception(self, cat):
        code = """
        struct MyError extends(ValueError) { message }
        try { raise MyError("boom") } except { e: Exception => { "ok" } }
        """
        assert run(cat, code) == "ok"

    def test_deep_user_hierarchy(self, cat):
        code = """
        struct BaseError extends(RuntimeError) { message }
        struct SpecificError extends(BaseError) { }
        try { raise SpecificError("deep") } except { e: RuntimeError => { e } }
        """
        result = run(cat, code)
        assert "deep" in str(result)

    def test_user_exception_not_caught_by_unrelated(self, cat):
        """A user exception extending RuntimeError is NOT caught by TypeError."""
        code = """
        struct AppError extends(RuntimeError) { message }
        try { raise AppError("nope") } except { e: TypeError => { "wrong" } }
        """
        with pytest.raises(Exception):
            run(cat, code)
