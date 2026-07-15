# FILE: tests/language/test_unions.py
"""Tests for tagged union (ADT) declarations and pattern matching.

The union runtime is wired through both the AST executor (`op_union` in
the registry) and the VM bytecode path (`MakeUnion`). `_ast_cat()`
defaults to the VM mode now; specific edge cases that exercise the AST
executor explicitly use `vm_mode='off'`.
"""

import pytest

from catnip import Catnip


def _ast_cat():
    # Default VM mode: MakeUnion bytecode is wired through the PyO3 VM.
    # Name kept for historical reasons; tests run on both executors
    # transparently since MakeUnion delegates to the shared
    # build_union_type helper used by op_union.
    return Catnip()


class TestUnionDeclaration:
    """Union declaration and variant lookup."""

    def test_nullary_only_declares(self):
        cat = _ast_cat()
        cat.parse("union Color { red; green; blue }\nColor")
        result = cat.execute()
        assert result is not None

    def test_with_payload_declares(self):
        cat = _ast_cat()
        cat.parse("union Option { Some(value); None; }\nOption")
        result = cat.execute()
        assert result is not None

    def test_generics_parse(self):
        cat = _ast_cat()
        cat.parse("union Result[T, E] { Ok(value: T); Err(error: E); }\nResult")
        result = cat.execute()
        assert result is not None

    def test_unknown_variant_raises(self):
        cat = _ast_cat()
        cat.parse("union Option { Some(value); None; }\nOption.Maybe")
        with pytest.raises(Exception):
            cat.execute()

    def test_typeof_nullary_variant_is_union_name(self):
        # Nullary union variants live in the symbol table, not the enum
        # registry; typeof must still report the declaring union name.
        cat = _ast_cat()
        cat.parse("union Color { red; green; blue }\ntypeof(Color.red)")
        assert cat.execute() == "Color"


class TestUnionConstruction:
    """Constructing values of union variants."""

    def test_nullary_singleton_equal(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
Option.None == Option.None
""")
        assert cat.execute() is True

    def test_with_payload_constructs(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
x = Option.Some(42)
x.value
""")
        assert cat.execute() == 42

    def test_distinct_variants_not_equal(self):
        cat = _ast_cat()
        cat.parse("""
union Box { A(v); B(v); }
Box.A(1) == Box.B(1)
""")
        # Different qualified type names -- structurally non-equal.
        assert cat.execute() is False


class TestUnionEquality:
    """Structural equality of variants with payload."""

    def test_same_variant_same_value(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
Option.Some(1) == Option.Some(1)
""")
        assert cat.execute() is True

    def test_same_variant_diff_value(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
Option.Some(1) == Option.Some(2)
""")
        assert cat.execute() is False


class TestUnionPatternMatch:
    """Pattern matching against union variants."""

    def test_match_with_payload(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
opt = Option.Some(42)
match opt {
    Option.Some{value} => { value }
    Option.None => { 0 }
}
""")
        assert cat.execute() == 42

    def test_match_nullary(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
opt = Option.None
match opt {
    Option.Some{value} => { value }
    Option.None => { -1 }
}
""")
        assert cat.execute() == -1

    def test_match_multi_field(self):
        cat = _ast_cat()
        cat.parse("""
union Event {
    Click(x, y);
    KeyPress(code);
    Quit;
}
e = Event.Click(10, 20)
match e {
    Event.Click{x, y} => { x + y }
    Event.KeyPress{code} => { code }
    Event.Quit => { 0 }
}
""")
        assert cat.execute() == 30

    def test_variant_name_not_bound_as_field(self):
        """Regression: `Option.Some{value}` must extract `value`, not bind
        `Some` as a field. This was the P2 review finding."""
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
opt = Option.Some(7)
match opt {
    Option.Some{value} => { value * 2 }
    Option.None => { 0 }
}
""")
        assert cat.execute() == 14


class TestUnionErrors:
    """Error handling for malformed declarations."""

    def test_duplicate_variant_rejected(self):
        cat = _ast_cat()
        with pytest.raises(Exception):
            cat.parse("union Bad { a; a; }")

    def test_duplicate_field_rejected(self):
        cat = _ast_cat()
        with pytest.raises(Exception):
            cat.parse("union Bad { Variant(x, x); }")


class TestPatternStructPickle:
    """PatternStruct must round-trip through pickle including its variant.

    Regression for review: a pickled `Option.Some{value}` pattern was
    losing its `variant` field and reloading as a plain `Option{value}`
    pattern, which then failed to match `Option.Some(...)` instances.
    """

    def test_variant_survives_pickle(self):
        import pickle

        from catnip._rs import PatternStruct

        original = PatternStruct("Option", ["value"], "Some")
        restored = pickle.loads(pickle.dumps(original))
        assert restored.name == "Option"
        assert restored.variant == "Some"
        assert list(restored.fields) == ["value"]
        assert restored == original

    def test_plain_struct_pickle_unchanged(self):
        import pickle

        from catnip._rs import PatternStruct

        original = PatternStruct("Point", ["x", "y"])
        restored = pickle.loads(pickle.dumps(original))
        assert restored.name == "Point"
        assert restored.variant is None
        assert list(restored.fields) == ["x", "y"]
        assert restored == original


class TestUnionStackBalance:
    """Regression: MakeUnion must leave a result on the VM stack.

    The statement-list compiler emits `PopTop` after each non-last
    statement. If MakeUnion produces no value, a declaration followed by
    any expression underflows the VM stack.
    """

    def test_declaration_followed_by_expression(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(v); None; }
Option.Some(1)
""")
        result = cat.execute()
        # Field name is `v` per the declaration above.
        assert result.v == 1

    def test_declaration_followed_by_assignment(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(v); None; }
