# FILE: tests/language/test_broadcast_iterables.py
"""
Tests for broadcasting over Python iterables (range, filter, map, etc.)
"""

import pytest

from catnip import Catnip


def exec_catnip(code):
    """Helper to execute Catnip code"""
    c = Catnip()
    c.parse(code)
    return c.execute()


class TestBroadcastOnRange:
    """Tests pour le broadcast sur range()"""

    def test_range_with_multiply(self):
        """range(5).[* 2] multiplies each element by 2"""
        result = exec_catnip("range(5).[* 2]")
        assert result == [0, 2, 4, 6, 8]

    def test_range_with_add(self):
        """range(3).[+ 10] adds 10 to each element"""
        result = exec_catnip("range(3).[+ 10]")
        assert result == [10, 11, 12]

    def test_range_with_filter(self):
        """range(10).[if > 5] filters elements > 5"""
        result = exec_catnip("range(10).[if > 5]")
        assert result == [6, 7, 8, 9]

    def test_range_chaining(self):
        """range(5).[* 2].[+ 1] chains multiple operations"""
        result = exec_catnip("range(5).[* 2].[+ 1]")
        assert result == [1, 3, 5, 7, 9]

    def test_range_with_start_stop(self):
        """range(5, 10).[* 2] fonctionne avec start et stop"""
        result = exec_catnip("range(5, 10).[* 2]")
        assert result == [10, 12, 14, 16, 18]

    def test_range_with_step(self):
        """range(0, 10, 2).[+ 1] fonctionne avec step"""
        result = exec_catnip("range(0, 10, 2).[+ 1]")
        assert result == [1, 3, 5, 7, 9]


class TestBroadcastOnFilter:
    """Tests pour le broadcast sur filter()"""

    def test_filter_with_broadcast(self):
        """filter().[* 10] multiplies filtered elements"""
        code = "filter((x) => { x > 2 }, list(1, 2, 3, 4, 5)).[* 10]"
        result = exec_catnip(code)
        assert result == [30, 40, 50]

    def test_filter_then_filter(self):
        """Chaining filter then broadcast filter"""
        code = "filter((x) => { x > 2 }, list(1, 2, 3, 4, 5, 6, 7)).[if < 6]"
        result = exec_catnip(code)
        assert result == [3, 4, 5]

    def test_filter_chaining_multiple(self):
        """Multiple chaining on filter()"""
        code = "filter((x) => { x > 0 }, list(-2, -1, 0, 1, 2, 3)).[* 2].[+ 1]"
        result = exec_catnip(code)
        assert result == [3, 5, 7]


class TestBroadcastOnMap:
    """Tests pour le broadcast sur map()"""

    def test_map_with_broadcast(self):
        """map().[+ 1] adds 1 to mapped elements"""
        code = "map((x) => { x * 2 }, list(1, 2, 3)).[+ 1]"
        result = exec_catnip(code)
        assert result == [3, 5, 7]

    def test_map_then_multiply(self):
        """map() puis multiplication"""
        code = "map((x) => { x + 10 }, range(3)).[* 2]"
        result = exec_catnip(code)
        assert result == [20, 22, 24]


class TestBroadcastOnZip:
    """Tests pour le broadcast sur zip()"""

    def test_zip_iteration_in_for(self):
        """zip() fonctionne dans une boucle for"""
        code = """
        count = 0
        for pair in zip(list(1, 2, 3), list(10, 20, 30)) {
            count = count + 1
        }
        count
        """
        result = exec_catnip(code)
        assert result == 3


class TestBroadcastOnEnumerate:
    """Tests pour le broadcast sur enumerate()"""

    def test_enumerate_iteration_in_for(self):
        """enumerate() fonctionne dans une boucle for"""
        code = """
        count = 0
        for item in enumerate(list("a", "b", "c")) {
            count = count + 1
        }
        count
        """
        result = exec_catnip(code)
        assert result == 3


class TestBroadcastOnSorted:
    """Tests pour le broadcast sur sorted()"""

    def test_sorted_with_broadcast(self):
        """sorted().[* 2] multiplies sorted elements"""
        result = exec_catnip("sorted(list(5, 2, 8, 1, 9)).[* 2]")
        assert result == [2, 4, 10, 16, 18]

    def test_sorted_with_filter(self):
        """sorted() puis filter"""
        result = exec_catnip("sorted(list(5, 2, 8, 1, 9)).[if > 5]")
        assert result == [8, 9]


class TestBroadcastIterableEdgeCases:
    """Edge case tests for iterables"""

    def test_empty_range(self):
        """range(0) donne une liste vide"""
        result = exec_catnip("range(0).[* 2]")
        assert result == []

    def test_range_complex_chaining(self):
        """Complex chaining over range"""
        result = exec_catnip("range(10).[* 2].[if > 10].[+ 1]")
        assert result == [13, 15, 17, 19]

    def test_filter_to_empty(self):
        """filter qui ne retourne rien"""
        code = "filter((x) => { x > 100 }, range(10)).[* 2]"
        result = exec_catnip(code)
        assert result == []


class TestBroadcastOnVariable:
    """Broadcast over iterables via variables (regression)"""

    def test_range_via_variable(self):
        """range stored in a variable then broadcast"""
        code = "r = range(5); r.[* 2]"
        result = exec_catnip(code)
        assert result == [0, 2, 4, 6, 8]

    def test_filter_via_variable(self):
        """filter stored in a variable then broadcast"""
        code = "f = filter((x) => { x > 2 }, list(1, 2, 3, 4, 5)); f.[* 10]"
        result = exec_catnip(code)
        assert result == [30, 40, 50]
