# FILE: tests/language/test_literals.py
"""Tests for list, tuple, set, and dict literals."""

import unittest

from catnip import Catnip
from catnip.exc import CatnipTypeError


class TestListLiterals(unittest.TestCase):
    """Test list literal syntax [...]."""

    def test_empty_list(self):
        """Test empty list creation."""
        c = Catnip()
        c.parse("x = list()")
        c.execute()
        assert c.context.globals['x'] == []

    def test_list_with_numbers(self):
        """Test list with numeric literals."""
        c = Catnip()
        c.parse("x = list(1, 2, 3, 4, 5)")
        c.execute()
        assert c.context.globals['x'] == [1, 2, 3, 4, 5]

    def test_list_with_strings(self):
        """Test list with string literals."""
        c = Catnip()
        c.parse('x = list("a", "b", "c")')
        c.execute()
        assert c.context.globals['x'] == ["a", "b", "c"]

    def test_list_with_expressions(self):
        """Test list with computed values."""
        c = Catnip()
        c.parse("x = list(1 + 1, 2 * 3, 10 / 2)")
        c.execute()
        assert c.context.globals['x'] == [2, 6, 5.0]

    def test_list_with_variables(self):
        """Test list with variable references."""
        c = Catnip()
        c.parse("a = 10; b = 20; x = list(a, b, a + b)")
        c.execute()
        assert c.context.globals['x'] == [10, 20, 30]

    def test_nested_lists(self):
        """Test nested list literals."""
        c = Catnip()
        c.parse("x = list(list(1, 2), list(3, 4), list(5, 6))")
        c.execute()
        assert c.context.globals['x'] == [[1, 2], [3, 4], [5, 6]]

    def test_list_iteration(self):
        """Test iterating over list literals."""
        c = Catnip()
        c.parse("""
            nums = list(1, 2, 3, 4, 5)
            total = 0
            for n in nums {
                total = total + n
            }
        """)
        c.execute()
        assert c.context.globals['total'] == 15

    def test_list_indexing(self):
        """Test accessing list elements."""
        c = Catnip()
        c.parse("x = list(10, 20, 30); first = x.__getitem__(0); last = x.__getitem__(2)")
        c.execute()
        assert c.context.globals['first'] == 10
        assert c.context.globals['last'] == 30

    def test_list_trailing_comma(self):
        """Test that trailing commas are allowed."""
        c = Catnip()
        c.parse("x = list(1, 2, 3,)")
        c.execute()
        assert c.context.globals['x'] == [1, 2, 3]


