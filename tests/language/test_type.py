# FILE: tests/language/test_type.py
"""Tests for the native typeof() builtin."""

import pytest

from catnip import Catnip


def run(code):
    cat = Catnip()
    cat.parse(code)
    return cat.execute()


class TestTypeOf:
    """Basic typeof() returns."""

    def test_int(self):
        assert run('typeof(42)') == "int"

    def test_float(self):
        assert run('typeof(3.14)') == "float"

    def test_bool_true(self):
        assert run('typeof(True)') == "bool"

    def test_bool_false(self):
        assert run('typeof(False)') == "bool"

    def test_nil(self):
        assert run('typeof(None)') == "nil"

    def test_string(self):
        assert run('typeof("hello")') == "string"

    def test_list(self):
        assert run('typeof(list(1, 2, 3))') == "list"

    def test_tuple(self):
        assert run('typeof(tuple(1, 2, 3))') == "tuple"

    def test_empty_list(self):
        assert run('typeof(list())') == "list"

    def test_empty_tuple(self):
        assert run('typeof(tuple())') == "tuple"


class TestTypeOfFunctions:
    """typeof() on callables."""

    def test_lambda(self):
        assert run('typeof(() => { 1 })') == "function"

    def test_named_function(self):
        assert run('f = (x) => { x }; typeof(f)') == "function"


class TestTypeOfStruct:
    """typeof() on struct instances."""

    def test_struct_instance(self):
        assert run('struct Point { x; y }; typeof(Point(1, 2))') == "Point"

    def test_struct_different_types(self):
        code = '''
        struct Foo { a }
        struct Bar { b }
        f = Foo(1)
        b = Bar(2)
        typeof(f) + " " + typeof(b)
        '''
        assert run(code) == "Foo Bar"


class TestTypeOfExpressions:
    """typeof() on computed values."""

    def test_arithmetic_result(self):
        assert run('typeof(1 + 2)') == "int"

    def test_division_result(self):
        assert run('typeof(1 / 2)') == "float"

    def test_string_concat(self):
        assert run('typeof("a" + "b")') == "string"

    def test_comparison_result(self):
        assert run('typeof(1 < 2)') == "bool"

    def test_type_in_condition(self):
        assert run('if typeof(42) == "int" { "yes" } else { "no" }') == "yes"

    def test_bigint(self):
        assert run('typeof(2 ** 100)') == "int"


class TestTypedParamBoundary:
    """TH2-B step 0b: an annotated param is checked + coerced to its declared
    type at the function prologue. Runs in whatever executor the suite selects
    (VM prologue / AST registry have parity)."""

    def test_int_widens_to_float(self):
        # int passed to a float param is coerced (numeric tower).
        assert run('f = (x: float) => { typeof(x) }\nf(1)') == "float"
        assert run('f = (x: float) => { x }\nf(1)') == 1.0

    def test_bool_widens_to_int(self):
        assert run('f = (x: int) => { typeof(x) }\nf(True)') == "int"
        assert run('f = (x: int) => { x + 1 }\nf(True)') == 2

    def test_exact_type_passes(self):
        assert run('f = (x: int) => { x + 1 }\nf(5)') == 6

    def test_unannotated_param_unchanged(self):
        assert run('f = (x) => { typeof(x) }\nf(1)') == "int"

    def test_dynamic_mismatch_rejected_at_boundary(self):
        # A value whose type is not statically provable (so no E300) but is wrong
        # at runtime is refused at the boundary, not silently run.
        with pytest.raises(Exception):
            run('g = (y) => { y }\nf = (x: int) => { x }\nf(g("no"))')

    def test_narrowing_rejected_at_boundary(self):
        # float into an int slot is one-way refused (tower direction).
        with pytest.raises(Exception):
            run('g = (y) => { y }\nf = (x: int) => { x }\nf(g(1.5))')

    def test_dynamic_widening_accepted_at_boundary(self):
        # int reaching a float param dynamically is coerced, not refused.
        assert run('g = (y) => { y }\nf = (x: float) => { x }\nf(g(2))') == 2.0

    def test_str_param_accepts_str(self):
        assert run('f = (x: str) => { x }\nf("hi")') == "hi"
        # the param is a usable str (concatenation works)
        assert run('f = (x: str) => { x + "!" }\nf("hi")') == "hi!"

    def test_str_param_rejects_non_str_statically(self):
        # A provably-int literal into a str param is refused (E300, no widening).
        with pytest.raises(Exception):
            run('f = (x: str) => { x }\nf(5)')

    def test_str_dynamic_mismatch_rejected_at_boundary(self):
        # A non-str whose type is not statically provable is refused at the
        # runtime boundary (str has no coercion).
        with pytest.raises(Exception):
            run('g = (y) => { y }\nf = (x: str) => { x }\nf(g(5))')

    def test_bigint_widens_to_float(self):
        # A bigint within f64 range is coerced like an int. `2 ** 64` is built by
        # arithmetic, so it is a real bigint (not a float literal).
        assert run('f = (x: float) => { x }\nf(2 ** 64)') == float(2**64)

    def test_bigint_overflow_to_float_rejected(self):
        # A bigint too large for f64 is refused, not silently coerced to inf —
        # matching Python's float(huge_int), and consistent across VM and AST.
        with pytest.raises(Exception):
            run('f = (x: float) => { x }\nf(2 ** 10000)')

    def test_bigint_overflow_caught_as_typeerror(self):
        # The overflow surfaces as a boundary TypeError — the same class as any
        # other boundary failure — so `except TypeError` catches it uniformly in
        # both VM and AST (one exception contract across executors).
        prog = 'f = (x: float) => { x }\n' 'try { f(2 ** 10000) } except { e: TypeError => { "caught" } }'
        assert run(prog) == "caught"


# Nominal-type definitions shared by the nominal boundary tests.
_NOM_DEFS = (
    'struct Point { x; y }\n'
    'struct Base { a }\n'
    'struct Child extends(Base) { b }\n'
    'trait Drawable { }\n'
    'struct Widget implements(Drawable) { w }\n'
    'enum Color { Red; Green }\n'
    'union Opt { Some(v); None }\n'
)


