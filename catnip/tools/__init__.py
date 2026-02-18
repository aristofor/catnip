# FILE: catnip/tools/__init__.py
"""Catnip tools package - utilities for grammar extraction, syntax highlighting, and linting."""

from catnip._rs import FormatConfig, format_code

from .linter import lint_code, lint_file

__all__ = (
    'FormatConfig',
    'format_code',
    'lint_code',
    'lint_file',
)
