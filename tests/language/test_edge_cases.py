# FILE: tests/language/test_edge_cases.py
"""
Edge-case tests for production-realistic scenarios.

Covers feature interactions that individual feature tests don't:
closures in loops, control flow inside match, pattern matching on
inherited structs, scoping subtleties, broadcast corner cases, etc.
"""

import pytest

from catnip.exc import CatnipPatternError, CatnipRuntimeError, CatnipTypeError

# ======================================================================
# 1. Closures in loops
# ======================================================================


class TestClosuresInLoops:
    """Classic closure-captures-loop-variable scenarios."""

    def test_closure_captures_for_variable(self, cat):
        """Each closure should capture its own iteration value."""
        code = """
        funcs = list()
        for i in list(1, 2, 3) {
            f = (n) => { () => { n } }
            funcs = funcs + list(f(i))
        }
        list(funcs[0](), funcs[1](), funcs[2]())
        """
        cat.parse(code)
        assert cat.execute() == [1, 2, 3]

    def test_closure_captures_for_variable_direct(self, cat):
        """Direct capture of loop variable -- what does Catnip do?"""
        code = """
        funcs = list()
        for i in list(1, 2, 3) {
            funcs = funcs + list(() => { i })
        }
        list(funcs[0](), funcs[1](), funcs[2]())
        """
        cat.parse(code)
        # If for-loop scopes each iteration: [1, 2, 3]
        # If shared capture (Python-like): [3, 3, 3]
        result = cat.execute()
        assert result == [1, 2, 3] or result == [3, 3, 3]

    def test_closure_captures_while_variable(self, cat):
        """Closure created inside while loop."""
        code = """
        funcs = list()
        i = 0
        while i < 3 {
            n = i
            funcs = funcs + list(() => { n })
            i = i + 1
        }
        list(funcs[0](), funcs[1](), funcs[2]())
        """
        cat.parse(code)
        result = cat.execute()
        # n is reassigned each iteration in same scope
        assert result == [0, 1, 2] or result == [2, 2, 2]

    def test_mutable_closure_counter(self, cat):
        """Closure with mutable state across calls."""
        code = """
        make_counter = () => {
            count = 0
            () => { count = count + 1; count }
        }
        counter = make_counter()
        list(counter(), counter(), counter())
        """
        cat.parse(code)
        assert cat.execute() == [1, 2, 3]

    def test_closure_in_for_with_accumulator(self, cat):
        """Closures built in a loop, each adding its captured value."""
        code = """
        adders = list()
        for i in list(10, 20, 30) {
            val = i
            adders = adders + list((x) => { x + val })
        }
        list(adders[0](1), adders[1](1), adders[2](1))
        """
        cat.parse(code)
        result = cat.execute()
        assert result == [11, 21, 31] or result == [31, 31, 31]


# ======================================================================
# 2. Control flow inside match
# ======================================================================


class TestControlFlowInMatch:
    """break, continue, return inside match arms."""

    def test_return_inside_match_inside_for(self, cat):
        """Return from match arm exits function."""
        code = """
        find_first_even = (lst) => {
            for x in lst {
                match x % 2 {
                    0 => { return x }
                    _ => { None }
                }
            }
            None
        }
        find_first_even(list(1, 3, 5, 4, 7))
        """
        cat.parse(code)
        assert cat.execute() == 4

    def test_break_inside_match(self, cat):
        """Break from match arm exits for loop."""
        code = """
        result = 0
        for i in list(1, 2, 3, 4, 5) {
            match i {
                3 => { break }
                n => { result = result + n }
            }
        }
        result
        """
        cat.parse(code)
        assert cat.execute() == 3  # 1 + 2

    def test_continue_inside_match(self, cat):
        """Continue from match arm skips rest of loop body."""
        code = """
        result = 0
        for i in list(1, 2, 3, 4, 5) {
            match i {
                3 => { continue }
                n => { result = result + n }
            }
        }
        result
        """
        cat.parse(code)
        assert cat.execute() == 12  # 1 + 2 + 4 + 5

    def test_break_inside_match_in_while(self, cat):
        """Break from match inside while loop."""
        code = """
        i = 0
        result = list()
        while True {
            match i {
                5 => { break }
                n if n % 2 == 0 => { result = result + list(n) }
                _ => { None }
            }
            i = i + 1
        }
        result
        """
        cat.parse(code)
        assert cat.execute() == [0, 2, 4]

    def test_nested_match(self, cat):
        """Match expression inside another match arm."""
        code = """
        classify = (x, y) => {
            match x {
                0 => {
                    match y {
                        0 => { "origin" }
                        _ => { "y-axis" }
                    }
                }
                _ => {
                    match y {
                        0 => { "x-axis" }
                        _ => { "plane" }
                    }
                }
            }
        }
        list(classify(0, 0), classify(0, 5), classify(3, 0), classify(3, 5))
        """
        cat.parse(code)
        assert cat.execute() == ["origin", "y-axis", "x-axis", "plane"]

    def test_return_from_nested_match(self, cat):
        """Return from deeply nested match."""
        code = """
        f = (x) => {
            match x > 0 {
                True => {
                    match x > 10 {
                        True => { return "big" }
                        _ => { return "small" }
                    }
                }
                _ => { return "negative" }
            }
        }
        list(f(20), f(5), f(-1))
        """
        cat.parse(code)
        assert cat.execute() == ["big", "small", "negative"]


