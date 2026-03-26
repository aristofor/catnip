# FILE: tests/libs/test_io.py
"""Tests for the io module (Rust backend via catnip_libs standalone crate)."""

import builtins
import io
import os
import sys
import tempfile

import pytest

builtins_open = builtins.open

# Helpers


def capture_stdout(mod, func_name, *args, **kwargs):
    """Call mod.<func_name> and capture what it writes to stdout."""
    buf = io.StringIO()
    getattr(mod, func_name)(*args, file=buf, **kwargs)
    return buf.getvalue()


class TestIORust:
    """IO module Rust backend."""

    def test_protocol(self, io_rust):
        assert io_rust.PROTOCOL == "rust"

    def test_version(self, io_rust):
        assert io_rust.VERSION == "0.1.0"

    def test_print_basic(self, io_rust):
        assert capture_stdout(io_rust, "print", "hello") == "hello\n"

    def test_print_multiple(self, io_rust):
        assert capture_stdout(io_rust, "print", "a", "b", "c") == "a b c\n"

    def test_print_sep(self, io_rust):
        assert capture_stdout(io_rust, "print", "a", "b", sep="-") == "a-b\n"

    def test_print_end(self, io_rust):
        assert capture_stdout(io_rust, "print", "x", end="!") == "x!"

    def test_write(self, io_rust):
        assert capture_stdout(io_rust, "write", "ab", "cd") == "abcd"

    def test_writeln(self, io_rust):
        assert capture_stdout(io_rust, "writeln", "ab", "cd") == "abcd\n"

    def test_input(self, io_rust):
        old = sys.stdin
        sys.stdin = io.StringIO("hello\n")
        try:
            result = io_rust.input()
            assert result == "hello"
        finally:
            sys.stdin = old

    def test_input_prompt(self, io_rust):
        old = sys.stdin
        sys.stdin = io.StringIO("val\n")
        buf = io.StringIO()
        old_out = sys.stdout
        sys.stdout = buf
        try:
            result = io_rust.input(">> ")
            assert result == "val"
            assert buf.getvalue() == ">> "
        finally:
            sys.stdin = old
            sys.stdout = old_out

    def test_input_eof(self, io_rust):
        old = sys.stdin
        sys.stdin = io.StringIO("")
        try:
            with pytest.raises(EOFError):
                io_rust.input()
        finally:
            sys.stdin = old

    def test_print_empty(self, io_rust):
        assert capture_stdout(io_rust, "print") == "\n"

    def test_print_numbers(self, io_rust):
        assert capture_stdout(io_rust, "print", 1, 2, 3) == "1 2 3\n"

    def test_print_mixed_types(self, io_rust):
        assert capture_stdout(io_rust, "print", "x", 42, True) == "x 42 True\n"

    def test_print_custom_sep_end(self, io_rust):
        assert capture_stdout(io_rust, "print", "a", "b", sep=",", end=".") == "a,b."

    def test_write_no_trailing(self, io_rust):
        result = capture_stdout(io_rust, "write", "test")
        assert not result.endswith("\n")

    def test_writeln_trailing(self, io_rust):
        result = capture_stdout(io_rust, "writeln", "test")
        assert result == "test\n"

    def test_input_strips_newline(self, io_rust):
        old = sys.stdin
        sys.stdin = io.StringIO("line\n")
        try:
            assert io_rust.input() == "line"
        finally:
            sys.stdin = old

    def test_open_read(self, io_rust):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as tmp:
            tmp.write("hello catnip")
            path = tmp.name
        try:
            f = io_rust.open(path, "r")
            assert f.read() == "hello catnip"
            f.close()
        finally:
            os.unlink(path)

    def test_open_write(self, io_rust):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as tmp:
            path = tmp.name
        try:
            f = io_rust.open(path, "w")
            f.write("written")
            f.close()
            f = io_rust.open(path, "r")
            assert f.read() == "written"
            f.close()
        finally:
            os.unlink(path)

    def test_open_encoding(self, io_rust):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False, encoding='utf-8') as tmp:
            tmp.write("caf\u00e9")
            path = tmp.name
        try:
            f = io_rust.open(path, "r", encoding="utf-8")
            assert f.read() == "caf\u00e9"
            f.close()
        finally:
            os.unlink(path)

    def test_open_default_mode(self, io_rust):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as tmp:
            tmp.write("default")
            path = tmp.name
        try:
            f = io_rust.open(path)
            assert f.read() == "default"
            f.close()
        finally:
            os.unlink(path)

    def test_open_binary_mode(self, io_rust):
        with tempfile.NamedTemporaryFile(suffix='.bin', delete=False) as tmp:
            tmp.write(b"\x00\x01\x02\x03")
            path = tmp.name
        try:
            f = io_rust.open(path, "rb")
            assert f.read() == b"\x00\x01\x02\x03"
            f.close()
        finally:
            os.unlink(path)

    def test_open_append_mode(self, io_rust):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as tmp:
            tmp.write("first")
            path = tmp.name
        try:
            f = io_rust.open(path, "a")
            f.write("second")
            f.close()
            f = io_rust.open(path, "r")
            assert f.read() == "firstsecond"
            f.close()
        finally:
            os.unlink(path)

    def test_open_buffering(self, io_rust):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as tmp:
            tmp.write("buffered")
            path = tmp.name
        try:
            # buffering=0 only valid for binary mode
            f = io_rust.open(path, "rb", buffering=0)
            assert f.read() == b"buffered"
            f.close()
        finally:
            os.unlink(path)

    def test_open_errors(self, io_rust):
        with tempfile.NamedTemporaryFile(suffix='.bin', delete=False) as tmp:
            tmp.write(b"\xff\xfe invalid utf-8 \x80\x81")
            path = tmp.name
        try:
            f = io_rust.open(path, "r", encoding="utf-8", errors="replace")
            content = f.read()
            f.close()
            assert "\ufffd" in content  # replacement character
        finally:
            os.unlink(path)

    def test_open_newline(self, io_rust):
        with tempfile.NamedTemporaryFile(suffix='.txt', delete=False, mode='wb') as tmp:
            tmp.write(b"a\r\nb\r\n")
            path = tmp.name
        try:
            # newline='' disables universal newline translation
            f = io_rust.open(path, "r", newline="")
            content = f.read()
            f.close()
            assert "\r\n" in content
        finally:
            os.unlink(path)

    def test_open_closefd(self, io_rust):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as tmp:
            tmp.write("fd test")
            path = tmp.name
        try:
            # open via fd with closefd=False: fd stays open after file.close()
            fd = os.open(path, os.O_RDONLY)
            f = io_rust.open(fd, "r", closefd=False)
            assert f.read() == "fd test"
            f.close()
            # fd still valid since closefd=False
            os.close(fd)
        finally:
            os.unlink(path)

    def test_open_opener(self, io_rust):
        with tempfile.TemporaryDirectory() as tmpdir:
            path = os.path.join(tmpdir, "target.txt")
            with builtins_open(path, "w") as f:
                f.write("opener test")
            dir_fd = os.open(tmpdir, os.O_RDONLY)
            try:

                def my_opener(p, flags):
                    return os.open(p, flags, dir_fd=dir_fd)

                f = io_rust.open("target.txt", "r", opener=my_opener)
                assert f.read() == "opener test"
                f.close()
            finally:
                os.close(dir_fd)

    def test_open_nonexistent(self, io_rust):
        with pytest.raises(FileNotFoundError):
            io_rust.open("/tmp/catnip_nonexistent_file_test.txt")
