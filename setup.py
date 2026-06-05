"""
Minimal setup.py pour l'extension Rust.
La configuration principale est dans pyproject.toml.
"""

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

from setuptools import setup
from setuptools.command.build_py import build_py as _build_py
from setuptools_rust import Binding, RustExtension


def _discover_native_libs():
    """Names of native (PyO3-less) stdlib plugins to build as catnip_vm cdylib
    `.so` plugins. Source of truth: catnip_libs/*/spec.toml with
    `backends.pyo3 = false`. setuptools-rust builds the PyO3 ones (io, sys);
    these have no Python binding so they are built + bundled here instead.

    Kept dependency-free (no tomllib) so it works on every cibuildwheel Python.
    """
    libs = []
    for spec in sorted(Path('catnip_libs').glob('*/spec.toml')):
        text = spec.read_text()
        if not re.search(r'pyo3\s*=\s*false', text):
            continue
        m = re.search(r'^\s*name\s*=\s*"([^"]+)"', text, re.M)
        libs.append(m.group(1) if m else spec.parent.name)
    return libs


class BuildPyCustom(_build_py):
    """Custom build_py: builds native stdlib plugins and copies Rust binaries."""

    def run(self):
        super().run()
        self._build_native_plugins()
        self._copy_binaries()

    def _build_native_plugins(self):
        """Build native catnip_vm cdylib plugins (e.g. `http`) and copy them next
        to the extension so normal installs/wheels include them."""
        libs = _discover_native_libs()
        if not libs:
            return

        profile = 'fastdev' if _dev_mode else 'release'
        suffix = '.dylib' if sys.platform == 'darwin' else '.so'
        pkg_dir = Path(self.build_lib) / 'catnip'
        pkg_dir.mkdir(parents=True, exist_ok=True)

        for name in libs:
            self.announce(f'Building native stdlib plugin: {name}', level=3)
            cmd = ['cargo', 'build', '-p', f'catnip-{name}']
            cmd += list(_profile_args) if _dev_mode else ['--release']
            subprocess.run(cmd, check=True)
            lib_name = f'libcatnip_{name}{suffix}'
            shutil.copy2(Path('target') / profile / lib_name, pkg_dir / lib_name)

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
