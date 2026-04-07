# FILE: tests/tools/test_linter.py
"""Tests for the Catnip linter (Rust implementation)."""

import pytest
from catnip.tools.linter import lint_code, lint_file, LintResult
from catnip._rs import Severity, Diagnostic, LintConfig, lint_code as rs_lint_code


class TestSyntax:
    """Syntax checking (E100)."""

    def test_valid_code(self):
        result = lint_code("x = 1 + 2", check_style=False, check_semantic=False)
        assert not result.has_errors

    def test_syntax_error(self):
        result = lint_code("if {", check_style=False, check_semantic=False)
        assert result.has_errors
        assert any(d.code == 'E100' for d in result.diagnostics)

    def test_syntax_error_stops_analysis(self):
        """Syntax errors should prevent style/semantic checks."""
        result = lint_code("if {")
        # Only syntax errors, no style/semantic
        assert all(d.code.startswith('E1') for d in result.diagnostics)

    def test_empty_code(self):
        result = lint_code("", check_style=False, check_semantic=False)
        assert not result.has_errors


class TestStyle:
    """Style checking (W100, W101, W102)."""

    def test_well_formatted(self):
        result = lint_code("x = 1 + 2\n", check_syntax=False, check_semantic=False)
        style_diags = [d for d in result.diagnostics if d.code.startswith('W2')]
        assert len(style_diags) == 0

    def test_spacing_issue(self):
        result = lint_code("x=1+2\n", check_syntax=False, check_semantic=False)
        assert any(d.code == 'W100' for d in result.diagnostics)

    def test_trailing_whitespace(self):
        result = lint_code("x = 1  \n", check_syntax=False, check_semantic=False)
        assert any(d.code == 'W101' for d in result.diagnostics)

    def test_suggestion_provided(self):
        result = lint_code("x=1\n", check_syntax=False, check_semantic=False)
        w200 = [d for d in result.diagnostics if d.code == 'W100']
        assert len(w200) > 0
        assert w200[0].suggestion is not None


