# FILE: tests/language/test_statement_separators.py
"""
Tests for statement separators and terminators.
Verify that newlines, semicolons, and multiline expressions work correctly.
"""

import unittest

from catnip import Catnip


class TestStatementSeparators(unittest.TestCase):
    """Statement separator tests (newlines and semicolons)."""

    def test_newline_separator(self):
        """Newline separates two statements."""
        c = Catnip()
        c.parse("x = 10\ny = 20")
        c.execute()
        self.assertEqual(c.context.globals['x'], 10)
        self.assertEqual(c.context.globals['y'], 20)

    def test_semicolon_separator(self):
        """Semicolon separates two statements."""
        c = Catnip()
        c.parse("x = 10; y = 20")
        c.execute()
        self.assertEqual(c.context.globals['x'], 10)
        self.assertEqual(c.context.globals['y'], 20)

    def test_mixed_separators(self):
        """Mix of newlines and semicolons."""
        c = Catnip()
        c.parse("a = 1; b = 2\nc = 3; d = 4")
        c.execute()
        self.assertEqual(c.context.globals['a'], 1)
        self.assertEqual(c.context.globals['b'], 2)
        self.assertEqual(c.context.globals['c'], 3)
        self.assertEqual(c.context.globals['d'], 4)

    def test_multiple_newlines(self):
        """Multiple consecutive newlines are ignored."""
        c = Catnip()
        c.parse("x = 10\n\n\ny = 20")
        c.execute()
        self.assertEqual(c.context.globals['x'], 10)
        self.assertEqual(c.context.globals['y'], 20)

    def test_trailing_semicolon(self):
        """Point-virgule final optionnel."""
        c = Catnip()
        c.parse("x = 10; y = 20;")
        c.execute()
        self.assertEqual(c.context.globals['x'], 10)
        self.assertEqual(c.context.globals['y'], 20)

    def test_semicolon_without_space(self):
        """Point-virgule fonctionne sans espace."""
        c = Catnip()
        c.parse("x=10;y=20")
        c.execute()
        self.assertEqual(c.context.globals['x'], 10)
        self.assertEqual(c.context.globals['y'], 20)


class TestMultilineExpressions(unittest.TestCase):
    """Multiline expression tests with parentheses."""

    def test_multiline_arithmetic(self):
        """Arithmetic expression across multiple lines."""
        c = Catnip()
        c.parse("""
x = (
    1 + 2 +
    3 + 4
)
""")
        c.execute()
        self.assertEqual(c.context.globals['x'], 10)

    def test_multiline_arithmetic_no_leading_newline(self):
        """Arithmetic expression without a newline before the first operand."""
        c = Catnip()
        c.parse("""
x = (1 + 2 +
     3 + 4)
""")
        c.execute()
        self.assertEqual(c.context.globals['x'], 10)

    def test_multiline_complex_expression(self):
        """Complex expression with multiple operators."""
        c = Catnip()
        c.parse("""
result = (
    10 * 2 +
    5 - 3 /
    2
)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], 10 * 2 + 5 - 3 / 2)

    def test_multiline_boolean_expression(self):
        """Multiline boolean expression."""
        c = Catnip()
        c.parse("""
age = 25
permis = True
peut_conduire = (
    age >= 18
    and permis == True
)
""")
        c.execute()
        self.assertEqual(c.context.globals['peut_conduire'], True)

    def test_multiline_comparison_chain(self):
        """Multiline chained comparisons."""
        c = Catnip()
        c.parse("""
x = 50
valid = (
    0 < x
    and x < 100
)
""")
        c.execute()
        self.assertEqual(c.context.globals['valid'], True)

    def test_nested_parentheses(self):
        """Nested parentheses with newlines."""
        c = Catnip()
        c.parse("""
result = (
    (1 + 2) *
    (3 + 4)
)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], 21)