# ======================================================================
# 3. Pattern matching on inherited structs
# ======================================================================


class TestPatternMatchInheritance:
    """Match on child struct with parent pattern and vice versa."""

    def test_child_matched_by_own_pattern(self, cat):
        """Child struct matched by its own type pattern."""
        code = """
        struct Shape { sides }
        struct Square extends(Shape) { size }
        s = Square(4, 10)
        match s {
            Square{sides, size} => { sides * size }
            _ => { -1 }
        }
        """
        cat.parse(code)
        assert cat.execute() == 40

    def test_match_multiple_struct_types(self, cat):
        """Dispatch on struct type in match arms."""
        code = """
        struct Animal { name }
        struct Dog extends(Animal) { breed }
        struct Cat extends(Animal) { color }
        classify = (a) => {
            match a {
                Dog{name, breed} => { name + " the " + breed }
                Cat{name, color} => { name + " (" + color + ")" }
                _ => { "unknown" }
            }
        }
        list(classify(Dog("Rex", "Lab")), classify(Cat("Mimi", "black")))
        """
        cat.parse(code)
        assert cat.execute() == ["Rex the Lab", "Mimi (black)"]

    def test_struct_pattern_with_guard_on_inherited_field(self, cat):
        """Guard accesses field inherited from parent."""
        code = """
        struct Shape { sides }
        struct Polygon extends(Shape) { name }
        match Polygon(3, "triangle") {
            Polygon{sides, name} if sides == 3 => { name }
            _ => { "other" }
        }
        """
        cat.parse(code)
        assert cat.execute() == "triangle"


# ======================================================================
# 4. Super edge cases
# ======================================================================


class TestSuperEdgeCases:
    """Super resolution in non-trivial contexts."""

    def test_super_init_chain(self, cat):
        """Super.init called through inheritance chain."""
        code = """
        struct A {
            trace = ""
            init(self) => { self.trace = self.trace + "A" }
        }
        struct B extends(A) {
            init(self) => { super.init(); self.trace = self.trace + "B" }
        }
        struct C extends(B) {
            init(self) => { super.init(); self.trace = self.trace + "C" }
        }
        C().trace
        """
        cat.parse(code)
        assert cat.execute() == "ABC"

    def test_super_method_in_override(self, cat):
        """super.method() calls parent version."""
        code = """
        struct Base {
            greet(self) => { "hello" }
        }
        struct Child extends(Base) {
            greet(self) => { super.greet() + " world" }
        }
        Child().greet()
        """
        cat.parse(code)
        assert cat.execute() == "hello world"


# ======================================================================
# 5. Scoping edge cases
# ======================================================================


