# FILE: tests/serial/jit/test_inlining.py
"""
Tests d'inline de fonctions pures dans le JIT.

Vérifie que les fonctions marquées @pure sont correctement inlinées
dans les hot loops pour réduire l'overhead des appels de fonction.
"""

import pytest

from tests.serial.jit.conftest import compile_code

# Force serial execution to avoid JIT state conflicts
pytestmark = pytest.mark.xdist_group(name="jit")


class TestPureFunctionInlining:
    """Tests d'inline de fonctions pures."""

    def test_inline_builtin_pure(self, vm_with_jit):
        """Inline de builtin pure (abs) dans hot loop."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            # Call abs() in hot loop - should be inlined
            sum = 0
            i = 0
            while i < 120 {
                sum = sum + abs(i - 60)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        # Sum of |i - 60| for i in [0, 120)
        # = sum(60-i for i in 0..60) + sum(i-60 for i in 60..120)
        # = 60*61/2 + 60*59/2 = 1830 + 1770 = 3600
        expected = sum(abs(i - 60) for i in range(120))
        assert result == expected

    def test_inline_user_pure_simple(self, vm_with_jit):
        """Inline de fonction user marquée @pure - cas simple."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            # Define pure function
            square = (x) => { x * x }

            # Mark as pure via context
            # (decorator @pure not available in bytecode mode)

            # Call in hot loop
            sum = 0
            i = 0
            while i < 120 {
                sum = sum + square(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        # Sum of i^2 for i in [0, 120)
        expected = sum(i * i for i in range(120))
        assert result == expected

    def test_inline_arithmetic_pure(self, vm_with_jit):
        """Inline de fonction pure arithmétique."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            # Pure function with multiple ops
            calc = (x) => { (x + 1) * 2 - 1 }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + calc(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        # Sum of (i+1)*2-1 for i in [0, 120)
        expected = sum((i + 1) * 2 - 1 for i in range(120))
        assert result == expected

    def test_inline_conditional_pure(self, vm_with_jit):
        """Inline de fonction pure avec branche conditionnelle."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            # Pure function with if/else
            clamp = (x) => {
                if x < 0 {
                    0
                } elif x > 100 {
                    100
                } else {
                    x
                }
            }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + clamp(i - 10)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        # Sum of clamp(i-10, 0, 100) for i in [0, 120)
        expected = sum(max(0, min(100, i - 10)) for i in range(120))
        assert result == expected

    def test_inline_size_limit(self, vm_with_jit):
        """Fonction trop grosse ne devrait PAS être inlinée."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            # Large function with >20 ops (should NOT inline)
            huge = (x) => {
                a = x + 1
                b = a * 2
                c = b - 3
                d = c + 4
                e = d * 5
                f = e - 6
                g = f + 7
                h = g * 8
                val_i = h - 9
                j = val_i + 10
                k = j * 11
                l = k - 12
                m = l + 13
                n = m * 14
                o = n - 15
                p = o + 16
                q = p * 17
                r = q - 18
                s = r + 19
                t = s * 20
                t
            }

            # Should still execute correctly (fallback to normal call)
            sum = 0
            idx = 0
            while idx < 120 {
                sum = sum + huge(idx)
                idx = idx + 1
            }
            sum
        }
        """)

        # Should execute without error (graceful fallback)
        result = vm.execute(code, (), {}, None)

        # Verify result is correct (even if not inlined)
        def huge(x):
            a = x + 1
            b = a * 2
            c = b - 3
            d = c + 4
            e = d * 5
            f = e - 6
            g = f + 7
            h = g * 8
            val_i = h - 9
            j = val_i + 10
            k = j * 11
            val_l = k - 12
            m = val_l + 13
            n = m * 14
            o = n - 15
            p = o + 16
            q = p * 17
            r = q - 18
            s = r + 19
            t = s * 20
            return t

        expected = sum(huge(idx) for idx in range(120))
        assert result == expected

    def test_inline_nested_pure(self, vm_with_jit):
        """Inline transitive : f→g, les deux pures."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            # Two-level pure function calls
            double = (x) => { x * 2 }
            quad = (x) => { double(double(x)) }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + quad(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        # Sum of i*4 for i in [0, 120)
        expected = sum(i * 4 for i in range(120))
        assert result == expected


class TestInliningCorrectness:
    """Vérifie que l'inline ne change pas les résultats."""

    def test_jit_vs_no_jit_equivalence(self, vm_with_jit, vm_without_jit):
        """Même résultat avec et sans JIT."""
        code_str = """
        {
            square = (x) => { x * x }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + square(i)
                i = i + 1
            }
            sum
        }
        """

        code = compile_code(code_str)

        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit

        result_jit = vm_jit.execute(code, (), {}, None)
        result_no_jit = vm_no_jit.execute(code, (), {}, None)

        assert result_jit == result_no_jit

    def test_builtin_inline_correctness(self, vm_with_jit, vm_without_jit):
        """Builtin inliné donne même résultat."""
        code_str = """
        {
            sum = 0
            i = 0
            while i < 120 {
                sum = sum + abs(i - 60) + min(i, 50) + max(i, 10)
                i = i + 1
            }
            sum
        }
        """

        code = compile_code(code_str)

        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit

        result_jit = vm_jit.execute(code, (), {}, None)
        result_no_jit = vm_no_jit.execute(code, (), {}, None)

        assert result_jit == result_no_jit


class TestPureRegistration:
    """Vérifie que @pure enregistre les fonctions dans le JIT."""

    def test_pure_decorator_registers_in_jit(self, vm_with_jit):
        """@pure + hot loop doit enregistrer la fonction pour inlining."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            @pure
            square = (x) => { x * x }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + square(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(i * i for i in range(120))
        assert result == expected

    def test_pure_decorator_correctness(self, vm_with_jit, vm_without_jit):
        """@pure ne change pas le résultat vs non-JIT."""
        code_str = """
        {
            @pure
            add_one = (x) => { x + 1 }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + add_one(i)
                i = i + 1
            }
            sum
        }
        """

        code = compile_code(code_str)
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit

        result_jit = vm_jit.execute(code, (), {}, None)
        result_no_jit = vm_no_jit.execute(code, (), {}, None)

        assert result_jit == result_no_jit
        assert result_jit == sum(i + 1 for i in range(120))

    def test_non_pure_not_registered(self, vm_with_jit):
        """Fonction sans @pure ne devrait pas être inlinée."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            double = (x) => { x * 2 }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + double(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(i * 2 for i in range(120))
        assert result == expected


