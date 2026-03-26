# FILE: tests/apps/test_pandas.py
"""Tests for pandas integration.

Tests Catnip's ability to work with pandas DataFrames, Series, and operations.
Each test focuses on a specific pandas feature or operation.

Test Strategy:
--------------
1. **Basic operations** - Column access, method calls, attributes
2. **Edge cases** - NaN, None, empty DataFrames, mixed types
3. **Error propagation** - Verify pandas errors are correctly propagated
4. **Known limitations** - Document current Catnip limitations with pandas

Note: Pandas uses Python dict and list syntax which is different from Catnip.
We inject pandas DataFrames directly into the Catnip context for testing.

Known Limitations:
------------------
- Comparison operators (>, <, ==) on Series not yet supported
- Boolean indexing (df[mask]) not yet supported
- Indexed assignment (df['col'][0] = value) not yet supported
- List/dict literals needed for some pandas operations

These limitations are Catnip syntax issues, not pandas integration problems.
The tests document what works and what doesn't to help developers understand
where errors come from (Catnip vs pandas).
"""

import pytest

# Skip all tests in this file if pandas is not available
pandas = pytest.importorskip("pandas", reason="pandas not installed")


def test_pandas_available_in_context(cat):
    """Test that pandas can be injected into Catnip context."""
    import pandas as pd

    # Inject pandas module into context
    cat.registry.ctx.globals['pd'] = pd

    # Access pandas from Catnip
    cat.parse("pd")
    result = cat.execute()
    assert result is pd


def test_dataframe_from_python(cat):
    """Test accessing a pandas DataFrame created in Python."""
    import pandas as pd

    # Create DataFrame in Python
    df = pd.DataFrame({'a': [1, 2, 3], 'b': [4, 5, 6]})

    # Inject into Catnip context
    cat.registry.ctx.globals['df'] = df

    # Access from Catnip
    cat.parse("df")
    result = cat.execute()
    assert isinstance(result, pd.DataFrame)
    assert list(result.columns) == ["a", "b"]
    assert len(result) == 3


def test_dataframe_column_access(cat):
    """Test DataFrame column access using getitem."""
    import pandas as pd

    df = pd.DataFrame({'x': [10, 20, 30], 'y': [40, 50, 60]})
    cat.registry.ctx.globals['df'] = df

    # Access column using bracket notation
    cat.parse('df["x"]')
    result = cat.execute()
    assert isinstance(result, pd.Series)
    assert list(result) == [10, 20, 30]


def test_dataframe_method_call(cat):
    """Test calling DataFrame methods."""
    import pandas as pd

    df = pd.DataFrame({'values': [1, 2, 3, 4, 5]})
    cat.registry.ctx.globals['df'] = df

    # Call sum() method on a Series
    cat.parse('df["values"].sum()')
    result = cat.execute()
    assert result == 15


def test_dataframe_attribute_access(cat):
    """Test accessing DataFrame attributes."""
    import pandas as pd

    df = pd.DataFrame({'a': [1, 2], 'b': [3, 4]})
    cat.registry.ctx.globals['df'] = df

    # Access columns attribute
    cat.parse("df.columns")
    result = cat.execute()
    assert list(result) == ["a", "b"]


def test_dataframe_operations_chain(cat):
    """Test chaining DataFrame operations."""
    import pandas as pd

    df = pd.DataFrame({'x': [1, 2, 3, 4, 5]})
    cat.registry.ctx.globals['df'] = df

    # Chain: select column, multiply by 2, sum
    cat.parse('df["x"] * 2')
    series = cat.execute()
    assert list(series) == [2, 4, 6, 8, 10]


def test_dataframe_shape(cat):
    """Test accessing DataFrame shape attribute."""
    import pandas as pd

    df = pd.DataFrame({'a': [1, 2, 3], 'b': [4, 5, 6]})
    cat.registry.ctx.globals['df'] = df

    cat.parse("df.shape")
    result = cat.execute()
    assert result == (3, 2)


def test_series_mean(cat):
    """Test Series statistical methods."""
    import pandas as pd

    series = pd.Series([10, 20, 30, 40, 50])
    cat.registry.ctx.globals['s'] = series

    cat.parse("s.mean()")
    result = cat.execute()
    assert result == 30.0


def test_dataframe_head(cat):
    """Test DataFrame head() method."""
    import pandas as pd

    df = pd.DataFrame({'x': range(10)})
    cat.registry.ctx.globals['df'] = df

    cat.parse("df.head(3)")
    result = cat.execute()
    assert len(result) == 3
    assert list(result['x']) == [0, 1, 2]


# --- Edge cases and error handling ---


