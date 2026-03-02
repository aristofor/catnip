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
        'theme': 'auto',
    },
    'optimize': {
        'jit': False,
        'tco': True,
        'optimize': 3,
        'executor': 'vm',
        'memory_limit': 512,
    },
    'format': {
        'indent_size': 4,
        'line_length': 120,
    },
    'cache': {
        'enable_cache': True,
        'cache_max_size_mb': None,  # None = unlimited
        'cache_ttl_seconds': None,  # None = unlimited
    },
    'diagnostics': {
        'log_weird_errors': True,
        'max_weird_logs': 50,
    },
}

# Reverse mapping: flat key -> section name
_KEY_TO_SECTION = {}
for _section, _keys in DEFAULT_CONFIG.items():
    for _key in _keys:
        _KEY_TO_SECTION[_key] = _section

VALID_KEYS = frozenset(
    [
        'no_color',
        'jit',
        'tco',
        'optimize',
        'executor',
        'cache_max_size_mb',
        'cache_ttl_seconds',
        'theme',
        'memory_limit',
        'enable_cache',
        'log_weird_errors',
        'max_weird_logs',
    ]
)
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
    """
    import tomllib
    from pathlib import Path

    path = Path(path) if path else get_config_path()
    path.parent.mkdir(parents=True, exist_ok=True)

    # Read existing file to preserve sections not in config
    existing = {}
    if path.exists():
        with open(path, 'rb') as f:
            try:
                existing = tomllib.load(f)
            except Exception:
                pass

    # Group flat config into sections
    sections = {}
    for key, value in config.items():
        section = _KEY_TO_SECTION.get(key)
        if section:
            sections.setdefault(section, {})[key] = value

    # Merge with existing sections (preserve keys not in current config)
    for section_name in DEFAULT_CONFIG:
        if section_name in existing and isinstance(existing[section_name], dict):
            for key, val in existing[section_name].items():
                sections.setdefault(section_name, {}).setdefault(key, val)

    _write_toml(sections, path)


def get_config_value(key: str):
    """Get a single config value."""
    if key not in VALID_KEYS:
        raise KeyError(f"Unknown config key: {key}")
    return load_config()[key]


def set_config_value(key: str, value) -> None:
    """Set a single config value (supports format.key for format keys)."""
    if key.startswith('format.'):
        fmt_key = key[7:]
        if fmt_key not in VALID_FORMAT_KEYS:
            raise KeyError(f"Unknown format config key: {fmt_key}")
        _set_format_value(fmt_key, value)
        return
    if key not in VALID_KEYS:
        raise KeyError(f"Unknown config key: {key}")
    config = load_config()
    config[key] = value
    save_config(config)


def _set_format_value(key: str, value) -> None:
    """Set a format config value (indent_size, line_length)."""
    import tomllib
    from pathlib import Path

    path = get_config_path()
    path.parent.mkdir(parents=True, exist_ok=True)

    existing = {}
    if path.exists():
        with open(path, 'rb') as f:
            try:
                existing = tomllib.load(f)
            except Exception:
                pass

    format_section = existing.get('format', {})
    if not isinstance(format_section, dict):
        format_section = {}
    format_section[key] = value

    existing['format'] = format_section
    _write_toml(existing, path)


def _write_toml(data: dict, path) -> None:
    """Write a nested config dict as TOML."""

    def format_value(value):
        if type(value) is bool:
            return str(value).lower()
        elif type(value) is str:
            return f'"{value}"'
        elif value is None:
            return None
        else:
            return str(value)

    lines = []
    for section_name in ['repl', 'optimize', 'cache', 'diagnostics', 'format']:
        section = data.get(section_name)
        if not section or not isinstance(section, dict):
            continue
        lines.append(f'[{section_name}]')
        for key in sorted(section.keys()):
            val = format_value(section[key])
            if val is None:
                lines.append(f"# {key} = unlimited")
            else:
                lines.append(f"{key} = {val}")
        lines.append('')

    with open(path, 'w') as f:
        f.write('\n'.join(lines) + '\n')