class TestNominalParamBoundary:
    """Enforcement nominal: a param annotated with a struct/enum/union/trait is
    checked for membership (with subtyping) at the function prologue -- the VM
    `CheckNominal` opcode and its AST mirror. No coercion: a nominal value is
    never rewritten. Runs in whatever executor the suite selects (VM/AST parity)."""

    def test_struct_accept(self):
        assert run(_NOM_DEFS + 'f = (p: Point) => { p.x }\nf(Point(10, 20))') == 10

    def test_struct_reject(self):
        with pytest.raises(Exception):
            run(_NOM_DEFS + 'f = (p: Point) => { p.x }\nf(5)')

    def test_struct_reject_is_typeerror(self):
        # The nominal mismatch surfaces as a TypeError, catchable uniformly.
        prog = _NOM_DEFS + 'f = (p: Point) => { 1 }\ntry { f(5) } except { e: TypeError => { "caught" } }'
        assert run(prog) == "caught"

    def test_wrong_struct_rejected(self):
        with pytest.raises(Exception):
            run(_NOM_DEFS + 'f = (p: Point) => { 1 }\nf(Base(1))')

    def test_subtype_extends_accepted(self):
        # A Child extends(Base) is accepted where a Base is declared (Liskov).
        assert run(_NOM_DEFS + 'g = (b: Base) => { b.a }\ng(Child(7, 8))') == 7

    def test_trait_implementer_accepted(self):
        assert run(_NOM_DEFS + 'd = (x: Drawable) => { x.w }\nd(Widget(3))') == 3

    def test_trait_non_implementer_rejected(self):
        # A known trait makes a non-implementer a type error (not a silent no-op).
        with pytest.raises(Exception):
            run(_NOM_DEFS + 'd = (x: Drawable) => { 1 }\nd(Point(1, 2))')

    def test_enum_accept(self):
        assert run(_NOM_DEFS + 'h = (c: Color) => { 1 }\nh(Color.Red)') == 1

    def test_enum_reject(self):
        with pytest.raises(Exception):
            run(_NOM_DEFS + 'h = (c: Color) => { 1 }\nh(5)')

    def test_union_payload_variant_accept(self):
        assert run(_NOM_DEFS + 'k = (o: Opt) => { 1 }\nk(Opt.Some(3))') == 1

    def test_union_nullary_variant_accept(self):
        assert run(_NOM_DEFS + 'k = (o: Opt) => { 1 }\nk(Opt.None)') == 1

    def test_union_reject(self):
        with pytest.raises(Exception):
            run(_NOM_DEFS + 'k = (o: Opt) => { 1 }\nk(5)')

    def test_unknown_name_inert(self):
        # An unknown type name (typo or unmodeled composite) stays inert: no-op.
        assert run('m = (x: Unknown) => { x }\nm(99)') == 99

    def test_no_coercion(self):
        # A nominal value passes through unchanged (typeof unaffected).
        assert run(_NOM_DEFS + 'f = (p: Point) => { typeof(p) }\nf(Point(1, 2))') == "Point"

    def test_omitted_nominal_param_rejected(self):
        # Catnip is permissive on arity (omitted -> None); None is not a member
        # of a nominal type, so an omitted nominal param is refused at the boundary.
        with pytest.raises(Exception):
            run(_NOM_DEFS + 'f = (p: Point) => { 1 }\nf()')

    def test_nd_broadcast_nominal_accept(self):
        # The boundary fires per element in a shared-VM broadcast (thread/sequential).
        assert run(_NOM_DEFS + 'list(Point(1, 2), Point(3, 4)).[~> (p: Point) => { p.x }]') == [1, 3]

    def test_nd_broadcast_nominal_reject(self):
        with pytest.raises(Exception):
            run(_NOM_DEFS + 'list(1, 2, 3).[~> (p: Point) => { p }]')

    def test_direct_trait_accepts_implementer(self):
        # The directly-implemented trait accepts the struct (sanity vs the
        # super-trait case below).
        defs = 'trait A { }\ntrait B extends(A) { }\nstruct S implements(B) { w }\n'
        assert run(defs + 'f = (x: B) => { x.w }\nf(S(7))') == 7

    def test_super_trait_not_member(self):
        # Direct-trait membership only: a struct implementing B (where B extends A)
        # is NOT a member of the super-trait A. This must be consistent across VM
        # and AST -- the VM mro carries the trait linearization, so the membership
        # check must exclude trait names from the mro (parity with AST/PureVM).
        defs = 'trait A { }\ntrait B extends(A) { }\nstruct S implements(B) { w }\n'
        with pytest.raises(Exception):
            run(defs + 'f = (x: A) => { 1 }\nf(S(7))')

    def test_struct_trait_name_collision_accepts_subtype(self):
        # A struct and a trait may share a name; a struct subtype must still be
        # accepted for the struct-named param. Struct ancestry is resolved via the
        # extends chain, not the trait-conflated VM mro, so VM and AST agree.
        defs = 'trait Base { }\nstruct Base { a }\nstruct Child extends(Base) { b }\n'
        assert run(defs + 'f = (x: Base) => { x.a }\nf(Child(1, 2))') == 1

    def test_multilevel_struct_extends_accepted(self):
        # Transitive struct extends (C extends B extends A) is accepted for (x: A).
        defs = 'struct A { a }\nstruct B extends(A) { b }\nstruct C extends(B) { c }\n'
        assert run(defs + 'f = (x: A) => { x.a }\nf(C(1, 2, 3))') == 1


