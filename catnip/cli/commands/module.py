# FILE: catnip/cli/commands/module.py
"""Inspect module policies."""

from __future__ import annotations

import click


@click.group('module')
def cmd_module():
    """Inspect module access policies.

    \b
    Examples:
        catnip module list-profiles
        catnip module check sandbox os math
    """


@cmd_module.command('list-profiles')
@click.pass_context
def list_profiles(ctx):
    """List policy profiles from [modules.policies.*] in config."""
    config_manager = (ctx.obj or {}).get('config_manager')
    if config_manager is None:
        from ..._rs import ConfigManager

        config_manager = ConfigManager()
        config_manager.load_file()

    profiles = config_manager.list_policy_profiles()
    if not profiles:
        click.echo("No profiles found.")
        return

    for name in profiles:
        policy = config_manager.get_module_policy(name)
        click.echo(f"  {name}  ({policy.policy_hash[:8]})")


@cmd_module.command('check')
@click.argument('profile')
@click.argument('modules', nargs=-1, required=True)
@click.pass_context
def check(ctx, profile, modules):
    """Check module access against a named policy profile.

    \b
    Examples:
        catnip module check sandbox os math json
    """
    config_manager = (ctx.obj or {}).get('config_manager')
    if config_manager is None:
        from ..._rs import ConfigManager

        config_manager = ConfigManager()
        config_manager.load_file()

    policy = config_manager.get_module_policy(profile)
    if policy is None:
        available = config_manager.list_policy_profiles()
        msg = f"policy '{profile}' not found"
        if available:
            msg += f" (available: {', '.join(available)})"
        raise click.ClickException(msg)

    for name in modules:
        allowed = policy.check(name)
        symbol = "+" if allowed else "-"
        click.echo(f"  {symbol} {name}")
