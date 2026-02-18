# FILE: tests/cli/test_config.py
from pathlib import Path

from catnip.config import get_cache_dir, get_config_dir, get_data_dir, get_state_dir


def test_get_config_dir_uses_xdg_config_home(monkeypatch, tmp_path):
    xdg_config_home = tmp_path / "xdg_config"
    monkeypatch.setenv('XDG_CONFIG_HOME', str(xdg_config_home))
    assert get_config_dir() == xdg_config_home / "catnip"


def test_get_config_dir_fallbacks_to_home(monkeypatch, tmp_path):
    monkeypatch.delenv('XDG_CONFIG_HOME', raising=False)
    fake_home = tmp_path / "home"
    monkeypatch.setenv('HOME', str(fake_home))
    assert get_config_dir() == fake_home / ".config" / "catnip"


def test_get_state_dir_uses_xdg_state_home(monkeypatch, tmp_path):
    xdg_state_home = tmp_path / "xdg_state"
    monkeypatch.setenv('XDG_STATE_HOME', str(xdg_state_home))
    assert get_state_dir() == xdg_state_home / "catnip"


def test_get_state_dir_fallbacks_to_home(monkeypatch, tmp_path):
    monkeypatch.delenv('XDG_STATE_HOME', raising=False)
    fake_home = tmp_path / "home"
    monkeypatch.setenv('HOME', str(fake_home))
    assert get_state_dir() == fake_home / ".local" / "state" / "catnip"


def test_get_cache_dir_uses_xdg_cache_home(monkeypatch, tmp_path):
    xdg_cache_home = tmp_path / "xdg_cache"
    monkeypatch.setenv('XDG_CACHE_HOME', str(xdg_cache_home))
    assert get_cache_dir() == xdg_cache_home / "catnip"


def test_get_cache_dir_fallbacks_to_home(monkeypatch, tmp_path):
    monkeypatch.delenv('XDG_CACHE_HOME', raising=False)
    fake_home = tmp_path / "home"
    monkeypatch.setenv('HOME', str(fake_home))
    assert get_cache_dir() == fake_home / ".cache" / "catnip"


def test_get_data_dir_uses_xdg_data_home(monkeypatch, tmp_path):
    xdg_data_home = tmp_path / "xdg_data"
    monkeypatch.setenv('XDG_DATA_HOME', str(xdg_data_home))
    assert get_data_dir() == xdg_data_home / "catnip"


def test_get_data_dir_fallbacks_to_home(monkeypatch, tmp_path):
    monkeypatch.delenv('XDG_DATA_HOME', raising=False)
    fake_home = tmp_path / "home"
    monkeypatch.setenv('HOME', str(fake_home))
    assert get_data_dir() == fake_home / ".local" / "share" / "catnip"