class TestSetLiterals(unittest.TestCase):
    """Test set literal syntax set(...)."""

    def test_empty_set(self):
        """Test empty set creation."""
        c = Catnip()
        c.parse("x = set()")
        c.execute()
        assert c.context.globals['x'] == set()

    def test_set_with_numbers(self):
        """Test set with numeric literals."""
        c = Catnip()
        c.parse("x = set(1, 2, 3, 4, 5)")
        c.execute()
        assert c.context.globals['x'] == {1, 2, 3, 4, 5}

    def test_set_with_strings(self):
        """Test set with string literals."""
        c = Catnip()
        c.parse('x = set("a", "b", "c")')
        c.execute()
        assert c.context.globals['x'] == {'a', 'b', 'c'}

    def test_set_with_duplicates(self):
        """Test that duplicates are removed."""
        c = Catnip()
        c.parse("x = set(1, 2, 2, 3, 3, 3)")
        c.execute()
        assert c.context.globals['x'] == {1, 2, 3}

    def test_set_with_expressions(self):
        """Test set with computed values."""
        c = Catnip()
        c.parse("x = set(1 + 1, 2 * 3, 10 / 5)")
        c.execute()
        assert c.context.globals['x'] == {2, 6, 2.0}

    def test_set_with_variables(self):
        """Test set with variable references."""
        c = Catnip()
        c.parse("a = 10; b = 20; x = set(a, b, a + b)")
        c.execute()
        assert c.context.globals['x'] == {10, 20, 30}

    def test_set_membership(self):
        """Test checking if element is in set."""
        c = Catnip()
        c.parse("s = set(1, 2, 3); has_two = s.__contains__(2); has_five = s.__contains__(5)")
        c.execute()
        assert c.context.globals['has_two'] is True
        assert c.context.globals['has_five'] is False

    def test_set_length(self):
        """Test getting set size."""
        c = Catnip()
        c.parse("s = set(1, 2, 3, 4, 5); size = len(s)")
        c.execute()
        assert c.context.globals['size'] == 5

    def test_set_length_with_duplicates(self):
        """Test that length reflects unique elements only."""
        c = Catnip()
        c.parse("s = set(1, 1, 2, 2, 3, 3); size = len(s)")
        c.execute()
        assert c.context.globals['size'] == 3

    def test_set_add(self):
        """Test adding element to set."""
        c = Catnip()
        c.parse("s = set(1, 2, 3); s.add(4); s.add(2)")
        c.execute()
        assert c.context.globals['s'] == {1, 2, 3, 4}

    def test_set_remove(self):
        """Test removing element from set."""
        c = Catnip()
        c.parse("s = set(1, 2, 3, 4); s.remove(2)")
        c.execute()
        assert c.context.globals['s'] == {1, 3, 4}

    def test_set_discard(self):
        """Test discarding element (no error if missing)."""
        c = Catnip()
        c.parse("s = set(1, 2, 3); s.discard(2); s.discard(99)")
        c.execute()
        assert c.context.globals['s'] == {1, 3}

    def test_set_union(self):
        """Test union of two sets."""
        c = Catnip()
        c.parse("a = set(1, 2, 3); b = set(3, 4, 5); c = a.union(b)")
        c.execute()
        assert c.context.globals['c'] == {1, 2, 3, 4, 5}

    def test_set_intersection(self):
        """Test intersection of two sets."""
        c = Catnip()
        c.parse("a = set(1, 2, 3, 4); b = set(3, 4, 5, 6); c = a.intersection(b)")
        c.execute()
        assert c.context.globals['c'] == {3, 4}

    def test_set_difference(self):
        """Test difference of two sets."""
        c = Catnip()
        c.parse("a = set(1, 2, 3, 4); b = set(3, 4, 5); c = a.difference(b)")
        c.execute()
        assert c.context.globals['c'] == {1, 2}

    def test_set_symmetric_difference(self):
        """Test symmetric difference of two sets."""
        c = Catnip()
        c.parse("a = set(1, 2, 3); b = set(3, 4, 5); c = a.symmetric_difference(b)")
        c.execute()
        assert c.context.globals['c'] == {1, 2, 4, 5}

    def test_set_issubset(self):
        """Test checking if one set is subset of another."""
        c = Catnip()
        c.parse("a = set(1, 2); b = set(1, 2, 3, 4); is_sub = a.issubset(b); not_sub = b.issubset(a)")
        c.execute()
        assert c.context.globals['is_sub'] is True
        assert c.context.globals['not_sub'] is False

    def test_set_issuperset(self):
        """Test checking if one set is superset of another."""
        c = Catnip()
        c.parse("a = set(1, 2, 3, 4); b = set(1, 2); is_super = a.issuperset(b); not_super = b.issuperset(a)")
        c.execute()
        assert c.context.globals['is_super'] is True
        assert c.context.globals['not_super'] is False

    def test_set_isdisjoint(self):
        """Test checking if two sets have no common elements."""
        c = Catnip()
        c.parse(
            "a = set(1, 2, 3); b = set(4, 5, 6); c = set(3, 4, 5); disjoint = a.isdisjoint(b); not_disjoint = a.isdisjoint(c)"
        )
        c.execute()
        assert c.context.globals['disjoint'] is True
        assert c.context.globals['not_disjoint'] is False

    def test_set_clear(self):
        """Test clearing all elements from set."""
        c = Catnip()
        c.parse("s = set(1, 2, 3, 4, 5); s.clear(); size = len(s)")
        c.execute()
        assert c.context.globals['s'] == set()
        assert c.context.globals['size'] == 0

    def test_set_copy(self):
        """Test creating a copy of set."""
        c = Catnip()
        c.parse("a = set(1, 2, 3); b = a.copy(); a.add(4)")
        c.execute()
        assert c.context.globals['a'] == {1, 2, 3, 4}
        assert c.context.globals['b'] == {1, 2, 3}

    def test_set_update(self):
        """Test updating set with elements from another."""
        c = Catnip()
        c.parse("a = set(1, 2, 3); b = set(3, 4, 5); a.update(b)")
        c.execute()
        assert c.context.globals['a'] == {1, 2, 3, 4, 5}

    def test_set_trailing_comma(self):
        """Test that trailing commas are allowed."""
        c = Catnip()
        c.parse("x = set(1, 2, 3,)")
        c.execute()
        assert c.context.globals['x'] == {1, 2, 3}

    def test_set_iteration(self):
        """Test iterating over set elements."""
        c = Catnip()
        c.parse("""
            s = set(1, 2, 3, 4, 5)
            total = 0
            for n in s {
                total = total + n
            }
        """)
        c.execute()
        assert c.context.globals['total'] == 15

    def test_set_mixed_types(self):
        """Test set with mixed types."""
        c = Catnip()
        c.parse('x = set(1, "two", 3.0)')
        c.execute()
        assert c.context.globals['x'] == {1, "two", 3.0}


