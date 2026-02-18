# FILE: catnip/cli/commands/commands.py
"""List available commands."""

from __future__ import annotations

import click

from ..plugins import discover_plugins


def _format_table(rows, headers):
    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))

    header_line = "  ".join(headers[i].ljust(widths[i]) for i in range(len(headers)))
    sep_line = "  ".join("-" * widths[i] for i in range(len(headers)))
    body_lines = ["  ".join(row[i].ljust(widths[i]) for i in range(len(headers))) for row in rows]
    return "\n".join([header_line, sep_line, *body_lines])


def _command_source(entry_point):
    if entry_point is None:
        return 'builtin'
    if entry_point.value.startswith('catnip.cli.commands.'):
        return 'builtin'
    if entry_point.dist:
        return f"plugin:{entry_point.dist.name}"
    return 'plugin'


@click.command("commands")
@click.option(
    "--resolve/--no-resolve",
    default=True,
    help="Load commands to show short help strings",
)
@click.pass_context
def cmd_commands(ctx, resolve):
    """List available commands (built-ins + plugins)."""
    group = ctx.parent.command if ctx.parent else None
    if group is None:
        click.echo("Error: No CLI group context available", err=True)
        ctx.exit(1)

    plugins = getattr(group, "plugins", None)
    if plugins is None:
        plugins = discover_plugins()

    command_names = group.list_commands(ctx.parent)
    rows = []
    for name in command_names:
        entry_point = plugins.get(name)
        short_help = ''
        status = 'unknown' if not resolve else 'ok'
        if resolve:
            cmd = group.get_command(ctx.parent, name)
            if cmd is None:
                status = 'missing'
                short_help = "unable to load command"
            else:
                short_help = cmd.get_short_help_str()
        rows.append([name, _command_source(entry_point), status, short_help])

    if not rows:
        click.echo("No commands found.")
        return

    table = _format_table(rows, headers=['command', 'source', 'status', 'help'])
    click.echo(table)