class TestSemantic:
    """Semantic checking (E200, W200)."""

    def test_used_variable_no_warning(self):
        result = lint_code("x = 1\nlen(x)", check_syntax=False, check_style=False)
        warnings = [d for d in result.diagnostics if d.code == 'W200']
        assert len(warnings) == 0

    def test_lambda_params_defined(self):
        result = lint_code("f = (x, y) => { x + y }", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E200']
        assert len(errors) == 0

    def test_for_loop_var_defined(self):
        result = lint_code("for i in range(10) { abs(i) }", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E200']
        assert len(errors) == 0

    def test_match_capture_defined(self):
        result = lint_code("x = 1\nmatch x { n => { abs(n) } }", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E200']
        assert len(errors) == 0

    def test_builtins_predefined(self):
        result = lint_code("len(range(10))", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E200']
        assert len(errors) == 0

    def test_nested_scopes(self):
        """Variables from outer scope visible in inner scope."""
        result = lint_code("x = 1\nf = () => { abs(x) }", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E200']
        assert len(errors) == 0

    def test_inner_scope_not_leaking(self):
        """Variables from inner scope not visible in outer scope."""
        result = lint_code("f = () => { y = 1 }\nabs(y)", check_syntax=False, check_style=False, check_names=True)
        errors = [d for d in result.diagnostics if d.code == 'E200']
        assert len(errors) == 1
        assert "'y'" in errors[0].message

    def test_self_referencing_function(self):
        """Function can reference itself (recursion)."""
        result = lint_code(
            "fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }", check_syntax=False, check_style=False
        )
        errors = [d for d in result.diagnostics if d.code == 'E200']
        assert len(errors) == 0

    def test_multiple_assignment(self):
        result = lint_code("x = 1\ny = 2\nz = x + y", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E200']
        assert len(errors) == 0


class TestIntegration:
    """Full pipeline integration tests."""

    def test_full_pipeline(self):
        code = "x = 1\ny = x + 1\nabs(y)"
        result = lint_code(code)
        assert not result.has_errors

    def test_lint_result_summary(self):
        result = lint_code("y = undefined_var", check_names=True)
        assert 'error' in result.summary()

    def test_lint_result_properties(self):
        result = lint_code("x = 1", check_syntax=False, check_style=False)
        assert isinstance(result, LintResult)
        assert isinstance(result.warnings, list)
        assert isinstance(result.errors, list)

    def test_diagnostics_sorted(self):
        """Diagnostics should be sorted by line, then column."""
        result = lint_code("b = a\nd = c", check_syntax=False, check_style=False)
        lines = [d.line for d in result.diagnostics if d.code == 'E200']
        assert lines == sorted(lines)


class TestRustDirect:
    """Direct Rust API tests."""

    def test_rs_lint_code_returns_list(self):
        config = LintConfig(check_syntax=True, check_style=False, check_semantic=False)
        result = rs_lint_code("x = 1", config)
        assert isinstance(result, list)

    def test_severity_values(self):
        assert Severity.Error != Severity.Warning
        assert Severity.Warning != Severity.Info

    def test_lint_config_defaults(self):
        config = LintConfig()
        assert config.check_syntax is True
        assert config.check_style is True
        assert config.check_semantic is True

    def test_diagnostic_str(self):
        config = LintConfig(check_syntax=True, check_style=False, check_semantic=False)
        diags = rs_lint_code("if {", config)
        assert len(diags) > 0
        s = str(diags[0])
        assert 'E100' in s

    def test_lint_config_check_ir_default(self):
        config = LintConfig()
        assert config.check_ir is False


class TestImprovements:
    """Improvement suggestions (I100, I101, I102)."""

    # I100 - TCO

    def test_non_tail_recursive_call(self):
        code = "fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I100' for d in result.diagnostics)

    def test_tail_recursive_no_warning(self):
        code = "fact = (n, acc=1) => { if n <= 1 { acc } else { fact(n - 1, n * acc) } }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I100' for d in result.diagnostics)

    def test_non_recursive_no_warning(self):
        code = "f = (x) => { x + 1 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I100' for d in result.diagnostics)

    # I101 - Redundant boolean

    def test_redundant_eq_true(self):
        code = "x = True\nif x == True { 1 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I101' for d in result.diagnostics)

    def test_no_redundant_comparison(self):
        code = "x = 1\nif x == 1 { 1 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I101' for d in result.diagnostics)

    # I102 - Self-assignment

    def test_self_assignment(self):
        code = "x = 1\nx = x"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I102' for d in result.diagnostics)

    def test_different_assignment_no_warning(self):
        code = "x = 1\ny = x"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I102' for d in result.diagnostics)


class TestPatterns:
    """Pattern-oriented warnings and hints."""

    def test_unreachable_after_return(self):
        code = "f = () => {\n  return 1\n  x = 2\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'W300' for d in result.diagnostics)

    def test_return_inside_if_does_not_mark_following_statement_unreachable(self):
        code = "f = () => {\n  if cond { return 1 }\n  x = 2\n  x\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'W300' for d in result.diagnostics)

    def test_unused_parameter(self):
        code = "f = (x, y) => { x + 1 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        warnings = [d for d in result.diagnostics if d.code == 'W201']
        assert len(warnings) == 1
        assert "'y'" in warnings[0].message

    def test_unused_parameter_underscore_ignored(self):
        code = "f = (_x, y) => { y }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'W201' for d in result.diagnostics)

    def test_parameter_used_in_nested_lambda(self):
        code = "outer = (x) => {\n  inner = () => { x }\n  inner()\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'W201' for d in result.diagnostics)

    def test_dead_else_branch_when_if_true(self):
        code = "if True { 1 } else { 2 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'W301' for d in result.diagnostics)

    def test_dead_if_branch_when_if_false(self):
        code = "if False { 1 } else { 2 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'W301' for d in result.diagnostics)

    def test_non_constant_if_no_dead_branch_warning(self):
        code = "x = True\nif x { 1 } else { 2 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'W301' for d in result.diagnostics)

    def test_detectable_infinite_loop_without_break(self):
        code = "f = () => {\n  while True { tick() }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'W302' for d in result.diagnostics)

    def test_while_true_with_break_does_not_warn(self):
        code = "f = () => {\n  while True { if stop { break } }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'W302' for d in result.diagnostics)

    def test_break_in_nested_loop_does_not_silence_outer_infinite_loop(self):
        code = "f = () => {\n  while True {\n    for x in items { break }\n  }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'W302' for d in result.diagnostics)

    def test_shadowing_outer_variable(self):
        code = "x = 10\nf = () => {\n  x = 20\n  x\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'W204' for d in result.diagnostics)

    def test_mutating_captured_variable_does_not_warn_shadowing(self):
        code = "make = () => {\n  count = 0\n  () => { count = count + 1; count }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'W204' for d in result.diagnostics)


class TestMetrics:
    """Complexity and structure metrics."""

    def test_cyclomatic_complexity_exceeded(self):
        code = """
        f = () => {
            if a and b { 1 }
            elif c or d { 2 }
            elif e { 3 }
            while g { 4 }
            for x in items { x }
            match y {
                1 => { 1 }
                2 => { 2 }
                3 => { 3 }
                _ => { 4 }
            }
        }
        """
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I201' for d in result.diagnostics)

    def test_cyclomatic_complexity_threshold_not_exceeded(self):
        code = """
        f = () => {
            if a and b { 1 }
            elif c or d { 2 }
            while g { 3 }
            for x in items { x }
            match y {
                1 => { 1 }
                2 => { 2 }
                3 => { 3 }
                _ => { 4 }
            }
        }
        """
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I201' for d in result.diagnostics)

    def test_nested_lambda_complexity_not_counted_in_outer_function(self):
        code = """
        f = () => {
            if a and b { 1 }
            elif c or d { 2 }
            while g { 3 }
            for x in items { x }
            helper = () => {
                if h { 1 }
                while i { 2 }
                for y in zs { y }
            }
            match y {
                1 => { 1 }
                2 => { 2 }
                3 => { 3 }
                _ => { 4 }
            }
        }
        """
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I201' for d in result.diagnostics)

    def test_match_case_count_contributes_to_cyclomatic_complexity(self):
        code = """
        f = () => {
            match x {
                1 => { 1 }
                2 => { 2 }
                3 => { 3 }
                4 => { 4 }
                5 => { 5 }
                6 => { 6 }
                7 => { 7 }
                8 => { 8 }
                9 => { 9 }
                10 => { 10 }
                _ => { 11 }
            }
        }
        """
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I201' for d in result.diagnostics)

    def test_too_many_parameters(self):
        code = "f = (a, b, c, d, e, f, g) => { a }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I203' for d in result.diagnostics)

    def test_parameter_threshold_not_exceeded(self):
        code = "f = (a, b, c, d, e, f) => { a }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I203' for d in result.diagnostics)

    def test_match_without_wildcard(self):
        code = "x = 1\nmatch x { 1 => { 1 } }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I103' for d in result.diagnostics)

    def test_match_with_wildcard_no_warning(self):
        code = "x = 1\nmatch x { 1 => { 1 } _ => { 2 } }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I103' for d in result.diagnostics)

    def test_guarded_wildcard_still_warns(self):
        code = "x = 1\nmatch x { _ if x > 0 => { 1 } }"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I103' for d in result.diagnostics)

    def test_match_exhaustive_enum_no_warning(self):
        code = "enum Color { red; green; blue }\nc = Color.green\nmatch c {\n    Color.red => { 1 }\n    Color.green => { 2 }\n    Color.blue => { 3 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I103' for d in result.diagnostics)

    def test_match_partial_enum_warns(self):
        code = "enum Color { red; green; blue }\nc = Color.green\nmatch c {\n    Color.red => { 1 }\n    Color.green => { 2 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I103' for d in result.diagnostics)

    def test_match_exhaustive_enum_with_or(self):
        code = "enum Color { red; green; blue }\nc = Color.green\nmatch c {\n    Color.red | Color.green => { 1 }\n    Color.blue => { 2 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I103' for d in result.diagnostics)

    def test_match_exhaustive_boolean_no_warning(self):
        code = "x = True\nmatch x {\n    True => { 1 }\n    False => { 0 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert not any(d.code == 'I103' for d in result.diagnostics)

    def test_match_partial_boolean_warns(self):
        code = "x = True\nmatch x {\n    True => { 1 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I103' for d in result.diagnostics)

    def test_match_enum_wrong_type_warns(self):
        code = "enum Color { red; green; blue }\nenum Size { small; large }\nc = Size.small\nmatch c {\n    Color.red => { 1 }\n    Color.green => { 2 }\n    Color.blue => { 3 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I103' for d in result.diagnostics)

    def test_match_enum_unknown_scrutinee_warns(self):
        code = "enum Color { red; green; blue }\nmatch c {\n    Color.red => { 1 }\n    Color.green => { 2 }\n    Color.blue => { 3 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I103' for d in result.diagnostics)

    def test_function_too_long(self):
        body = "\n".join(f"  {i}" for i in range(31))
        code = f"f = () => {{\n{body}\n}}"
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I202' for d in result.diagnostics)

    def test_nesting_depth_exceeded(self):
        code = """
        f = () => {
            if a {
                if b {
                    while c {
                        for x in range(1) {
                            match x {
                                1 => {
                                    if d { 1 }
                                }
                            }
                        }
                    }
                }
            }
        }
        """
        result = lint_code(code, check_syntax=False, check_style=False)
        assert any(d.code == 'I200' for d in result.diagnostics)


class TestW200ScopeAware:
    """W200 unused variable: scope-aware behavior."""

    def test_global_variable_used(self):
        """Global variable used later should not trigger W200."""
        code = "x = 42\nprint(x)"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W200']
        assert len(w310) == 0

    def test_global_variable_unused(self):
        """Global variable not used should not trigger W200 (global scope exemption)."""
        code = "x = 42"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W200']
        assert len(w310) == 0

    def test_local_unused_variable(self):
        """Local unused variable inside a function should trigger W200."""
        code = "f = () => {\n  unused = 42\n  1\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W200']
        assert len(w310) == 1
        assert "'unused'" in w310[0].message

    def test_local_used_variable(self):
        """Local used variable should not trigger W200."""
        code = "f = (x) => { x + 1 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W200']
        assert len(w310) == 0

    def test_nested_function_closure(self):
        """Outer variable used in inner function (closure) should not trigger W200."""
        code = "outer = () => {\n  x = 1\n  inner = () => { x }\n  inner()\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W200']
        assert len(w310) == 0

    def test_struct_self_parameter(self):
        """self parameter in struct method should not trigger W200."""
        code = "struct Foo {\n  method(self) => { 1 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W200']
        assert len(w310) == 0

    def test_multiple_assignments_last_used(self):
        """Multiple assignments where only the last value is used."""
        code = "f = () => {\n  x = 1\n  x = 2\n  x\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W200']
        assert len(w310) == 0


class TestNoqa:
    """Inline suppression via # noqa comments."""

    def test_bare_noqa_suppresses_all(self):
        result = lint_code("y = x + 1  # noqa", check_style=False, check_names=True)
        assert not result.diagnostics

    def test_specific_code(self):
        result = lint_code("y = x + 1  # noqa: E200", check_style=False, check_names=True)
        assert not any(d.code == 'E200' for d in result.diagnostics)

    def test_wrong_code_not_suppressed(self):
        result = lint_code("y = x + 1  # noqa: W200", check_style=False, check_names=True)
        assert any(d.code == 'E200' for d in result.diagnostics)

    def test_multiple_codes(self):
        result = lint_code("y = x + 1  # noqa: E200, W200", check_style=False, check_names=True)
        assert not any(d.code == 'E200' for d in result.diagnostics)

    def test_does_not_affect_other_lines(self):
        result = lint_code("y = x + 1  # noqa\nz = w + 2", check_style=False, check_names=True)
        e300 = [d for d in result.diagnostics if d.code == 'E200']
        assert len(e300) == 1
        assert e300[0].line == 2

    def test_suppresses_unused_variable(self):
        result = lint_code("f = () => { y = 1; 2 }  # noqa: W200", check_style=False)
        assert not any(d.code == 'W200' for d in result.diagnostics)

    def test_noqa_with_reason(self):
        result = lint_code("y = x + 1  # noqa -- needed", check_style=False, check_names=True)
        assert not result.diagnostics

    def test_noqa_in_string_not_suppressed(self):
        """# noqa inside a string literal must not suppress diagnostics."""
        result = lint_code('f = () => { y = "# noqa"; 1 }', check_style=False)
        assert any(d.code == 'W200' for d in result.diagnostics)

    def test_noqa_code_with_trailing_reason(self):
        """# noqa: E200 -- reason should still suppress E200."""
        result = lint_code("y = x + 1  # noqa: E200 -- false positive", check_style=False, check_names=True)
        assert not any(d.code == 'E200' for d in result.diagnostics)

    def test_noqa_not_a_directive(self):
        """# noqa123 is not a valid noqa directive."""
        result = lint_code("y = x + 1  # noqa123", check_style=False, check_names=True)
        assert any(d.code == 'E200' for d in result.diagnostics)


class TestDeepAnalysis:
    """Deep CFG analysis (W310 -- possibly uninitialized variable)."""

    def _lint(self, source, **kwargs):
        return lint_code(source, check_ir=True, check_style=False, **kwargs)

    def test_if_without_else_triggers_w310(self):
        code = "if cond {\n    x = 1\n}\nprint(x)"
        result = self._lint(code)
        w310 = [d for d in result.diagnostics if d.code == 'W310']
        assert len(w310) == 1
        assert w310[0].line == 4

    def test_if_else_complete_no_w310(self):
        code = "if cond {\n    x = 1\n} else {\n    x = 2\n}\nprint(x)"
        result = self._lint(code)
        assert not any(d.code == 'W310' for d in result.diagnostics)

    def test_check_ir_off_by_default(self):
        code = "if cond {\n    x = 1\n}\nprint(x)"
        result = lint_code(code, check_style=False)
        assert not any(d.code == 'W310' for d in result.diagnostics)

    def test_noqa_suppresses_w310(self):
        code = "if cond {\n    x = 1\n}\nprint(x)  # noqa: W310"
        result = self._lint(code)
        assert not any(d.code == 'W310' for d in result.diagnostics)

    def test_w310_with_other_diagnostics(self):
        code = "if cond {\n    x = 1\n}\nprint(x)"
        result = lint_code(code, check_ir=True, check_style=True, check_names=True)
        codes = {d.code for d in result.diagnostics}
        assert 'W310' in codes