class TestUnionParamBoundary:
    """Type unions (`int | str`, `Point | None`): a param annotated with a union
    is accepted when it satisfies any member -- primitives by the numeric tower
    (no coercion), nominals by subtyping -- and rejected with a TypeError naming
    the union otherwise. Provable mismatches are caught statically (E300); the
    rest by the VM `CheckUnion` opcode and its AST mirror. Runs in whatever
    executor the suite selects (VM/AST parity)."""

    def test_accept_each_primitive_member(self):
        assert run('f = (x: int | str) => { x }\nf(1)') == 1
        assert run('f = (x: int | str) => { x }\nf("a")') == "a"

    def test_no_coercion_between_members(self):
        # A union can't coerce toward one member: an int stays an int, a bool a
        # bool (contrast with `(x: float)` which would coerce 1 -> 1.0).
        assert run('f = (x: int | str) => { typeof(x) }\nf(1)') == "int"
        assert run('f = (x: int | str) => { typeof(x) }\nf(True)') == "bool"

    def test_numeric_tower_member(self):
        # A bool belongs to an `int` member (bool <: int); an int to a `float` one.
        assert run('f = (x: int | str) => { x }\nf(True)') is True
        assert run('f = (x: float | str) => { typeof(x) }\nf(2)') == "int"

    def test_reject_non_member_static(self):
        # A provably-float literal into `int | str` is rejected (E300, static).
        with pytest.raises(Exception):
            run('f = (x: int | str) => { x }\nf(1.5)')

    def test_reject_non_member_runtime(self):
        # A float whose type is not statically provable reaches CheckUnion and is
        # rejected at runtime (exercises the opcode, not just E300).
        with pytest.raises(Exception):
            run('f = (x: int | str) => { x }\ny = 1.0 * 1.5\nf(y)')

    def test_optional_nominal_accept(self):
        assert run(_NOM_DEFS + 'f = (p: Point | None) => { p.x }\nf(Point(7, 8))') == 7

    def test_optional_nominal_accept_none(self):
        assert run(_NOM_DEFS + 'f = (p: Point | None) => { p }\nf(None)') is None

    def test_optional_nominal_reject(self):
        with pytest.raises(Exception):
            run(_NOM_DEFS + 'f = (p: Point | None) => { p }\nf("a")')

    def test_optional_nominal_reject_is_typeerror(self):
        prog = _NOM_DEFS + 'f = (p: Point | None) => { 1 }\ntry { f("a") } except { e: TypeError => { "caught" } }'
        assert run(prog) == "caught"

    def test_subtype_in_union_accepted(self):
        # Subtyping holds for a nominal member: Child extends(Base) into Base|None.
        assert run(_NOM_DEFS + 'g = (b: Base | None) => { b.a }\ng(Child(1, 2))') == 1

    def test_unresolved_member_inert(self):
        # A member the boundary can't enforce (an *undeclared* generic head) makes
        # the union inert. Option[int] is now modeled, so use an undeclared union.
        assert run('m = (x: int | Undeclared[int]) => { x }\nm("anything")') == "anything"

    def test_unknown_nominal_member_inert(self):
        # An unknown nominal member can't be proven absent, so the union stays
        # inert rather than rejecting a possibly-valid value (mirrors CheckNominal).
        assert run('m = (x: int | Unknown) => { x }\nm("anything")') == "anything"

    def test_nd_broadcast_union_accept(self):
        assert run('list(1, 2, 3).[~> (n: int | str) => { n }]') == [1, 2, 3]

    def test_nd_broadcast_union_reject(self):
        with pytest.raises(Exception):
            run('list(1.5, 2.5).[~> (n: int | str) => { n }]')


class TestCompositeParamBoundary:
    """Composite annotations (`list[T]`, `dict[K, V]`): the container type is
    checked and, when the annotation carries type parameters, each element (and,
    for a dict, each key/value) is checked against them, recursively. No coercion:
    only a list satisfies `list`, only a dict satisfies `dict`. Provable mismatches
    are caught statically (E300); dynamic values by the VM `CheckComposite` opcode
    and its AST mirror. VM/AST parity."""

    def test_list_accepts_list(self):
        assert run('f = (xs: list) => { xs }\nf([1, 2, 3])') == [1, 2, 3]

    def test_list_param_checks_element_type(self):
        # list[str] checks the element: a provable int-filled literal is rejected
        # (E300), a str-filled one passes; list[int] accepts an int-filled one.
        with pytest.raises(Exception):
            run('f = (xs: list[str]) => { xs }\nf([1, 2, 3])')
        assert run('f = (xs: list[str]) => { xs }\nf(["a", "b"])') == ["a", "b"]
        assert run('f = (xs: list[int]) => { xs }\nf([1, 2, 3])') == [1, 2, 3]

    def test_dict_accepts_dict(self):
        assert run('f = (d: dict) => { d }\nf({1: 2})') == {1: 2}

    def test_dict_param_checks_kv_types(self):
        # dict[str, int] checks key and value: a provable {int: int} literal is
        # rejected, a {str: int} one passes.
        with pytest.raises(Exception):
            run('f = (d: dict[str, int]) => { d }\nf({1: 2})')
        assert run('f = (d: dict[str, int]) => { d }\nf({"a": 2})') == {"a": 2}

    def test_dynamic_value_accepted_at_boundary(self):
        # A list whose type isn't statically known (returned by an untyped call)
        # still passes the runtime boundary check.
        assert run('mk = (v) => { v }\nf = (xs: list) => { xs }\nf(mk(list(1, 2, 3)))') == [1, 2, 3]

    def test_list_rejects_int(self):
        # A provable int into a `list` slot is rejected statically (E300).
        with pytest.raises(Exception):
            run('f = (xs: list) => { xs }\nf(42)')

    def test_list_rejects_dict_literal_static(self):
        # A dict literal infers to `dict`, so a list slot rejects it statically.
        with pytest.raises(Exception):
            run('f = (xs: list) => { xs }\nf({1: 2})')

    def test_dict_rejects_list_literal_static(self):
        with pytest.raises(Exception):
            run('f = (d: dict) => { d }\nf([1, 2, 3])')

    def test_runtime_boundary_rejects_wrong_container(self):
        # A dict whose type isn't statically known reaches CheckType(list) and is
        # rejected at runtime -- exercises the opcode, not just static E300.
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (xs: list) => { xs }\nf(mk({1: 2}))')

    def test_reject_is_typeerror(self):
        # The runtime boundary failure is a catchable TypeError. Uses an untyped
        # call so the mismatch reaches the opcode rather than static E300.
        prog = (
            'mk = (v) => { v }\nf = (xs: list) => { 1 }\n'
            'try { f(mk({1: 2})) } except { e: TypeError => { "caught" } }'
        )
        assert run(prog) == "caught"

    def test_return_type_list_ok(self):
        assert run('f = (): list => { [1, 2, 3] }\nf()') == [1, 2, 3]

    def test_return_type_list_mismatch(self):
        with pytest.raises(Exception):
            run('f = (): list => { 42 }\nf()')

    def test_union_list_member_accepts_list(self):
        assert run('f = (x: int | list[int]) => { x }\nf([1, 2, 3])') == [1, 2, 3]

    def test_union_list_member_accepts_int(self):
        assert run('f = (x: int | list[int]) => { x }\nf(5)') == 5

    def test_union_list_member_rejects_str(self):
        with pytest.raises(Exception):
            run('f = (x: int | list[int]) => { x }\nf("a")')

    def test_runtime_rejects_bad_element(self):
        # A bad element in a dynamically-typed list reaches CheckComposite.
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (xs: list[int]) => { xs }\nf(mk(["a", "b"]))')

    def test_runtime_rejects_bad_dict_key(self):
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (d: dict[str, int]) => { d }\nf(mk({1: 2}))')

    def test_runtime_rejects_bad_dict_value(self):
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (d: dict[str, int]) => { d }\nf(mk({"k": "x"}))')

    def test_runtime_accepts_good_dynamic_elements(self):
        assert run('mk = (v) => { v }\nf = (xs: list[int]) => { xs }\nf(mk([1, 2, 3]))') == [1, 2, 3]

    def test_empty_composites_accept_any_param(self):
        # An empty list/dict satisfies any element annotation (no element to fail).
        # `{}` is an empty block in Catnip, so an empty dict is `dict()`.
        assert run('f = (xs: list[int]) => { xs }\nf([])') == []
        assert run('f = (d: dict[str, int]) => { d }\nf(dict())') == {}

    def test_bare_composite_ignores_elements(self):
        # A bare `list`/`dict` (no params) checks only the container.
        assert run('f = (xs: list) => { xs }\nf([1, "a"])') == [1, "a"]

    def test_nested_composite_checks_recursively(self):
        assert run('f = (xs: list[list[int]]) => { xs }\nf([[1, 2], [3]])') == [[1, 2], [3]]
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (xs: list[list[int]]) => { xs }\nf(mk([["a"]]))')

    def test_literal_param_is_covariant(self):
        # A freshly built int-literal list satisfies list[float] (covariance on
        # literals, numeric tower on the element); no coercion of the elements.
        assert run('f = (xs: list[float]) => { xs }\nf([1, 2, 3])') == [1, 2, 3]

    def test_typed_value_param_is_invariant(self):
        # A list[int]-typed value does NOT satisfy list[float] (invariance: a
        # mutation through the alias would be unsound), caught statically when one
        # typed function passes its parameter to another.
        with pytest.raises(Exception):
            run('f = (xs: list[float]) => { xs }\ng = (ys: list[int]) => { f(ys) }\ng([1, 2, 3])')
        # Same parameter type passes.
        assert run('f = (xs: list[int]) => { xs }\ng = (ys: list[int]) => { f(ys) }\ng([1, 2, 3])') == [1, 2, 3]


