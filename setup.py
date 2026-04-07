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


class BuildPyCustom(_build_py):
    """Custom build_py: copies optional Rust binaries."""

    def run(self):
        super().run()
        self._copy_binaries()

    def _copy_binaries(self):
        """Copy pre-built Rust binaries if available."""
        bin_dir = Path(self.build_lib) / 'catnip' / 'bin'
        bin_dir.mkdir(parents=True, exist_ok=True)

        for binary in ['catnip', 'catnip-repl']:
            src = Path('target/bins/release') / binary
            dst = bin_dir / binary

            if src.exists():
                self.announce(f'Including Rust binary: {binary}', level=3)
                shutil.copy2(src, dst)
                os.chmod(dst, 0o755)
            else:
                self.announce(f'Skipping {binary} (not found, run make build-bins first)', level=2)


# CATNIP_DEV=1 → profil "fastdev" (incremental, thin LTO, codegen-units=16)
_dev_mode = os.environ.get('CATNIP_DEV', '') == '1'
_profile_args = ('--profile', 'fastdev') if _dev_mode else ()

# Extensions Rust (PyO3)
rust_extensions = [
    RustExtension(
        "catnip._rs",
        path="catnip_rs/Cargo.toml",
        binding=Binding.PyO3,
        debug=False,
        args=_profile_args,
    ),
    RustExtension(
        "catnip._repl",
        path="catnip_repl/Cargo.toml",
        binding=Binding.PyO3,
        debug=False,
        args=_profile_args,
    ),
    # @generated-stdlib-extensions-start
    RustExtension(
        "catnip.catnip_io",
        path="catnip_libs/io/rust/Cargo.toml",
        binding=Binding.PyO3,
        debug=False,
        args=_profile_args,
    ),
    RustExtension(
        "catnip.catnip_sys",
        path="catnip_libs/sys/rust/Cargo.toml",
        binding=Binding.PyO3,
        debug=False,
        args=_profile_args,
    ),
    # @generated-stdlib-extensions-end
]

setup(
    rust_extensions=rust_extensions,
    cmdclass={
        'build_py': BuildPyCustom,
    },
)
