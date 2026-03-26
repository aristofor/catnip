# FILE: catnip/extensions.py
"""Compiled extension system for Catnip.

Extensions are Python modules (pure or PyO3) with a ``__catnip_extension__``
attribute exposing name, version, optional register hook and exports dict.
"""

from __future__ import annotations

import importlib.metadata
import logging

log = logging.getLogger(__name__)

ENTRY_POINT_GROUP = 'catnip.extensions'


class ExtensionInfo:
    """Metadata for a loaded extension."""

    __slots__ = ('name', 'version', 'description', 'module')

    def __init__(self, name: str, version: str, description: str, module):
        self.name = name
        self.version = version
        self.description = description
        self.module = module

    def to_dict(self):
        return dict(name=self.name, version=self.version, description=self.description)

    def __repr__(self):
        return f"<Extension {self.name} {self.version}>"


def validate_extension(module) -> dict | None:
    """Return the extension descriptor dict, or None if not an extension.

    Raises ValueError if ``__catnip_extension__`` exists but is malformed.
    """
    descriptor = getattr(module, '__catnip_extension__', None)
    if descriptor is None:
        return None

    if not isinstance(descriptor, dict):
        raise ValueError(f"__catnip_extension__ must be a dict, got {type(descriptor).__name__}")

    for key in ('name', 'version'):
        if key not in descriptor:
            raise ValueError(f"__catnip_extension__ missing required key '{key}'")
        if not isinstance(descriptor[key], str):
            raise ValueError(f"__catnip_extension__['{key}'] must be str, got {type(descriptor[key]).__name__}")

    register = descriptor.get('register')
    if register is not None and not callable(register):
        raise ValueError("__catnip_extension__['register'] must be callable")

    exports = descriptor.get('exports')
    if exports is not None and not isinstance(exports, dict):
        raise ValueError("__catnip_extension__['exports'] must be a dict")

    return descriptor


def discover_extensions() -> dict[str, importlib.metadata.EntryPoint]:
    """Discover installed extensions via entry points (without importing)."""
    eps = importlib.metadata.entry_points()
    if isinstance(eps, dict):
        entries = eps.get(ENTRY_POINT_GROUP, [])
    else:
        entries = eps.select(group=ENTRY_POINT_GROUP)
    return {ep.name: ep for ep in entries}


def load_extension(module, context, *, verbose: bool = False) -> ExtensionInfo:
    """Validate, register and inject an extension module.

    1. Validate the ``__catnip_extension__`` descriptor
    2. Call the optional ``register(context)`` hook
    3. Inject ``exports`` into context globals
    4. Track in ``context._extensions``

    Returns the ExtensionInfo on success.
    Raises ValueError if the descriptor is invalid.
    """
    descriptor = validate_extension(module)
    if descriptor is None:
        raise ValueError(f"{module} is not a Catnip extension")

    name = descriptor['name']
    version = descriptor['version']
    description = descriptor.get('description', '')

    if verbose:
        log.info("loading extension %s %s", name, version)

    # register hook
    register = descriptor.get('register')
    if register is not None:
        register(context)

    # exports injection
    exports = descriptor.get('exports')
    if exports is not None:
        context.globals.update(exports)

    info = ExtensionInfo(name, version, description, module)
    context._extensions[name] = info
    return info