x = Option.Some(42)
y = 100
y
""")
        assert cat.execute() == 100


class TestUnionTruthiness:
    """All variants -- nullary or with payload -- are truthy."""

    def test_nullary_truthy(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
if Option.None { 1 } else { 0 }
""")
        assert cat.execute() == 1

    def test_with_payload_truthy(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
if Option.Some(0) { 1 } else { 0 }
""")
        assert cat.execute() == 1


class TestUnionGuardsAndOr:
    """Match patterns combining union variants with guards and OR."""

    def test_guard_on_payload(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
opt = Option.Some(10)
match opt {
    Option.Some{value} if value > 5 => { "big" }
    Option.Some{value} => { "small" }
    Option.None => { "none" }
}
""")
        assert cat.execute() == "big"

    def test_guard_filters_to_next_branch(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
opt = Option.Some(3)
match opt {
    Option.Some{value} if value > 5 => { "big" }
    Option.Some{value} => { "small" }
    Option.None => { "none" }
}
""")
        assert cat.execute() == "small"

    def test_or_pattern_two_nullaries(self):
        cat = _ast_cat()
        cat.parse("""
union Event { Click(x, y); KeyPress(code); Quit; Reset; }
e = Event.Reset
match e {
    Event.Quit | Event.Reset => { "terminal" }
    _ => { "other" }
}
""")
        assert cat.execute() == "terminal"


class TestNestedUnions:
    """Union variants whose payload is another union value."""

    def test_nested_some(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
inner = Option.Some(7)
outer = Option.Some(inner)
match outer {
    Option.Some{value} => {
        match value {
            Option.Some{value} => { value }
            Option.None => { -1 }
        }
    }
    Option.None => { -2 }
}
""")
        assert cat.execute() == 7

    def test_nested_with_none_payload(self):
        cat = _ast_cat()
        cat.parse("""
union Option { Some(value); None; }
outer = Option.Some(Option.None)
match outer {
    Option.Some{value} => {
        match value {
            Option.Some{value} => { value }
            Option.None => { -1 }
        }
    }
    Option.None => { -2 }
}
""")
        assert cat.execute() == -1


class TestUnionHashable:
    """Payload-bearing variants are usable as dict keys / set members."""

    def test_payload_as_dict_key(self):
        # Note: `{}` parses as an empty set, not a dict, in this executor
        # path; use `dict()` to get an empty dict.
        cat = _ast_cat()
        cat.parse("""
union Status { Ok(code); Err(msg); }
d = dict()
d[Status.Ok(200)] = "success"
d[Status.Err("timeout")] = "retry"
d[Status.Ok(200)]
""")
        assert cat.execute() == "success"

    def test_nullary_as_set_member(self):
        cat = _ast_cat()
        cat.parse("""
union Event { Click(x, y); Quit; }
s = set()
s.add(Event.Quit)
Event.Quit in s
""")
        assert cat.execute() is True


OPTION_WITH_METHODS = """
union Option {
    Some(value);
    None;
    map(self, f) => {
        match self {
            Option.Some{value} => { Option.Some(f(value)) }
            Option.None => { Option.None }
        }
    }
    unwrap_or(self, default) => {
        match self {
            Option.Some{value} => { value }
            Option.None => { default }
        }
    }
}
"""


