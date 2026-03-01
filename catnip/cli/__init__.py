# FILE: catnip/cli/__init__.py
"""CLI with plugin support via entry points."""

from .main import main
from .plugins import ENTRY_POINT_GROUP, discover_plugins, load_plugin

__all__ = (
    'ENTRY_POINT_GROUP',
    'discover_plugins',
    'load_plugin',
    'main',
)
