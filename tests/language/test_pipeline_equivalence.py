# FILE: tests/language/test_pipeline_equivalence.py
"""Pipeline equivalence tests: Catnip (full) vs Pipeline (REPL).

Ensures both pipelines produce identical results for the same source code.
"""

import pytest
from catnip._rs import Pipeline

from catnip import Catnip


def catnip_eval(code):
    c = Catnip()
    c.parse(code)
    return c.execute()


def standalone_eval(code):
    p = Pipeline()
    return p.execute(code)


# ---------------------------------------------------------------------------
# Arithmetic
# ---------------------------------------------------------------------------

ARITHMETIC_CASES = [
    ("2 + 3", 5),
    ("10 - 4", 6),
    ("3 * 7", 21),
    ("15 / 3", 5.0),
    ("17 // 5", 3),
    ("17 % 5", 2),
    ("-7 % 3", 2),  # Python floored modulo (negative dividend)
    ("-7.0 % 3.0", 2.0),  # float floored modulo
    ("7.0 % -3.0", -2.0),  # float floored modulo (negative divisor)
    ("2 ** 10", 1024),
    ("-42", -42),
    ("+7", 7),
    ("1.5 + 2.5", 4.0),
    ("0.1 + 0.2", 0.1 + 0.2),
    ("2 ** 100", 2**100),  # BigInt
]


