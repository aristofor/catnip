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
    """Style checking (W200, W201, W202)."""

    def test_well_formatted(self):
        result = lint_code("x = 1 + 2\n", check_syntax=False, check_semantic=False)
        style_diags = [d for d in result.diagnostics if d.code.startswith('W2')]
        assert len(style_diags) == 0

    def test_spacing_issue(self):
        result = lint_code("x=1+2\n", check_syntax=False, check_semantic=False)
        assert any(d.code == 'W200' for d in result.diagnostics)

    def test_trailing_whitespace(self):
        result = lint_code("x = 1  \n", check_syntax=False, check_semantic=False)
        assert any(d.code == 'W201' for d in result.diagnostics)

    def test_suggestion_provided(self):
        result = lint_code("x=1\n", check_syntax=False, check_semantic=False)
        w200 = [d for d in result.diagnostics if d.code == 'W200']
        assert len(w200) > 0
        assert w200[0].suggestion is not None


class TestSemantic:
    """Semantic checking (E300, W310)."""

    def test_used_variable_no_warning(self):
        result = lint_code("x = 1\nlen(x)", check_syntax=False, check_style=False)
        warnings = [d for d in result.diagnostics if d.code == 'W310']
        assert len(warnings) == 0

    def test_lambda_params_defined(self):
        result = lint_code("f = (x, y) => { x + y }", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E300']
        assert len(errors) == 0

    def test_for_loop_var_defined(self):
        result = lint_code("for i in range(10) { abs(i) }", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E300']
        assert len(errors) == 0

    def test_match_capture_defined(self):
        result = lint_code("x = 1\nmatch x { n => { abs(n) } }", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E300']
        assert len(errors) == 0

    def test_builtins_predefined(self):
        result = lint_code("len(range(10))", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E300']
        assert len(errors) == 0

    def test_nested_scopes(self):
        """Variables from outer scope visible in inner scope."""
        result = lint_code("x = 1\nf = () => { abs(x) }", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E300']
        assert len(errors) == 0

    def test_inner_scope_not_leaking(self):
        """Variables from inner scope not visible in outer scope."""
        result = lint_code("f = () => { y = 1 }\nabs(y)", check_syntax=False, check_style=False, check_names=True)
        errors = [d for d in result.diagnostics if d.code == 'E300']
        assert len(errors) == 1
        assert "'y'" in errors[0].message

    def test_self_referencing_function(self):
        """Function can reference itself (recursion)."""
        result = lint_code(
            "fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }", check_syntax=False, check_style=False
        )
        errors = [d for d in result.diagnostics if d.code == 'E300']
        assert len(errors) == 0

    def test_multiple_assignment(self):
        result = lint_code("x = 1\ny = 2\nz = x + y", check_syntax=False, check_style=False)
        errors = [d for d in result.diagnostics if d.code == 'E300']
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
        lines = [d.line for d in result.diagnostics if d.code == 'E300']
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


class TestW310ScopeAware:
    """W310 unused variable: scope-aware behavior."""

    def test_global_variable_used(self):
        """Global variable used later should not trigger W310."""
        code = "x = 42\nprint(x)"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W310']
        assert len(w310) == 0

    def test_global_variable_unused(self):
        """Global variable not used should not trigger W310 (global scope exemption)."""
        code = "x = 42"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W310']
        assert len(w310) == 0

    def test_local_unused_variable(self):
        """Local unused variable inside a function should trigger W310."""
        code = "f = () => {\n  unused = 42\n  1\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W310']
        assert len(w310) == 1
        assert "'unused'" in w310[0].message

    def test_local_used_variable(self):
        """Local used variable should not trigger W310."""
        code = "f = (x) => { x + 1 }"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W310']
        assert len(w310) == 0

    def test_nested_function_closure(self):
        """Outer variable used in inner function (closure) should not trigger W310."""
        code = "outer = () => {\n  x = 1\n  inner = () => { x }\n  inner()\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W310']
        assert len(w310) == 0

    def test_struct_self_parameter(self):
        """self parameter in struct method should not trigger W310."""
        code = "struct Foo {\n  method(self) => { 1 }\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W310']
        assert len(w310) == 0

    def test_multiple_assignments_last_used(self):
        """Multiple assignments where only the last value is used."""
        code = "f = () => {\n  x = 1\n  x = 2\n  x\n}"
        result = lint_code(code, check_syntax=False, check_style=False)
        w310 = [d for d in result.diagnostics if d.code == 'W310']
        assert len(w310) == 0
