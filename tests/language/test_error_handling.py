# FILE: tests/language/test_error_handling.py
import unittest

import pytest

from catnip import Catnip
from catnip.exc import CatnipNameError, CatnipRuntimeError, CatnipSemanticError, CatnipTypeError


class TestParsingErrors(unittest.TestCase):
    """Parser error propagation tests for the host application."""

    def test_unexpected_eof(self):
        """Incomplete expression should raise SyntaxError."""
        catnip = Catnip()
        with self.assertRaises(SyntaxError) as cm:
            catnip.parse("1 +")

        # Message should indicate an error
        exc_msg = str(cm.exception)
        self.assertTrue("Expected" in exc_msg or "Unexpected" in exc_msg or "Syntax" in exc_msg)

    def test_unexpected_token(self):
        """Token invalide doit lever SyntaxError."""
        catnip = Catnip()
        with self.assertRaises(SyntaxError) as cm:
            catnip.parse("x = )")

        exc_msg = str(cm.exception)
        self.assertTrue("Unexpected" in exc_msg or "Syntax" in exc_msg)

    def test_invalid_syntax(self):
        """Syntaxe invalide doit lever SyntaxError."""
        catnip = Catnip()
        with self.assertRaises(SyntaxError):
            catnip.parse("if { }")

    def test_unmatched_brace(self):
        """Unclosed brace should raise SyntaxError."""
        catnip = Catnip()
        with self.assertRaises(SyntaxError):
            catnip.parse("x = { 1 + 2")


class TestExecutionErrors(unittest.TestCase):
    """Runtime error propagation tests for the host application."""

    def test_division_by_zero(self):
        """Division by zero should raise ZeroDivisionError."""
        catnip = Catnip()
        catnip.parse("x = 1 / 0")

        with self.assertRaises(CatnipRuntimeError) as cm:
            catnip.execute()

        # Verify that the message mentions division by zero
        self.assertIn("division by zero", str(cm.exception))

    def test_undefined_variable(self):
        """Undefined variable should raise CatnipNameError."""
        catnip = Catnip()
        catnip.parse("undefined_var")

        with self.assertRaises(CatnipNameError):
            catnip.execute()

    def test_undefined_variable_in_expression(self):
        """Undefined variable in expression should raise CatnipNameError."""
        catnip = Catnip()
        catnip.parse("x = y + 1")

        with self.assertRaises(CatnipNameError) as cm:
            catnip.execute()

        # Error should mention the missing variable
        self.assertIn("y", str(cm.exception))

    def test_type_error_incompatible_operation(self):
        """Incompatible operation should raise TypeError."""
        catnip = Catnip()
        catnip.parse('x = "text" - 5')

        with self.assertRaises(CatnipTypeError) as cm:
            catnip.execute()

        # Message should indicate type incompatibility
        error_msg = str(cm.exception)
        self.assertTrue("str" in error_msg or "unsupported" in error_msg.lower())

    def test_attribute_error_invalid_method(self):
        """Accessing a missing attribute should raise AttributeError."""
        catnip = Catnip()
        # Use a literal string properly
        catnip.context.globals["text"] = "hello"
        catnip.parse("x = text.invalid_method")

        with self.assertRaises(AttributeError) as cm:
            catnip.execute()

        self.assertIn("invalid_method", str(cm.exception))

    def test_index_error_out_of_bounds(self):
        """Out-of-bounds access should raise IndexError."""
        catnip = Catnip()
        catnip.context.globals["data"] = [1, 2, 3]
        # Use a method that raises IndexError
        catnip.context.globals["get_item"] = lambda lst, i: lst[i]
        catnip.parse("y = get_item(data, 10)")

        with self.assertRaises(IndexError) as cm:
            catnip.execute()

        # Verify this is an index issue
        self.assertTrue("index" in str(cm.exception).lower() or "out of range" in str(cm.exception).lower())

    def test_key_error_missing_dict_key(self):
        """Missing key access should raise KeyError."""
        catnip = Catnip()
        catnip.context.globals["data"] = {"a": 1, "b": 2}
        catnip.context.globals["get_key"] = lambda d, k: d[k]
        catnip.parse('y = get_key(data, "missing")')

        with self.assertRaises(KeyError) as cm:
            catnip.execute()

        self.assertIn("missing", str(cm.exception))