class TestSetParamBoundary:
    """Set annotations (`set[T]`): the container type is checked and, when the
    annotation carries an element type, each element is checked against it,
    recursively. Homogeneous like `list[T]` but a distinct container -- no
    coercion, a set never satisfies a `list` slot nor the reverse. Provable
    mismatches are caught statically (E300); dynamic values by the VM
    `CheckComposite` opcode and its AST mirror. VM/AST parity."""

    def test_set_accepts_set(self):
        assert run('f = (xs: set) => { xs }\nf(set(1, 2, 3))') == {1, 2, 3}

    def test_set_param_checks_element_type(self):
        # set[str] checks the element: a provable int-filled literal is rejected
        # (E300), a str-filled one passes; set[int] accepts an int-filled one.
        with pytest.raises(Exception):
            run('f = (xs: set[str]) => { xs }\nf(set(1, 2, 3))')
        assert run('f = (xs: set[str]) => { xs }\nf(set("a", "b"))') == {"a", "b"}
        assert run('f = (xs: set[int]) => { xs }\nf(set(1, 2, 3))') == {1, 2, 3}

    def test_set_rejects_int(self):
        # A provable int into a `set` slot is rejected statically (E300).
        with pytest.raises(Exception):
            run('f = (xs: set) => { xs }\nf(42)')

    def test_set_rejects_list_literal_static(self):
        # A list literal infers to `list`, a distinct container -> set slot rejects.
        with pytest.raises(Exception):
            run('f = (xs: set) => { xs }\nf([1, 2, 3])')

    def test_list_rejects_set_literal_static(self):
        # Symmetric: a set literal does not satisfy a list slot.
        with pytest.raises(Exception):
            run('f = (xs: list) => { xs }\nf(set(1, 2, 3))')

    def test_dynamic_value_accepted_at_boundary(self):
        # A set whose type isn't statically known (returned by an untyped call)
        # still passes the runtime boundary check.
        assert run('mk = (v) => { v }\nf = (xs: set) => { xs }\nf(mk(set(1, 2, 3)))') == {1, 2, 3}

    def test_runtime_boundary_rejects_wrong_container(self):
        # A list whose type isn't statically known reaches CheckType(set) and is
        # rejected at runtime -- exercises the opcode, not just static E300.
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (xs: set) => { xs }\nf(mk([1, 2, 3]))')

    def test_reject_is_typeerror(self):
        prog = (
            'mk = (v) => { v }\nf = (xs: set) => { 1 }\n'
            'try { f(mk([1, 2, 3])) } except { e: TypeError => { "caught" } }'
        )
        assert run(prog) == "caught"

    def test_return_type_set_ok(self):
        assert run('f = (): set => { set(1, 2, 3) }\nf()') == {1, 2, 3}

    def test_return_type_set_mismatch(self):
        with pytest.raises(Exception):
            run('f = (): set => { 42 }\nf()')

    def test_union_set_member_accepts_set(self):
        assert run('f = (x: int | set[int]) => { x }\nf(set(1, 2, 3))') == {1, 2, 3}

    def test_union_set_member_accepts_int(self):
        assert run('f = (x: int | set[int]) => { x }\nf(5)') == 5

    def test_union_set_member_rejects_str(self):
        with pytest.raises(Exception):
            run('f = (x: int | set[int]) => { x }\nf("a")')

    def test_runtime_rejects_bad_element(self):
        # A bad element in a dynamically-typed set reaches CheckComposite.
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (xs: set[int]) => { xs }\nf(mk(set("a", "b")))')

    def test_runtime_accepts_good_dynamic_elements(self):
        assert run('mk = (v) => { v }\nf = (xs: set[int]) => { xs }\nf(mk(set(1, 2, 3)))') == {1, 2, 3}

    def test_empty_set_accepts_any_param(self):
        # An empty set satisfies any element annotation (no element to fail).
        assert run('f = (xs: set[int]) => { xs }\nf(set())') == set()

    def test_bare_set_ignores_elements(self):
        # A bare `set` (no params) checks only the container.
        assert run('f = (xs: set) => { xs }\nf(set(1, "a"))') == {1, "a"}

    def test_set_of_structs_element_check(self):
        # A set of struct instances in a set[Nominal] slot: the element check
        # resolves each struct's type, accepting matching ones and rejecting others.
        defs = 'struct Point { x: int }\n'
        assert run(defs + 'f = (xs: set[Point]) => { len(xs) }\nf(set(Point(1), Point(2)))') == 2
        with pytest.raises(Exception):
            run(defs + 'mk = (v) => { v }\nf = (xs: set[Point]) => { xs }\nf(mk(set(1, 2)))')

    def test_nested_set_in_list_checks_recursively(self):
        # set as an element of a list: list[set[int]], checked recursively.
        assert run('f = (xs: list[set[int]]) => { xs }\nf([set(1, 2), set(3)])') == [{1, 2}, {3}]
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (xs: list[set[int]]) => { xs }\nf(mk([set("a")]))')

    def test_literal_param_is_covariant(self):
        # A freshly built int-literal set satisfies set[float] (covariance on
        # literals, numeric tower on the element); no coercion of the elements.
        assert run('f = (xs: set[float]) => { xs }\nf(set(1, 2, 3))') == {1, 2, 3}

    def test_typed_value_param_is_invariant(self):
        # A set[int]-typed value does NOT satisfy set[float] (invariance: a
        # mutation through the alias would be unsound), caught statically when one
        # typed function passes its parameter to another.
        with pytest.raises(Exception):
            run('f = (xs: set[float]) => { xs }\ng = (ys: set[int]) => { f(ys) }\ng(set(1, 2, 3))')
        # Same parameter type passes.
        assert run('f = (xs: set[int]) => { xs }\ng = (ys: set[int]) => { f(ys) }\ng(set(1, 2, 3))') == {1, 2, 3}