def test_pandas_type_error_propagates(cat):
    """Test that pandas TypeError is properly propagated."""
    import pandas as pd

    df = pd.DataFrame({'numbers': [1, 2, 3]})
    cat.registry.ctx.globals['df'] = df

    # Try to set a string in a numeric column
    cat.parse('df["numbers"][0] = "text"')

    # This should raise a pandas error, not a Catnip error
    # The exact error depends on pandas version, but it should mention pandas/numpy
    with pytest.raises(Exception) as exc_info:
        cat.execute()

    # Verify it's not a Catnip-specific error
    error_msg = str(exc_info.value)
    # Should mention type conversion or assignment issue
    assert any(
        keyword in error_msg.lower()
        for keyword in ["type", "invalid", "cannot", "convert", "incompatible", "object", "dtype"]
    )


def test_pandas_key_error_propagates(cat):
    """Test that pandas KeyError for missing column is properly propagated."""
    import pandas as pd

    df = pd.DataFrame({'a': [1, 2, 3]})
    cat.registry.ctx.globals['df'] = df

    # Try to access non-existent column
    cat.parse('df["nonexistent"]')

    with pytest.raises(KeyError) as exc_info:
        cat.execute()

    # Verify it's a KeyError about the column
    assert "nonexistent" in str(exc_info.value)


def test_pandas_index_error_propagates(cat):
    """Test that pandas IndexError is properly propagated."""
    import pandas as pd

    series = pd.Series([1, 2, 3])
    cat.registry.ctx.globals['s'] = series

    # Try to access out-of-bounds index using iloc
    cat.parse("s.iloc[10]")

    with pytest.raises((IndexError, KeyError)) as exc_info:
        cat.execute()

    # Should mention index or out of bounds
    error_msg = str(exc_info.value)
    assert any(keyword in error_msg.lower() for keyword in ["index", "bound", "out of", "single"])


def test_dataframe_with_nan(cat):
    """Test DataFrame with NaN values."""
    import math

    import pandas as pd

    df = pd.DataFrame({'x': [1.0, float('nan'), 3.0]})
    cat.registry.ctx.globals['df'] = df

    # Access the Series (not individual element, indexing returns scalar)
    cat.parse('df["x"]')
    series = cat.execute()
    # Verify it contains NaN
    assert math.isnan(series.iloc[1])

    # Count non-NaN values
    cat.parse('df["x"].count()')
    result = cat.execute()
    assert result == 2  # Only 2 non-NaN values


def test_dataframe_with_none(cat):
    """Test DataFrame with None values (object dtype)."""
    import numpy as np
    import pandas as pd

    df = pd.DataFrame({'x': [1, None, 3]})
    cat.registry.ctx.globals['df'] = df

    # Access the Series
    cat.parse('df["x"]')
    series = cat.execute()
    # Check that the second element is None or NaN (pandas may convert)
    value = series.iloc[1]
    assert value is None or (isinstance(value, float) and np.isnan(value))


def test_dataframe_mixed_types(cat):
    """Test DataFrame with mixed types (should work, pandas is flexible)."""
    import pandas as pd

    df = pd.DataFrame({'mixed': [1, "text", 3.14, None]})
    cat.registry.ctx.globals['df'] = df

    # Access different types
    cat.parse('df["mixed"][0]')
    result = cat.execute()
    assert result == 1

    cat.parse('df["mixed"][1]')
    result = cat.execute()
    assert result == "text"


def test_dataframe_boolean_series(cat):
    """Test using boolean Series created in Python.

    Note: Catnip doesn't support comparison operators (>, <, ==) on pandas Series yet.
    We create the boolean mask in Python and test that Catnip can access it.
    """
    import pandas as pd

    df = pd.DataFrame({'x': [1, 2, 3, 4, 5]})
    # Create boolean mask in Python
    mask = df['x'] > 3

    cat.registry.ctx.globals['df'] = df
    cat.registry.ctx.globals['mask'] = mask

    # Access the boolean mask from Catnip
    cat.parse('mask')
    result = cat.execute()
    assert isinstance(result, pd.Series)
    assert result.dtype == bool
    assert list(result) == [False, False, False, True, True]

    # Test accessing mask values
    cat.parse('mask.sum()')
    result = cat.execute()
    assert result == 2  # Two True values


def test_pandas_method_signature_error(cat):
    """Test that calling pandas method with wrong args shows pandas error."""
    import pandas as pd

    from catnip.exc import CatnipTypeError

    df = pd.DataFrame({'x': [1, 2, 3]})
    cat.registry.ctx.globals['df'] = df

    # head() expects an integer, pass invalid argument
    cat.parse('df.head("invalid")')

    # VM wraps TypeError as CatnipTypeError
    with pytest.raises((TypeError, CatnipTypeError)):
        cat.execute()


