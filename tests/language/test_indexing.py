# FILE: tests/language/test_indexing.py
"""
Tests for indexation avec la syntaxe [].

Tests pour la nouvelle syntaxe obj[index] qui remplace obj.__getitem__(index).
"""

import pytest

from catnip import Catnip


class TestListIndexing:
    """Tests for l'indexation of lists."""

    def test_list_index_simple(self):
        """Verify l'indexation basique d'une liste."""
        catnip = Catnip()
        code = """
        lst = list(10, 20, 30, 40, 50)
        lst[2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 30

    def test_list_index_first(self):
        """Verify indexing the first element."""
        catnip = Catnip()
        code = """
        lst = list(5, 10, 15)
        lst[0]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 5

    def test_list_index_last(self):
        """Verify indexing the last element."""
        catnip = Catnip()
        code = """
        lst = list(100, 200, 300)
        lst[2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 300

    def test_list_index_with_variable(self):
        """Verify l'indexation avec une variable comme index."""
        catnip = Catnip()
        code = """
        lst = list(1, 2, 3, 4, 5)
        idx = 3
        lst[idx]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 4

    def test_list_index_with_expression(self):
        """Verify l'indexation avec une expression comme index."""
        catnip = Catnip()
        code = """
        lst = list(10, 20, 30, 40)
        lst[1 + 1]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 30


class TestDictIndexing:
    """Tests for l'indexation of dictionaries."""

    def test_dict_index_string_key(self):
        """Verify dict indexing with a string key."""
        catnip = Catnip()
        code = """
        d = dict(('a', 1), ('b', 2), ('c', 3))
        d['b']
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 2

    def test_dict_index_multiple_keys(self):
        """Verify plusieurs indexations de dict."""
        catnip = Catnip()
        code = """
        d = dict(('x', 10), ('y', 20), ('z', 30))
        a = d['x']
        b = d['y']
        c = d['z']
        a + b + c
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 60


class TestNestedIndexing:
    """Tests for l'indexation nested."""

    def test_list_of_lists_indexing(self):
        """Verify l'indexation of lists nesteds."""
        catnip = Catnip()
        # Note: multiline list() not yet supported, using single line
        code = """
        matrix = list(list(1, 2, 3), list(4, 5, 6), list(7, 8, 9))
        row = matrix[1]
        row[2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 6

    def test_triple_nested_indexing(self):
        """Verify 3-level indexing."""
        catnip = Catnip()
        # Note: multiline list() not yet supported, using single line
        code = """
        cube = list(list(list(1, 2), list(3, 4)), list(list(5, 6), list(7, 8)))
        plane = cube[1]
        row = plane[0]
        row[1]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 6