class TestTupleLiterals(unittest.TestCase):
    """Test tuple literal syntax tuple(...)."""

    def test_empty_tuple(self):
        """Test empty tuple creation."""
        c = Catnip()
        c.parse("x = tuple()")
        c.execute()
        assert c.context.globals['x'] == ()

    def test_tuple_with_numbers(self):
        """Test tuple with numeric literals."""
        c = Catnip()
        c.parse("x = tuple(1, 2, 3, 4, 5)")
        c.execute()
        assert c.context.globals['x'] == (1, 2, 3, 4, 5)

    def test_tuple_with_strings(self):
        """Test tuple with string literals."""
        c = Catnip()
        c.parse('x = tuple("a", "b", "c")')
        c.execute()
        assert c.context.globals['x'] == ("a", "b", "c")

    def test_tuple_with_expressions(self):
        """Test tuple with computed values."""
        c = Catnip()
        c.parse("x = tuple(1 + 1, 2 * 3, 10 / 2)")
        c.execute()
        assert c.context.globals['x'] == (2, 6, 5.0)

    def test_tuple_with_variables(self):
        """Test tuple with variable references."""
        c = Catnip()
        c.parse("a = 10; b = 20; x = tuple(a, b, a + b)")
        c.execute()
        assert c.context.globals['x'] == (10, 20, 30)

    def test_nested_tuples(self):
        """Test nested tuple literals."""
        c = Catnip()
        c.parse("x = tuple(tuple(1, 2), tuple(3, 4), tuple(5, 6))")
        c.execute()
        assert c.context.globals['x'] == ((1, 2), (3, 4), (5, 6))

    def test_tuple_iteration(self):
        """Test iterating over tuple literals."""
        c = Catnip()
        c.parse("""
            nums = tuple(1, 2, 3, 4, 5)
            total = 0
            for n in nums {
                total = total + n
            }
        """)
        c.execute()
        assert c.context.globals['total'] == 15

    def test_tuple_indexing(self):
        """Test accessing tuple elements."""
        c = Catnip()
        c.parse("x = tuple(10, 20, 30); first = x.__getitem__(0); last = x.__getitem__(2)")
        c.execute()
        assert c.context.globals['first'] == 10
        assert c.context.globals['last'] == 30

    def test_tuple_length(self):
        """Test getting tuple size."""
        c = Catnip()
        c.parse("t = tuple(1, 2, 3, 4, 5); size = len(t)")
        c.execute()
        assert c.context.globals['size'] == 5

    def test_tuple_membership(self):
        """Test checking if element is in tuple."""
        c = Catnip()
        c.parse("t = tuple(1, 2, 3); has_two = t.__contains__(2); has_five = t.__contains__(5)")
        c.execute()
        assert c.context.globals['has_two'] is True
        assert c.context.globals['has_five'] is False

    def test_tuple_trailing_comma(self):
        """Test that trailing commas are allowed."""
        c = Catnip()
        c.parse("x = tuple(1, 2, 3,)")
        c.execute()
        assert c.context.globals['x'] == (1, 2, 3)

    def test_tuple_mixed_types(self):
        """Test tuple with mixed types."""
        c = Catnip()
        c.parse('x = tuple(1, "two", 3.0)')
        c.execute()
        assert c.context.globals['x'] == (1, "two", 3.0)

    def test_single_element_tuple(self):
        """Test tuple with single element."""
        c = Catnip()
        c.parse("x = tuple(42)")
        c.execute()
        assert c.context.globals['x'] == (42,)

    def test_tuple_immutability(self):
        """Test that tuples are immutable (no mutation methods)."""
        c = Catnip()
        c.parse("t = tuple(1, 2, 3)")
        c.execute()
        # Tuples don't have append, add, etc. - just verify it's a real tuple
        assert isinstance(c.context.globals['t'], tuple)


