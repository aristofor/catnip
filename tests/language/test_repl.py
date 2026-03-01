# FILE: tests/language/test_repl.py
"""
Tests for the REPL module.

Tests completer, multiline detection, command parsing, and preprocessing.
"""

import pytest

from catnip.repl import (
    parse_repl_command,
    preprocess_multiline,
    should_continue_multiline,
)


class TestMultilineContinuation:
    """Test should_continue_multiline detection."""

    # --- Unclosed delimiters ---

    def test_unclosed_brace(self):
        """Unclosed { triggers continuation."""
        assert should_continue_multiline("if x > 0 {") is True

    def test_unclosed_paren(self):
        """Unclosed ( triggers continuation."""
        assert should_continue_multiline("fn(a, b") is True

    def test_unclosed_bracket(self):
        """Unclosed [ triggers continuation."""
        assert should_continue_multiline("list = [1, 2, 3") is True

    def test_multiple_unclosed(self):
        """Multiple unclosed delimiters trigger continuation."""
        assert should_continue_multiline("fn({a: [1") is True

    def test_closed_delimiters(self):
        """Closed delimiters don't trigger continuation."""
        assert should_continue_multiline("fn(a, b)") is False
        assert should_continue_multiline("{a: 1}") is False
        assert should_continue_multiline("[1, 2, 3]") is False

    def test_nested_closed(self):
        """Nested but closed delimiters don't trigger continuation."""
        assert should_continue_multiline("fn({a: [1, 2]})") is False

    # --- Continuation operators ---

    @pytest.mark.parametrize("op", ["+", "-", "*", "/", "//", "%", "**"])
    def test_arithmetic_operators(self, op):
        """Arithmetic operators trigger continuation."""
        assert should_continue_multiline(f"a {op}") is True

    @pytest.mark.parametrize("op", ["&", "|", "^", "<<", ">>"])
    def test_bitwise_operators(self, op):
        """Bitwise operators trigger continuation."""
        assert should_continue_multiline(f"a {op}") is True

    @pytest.mark.parametrize("op", ["==", "!=", "<", ">", "<=", ">="])
    def test_comparison_operators(self, op):
        """Comparison operators trigger continuation."""
        assert should_continue_multiline(f"a {op}") is True

    def test_comma_operator(self):
        """Comma triggers continuation."""
        assert should_continue_multiline("a,") is True
        assert should_continue_multiline("fn(a, b,") is True

    def test_assignment_operator(self):
        """Assignment triggers continuation."""
        assert should_continue_multiline("x =") is True

    def test_trailing_whitespace(self):
        """Trailing whitespace doesn't affect detection."""
        assert should_continue_multiline("a +   ") is True
        assert should_continue_multiline("a + b   ") is False

    # --- Continuation keywords ---

    @pytest.mark.parametrize("kw", ["if", "elif", "else", "while", "for", "match"])
    def test_continuation_keywords(self, kw):
        """Keywords at end trigger continuation."""
        assert should_continue_multiline(f"x = {kw}") is True

    def test_keyword_in_middle_no_continuation(self):
        """Keyword not at end doesn't trigger continuation."""
        assert should_continue_multiline("if x > 0 { 1 }") is False

    # --- Edge cases ---

    def test_empty_string(self):
        """Empty string doesn't trigger continuation."""
        assert should_continue_multiline("") is False

    def test_whitespace_only(self):
        """Whitespace-only doesn't trigger continuation."""
        assert should_continue_multiline("   \t  ") is False

    def test_complete_expression(self):
        """Complete expression doesn't trigger continuation."""
        assert should_continue_multiline("x = 1 + 2") is False


class TestPreprocessMultiline:
    """Test preprocess_multiline code preprocessing."""

    def test_single_line_unchanged(self):
        """Single line passes through unchanged."""
        code = "x = 1 + 2"
        assert preprocess_multiline(code) == code

    def test_multiline_with_continuation(self):
        """Lines ending with operator are joined."""
        code = "a +\nb"
        result = preprocess_multiline(code)
        assert result == "a + b"

    def test_multiline_preserves_complete_lines(self):
        """Complete lines are preserved."""
        code = "x = 1\ny = 2"
        result = preprocess_multiline(code)
        assert result == "x = 1\ny = 2"

    def test_chain_continuations(self):
        """Multiple continuation lines are chained."""
        code = "a +\nb +\nc"
        result = preprocess_multiline(code)
        assert result == "a + b + c"

    def test_comma_continuation(self):
        """Comma continuations are handled."""
        code = "fn(a,\nb,\nc)"
        result = preprocess_multiline(code)
        assert result == "fn(a, b, c)"

    def test_mixed_lines(self):
        """Mix of continuation and complete lines."""
        code = "x = 1\na +\nb\ny = 2"
        result = preprocess_multiline(code)
        lines = result.split("\n")
        assert lines[0] == "x = 1"
        assert lines[1] == "a + b"
        assert lines[2] == "y = 2"

    def test_preserves_indentation_in_complete_lines(self):
        """Complete lines preserve their indentation."""
        code = "if x {\n  y\n}"
        result = preprocess_multiline(code)
        assert result == code

    def test_strips_leading_whitespace_in_continuation(self):
        """Continuation lines have leading whitespace stripped."""
        code = "a +\n    b"
        result = preprocess_multiline(code)
        assert result == "a + b"


class TestParseReplCommand:
    """Test parse_repl_command parsing."""

    def test_parse_simple_command(self):
        """Parse command without args."""
        cmd, args = parse_repl_command("/help")
        assert cmd == "help"
        assert args is None

    def test_parse_command_with_args(self):
        """Parse command with args."""
        cmd, args = parse_repl_command("/inspect myvar")
        assert cmd == "inspect"
        assert args == "myvar"

    def test_parse_command_with_multiple_args(self):
        """Args after first space are preserved."""
        cmd, args = parse_repl_command("/verbose on now")
        assert cmd == "verbose"
        assert args == "on now"

    def test_parse_case_insensitive(self):
        """Command names are lowercased."""
        cmd, args = parse_repl_command("/HELP")
        assert cmd == "help"

    def test_parse_with_leading_whitespace(self):
        """Leading whitespace in args is handled."""
        cmd, args = parse_repl_command("/inspect   myvar")
        assert cmd == "inspect"
        assert args == "myvar"

    def test_not_a_command(self):
        """Non-command returns None."""
        cmd, args = parse_repl_command("x = 1")
        assert cmd is None
        assert args is None

    def test_empty_command(self):
        """Just slash returns None."""
        cmd, args = parse_repl_command("/")
        assert cmd is None
        assert args is None

    def test_slash_with_whitespace(self):
        """Slash with only whitespace returns None."""
        cmd, args = parse_repl_command("/   ")
        assert cmd is None
        assert args is None

    def test_valid_commands(self):
        """All valid REPL commands parse correctly."""
        commands = [
            "help",
            "exit",
            "quit",
            "clear",
            "vars",
            "inspect",
            "verbose",
            "colors",
        ]
        for name in commands:
            cmd, _ = parse_repl_command(f"/{name}")
            assert cmd == name


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
