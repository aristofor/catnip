# FILE: tests/language/test_fstrings.py
"""Tests for f-strings (formatted strings)."""

import pytest

from catnip import Catnip


class TestFStringsBasic:
    """Basic f-string tests."""

    def test_fstring_empty(self):
        """f-string vide."""
        c = Catnip()
        c.parse('result = f""')
        c.execute()
        assert c.context.globals['result'] == ""

    def test_fstring_text_only(self):
        """f-string avec seulement du texte."""
        c = Catnip()
        c.parse('result = f"Hello, World!"')
        c.execute()
        assert c.context.globals['result'] == "Hello, World!"

    def test_fstring_simple_variable(self):
        """f-string avec variable simple."""
        c = Catnip()
        c.parse('name = "Alice"\nresult = f"Hello, {name}!"')
        c.execute()
        assert c.context.globals['result'] == "Hello, Alice!"

    def test_fstring_multiple_variables(self):
        """f-string avec plusieurs variables."""
        c = Catnip()
        c.parse('name = "Bob"\nage = 25\nresult = f"{name} is {age} years old"')
        c.execute()
        assert c.context.globals['result'] == "Bob is 25 years old"

    def test_fstring_number(self):
        """f-string avec nombre."""
        c = Catnip()
        c.parse('x = 42\nresult = f"The answer is {x}"')
        c.execute()
        assert c.context.globals['result'] == "The answer is 42"


class TestFStringsExpressions:
    """F-string expression tests."""

    def test_fstring_arithmetic(self):
        """f-string with arithmetic."""
        c = Catnip()
        c.parse('a = 10\nb = 20\nresult = f"Sum: {a + b}"')
        c.execute()
        assert c.context.globals['result'] == "Sum: 30"

    def test_fstring_comparison(self):
        """f-string avec comparaison."""
        c = Catnip()
        c.parse('x = 5\nresult = f"Is x > 3? {x > 3}"')
        c.execute()
        assert c.context.globals['result'] == "Is x > 3? True"

    def test_fstring_function_call(self):
        """f-string avec appel de fonction."""
        c = Catnip()
        c.parse("""
square = (x) => { x * x }
n = 7
result = f"Square of {n} is {square(n)}"
""")
        c.execute()
        assert c.context.globals['result'] == "Square of 7 is 49"

    def test_fstring_builtin_function(self):
        """f-string avec fonction builtin."""
        c = Catnip()
        c.parse('numbers = list(1, 2, 3)\nresult = f"Length: {len(numbers)}"')
        c.execute()
        assert c.context.globals['result'] == "Length: 3"

    def test_fstring_complex_expression(self):
        """f-string avec expression complexe."""
        c = Catnip()
        c.parse('result = f"Result: {(10 + 20) * 2}"')
        c.execute()
        assert c.context.globals['result'] == "Result: 60"


class TestFStringsCaseInsensitive:
    """Case-insensitive f tag tests."""

    def test_lowercase_f(self):
        """Lowercase f."""
        c = Catnip()
        c.parse('result = f"value: {42}"')
        c.execute()
        assert c.context.globals['result'] == "value: 42"

    def test_uppercase_f(self):
        """Uppercase F."""
        c = Catnip()
        c.parse('result = F"value: {42}"')
        c.execute()
        assert c.context.globals['result'] == "value: 42"

    def test_mixed_case_produces_same_result(self):
        """f and F produce the same result."""
        c1 = Catnip()
        c1.parse('x = 10\nresult = f"x={x}"')
        c1.execute()

        c2 = Catnip()
        c2.parse('x = 10\nresult = F"x={x}"')
        c2.execute()

        assert c1.context.globals['result'] == c2.context.globals['result']


class TestFStringsEscapes:
    """F-string escape sequence tests."""

    def test_fstring_newline(self):
        """f-string with newline."""
        c = Catnip()
        c.parse(r'result = f"Line 1\nLine 2"')
        c.execute()
        assert c.context.globals['result'] == "Line 1\nLine 2"

    def test_fstring_tab(self):
        """f-string avec tabulation."""
        c = Catnip()
        c.parse(r'result = f"Col1\tCol2"')
        c.execute()
        assert c.context.globals['result'] == "Col1\tCol2"


class TestFStringsErrors:
    """F-string error handling tests."""

    def test_fstring_undefined_variable(self):
        """f-string with undefined variable."""
        c = Catnip()
        c.parse('result = f"Value: {undefined_var}"')
        with pytest.raises(Exception) as exc_info:
            c.execute()
        assert 'undefined_var' in str(exc_info.value)

    def test_fstring_syntax_error(self):
        """f-string avec erreur de syntaxe."""
        c = Catnip()
        with pytest.raises(SyntaxError) as exc_info:
            c.parse('result = f"Bad: {1 +}"')
        assert "Unexpected" in str(exc_info.value) or "line" in str(exc_info.value)