class TestScopingEdgeCases:
    """Variable visibility, shadowing, scope leaks."""

    def test_block_does_not_shadow_outer(self, cat):
        """Assignment in block modifies outer scope (blocks don't create scope)."""
        code = """
        x = 10
        result = { x = 20; x }
        list(x, result)
        """
        cat.parse(code)
        result = cat.execute()
        # Blocks share scope with parent, so x is 20
        assert result == [20, 20]

    def test_for_variable_does_not_leak(self, cat):
        """For loop variable should not leak to outer scope."""
        code = """
        i = 100
        for i in list(1, 2, 3) {
            None
        }
        i
        """
        cat.parse(code)
        # i should be 100 if loop var is scoped
        assert cat.execute() == 100

    def test_nested_blocks_scope(self, cat):
        """Nested block assignment visibility."""
        code = """
        x = 1
        {
            x = 2
            {
                x = 3
            }
        }
        x
        """
        cat.parse(code)
        # blocks share scope, so x == 3
        assert cat.execute() == 3

    def test_variable_defined_in_if_branch(self, cat):
        """Variable assigned in if branch should be visible after."""
        code = """
        if True { x = 42 }
        x
        """
        cat.parse(code)
        assert cat.execute() == 42

    def test_variable_from_else_branch(self, cat):
        """Variable from else branch visible after if/else."""
        code = """
        if False { x = 1 }
        else { x = 2 }
        x
        """
        cat.parse(code)
        assert cat.execute() == 2


# ======================================================================
# 6. None interactions
# ======================================================================


class TestNoneEdgeCases:
    """None in various contexts."""

    def test_none_equality(self, cat):
        code = "None == None"
        cat.parse(code)
        assert cat.execute() is True

    def test_none_in_pattern_match(self, cat):
        code = """
        classify = (x) => {
            match x {
                None => { "nothing" }
                0 => { "zero" }
                n => { "something" }
            }
        }
        list(classify(None), classify(0), classify(42))
        """
        cat.parse(code)
        assert cat.execute() == ["nothing", "zero", "something"]

    def test_none_from_match_arm(self, cat):
        """Match arm returning None used in expression."""
        code = """
        x = match 5 {
            1 => { 10 }
            _ => { None }
        }
        x
        """
        cat.parse(code)
        assert cat.execute() is None

    def test_none_as_struct_field(self, cat):
        code = """
        struct Config { value = None }
        c = Config()
        c.value == None
        """
        cat.parse(code)
        assert cat.execute() is True

    def test_none_in_list(self, cat):
        code = "list(1, None, 3)"
        cat.parse(code)
        assert cat.execute() == [1, None, 3]


# ======================================================================
# 7. Match edge cases
# ======================================================================


class TestMatchEdgeCases:
    """Pattern matching corner cases."""

    def test_match_no_arm_hits(self, cat):
        """Match with no matching arm should raise error."""
        code = """
        match 42 {
            1 => { "one" }
            2 => { "two" }
        }
        """
        cat.parse(code)
        with pytest.raises((CatnipPatternError, CatnipRuntimeError)):
            cat.execute()

    def test_match_on_boolean(self, cat):
        code = """
        match True {
            True => { "yes" }
            False => { "no" }
        }
        """
        cat.parse(code)
        assert cat.execute() == "yes"

    def test_match_on_string(self, cat):
        code = """
        match "hello" {
            "hello" => { "greeting" }
            "bye" => { "farewell" }
            _ => { "unknown" }
        }
        """
        cat.parse(code)
        assert cat.execute() == "greeting"

    def test_match_or_pattern_with_guard(self, cat):
        """OR pattern combined with guard."""
        code = """
        classify = (x) => {
            match x {
                1 | 2 | 3 => { "small" }
                n if n > 100 => { "big" }
                _ => { "medium" }
            }
        }
        list(classify(2), classify(50), classify(200))
        """
        cat.parse(code)
        assert cat.execute() == ["small", "medium", "big"]

    def test_match_struct_guard_on_captured_field(self, cat):
        """Guard references captured struct fields."""
        code = """
        struct Point { x, y }
        match Point(3, 4) {
            Point{x, y} if x * x + y * y == 25 => { "on circle r=5" }
            _ => { "elsewhere" }
        }
        """
        cat.parse(code)
        assert cat.execute() == "on circle r=5"

    def test_match_as_expression(self, cat):
        """Match used directly in arithmetic."""
        code = """
        x = 10 + match 3 {
            1 => { 100 }
            3 => { 200 }
            _ => { 0 }
        }
        x
        """
        cat.parse(code)
        assert cat.execute() == 210