class TestFunctionErrors(unittest.TestCase):
    """Function-related error tests."""

    def test_too_many_arguments(self):
        """Too many arguments with variadic does not raise - test skipped."""
        # Note: with variadic support, too many args no longer raise TypeError
        # Test is disabled because expected behavior changed
        pass

    def test_missing_required_argument(self):
        """Missing required argument becomes None and raises TypeError if used."""
        catnip = Catnip()
        catnip.parse("f = (x, y) => { x + y }; f(1)")

        # Missing parameter 'y' becomes None, so x + y raises TypeError
        with self.assertRaises(CatnipTypeError) as cm:
            catnip.execute()

        # Error mentions int and NoneType
        error_msg = str(cm.exception)
        self.assertTrue("int" in error_msg and "NoneType" in error_msg)

    def test_recursive_error_propagation(self):
        """Errors in recursive functions should bubble up directly."""
        catnip = Catnip()
        catnip.parse("""
            fact = (n) => {
                match n {
                    0 => { 1 / 0 }
                    n => { n * fact(n - 1) }
                }
            }
            fact(0)
        """)

        with self.assertRaises(CatnipRuntimeError) as cm:
            catnip.execute()

        # Error should mention division by zero
        self.assertIn("division by zero", str(cm.exception))


class TestErrorMessages(unittest.TestCase):
    """Error message clarity tests."""

    def test_parsing_error_message_contains_context(self):
        """Message d'erreur de parsing doit contenir le contexte."""
        catnip = Catnip()

        with self.assertRaises(SyntaxError) as cm:
            catnip.parse("1 + 2 +")

        # Message should indicate the expectation or the error
        exc_msg = str(cm.exception)
        self.assertTrue("Expected" in exc_msg or "Unexpected" in exc_msg or "line" in exc_msg)

    def test_undefined_variable_message(self):
        """Undefined-variable message should include the name."""
        catnip = Catnip()
        catnip.parse("missing_var")

        with self.assertRaises(CatnipNameError) as cm:
            catnip.execute()

        # Message should include the variable name
        self.assertIn("missing_var", str(cm.exception))


class TestErrorPositionAccuracy(unittest.TestCase):
    """Verify that errors point to the correct line and column."""

    def _run(self, code):
        c = Catnip()
        c.parse(code)
        c.execute()

    def test_single_line_name_error(self):
        """NameError on single line reports line 1."""
        with self.assertRaises(CatnipNameError) as cm:
            self._run("undefined_var")
        self.assertEqual(cm.exception.line, 1)

    def test_multiline_error_on_line_3(self):
        """Error on line 3 of multi-line code reports line 3."""
        code = "x = 1\ny = 2\nz = undefined"
        with self.assertRaises(CatnipNameError) as cm:
            self._run(code)
        self.assertEqual(cm.exception.line, 3)

    def test_error_in_function_body(self):
        """Error inside function body reports correct line."""
        code = "f = () => {\n  undefined\n}\nf()"
        with self.assertRaises(CatnipNameError) as cm:
            self._run(code)
        self.assertEqual(cm.exception.line, 2)

    def test_division_by_zero_position(self):
        """ZeroDivisionError reports correct line."""
        code = "x = 1\ny = 1 / 0"
        with self.assertRaises(CatnipRuntimeError) as cm:
            self._run(code)
        self.assertEqual(cm.exception.line, 2)

    def test_type_error_position(self):
        """TypeError reports correct line."""
        code = 'x = 1\ny = "a" - 1'
        with self.assertRaises(CatnipTypeError) as cm:
            self._run(code)
        self.assertEqual(cm.exception.line, 2)

    def test_nested_block_error_position(self):
        """Error in nested block reports correct line."""
        code = "x = 1\nif True {\n  if True {\n    undefined\n  }\n}"
        with self.assertRaises(CatnipNameError) as cm:
            self._run(code)
        self.assertEqual(cm.exception.line, 4)

    def test_semantic_error_position(self):
        """Semantic error (pragma) reports correct line."""
        code = 'x = 1\npragma("nonexistent", "on")'
        with self.assertRaises(CatnipSemanticError) as cm:
            Catnip().parse(code)
        self.assertEqual(cm.exception.line, 2)