class TestFStringsAdvanced:
    """Advanced f-string tests."""

    def test_fstring_in_function(self):
        """f-string dans une fonction."""
        c = Catnip()
        c.parse("""
greet = (name) => { f"Hello, {name}!" }
result = greet("World")
""")
        c.execute()
        assert c.context.globals['result'] == "Hello, World!"

    def test_fstring_multiple_expressions(self):
        """f-string avec plusieurs expressions."""
        c = Catnip()
        c.parse('x = 5\ny = 3\nresult = f"{x} + {y} = {x + y}, {x} * {y} = {x * y}"')
        c.execute()
        assert c.context.globals['result'] == "5 + 3 = 8, 5 * 3 = 15"

    def test_fstring_nested_quotes(self):
        """f-string with nested quotes (via variable)."""
        c = Catnip()
        c.parse('text = "quoted"\nresult = f"Text: {text}"')
        c.execute()
        assert c.context.globals['result'] == "Text: quoted"


class TestMultilineStrings:
    """Multiline string tests."""

    def test_string_multiline_double_quotes(self):
        """Multiline string avec triple double quotes."""
        c = Catnip()
        c.parse('result = """Line 1\nLine 2\nLine 3"""')
        c.execute()
        assert c.context.globals['result'] == "Line 1\nLine 2\nLine 3"

    def test_string_multiline_single_quotes(self):
        """Multiline string avec triple simple quotes."""
        c = Catnip()
        c.parse("result = '''Line 1\nLine 2\nLine 3'''")
        c.execute()
        assert c.context.globals['result'] == "Line 1\nLine 2\nLine 3"

    def test_string_multiline_empty(self):
        """Multiline string vide."""
        c = Catnip()
        c.parse('result = """"""')
        c.execute()
        assert c.context.globals['result'] == ""

    def test_string_multiline_with_quotes(self):
        """Multiline string contenant des quotes simples."""
        c = Catnip()
        c.parse('result = """He said "Hello" to me"""')
        c.execute()
        assert c.context.globals['result'] == 'He said "Hello" to me'


class TestMultilineFStrings:
    """Tests des f-strings multilignes."""

    def test_fstring_multiline_double_quotes(self):
        """f-string multiligne avec triple double quotes."""
        c = Catnip()
        c.parse('x = 42\nresult = f"""Value:\n{x}"""')
        c.execute()
        assert c.context.globals['result'] == "Value:\n42"

    def test_fstring_multiline_single_quotes(self):
        """f-string multiligne avec triple simple quotes."""
        c = Catnip()
        c.parse("x = 42\nresult = f'''Value:\n{x}'''")
        c.execute()
        assert c.context.globals['result'] == "Value:\n42"

    def test_fstring_multiline_multiple_expressions(self):
        """f-string multiligne avec plusieurs expressions."""
        c = Catnip()
        c.parse('''name = "Catnip"
version = 1.0
result = f"""
Project: {name}
Version: {version}
"""''')
        c.execute()
        assert c.context.globals['result'] == "\nProject: Catnip\nVersion: 1.0\n"

    def test_fstring_multiline_uppercase_f(self):
        """f-string multiligne avec F majuscule."""
        c = Catnip()
        c.parse('x = 10\nresult = F"""Value: {x}"""')
        c.execute()
        assert c.context.globals['result'] == "Value: 10"

    def test_fstring_multiline_complex_expression(self):
        """f-string multiligne avec expression complexe."""
        c = Catnip()
        c.parse('''a = 5
b = 3
result = f"""
Sum: {a + b}
Product: {a * b}
"""''')
        c.execute()
        assert c.context.globals['result'] == "\nSum: 8\nProduct: 15\n"