class TestTupleParamBoundary:
    """Tuple annotations (`tuple[T0, T1, ...]`): a positional heterogeneous
    composite. The container is checked, and when the annotation carries
    parameters the *arity* is enforced (a wrong-length tuple is rejected) and each
    position `i` is checked against `params[i]`. No coercion; a tuple never
    satisfies a `list` slot nor the reverse. Provable mismatches (wrong position,
    wrong arity) are caught statically (E300); dynamic values by the VM
    `CheckComposite` opcode and its AST mirror. VM/AST parity."""

    def test_tuple_accepts_tuple(self):
        assert run('f = (t: tuple) => { t }\nf(tuple(1, 2, 3))') == (1, 2, 3)

    def test_tuple_param_checks_positional_types(self):
        # tuple[int, str] is positional: each slot is checked against its position.
        with pytest.raises(Exception):
            run('f = (t: tuple[int, str]) => { t }\nf(tuple(1, 2))')  # pos 1 not a str
        assert run('f = (t: tuple[int, str]) => { t }\nf(tuple(1, "a"))') == (1, "a")
        assert run('f = (t: tuple[str, int]) => { t }\nf(tuple("a", 1))') == ("a", 1)

    def test_tuple_arity_mismatch_static(self):
        # Arity is part of the contract: a 3-tuple literal does not satisfy
        # tuple[int, str] -- a provable mismatch caught statically (E300).
        with pytest.raises(Exception):
            run('f = (t: tuple[int, str]) => { t }\nf(tuple(1, "a", 3))')

    def test_tuple_rejects_int(self):
        # A provable int into a `tuple` slot is rejected statically (E300).
        with pytest.raises(Exception):
            run('f = (t: tuple) => { t }\nf(42)')

    def test_tuple_rejects_list_literal_static(self):
        # A list literal infers to `list`, a distinct container -> tuple slot rejects.
        with pytest.raises(Exception):
            run('f = (t: tuple) => { t }\nf([1, 2, 3])')

    def test_list_rejects_tuple_literal_static(self):
        # Symmetric: a tuple literal does not satisfy a list slot.
        with pytest.raises(Exception):
            run('f = (xs: list) => { xs }\nf(tuple(1, 2, 3))')

    def test_dynamic_value_accepted_at_boundary(self):
        # A tuple whose type isn't statically known still passes the boundary.
        assert run('mk = (v) => { v }\nf = (t: tuple) => { t }\nf(mk(tuple(1, 2, 3)))') == (1, 2, 3)

    def test_runtime_boundary_rejects_wrong_container(self):
        # A list whose type isn't statically known reaches CheckComposite(tuple)
        # and is rejected at runtime -- exercises the opcode, not just static E300.
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (t: tuple) => { t }\nf(mk([1, 2, 3]))')

    def test_runtime_rejects_wrong_arity(self):
        # A dynamically-typed tuple of the wrong length reaches the arity check.
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (t: tuple[int, str]) => { t }\nf(mk(tuple(1, "a", 3)))')

    def test_runtime_rejects_bad_element(self):
        # A bad element at a position in a dynamically-typed tuple reaches the
        # positional element pass of CheckComposite.
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (t: tuple[int, str]) => { t }\nf(mk(tuple(1, 2)))')

    def test_runtime_accepts_good_dynamic(self):
        assert run('mk = (v) => { v }\nf = (t: tuple[int, str]) => { t }\nf(mk(tuple(1, "a")))') == (1, "a")

    def test_reject_is_typeerror(self):
        prog = (
            'mk = (v) => { v }\nf = (t: tuple) => { 1 }\n'
            'try { f(mk([1, 2, 3])) } except { e: TypeError => { "caught" } }'
        )
        assert run(prog) == "caught"

    def test_return_type_tuple_ok(self):
        assert run('f = (): tuple => { tuple(1, 2, 3) }\nf()') == (1, 2, 3)

    def test_return_type_tuple_mismatch(self):
        with pytest.raises(Exception):
            run('f = (): tuple => { 42 }\nf()')

    def test_union_tuple_member_accepts_tuple(self):
        assert run('f = (x: int | tuple[int, str]) => { x }\nf(tuple(1, "a"))') == (1, "a")

    def test_union_tuple_member_accepts_int(self):
        assert run('f = (x: int | tuple[int, str]) => { x }\nf(5)') == 5

    def test_union_tuple_member_rejects_str(self):
        with pytest.raises(Exception):
            run('f = (x: int | tuple[int, str]) => { x }\nf("a")')

    def test_bare_tuple_ignores_arity_and_elements(self):
        # A bare `tuple` (no params) checks only the container -- any arity, any
        # element types.
        assert run('f = (t: tuple) => { t }\nf(tuple(1, "a", True))') == (1, "a", True)

    def test_empty_tuple_literal_rejected_statically(self):
        # A direct empty literal () has a KNOWN arity 0, a provable mismatch against
        # a fixed-arity slot -- rejected at compile time (E300), like tuple(1, 2, 3).
        with pytest.raises(Exception):
            run('f = (t: tuple[int, str]) => { t }\nf(tuple())')

    def test_empty_tuple_dynamic_rejected_at_runtime(self):
        # When the arity isn't statically known (an untyped call's return), the
        # runtime boundary's arity check rejects the empty tuple instead.
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (t: tuple[int, str]) => { t }\nf(mk(tuple()))')

    def test_bare_tuple_value_defers(self):
        # A value of bare type `tuple` (unknown arity) into a fixed-arity slot is
        # NOT a provable mismatch -- it defers to the runtime boundary, which here
        # accepts a matching 2-tuple. Distinguishes None (defer) from Some([]) (reject).
        assert run('g = (u: tuple) => { u }\nf = (t: tuple[int, str]) => { len(t) }\nf(g(tuple(1, "a")))') == 2

    def test_tuple_of_structs_element_check(self):
        # A struct instance at a position in tuple[Nominal, V]: the element check
        # resolves the struct's type, accepting a match and rejecting a mismatch.
        defs = 'struct Point { x: int }\n'
        assert run(defs + 'f = (t: tuple[Point, int]) => { len(t) }\nf(tuple(Point(1), 9))') == 2
        with pytest.raises(Exception):
            run(defs + 'mk = (v) => { v }\nf = (t: tuple[Point, int]) => { t }\nf(mk(tuple(1, 9)))')

    def test_tuple_struct_field(self):
        # tuple[int, str] as a struct field: enforced at the constructor (the field
        # boundary path, exercised in all three executors).
        assert run('struct P { t: tuple[int, str] }\nlen(P(tuple(1, "a")).t)') == 2
        with pytest.raises(Exception):
            run('struct P { t: tuple[int, str] }\nmk = (v) => { v }\nP(mk(tuple(1, 2)))')

    def test_nested_tuple_in_list_checks_recursively(self):
        # tuple as an element of a list: list[tuple[int, str]], checked recursively.
        assert run('f = (xs: list[tuple[int, str]]) => { xs }\nf([tuple(1, "a"), tuple(2, "b")])') == [
            (1, "a"),
            (2, "b"),
        ]
        with pytest.raises(Exception):
            run('mk = (v) => { v }\nf = (xs: list[tuple[int, str]]) => { xs }\nf(mk([tuple(1, 2)]))')

    def test_literal_param_is_covariant(self):
        # A freshly built int-literal at a float position satisfies tuple[float, str]
        # (covariance on literals, numeric tower on the element); no coercion.
        assert run('f = (t: tuple[float, str]) => { t }\nf(tuple(1, "a"))') == (1, "a")

    def test_typed_value_param_is_covariant(self):
        # A tuple is immutable, so it is covariant (unlike the mutable list/set/dict
        # which are invariant for a typed value): a tuple[int, str]-typed value DOES
        # satisfy tuple[float, str] (int <: float), even passed between typed
        # functions. No alias-mutation hazard can make this unsound (PEP 484).
        assert run('f = (t: tuple[float, str]) => { t }\ng = (u: tuple[int, str]) => { f(u) }\ng(tuple(1, "a"))') == (
            1,
            "a",
        )
        # And the same parameter type passes too.
        assert run('f = (t: tuple[int, str]) => { t }\ng = (u: tuple[int, str]) => { f(u) }\ng(tuple(1, "a"))') == (
            1,
            "a",
        )


