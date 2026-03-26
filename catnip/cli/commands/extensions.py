# FILE: catnip/cli/commands/extensions.py
"""Inspect compiled extensions."""

from __future__ import annotations

import click

from ...extensions import discover_extensions
from ..utils import format_table


@click.group('extensions')
def cmd_extensions():
    """Manage compiled extensions."""


@cmd_extensions.command('list')
def cmd_list():
    """List installed extensions (entry points)."""
    eps = discover_extensions()
    if not eps:
        click.echo("No extensions found.")
        return

    rows = [[name, ep.value] for name, ep in sorted(eps.items())]
    click.echo(format_table(rows, headers=['name', 'entry_point']))


@cmd_extensions.command('info')
@click.argument('name')
def cmd_info(name):
    """Show details of an installed extension."""
    eps = discover_extensions()
    ep = eps.get(name)
    if ep is None:
        click.echo(f"Extension '{name}' not found.")
        raise SystemExit(1)

    try:
        module = ep.load()
    except Exception as exc:
        click.echo(f"Failed to load '{name}': {exc}")
        raise SystemExit(1)

    descriptor = getattr(module, '__catnip_extension__', None)
    if descriptor is None:
        click.echo(f"'{name}' has no __catnip_extension__ descriptor.")
        raise SystemExit(1)

    click.echo(f"name:        {descriptor.get('name', '?')}")
    click.echo(f"version:     {descriptor.get('version', '?')}")
    desc = descriptor.get('description', '')
    if desc:
        click.echo(f"description: {desc}")
    click.echo(f"register:    {'yes' if descriptor.get('register') else 'no'}")
    exports = descriptor.get('exports', {})
    if exports:
        click.echo(f"exports:     {', '.join(sorted(exports))}")