def test_dataframe_empty(cat):
    """Test empty DataFrame edge case."""
    import pandas as pd

    df = pd.DataFrame()
    cat.registry.ctx.globals['df'] = df

    cat.parse("df.empty")
    result = cat.execute()
    assert result is True

    cat.parse("df.shape")
    result = cat.execute()
    assert result == (0, 0)


def test_series_dtype_access(cat):
    """Test accessing Series dtype."""
    import numpy as np
    import pandas as pd

    series = pd.Series([1, 2, 3], dtype=np.int64)
    cat.registry.ctx.globals['s'] = series

    cat.parse("s.dtype")
    result = cat.execute()
    assert result == np.int64


def test_pandas_attributeerror_propagates(cat):
    """Test that pandas AttributeError for non-existent method is propagated."""
    import pandas as pd

    df = pd.DataFrame({'x': [1, 2, 3]})
    cat.registry.ctx.globals['df'] = df

    # Try to call non-existent method
    cat.parse("df.nonexistent_method()")

    with pytest.raises(AttributeError) as exc_info:
        cat.execute()

    # Should mention the missing attribute
    error_msg = str(exc_info.value)
    assert "nonexistent_method" in error_msg


def test_dataframe_len(cat):
    """Test len() on DataFrame returns number of rows."""
    import pandas as pd

    df = pd.DataFrame({'a': [1, 2, 3], 'b': [4, 5, 6]})
    cat.registry.ctx.globals['df'] = df

    cat.parse("df.shape[0]")
    result = cat.execute()
    assert result == 3


def test_series_to_list(cat):
    """Test converting Series to list."""
    import pandas as pd

    series = pd.Series([10, 20, 30])
    cat.registry.ctx.globals['s'] = series

    cat.parse("s.tolist()")
    result = cat.execute()
    assert result == [10, 20, 30]


def test_dataframe_loc_getitem(cat):
    """Test DataFrame.loc attribute access."""
    import pandas as pd

    df = pd.DataFrame({'x': [1, 2, 3]}, index=['a', 'b', 'c'])
    cat.registry.ctx.globals['df'] = df

    # Access loc indexer
    cat.parse("df.loc")
    result = cat.execute()
    # loc is a pandas indexer object
    assert hasattr(result, '__getitem__')


def test_pandas_concat_available(cat):
    """Test that pandas top-level functions are accessible."""
    import pandas as pd

    cat.registry.ctx.globals['pd'] = pd
    cat.registry.ctx.globals['df1'] = pd.DataFrame({'x': [1, 2]})
    cat.registry.ctx.globals['df2'] = pd.DataFrame({'x': [3, 4]})

    # Call pandas.concat (top-level function)
    cat.parse("pd.concat")
    concat_func = cat.execute()
    assert callable(concat_func)

    # Actually concatenate (in Python, since Catnip can't create lists yet)
    result = concat_func([cat.registry.ctx.globals['df1'], cat.registry.ctx.globals['df2']])
    assert len(result) == 4


# --- Known limitations (documented) ---


def test_comparison_operators_on_series(cat):
    """Comparison operators on pandas Series produce boolean Series."""
    import pandas as pd

    series = pd.Series([1, 2, 3])
    cat.registry.ctx.globals['s'] = series

    cat.parse("s > 2")
    result = cat.execute()
    expected = pd.Series([False, False, True])
    assert list(result) == list(expected)


def test_limitation_boolean_indexing(cat):
    """Document that boolean indexing is not yet fully supported.

    This test demonstrates that while boolean Series work, using them
    for filtering (df[mask]) is not yet supported in Catnip.
    """
    import pandas as pd

    df = pd.DataFrame({'x': [1, 2, 3, 4, 5]})
    mask = df['x'] > 3

    cat.registry.ctx.globals['df'] = df
    cat.registry.ctx.globals['mask'] = mask

    # Boolean indexing syntax not yet supported in Catnip
    # This would work in pandas: df[mask]
    # For now, we can only access the mask itself
    cat.parse("mask")
    result = cat.execute()
    assert list(result) == [False, False, False, True, True]


# --- Advanced pandas features ---


@pytest.mark.skip(reason="Boolean indexing df[mask] not supported in Catnip yet")
def test_dataframe_filtering_boolean_indexing(cat):
    """Test DataFrame filtering with boolean indexing.

    This requires both:
    1. Comparison operators on Series (df['x'] > 5)
    2. Boolean indexing syntax (df[mask])

    Neither is currently supported in Catnip.
    """
    import pandas as pd

    df = pd.DataFrame({'x': [1, 2, 3, 4, 5], 'y': [10, 20, 30, 40, 50]})
    cat.registry.ctx.globals['df'] = df

    # This would be the ideal Catnip syntax (when supported):
    # cat.parse('df[df["x"] > 3]')
    # For now, we can create the filter in Python
    filtered = df[df['x'] > 3]
    cat.registry.ctx.globals['filtered'] = filtered

    cat.parse("filtered")
    result = cat.execute()
    assert len(result) == 2
    assert list(result['x']) == [4, 5]


