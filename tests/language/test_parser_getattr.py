# FILE: tests/language/test_parser_getattr.py
import unittest

from catnip.nodes import Ref
from catnip.parser import Parser
from catnip.transformer import IR, Call, OpCode

parser = Parser()
parse = parser.parse


class TestParserAttributeAccess(unittest.TestCase):
    def test_simple_getattr(self):
        # For "a.b", expect an AST like: Mi('getattr', (Ref("a"), "b"))
        ast = parse("a.b")
        self.assertIsInstance(ast, list)
        self.assertEqual(len(ast), 1)
        node = ast[0]
        self.assertIsInstance(node, IR)
        self.assertEqual(node.ident, OpCode.GETATTR)
        # Arguments should be: (Ref("a"), "b")
        self.assertEqual(len(node.args), 2)
        # First argument should be a reference to "a".
        self.assertIsInstance(node.args[0], Ref)
        self.assertEqual(node.args[0], Ref("a"))
        # Second argument should be the attribute name "b".
        self.assertEqual(node.args[1], "b")

    def test_chained_getattr(self):
        # For "a.b.c", expect a nested structure:
        # Mi('getattr', (Mi('getattr', (Ref("a"), "b")), "c"))
        ast = parse("a.b.c")
        self.assertIsInstance(ast, list)
        self.assertEqual(len(ast), 1)
        node = ast[0]
        self.assertIsInstance(node, IR)
        self.assertEqual(node.ident, OpCode.GETATTR)
        self.assertEqual(len(node.args), 2)
        # First argument should be a GETATTR IR for "a.b"
        inner = node.args[0]
        self.assertIsInstance(inner, IR)
        self.assertEqual(inner.ident, OpCode.GETATTR)
        self.assertEqual(len(inner.args), 2)
        self.assertIsInstance(inner.args[0], Ref)
        self.assertEqual(inner.args[0], Ref("a"))
        self.assertEqual(inner.args[1], "b")
        # Second argument should be "c"
        self.assertEqual(node.args[1], "c")

    def test_call_then_getattr(self):
        # For "a.b().c", expect a call followed by attribute access.
        # Result should look like:
        # Mi('getattr', (Call(... representing a.b(), ...), "c"))
        ast = parse("a.b().c")
        self.assertIsInstance(ast, list)
        self.assertEqual(len(ast), 1)
        node = ast[0]
        self.assertIsInstance(node, IR)
        self.assertEqual(node.ident, OpCode.GETATTR)
        self.assertEqual(len(node.args), 2)
        # First element should be a Call representing "a.b()"
        call_node = node.args[0]
        self.assertIsInstance(call_node, Call)
        # Second argument should be "c"
        self.assertEqual(node.args[1], "c")


if __name__ == "__main__":
    unittest.main()