@pytest.mark.parametrize("code, expected", ARITHMETIC_CASES, ids=[c for c, _ in ARITHMETIC_CASES])
def test_arithmetic(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Comparison & Boolean
# ---------------------------------------------------------------------------

COMPARISON_CASES = [
    ("3 > 2", True),
    ("3 < 2", False),
    ("3 >= 3", True),
    ("3 <= 2", False),
    ("3 == 3", True),
    ("3 != 4", True),
    ("True and False", False),
    ("True or False", True),
    ("not True", False),
    ("not False", True),
]


@pytest.mark.parametrize("code, expected", COMPARISON_CASES, ids=[c for c, _ in COMPARISON_CASES])
def test_comparison(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Bitwise
# ---------------------------------------------------------------------------

BITWISE_CASES = [
    ("5 & 3", 5 & 3),
    ("5 | 3", 5 | 3),
    ("5 ^ 3", 5 ^ 3),
    ("~0", ~0),
    ("1 << 4", 1 << 4),
    ("16 >> 2", 16 >> 2),
]


@pytest.mark.parametrize("code, expected", BITWISE_CASES, ids=[c for c, _ in BITWISE_CASES])
def test_bitwise(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Strings & f-strings
# ---------------------------------------------------------------------------

STRING_CASES = [
    ('"hello"', "hello"),
    ('"hello" + " " + "world"', "hello world"),
    ('x = 42\nf"val={x}"', "val=42"),
    ('f"{1 + 2}"', "3"),
]


@pytest.mark.parametrize("code, expected", STRING_CASES, ids=[c for c, _ in STRING_CASES])
def test_strings(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Collections
# ---------------------------------------------------------------------------

COLLECTION_CASES = [
    ("list(1, 2, 3)", [1, 2, 3]),
    ("tuple(1, 2, 3)", (1, 2, 3)),
    ("set(1, 2, 3)", {1, 2, 3}),
    ("dict(a=1, b=2)", {"a": 1, "b": 2}),
    ("list()", []),
    ("tuple()", ()),
]


@pytest.mark.parametrize("code, expected", COLLECTION_CASES, ids=[c for c, _ in COLLECTION_CASES])
def test_collections(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Variables & assignment
# ---------------------------------------------------------------------------

VARIABLE_CASES = [
    ("x = 10\nx", 10),
    ("x = 1\ny = 2\nx + y", 3),
    ("x = 5\nx = x + 1\nx", 6),
]


@pytest.mark.parametrize("code, expected", VARIABLE_CASES, ids=[c for c, _ in VARIABLE_CASES])
def test_variables(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Control flow
# ---------------------------------------------------------------------------

CONTROL_FLOW_CASES = [
    ("if True { 1 } else { 2 }", 1),
    ("if False { 1 } else { 2 }", 2),
    ("x = 0\nwhile x < 5 { x = x + 1 }\nx", 5),
    ("s = 0\nfor i in list(1, 2, 3) { s = s + i }\ns", 6),
    ("{ x = 10\n x + 1 }", 11),
]


@pytest.mark.parametrize("code, expected", CONTROL_FLOW_CASES, ids=[c for c, _ in CONTROL_FLOW_CASES])
def test_control_flow(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Functions & lambdas
# ---------------------------------------------------------------------------

FUNCTION_CASES = [
    ("f = (x) => { x * 2 }\nf(5)", 10),
    ("f = (x, y) => { x + y }\nf(3, 4)", 7),
    ("f = (x=10) => { x }\nf()", 10),
    ("f = () => { 42 }\nf()", 42),
    # closure
    ("x = 10\nf = () => { x }\nf()", 10),
    # higher-order
    ("apply = (f, x) => { f(x) }\ndouble = (x) => { x * 2 }\napply(double, 5)", 10),
]


@pytest.mark.parametrize("code, expected", FUNCTION_CASES, ids=[c for c, _ in FUNCTION_CASES])
def test_functions(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Recursion
# ---------------------------------------------------------------------------

RECURSION_CASES = [
    ("fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }\nfact(10)", 3628800),
    # Fibonacci
    ("fib = (n) => { if n <= 1 { n } else { fib(n-1) + fib(n-2) } }\nfib(10)", 55),
]


@pytest.mark.parametrize("code, expected", RECURSION_CASES, ids=[c for c, _ in RECURSION_CASES])
def test_recursion(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Pattern matching
# ---------------------------------------------------------------------------

PATTERN_CASES = [
    ("match-literal-hit", 'match 1 {\n  1 => { "one" }\n  2 => { "two" }\n  _ => { "other" }\n}', "one"),
    ("match-literal-wildcard", 'match 3 {\n  1 => { "one" }\n  2 => { "two" }\n  _ => { "other" }\n}', "other"),
    ("match-capture", "match 5 {\n  x => { x * 2 }\n}", 10),
    ("match-guard", 'match 42 {\n  x if x > 10 => { "big" }\n  _ => { "small" }\n}', "big"),
    ("match-or-pattern", 'match 3 {\n  1 | 2 | 3 => { "low" }\n  _ => { "high" }\n}', "low"),
]


@pytest.mark.parametrize("id, code, expected", PATTERN_CASES, ids=[c[0] for c in PATTERN_CASES])
def test_pattern_matching(id, code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Access (getitem, getattr)
# ---------------------------------------------------------------------------

ACCESS_CASES = [
    ("x = list(10, 20, 30)\nx[1]", 20),
    ("x = dict(a=1, b=2)\nx['a']", 1),
    ("x = list(1, 2, 3)\nlen(x)", 3),
]


@pytest.mark.parametrize("code, expected", ACCESS_CASES, ids=[c for c, _ in ACCESS_CASES])
def test_access(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Builtins
# ---------------------------------------------------------------------------

BUILTIN_CASES = [
    ("abs(-5)", 5),
    ("max(1, 5, 3)", 5),
    ("min(1, 5, 3)", 1),
    ("len(list(1, 2, 3))", 3),
    ("len('hello')", 5),
    ("int(3.7)", 3),
    ("float(42)", 42.0),
    ("str(42)", "42"),
    ("bool(1)", True),
    ("bool(0)", False),
]


@pytest.mark.parametrize("code, expected", BUILTIN_CASES, ids=[c for c, _ in BUILTIN_CASES])
def test_builtins(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# in / not in (requires fixed opcodes)
# ---------------------------------------------------------------------------

IN_CASES = [
    ("2 in list(1, 2, 3)", True),
    ("5 in list(1, 2, 3)", False),
    ("2 not in list(1, 2, 3)", False),
    ("5 not in list(1, 2, 3)", True),
    ('"a" in dict(a=1, b=2)', True),
    ('"cat" in "catnip"', True),
]


@pytest.mark.parametrize("code, expected", IN_CASES, ids=[c for c, _ in IN_CASES])
def test_in_operator(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# is / is not (requires fixed opcodes)
# ---------------------------------------------------------------------------

IS_CASES = [
    ("None is None", True),
    ("True is True", True),
    ("None is not True", True),
    ("True is not True", False),
    ("1 is not None", True),
]


@pytest.mark.parametrize("code, expected", IS_CASES, ids=[c for c, _ in IS_CASES])
def test_is_operator(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Broadcast
# ---------------------------------------------------------------------------

BROADCAST_CASES = [
    ("list(1, 2, 3).[+ 1]", [2, 3, 4]),
    ("list(1, 2, 3).[* 2]", [2, 4, 6]),
]


@pytest.mark.parametrize("code, expected", BROADCAST_CASES, ids=[c for c, _ in BROADCAST_CASES])
def test_broadcast(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Structs (xfail if standalone doesn't support them yet)
# ---------------------------------------------------------------------------

STRUCT_CASES = [
    ("struct Point { x; y; }\np = Point(1, 2)\np.x", 1),
    ("struct Point { x; y; }\np = Point(1, 2)\np.y", 2),
    # field then method
    ("struct S { x; f(self) => { self.x } }\nS(42).f()", 42),
    # separator (no trailing ;)
    ("struct S { x\nf(self) => { self.x } }\nS(42).f()", 42),
]


@pytest.mark.parametrize("code, expected", STRUCT_CASES, ids=[c for c, _ in STRUCT_CASES])
def test_structs(code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Null coalesce (??)
# ---------------------------------------------------------------------------

NULL_COALESCE_CASES = [
    ("null-to-default", "x = None; x ?? 42", 42),
    ("non-null-kept", "x = 10; x ?? 42", 10),
    ("false-kept", "x = False; x ?? True", False),
    ("zero-kept", "x = 0; x ?? 99", 0),
    ("chained", "a = None; b = None; a ?? b ?? 7", 7),
]


@pytest.mark.parametrize("id, code, expected", NULL_COALESCE_CASES, ids=[c[0] for c in NULL_COALESCE_CASES])
def test_null_coalesce(id, code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


# ---------------------------------------------------------------------------
# Import (stdlib)
# ---------------------------------------------------------------------------

IMPORT_CASES = [
    ("import-math", 'x = import("math"); x.sqrt(16)', 4.0),
]

IMPORT_SELECTIVE_CASES = [
    ("import-selective", 'import("math", "pi"); pi > 3', True),
]


@pytest.mark.parametrize("id, code, expected", IMPORT_CASES, ids=[c[0] for c in IMPORT_CASES])
def test_import(id, code, expected):
    assert catnip_eval(code) == expected
    assert standalone_eval(code) == expected


@pytest.mark.parametrize("id, code, expected", IMPORT_SELECTIVE_CASES, ids=[c[0] for c in IMPORT_SELECTIVE_CASES])
def test_import_selective(id, code, expected):
    # Selective imports inject names into globals; VM mode doesn't sync them back
    c = Catnip(vm_mode='off')
    c.parse(code)
    assert c.execute() == expected


# ---------------------------------------------------------------------------
# Error cases: both pipelines should fail
# ---------------------------------------------------------------------------

ERROR_CASES = [
    "1 + + +",
    "if { }",
    "unknown_var_xyz",
]


@pytest.mark.parametrize("code", ERROR_CASES, ids=ERROR_CASES)
def test_both_fail(code):
    catnip_failed = False
    try:
        catnip_eval(code)
    except Exception:
        catnip_failed = True

    standalone_failed = False
    try:
        standalone_eval(code)
    except Exception:
        standalone_failed = True

    assert (
        catnip_failed == standalone_failed
    ), f"Divergence on error case: catnip_failed={catnip_failed}, standalone_failed={standalone_failed}"
