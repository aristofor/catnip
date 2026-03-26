# FILE: tests/conftest.py
"""
Pytest configuration and shared fixtures for Catnip tests.

Provides fixtures for conditional test skipping based on optimization levels
and TCO state.
"""

import os

import pytest

from catnip import Catnip


def pytest_addoption(parser):
    """Add custom command-line options."""
    # Get default from env var
    default = os.environ.get('CATNIP_EXECUTOR', 'vm').lower()
    if default not in {'vm', 'ast'}:
        default = 'vm'

    parser.addoption(
        '--executor',
        action='store',
        default=default,
        choices=['vm', 'ast', 'standalone'],
        help="Execution mode: vm (default), ast, standalone",
    )


# --- Custom pytest markers ---


def pytest_configure(config):
    """Register custom markers and propagate executor mode."""
    # Propagate --executor option to environment variable
    # so tests creating Catnip() directly also see the mode
    executor = config.getoption('--executor', default=None)
    if executor:
        os.environ['CATNIP_EXECUTOR'] = executor

    # Monkey-patch Catnip → CatnipStandalone when in standalone mode
    if executor == 'standalone':
        import catnip
        from catnip.compat import CatnipStandalone

        catnip.Catnip = CatnipStandalone

    config.addinivalue_line(
        'markers',
        "requires_tco: skip test if TCO is disabled",
    )
    config.addinivalue_line(
        'markers',
        "requires_optimization(level): skip test if optimize_level < level",
    )
    config.addinivalue_line(
        'markers',
        "no_vm: skip test when running with VM (unsupported features)",
    )
    config.addinivalue_line(
        'markers',
        "serial: skip test when running under pytest-xdist",
    )
    config.addinivalue_line(
        'markers',
        "no_standalone: skip test in standalone mode (features requiring Python Context)",
    )


# --- Fixtures for different configurations ---


@pytest.fixture(scope='session')
def vm_mode(request):
    """Get VM mode from command line (mapped to internal values)."""
    executor = request.config.getoption('--executor')
    # Map to internal values: vm→on, ast→off
    internal_map = {'vm': 'on', 'ast': 'off'}
    return internal_map.get(executor, 'on')


def pytest_collection_modifyitems(config, items):
    """Skip tests marked with no_vm (unsupported in both VM and AST modes)."""
    executor = config.getoption('--executor')
    if executor in ('vm', 'ast'):
        skip_vm = pytest.mark.skip(reason=f"Test not supported with executor={executor}")
        for item in items:
            if 'no_vm' in item.keywords:
                item.add_marker(skip_vm)

    if executor == 'standalone':
        skip_sa = pytest.mark.skip(reason="Test not supported with standalone executor")
        for item in items:
            if 'no_standalone' in item.keywords:
                item.add_marker(skip_sa)

    # Skip serial-only tests when running under pytest-xdist.
    numprocesses = getattr(config.option, 'numprocesses', None)
    dist_mode = getattr(config.option, 'dist', None)
    xdist_enabled = (numprocesses not in (None, 0)) or (dist_mode not in (None, 'no'))
    if xdist_enabled:
        skip_serial = pytest.mark.skip(reason="Test requires serial execution (xdist enabled)")
        for item in items:
            if 'serial' in item.keywords:
                item.add_marker(skip_serial)


@pytest.fixture
def cat(vm_mode, request):
    """Standard Catnip instance with default settings."""
    executor = request.config.getoption('--executor')
    if executor == 'standalone':
        from catnip.compat import CatnipStandalone

        return CatnipStandalone()
    return Catnip(vm_mode=vm_mode)


@pytest.fixture
def cat_no_tco(vm_mode):
    """Catnip instance with TCO disabled."""
    c = Catnip(vm_mode=vm_mode)
    c.pragma_context.tco_enabled = False
    return c


@pytest.fixture
def cat_no_opt(vm_mode):
    """Catnip instance with optimization disabled (level 0)."""
    c = Catnip(vm_mode=vm_mode)
    c.pragma_context.optimize_level = 0
    return c


@pytest.fixture
def cat_opt_level(request, vm_mode):
    """
    Parametrized fixture for specific optimization level.

    Usage:
        @pytest.mark.parametrize('cat_opt_level', [0, 1, 2, 3], indirect=True)
        def test_something(cat_opt_level):
            ...
    """
    c = Catnip(vm_mode=vm_mode)
    c.pragma_context.optimize_level = request.param
    return c


# --- Skip fixtures ---


@pytest.fixture
def skip_if_no_tco():
    """
    Fixture that skips the test if TCO is disabled.

    Usage:
        def test_tco_feature(skip_if_no_tco, cat):
            ...
    """
    c = Catnip()
    if not c.pragma_context.tco_enabled:
        pytest.skip("TCO is disabled")


@pytest.fixture
def skip_if_no_optimization():
    """
    Fixture that skips the test if optimization is disabled (level 0).

    Usage:
        def test_optimizer_feature(skip_if_no_optimization, cat):
            ...
    """
    c = Catnip()
    if c.pragma_context.optimize_level == 0:
        pytest.skip("Optimization is disabled (level 0)")