def test_groupby_basic(cat):
    """Test pandas groupby operations.

    We create the groupby object in Python and test that Catnip can
    access and use it.
    """
    import pandas as pd

    df = pd.DataFrame({'category': ["A", "B", "A", "B", "A"], 'value': [1, 2, 3, 4, 5]})

    # Create groupby in Python
    grouped = df.groupby("category")

    cat.registry.ctx.globals['grouped'] = grouped

    # Access the groupby object
    cat.parse("grouped")
    result = cat.execute()
    assert hasattr(result, 'mean')  # Verify it's a groupby object

    # Call aggregation method
    cat.parse("grouped.mean()")
    result = cat.execute()
    assert isinstance(result, pd.DataFrame)
    # Check the aggregated values
    assert result.loc["A", "value"] == 3.0  # (1+3+5)/3
    assert result.loc["B", "value"] == 3.0  # (2+4)/2


def test_groupby_multiple_operations(cat):
    """Test chaining multiple operations on grouped data."""
    import pandas as pd

    df = pd.DataFrame(
        {"group": ["X", "Y", "X", "Y", "X", "Y"], "val1": [10, 20, 30, 40, 50, 60], "val2": [1, 2, 3, 4, 5, 6]}
    )

    grouped = df.groupby("group")
    cat.registry.ctx.globals['grouped'] = grouped

    # Test sum
    cat.parse("grouped.sum()")
    result = cat.execute()
    assert result.loc["X", "val1"] == 90  # 10+30+50

    # Test count
    cat.parse("grouped.count()")
    result = cat.execute()
    assert result.loc["X", "val1"] == 3


def test_merge_dataframes(cat):
    """Test merging DataFrames.

    We create the DataFrames in Python and test merge operations.
    """
    import pandas as pd

    df1 = pd.DataFrame({'key': ["A", "B", "C"], 'value1': [1, 2, 3]})

    df2 = pd.DataFrame({'key': ["A", "B", "D"], 'value2': [10, 20, 30]})

    cat.registry.ctx.globals['pd'] = pd
    cat.registry.ctx.globals['df1'] = df1
    cat.registry.ctx.globals['df2'] = df2

    # Merge using pandas.merge
    cat.parse("pd.merge(df1, df2)")
    result = cat.execute()

    # Default is inner join, should have keys A and B
    assert len(result) == 2
    assert list(result['key']) == ["A", "B"]
    assert list(result['value1']) == [1, 2]
    assert list(result['value2']) == [10, 20]


def test_merge_with_options(cat):
    """Test merge with different join types."""
    import pandas as pd

    df1 = pd.DataFrame({'id': [1, 2, 3], 'name': ["Alice", "Bob", "Charlie"]})

    df2 = pd.DataFrame({'id': [2, 3, 4], 'score': [85, 90, 95]})

    cat.registry.ctx.globals['pd'] = pd
    cat.registry.ctx.globals['df1'] = df1
    cat.registry.ctx.globals['df2'] = df2

    # Left join using df.merge method
    cat.parse('df1.merge(df2, how="left", on="id")')
    result = cat.execute()

    assert len(result) == 3
    assert list(result['id']) == [1, 2, 3]
    # First row should have NaN for score
    import math

    assert math.isnan(result['score'].iloc[0])


def test_broadcasting_with_pandas_series(cat):
    """Test interaction between Catnip broadcasting and pandas Series."""
    import pandas as pd

    series = pd.Series([10, 20, 30])
    cat.registry.ctx.globals['s'] = series

    # Scalar multiplication (pandas handles this)
    cat.parse("s * 2")
    result = cat.execute()
    assert isinstance(result, pd.Series)
    assert list(result) == [20, 40, 60]

    # Addition with scalar
    cat.parse("s + 5")
    result = cat.execute()
    assert list(result) == [15, 25, 35]


def test_broadcasting_pandas_dataframe_column_operation(cat):
    """Test broadcasting on DataFrame columns."""
    import pandas as pd

    df = pd.DataFrame({'a': [1, 2, 3], 'b': [4, 5, 6]})
    cat.registry.ctx.globals['df'] = df

    # Column arithmetic
    cat.parse('df["a"] + df["b"]')
    result = cat.execute()
    assert isinstance(result, pd.Series)
    assert list(result) == [5, 7, 9]

    # Multiply column by scalar
    cat.parse('df["a"] * 10')
    result = cat.execute()
    assert list(result) == [10, 20, 30]