class TestMultilineFunctionCalls(unittest.TestCase):
    """Tests des appels de fonction multilignes."""

    def test_multiline_function_call(self):
        """Appel de fonction avec arguments sur plusieurs lignes."""
        c = Catnip()
        c.parse("""
result = max(10,
    20,
    30,
    5)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], 30)

    def test_multiline_function_call_with_kwargs(self):
        """Multiline call with named arguments."""
        c = Catnip()
        c.parse("""
make_dict = (a, b, c) => {
    dict(("a", a), ("b", b), ("c", c))
}
result = make_dict(a=1,
    b=2,
    c=3)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], {'a': 1, 'b': 2, 'c': 3})

    def test_multiline_nested_calls(self):
        """Multiline nested calls."""
        c = Catnip()
        c.parse("""
result = max(min(10, 20),
    min(30, 40))
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], 30)

    def test_multiline_chained_calls(self):
        """Chained calls with newlines."""
        c = Catnip()
        c.parse("""
text = "hello world"
result = text.upper().replace("HELLO",
    "GOODBYE")
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], "GOODBYE WORLD")


class TestMultilineLambdas(unittest.TestCase):
    """Tests des lambdas multilignes."""

    def test_multiline_lambda_params(self):
        """Lambda with parameters across multiple lines."""
        c = Catnip()
        c.parse("""
f = (a,
    b,
    c) => {
    a + b + c
}
result = f(1, 2, 3)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], 6)

    def test_multiline_lambda_with_defaults(self):
        """Lambda with multiline default values."""
        c = Catnip()
        c.parse("""
f = (x,
    y=10,
    z=20) => {
    x + y + z
}
result = f(5)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], 35)

    def test_multiline_lambda_variadic(self):
        """Lambda variadique multiligne."""
        c = Catnip()
        c.parse("""
f = (first,
    *rest) => {
    result = list(first)
    for item in rest {
        result = result + list(item)
    }
    result
}
result = f(1, 2, 3, 4)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], [1, 2, 3, 4])


class TestMultilineControlFlow(unittest.TestCase):
    """Multiline control structure tests."""

    def test_multiline_if_condition(self):
        """Condition if multiligne."""
        c = Catnip()
        c.parse("""
x = 10
y = 20
result = None
if (
    x > 5
    and y > 15
) {
    result = True
} else {
    result = False
}
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], True)

    def test_multiline_while_condition(self):
        """Condition while multiligne."""
        c = Catnip()
        c.parse("""
x = 0
count = 0
while (
    x < 10
    and count < 5
) {
    x = x + 2
    count = count + 1
}
""")
        c.execute()
        self.assertEqual(c.context.globals['x'], 10)
        self.assertEqual(c.context.globals['count'], 5)


class TestEdgeCases(unittest.TestCase):
    """Edge case tests."""

    def test_empty_parentheses_group(self):
        """Groupe vide avec newlines."""
        c = Catnip()
        # Empty parentheses are invalid as a grouping expression
        # but still valid for lambdas
        c.parse("f = () => { 42 }; result = f()")
        c.execute()
        self.assertEqual(c.context.globals['result'], 42)

    def test_deeply_nested_multiline(self):
        """Deeply nested multiline expressions."""
        c = Catnip()
        c.parse("""
result = (
    (1 + 2) *
    (
        3 + 4 *
        (5 - 2)
    )
)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], (1 + 2) * (3 + 4 * (5 - 2)))

    def test_multiline_with_comments(self):
        """Expressions multilignes avec commentaires."""
        c = Catnip()
        c.parse("""
result = (
    1 +  # first
    2 +  # second
    3    # third
)
""")
        c.execute()
        self.assertEqual(c.context.globals['result'], 6)

    def test_single_line_in_parentheses(self):
        """Single-line expression in parentheses."""
        c = Catnip()
        c.parse("x = (1 + 2 + 3)")
        c.execute()
        self.assertEqual(c.context.globals['x'], 6)

    def test_newline_before_operator(self):
        """Newline before an operator (inside parentheses)."""
        c = Catnip()
        c.parse("""
x = (1
    + 2
    + 3)
""")
        c.execute()
        self.assertEqual(c.context.globals['x'], 6)

    def test_newline_after_operator(self):
        """Newline after an operator (inside parentheses)."""
        c = Catnip()
        c.parse("""
x = (1 +
    2 +
    3)
""")
        c.execute()
        self.assertEqual(c.context.globals['x'], 6)


if __name__ == "__main__":
    unittest.main()
