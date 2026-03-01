# FILE: catnip/_version.py
"""Version information for Catnip."""

from datetime import datetime
from pathlib import Path

__version__ = "0.0.6"
__lang_id__ = "catnip"


def _get_build_date():
    """
    Determine the language build date from compiled Rust extension.

    Uses the most recent timestamp among .so modules to automatically detect
    when the language was recompiled. This allows automatic cache invalidation
    upon recompilation.

    Returns:
        str: Date in ISO format (YYYY-MM-DD-HH:MM:SS) or None if no compiled modules
    """
    try:
        # Search for compiled Rust extension
        libs_dir = Path(__file__).parent
        if not libs_dir.exists():
            return None

        max_mtime = 0
        for so_file in libs_dir.rglob("*.so"):
            mtime = so_file.stat().st_mtime
            max_mtime = max(max_mtime, mtime)

        if max_mtime > 0:
            # Format with seconds to detect even fast recompilations
            return datetime.fromtimestamp(max_mtime).strftime("%F-%T")

    except Exception:
        pass

    return None


# Build date automatically computed from compiled modules
# Used to invalidate cache upon recompilations
__build_date__ = _get_build_date() or "0000-00-00-00:00:00"
