# FILE: tests/tools/test_json_serialization.py
"""Integration tests for JSON IR serialization APIs and CLI output."""

import json
import subprocess

import pytest

from catnip import Catnip
from catnip._rs import (
    ir_to_json,
    ir_to_json_compact,
    ir_to_json_compact_pretty,
    ir_to_json_pretty,
    process_input as rust_process_input,
)


@pytest.fixture
def cat():
    return Catnip()


def test_python_api_json_shapes(cat):
    """Python API returns valid JSON with expected top-level contracts."""
    ir = rust_process_input(cat, "2 + 3", 1)[0]

    regular = json.loads(ir_to_json(ir))
    pretty = json.loads(ir_to_json_pretty(ir))
    compact = json.loads(ir_to_json_compact(ir))
    compact_pretty = json.loads(ir_to_json_compact_pretty(ir))

    assert "Op" in regular
    assert "Op" in pretty
    assert compact["op"] == "Add"
    assert compact_pretty["op"] == "Add"


def test_compact_contract(cat):
    """Compact JSON contract: no serde wrapper, args/kwargs present, no tail=false."""
    ir = rust_process_input(cat, "2 + 3", 1)[0]
    data = json.loads(ir_to_json_compact(ir))

    assert "Op" not in data
    assert data["op"] == "Add"
    assert "args" in data
    assert "kwargs" in data
    assert "tail" not in data


def test_cli_json_format_stdin():
    """CLI --format json emits valid JSON list from stdin."""
    result = subprocess.run(
        ["catnip", "-p", "1", "--format", "json"],
        input="2 + 3",
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"stderr: {result.stderr}"
    data = json.loads(result.stdout)
    assert isinstance(data, list)
    assert len(data) == 1


def test_cli_json_format_command_mode():
    """CLI --format json emits valid JSON list with -c."""
    result = subprocess.run(
        ["catnip", "-p", "1", "--format", "json", "-c", "5 + 10"],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"stderr: {result.stderr}"
    data = json.loads(result.stdout)
    assert isinstance(data, list)
    assert len(data) == 1


def test_cli_default_compact_output():
    """Default CLI format is compact JSON."""
    result = subprocess.run(
        ["catnip", "-p", "1", "-c", "2 + 3"],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"stderr: {result.stderr}"
    data = json.loads(result.stdout)
    assert isinstance(data, list)
    assert data[0]["op"] == "Add"


def test_cli_repr_output():
    """CLI --format repr returns repr-like output, not JSON."""
    result = subprocess.run(
        ["catnip", "-p", "1", "--format", "repr", "-c", "2 + 3"],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"stderr: {result.stderr}"
    output = result.stdout.strip()
    assert "<IR" in output or "Op(" in output or "ident=" in output