class TestDictLiterals(unittest.TestCase):
    """Test dict literal syntax dict((k,v), ...)."""

    def test_empty_dict(self):
        """Test empty dict creation."""
        c = Catnip()
        c.parse("x = dict()")
        c.execute()
        assert c.context.globals['x'] == {}

    def test_dict_with_strings(self):
        """Test dict with string keys."""
        c = Catnip()
        c.parse('x = dict(("a", 1), ("b", 2), ("c", 3))')
        c.execute()
        assert c.context.globals['x'] == {'a': 1, 'b': 2, 'c': 3}

    def test_dict_with_number_keys(self):
        """Test dict with numeric keys."""
        c = Catnip()
        c.parse('x = dict((1, "one"), (2, "two"), (3, "three"))')
        c.execute()
        assert c.context.globals['x'] == {1: "one", 2: "two", 3: "three"}

    def test_dict_with_expressions(self):
        """Test dict with computed values."""
        c = Catnip()
        c.parse('x = dict(("a", 1 + 1), ("b", 2 * 3), ("c", 10 / 2))')
        c.execute()
        assert c.context.globals['x'] == {'a': 2, 'b': 6, 'c': 5.0}

    def test_dict_with_variables(self):
        """Test dict with variable references."""
        c = Catnip()
        c.parse('a = 10; b = 20; x = dict(("first", a), ("second", b), ("sum", a + b))')
        c.execute()
        assert c.context.globals['x'] == {'first': 10, 'second': 20, 'sum': 30}

    def test_dict_access(self):
        """Test accessing dict values."""
        c = Catnip()
        c.parse('x = dict(("name", "Alice"), ("age", 30)); name = x.__getitem__("name")')
        c.execute()
        assert c.context.globals['name'] == "Alice"

    def test_dict_trailing_comma(self):
        """Test that trailing commas are allowed."""
        c = Catnip()
        c.parse('x = dict(("a", 1), ("b", 2),)')
        c.execute()
        assert c.context.globals['x'] == {'a': 1, 'b': 2}

    def test_nested_structures(self):
        """Test dicts containing lists and vice versa."""
        c = Catnip()
        c.parse('x = dict(("nums", list(1, 2, 3)), ("names", list("a", "b")))')
        c.execute()
        assert c.context.globals['x'] == {'nums': [1, 2, 3], 'names': ["a", "b"]}


class TestDictKwargs(unittest.TestCase):
    """Test dict kwargs syntax dict(key=value, ...)."""

    def test_dict_kwargs_only(self):
        c = Catnip()
        c.parse('x = dict(name="Alice", age=30)')
        c.execute()
        assert c.context.globals['x'] == {'name': "Alice", 'age': 30}

    def test_dict_kwargs_mixed(self):
        c = Catnip()
        c.parse('x = dict((1, "un"), name="Alice", (2, "deux"))')
        c.execute()
        assert c.context.globals['x'] == {1: "un", 'name': "Alice", 2: "deux"}

    def test_dict_kwargs_all_keys_present(self):
        c = Catnip()
        c.parse('x = dict(z=1, a=2, m=3)')
        c.execute()
        assert c.context.globals['x'] == {'z': 1, 'a': 2, 'm': 3}

    def test_dict_kwargs_trailing_comma(self):
        c = Catnip()
        c.parse('x = dict(a=1,)')
        c.execute()
        assert c.context.globals['x'] == {'a': 1}

    def test_dict_kwargs_with_expressions(self):
        c = Catnip()
        c.parse('x = dict(sum=1+2)')
        c.execute()
        assert c.context.globals['x'] == {'sum': 3}

    def test_dict_kwargs_with_variables(self):
        c = Catnip()
        c.parse('a = 42; x = dict(val=a)')
        c.execute()
        assert c.context.globals['x'] == {'val': 42}

    def test_dict_backwards_compat(self):
        c = Catnip()
        c.parse('x = dict(("a", 1), ("b", 2))')
        c.execute()
        assert c.context.globals['x'] == {'a': 1, 'b': 2}


