# FILE: catnip/cli/commands/config.py
"""Manage Catnip configuration."""

from __future__ import annotations

import click

from ...config import (
    VALID_KEYS,
    ConfigManager,
    get_config_path,
)


def _parse_bool(value: str) -> bool:
    """Parse a boolean value from string."""
    lower = value.lower()
    if lower in ('true', 'on', 'yes', '1'):
        return True
    if lower in ('false', 'off', 'no', '0'):
        return False
    raise ValueError(f"Invalid boolean: {value}")


@click.group("config")
@click.pass_context
def cmd_config(ctx):
    """Manage Catnip configuration.

    Configuration is stored in ~/.config/catnip/catnip.toml by default.
    Use --config to specify an alternate file.

    \b
    Examples:
        catnip config show
        catnip config get jit
        catnip config set no_color true
        catnip --config my-catnip.toml config show
    """
    pass


@cmd_config.command("show")
@click.option("--debug", is_flag=True, help="Show value sources (default/file/env/cli)")
@click.pass_context
def config_show(ctx, debug):
    """Display all configuration values.

    Use --debug to see where each value comes from:
    - default: hardcoded default
    - file: catnip.toml
    - env: environment variable (CATNIP_OPTIMIZE, CATNIP_EXECUTOR, NO_COLOR)
    - cli: command line option (-o, -x, --no-color)
    """
    from pathlib import Path

    # Determine config path (custom or default)
    parent_obj = ctx.parent.obj if ctx.parent else None
    config_path = None
    if parent_obj and 'config_path' in parent_obj:
        config_path = Path(parent_obj['config_path']) if parent_obj['config_path'] else None

    if debug:
        # Use parent context's config_manager if available (includes CLI overrides)
        if parent_obj and 'config_manager' in parent_obj:
            config_manager = parent_obj['config_manager']
            # Apply CLI optimizations if not already done
            if 'optimizations' in parent_obj:
                config_manager.apply_cli_optimizations(parent_obj['optimizations'])
        else:
            # Standalone: build ConfigManager with file + env only
            config_manager = ConfigManager()
            config_manager.load_file(config_path)
            config_manager.load_env()

        # Show which config file was used
        actual_path = config_path or get_config_path()
        click.echo(f"Configuration from: {actual_path}\n")
        for line in config_manager.debug_report():
            click.echo(f"  {line}")
    else:
        config_manager = ConfigManager()
        config_manager.load_file(config_path)
        config_manager.load_env()
        config = config_manager.items()

        actual_path = config_path or get_config_path()
        click.echo(f"# {actual_path}")
        for key in sorted(config.keys()):
            value = config[key]
            if type(value) is bool:
                click.echo(f"{key} = {str(value).lower()}")
            else:
                click.echo(f"{key} = {value}")


@cmd_config.command("get")
@click.argument("key")
@click.pass_context
def config_get(ctx, key):
    """Get a configuration value.

    \b
    Available keys:
        jit       - Enable JIT compilation (true/false)
        no_color  - Disable color output (true/false)
    """
    from pathlib import Path

    # Determine config path (custom or default)
    parent_obj = ctx.parent.obj if ctx.parent else None
    config_path = None
    if parent_obj and 'config_path' in parent_obj:
        config_path = Path(parent_obj['config_path']) if parent_obj['config_path'] else None

    config_manager = ConfigManager()
    config_manager.load_file(config_path)

    try:
        value = config_manager.get(key)
        if type(value) is bool:
            click.echo(str(value).lower())
        else:
            click.echo(value)
    except KeyError:
        raise click.ClickException(f"Unknown config key: {key}")


@cmd_config.command("set")
@click.argument("key")
@click.argument("value")
@click.pass_context
def config_set(ctx, key, value):
    """Set a configuration value.

    \b
    Available keys:
        jit                - Enable JIT compilation (true/false)
        no_color           - Disable color output (true/false)
        tco                - Tail-call optimization (true/false)
        optimize           - Optimization level (0-3)
        executor           - Execution mode (vm/ast)
        cache_max_size_mb  - Max cache size in MB (number or "unlimited")
        cache_ttl_seconds  - Cache TTL in seconds (number or "unlimited")

    \b
    Boolean values accept: true/false, on/off, yes/no, 1/0
    """
    from pathlib import Path

    from ...config import DEFAULT_CONFIG, save_config

    if key not in VALID_KEYS:
        raise click.ClickException(f"Unknown config key: {key}. Valid keys: {', '.join(sorted(VALID_KEYS))}")

    # Determine config path (custom or default)
    parent_obj = ctx.parent.obj if ctx.parent else None
    config_path = None
    if parent_obj and 'config_path' in parent_obj:
        config_path = Path(parent_obj['config_path']) if parent_obj['config_path'] else None

    # Load config
    config_manager = ConfigManager()
    config_manager.load_file(config_path)
    config = config_manager.items()

    # Parse value based on expected type (from defaults)
    # Find default value to determine expected type
    expected_type = None
    for section in ['repl', 'optimize', 'format', 'cache']:
        if key in DEFAULT_CONFIG[section]:
            default_value = DEFAULT_CONFIG[section][key]
            # For cache options, None is a valid type (means unlimited)
            expected_type = type(default_value) if default_value is not None else None
            break

    # Special handling for "unlimited" keyword (maps to None)
    if value.lower() == 'unlimited':
        parsed = None
    # Parse according to type
    elif expected_type is bool:
        try:
            parsed = _parse_bool(value)
        except ValueError:
            raise click.ClickException(f"Invalid boolean value: {value}. Use true/false, on/off, yes/no, or 1/0")
    elif expected_type is int:
        try:
            parsed = int(value)
        except ValueError:
            raise click.ClickException(f"Invalid integer value: {value}")
    elif expected_type is str:
        parsed = value
    else:
        # Fallback: try boolean, then int, then string
        try:
            parsed = _parse_bool(value)
        except ValueError:
            try:
                parsed = int(value)
            except ValueError:
                parsed = value

    # Update and save
    config[key] = parsed
    save_config(config, config_path)

    actual_path = config_path or get_config_path()
    if parsed is None:
        click.echo(f"Set {key} = unlimited in {actual_path}")
    elif type(parsed) is bool:
        click.echo(f"Set {key} = {str(parsed).lower()} in {actual_path}")
    else:
        click.echo(f"Set {key} = {parsed} in {actual_path}")


@cmd_config.command("path")
@click.pass_context
def config_path(ctx):
    """Show the configuration file path."""
    from pathlib import Path

    # Determine config path (custom or default)
    parent_obj = ctx.parent.obj if ctx.parent else None
    custom_path = None
    if parent_obj and 'config_path' in parent_obj:
        custom_path = Path(parent_obj['config_path']) if parent_obj['config_path'] else None

    actual_path = custom_path or get_config_path()
    click.echo(actual_path)
