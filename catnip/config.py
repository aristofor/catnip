# FILE: catnip/config.py
"""Configuration helpers for Catnip with source tracking.

IMPORTANT: Core config functionality migrated to Rust (catnip._rs).
This module provides backward-compatible imports and legacy functions.
"""

# Re-export Rust implementations (used by cli/commands/)
from catnip._rs import (  # noqa: F401
    ConfigManager,
    ConfigSource,
    get_cache_dir,
    get_config_path,
    set_config_value,
    valid_config_keys,
    valid_format_keys,
)


def _build_default_config() -> dict:
    """Reflect runtime defaults from the Rust ConfigManager."""
    manager = ConfigManager()
    config = manager.items()
    format_config = manager.get_format_config()

    return {
        'repl': {
            'no_color': config['no_color'],
            'theme': config['theme'],
        },
        'optimize': {
            'jit': config['jit'],
            'tco': config['tco'],
            'optimize': config['optimize'],
            'executor': config['executor'],
            'memory_limit': config['memory_limit'],
        },
        'format': {
            'indent_size': format_config.indent_size,
            'line_length': format_config.line_length,
        },
        'cache': {
            'enable_cache': config['enable_cache'],
            'cache_max_size_mb': config['cache_max_size_mb'],
            'cache_ttl_seconds': config['cache_ttl_seconds'],
        },
        'diagnostics': {
            'log_weird_errors': config['log_weird_errors'],
            'max_weird_logs': config['max_weird_logs'],
        },
    }


DEFAULT_CONFIG = _build_default_config()

VALID_KEYS = frozenset(valid_config_keys())
VALID_FORMAT_KEYS = frozenset(valid_format_keys())


def executor_to_vm_mode(executor: str) -> str:
    """Map executor name to vm_mode: 'vm' -> 'on', 'ast' -> 'off'."""
    if executor == 'vm':
        return 'on'
    if executor == 'ast':
        return 'off'
    return executor


def save_config(config: dict, path=None) -> None:
    """Save configuration to TOML file, preserving comments.

    Uses Rust set_config_value (toml_edit) for each key.
    """
    from pathlib import Path

    path = Path(path) if path else None
    for key, value in config.items():
        if key in VALID_KEYS or key in VALID_FORMAT_KEYS:
            target = f"format.{key}" if key in VALID_FORMAT_KEYS else key
            set_config_value(target, value, path)