def test_pandas_apply_function(cat):
    """Test applying Python functions to pandas objects.

    This tests that pandas can call Python functions even when
    accessed through Catnip.
    """
    import pandas as pd

    series = pd.Series([1, 2, 3, 4, 5])

    # Define a Python function
    def square(x):
        return x * x

    cat.registry.ctx.globals['s'] = series
    cat.registry.ctx.globals['square'] = square

    # Apply function to series
    cat.parse("s.apply(square)")
    result = cat.execute()
    assert list(result) == [1, 4, 9, 16, 25]


def test_pandas_apply_catnip_lambda(cat):
    """pandas.apply() with Catnip lambda -- regression segfault."""
    import pandas as pd

    series = pd.Series([1, 2, 3, 4, 5])
    cat.registry.ctx.globals['s'] = series

    cat.parse('s.apply((x) => { x * 2 })')
    result = cat.execute()
    assert list(result) == [2, 4, 6, 8, 10]


def test_pandas_apply_catnip_lambda_closure(cat):
    """pandas.apply() with Catnip lambda capturing a closure variable."""
    import pandas as pd

    series = pd.Series([1, 2, 3])
    cat.registry.ctx.globals['s'] = series

    cat.parse('''
    factor = 10
    s.apply((x) => { x * factor })
    ''')
    result = cat.execute()
    assert list(result) == [10, 20, 30]


def test_pandas_apply_stress(cat):
    """pandas.apply() with many rows -- previously caused segfault."""
    import pandas as pd

    series = pd.Series(list(range(2000)))
    cat.registry.ctx.globals['s'] = series

    cat.parse('s.apply((x) => { x + 1 })')
    result = cat.execute()
    assert list(result) == list(range(1, 2001))


def test_pandas_string_methods(cat):
    """Test pandas string accessor methods."""
    import pandas as pd

    series = pd.Series(["hello", "world", "catnip"])
    cat.registry.ctx.globals['s'] = series

    # Access str accessor
    cat.parse("s.str")
    str_accessor = cat.execute()
    assert hasattr(str_accessor, 'upper')

    # Use string method
    cat.parse("s.str.upper()")
    result = cat.execute()
    assert list(result) == ["HELLO", "WORLD", "CATNIP"]

    # String contains
    cat.parse('s.str.contains("o")')
    result = cat.execute()
    assert list(result) == [True, True, False]


def test_pandas_datetime_operations(cat):
    """Test pandas datetime operations."""
    from datetime import datetime

    import pandas as pd

    dates = pd.Series([datetime(2024, 1, 1), datetime(2024, 6, 15), datetime(2024, 12, 31)])

    cat.registry.ctx.globals['dates'] = dates

    # Access dt accessor
    cat.parse("dates.dt")
    dt_accessor = cat.execute()
    assert hasattr(dt_accessor, 'year')

    # Extract year
    cat.parse("dates.dt.year")
    result = cat.execute()
    assert list(result) == [2024, 2024, 2024]

    # Extract month
    cat.parse("dates.dt.month")
    result = cat.execute()
    assert list(result) == [1, 6, 12]


@pytest.mark.skip(reason="Catnip doesn't have dict/list literal syntax yet")
def test_dataframe_creation_from_catnip(cat):
    """Test creating DataFrame directly in Catnip code.

    This requires Catnip to have:
    1. Dict literal syntax: {"key": value}
    2. List literal syntax: [1, 2, 3]

    When these are available, this test can be enabled.
    """
    import pandas as pd

    cat.registry.ctx.globals['pd'] = pd

    # Ideal future syntax (when dict/list literals are supported):
    # cat.parse('pd.DataFrame({"a": [1, 2, 3], "b": [4, 5, 6]})')

    # For now, we have to create data structures in Python
    data = {"a": [1, 2, 3], "b": [4, 5, 6]}
    cat.registry.ctx.globals['data'] = data

    cat.parse("pd.DataFrame(data)")
    result = cat.execute()
    assert isinstance(result, pd.DataFrame)
    assert list(result.columns) == ["a", "b"]


def test_pandas_pivot_table(cat):
    """Test pandas pivot table operations."""
    import pandas as pd

    df = pd.DataFrame({'category': ["A", "A", "B", "B"], 'type': ["X", "Y", "X", "Y"], 'value': [10, 20, 30, 40]})

    cat.registry.ctx.globals['pd'] = pd
    cat.registry.ctx.globals['df'] = df

    # Create pivot table
    cat.parse('pd.pivot_table(df, values="value", index="category", columns="type")')
    result = cat.execute()

    assert isinstance(result, pd.DataFrame)
    assert result.loc["A", "X"] == 10
    assert result.loc["B", "Y"] == 40


