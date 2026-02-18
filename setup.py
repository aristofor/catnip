"""
Minimal setup.py pour l'extension Rust.
La configuration principale est dans pyproject.toml.
"""

import os
import shutil
from pathlib import Path

from setuptools import setup
from setuptools.command.build_py import build_py as _build_py
from setuptools_rust import Binding, RustExtension


class BuildPyWithBinaries(_build_py):
    """Custom build_py qui inclut les binaires Rust précompilés."""

    def run(self):
        # Build standard Python package
        super().run()

        # Copy pre-built binaries if they exist
        # Note: Binaries must be compiled separately with:
        #   cargo build --release --bin catnip-standalone --no-default-features --features embedded
        #   cargo build --release --bin catnip-repl --no-default-features --features repl-standalone
        bin_dir = Path(self.build_lib) / 'catnip' / 'bin'
        bin_dir.mkdir(parents=True, exist_ok=True)

        for binary in ['catnip-standalone', 'catnip-repl']:
            src = Path('target/release') / binary
            dst = bin_dir / binary

            if src.exists():
                self.announce(f'Including Rust binary: {binary}', level=3)
                shutil.copy2(src, dst)
                # Rendre exécutable
                os.chmod(dst, 0o755)
            else:
                self.announce(f'Skipping {binary} (not found, run make build-bins first)', level=2)


# Extensions Rust (PyO3)
rust_extensions = [
    RustExtension(
        "catnip._rs",
        path="catnip_rs/Cargo.toml",
        binding=Binding.PyO3,
        debug=False,
    ),
    RustExtension(
        "catnip._repl",
        path="catnip_repl/Cargo.toml",
        binding=Binding.PyO3,
        debug=False,
    ),
]

setup(
    rust_extensions=rust_extensions,
    cmdclass={
        'build_py': BuildPyWithBinaries,
    },
)
