# FILE: catnip/config.py
"""Configuration helpers for Catnip with source tracking.

IMPORTANT: Core config functionality migrated to Rust (catnip._rs).
This module provides backward-compatible imports and legacy functions.
"""

# Re-export Rust implementations
from catnip._rs import (
    ConfigManager,
    ConfigSource,
    get_cache_dir,
    get_config_dir,
    get_config_path,
    get_data_dir,
    get_state_dir,
)

# Legacy constants for backward compatibility
CONFIG_FILE = "catnip.toml"

DEFAULT_CONFIG = {
    'repl': {
        'no_color': False,
    },
    'optimize': {
        'jit': False,
        'tco': True,
        'optimize': 3,
        'executor': 'vm',
    },
    'format': {
        'indent_size': 4,
        'line_length': 120,
    },
    'cache': {
        'cache_max_size_mb': None,  # None = unlimited
        'cache_ttl_seconds': None,  # None = unlimited
    },
}

VALID_KEYS = frozenset(['no_color', 'jit', 'tco', 'optimize', 'executor', 'cache_max_size_mb', 'cache_ttl_seconds'])
VALID_FORMAT_KEYS = frozenset(['indent_size', 'line_length'])


# --- Legacy functions (for backwards compatibility) ---


def load_config() -> dict:
    """Load configuration from TOML file, with defaults for missing keys."""
    manager = ConfigManager()
    manager.load_file()
    return manager.items()


def save_config(config: dict, path=None) -> None:
    """Save configuration to TOML file with sections.

    Args:
        config: Configuration dict to save (flat keys)
        path: Target file path (default: ~/.config/catnip/catnip.toml)

    Organizes keys into sections: [repl], [optimize], [format]
    """
    import tomllib
    from pathlib import Path

    path = Path(path) if path else get_config_path()
    path.parent.mkdir(parents=True, exist_ok=True)

    # Read existing file to preserve [format] and [cache]
    format_section = None
    cache_section = None
    if path.exists():
        with open(path, 'rb') as f:
            try:
                existing = tomllib.load(f)
                if 'format' in existing and isinstance(existing['format'], dict):
                    format_section = existing['format']
                if 'cache' in existing and isinstance(existing['cache'], dict):
                    cache_section = existing['cache']
            except Exception:
                pass

    def format_value(value):
        """Format a value for TOML."""
        if type(value) is bool:
            return str(value).lower()
        elif type(value) is str:
            return f'"{value}"'
        else:
            return str(value)

    lines = []

    # Section [repl]
    repl_keys = DEFAULT_CONFIG['repl'].keys()
    repl_values = {k: v for k, v in config.items() if k in repl_keys}
    if repl_values:
        lines.append('[repl]')
        for key in sorted(repl_values.keys()):
            lines.append(f"{key} = {format_value(repl_values[key])}")
        lines.append('')

    # Section [optimize]
    optimize_keys = DEFAULT_CONFIG['optimize'].keys()
    optimize_values = {k: v for k, v in config.items() if k in optimize_keys}
    if optimize_values:
        lines.append('[optimize]')
        for key in sorted(optimize_values.keys()):
            lines.append(f"{key} = {format_value(optimize_values[key])}")
        lines.append('')

    # Section [cache]
    cache_keys = DEFAULT_CONFIG['cache'].keys()
    cache_values = {k: v for k, v in config.items() if k in cache_keys}
    # Merge with existing cache section
    if cache_section:
        for key in cache_keys:
            if key not in cache_values and key in cache_section:
                cache_values[key] = cache_section[key]
    if cache_values:
        lines.append('[cache]')
        for key in sorted(cache_values.keys()):
            value = cache_values[key]
            if value is None:
                lines.append(f"# {key} = unlimited")
            else:
                lines.append(f"{key} = {format_value(value)}")
        lines.append('')

    # Section [format]
    if format_section:
        lines.append('[format]')
        for key in sorted(format_section.keys()):
            lines.append(f"{key} = {format_value(format_section[key])}")

    with open(path, 'w') as f:
        f.write('\n'.join(lines) + '\n')


def get_config_value(key: str):
    """Get a single config value."""
    if key not in VALID_KEYS:
        raise KeyError(f"Unknown config key: {key}")
    return load_config()[key]


def set_config_value(key: str, value) -> None:
    """Set a single config value."""
    if key not in VALID_KEYS:
        raise KeyError(f"Unknown config key: {key}")
    config = load_config()
    config[key] = value
    save_config(config)
