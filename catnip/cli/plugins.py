# FILE: catnip/cli/plugins.py
"""Plugin discovery and loading via entry points."""

from importlib.metadata import entry_points

import click

ENTRY_POINT_GROUP = 'catnip.commands'


def discover_plugins() -> dict:
    """
    Discover all registered plugins.

    Returns a dict {name: EntryPoint} without loading modules.
    Loading is lazy (at invocation time).
    """
    eps = entry_points(group=ENTRY_POINT_GROUP)
    return {ep.name: ep for ep in eps}


def load_plugin(entry_point) -> click.Command | None:
    """
    Load a plugin from its entry point.

    Returns the Click command or None if loading fails.
    Errors are logged to stderr without crashing.
    """
    try:
        cmd = entry_point.load()
        if not isinstance(cmd, click.Command):
            click.echo(
                f"Warning: Plugin '{entry_point.name}' is not a Click command",
                err=True,
            )
            return None
        return cmd
    except Exception as e:
        click.echo(
            f"Warning: Failed to load plugin '{entry_point.name}': {e}",
            err=True,
        )
        return None
