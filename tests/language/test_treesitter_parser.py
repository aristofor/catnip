# FILE: tests/language/test_treesitter_parser.py
"""
Tests for the Tree-sitter parser (integration tests).

Note: Basic parser tests (arithmetic, comparisons, literals, patterns, etc.)
are now validated in Rust tests (catnip_rs/src/parser/*/tests.rs) which run ~5x faster.
This file focuses on integration tests and edge cases not covered by Rust.

Rust test coverage:
- operators.rs: arithmetic, comparisons, logical, bitwise (~20 tests)
- control_flow.rs: if/elif/else, while, for, blocks (~17 tests)
- patterns.rs: pattern matching, wildcards, OR patterns (~24 tests)
- literals.rs: integers, floats, strings, booleans, None (~62 tests)
- remaining.rs: collections, functions, lambdas, calls (~30 tests)
"""

from catnip import Catnip


def make_catnip():
    """Create Catnip instance (tree-sitter parser is now integrated in Rust)."""
    return Catnip()


class TestArithmetic:
    """Arithmetic integration tests (basic ops tested in Rust)."""

    def test_complex_expression(self):
        """Complex expression with precedence."""
        cat = make_catnip()
        cat.parse("2 + 3 * 4 - 1")
        assert cat.execute() == 13

    def test_parentheses(self):
        """Parentheses override precedence."""
        cat = make_catnip()
        cat.parse("(2 + 3) * 4")
        assert cat.execute() == 20


class TestComparisons:
    """Comparison chaining (basic ops tested in Rust)."""

    def test_chained_less_than(self):
        """Chained comparison evaluation."""
        cat = make_catnip()
        cat.parse("1 < 2 < 3")
        assert cat.execute() is True

    def test_chained_less_than_short_circuit(self):
        """Chained comparison short-circuits correctly."""
        cat = make_catnip()
        cat.parse("3 < 2 < 4")
        assert cat.execute() is False


class TestVariables:
    """Variable assignment and access."""

    def test_simple_assignment(self):
        cat = make_catnip()
        cat.parse("x = 42; x")
        assert cat.execute() == 42

    def test_multiple_assignments(self):
        cat = make_catnip()
        cat.parse("a = 1; b = 2; c = 3; a + b + c")
        assert cat.execute() == 6

    def test_reassignment(self):
        cat = make_catnip()
        cat.parse("x = 10; x = 20; x")
        assert cat.execute() == 20


class TestConditionals:
    """If/elif/else integration (basic cases tested in Rust)."""

    def test_if_elif_else(self):
        """Multiple elif branches with variable."""
        cat = make_catnip()
        cat.parse("x = 2; if x == 1 { 10 } elif x == 2 { 20 } else { 30 }")
        assert cat.execute() == 20


class TestLoops:
    """Loop integration (basic while/for tested in Rust)."""

    def test_while_loop_accumulation(self):
        """While loop with accumulator."""
        cat = make_catnip()
        cat.parse("i = 0; s = 0; while i < 5 { s = s + i; i = i + 1 }; s")
        assert cat.execute() == 10

    def test_for_loop_accumulation(self):
        """For loop with accumulator."""
        cat = make_catnip()
        cat.parse("s = 0; for x in list(1, 2, 3, 4, 5) { s = s + x }; s")
        assert cat.execute() == 15


class TestFunctions:
    """Lambda functions and calls (simple lambdas tested in Rust)."""

    def test_recursive_function(self):
        """Recursive factorial function."""
        cat = make_catnip()
        cat.parse("fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }; fact(5)")
        assert cat.execute() == 120

    def test_closure(self):
        """Closure capturing outer variable."""
        cat = make_catnip()
        cat.parse("make_adder = (x) => { (y) => { x + y } }; add5 = make_adder(5); add5(10)")
        assert cat.execute() == 15


class TestControlFlow:
    """Break and continue."""

    def test_break(self):
        cat = make_catnip()
        cat.parse("i = 0; while True { i = i + 1; if i >= 5 { break } }; i")
        assert cat.execute() == 5

    def test_continue(self):
        cat = make_catnip()
        cat.parse("s = 0; for x in list(1, 2, 3, 4, 5) { if x == 3 { continue }; s = s + x }; s")
        assert cat.execute() == 12  # 1 + 2 + 4 + 5 = 12

    def test_return(self):
        cat = make_catnip()
        cat.parse("f = () => { return 42; 100 }; f()")
        assert cat.execute() == 42


class TestFStrings:
    """Format strings."""

    def test_fstring_simple(self):
        cat = make_catnip()
        cat.parse('x = 42; f"value: {x}"')
        assert cat.execute() == "value: 42"

    def test_fstring_expression(self):
        cat = make_catnip()
        cat.parse('f"result: {2 + 3 * 4}"')
        assert cat.execute() == "result: 14"


class TestChaining:
    """Method chaining and attribute access."""

    def test_method_call(self):
        cat = make_catnip()
        cat.parse('"hello".upper()')
        assert cat.execute() == "HELLO"

    def test_attribute_access(self):
        cat = make_catnip()
        cat.parse("list(1, 2, 3).__len__()")
        assert cat.execute() == 3

    def test_indexing(self):
        cat = make_catnip()
        cat.parse("list(10, 20, 30)[1]")
        assert cat.execute() == 20

    def test_chained_methods(self):
        cat = make_catnip()
        cat.parse('"-".join("hello world".split(" "))')
        assert cat.execute() == "hello-world"


class TestDefaultParams:
    """Functions with default parameters."""

    def test_default_param(self):
        cat = make_catnip()
        cat.parse("greet = (name, msg='Hello') => { f'{msg}, {name}!' }; greet('World')")
        assert cat.execute() == "Hello, World!"

    def test_override_default(self):
        cat = make_catnip()
        cat.parse("greet = (name, msg='Hello') => { f'{msg}, {name}!' }; greet('World', 'Hi')")
        assert cat.execute() == "Hi, World!"

    def test_default_param_simple(self):
        """Test default params without f-strings."""
        cat = make_catnip()
        cat.parse("add = (a, b=10) => { a + b }; add(5)")
        assert cat.execute() == 15

    def test_override_default_simple(self):
        """Test overriding default without f-strings."""
        cat = make_catnip()
        cat.parse("add = (a, b=10) => { a + b }; add(5, 20)")
        assert cat.execute() == 25