class TestFieldTypes:
    """Runtime enforcement of struct field type annotations (boundary at the
    constructor). A statically-provable mismatch is E300 at compile time; these
    route a dynamically-typed value (an unannotated function's return) into a
    typed field so the runtime check fires. Covers the three executors via VM/AST
    test runs."""

    def test_primitive_dynamic_mismatch_rejected(self):
        with pytest.raises(Exception):
            run('struct P { x: int }\nget = () => { "h" }\nP(get())')

    def test_primitive_mismatch_is_typeerror_with_field_message(self):
        prog = 'struct P { x: int }\nget = () => { "h" }\n' 'try { P(get()) } except { e: TypeError => { "caught" } }'
        assert run(prog) == "caught"

    def test_primitive_coerced_numeric_tower(self):
        # int into a float field is coerced (parity with typed params).
        assert run('struct P { x: float }\nget = () => { 3 }\nP(get()).x') == 3.0

    def test_exact_type_passes(self):
        assert run('struct P { x: int }\nget = () => { 5 }\nP(get()).x') == 5

    def test_unannotated_field_unchanged(self):
        assert run('struct P { x }\nget = () => { "h" }\nP(get()).x') == "h"

    def test_nominal_field_rejected(self):
        with pytest.raises(Exception):
            run('struct Pt { a }\nstruct Box { p: Pt }\nget = () => { 5 }\nBox(get())')

    def test_nominal_field_accepted(self):
        assert run('struct Pt { a }\nstruct Box { p: Pt }\nget = () => { Pt(1) }\nBox(get()).p.a') == 1

    def test_nominal_subtype_accepted(self):
        prog = (
            'struct Base { a }\nstruct Child extends(Base) { b }\n'
            'struct Box { p: Base }\nget = () => { Child(1, 2) }\nBox(get()).p.a'
        )
        assert run(prog) == 1

    def test_union_field_rejected(self):
        with pytest.raises(Exception):
            run('struct P { x: int | str }\nget = () => { 1.5 }\nP(get())')

    def test_union_field_accepted(self):
        assert run('struct P { x: int | str }\nget = () => { "ok" }\nP(get()).x') == "ok"

    def test_composite_field_rejected(self):
        with pytest.raises(Exception):
            run('struct P { xs: list[int] }\nget = () => { ["a"] }\nP(get())')

    def test_composite_field_accepted(self):
        assert run('struct P { xs: list[int] }\nget = () => { [1, 2] }\nlen(P(get()).xs)') == 2

    def test_set_field_rejected(self):
        with pytest.raises(Exception):
            run('struct P { xs: set[int] }\nget = () => { set("a") }\nP(get())')

    def test_set_field_accepted(self):
        assert run('struct P { xs: set[int] }\nget = () => { set(1, 2) }\nlen(P(get()).xs)') == 2

    def test_inherited_field_enforced(self):
        # Field `x: int` declared on the base is enforced on the subtype's
        # constructor (the check travels with the cloned field).
        with pytest.raises(Exception):
            run('struct B { x: int }\nstruct C extends(B) { y }\nget = () => { "h" }\nC(get(), 1)')

    def test_kwargs_field_enforced(self):
        # Keyword-argument construction path is enforced too.
        with pytest.raises(Exception):
            run('struct P { x: int; y }\nget = () => { "h" }\nP(y=1, x=get())')

    def test_default_typed_field_ok(self):
        assert run('struct P { x: int = 7 }\nP().x') == 7

    def test_static_literal_mismatch_rejected(self):
        # Provable at compile time: still E300, raised as an exception.
        with pytest.raises(Exception):
            run('struct P { x: int }\nP("hello")')

    def test_coercion_before_later_field_failure_is_safe(self):
        # x: float coerces a bigint, y: int then fails. Validation precedes any
        # coercion (no double-free on the error path); must raise, not crash.
        prog = (
            'struct P { x: float; y: int }\n'
            'gb = () => { 2 ** 100 }\ngs = () => { "bad" }\n'
            'try { P(gb(), gs()); "no" } except { e: TypeError => { "caught" } }'
        )
        assert run(prog) == "caught"


_GEN_DEFS = 'union Option[T] { Some(value: T); None }\n' 'union Result[T, E] { Ok(value: T); Err(error: E) }\n'