class TestNewBuiltinJIT:
    """Tests des nouveaux builtins purs dans le JIT (round, int, bool, float)."""

    def test_builtin_round_in_hot_loop(self, vm_with_jit):
        """round() dans hot loop - identity sur int."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + round(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(round(i) for i in range(200))
        assert result == expected

    def test_builtin_int_in_hot_loop(self, vm_with_jit):
        """int() dans hot loop - identity sur int."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + int(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(int(i) for i in range(200))
        assert result == expected

    def test_builtin_bool_in_hot_loop(self, vm_with_jit):
        """bool() dans hot loop - x != 0."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + bool(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        # bool(0) = 0, bool(1..199) = 1 each
        expected = sum(bool(i) for i in range(200))
        assert result == expected

    def test_builtin_float_via_callback(self, vm_with_jit):
        """float() dans hot loop via CallBuiltinPure."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0.0
            i = 0
            while i < 200 {
                sum = sum + float(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(float(i) for i in range(200))
        assert result == expected

    def test_builtin_round_correctness(self, vm_with_jit, vm_without_jit):
        """round() donne meme resultat avec et sans JIT."""
        code_str = """
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + round(i * 3)
                i = i + 1
            }
            sum
        }
        """

        code = compile_code(code_str)
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit

        result_jit = vm_jit.execute(code, (), {}, None)
        result_no_jit = vm_no_jit.execute(code, (), {}, None)

        assert result_jit == result_no_jit


class TestBuiltinJIT:
    """Tests de compilation JIT des builtins purs (abs, min, max)."""

    def test_builtin_abs_in_hot_loop(self, vm_with_jit):
        """abs() dans hot loop doit etre compile par le JIT."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + abs(i - 100)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(abs(i - 100) for i in range(200))
        assert result == expected

    def test_builtin_min_in_hot_loop(self, vm_with_jit):
        """min() dans hot loop."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + min(i, 50)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(min(i, 50) for i in range(200))
        assert result == expected

    def test_builtin_max_in_hot_loop(self, vm_with_jit):
        """max() dans hot loop."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + max(i, 100)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(max(i, 100) for i in range(200))
        assert result == expected

    def test_builtin_combined_in_hot_loop(self, vm_with_jit):
        """abs + min + max combines dans une hot loop."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + abs(i - 100) + min(i, 50) + max(i, 150)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(abs(i - 100) + min(i, 50) + max(i, 150) for i in range(200))
        assert result == expected

    def test_builtin_abs_negative(self, vm_with_jit):
        """abs() avec valeurs negatives."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            sum = 0
            i = 0
            while i < 200 {
                sum = sum + abs(0 - i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        expected = sum(abs(-i) for i in range(200))
        assert result == expected


class TestInliningEdgeCases:
    """Cas limites de l'inline."""

    def test_inline_with_zero_args(self, vm_with_jit):
        """Fonction pure sans arguments."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            get_constant = () => { 42 }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + get_constant()
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        assert result == 42 * 120

    def test_inline_with_multiple_args(self, vm_with_jit):
        """Fonction pure avec plusieurs arguments."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            add3 = (a, b, c) => { a + b + c }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + add3(i, i * 2, i * 3)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        # Sum of i + 2i + 3i = 6i for i in [0, 120)
        expected = sum(i * 6 for i in range(120))
        assert result == expected

    def test_inline_preserves_variable_scope(self, vm_with_jit):
        """L'inline préserve le scope des variables (closures)."""
        vm, c = vm_with_jit
        code = compile_code("""
        {
            outer = 100

            use_outer = (x) => { x + outer }

            sum = 0
            i = 0
            while i < 120 {
                sum = sum + use_outer(i)
                i = i + 1
            }
            sum
        }
        """)

        result = vm.execute(code, (), {}, None)
        # Sum of (i + 100) for i in [0, 120)
        expected = sum(i + 100 for i in range(120))
        assert result == expected


class TestUserPureInliningCompiles:
    """A hot loop calling a scope-resolved @pure helper must actually JIT-compile
    (the inlined call replacing the interpreted CallPure), not merely return the
    right value via the interpreter.

    The helper is defined at module scope and the loop lives inside a function,
    so inside that function the helper is loaded via LoadScope (not a frame
    local). This is the v1 inlining target: the function value is never on the
    JIT stack, the callee body is reconstructed from the registry, and a
    function-identity guard keeps it sound across reassignment.
    """

    def test_scope_pure_helper_compiles_and_is_correct(self, vm_with_jit, vm_without_jit):
        """Single-arg @pure helper: loop compiles and matches the interpreter."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        code = compile_code("""
        @pure
        square = (x) => { x * x }

        run = () => {
            total = 0
            i = 0
            while i < 5000 {
                total = total + square(i)
                i = i + 1
            }
            total
        }
        run()
        """)
        result = vm_jit.execute(code, (), {}, None)
        assert result == vm_no_jit.execute(code, (), {}, None)
        assert result == sum(i * i for i in range(5000))
        assert vm_jit.get_jit_stats()['compiled_loops'] >= 1

    def test_multi_arg_pure_helper_arg_order(self, vm_with_jit, vm_without_jit):
        """Two-arg @pure helper: arg binding order is correct (a - b + a, not b - a)."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        code = compile_code("""
        @pure
        f = (a, b) => { a - b + a }

        run = () => {
            total = 0
            i = 0
            while i < 5000 {
                total = total + f(i, 3)
                i = i + 1
            }
            total
        }
        run()
        """)
        result = vm_jit.execute(code, (), {}, None)
        assert result == vm_no_jit.execute(code, (), {}, None)
        assert result == sum((i - 3 + i) for i in range(5000))
        assert vm_jit.get_jit_stats()['compiled_loops'] >= 1

    def test_reassigned_helper_falls_back_soundly(self, vm_with_jit):
        """Reassigning the helper between two runs of the same loop must NOT run the
        stale inlined body: the function-identity guard fails and the loop falls
        back to the interpreter, so the second run reflects the new helper."""
        vm, _ = vm_with_jit
        code = compile_code("""
        @pure
        helper = (x) => { x * x }

        run = () => {
            total = 0
            i = 0
            while i < 5000 {
                total = total + helper(i)
                i = i + 1
            }
            total
        }
        r1 = run()
        helper = (x) => { x * x * x }
        r2 = run()
        tuple(r1, r2)
        """)
        r1, r2 = vm.execute(code, (), {}, None)
        assert r1 == sum(i * i for i in range(5000))
        # If the guard were missing, r2 would still be sum(i*i) (stale body).
        assert r2 == sum(i * i * i for i in range(5000))

    def test_pure_call_loop_does_not_poison_later_loop(self, vm_with_jit):
        """A loop calling a pure helper increments the trace-recording call-depth.
        That depth must not leak into a subsequent loop's trace (it would suspend
        recording and prevent compilation). A plain arithmetic loop following the
        pure-call loop must still compile."""
        vm, _ = vm_with_jit
        code = compile_code("""
        @pure
        h = (x) => { x + 1 }

        run = () => {
            t = 0
            i = 0
            while i < 5000 { t = t + h(i); i = i + 1 }
            s = 0
            j = 0
            while j < 5000 { s = s + j; j = j + 1 }
            tuple(t, s)
        }
        run()
        """)
        t, s = vm.execute(code, (), {}, None)
        assert t == sum(i + 1 for i in range(5000))
        assert s == sum(range(5000))
        # Both loops must compile: the second proves no call-depth poison leaked.
        assert vm.get_jit_stats()['compiled_loops'] >= 2


class TestLocalSlotPureInlining:
    """A @pure helper held in a frame LOCAL slot (defined inside the same function
    as the loop, so loaded via LoadLocal) is also inlined.

    The function load is elided from the trace (no value on the JIT stack, no
    bogus GuardFloat); a slot-keyed identity guard read from frame.locals keeps
    it sound, and those slots are excluded from the numeric-only local scan.
    """

    def test_local_pure_helper_compiles_and_is_correct(self, vm_with_jit, vm_without_jit):
        """Single-arg @pure helper in a local slot: loop compiles, matches interp."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        code = compile_code("""
        run = () => {
            @pure
            sq = (x) => { x * x }
            total = 0
            i = 0
            while i < 5000 {
                total = total + sq(i)
                i = i + 1
            }
            total
        }
        run()
        """)
        result = vm_jit.execute(code, (), {}, None)
        assert result == vm_no_jit.execute(code, (), {}, None)
        assert result == sum(i * i for i in range(5000))
        assert vm_jit.get_jit_stats()['compiled_loops'] >= 1

    def test_local_multi_arg_helper_arg_order(self, vm_with_jit, vm_without_jit):
        """Two-arg local @pure helper: arg binding order is correct (a - b + a)."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        code = compile_code("""
        run = () => {
            @pure
            f = (a, b) => { a - b + a }
            total = 0
            i = 0
            while i < 5000 { total = total + f(i, 3); i = i + 1 }
            total
        }
        run()
        """)
        result = vm_jit.execute(code, (), {}, None)
        assert result == vm_no_jit.execute(code, (), {}, None)
        assert result == sum(i - 3 + i for i in range(5000))
        assert vm_jit.get_jit_stats()['compiled_loops'] >= 1

    def test_local_helper_reassigned_in_loop_is_sound(self, vm_with_jit, vm_without_jit):
        """Reassigning the local helper inside the loop (a StoreLocal to its slot)
        drops the trace, so the loop runs interpreted with the live helper instead
        of a stale inlined body."""
        vm_jit, _ = vm_with_jit
        vm_no_jit, _ = vm_without_jit
        code = compile_code("""
        run = () => {
            @pure
            h = (x) => { x * x }
            total = 0
            i = 0
            while i < 5000 {
                total = total + h(i)
                h = (x) => { x + x }
                i = i + 1
            }
            total
        }
        run()
        """)
        result = vm_jit.execute(code, (), {}, None)
        assert result == vm_no_jit.execute(code, (), {}, None)