class TestIndexingWithFunctions:
    """Tests for l'indexation combined with functions."""

    def test_indexing_function_result(self):
        """Verify indexing a function result."""
        catnip = Catnip()
        code = """
        get_list = () => { list(100, 200, 300) }
        result_list = get_list()
        result_list[1]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 200

    def test_function_returning_index(self):
        """Verify l'utilisation d'une fonction pour calculer l'index."""
        catnip = Catnip()
        code = """
        get_index = () => { 2 }
        lst = list(10, 20, 30, 40)
        lst[get_index()]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 30


class TestBroadcastAndIndexing:
    """Tests to ensure broadcasting and indexing coexist."""

    def test_broadcast_then_index(self):
        """Verify qu'on peut broadcaster puis indexer."""
        catnip = Catnip()
        code = """
        data = list(1, 2, 3, 4)
        result = data.[* 10]
        result[2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 30

    def test_index_then_broadcast(self):
        """Verify you can index then broadcast (on the result if it is a list)."""
        catnip = Catnip()
        # Note: multiline list() not yet supported, using single line
        code = """
        matrix = list(list(1, 2, 3), list(4, 5, 6))
        row = matrix[1]
        row.[* 2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [8, 10, 12]


class TestIndexingEdgeCases:
    """Tests for edge cases de l'indexation."""

    def test_empty_list_index_error(self):
        """Verify indexing an empty list raises an error."""
        catnip = Catnip()
        code = """
        lst = list()
        lst[0]
        """
        catnip.parse(code)

        with pytest.raises(IndexError, match="list index out of range"):
            catnip.execute()

    def test_index_out_of_range(self):
        """Verify an out-of-bounds index raises an error."""
        catnip = Catnip()
        code = """
        lst = list(1, 2, 3)
        lst[10]
        """
        catnip.parse(code)

        with pytest.raises(IndexError, match="list index out of range"):
            catnip.execute()

    def test_dict_missing_key_error(self):
        """Verify a missing dict key raises an error."""
        catnip = Catnip()
        code = """
        d = dict(('a', 1))
        d['missing']
        """
        catnip.parse(code)

        with pytest.raises(KeyError, match="'missing'"):
            catnip.execute()


class TestSlicing:
    """Tests for slicing avec la syntaxe [start:stop:step]."""

    def test_slice_start_stop(self):
        """Verify le slicing basique start:stop."""
        catnip = Catnip()
        code = """
        lst = list(10, 20, 30, 40, 50)
        lst[1:4]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [20, 30, 40]

    def test_slice_start_only(self):
        """Verify le slicing avec start seulement."""
        catnip = Catnip()
        code = """
        lst = list(1, 2, 3, 4, 5)
        lst[2:]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [3, 4, 5]

    def test_slice_stop_only(self):
        """Verify le slicing avec stop seulement."""
        catnip = Catnip()
        code = """
        lst = list(10, 20, 30, 40, 50)
        lst[:3]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [10, 20, 30]

    def test_slice_with_step(self):
        """Verify le slicing avec step."""
        catnip = Catnip()
        code = """
        lst = list(0, 1, 2, 3, 4, 5, 6, 7, 8, 9)
        lst[::2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [0, 2, 4, 6, 8]

    def test_slice_full_syntax(self):
        """Verify le slicing complet start:stop:step."""
        catnip = Catnip()
        code = """
        lst = list(0, 10, 20, 30, 40, 50, 60, 70, 80, 90)
        lst[1:8:2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [10, 30, 50, 70]

    def test_slice_negative_stop(self):
        """Verify slicing with a negative stop index."""
        catnip = Catnip()
        code = """
        lst = list(1, 2, 3, 4, 5, 6, 7, 8, 9)
        lst[4:-1]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [5, 6, 7, 8]

    def test_slice_negative_indices(self):
        """Verify slicing with negative indices."""
        catnip = Catnip()
        code = """
        lst = list(10, 20, 30, 40, 50)
        lst[-3:-1]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [30, 40]

    def test_slice_string(self):
        """Verify le slicing sur des strings."""
        catnip = Catnip()
        code = """
        s = "hello world"
        s[0:5]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == "hello"

    def test_slice_string_step(self):
        """Verify le slicing sur string avec step."""
        catnip = Catnip()
        code = """
        s = "abcdefgh"
        s[::2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == "aceg"

    def test_slice_reverse(self):
        """Verify slicing with a negative step for reversal."""
        catnip = Catnip()
        code = """
        lst = list(1, 2, 3, 4, 5)
        lst[::-1]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [5, 4, 3, 2, 1]

    def test_slice_empty_result(self):
        """Verify le slicing qui retourne une liste vide."""
        catnip = Catnip()
        code = """
        lst = list(1, 2, 3)
        lst[5:10]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == []


class TestFullslice:
    """Tests pour la syntaxe fullslice .[start:stop:step]."""

    def test_fullslice_all(self):
        """Verify le fullslice complet .[:]."""
        catnip = Catnip()
        code = """
        lst = list(1, 2, 3, 4, 5, 6)
        lst.[:]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [1, 2, 3, 4, 5, 6]

    def test_fullslice_start_stop(self):
        """Verify le fullslice avec start:stop."""
        catnip = Catnip()
        code = """
        lst = list(10, 20, 30, 40, 50)
        lst.[1:4]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [20, 30, 40]

    def test_fullslice_with_step(self):
        """Verify le fullslice avec step."""
        catnip = Catnip()
        code = """
        lst = list(0, 1, 2, 3, 4, 5, 6, 7, 8, 9)
        lst.[::2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [0, 2, 4, 6, 8]

    def test_fullslice_full_syntax(self):
        """Verify le fullslice complet start:stop:step."""
        catnip = Catnip()
        code = """
        lst = list(0, 10, 20, 30, 40, 50, 60, 70, 80, 90)
        lst.[1:8:2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [10, 30, 50, 70]

    def test_fullslice_negative_indices(self):
        """Verify fullslice with negative indices."""
        catnip = Catnip()
        code = """
        lst = list(0, 1, 2, 3, 4, 5, 6, 7, 8, 9)
        lst.[2:-2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [2, 3, 4, 5, 6, 7]

    def test_fullslice_on_function_call(self):
        """Verify le fullslice directement sur un appel de fonction."""
        catnip = Catnip()
        code = """
        get_list = () => { list(100, 200, 300, 400, 500) }
        get_list().[1:4]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [200, 300, 400]

    def test_fullslice_on_expression(self):
        """Verify le fullslice sur une expression."""
        catnip = Catnip()
        code = """
        (list(1, 2, 3) + list(4, 5, 6)).[1:5]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [2, 3, 4, 5]

    def test_fullslice_chained_with_broadcast(self):
        """Verify fullslice + broadcast chaining."""
        catnip = Catnip()
        code = """
        list(100, 200, 300, 400, 500).[:3].[* 2]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [200, 400, 600]

    def test_fullslice_on_string(self):
        """Verify le fullslice sur une string."""
        catnip = Catnip()
        code = """
        s = "hello world"
        s.[0:5]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == "hello"

    def test_fullslice_reverse(self):
        """Verify fullslice with a negative step for reversal."""
        catnip = Catnip()
        code = """
        list(1, 2, 3, 4, 5).[::-1]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [5, 4, 3, 2, 1]

    def test_fullslice_start_only(self):
        """Verify le fullslice avec start seulement."""
        catnip = Catnip()
        code = """
        data = list(10, 20, 30, 40, 50)
        data.[2:]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [30, 40, 50]

    def test_fullslice_stop_only(self):
        """Verify le fullslice avec stop seulement."""
        catnip = Catnip()
        code = """
        data = list(5, 10, 15, 20, 25)
        data.[:3]
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == [5, 10, 15]

    def test_fullslice_vs_regular_slice(self):
        """Verify fullslice .[...] and regular slice [...] give the same result."""
        catnip = Catnip()
        code = """
        data = list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
        regular = data[2:7:2]
        fullslice = data.[2:7:2]
        regular == fullslice
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result is True


class TestBinarySearchExample:
    """Test real-world case : binary_search avec indexation."""

    def test_binary_search_with_indexing(self):
        """Verify que binary_search fonctionne avec la syntaxe []."""
        catnip = Catnip()
        code = """
        binary_search = (liste, cible, gauche=0, droite=None) => {
            if droite == None { droite = len(liste) - 1 }

            if gauche > droite {
                -1
            } else {
                milieu = (gauche + droite) // 2
                valeur = liste[milieu]

                if valeur == cible {
                    milieu
                } elif valeur < cible {
                    binary_search(liste, cible, milieu + 1, droite)
                } else {
                    binary_search(liste, cible, gauche, milieu - 1)
                }
            }
        }

        liste = list(1, 3, 5, 7, 9, 11, 13, 15)
        binary_search(liste, 7)
        """
        catnip.parse(code)
        result = catnip.execute()

        assert result == 3


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