class TestDictFromIterable(unittest.TestCase):
    """Test dict(iterable_of_pairs) - construct dict from a single iterable."""

    def test_dict_from_list_of_tuples(self):
        c = Catnip()
        c.parse('x = dict(list(tuple("a", 1), tuple("b", 2)))')
        c.execute()
        assert c.context.globals['x'] == {'a': 1, 'b': 2}

    def test_dict_from_zip(self):
        c = Catnip()
        c.parse('x = dict(zip(list("x", "y", "z"), list(1, 2, 3)))')
        c.execute()
        assert c.context.globals['x'] == {'x': 1, 'y': 2, 'z': 3}

    def test_dict_from_broadcast(self):
        c = Catnip()
        c.parse('x = dict(range(5).[(n) => { tuple(n, n ** 2) }])')
        c.execute()
        assert c.context.globals['x'] == {0: 0, 1: 1, 2: 4, 3: 9, 4: 16}

    def test_dict_from_filtered_broadcast(self):
        c = Catnip()
        c.parse('x = dict(range(10).[if (n) => { n % 2 == 0 }].[(n) => { tuple(n, n * 10) }])')
        c.execute()
        assert c.context.globals['x'] == {0: 0, 2: 20, 4: 40, 6: 60, 8: 80}

    def test_dict_from_variable(self):
        c = Catnip()
        c.parse('pairs = list(tuple("a", 1), tuple("b", 2)); x = dict(pairs)')
        c.execute()
        assert c.context.globals['x'] == {'a': 1, 'b': 2}

    def test_dict_from_iterable_with_kwargs(self):
        c = Catnip()
        c.parse('x = dict(list(tuple("a", 1)), extra=42)')
        c.execute()
        assert c.context.globals['x'] == {'a': 1, 'extra': 42}

    def test_dict_from_iterable_with_spread(self):
        c = Catnip()
        c.parse('base = dict(x=10); x = dict(list(tuple("a", 1)), **base)')
        c.execute()
        assert c.context.globals['x'] == {'a': 1, 'x': 10}

    def test_dict_from_iterable_preserves_existing_syntax(self):
        """Existing dict() forms still work."""
        c = Catnip()
        c.parse('a = dict(); b = dict(x=1); c = dict(("k", 2))')
        c.execute()
        assert c.context.globals['a'] == {}
        assert c.context.globals['b'] == {'x': 1}
        assert c.context.globals['c'] == {'k': 2}


class TestCombined(unittest.TestCase):
    """Test combining lists, tuples, sets, and dicts."""

    def test_list_of_dicts(self):
        """Test list containing dict literals."""
        c = Catnip()
        c.parse('x = list(dict(("a", 1)), dict(("b", 2)), dict(("c", 3)))')
        c.execute()
        assert c.context.globals['x'] == [{'a': 1}, {'b': 2}, {'c': 3}]

    def test_dict_with_list_values(self):
        """Test dict with list values."""
        c = Catnip()
        c.parse('x = dict(("evens", list(2, 4, 6)), ("odds", list(1, 3, 5)))')
        c.execute()
        assert c.context.globals['x'] == {'evens': [2, 4, 6], 'odds': [1, 3, 5]}

    def test_list_of_tuples(self):
        """Test list containing tuple literals."""
        c = Catnip()
        c.parse("x = list(tuple(1, 2), tuple(3, 4), tuple(5, 6))")
        c.execute()
        assert c.context.globals['x'] == [(1, 2), (3, 4), (5, 6)]

    def test_tuple_of_lists(self):
        """Test tuple containing list literals."""
        c = Catnip()
        c.parse("x = tuple(list(1, 2), list(3, 4), list(5, 6))")
        c.execute()
        assert c.context.globals['x'] == ([1, 2], [3, 4], [5, 6])

    def test_dict_with_tuple_keys(self):
        """Test dict with tuple keys."""
        c = Catnip()
        c.parse('x = dict((tuple(1, 2), "a"), (tuple(3, 4), "b"))')
        c.execute()
        assert c.context.globals['x'] == {(1, 2): "a", (3, 4): "b"}

    def test_set_of_tuples(self):
        """Test set containing tuple literals."""
        c = Catnip()
        c.parse("x = set(tuple(1, 2), tuple(3, 4), tuple(1, 2))")
        c.execute()
        assert c.context.globals['x'] == {(1, 2), (3, 4)}