# ======================================================================
# 8. Struct edge cases
# ======================================================================


class TestStructEdgeCases2:
    """Struct corner cases not covered by test_struct_edge_cases.py."""

    def test_empty_struct(self, cat):
        """Struct with no fields."""
        code = """
        struct Empty {}
        e = Empty()
        str(e)
        """
        cat.parse(code)
        result = cat.execute()
        assert "Empty" in result

    def test_self_referential_struct(self, cat):
        """Struct field pointing to another instance of same type."""
        code = """
        struct Node { value, next = None }
        a = Node(1)
        b = Node(2, a)
        b.next.value
        """
        cat.parse(code)
        assert cat.execute() == 1

    def test_struct_equality_nested(self, cat):
        """Deep structural equality with nested structs."""
        code = """
        struct Point { x, y }
        struct Line { start, end }
        l1 = Line(Point(0, 0), Point(1, 1))
        l2 = Line(Point(0, 0), Point(1, 1))
        l3 = Line(Point(0, 0), Point(2, 2))
        list(l1 == l2, l1 == l3)
        """
        cat.parse(code)
        assert cat.execute() == [True, False]

    def test_method_returns_self_chaining(self, cat):
        """Method returning self enables chaining."""
        code = """
        struct Builder {
            items
            add(self, item) => {
                self.items = self.items + list(item)
                self
            }
        }
        Builder(list()).add(1).add(2).add(3).items
        """
        cat.parse(code)
        assert cat.execute() == [1, 2, 3]

    def test_operator_overload_in_child(self, cat):
        """Child struct defines operator, parent doesn't."""
        code = """
        struct Base { x }
        struct Child extends(Base) {
            op +(self, rhs) => { Child(self.x + rhs.x) }
        }
        c = Child(1) + Child(2)
        c.x
        """
        cat.parse(code)
        assert cat.execute() == 3


# ======================================================================
# 9. TCO + closures / structs
# ======================================================================


class TestTCOInteractions:
    """Tail-call optimization with closures and structs."""

    def test_tco_function_returning_closures(self, cat):
        """Tail-recursive function that builds a list of closures."""
        code = """
        make_adders = (n, acc) => {
            if n == 0 { acc }
            else {
                val = n
                new_acc = acc + list((x) => { x + val })
                make_adders(n - 1, new_acc)
            }
        }
        adders = make_adders(3, list())
        list(adders[0](10), adders[1](10), adders[2](10))
        """
        cat.parse(code)
        result = cat.execute()
        # adders[0] was created when n=3, adders[1] when n=2, adders[2] when n=1
        assert result == [13, 12, 11]

    def test_tco_struct_method_deep_recursion(self, cat):
        """Recursive struct method with enough depth to blow stack without TCO."""
        code = """
        struct Counter {
            n
            count_down(self) => {
                if self.n <= 0 { 0 }
                else { Counter(self.n - 1).count_down() }
            }
        }
        Counter(500).count_down()
        """
        cat.parse(code)
        assert cat.execute() == 0

    def test_tco_mutual_like_via_match(self, cat):
        """Match-based dispatch simulating mutual recursion."""
        code = """
        f = (n, which) => {
            if n <= 0 { n }
            else {
                match which {
                    "a" => { f(n - 1, "b") }
                    "b" => { f(n - 1, "a") }
                }
            }
        }
        f(100, "a")
        """
        cat.parse(code)
        assert cat.execute() == 0


# ======================================================================
# 10. Broadcast edge cases
# ======================================================================


