# FILE: tests/language/test_nd_recursion.py
"""
Tests for ND-recursion operators: @@, @>, @[]

Phase 3 implementation: sequential execution.
"""

import pytest

from catnip import Catnip
from catnip.exc import CatnipTypeError


def exec_catnip(code):
    """Helper to execute Catnip code."""
    c = Catnip()
    c.parse(code)
    return c.execute()


def parse_ir(code):
    """Helper to get IR at level 1."""
    c = Catnip()
    return c.parse(code, 1)


class TestNDEmptyTopos:
    """Tests for @[] literal."""

    def test_empty_topos_literal(self):
        """@[] returns the empty topos singleton."""
        result = exec_catnip("@[]")
        assert str(result) == "@[]"

    def test_empty_topos_is_falsy(self):
        """@[] is falsy in boolean context."""
        result = exec_catnip("if @[] { 1 } else { 2 }")
        assert result == 2

    def test_empty_topos_equality(self):
        """@[] equals itself."""
        result = exec_catnip("@[] == @[]")
        assert result is True

    def test_empty_topos_len(self):
        """@[] has length 0."""
        result = exec_catnip("len(@[])")
        assert result == 0


class TestNDRecursion:
    """Tests for @@ operator."""

    def test_nd_recursion_countdown(self):
        """@@(seed, lambda) executes recursive computation."""
        result = exec_catnip("""
            @@(5, (v, recur) => {
                if v > 0 { recur(v - 1) }
                else { v }
            })
        """)
        assert result == 0

    def test_nd_recursion_factorial(self):
        """ND-recursion for factorial."""
        result = exec_catnip("""
            @@(5, (n, recur) => {
                if n <= 1 { 1 }
                else { n * recur(n - 1) }
            })
        """)
        assert result == 120

    def test_nd_recursion_declaration_form(self):
        """@@ lambda creates a reusable ND-recursive function."""
        result = exec_catnip("""
            countdown = @@ (n, recur) => {
                if n > 0 { recur(n - 1) }
                else { "done" }
            }
            countdown
        """)
        # Declaration form returns the lambda
        assert callable(result)


class TestNDMap:
    """Tests for @> operator."""

    def test_nd_map_lift_form(self):
        """@> f lifts a function to ND context."""
        result = exec_catnip("f = @> abs; f(-5)")
        assert result == 5

    def test_nd_map_applicative_form(self):
        """@>(data, f) applies f in ND context."""
        result = exec_catnip("@>(list(-1, -2, -3), abs)")
        assert result == [1, 2, 3]


class TestBroadcastND:
    """Tests for broadcast ND forms: data.[@@ lambda] and data.[@> f]"""

    def test_broadcast_nd_map(self):
        """data.[@> f] maps f over each element."""
        result = exec_catnip("list(-1, -2, 3).[@> abs]")
        assert result == [1, 2, 3]

    def test_broadcast_nd_map_with_lambda(self):
        """data.[@> lambda] maps lambda over elements."""
        result = exec_catnip("list(1, 2, 3).[@> (x) => { x * 2 }]")
        assert result == [2, 4, 6]

    def test_broadcast_nd_recursion(self):
        """data.[@@ lambda] applies ND-recursion to each element."""
        result = exec_catnip("""
            list(5, 3, 7).[@@ (n, recur) => {
                if n <= 1 { 1 }
                else { n * recur(n - 1) }
            }]
        """)
        assert result == [120, 6, 5040]  # factorials of 5, 3, 7

    def test_broadcast_nd_recursion_countdown(self):
        """data.[@@ lambda] countdown on each element."""
        result = exec_catnip("""
            list(3, 5, 2).[@@ (v, recur) => {
                if v > 0 { recur(v - 1) }
                else { v }
            }]
        """)
        assert result == [0, 0, 0]

    def test_broadcast_nd_map_preserves_tuple(self):
        """data.[@> f] preserves tuple type."""
        result = exec_catnip("tuple(-1, -2, 3).[@> abs]")
        assert result == (1, 2, 3)
        assert isinstance(result, tuple)

    def test_broadcast_nd_recursion_preserves_tuple(self):
        """data.[@@ lambda] preserves tuple type."""
        result = exec_catnip("""
            tuple(3, 2, 4).[@@ (n, recur) => {
                if n <= 1 { 1 }
                else { n * recur(n - 1) }
            }]
        """)
        assert result == (6, 2, 24)
        assert isinstance(result, tuple)


class TestNDEdgeCases:
    """Edge cases and error handling."""

    def test_nd_map_on_empty_list(self):
        """ND-map on empty list returns empty list."""
        result = exec_catnip("list().[@> abs]")
        assert result == []

    def test_nd_map_on_scalar(self):
        """ND-map on scalar applies function directly."""
        result = exec_catnip("@>(-5, abs)")
        assert result == 5


class TestNDArityValidation:
    """Signature validation for ND lambdas."""

    def test_nd_recursion_wrong_arity_1_param(self):
        """@@ with 1-param lambda raises TypeError."""
        with pytest.raises(CatnipTypeError, match="2 parameters"):
            exec_catnip("@@(5, (x) => { x })")

    def test_nd_recursion_wrong_arity_3_params(self):
        """@@ with 3-param lambda raises TypeError."""
        with pytest.raises(CatnipTypeError, match="2 parameters"):
            exec_catnip("@@(5, (a, b, c) => { a })")

    def test_nd_map_wrong_arity_2_params(self):
        """@> with 2-param lambda raises TypeError."""
        with pytest.raises(CatnipTypeError, match="1 parameters"):
            exec_catnip("@>(list(1, 2), (a, b) => { a })")

    def test_nd_recursion_correct_arity(self):
        """@@ with 2-param lambda works fine."""
        result = exec_catnip("@@(3, (n, recur) => { if n <= 0 { 0 } else { recur(n - 1) } })")
        assert result == 0

    def test_nd_map_correct_arity(self):
        """@> with 1-param lambda works fine."""
        result = exec_catnip("@>(list(1, 2, 3), (x) => { x * 10 })")
        assert result == [10, 20, 30]

    def test_nd_map_builtin_no_validation(self):
        """Python builtins (no .params) skip validation."""
        result = exec_catnip("@>(list(-1, -2), abs)")
        assert result == [1, 2]

    def test_broadcast_nd_recursion_wrong_arity(self):
        """Broadcast @@ with wrong arity raises TypeError."""
        with pytest.raises(CatnipTypeError, match="2 parameters"):
            exec_catnip("list(1, 2).[@@ (x) => { x }]")

    def test_broadcast_nd_map_wrong_arity(self):
        """Broadcast @> with wrong arity raises TypeError."""
        with pytest.raises(CatnipTypeError, match="1 parameters"):
            exec_catnip("list(1, 2).[@> (a, b) => { a }]")


class TestNDTransformation:
    """Tests for correct IR transformation."""

    def test_nd_empty_topos_opcode(self):
        """@[] transforms to ND_EMPTY_TOPOS opcode."""
        ir = parse_ir("@[]")
        assert len(ir) == 1
        # OpCode.ND_EMPTY_TOPOS = 56
        assert ir[0].ident == 56

    def test_nd_recursion_opcode(self):
        """@@ transforms to ND_RECURSION opcode."""
        ir = parse_ir("@@(0, (v, r) => { v })")
        assert len(ir) == 1
        # OpCode.ND_RECURSION = 54
        assert ir[0].ident == 54

    def test_nd_map_opcode(self):
        """@> transforms to ND_MAP opcode."""
        ir = parse_ir("@> abs")
        assert len(ir) == 1
        # OpCode.ND_MAP = 55
        assert ir[0].ident == 55
