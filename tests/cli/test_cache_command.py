# FILE: tests/cli/test_cache_command.py
"""Tests for cache CLI formatting helpers."""

from catnip.cli.commands.cache import _format_volume


def test_format_volume_zero():
    assert _format_volume({"volume_bytes": 0}) == "0.00 MB"


def test_format_volume_bytes_for_tiny_values():
    assert _format_volume({"volume_bytes": 103}) == "103 bytes"


def test_format_volume_kb_for_small_values():
    assert _format_volume({"volume_bytes": 4096}) == "4.00 KB"


def test_format_volume_mb_for_regular_values():
    assert _format_volume({"volume_bytes": 20 * 1024}) == "0.02 MB"
