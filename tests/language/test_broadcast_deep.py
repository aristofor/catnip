# FILE: tests/language/test_broadcast_deep.py
"""
Tests pour le broadcast deep (récursion automatique dans les structures imbriquées).
"""

from catnip import Catnip


def exec_catnip(code):
    """Helper to execute Catnip code"""
    c = Catnip()
    c.parse(code)
    return c.execute()


class TestBroadcastDeepMap:
    """Tests pour le broadcast deep sur structures imbriquées"""

    def test_nested_multiply(self):
        result = exec_catnip("list(list(1, 2), list(3, 4)).[* 2]")
        assert result == [[2, 4], [6, 8]]

    def test_nested_add(self):
        result = exec_catnip("list(list(1, 2), list(3, 4)).[+ 10]")
        assert result == [[11, 12], [13, 14]]

    def test_mixed_depth(self):
        result = exec_catnip("list(1, list(2, 3)).[+ 10]")
        assert result == [11, [12, 13]]

    def test_three_levels(self):
        result = exec_catnip("list(list(list(1))).[* 3]")
        assert result == [[[3]]]

    def test_empty_nested(self):
        result = exec_catnip("list(list(), list(1)).[* 2]")
        assert result == [[], [2]]

    def test_tuple_nested(self):
        result = exec_catnip("tuple(tuple(1, 2), tuple(3, 4)).[* 2]")
        assert result == ((2, 4), (6, 8))

    def test_list_of_tuples(self):
        result = exec_catnip("list(tuple(1, 2), tuple(3, 4)).[* 2]")
        assert result == [(2, 4), (6, 8)]

    def test_lambda_deep(self):
        result = exec_catnip("list(list(1, 2)).[(x) => { x + 100 }]")
        assert result == [[101, 102]]

    def test_abs_deep(self):
        result = exec_catnip("list(list(-1, -2), list(3, -4)).[~> abs]")
        assert result == [[1, 2], [3, 4]]

    def test_flat_unchanged(self):
        """Le broadcast flat reste identique"""
        result = exec_catnip("list(1, 2, 3).[* 2]")
        assert result == [2, 4, 6]

    def test_scalar_unchanged(self):
        """Le broadcast scalar reste identique"""
        result = exec_catnip("5.[* 2]")
        assert result == 10

    def test_explicit_composition_works(self):
        """La composition explicite .[.[...]] reste valide"""
        result = exec_catnip("list(list(1, 2)).[.[* 2]]")
        assert result == [[2, 4]]

    def test_struct_as_leaf(self):
        """Les structs sont traités comme des feuilles"""
        code = """
        struct Point { x, y }
        p = Point(1, 2)
        list(list(p)).[(p) => { p.x }]
        """
        result = exec_catnip(code)
        assert result == [[1]]
