# FILE: tests/serial/test_repl_process.py
"""Subprocess tests for REPL / CLI process behavior."""

import subprocess
import sys

import pytest

CATNIP = [sys.executable, "-m", "catnip"]


class TestReplProcess:
    def test_repl_eval_pipe(self):
        """echo "1 + 2" | catnip returns 3"""
        result = subprocess.run(
            CATNIP,
            input="1 + 2\n",
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode == 0
        assert "3" in result.stdout

    def test_repl_c_flag(self):
        """catnip -c "42" returns 42"""
        result = subprocess.run(
            [*CATNIP, "-c", "42"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode == 0
        assert "42" in result.stdout

    def test_repl_syntax_error_exit(self):
        """catnip -c "if" returns non-zero exit code"""
        result = subprocess.run(
            [*CATNIP, "-c", "if"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode != 0