class TestSingleArgLiteral(unittest.TestCase):
    """Single-arg collection literals wrap, they do not auto-consume iterables."""

    def test_list_range(self):
        c = Catnip()
        c.parse("x = list(range(5))")
        c.execute()
        assert c.context.globals['x'] == [range(0, 5)]

    def test_list_reversed(self):
        c = Catnip()
        c.parse("x = list(reversed(list(1, 2, 3)))")
        c.execute()
        assert len(c.context.globals['x']) == 1

    def test_list_string_iterates(self):
        """Single arg string is wrapped as one element."""
        c = Catnip()
        c.parse('x = list("hello")')
        c.execute()
        assert c.context.globals['x'] == ["hello"]

    def test_list_non_iterable_wraps(self):
        c = Catnip()
        c.parse("x = list(42)")
        c.execute()
        assert c.context.globals['x'] == [42]

    def test_list_of_list_wraps(self):
        c = Catnip()
        c.parse("x = list(list(1, 2, 3))")
        c.execute()
        assert c.context.globals['x'] == [[1, 2, 3]]

    def test_tuple_range(self):
        c = Catnip()
        c.parse("x = tuple(range(3))")
        c.execute()
        assert len(c.context.globals['x']) == 1

    def test_set_range(self):
        c = Catnip()
        c.parse("x = set(range(5))")
        c.execute()
        assert len(c.context.globals['x']) == 1

    def test_list_multi_arg_unchanged(self):
        c = Catnip()
        c.parse("x = list(1, 2, 3)")
        c.execute()
        assert c.context.globals['x'] == [1, 2, 3]

    def test_list_map(self):
        c = Catnip()
        c.parse("""
            double = (x) => { x * 2 }
            x = list(map(double, list(1, 2, 3)))
        """)
        c.execute()
        assert len(c.context.globals['x']) == 1

    def test_tuple_of_list(self):
        """Single arg list is wrapped in tuple."""
        c = Catnip()
        c.parse("x = tuple(list(1, 2, 3))")
        c.execute()
        assert c.context.globals['x'] == ([1, 2, 3],)

    def test_set_of_list_raises_unhashable(self):
        """Single arg list is wrapped; adding list to set is unhashable."""
        c = Catnip()
        c.parse("x = set(list(1, 2, 2, 3))")
        with self.assertRaises(CatnipTypeError):
            c.execute()

    def test_tuple_string_iterates(self):
        """Single arg string is wrapped as one tuple element."""
        c = Catnip()
        c.parse('x = tuple("hello")')
        c.execute()
        assert c.context.globals['x'] == ("hello",)

    def test_set_string_iterates(self):
        """Single arg string is wrapped as one set element."""
        c = Catnip()
        c.parse('x = set("hello")')
        c.execute()
        assert c.context.globals['x'] == {"hello"}

    def test_list_multi_string_wraps(self):
        """Multi-arg strings are wrapped, not iterated."""
        c = Catnip()
        c.parse('x = list("hello", "world")')
        c.execute()
        assert c.context.globals['x'] == ["hello", "world"]


class TestCollectionSpreadLiterals(unittest.TestCase):
    """Explicit spread (*, **) in collection literals."""

    def test_list_spread(self):
        c = Catnip()
        c.parse("x = list(*list(1, 2), 3, *tuple(4, 5))")
        c.execute()
        assert c.context.globals['x'] == [1, 2, 3, 4, 5]

    def test_tuple_spread(self):
        c = Catnip()
        c.parse("x = tuple(*list(1, 2), 3)")
        c.execute()
        assert c.context.globals['x'] == (1, 2, 3)

    def test_set_spread(self):
        c = Catnip()
        c.parse("x = set(*list(1, 2, 2), 3)")
        c.execute()
        assert c.context.globals['x'] == {1, 2, 3}

    def test_dict_spread(self):
        c = Catnip()
        c.parse('x = dict(**dict(a=1), ("b", 2), c=3, **dict(d=4))')
        c.execute()
        assert c.context.globals['x'] == {'a': 1, 'b': 2, 'c': 3, 'd': 4}


