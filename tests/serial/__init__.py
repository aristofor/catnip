# FILE: tests/serial/__init__.py
"""
Tests that must run serially (non-parallelizable).

These tests use @pytest.mark.xdist_group() to avoid conflicts
when running in parallel with pytest-xdist.

Examples:
- JIT tests executing native machine code via mmap
- Tests that require exclusive shared resources
"""
