# FILE: tests/language/test_context.py
import unittest

from catnip import Scope
from catnip.context import Context, MinimalLogger


# Dummy logger used to test initialization with a custom logger.
class DummyLogger:
    def __init__(self):
        self.messages = []

    def debug(self, msg):
        self.messages.append(msg)


class TestContext(unittest.TestCase):
    def test_default_initialization(self):
        ctx = Context()
        # Validate the default logger.
        self.assertIsNotNone(ctx.logger)
        self.assertIsInstance(ctx.logger, MinimalLogger)
        # Default globals should include Python builtins.
        self.assertIsInstance(ctx.globals, dict)
        # Verify that essential builtins are present
        self.assertIn("range", ctx.globals)
        self.assertIn("len", ctx.globals)
        self.assertIn("print", ctx.globals)
        # Ensure logger and debug are exposed in globals
        self.assertIn("logger", ctx.globals)
        self.assertIn("debug", ctx.globals)
        self.assertEqual(ctx.globals["logger"], ctx.logger)
        # locals should be a Scope instance.
        self.assertIsInstance(ctx.locals, Scope)
        # Scope stack should have depth 1 (global scope only).
        self.assertEqual(ctx.locals.depth(), 1)
        # Initial result should be None.
        self.assertIsNone(ctx.result)

    def test_initialization_with_globals_and_locals(self):
        globals_dict = {"a": 1, "b": 2}
        locals_dict = {"x": 10}
        ctx = Context(globals=globals_dict, locals=locals_dict)
        # Ensure logger and debug are added even with custom globals
        self.assertIn("logger", ctx.globals)
        self.assertIn("debug", ctx.globals)
        self.assertIn("a", ctx.globals)
        self.assertIn("b", ctx.globals)
        # Assume Scope stores symbols in _symbols.
        self.assertEqual(ctx.locals._symbols, locals_dict)

    def test_initialization_with_logger(self):
        dummy_logger = DummyLogger()
        ctx = Context(logger=dummy_logger)
        self.assertEqual(ctx.logger, dummy_logger)

    def test_push_scope_with_dict(self):
        ctx = Context(locals={"initial": "value"})
        initial_depth = ctx.locals.depth()
        ctx.push_scope({"x": 42})
        # Scope uses frame depth instead of parent scope objects.
        self.assertEqual(ctx.locals.depth(), initial_depth + 1)
        self.assertEqual(ctx.locals["x"], 42)
        self.assertEqual(ctx.locals["initial"], "value")

    def test_push_scope_with_scope_instance(self):
        ctx = Context()
        # Create a Scope instance with a symbols dict.
        dummy_scope = Scope(symbols={"y": 100})
        ctx.push_scope(dummy_scope)
        self.assertEqual(ctx.locals["y"], 100)

    def test_pop_scope_success(self):
        ctx = Context()
        ctx.push_scope({"x": 42})
        # Two scopes now; pop should succeed.
        popped_scope = ctx.pop_scope()
        self.assertIsNone(popped_scope)
        # Popped frame should remove x.
        self.assertIsNone(ctx.locals.get("x"))
        # Scope stack should return to depth 1.
        self.assertEqual(ctx.locals.depth(), 1)

    def test_pop_scope_failure(self):
        ctx = Context()
        # Popping the global scope should raise.
        with self.assertRaises(Exception) as context_manager:
            ctx.pop_scope()
        self.assertEqual(str(context_manager.exception), "Cannot pop the global scope.")


class TestPureFunctions(unittest.TestCase):
    def test_pure_functions_initialized_with_builtins(self):
        """Test that pure_functions is initialized with known pure builtins."""
        ctx = Context()
        # Verify that pure_functions contains the known pure builtins
        self.assertIsInstance(ctx.pure_functions, set)
        self.assertGreater(len(ctx.pure_functions), 0)
        # Check for some specific known pure functions
        self.assertIn("len", ctx.pure_functions)
        self.assertIn("abs", ctx.pure_functions)
        self.assertIn("sum", ctx.pure_functions)
        self.assertIn("max", ctx.pure_functions)
        self.assertIn("min", ctx.pure_functions)

    def test_known_pure_functions_constant(self):
        """Test that KNOWN_PURE_FUNCTIONS contains expected builtins."""
        self.assertIn("len", Context.KNOWN_PURE_FUNCTIONS)
        self.assertIn("abs", Context.KNOWN_PURE_FUNCTIONS)
        self.assertIn("int", Context.KNOWN_PURE_FUNCTIONS)
        self.assertIn("float", Context.KNOWN_PURE_FUNCTIONS)
        self.assertIn("str", Context.KNOWN_PURE_FUNCTIONS)

    def test_pure_function_decorator(self):
        """Test that @pure decorator marks functions as pure."""
        from catnip import pure

        @pure
        def square(x):
            return x**2

        # Check that the decorator adds is_pure attribute
        self.assertTrue(hasattr(square, "is_pure"))
        self.assertTrue(square.is_pure)

    def test_broadcast_detects_pure_function_decorator(self):
        """Test that _broadcast_callable detects @pure decorator."""
        from catnip import Catnip, pure

        @pure
        def double(x):
            return x * 2

        catnip = Catnip()
        catnip.context.globals["double"] = double
        catnip.context.globals["data"] = [1, 2, 3]

        # Use broadcast to apply the pure function
        code = catnip.parse("data.[double]")
        result = catnip.execute()

        # Check that the function was detected as pure and added to context
        self.assertIn("double", catnip.context.pure_functions)
        # Verify the result is correct
        self.assertEqual(result, [2, 4, 6])

    def test_broadcast_detects_builtin_pure_function(self):
        """Test that _broadcast_callable detects known pure builtins."""
        from catnip import Catnip

        catnip = Catnip()
        catnip.context.globals["data"] = [1, -2, 3]

        # Use broadcast with a known pure builtin
        code = catnip.parse("data.[abs]")
        result = catnip.execute()

        # Check that abs is in pure_functions
        self.assertIn("abs", catnip.context.pure_functions)
        # Verify the result is correct
        self.assertEqual(result, [1, 2, 3])

    def test_broadcast_tracks_custom_pure_function(self):
        """Test that custom pure functions are tracked after broadcast."""
        from catnip import Catnip, pure

        @pure
        def increment(x):
            return x + 1

        catnip = Catnip()
        catnip.context.globals["increment"] = increment
        catnip.context.globals["data"] = [10, 20, 30]

        # Initially, increment should not be in pure_functions
        self.assertNotIn("increment", catnip.context.pure_functions)

        # Use broadcast to apply the function
        code = catnip.parse("data.[increment]")
        result = catnip.execute()

        # Now increment should be tracked as pure
        self.assertIn("increment", catnip.context.pure_functions)
        self.assertEqual(result, [11, 21, 31])


if __name__ == "__main__":
    unittest.main()