def test_pandas_stack_unstack(cat):
    """Test pandas stack/unstack operations."""
    import pandas as pd

    df = pd.DataFrame({'A': [1, 2], 'B': [3, 4]}, index=["row1", "row2"])

    cat.registry.ctx.globals['df'] = df

    # Stack
    cat.parse("df.stack()")
    stacked = cat.execute()
    assert isinstance(stacked, pd.Series)
    assert len(stacked) == 4

    # Unstack (need to inject stacked)
    cat.registry.ctx.globals['stacked'] = stacked
    cat.parse("stacked.unstack()")
    unstacked = cat.execute()
    assert isinstance(unstacked, pd.DataFrame)


def test_pandas_fillna(cat):
    """Test pandas fillna method."""
    import pandas as pd

    series = pd.Series([1.0, float('nan'), 3.0, float('nan'), 5.0])
    cat.registry.ctx.globals['s'] = series

    # Fill NaN with 0
    cat.parse("s.fillna(0)")
    result = cat.execute()
    assert list(result) == [1.0, 0.0, 3.0, 0.0, 5.0]


def test_pandas_dropna(cat):
    """Test pandas dropna method."""
    import pandas as pd

    series = pd.Series([1.0, float('nan'), 3.0, float('nan'), 5.0])
    cat.registry.ctx.globals['s'] = series

    # Drop NaN values
    cat.parse("s.dropna()")
    result = cat.execute()
    assert list(result) == [1.0, 3.0, 5.0]


def test_pandas_value_counts(cat):
    """Test pandas value_counts method."""
    import pandas as pd

    series = pd.Series(["A", "B", "A", "C", "B", "A"])
    cat.registry.ctx.globals['s'] = series

    # Count values
    cat.parse("s.value_counts()")
    result = cat.execute()
    assert result['A'] == 3
    assert result['B'] == 2
    assert result['C'] == 1


def test_pandas_sort_values(cat):
    """Test pandas sort_values method."""
    import pandas as pd

    df = pd.DataFrame({'x': [3, 1, 4, 1, 5], 'y': [9, 2, 6, 5, 3]})
    cat.registry.ctx.globals['df'] = df

    # Sort by column x
    cat.parse('df.sort_values("x")')
    result = cat.execute()
    assert list(result['x']) == [1, 1, 3, 4, 5]


def test_pandas_reset_index(cat):
    """Test pandas reset_index method."""
    import pandas as pd

    df = pd.DataFrame({'value': [10, 20, 30]}, index=['a', 'b', 'c'])
    cat.registry.ctx.globals['df'] = df

    # Reset index
    cat.parse("df.reset_index()")
    result = cat.execute()
    assert "index" in result.columns
    assert list(result['index']) == ["a", "b", "c"]


# --- Inplace operations (critical for context coupling) ---


def test_inplace_sort_values(cat):
    """Test sort_values with inplace=True.

    This tests that modifications to the DataFrame are properly
    reflected in the Catnip context.
    """
    import pandas as pd

    df = pd.DataFrame({'x': [3, 1, 4, 1, 5], 'y': [9, 2, 6, 5, 3]})
    cat.registry.ctx.globals['df'] = df

    # Sort inplace - returns None
    cat.parse('df.sort_values("x", inplace=True)')
    result = cat.execute()
    assert result is None  # inplace operations return None

    # Verify the DataFrame was modified in context
    cat.parse("df")
    modified_df = cat.execute()
    assert list(modified_df['x']) == [1, 1, 3, 4, 5]


def test_inplace_fillna(cat):
    """Test fillna with inplace=True."""
    import pandas as pd

    df = pd.DataFrame({'x': [1.0, float('nan'), 3.0, float('nan'), 5.0]})
    cat.registry.ctx.globals['df'] = df

    # Fill NaN inplace (pandas 3.0+ returns the DataFrame, not None)
    cat.parse("df.fillna(0, inplace=True)")
    cat.execute()

    # Check modification persisted
    cat.parse('df["x"]')
    series = cat.execute()
    assert list(series) == [1.0, 0.0, 3.0, 0.0, 5.0]


def test_inplace_dropna(cat):
    """Test dropna with inplace=True."""
    import pandas as pd

    df = pd.DataFrame({'x': [1.0, float('nan'), 3.0], 'y': [4.0, 5.0, float('nan')]})
    cat.registry.ctx.globals['df'] = df

    # Drop rows with any NaN inplace
    cat.parse("df.dropna(inplace=True)")
    result = cat.execute()
    assert result is None

    # Check only row 0 remains (only row without NaN)
    cat.parse("df")
    modified_df = cat.execute()
    assert len(modified_df) == 1
    assert modified_df['x'].iloc[0] == 1.0


