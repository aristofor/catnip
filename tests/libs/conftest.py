# FILE: tests/libs/conftest.py
"""Fixtures for catnip_libs backend testing."""

import importlib

import pytest

# Mapping: module name -> installed package name
_INSTALLED_MODULES = {
    'io': 'catnip.catnip_io',
    'sys': 'catnip.catnip_sys',
}


def _load_rust_module(module_name):
    """Load a Rust backend from installed package."""
    pkg = _INSTALLED_MODULES.get(module_name)
    if not pkg:
        pytest.skip(f"No installed package mapping for {module_name!r}")
    try:
        return importlib.import_module(pkg)
    except ImportError:
        pytest.skip(f"{pkg} not installed")


@pytest.fixture(scope='session')
def io_rust():
    """Load io module Rust backend."""
    return _load_rust_module('io')