class TestUnionMethods:
    """Methods declared on the union, dispatched on every variant.

    `self` receives whichever variant the method is called on; the body
    discriminates with `match`. Methods never mutate `self` (unions are
    immutable) -- they return values.
    """

    def test_method_on_payload_variant(self):
        cat = _ast_cat()
        cat.parse(OPTION_WITH_METHODS + "Option.Some(21).map((x) => { x * 2 }).unwrap_or(0)")
        assert cat.execute() == 42

    def test_method_on_nullary_variant(self):
        cat = _ast_cat()
        cat.parse(OPTION_WITH_METHODS + "Option.None.unwrap_or(-1)")
        assert cat.execute() == -1

    def test_method_chain_through_nullary(self):
        cat = _ast_cat()
        cat.parse(OPTION_WITH_METHODS + "Option.None.map((x) => { x }).unwrap_or(-2)")
        assert cat.execute() == -2

    def test_method_on_stored_nullary(self):
        cat = _ast_cat()
        cat.parse(OPTION_WITH_METHODS + "x = Option.None\nx.unwrap_or(-3)")
        assert cat.execute() == -3

    def test_unknown_method_raises(self):
        cat = _ast_cat()
        cat.parse(OPTION_WITH_METHODS + "Option.Some(1).nope()")
        with pytest.raises(Exception):
            cat.execute()

    def test_method_variant_collision_rejected(self):
        cat = _ast_cat()
        with pytest.raises(Exception, match="collides"):
            cat.parse("union U { A; A(self) => { 1 } }\nU")
            cat.execute()

    def test_init_method_rejected(self):
        cat = _ast_cat()
        with pytest.raises(Exception, match="no init"):
            cat.parse("union V { A; init(self) => { 1 } }\nV")
            cat.execute()

    def test_duplicate_method_rejected(self):
        cat = _ast_cat()
        with pytest.raises(Exception, match="duplicate method"):
            cat.parse("union W { A; m(self) => { 1 } m(self) => { 2 } }\nW")
            cat.execute()


class TestUnionMethodsInBroadcast:
    """Union variants must survive the ND-broadcast (`.[]`) boundary.

    A `.[]` lambda runs in a nested VM. That VM inherits the parent's
    symbol table and enum registry, so a variant keeps its identity:
    calling a union method inside the lambda works, and a struct holding
    a variant built inside a `.[]` keeps the live variant in its field.
    Without that inheritance the variant would be demoted to its tag int.
    """

    FLAG = (
        "union Flag {\n"
        "    on; off\n"
        '    label(self) => { match self { Flag.on => { "on" } Flag.off => { "off" } } }\n'
        "}\n"
    )

    def test_method_call_in_broadcast(self):
        cat = _ast_cat()
        cat.parse(self.FLAG + "list(Flag.on, Flag.off).[(v) => { v.label() }]")
        assert cat.execute() == ["on", "off"]

    def test_variant_field_constructed_in_broadcast(self):
        cat = _ast_cat()
        code = (
            "union Flag { on; off }\n"
            "struct Box { flag }\n"
            "boxes = list(Flag.on, Flag.off).[(f) => { Box(f) }]\n"
            "list(boxes[0].flag == Flag.on, boxes[1].flag == Flag.off)\n"
        )
        cat.parse(code)
        assert cat.execute() == [True, True]

    def test_variant_created_inside_broadcast(self):
        # The variant is produced *inside* the `.[]` lambda (not passed in),
        # so its symbol must already live in the shared table -- it is interned
        # when the union is declared, not lazily on the first crossing.
        cat = _ast_cat()
        code = (
            self.FLAG + "pick = (n) => { match True { _ if n > 0 => { Flag.on } _ => { Flag.off } } }\n"
            "list(1, -1).[(n) => { pick(n).label() }]\n"
        )
        cat.parse(code)
        assert cat.execute() == ["on", "off"]


class TestTypedVariantPayload:
    """Concrete payload field types are enforced at a variant constructor
    (`U.A(...)`), exactly like struct fields: a provable mismatch is a static
    E300, a non-provable one a runtime TypeError. A type-parameter field
    (`Some(v: T)`) is not fixed at construction and never fires."""

    def test_literal_mismatch_is_static_error(self):
        cat = _ast_cat()
        with pytest.raises(Exception, match='E300'):
            cat.parse("union U { A(x: int) }\nU.A(1.5)\n")

    @pytest.mark.parametrize('vm_mode', ['on', 'off'])
    def test_runtime_mismatch_rejected(self, vm_mode):
        # `get()` return type is unknown statically, so this is caught at
        # construction time on both executors (fused `U.A(...)` call path).
        cat = Catnip(vm_mode=vm_mode)
        cat.parse("get = () => { 1.5 }\nunion U { A(x: int) }\nr = U.A(get())\n")
        with pytest.raises(Exception, match="field 'x' of 'U.A'"):
            cat.execute()

    @pytest.mark.parametrize('vm_mode', ['on', 'off'])
    def test_conforming_construction_passes(self, vm_mode):
        cat = Catnip(vm_mode=vm_mode)
        cat.parse("union U { A(x: int) }\nr = U.A(3)\nr.x\n")
        assert cat.execute() == 3

    @pytest.mark.parametrize('vm_mode', ['on', 'off'])
    def test_int_coerced_into_float_field(self, vm_mode):
        cat = Catnip(vm_mode=vm_mode)
        cat.parse("union U { A(x: float) }\nr = U.A(3)\nr.x\n")
        assert cat.execute() == 3.0

    @pytest.mark.parametrize('vm_mode', ['on', 'off'])
    def test_generic_type_param_field_not_checked(self, vm_mode):
        # `Some(v: T)` accepts any argument -- `T` binds at the use-site
        # boundary, not at construction.
        cat = Catnip(vm_mode=vm_mode)
        cat.parse("union Opt[T] { Some(v: T)\n None }\nr = Opt.Some(1.5)\nr.v\n")
        assert cat.execute() == 1.5
