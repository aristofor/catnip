# FILE: catnip/cli/commands/module.py
"""Inspect module policies."""

from __future__ import annotations

import click


@click.group("module")
def cmd_module():
    """Inspect module access policies.

    \b
    Examples:
        catnip module list-profiles policies.toml
        catnip module check policies.toml sandbox os math
    """


@cmd_module.command("list-profiles")
@click.argument("file", type=click.Path(exists=True))
def list_profiles(file):
    """List policy profiles in a TOML file."""
    from ..._rs import ModulePolicy

    try:
        profiles = ModulePolicy.list_profiles(file)
    except (ValueError, FileNotFoundError) as e:
        raise click.ClickException(str(e))

    if not profiles:
        click.echo("No profiles found.")
        return

    for name in profiles:
        policy = ModulePolicy.from_file(file, name)
        click.echo(f"  {name}  ({policy.policy_hash[:8]})")


@cmd_module.command("check")
@click.argument("file", type=click.Path(exists=True))
@click.argument("profile")
@click.argument("modules", nargs=-1, required=True)
def check(file, profile, modules):
    """Check module access against a policy profile.

    \b
    Examples:
        catnip module check policies.toml sandbox os math json
    """
    from ..._rs import ModulePolicy

    try:
        policy = ModulePolicy.from_file(file, profile)
    except (KeyError, ValueError, FileNotFoundError) as e:
        raise click.ClickException(str(e))

    for name in modules:
        allowed = policy.check(name)
        symbol = "+" if allowed else "-"
        click.echo(f"  {symbol} {name}")