class TestGenericUnionBoundary:
    """Enforcement of generic nominal unions (`Option[int]`, `Result[T, E]`): a
    param/field/return annotated with a parameterized union checks membership and
    substitutes the type arguments into the variant payloads. Runs in whatever
    executor the suite selects (VM/AST parity)."""

    def test_accepts_matching_payload(self):
        assert run(_GEN_DEFS + 'f = (x: Option[int]) => { 1 }\nf(Option.Some(42))') == 1

    def test_rejects_wrong_payload(self):
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (x: Option[int]) => { 1 }\nf(Option.Some("s"))')

    def test_rejects_wrong_payload_is_typeerror(self):
        prog = (
            _GEN_DEFS
            + 'f = (x: Option[int]) => { 1 }\ntry { f(Option.Some("s")) } except { e: TypeError => { "caught" } }'
        )
        assert run(prog) == "caught"

    def test_accepts_nullary_variant(self):
        assert run(_GEN_DEFS + 'f = (x: Option[int]) => { 1 }\nf(Option.None)') == 1

    def test_rejects_non_member(self):
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (x: Option[int]) => { 1 }\nf(5)')

    def test_payload_numeric_tower(self):
        # int payload satisfies Option[float] (int <: float, covariant).
        assert run(_GEN_DEFS + 'f = (x: Option[float]) => { 1 }\nf(Option.Some(3))') == 1

    def test_result_ok_accepted(self):
        assert run(_GEN_DEFS + 'f = (x: Result[int, str]) => { 1 }\nf(Result.Ok(1))') == 1

    def test_result_ok_wrong_type_rejected(self):
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (x: Result[int, str]) => { 1 }\nf(Result.Ok("s"))')

    def test_result_err_second_param(self):
        assert run(_GEN_DEFS + 'f = (x: Result[int, str]) => { 1 }\nf(Result.Err("boom"))') == 1

    def test_result_err_wrong_type_rejected(self):
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (x: Result[int, str]) => { 1 }\nf(Result.Err(7))')

    def test_nested_composite_payload_accepted(self):
        assert run(_GEN_DEFS + 'f = (x: Option[list[int]]) => { 1 }\nf(Option.Some(list(1, 2, 3)))') == 1

    def test_nested_composite_payload_rejected(self):
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (x: Option[list[int]]) => { 1 }\nf(Option.Some(list("a")))')

    def test_as_union_member_int(self):
        assert run(_GEN_DEFS + 'f = (x: int | Option[int]) => { 1 }\nf(7)') == 1

    def test_as_union_member_good_option(self):
        assert run(_GEN_DEFS + 'f = (x: int | Option[int]) => { 1 }\nf(Option.Some(1))') == 1

    def test_as_union_member_bad_option_rejected(self):
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (x: int | Option[int]) => { 1 }\nf(Option.Some("s"))')

    def test_as_struct_field_accepted(self):
        assert run(_GEN_DEFS + 'struct Box { o: Option[int] }\nBox(Option.Some(1)).o.value') == 1

    def test_as_struct_field_rejected(self):
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'struct Box { o: Option[int] }\nBox(Option.Some("s"))')

    def test_arity_mismatch_is_static_error(self):
        # Option has one type parameter; two is a provable arity error (E300 fatal).
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (x: Option[int, str]) => { 1 }')

    def test_return_payload_mismatch_static(self):
        # Extended static inference: Option.Some("s") infers Option[str], rejected
        # by the declared Option[int] return type.
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (): Option[int] => { Option.Some("s") }\n1')

    def test_return_payload_ok_static(self):
        assert run(_GEN_DEFS + 'f = (): Option[int] => { Option.Some(1) }\nf().value') == 1

    def test_unknown_generic_head_inert(self):
        # An undeclared generic head can't be proven a mismatch -> inert (accept).
        assert run('f = (x: Undeclared[int]) => { x }\nf(5)') == 5

    def test_as_set_element_accepted(self):
        assert run(_GEN_DEFS + 'f = (x: set[Option[int]]) => { 1 }\nf(set(Option.Some(1)))') == 1

    def test_as_set_element_rejected(self):
        # Payload of a set element is checked (parity between the value path and
        # the key path across all executors).
        with pytest.raises(Exception):
            run(_GEN_DEFS + 'f = (x: set[Option[int]]) => { 1 }\nf(set(Option.Some("s")))')


class TestFunctionTypeStatic:
    """FT step 1 (static half): `(int) -> int` annotations parse, drive E300 at
    call sites (arity, contravariant params, covariant return) and propagate
    the declared return through callback calls. The runtime boundary for
    function types is a later step: an FT annotation is inert at runtime."""

    def test_conforming_lambda_runs(self):
        assert run('apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((y: int) => { y + 1 }, 20)') == 21

    def test_arity_mismatch_rejected_statically(self):
        with pytest.raises(Exception, match="E300"):
            run('apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((a: int, b: int) => { a }, 2)')

    def test_param_contravariance(self):
        # A str-taking callback cannot serve an int-taking slot...
        with pytest.raises(Exception, match="E300"):
            run('apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((s: str) => { 0 }, 2)')
        # ...but a float-taking callback can (it will happily receive ints).
        assert run('apply = (cb: (int) -> float, x: int) => { cb(x) }\napply((y: float) => { y }, 2)') == 2.0

    def test_return_covariance_rejected(self):
        with pytest.raises(Exception, match="E300"):
            run('apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((y: int): str => { "s" }, 2)')

    def test_non_callable_rejected_statically(self):
        with pytest.raises(Exception, match="E300"):
            run('apply = (cb: (int) -> int, x: int) => { cb(x) }\napply(5, 2)')

    def test_callback_call_arity_in_body(self):
        with pytest.raises(Exception, match="E300"):
            run('f = (cb: (int) -> int) => { cb(1, 2) }\nf((y: int) => { y })')

    def test_return_type_feeds_inference(self):
        # cb() is declared str; g wants int: static rejection through the call.
        with pytest.raises(Exception, match="E300"):
            run('g = (x: int) => { x }\nf = (cb: () -> str) => { g(cb()) }\nf(() => { "s" })')

    def test_unannotated_lambda_defers_and_runs(self):
        assert run('apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((y) => { y * 2 }, 5)') == 10

    def test_ft_in_union_and_absorption(self):
        # FT as last union member; arrow absorbing a trailing union.
        assert run('f = (cb: None | (int) -> int) => { 1 }\nf(None)') == 1
        assert run('f = (cb: (int) -> int | None) => { 1 }\nf((y: int) => { y })') == 1

    def test_higher_order_and_composite_param(self):
        assert run('f = (g: ((int) -> int) -> int) => { g((y: int) => { y }) }\nf((h: (int) -> int) => { h(7) })') == 7
        assert run('f = (hs: list[(int) -> int]) => { 1 }\nf([(y: int) => { y }])') == 1


