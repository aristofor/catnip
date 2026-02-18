# FILE: tests/tools/test_formatter.py
"""Tests for the Catnip code formatter."""

import pytest
from catnip.tools import format_code
from catnip._rs import FormatConfig


class TestFormatCode:
    """Test suite for format_code function."""

    def test_comparison_operators(self):
        result = format_code('x==1 and y>=2')
        assert result == 'x == 1 and y >= 2\n'

    def test_function_definition(self):
        result = format_code('f=(x,y)=>{x+y}')
        assert result == 'f = (x, y) => { x + y }\n'

    def test_block_indentation(self):
        code = '{\nx=1\n}'
        result = format_code(code)
        assert result == '{\n    x = 1\n}\n'

    def test_nested_blocks(self):
        code = '{\n{\nx=1\n}\n}'
        result = format_code(code)
        assert result == '{\n    {\n        x = 1\n    }\n}\n'

    def test_comments(self):
        code = 'x=1  # comment'
        result = format_code(code)
        # Comments should be preserved
        assert '# comment' in result

    def test_match_expression(self):
        code = 'match x{1=>a|2=>b|_=>c}'
        result = format_code(code)
        assert result == 'match x { 1 => a | 2 => b | _ => c }\n'

    def test_if_statement(self):
        code = 'if x==1{y=2}'
        result = format_code(code)
        assert 'if x == 1 { y = 2 }' in result

    def test_normalize_multiple_newlines(self):
        code = 'x=1\n\n\n\ny=2'
        result = format_code(code)
        # Max 2 consecutive newlines
        assert '\n\n\n' not in result

    def test_strip_leading_newlines(self):
        code = '\n\n\nx = 1'
        result = format_code(code)
        assert result == 'x = 1\n'

    def test_preserves_single_newlines(self):
        code = 'x = 1\ny = 2'
        result = format_code(code)
        assert result == 'x = 1\ny = 2\n'

    def test_trailing_newline(self):
        code = 'x = 1'
        result = format_code(code)
        assert result.endswith('\n')
        assert not result.endswith('\n\n')


class TestFormatConfig:
    """Test suite for FormatConfig."""

    def test_default_config(self):
        config = FormatConfig()
        assert config.indent_size == 4
        assert config.line_length == 120

    def test_custom_config(self):
        config = FormatConfig(indent_size=2, line_length=100)
        assert config.indent_size == 2
        assert config.line_length == 100

    def test_config_from_toml(self):
        toml_text = """
[format]
indent_size = 8
line_length = 120
"""
        config = FormatConfig.from_toml_section(toml_text)
        assert config.indent_size == 8
        assert config.line_length == 120

    def test_config_from_toml_partial(self):
        toml_text = """
[format]
indent_size = 2
"""
        config = FormatConfig.from_toml_section(toml_text)
        assert config.indent_size == 2
        assert config.line_length == 120  # Default

    def test_config_from_toml_no_section(self):
        toml_text = """
jit = false
tco = true
"""
        config = FormatConfig.from_toml_section(toml_text)
        # Should use defaults if no [format] section
        assert config.indent_size == 4
        assert config.line_length == 120

    def test_config_from_toml_invalid_value(self):
        toml_text = """
[format]
indent_size = invalid
"""
        with pytest.raises(ValueError):
            FormatConfig.from_toml_section(toml_text)


class TestUnaryOperators:
    """Test unary operator formatting (no space between operator and operand)."""

    def test_unary_minus(self):
        assert format_code('x = -1') == 'x = -1\n'

    def test_unary_minus_in_expr(self):
        assert format_code('y = a + -b') == 'y = a + -b\n'

    def test_unary_plus(self):
        assert format_code('y = +x') == 'y = +x\n'

    def test_unary_tilde(self):
        assert format_code('y = ~a') == 'y = ~a\n'

    def test_binary_minus_preserved(self):
        assert format_code('a - b') == 'a - b\n'

    def test_double_negative(self):
        assert format_code('x = --y') == 'x = --y\n'

    def test_unary_after_paren(self):
        assert format_code('f(-x)') == 'f(-x)\n'

    def test_unary_after_comma(self):
        assert format_code('f(a, -b)') == 'f(a, -b)\n'

    def test_unary_after_equal(self):
        assert format_code('x = -y') == 'x = -y\n'


class TestNotKeyword:
    """Test 'not' keyword spacing."""

    def test_not_keyword(self):
        assert format_code('z = not x') == 'z = not x\n'

    def test_not_in_expr(self):
        assert format_code('w = a and not b') == 'w = a and not b\n'


class TestSemicolons:
    """Test semicolon spacing."""

    def test_semicolons(self):
        assert format_code('x = 1; y = 2') == 'x = 1; y = 2\n'

    def test_triple_semicolons(self):
        assert format_code('a = 1; b = 2; c = 3') == 'a = 1; b = 2; c = 3\n'


class TestFstringsAndBstrings:
    """Test f-string and b-string formatting."""

    def test_fstring(self):
        result = format_code('x = f"hello {name}"')
        assert 'f"hello {name}"' in result

    def test_bstring(self):
        result = format_code("x = b'bytes'")
        assert "b'bytes'" in result


class TestLineWrapping:
    """Test line wrapping for long lines."""

    def test_long_line_wraps(self):
        config = FormatConfig(indent_size=4, line_length=30)
        code = 'f(aaaa, bbbb, cccc, dddd, eeee)'
        result = format_code(code, config)
        assert all(len(line) <= 30 for line in result.strip().split('\n'))

    def test_short_line_unchanged(self):
        config = FormatConfig(indent_size=4, line_length=120)
        assert format_code('x = 1 + 2', config) == 'x = 1 + 2\n'

    def test_operator_wrapping(self):
        config = FormatConfig(indent_size=4, line_length=20)
        code = 'result = a + b + c + d + e'
        result = format_code(code, config)
        assert all(len(line) <= 20 for line in result.strip().split('\n'))

    def test_wrapping_preserves_short_lines(self):
        """Lines within limit are not modified by wrapping."""
        config = FormatConfig(indent_size=4, line_length=80)
        code = 'f(a, b, c)\n'
        result = format_code(code, config)
        assert result == 'f(a, b, c)\n'


class TestDecorators:
    """Test decorator formatting."""

    def test_decorator(self):
        assert format_code('@pure\nf = (x) => { x }').startswith('@pure\n')


class TestRealWorldExamples:
    """Test formatting on real-world-like code."""

    def test_factorial(self):
        code = """
factorial=(n)=>{
if n<=1{
1
}else{
n*factorial(n-1)
}
}
"""
        result = format_code(code)
        # Check basic formatting
        assert 'factorial = (n) => {' in result
        assert 'if n <= 1 {' in result
        assert 'n * factorial(n - 1)' in result

    def test_list_operations(self):
        code = 'x=[1,2,3]\ny=x.[*2]'
        result = format_code(code)
        assert 'x = [1, 2, 3]' in result
        assert 'y = x.[* 2]' in result

    def test_complex_expression(self):
        code = 'result=(a+b)*(c-d)/2'
        result = format_code(code)
        assert 'result = (a + b) * (c - d) / 2' in result