class TestBroadcastEdgeCases:
    """Broadcasting in non-trivial contexts."""

    def test_broadcast_on_struct_with_operator_overload(self, cat):
        """Broadcast * on structs that overload *."""
        code = """
        struct Num {
            val
            op *(self, rhs) => { Num(self.val * rhs) }
        }
        result = list(Num(1), Num(2), Num(3)).[* 10]
        list(result[0].val, result[1].val, result[2].val)
        """
        cat.parse(code)
        assert cat.execute() == [10, 20, 30]

    def test_broadcast_with_lambda(self, cat):
        """Broadcast map with lambda."""
        code = """
        list(1, 2, 3).[~> (x) => { x * x }]
        """
        cat.parse(code)
        assert cat.execute() == [1, 4, 9]

    def test_broadcast_filter_basic(self, cat):
        """Broadcast filter."""
        code = """
        list(1, 2, 3, 4, 5).[if > 3]
        """
        cat.parse(code)
        assert cat.execute() == [4, 5]

    def test_broadcast_deep_three_levels(self, cat):
        """Three levels of nesting with broadcast."""
        code = """
        list(list(list(1, 2), list(3)), list(list(4))).[+ 100]
        """
        cat.parse(code)
        assert cat.execute() == [[[101, 102], [103]], [[104]]]

    def test_broadcast_on_empty(self, cat):
        """Broadcast on empty list."""
        code = """
        list().[* 2]
        """
        cat.parse(code)
        assert cat.execute() == []

    def test_broadcast_chained(self, cat):
        """Two broadcasts chained."""
        code = """
        list(1, 2, 3).[* 2].[+ 1]
        """
        cat.parse(code)
        assert cat.execute() == [3, 5, 7]


# ======================================================================
# 11. Trait interactions
# ======================================================================


class TestTraitInteractions:
    """Traits in complex combinations."""

    def test_trait_method_calls_another_trait_method(self, cat):
        """Default trait method delegates to another trait method."""
        code = """
        trait Measurable {
            width(self) => { self.w }
            height(self) => { self.h }
            area(self) => { self.width() * self.height() }
        }
        struct Rect implements(Measurable) { w, h }
        Rect(3, 4).area()
        """
        cat.parse(code)
        assert cat.execute() == 12

    def test_trait_with_abstract_and_default(self, cat):
        """Trait with abstract method used by default method."""
        code = """
        trait Formattable {
            @abstract
            format(self)
            display(self) => { "[" + self.format() + "]" }
        }
        struct Item implements(Formattable) {
            name
            format(self) => { self.name }
        }
        Item("test").display()
        """
        cat.parse(code)
        assert cat.execute() == "[test]"

    def test_trait_init_interaction(self, cat):
        """Trait method usable from init."""
        code = """
        trait Validatable {
            is_valid(self) => { self.x > 0 }
        }
        struct PositiveNumber implements(Validatable) {
            x
            init(self) => {
                if not self.is_valid() {
                    self.x = 0
                }
            }
        }
        list(PositiveNumber(5).x, PositiveNumber(-3).x)
        """
        cat.parse(code)
        assert cat.execute() == [5, 0]


# ======================================================================
# 12. Operator overload interactions
# ======================================================================


class TestOperatorOverloadInteractions:
    """Operator overloads in match guards, mixed types, etc."""

    def test_overloaded_eq_in_match_guard(self, cat):
        """Custom == used in match guard."""
        code = """
        struct Version {
            major, minor
            op ==(self, rhs) => { self.major == rhs.major and self.minor == rhs.minor }
        }
        v = Version(1, 0)
        target = Version(1, 0)
        match v {
            x if x == target => { "match" }
            _ => { "no match" }
        }
        """
        cat.parse(code)
        assert cat.execute() == "match"

    def test_mixed_type_operator(self, cat):
        """Operator overload handling both struct and scalar rhs."""
        code = """
        struct Vec2 {
            x, y
            op +(self, rhs) => {
                match rhs {
                    Vec2{x, y} => { Vec2(self.x + x, self.y + y) }
                    n => { Vec2(self.x + n, self.y + n) }
                }
            }
        }
        v = Vec2(1, 2)
        r1 = v + Vec2(3, 4)
        r2 = v + 10
        list(r1.x, r1.y, r2.x, r2.y)
        """
        cat.parse(code)
        assert cat.execute() == [4, 6, 11, 12]


# ======================================================================
# 13. F-strings with structs and methods
# ======================================================================