class TestBracketListLiterals(unittest.TestCase):
    """Test bracket list literal syntax [...]."""

    def test_empty(self):
        c = Catnip()
        c.parse("x = []")
        c.execute()
        assert c.context.globals['x'] == []

    def test_numbers(self):
        c = Catnip()
        c.parse("x = [1, 2, 3]")
        c.execute()
        assert c.context.globals['x'] == [1, 2, 3]

    def test_strings(self):
        c = Catnip()
        c.parse('x = ["a", "b", "c"]')
        c.execute()
        assert c.context.globals['x'] == ["a", "b", "c"]

    def test_expressions(self):
        c = Catnip()
        c.parse("x = [1 + 1, 2 * 3]")
        c.execute()
        assert c.context.globals['x'] == [2, 6]

    def test_nested(self):
        c = Catnip()
        c.parse("x = [[1, 2], [3, 4]]")
        c.execute()
        assert c.context.globals['x'] == [[1, 2], [3, 4]]

    def test_trailing_comma(self):
        c = Catnip()
        c.parse("x = [1, 2, 3,]")
        c.execute()
        assert c.context.globals['x'] == [1, 2, 3]

    def test_index(self):
        c = Catnip()
        c.parse("x = [10, 20, 30][1]")
        c.execute()
        assert c.context.globals['x'] == 20

    def test_spread(self):
        c = Catnip()
        c.parse("a = [1, 2]; x = [*a, 3, 4]")
        c.execute()
        assert c.context.globals['x'] == [1, 2, 3, 4]

    def test_in_function_arg(self):
        c = Catnip()
        c.parse("x = len([1, 2, 3])")
        c.execute()
        assert c.context.globals['x'] == 3

    def test_single_element(self):
        c = Catnip()
        c.parse("x = [42]")
        c.execute()
        assert c.context.globals['x'] == [42]

    def test_method_call(self):
        c = Catnip()
        c.parse("x = [3, 1, 2]; x.sort()")
        c.execute()
        assert c.context.globals['x'] == [1, 2, 3]


class TestBracketDictLiterals(unittest.TestCase):
    """Test bracket dict literal syntax {key: value}."""

    def test_string_keys(self):
        c = Catnip()
        c.parse('x = {"a": 1, "b": 2}')
        c.execute()
        assert c.context.globals['x'] == {"a": 1, "b": 2}

    def test_number_keys(self):
        c = Catnip()
        c.parse('x = {1: "one", 2: "two"}')
        c.execute()
        assert c.context.globals['x'] == {1: "one", 2: "two"}

    def test_expressions(self):
        c = Catnip()
        c.parse('x = {"sum": 1 + 2}')
        c.execute()
        assert c.context.globals['x'] == {"sum": 3}

    def test_index(self):
        c = Catnip()
        c.parse('x = {"a": 1, "b": 2}["a"]')
        c.execute()
        assert c.context.globals['x'] == 1

    def test_nested(self):
        c = Catnip()
        c.parse('x = {"inner": {"a": 1}}')
        c.execute()
        assert c.context.globals['x'] == {"inner": {"a": 1}}

    def test_with_bracket_list_values(self):
        c = Catnip()
        c.parse('x = {"nums": [1, 2, 3], "names": ["a", "b"]}')
        c.execute()
        assert c.context.globals['x'] == {"nums": [1, 2, 3], "names": ["a", "b"]}

    def test_trailing_comma(self):
        c = Catnip()
        c.parse('x = {"a": 1,}')
        c.execute()
        assert c.context.globals['x'] == {"a": 1}

    def test_spread(self):
        c = Catnip()
        c.parse('base = dict(a=1); x = {**base, "b": 2}')
        c.execute()
        assert c.context.globals['x'] == {"a": 1, "b": 2}

    def test_single_pair(self):
        c = Catnip()
        c.parse('x = {"only": 42}')
        c.execute()
        assert c.context.globals['x'] == {"only": 42}

    def test_bool_key(self):
        c = Catnip()
        c.parse('x = {True: "yes", False: "no"}')
        c.execute()
        assert c.context.globals['x'] == {True: "yes", False: "no"}


class TestBlockRegression(unittest.TestCase):
    """Ensure blocks still work after adding bracket dict."""

    def test_block_returns_last(self):
        c = Catnip()
        c.parse("{ 42 }")
        assert c.execute() == 42

    def test_block_with_statements(self):
        c = Catnip()
        c.parse("{ a = 1; a + 1 }")
        assert c.execute() == 2


if __name__ == "__main__":
    unittest.main()