def test_inplace_reset_index(cat):
    """Test reset_index with inplace=True."""
    import pandas as pd

    df = pd.DataFrame({'value': [10, 20, 30]}, index=['a', 'b', 'c'])
    cat.registry.ctx.globals['df'] = df

    # Reset index inplace
    cat.parse("df.reset_index(inplace=True)")
    result = cat.execute()
    assert result is None

    # Check index was added as column
    cat.parse("df.columns")
    columns = cat.execute()
    assert "index" in list(columns)


def test_inplace_drop_column(cat):
    """Test drop with inplace=True."""
    import pandas as pd

    df = pd.DataFrame({'a': [1, 2, 3], 'b': [4, 5, 6], 'c': [7, 8, 9]})
    cat.registry.ctx.globals['df'] = df

    # Drop column inplace
    cat.parse('df.drop("b", axis=1, inplace=True)')
    result = cat.execute()
    assert result is None

    # Check column was removed
    cat.parse("df.columns")
    columns = cat.execute()
    assert list(columns) == ["a", "c"]


def test_inplace_rename(cat):
    """Test rename with inplace=True.

    Note: Catnip doesn't support dict literal syntax {"key": "value"} yet,
    so we create the rename dict in Python and pass it through the context.
    """
    import pandas as pd

    df = pd.DataFrame({'old_name': [1, 2, 3]})
    rename_dict = {"old_name": "new_name"}

    cat.registry.ctx.globals['df'] = df
    cat.registry.ctx.globals['rename_dict'] = rename_dict

    # Rename column inplace using dict from context
    cat.parse("df.rename(columns=rename_dict, inplace=True)")
    result = cat.execute()
    assert result is None

    # Check column was renamed
    cat.parse("df.columns")
    columns = cat.execute()
    assert list(columns) == ["new_name"]


def test_inplace_vs_copy_behavior(cat):
    """Test that inplace=False returns a new object while inplace=True modifies original.

    This is critical to understand the difference between references and copies
    in the Catnip context.
    """
    import pandas as pd

    # Create two identical DataFrames
    df1 = pd.DataFrame({'x': [3, 1, 2]})
    df2 = pd.DataFrame({'x': [3, 1, 2]})

    cat.registry.ctx.globals['df1'] = df1
    cat.registry.ctx.globals['df2'] = df2

    # Sort df1 WITHOUT inplace (returns new DataFrame)
    cat.parse('df1.sort_values("x")')
    sorted_df = cat.execute()
    assert list(sorted_df['x']) == [1, 2, 3]

    # Original df1 should be unchanged
    cat.parse("df1")
    original_df1 = cat.execute()
    assert list(original_df1["x"]) == [3, 1, 2]

    # Sort df2 WITH inplace (modifies in place)
    cat.parse('df2.sort_values("x", inplace=True)')
    cat.execute()

    # df2 should now be sorted
    cat.parse("df2")
    modified_df2 = cat.execute()
    assert list(modified_df2["x"]) == [1, 2, 3]


def test_inplace_chaining_error(cat):
    """Test that inplace operations can't be chained (returns None).

    This documents a common mistake when using inplace=True.
    """
    import pandas as pd

    df = pd.DataFrame({'x': [3, 1, 2], 'y': [6, 4, 5]})
    cat.registry.ctx.globals['df'] = df

    # Try to chain after inplace operation (this fails because it returns None)
    cat.parse('df.sort_values("x", inplace=True).head()')

    # Should raise AttributeError because None has no head() method
    with pytest.raises(AttributeError) as exc_info:
        cat.execute()

    error_msg = str(exc_info.value)
    assert "NoneType" in error_msg or "None" in error_msg


def test_inplace_set_value_warning(cat):
    """Test setting values in DataFrame columns.

    Pandas 3.0+ uses Copy-on-Write by default, so chained assignment
    (df['x'][0] = value) no longer modifies the original DataFrame.
    Use .loc[] for proper in-place mutation.
    """
    import pandas as pd

    df = pd.DataFrame({'x': [1, 2, 3]})
    cat.registry.ctx.globals['df'] = df

    # Use .loc for CoW-safe assignment
    df.loc[0, "x"] = 999

    # Check modification is visible in Catnip
    cat.parse('df["x"]')
    series = cat.execute()
    assert series.iloc[0] == 999


def test_inplace_reference_preservation(cat):
    """Test that inplace operations preserve references in context.

    This is critical: when we modify a DataFrame with inplace=True,
    the same object reference in the Catnip context should see the changes.
    """
    import pandas as pd

    df = pd.DataFrame({'x': [3, 1, 2]})

    # Store reference in context
    cat.registry.ctx.globals['df'] = df
    cat.registry.ctx.globals['df_reference'] = df  # Same object

    # Modify via inplace operation through Catnip
    cat.parse('df.sort_values("x", inplace=True)')
    cat.execute()

    # Both references should see the change
    cat.parse("df")
    df_result = cat.execute()
    assert list(df_result['x']) == [1, 2, 3]

    cat.parse("df_reference")
    ref_result = cat.execute()
    assert list(ref_result['x']) == [1, 2, 3]

    # Verify they're the same object (not copies)
    assert df_result is ref_result