class TestFStringInteractions:
    """F-strings with struct fields, methods, and expressions."""

    def test_fstring_struct_field(self, cat):
        code = """
        struct Point { x, y }
        p = Point(3, 4)
        f"({p.x}, {p.y})"
        """
        cat.parse(code)
        assert cat.execute() == "(3, 4)"

    def test_fstring_method_call(self, cat):
        code = """
        struct Named { name; upper(self) => { self.name + "!" } }
        n = Named("hello")
        f"result: {n.upper()}"
        """
        cat.parse(code)
        assert cat.execute() == "result: hello!"

    def test_fstring_in_loop(self, cat):
        code = """
        result = list()
        for i in list(0, 1, 2) {
            result = result + list(f"item {i}")
        }
        result
        """
        cat.parse(code)
        assert cat.execute() == ["item 0", "item 1", "item 2"]

    def test_fstring_with_match_expression(self, cat):
        code = """
        label = (x) => {
            match x {
                1 => { "one" }
                2 => { "two" }
                _ => { "other" }
            }
        }
        f"got: {label(2)}"
        """
        cat.parse(code)
        assert cat.execute() == "got: two"


# ======================================================================
# 14. Variadic methods on structs
# ======================================================================


class TestVariadicMethods:
    """Variadic parameters on struct methods."""

    def test_variadic_method(self, cat):
        code = """
        struct Logger {
            prefix
            log(self, *args) => {
                result = self.prefix
                for arg in args {
                    result = result + " " + str(arg)
                }
                result
            }
        }
        Logger("[INFO]").log("hello", "world")
        """
        cat.parse(code)
        assert cat.execute() == "[INFO] hello world"

    def test_variadic_function_with_match(self, cat):
        code = """
        first_or_default = (*args) => {
            match len(args) {
                0 => { None }
                _ => { args[0] }
            }
        }
        list(first_or_default(), first_or_default(42, 99))
        """
        cat.parse(code)
        assert cat.execute() == [None, 42]


# ======================================================================
# 15. Block expression edge cases
# ======================================================================


class TestBlockEdgeCases:
    """Blocks as expressions in various positions."""

    def test_block_as_function_arg(self, cat):
        code = """
        add = (a, b) => { a + b }
        add({ 10 + 20 }, { 3 * 4 })
        """
        cat.parse(code)
        assert cat.execute() == 42

    def test_block_in_arithmetic(self, cat):
        code = """
        { 10 } + { 20 } * { 3 }
        """
        cat.parse(code)
        assert cat.execute() == 70  # 10 + 60

    def test_block_returning_last_expression(self, cat):
        code = """
        x = {
            a = 10
            b = 20
            a + b
        }
        x
        """
        cat.parse(code)
        assert cat.execute() == 30

    def test_return_from_block_inside_function(self, cat):
        """Return inside a block should exit the enclosing function."""
        code = """
        f = () => {
            x = {
                return 42
                99
            }
            x + 1
        }
        f()
        """
        cat.parse(code)
        assert cat.execute() == 42


# ======================================================================
# 16. Misc production patterns
# ======================================================================


