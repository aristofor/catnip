# FILE: tests/language/test_weird_log.py
"""Tests for CatnipWeirdError crash logging (Rust implementation)."""

import json
import os
import stat

import pytest


@pytest.fixture(autouse=True)
def _isolate_weird_log(monkeypatch, tmp_path):
    """Redirect weird logs to a temp dir."""
    state_dir = tmp_path / 'state'
    monkeypatch.setenv('XDG_STATE_HOME', str(state_dir))
    monkeypatch.delenv('CATNIP_WEIRD_LOG', raising=False)
    # Point config to nonexistent dir so TOML doesn't interfere
    monkeypatch.setenv('XDG_CONFIG_HOME', str(tmp_path / 'no_config'))


def _weird_dir(tmp_path):
    return tmp_path / 'state' / 'catnip' / 'weird'


def test_weird_error_creates_json(tmp_path):
    """A WeirdError should produce a JSON file on disk."""
    from catnip.exc import CatnipWeirdError

    CatnipWeirdError('test crash')

    wd = _weird_dir(tmp_path)
    files = list(wd.glob('weird_*.json'))
    assert len(files) == 1


def test_json_content(tmp_path):
    """Check that the JSON report has all expected fields."""
    from catnip.exc import CatnipWeirdError

    CatnipWeirdError('boom', cause='vm', details={'op': 42})

    wd = _weird_dir(tmp_path)
    data = json.loads(next(wd.glob('weird_*.json')).read_text())

    assert data['version'] == 1
    assert 'timestamp' in data
    assert data['catnip_version']
    assert data['python_version']
    assert data['platform']
    assert data['error']['message'] == 'boom'
    assert data['error']['cause'] == 'vm'
    assert data['error']['details'] == {'op': 42}
    assert 'location' in data


def test_location_fields(tmp_path):
    """Location info from the exception should appear in the report."""
    from catnip.exc import CatnipWeirdError

    CatnipWeirdError('loc test', filename='script.cat', line=10, column=5)

    wd = _weird_dir(tmp_path)
    data = json.loads(next(wd.glob('weird_*.json')).read_text())

    assert data['location']['filename'] == 'script.cat'
    assert data['location']['line'] == 10
    assert data['location']['column'] == 5


def test_rotation(tmp_path):
    """Should keep at most 50 log files."""
    from catnip.exc import CatnipWeirdError

    for i in range(55):
        CatnipWeirdError(f'error {i}')

    wd = _weird_dir(tmp_path)
    files = list(wd.glob('weird_*.json'))
    assert len(files) <= 50


def test_disabled_via_env_var(monkeypatch, tmp_path):
    """CATNIP_WEIRD_LOG=off should suppress logging."""
    monkeypatch.setenv('CATNIP_WEIRD_LOG', 'off')

    from catnip.exc import CatnipWeirdError

    CatnipWeirdError('should not log')

    wd = _weird_dir(tmp_path)
    assert not wd.exists() or len(list(wd.glob('weird_*.json'))) == 0


def test_enabled_via_env_var(monkeypatch, tmp_path):
    """CATNIP_WEIRD_LOG=on should enable logging."""
    monkeypatch.setenv('CATNIP_WEIRD_LOG', 'on')

    from catnip.exc import CatnipWeirdError

    CatnipWeirdError('should log')

    wd = _weird_dir(tmp_path)
    assert len(list(wd.glob('weird_*.json'))) == 1


def test_disabled_via_toml(monkeypatch, tmp_path):
    """TOML config [diagnostics].log_weird_errors = false should disable logging."""
    config_dir = tmp_path / 'config' / 'catnip'
    config_dir.mkdir(parents=True)
    config_file = config_dir / 'catnip.toml'
    config_file.write_text('[diagnostics]\nlog_weird_errors = false\n')
    monkeypatch.setenv('XDG_CONFIG_HOME', str(tmp_path / 'config'))

    from catnip.exc import CatnipWeirdError

    CatnipWeirdError('toml disabled')

    wd = _weird_dir(tmp_path)
    assert not wd.exists() or len(list(wd.glob('weird_*.json'))) == 0


def test_silent_on_io_error(monkeypatch, tmp_path):
    """I/O errors should be swallowed silently."""
    # Point state dir to a read-only location
    readonly_dir = tmp_path / 'readonly'
    readonly_dir.mkdir()
    readonly_dir.chmod(stat.S_IRUSR | stat.S_IXUSR)
    monkeypatch.setenv('XDG_STATE_HOME', str(readonly_dir))

    from catnip.exc import CatnipWeirdError

    # Should not raise
    try:
        CatnipWeirdError('io fail')
    finally:
        # Restore permissions for cleanup
        readonly_dir.chmod(stat.S_IRWXU)


def test_atomic_write(tmp_path):
    """No .tmp files should remain after a successful write."""
    from catnip.exc import CatnipWeirdError

    CatnipWeirdError('atomic test')

    wd = _weird_dir(tmp_path)
    tmp_files = list(wd.glob('*.tmp'))
    assert len(tmp_files) == 0


def test_rust_log_directly(tmp_path):
    """Test calling the Rust function directly (no Python exception needed)."""
    from catnip._rs import log_weird_error_py

    log_weird_error_py(
        message='direct rust call',
        cause='test',
    )

    wd = _weird_dir(tmp_path)
    files = list(wd.glob('weird_*.json'))
    assert len(files) == 1
    data = json.loads(files[0].read_text())
    assert data['error']['message'] == 'direct rust call'
    assert data['error']['cause'] == 'test'
    # No python_version when called without it
    assert data.get('python_version') is None
