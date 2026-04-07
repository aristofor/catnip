# FILE: catnip/cli/commands/lint.py
"""Lint command - full code analysis."""

import sys
from pathlib import Path

import click

from ..utils import expand_file_args


@click.command('lint')
@click.argument('files', nargs=-1, type=click.Path())
@click.option(
    '-l',
    '--level',
    type=click.Choice(['syntax', 'style', 'semantic', 'all']),
    default='all',
    help="Analysis level (default: all)",
)
@click.option('--deep', is_flag=True, help="Enable deep IR analysis (opt-in)")
@click.option('--check-names', is_flag=True, help="Check for undefined names (E200, opt-in)")
@click.option('--max-depth', type=click.IntRange(min=0), default=5, help="Max nesting depth (I200, default: 5, 0=off)")
@click.option(
    '--max-complexity',
    type=click.IntRange(min=0),
    default=10,
    help="Max cyclomatic complexity (I201, default: 10, 0=off)",
)
@click.option(
    '--max-length',
    type=click.IntRange(min=0),
    default=30,
    help="Max function length in statements (I202, default: 30, 0=off)",
)
@click.option(
    '--max-params', type=click.IntRange(min=0), default=6, help="Max parameters per function (I203, default: 6, 0=off)"
)
@click.option('--stdin', is_flag=True, help="Read from stdin")
@click.pass_context
def cmd_lint(ctx, files, level, deep, check_names, max_depth, max_complexity, max_length, max_params, stdin):
    """Full code analysis (syntax + style + semantic).

    \b
    Examples:
        catnip lint script.cat
        catnip lint src/                      # All .cat files recursively
        catnip lint --level syntax script.cat
        catnip lint -l style *.cat
        catnip lint --check-names script.cat
        catnip lint --deep script.cat
        cat code.cat | catnip lint --stdin
    """
    from ...tools.linter import Severity, lint_code, lint_file

    verbose = ctx.obj.get('verbose', False) if ctx.obj else False
    no_color = ctx.obj.get('no_color', False) if ctx.obj else False

    # Collect files to lint
    file_list = []
    if stdin:
        file_list.append(None)  # Marker for stdin
    stdin_args = [f for f in files if f == '--']
    file_args = [f for f in files if f != '--']
    for _ in stdin_args:
        if sys.stdin.isatty():
            raise click.UsageError("'--' requires piped input")
        file_list.append(None)
    file_list.extend(expand_file_args(file_args))

    if not file_list:
        if not file_args:
            raise click.UsageError("Provide at least one FILE or use --stdin")
        click.echo("No .cat files found", err=True)
        sys.exit(0)

    # Configure checks
    check_syntax = level in ('syntax', 'all')
    check_style = level in ('style', 'all')
    check_semantic = level in ('semantic', 'all')
    check_ir = deep
    threshold_kwargs = dict(
        max_nesting_depth=max_depth,
        max_cyclomatic_complexity=max_complexity,
        max_function_length=max_length,
        max_parameters=max_params,
    )

    exit_code = 0
    all_diagnostics = []
    files_linted = 0

    for file_path in file_list:
        if file_path is None:
            # stdin
            source = sys.stdin.read()
            result = lint_code(
                source,
                filename='<stdin>',
                check_syntax=check_syntax,
                check_style=check_style,
                check_semantic=check_semantic,
                check_ir=check_ir,
                check_names=check_names,
                **threshold_kwargs,
            )
            prefix = '<stdin>:'
            all_diagnostics.extend(result.diagnostics)
            files_linted += 1
        else:
            if not file_path.exists():
                click.echo(f"Error: File not found: {file_path}", err=True)
                exit_code = 1
                continue

            if not file_path.is_file():
                continue

            result = lint_file(
                file_path,
                check_syntax=check_syntax,
                check_style=check_style,
                check_semantic=check_semantic,
                check_ir=check_ir,
                check_names=check_names,
                **threshold_kwargs,
            )
            prefix = f"{file_path}:"
            all_diagnostics.extend(result.diagnostics)
            files_linted += 1

        # Output diagnostics
        if result.diagnostics:
            for diag in result.diagnostics:
                severity_color = {
                    Severity.Error: 'red',
                    Severity.Warning: 'yellow',
                    Severity.Info: 'blue',
                    Severity.Hint: 'cyan',
                }.get(diag.severity, None)

                msg = f"{prefix}{diag}"
                if not no_color and severity_color:
                    msg = click.style(msg, fg=severity_color)
                click.echo(msg, err=(diag.severity == Severity.Error))

            if result.has_errors:
                exit_code = 1
        else:
            if verbose:
                click.echo(f"{prefix[:-1]}: OK")

    # Summary
    if files_linted > 0 and (verbose or exit_code != 0):
        n_errors = sum(1 for d in all_diagnostics if d.severity == Severity.Error)
        n_warnings = sum(1 for d in all_diagnostics if d.severity == Severity.Warning)
        n_info = sum(1 for d in all_diagnostics if d.severity == Severity.Info)
        parts = []
        if n_errors:
            parts.append(f"{n_errors} error{'s' if n_errors > 1 else ''}")
        if n_warnings:
            parts.append(f"{n_warnings} warning{'s' if n_warnings > 1 else ''}")
        if n_info:
            parts.append(f"{n_info} info")
        summary = ", ".join(parts) if parts else "No issues found"
        click.echo(f"\n{summary}")

    sys.exit(exit_code)