class TestCallableBoundary:
    """FT3: the function-type runtime boundary (CheckCallable). Values reach
    the prologue through an untyped indirection (an unannotated function's
    return is Top) so the static half stays silent and the boundary decides.
    Runs in whatever executor the suite selects (VM prologue / AST parity)."""

    def test_conforming_callback_passes(self):
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> int) => { cb(41) }\n'
                'apply(pick([(y) => { y + 1 }]))'
            )
            == 42
        )

    def test_arity_mismatch_rejected(self):
        with pytest.raises(Exception):
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> int) => { cb(1) }\n'
                'apply(pick([(a, b) => { a }]))'
            )

    def test_non_callable_rejected(self):
        with pytest.raises(Exception):
            run('pick = (fs) => { fs[0] }\n' 'apply = (cb: (int) -> int) => { cb(1) }\n' 'apply(pick([5]))')

    def test_defaults_widen_acceptance(self):
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> int) => { cb(41) }\n'
                'apply(pick([(x, y = 1) => { x + y }]))'
            )
            == 42
        )

    def test_vararg_accepts_any_arity(self):
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int, int) -> int) => { cb(20, 22) }\n'
                'apply(pick([(*rest) => { rest[0] + rest[1] }]))'
            )
            == 42
        )

    def test_struct_constructor_passes(self):
        assert run('struct P { x }\n' 'apply = (cb: (int) -> P) => { cb(7) }\n' 'apply(P).x') == 7

    def test_builtin_passes_callable_only(self):
        # A builtin's arity is not introspectable: callable-only.
        assert run('apply = (cb: (list) -> int) => { cb([1, 2, 3]) }\n' 'apply(len)') == 3

    def test_callable_as_list_element(self):
        assert (
            run('pick = (xs) => { xs }\n' 'f = (hs: list[(int) -> int]) => { hs[0](1) }\n' 'f(pick([(y) => { y }]))')
            == 1
        )
        with pytest.raises(Exception):
            run('pick = (xs) => { xs }\n' 'f = (hs: list[(int) -> int]) => { 1 }\n' 'f(pick([5]))')

    def test_struct_field_callable(self):
        # FT5 for free: the field-check path reuses the same ParamCheck.
        assert (
            run(
                'pick = (xs) => { xs[0] }\n'
                'struct H { f: (int) -> int }\n'
                'h = H(pick([(y) => { y * 2 }]))\n'
                'h.f(21)'
            )
            == 42
        )
        with pytest.raises(Exception):
            run('pick = (xs) => { xs[0] }\n' 'struct H { f: (int) -> int }\n' 'H(pick([5]))')


class TestCallbackReturnCheck:
    """FT2-A: the declared return of a callback param is enforced on the
    caller side after each call (CheckReturn rewrite -> boundary opcodes)."""

    def test_lying_callback_rejected(self):
        with pytest.raises(Exception):
            run(
                'pick = (fs) => { fs[0] }\n' 'apply = (cb: (int) -> int) => { cb(1) }\n' 'apply(pick([(y) => { "s" }]))'
            )

    def test_conforming_callback_passes(self):
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> int) => { cb(20) + 1 }\n'
                'apply(pick([(y) => { y * 2 }]))'
            )
            == 41
        )

    def test_return_coerced_to_float(self):
        # A declared float return coerces an int result, like a param boundary.
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> float) => { cb(2) }\n'
                'typeof(apply(pick([(y) => { y * 2 }])))'
            )
            == 'float'
        )

    def test_curried_return_checked(self):
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> (int) -> int) => { cb(1)(2) }\n'
                'apply(pick([(y) => { (z) => { y + z } }]))'
            )
            == 3
        )
        with pytest.raises(Exception):
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> (int) -> int) => { cb(1) }\n'
                'apply(pick([(y) => { 7 }]))'
            )

    def test_callback_evaluated_once(self):
        # The wrap must not double-evaluate the call (AST fast path).
        assert (
            run(
                'n = 0\n'
                'tick = (y) => { n = n + 1\n y }\n'
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> int) => { cb(5) }\n'
                'apply(pick([tick]))\n'
                'n'
            )
            == 1
        )

    def test_assigned_callback_not_checked(self):
        # A reassigned param loses the trust rule: no wrap, stays dynamic.
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> int) => { cb = pick([(y) => { "s" }])\n cb(1) }\n'
                'apply(pick([(y) => { y }]))'
            )
            == 's'
        )

    def test_union_callable_member(self):
        # A Callable member of a union is a full alternative (was dropped by
        # the union member iteration: fatal false rejection).
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: None | (int) -> int) => { cb(1) }\n'
                'apply(pick([(y) => { y }]))'
            )
            == 1
        )
        assert (
            run('pick = (fs) => { fs[0] }\n' 'apply = (cb: None | (int) -> int) => { 7 }\n' 'apply(pick([None]))') == 7
        )
        with pytest.raises(Exception):
            run('pick = (fs) => { fs[0] }\n' 'apply = (cb: None | (int) -> int) => { 7 }\n' 'apply(pick([5]))')

    def test_plain_string_rejected(self):
        # Parity: a plain string is rejected in every executor (in PureVM it
        # must not pass as a pseudo builtin-by-name).
        with pytest.raises(Exception):
            run('pick = (fs) => { fs[0] }\n' 'apply = (cb: (int) -> int) => { 1 }\n' 'apply(pick(["hello"]))')

    def test_constructor_arity_mismatch_rejected(self):
        # Parity: constructor arity is enforced in every executor.
        with pytest.raises(Exception):
            run(
                'pick = (fs) => { fs[0] }\n'
                'struct P { x; y }\n'
                'apply = (cb: (int) -> P) => { 1 }\n'
                'apply(pick([P]))'
            )

    def test_list_element_arity_checked(self):
        # Parity: composite elements enforce arity, not just callability.
        with pytest.raises(Exception):
            run('pick = (xs) => { xs }\n' 'f = (hs: list[(int) -> int]) => { 1 }\n' 'f(pick([(a, b) => { a }]))')

    def test_nested_closure_return_checked(self):
        # The declared return follows the callback into inner closures.
        with pytest.raises(Exception):
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> int) => { w = () => { cb(1) }\n w() }\n'
                'apply(pick([(y) => { "menteur" }]))'
            )
        # An inner same-named param shadows: its calls stay unchecked.
        assert (
            run(
                'pick = (fs) => { fs[0] }\n'
                'apply = (cb: (int) -> int) => { w = (cb) => { cb(1) }\n w(pick([(a) => { "libre" }])) }\n'
                'apply(pick([(y) => { y }]))'
            )
            == 'libre'
        )