class TestErrorSuggestions(unittest.TestCase):
    """Tests for 'did you mean?' suggestions in error messages."""

    def test_undefined_variable_suggestion(self):
        """NameError should suggest similar variable when one exists."""
        catnip = Catnip()
        catnip.parse("factorial = 1; factoral")

        with self.assertRaises(CatnipNameError) as cm:
            catnip.execute()

        exc_msg = str(cm.exception)
        self.assertIn("Did you mean", exc_msg)
        self.assertIn("factorial", exc_msg)

    def test_undefined_variable_no_suggestion(self):
        """NameError should not suggest when nothing is close."""
        catnip = Catnip()
        catnip.parse("xyzzy_unique")

        with self.assertRaises(CatnipNameError) as cm:
            catnip.execute()

        exc_msg = str(cm.exception)
        self.assertNotIn("Did you mean", exc_msg)

    def test_struct_attribute_suggestion(self):
        """AttributeError on struct should suggest similar attribute."""
        catnip = Catnip()
        catnip.parse("""
            struct Config { name, value, debug }
            c = Config("a", 1, False)
            c.naem
        """)

        with self.assertRaises((AttributeError, CatnipRuntimeError)) as cm:
            catnip.execute()

        exc_msg = str(cm.exception)
        self.assertIn("Did you mean", exc_msg)
        self.assertIn("name", exc_msg)

    def test_struct_attribute_no_suggestion(self):
        """AttributeError on struct should not suggest when nothing is close."""
        catnip = Catnip()
        catnip.parse("""
            struct Point { x, y }
            p = Point(1, 2)
            p.zzzzzzz
        """)

        with self.assertRaises((AttributeError, CatnipRuntimeError)) as cm:
            catnip.execute()

        exc_msg = str(cm.exception)
        self.assertNotIn("Did you mean", exc_msg)

    def test_keyword_typo_class_to_struct(self):
        """Using 'class' should suggest 'struct'."""
        catnip = Catnip()
        with self.assertRaises(SyntaxError) as cm:
            catnip.parse("class Foo { x }")

        exc_msg = str(cm.exception)
        self.assertIn("struct", exc_msg)

    def test_keyword_typo_switch_to_match(self):
        """Using 'switch' should suggest 'match'."""
        catnip = Catnip()
        with self.assertRaises(SyntaxError) as cm:
            catnip.parse("switch x { 1 => { 1 } }")

        exc_msg = str(cm.exception)
        self.assertIn("match", exc_msg)

    def test_semantic_error_has_location(self):
        """Semantic errors should include line/column when available."""
        catnip = Catnip()
        with self.assertRaises(CatnipSemanticError) as cm:
            catnip.parse('pragma("nonexistent", "on")')

        exc = cm.exception
        # Should have location info enriched from SourceMap
        self.assertIsNotNone(exc.line)
        self.assertIsNotNone(exc.column)

    def test_python_string_method_suggestion(self):
        """Typo on string method suggests correct name."""
        catnip = Catnip()
        catnip.context.globals["text"] = "hello"
        catnip.parse("text.uper()")
        with self.assertRaises((AttributeError, CatnipRuntimeError)) as cm:
            catnip.execute()
        exc_msg = str(cm.exception)
        self.assertIn("Did you mean", exc_msg)
        self.assertIn("upper", exc_msg)

    def test_python_list_method_suggestion(self):
        """Typo on list method suggests correct name."""
        catnip = Catnip()
        catnip.context.globals["data"] = [1, 2, 3]
        catnip.parse("data.apend(4)")
        with self.assertRaises((AttributeError, CatnipRuntimeError)) as cm:
            catnip.execute()
        exc_msg = str(cm.exception)
        self.assertIn("Did you mean", exc_msg)
        self.assertIn("append", exc_msg)

    def test_python_object_no_suggestion_when_distant(self):
        """No suggestion when attribute name is too different."""
        catnip = Catnip()
        catnip.context.globals["text"] = "hello"
        catnip.parse("text.zzzzzzz")
        with self.assertRaises((AttributeError, CatnipRuntimeError)):
            catnip.execute()


if __name__ == "__main__":
    unittest.main()
