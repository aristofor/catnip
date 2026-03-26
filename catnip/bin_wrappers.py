# FILE: catnip/bin_wrappers.py
"""Wrappers for Rust binaries included in the package."""

import os
import sys
from pathlib import Path


def _find_binary(name):
    """Find the Rust binary in the package or PATH."""
    shim_path = Path(sys.argv[0]).resolve() if sys.argv and sys.argv[0] else None

    # 1. Chercher dans le package (wheel installé)
    package_dir = Path(__file__).parent
    bin_path = package_dir / 'bin' / name
    if bin_path.exists() and os.access(bin_path, os.X_OK):
        return bin_path

    # 2. Chercher dans le PATH (installation dev)
    import shutil

    system_path = shutil.which(name)
    if system_path:
        system_path_obj = Path(system_path).resolve()
        # Avoid wrapper recursion when the console-script resolves to itself.
        if shim_path is not None and system_path_obj == shim_path:
            return None
        return system_path_obj

    return None


def catnip_repl():
    """Entry point for the catnip-repl binary."""
    binary = _find_binary('catnip-repl')
    if not binary:
        print("Error: catnip-repl binary not found", file=sys.stderr)
        print("Install with: make install-bins", file=sys.stderr)
        sys.exit(1)

    os.execv(binary, [binary] + sys.argv[1:])
