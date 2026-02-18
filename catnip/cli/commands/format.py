# FILE: catnip/cli/commands/format.py
"""Format command - Catnip code formatting."""

import sys
from glob import glob
from pathlib import Path

import click


@click.command('format')
@click.argument('files', nargs=-1, type=click.Path())
@click.option('--stdin', is_flag=True, help='Read from stdin')
@click.option('--in-place', '-i', is_flag=True, help='Format files in place')
@click.option('--check', is_flag=True, help='Check if formatted (exit 1 if not)')
@click.option('--indent-size', type=int, help='Indent size (default: from config)')
@click.option('--line-length', type=int, help='Line length (default: from config)')
@click.option('--diff', is_flag=True, help='Show diff instead of formatted code')
@click.pass_context
def cmd_format(ctx, files, stdin, in_place, check, indent_size, line_length, diff):
    """Format Catnip code files.

    \b
    Examples:
        catnip format script.cat
        catnip format src/*.cat --in-place
        cat code.cat | catnip format --stdin
        catnip format --check src/*.cat  # For CI
        catnip format --indent-size 2 script.cat
    """
    from ...config import ConfigManager
    from ...tools import format_code

    verbose = ctx.obj.get('verbose', False) if ctx.obj else False

    # Charger la configuration
    if ctx.obj and 'config_manager' in ctx.obj:
        config_mgr = ctx.obj['config_manager']
    else:
        config_mgr = ConfigManager()
        # Utiliser config alternatif si spécifié
        if ctx.obj and 'config_path' in ctx.obj and ctx.obj['config_path']:
            config_mgr.load_file(Path(ctx.obj['config_path']))
        else:
            config_mgr.load_file()
        config_mgr.load_env()

    # Override avec CLI flags
    if indent_size is not None:
        config_mgr.apply_cli_format_indent_size(indent_size)
    if line_length is not None:
        config_mgr.apply_cli_format_line_length(line_length)

    format_config = config_mgr.get_format_config()

    # Mode stdin
    if stdin:
        if sys.stdin.isatty():
            raise click.UsageError('--stdin requires piped input')
        source = sys.stdin.read()
        try:
            formatted = format_code(source, format_config)
            if check:
                if source != formatted:
                    click.echo('Code is not formatted', err=True)
                    sys.exit(1)
            elif diff:
                show_diff(source, formatted, '<stdin>')
            else:
                click.echo(formatted, nl=False)
        except Exception as e:
            click.echo(f'Error: {e}', err=True)
            if verbose:
                import traceback

                traceback.print_exc()
            sys.exit(1)
        return

    # Mode fichiers
    if not files:
        raise click.UsageError('Provide FILES or use --stdin')

    # Expand globs
    expanded_files = []
    for pattern in files:
        matches = glob(pattern, recursive=True)
        if matches:
            expanded_files.extend(matches)
        else:
            # Pas de match, utiliser tel quel (peut être un fichier spécifique)
            expanded_files.append(pattern)

    exit_code = 0
    for file_path in expanded_files:
        path = Path(file_path)
        if not path.exists():
            click.echo(f'File not found: {file_path}', err=True)
            exit_code = 1
            continue

        if not path.is_file():
            continue  # Ignorer les répertoires

        try:
            source = path.read_text()
            formatted = format_code(source, format_config)

            if check:
                if source != formatted:
                    click.echo(f'{file_path}: not formatted', err=True)
                    exit_code = 1
            elif diff:
                show_diff(source, formatted, str(path))
            elif in_place:
                if source != formatted:
                    path.write_text(formatted)
                    if verbose:
                        click.echo(f'Formatted: {file_path}')
            else:
                # Afficher sur stdout (seulement si 1 fichier)
                if len(expanded_files) == 1:
                    click.echo(formatted, nl=False)
                else:
                    click.echo(f'# {file_path}')
                    click.echo(formatted)

        except Exception as e:
            click.echo(f'Error in {file_path}: {e}', err=True)
            if verbose:
                import traceback

                traceback.print_exc()
            exit_code = 1

    sys.exit(exit_code)


def show_diff(original: str, formatted: str, filename: str):
    """Show unified diff between original and formatted code."""
    import difflib

    original_lines = original.splitlines(keepends=True)
    formatted_lines = formatted.splitlines(keepends=True)

    diff = difflib.unified_diff(
        original_lines,
        formatted_lines,
        fromfile=f'{filename} (original)',
        tofile=f'{filename} (formatted)',
        lineterm='',
    )

    for line in diff:
        click.echo(line, nl=False)
