# FILE: catnip/cli/commands/lint.py
"""Lint command - full code analysis."""

import sys
from pathlib import Path

import click


@click.command("lint")
@click.argument("files", nargs=-1, type=click.Path())
@click.option(
    "-l",
    "--level",
    type=click.Choice(["syntax", "style", "semantic", "all"]),
    default="all",
    help="Analysis level (default: all)",
)
@click.option("--deep", is_flag=True, help="Enable deep IR analysis (opt-in)")
@click.option("--stdin", is_flag=True, help="Read from stdin")
@click.pass_context
def cmd_lint(ctx, files, level, deep, stdin):
    """Full code analysis (syntax + style + semantic).

    \b
    Examples:
        catnip lint script.cat
        catnip lint --level syntax script.cat
        catnip lint -l style *.cat
        catnip lint --deep script.cat
        cat code.cat | catnip lint --stdin
    """
    from ...tools.linter import Severity, lint_code, lint_file

    verbose = ctx.obj.get("verbose", False) if ctx.obj else False
    no_color = ctx.obj.get("no_color", False) if ctx.obj else False

    # Collect files to lint
    file_list = []
    if stdin:
        file_list.append(None)  # Marker for stdin
    for f in files:
        if f == "--":
            if sys.stdin.isatty():
                raise click.UsageError("'--' requires piped input")
            file_list.append(None)
        else:
            file_list.append(Path(f))

    if not file_list:
        raise click.UsageError("Provide at least one FILE or use --stdin")

    # Configure checks
    check_syntax = level in ("syntax", "all")
    check_style = level in ("style", "all")
    check_semantic = level in ("semantic", "all")
    check_ir = deep

    exit_code = 0
    all_diagnostics = []
    files_linted = 0

    for file_path in file_list:
        if file_path is None:
            # stdin
            source = sys.stdin.read()
            result = lint_code(
                source,
                filename="<stdin>",
                check_syntax=check_syntax,
                check_style=check_style,
                check_semantic=check_semantic,
                check_ir=check_ir,
            )
            prefix = "<stdin>:"
            all_diagnostics.extend(result.diagnostics)
            files_linted += 1
        else:
            if not file_path.exists():
                click.echo(f"Error: File not found: {file_path}", err=True)
                exit_code = 1
                continue

            result = lint_file(
                file_path,
                check_syntax=check_syntax,
                check_style=check_style,
                check_semantic=check_semantic,
                check_ir=check_ir,
            )
            prefix = f"{file_path}:"
            all_diagnostics.extend(result.diagnostics)
            files_linted += 1

        # Output diagnostics
        if result.diagnostics:
            for diag in result.diagnostics:
                severity_color = {
                    Severity.Error: "red",
                    Severity.Warning: "yellow",
                    Severity.Info: "blue",
                    Severity.Hint: "cyan",
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
