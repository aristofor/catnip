# FILE: catnip/cli/commands/plugins.py
"""Inspect CLI plugins."""

from __future__ import annotations

import click

from ..plugins import discover_plugins, load_plugin


def _format_table(rows, headers):
    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))

    header_line = "  ".join(headers[i].ljust(widths[i]) for i in range(len(headers)))
    sep_line = "  ".join("-" * widths[i] for i in range(len(headers)))
    body_lines = ["  ".join(row[i].ljust(widths[i]) for i in range(len(headers))) for row in rows]
    return "\n".join([header_line, sep_line, *body_lines])


def _plugin_source(entry_point):
    if entry_point.value.startswith('catnip.cli.commands.'):
        return 'builtin'
    if entry_point.dist:
        return entry_point.dist.name
    return 'external'


@click.command("plugins")
@click.option("--check", is_flag=True, help="Load each plugin to validate it")
@click.option(
    "--builtins/--no-builtins",
    default=True,
    help="Include built-in commands",
)
@click.option("--entrypoints", is_flag=True, help="Show entry point values")
@click.pass_context
def cmd_plugins(ctx, check, builtins, entrypoints):
    """List registered plugins and their status."""
    plugins = discover_plugins()
    if not plugins:
        click.echo("No plugins found.")
        return

    rows = []
    for name, entry_point in sorted(plugins.items(), key=lambda item: item[0]):
        if not builtins and entry_point.value.startswith("catnip.cli.commands."):
            continue

        status = 'ok'
        if check:
            status = 'ok' if load_plugin(entry_point) is not None else 'error'

        source = _plugin_source(entry_point)
        row = [name, source, status]
        if entrypoints:
            row.append(entry_point.value)
        rows.append(row)

    if not rows:
        click.echo("No plugins match the current filters.")
        return

    headers = ['plugin', 'source', 'status']
    if entrypoints:
        headers.append('entry_point')

    table = _format_table(rows, headers=headers)
    click.echo(table)