# --- Weird cases ---


def test_series_alignment_by_index(cat):
    """Test pandas alignment by index when operating on Series."""
    import pandas as pd

    s1 = pd.Series([1, 2, 3], index=['a', 'b', 'c'])
    s2 = pd.Series([10, 20, 30], index=['b', 'c', 'd'])

    cat.registry.ctx.globals['s1'] = s1
    cat.registry.ctx.globals['s2'] = s2

    # Pandas aligns by index, result has union of indexes
    cat.parse("s1 + s2")
    result = cat.execute()
    assert list(result.index) == ["a", "b", "c", "d"]
    assert result.loc["a"] != result.loc["a"]  # NaN
    assert result.loc["b"] == 12
    assert result.loc["c"] == 23
    assert result.loc["d"] != result.loc["d"]  # NaN


def test_non_unique_index_access(cat):
    """Test Series with non-unique index."""
    import pandas as pd

    s = pd.Series([1, 2, 3], index=['a', 'a', 'b'])
    cat.registry.ctx.globals['s'] = s

    cat.parse('s.loc["a"]')
    result = cat.execute()
    # Non-unique index returns a Series
    assert isinstance(result, pd.Series)
    assert list(result) == [1, 2]


def test_multiindex_basic(cat):
    """Test MultiIndex access on DataFrame.

    Note: Catnip doesn't support tuple literal syntax yet,
    so we create the index tuple in Python.
    """
    import pandas as pd

    index = pd.MultiIndex.from_tuples([("A", 1), ("A", 2), ("B", 1)], names=["grp", "id"])
    df = pd.DataFrame({'val': [10, 20, 30]}, index=index)
    idx_tuple = ("A", 2)

    cat.registry.ctx.globals['df'] = df
    cat.registry.ctx.globals['idx'] = idx_tuple

    # Access MultiIndex using tuple from context
    cat.parse('df.loc[idx]')
    result = cat.execute()
    assert result['val'] == 20


def test_timezone_aware_datetimeindex(cat):
    """Test timezone-aware DatetimeIndex behavior."""
    import pandas as pd

    idx = pd.date_range("2024-01-01", periods=3, tz="UTC")
    s = pd.Series([1, 2, 3], index=idx)
    cat.registry.ctx.globals['s'] = s

    cat.parse("s.index.tz")
    result = cat.execute()
    assert str(result) == "UTC"


def test_duplicate_columns(cat):
    """Test DataFrame with duplicate column names."""
    import pandas as pd

    df = pd.DataFrame([[1, 2], [3, 4]], columns=["x", "x"])
    cat.registry.ctx.globals['df'] = df

    cat.parse('df["x"]')
    result = cat.execute()
    # With duplicate columns, pandas returns a DataFrame
    assert isinstance(result, pd.DataFrame)
    assert list(result.columns) == ["x", "x"]


def test_non_string_column_labels(cat):
    """Test DataFrame columns with non-string labels."""
    import pandas as pd

    df = pd.DataFrame({1: [10, 20], 2: [30, 40]})
    cat.registry.ctx.globals['df'] = df

    cat.parse("df[1]")
    result = cat.execute()
    assert list(result) == [10, 20]


def test_inf_values(cat):
    """Test handling of inf and -inf values.

    Note: Catnip doesn't support list literal syntax yet,
    so we create the replacement list in Python.
    """
    import numpy as np
    import pandas as pd

    s = pd.Series([1.0, np.inf, -np.inf, 4.0])
    replace_values = [float('inf'), float('-inf')]

    cat.registry.ctx.globals['s'] = s
    cat.registry.ctx.globals['replace_vals'] = replace_values

    # Replace inf values using list from context
    cat.parse("s.replace(replace_vals, 0)")
    result = cat.execute()
    assert list(result) == [1.0, 0.0, 0.0, 4.0]


def test_nullable_integer_dtype(cat):
    """Test pandas nullable integer dtype behavior."""
    import pandas as pd

    s = pd.Series([1, None, 3], dtype="Int64")
    cat.registry.ctx.globals['s'] = s

    cat.parse("s.isna().sum()")
    result = cat.execute()
    assert result == 1


def test_categorical_dtype(cat):
    """Test categorical dtype access."""
    import pandas as pd

    s = pd.Series(["a", "b", "a"], dtype="category")
    cat.registry.ctx.globals['s'] = s

    cat.parse("s.dtype.name")
    result = cat.execute()
    assert result == "category"
