# FILE: tests/language/test_python_callbacks.py
"""Tests for Catnip callables used as Python callbacks.

Verifies that VMFunction works correctly when called from Python builtins
(sorted, map, filter) and external C code (pandas.apply).
"""


class TestPythonCallbacks:
    def test_sorted_with_catnip_key(self, cat):
        """sorted() with key= Catnip lambda."""
        code = '''
        items = list(3, 1, 4, 1, 5)
        sorted(items, key=(x) => { -x })
        '''
        cat.parse(code)
        assert list(cat.execute()) == [5, 4, 3, 1, 1]

    def test_map_with_catnip_function(self, cat):
        """map() calls a Catnip lambda from Python C code."""
        cat.parse('double = (x) => { x * 2 }')
        cat.execute()
        double = cat.context.globals['double']
        result = list(map(double, [1, 2, 3]))
        assert result == [2, 4, 6]

    def test_filter_with_catnip_function(self, cat):
        """filter() calls a Catnip lambda from Python C code."""
        cat.parse('keep = (x) => { x > 2 }')
        cat.execute()
        keep = cat.context.globals['keep']
        result = list(filter(keep, [1, 2, 3, 4, 5]))
        assert result == [3, 4, 5]

    def test_callback_with_closure(self, cat):
        """Callback captures closure variable."""
        cat.parse('''
        factor = 10
        mul = (x) => { x * factor }
        ''')
        cat.execute()
        mul = cat.context.globals['mul']
        result = list(map(mul, [1, 2, 3]))
        assert result == [10, 20, 30]

    def test_callback_stress(self, cat):
        """Stress test: 1000 callback invocations from Python."""
        cat.parse('inc = (x) => { x + 1 }')
        cat.execute()
        inc = cat.context.globals['inc']
        result = list(map(inc, range(1000)))
        assert result == list(range(1, 1001))