class TestFStringsFormatSpec:
    """Tests for f-string format specifications."""

    def test_float_precision(self):
        c = Catnip()
        c.parse('x = 3.14159\nresult = f"{x:.2f}"')
        c.execute()
        assert c.context.globals['result'] == "3.14"

    def test_float_scientific(self):
        c = Catnip()
        c.parse('x = 12345.6789\nresult = f"{x:.2e}"')
        c.execute()
        assert c.context.globals['result'] == "1.23e+04"

    def test_string_padding_right(self):
        c = Catnip()
        c.parse('name = "Bob"\nresult = f"{name:>10}"')
        c.execute()
        assert c.context.globals['result'] == "       Bob"

    def test_string_padding_left(self):
        c = Catnip()
        c.parse('name = "Bob"\nresult = f"{name:<10}"')
        c.execute()
        assert c.context.globals['result'] == "Bob       "

    def test_string_padding_center(self):
        c = Catnip()
        c.parse('name = "Bob"\nresult = f"{name:^10}"')
        c.execute()
        assert c.context.globals['result'] == "   Bob    "

    def test_integer_zero_padded(self):
        c = Catnip()
        c.parse('x = 42\nresult = f"{x:05d}"')
        c.execute()
        assert c.context.globals['result'] == "00042"

    def test_integer_binary(self):
        c = Catnip()
        c.parse('x = 255\nresult = f"{x:08b}"')
        c.execute()
        assert c.context.globals['result'] == "11111111"

    def test_integer_hex(self):
        c = Catnip()
        c.parse('x = 255\nresult = f"{x:#x}"')
        c.execute()
        assert c.context.globals['result'] == "0xff"

    def test_percentage(self):
        c = Catnip()
        c.parse('x = 0.75\nresult = f"{x:.1%}"')
        c.execute()
        assert c.context.globals['result'] == "75.0%"

    def test_thousands_separator(self):
        c = Catnip()
        c.parse('x = 1000000\nresult = f"{x:,}"')
        c.execute()
        assert c.context.globals['result'] == "1,000,000"

    def test_fill_char(self):
        c = Catnip()
        c.parse('x = 42\nresult = f"{x:*>5}"')
        c.execute()
        assert c.context.globals['result'] == "***42"

    def test_format_spec_with_text(self):
        c = Catnip()
        c.parse('x = 3.14\nresult = f"pi is {x:.1f} approx"')
        c.execute()
        assert c.context.globals['result'] == "pi is 3.1 approx"


class TestFStringsConversion:
    """Tests for f-string conversion flags (!r, !s, !a)."""

    def test_repr_string(self):
        c = Catnip()
        c.parse('x = "hello"\nresult = f"{x!r}"')
        c.execute()
        assert c.context.globals['result'] == "'hello'"

    def test_repr_number(self):
        c = Catnip()
        c.parse('x = 42\nresult = f"{x!r}"')
        c.execute()
        assert c.context.globals['result'] == "42"

    def test_str_conversion(self):
        c = Catnip()
        c.parse('x = 42\nresult = f"{x!s}"')
        c.execute()
        assert c.context.globals['result'] == "42"

    def test_ascii_conversion(self):
        c = Catnip()
        c.parse('x = "hello"\nresult = f"{x!a}"')
        c.execute()
        assert c.context.globals['result'] == "'hello'"

    def test_repr_with_format_spec(self):
        c = Catnip()
        c.parse('x = "hi"\nresult = f"{x!r:>10}"')
        c.execute()
        assert c.context.globals['result'] == "      'hi'"

    def test_conversion_in_context(self):
        c = Catnip()
        c.parse('x = "world"\nresult = f"Hello {x!r}!"')
        c.execute()
        assert c.context.globals['result'] == "Hello 'world'!"


class TestFStringsDebug:
    """Tests for f-string debug syntax (=)."""

    def test_debug_simple(self):
        c = Catnip()
        c.parse('x = 42\nresult = f"{x=}"')
        c.execute()
        assert c.context.globals['result'] == "x=42"

    def test_debug_expression(self):
        c = Catnip()
        c.parse('x = 5\ny = 3\nresult = f"{x + y=}"')
        c.execute()
        assert c.context.globals['result'] == "x + y=8"

    def test_debug_with_format(self):
        c = Catnip()
        c.parse('x = 3.14159\nresult = f"{x=:.2f}"')
        c.execute()
        assert c.context.globals['result'] == "x=3.14"

    def test_debug_with_conversion(self):
        c = Catnip()
        c.parse('x = "hello"\nresult = f"{x=!r}"')
        c.execute()
        assert c.context.globals['result'] == "x='hello'"

    def test_debug_with_conversion_and_format(self):
        c = Catnip()
        c.parse('x = "hi"\nresult = f"{x=!r:>10}"')
        c.execute()
        assert c.context.globals['result'] == "x=      'hi'"

    def test_debug_in_context(self):
        c = Catnip()
        c.parse('x = 42\nresult = f"value: {x=}, done"')
        c.execute()
        assert c.context.globals['result'] == "value: x=42, done"

    def test_debug_comparison_not_confused(self):
        """== in expression is not confused with = debug flag."""
        c = Catnip()
        c.parse('x = 5\nresult = f"{x == 5}"')
        c.execute()
        assert c.context.globals['result'] == "True"