class TestProductionPatterns:
    """Patterns commonly found in production code."""

    def test_accumulate_with_match_and_struct(self, cat):
        """Process a list of mixed types using match."""
        code = """
        struct Ok { value }
        struct Err { msg }
        results = list(Ok(1), Err("fail"), Ok(3), Ok(4), Err("oops"))
        total = 0
        errors = 0
        for r in results {
            match r {
                Ok{value} => { total = total + value }
                Err{msg} => { errors = errors + 1 }
            }
        }
        list(total, errors)
        """
        cat.parse(code)
        assert cat.execute() == [8, 2]

    def test_recursive_data_structure_processing(self, cat):
        """Process a linked list recursively."""
        code = """
        struct Node { value, next = None }
        sum_list = (node) => {
            if node == None { 0 }
            else { node.value + sum_list(node.next) }
        }
        chain = Node(1, Node(2, Node(3, Node(4))))
        sum_list(chain)
        """
        cat.parse(code)
        assert cat.execute() == 10

    def test_struct_factory_pattern(self, cat):
        """Function that creates different struct types."""
        code = """
        struct Circle { radius }
        struct Square { side }
        make_shape = (kind, size) => {
            match kind {
                "circle" => { Circle(size) }
                "square" => { Square(size) }
            }
        }
        c = make_shape("circle", 5)
        s = make_shape("square", 3)
        list(c.radius, s.side)
        """
        cat.parse(code)
        assert cat.execute() == [5, 3]

    def test_higher_order_with_struct_methods(self, cat):
        """Pass a method reference around."""
        code = """
        struct Transformer {
            factor
            transform(self, x) => { x * self.factor }
        }
        t = Transformer(3)
        apply = (f, val) => { f(val) }
        apply(t.transform, 7)
        """
        cat.parse(code)
        assert cat.execute() == 21

    def test_swap_via_unpacking_in_loop(self, cat):
        """Repeated swap via tuple unpacking."""
        code = """
        a = 1
        b = 2
        for _ in range(3) {
            (a, b) = tuple(b, a)
        }
        list(a, b)
        """
        cat.parse(code)
        assert cat.execute() == [2, 1]

    def test_fibonacci_via_match(self, cat):
        """Classic fib using match for base cases."""
        code = """
        fib = (n) => {
            match n {
                0 => { 0 }
                1 => { 1 }
                n => { fib(n - 1) + fib(n - 2) }
            }
        }
        list(fib(0), fib(1), fib(5), fib(10))
        """
        cat.parse(code)
        assert cat.execute() == [0, 1, 5, 55]

    def test_state_machine_with_match(self, cat):
        """Simple state machine using match in a loop."""
        code = """
        state = "start"
        input = list("a", "b", "a", "end")
        count = 0
        for ch in input {
            state = match state {
                "start" => {
                    match ch {
                        "a" => { count = count + 1; "seen_a" }
                        _ => { "start" }
                    }
                }
                "seen_a" => {
                    match ch {
                        "b" => { count = count + 10; "start" }
                        "a" => { count = count + 1; "seen_a" }
                        _ => { "start" }
                    }
                }
                _ => { "start" }
            }
        }
        list(state, count)
        """
        cat.parse(code)
        # start -> "a" -> seen_a (count=1) -> "b" -> start (count=11)
        # -> "a" -> seen_a (count=12) -> "end" -> start (count=12)
        assert cat.execute() == ["start", 12]


# ======================================================================
# 17. Filthy edge cases
# ======================================================================


class TestFilthyEdgeCases:
    """Intentionally nasty feature-interaction tests."""

    def test_guard_not_evaluated_when_pattern_does_not_match(self, cat):
        """Guard side effects must not run if structural pattern already fails."""
        code = """
        hits = 0
        bump = () => { hits = hits + 1; True }
        match 42 {
            (x, y) if bump() => { "tuple" }
            _ => { hits }
        }
        """
        cat.parse(code)
        assert cat.execute() == 0

    def test_guard_exception_propagates_without_fallback(self, cat):
        """If guard raises, match must fail immediately (no silent fallback)."""
        code = """
        boom = () => { 1 / 0 }
        match 1 {
            n if boom() => { "ok" }
            _ => { "fallback" }
        }
        """
        cat.parse(code)
        with pytest.raises(Exception, match="division by zero"):
            cat.execute()

    def test_unpacking_is_atomic_on_nested_failure(self, cat):
        """Failed nested unpack must not partially overwrite existing vars."""
        code = """
        a = 111
        b = 222
        c = 333
        (a, (b, c)) = list(9, 2)
        """
        cat.parse(code)
        with pytest.raises((CatnipTypeError, CatnipRuntimeError)):
            cat.execute()
        assert cat.context.globals["a"] == 111
        assert cat.context.globals["b"] == 222
        assert cat.context.globals["c"] == 333

    def test_failed_guard_does_not_leak_capture_to_outer_scope(self, cat):
        """Captured vars in a failed guard arm must not mutate outer binding."""
        code = """
        n = 99
        match 5 {
            n if n > 10 => { "big" }
            _ => { "small" }
        }
        n
        """
        cat.parse(code)
        assert cat.execute() == 99

    def test_for_unpacking_failure_keeps_previous_iteration_side_effects(self, cat):
        """Loop should keep effects from completed iterations before unpack error."""
        code = """
        total = 0
        for (a, b) in list(tuple(1, 2), 3, tuple(4, 5)) {
            total = total + a + b
        }
        """
        cat.parse(code)
        with pytest.raises((CatnipTypeError, CatnipRuntimeError)):
            cat.execute()
        assert cat.context.globals["total"] == 3
