# FILE: catnip/tools/linter.py
"""
Catnip linter - Rust implementation with Python wrapper.

Three levels of verification:
1. Syntax: does the code parse?
2. Style: does it follow formatting conventions?
3. Semantic: defined variables, consistent types, etc.

Usage:
    catnip lint script.cat
    catnip lint --level syntax script.cat
"""

from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional

from catnip._rs import (
    Diagnostic,
    LintConfig,
    Severity,
    lint_code as _rs_lint_code,
)


@dataclass
class LintResult:
    """Complete result of a lint analysis."""

    diagnostics: List[Diagnostic] = field(default_factory=list)
    source: str = ""
    filename: Optional[str] = None

    @property
    def errors(self) -> List[Diagnostic]:
        return [d for d in self.diagnostics if d.severity == Severity.Error]

    @property
    def warnings(self) -> List[Diagnostic]:
        return [d for d in self.diagnostics if d.severity == Severity.Warning]

    @property
    def has_errors(self) -> bool:
        return len(self.errors) > 0

    def add(self, diagnostic: Diagnostic) -> None:
        self.diagnostics.append(diagnostic)

    def summary(self) -> str:
        n_errors = len(self.errors)
        n_warnings = len(self.warnings)
        n_info = len([d for d in self.diagnostics if d.severity == Severity.Info])

        parts = []
        if n_errors:
            parts.append(f"{n_errors} error{'s' if n_errors > 1 else ''}")
        if n_warnings:
            parts.append(f"{n_warnings} warning{'s' if n_warnings > 1 else ''}")
        if n_info:
            parts.append(f"{n_info} info")

        if not parts:
            return "No issues found"
        return ", ".join(parts)


def lint_code(
    source: str,
    filename: Optional[str] = None,
    check_syntax: bool = True,
    check_style: bool = True,
    check_semantic: bool = True,
    check_ir: bool = False,
    check_names: bool = False,
    max_nesting_depth: int = 5,
    max_cyclomatic_complexity: int = 10,
    max_function_length: int = 30,
    max_parameters: int = 6,
) -> LintResult:
    config = LintConfig(
        check_syntax=check_syntax,
        check_style=check_style,
        check_semantic=check_semantic,
        check_ir=check_ir,
        check_names=check_names,
        max_nesting_depth=max_nesting_depth,
        max_cyclomatic_complexity=max_cyclomatic_complexity,
        max_function_length=max_function_length,
        max_parameters=max_parameters,
    )
    diagnostics = _rs_lint_code(source, config)
    return LintResult(diagnostics=diagnostics, source=source, filename=filename)


def lint_file(path: Path, **kwargs) -> LintResult:
    source = Path(path).read_text()
    return lint_code(source, filename=str(path), **kwargs)
