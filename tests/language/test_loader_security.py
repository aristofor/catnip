# FILE: tests/language/test_loader_security.py
import math as _real_math
import sys
import uuid

import pytest

from catnip.context import Context
from catnip.loader import ModuleLoader


def test_local_shadows_importlib(tmp_path, monkeypatch):
    (tmp_path / "math.py").write_text("def sqrt(x):\n    return 999\n")
    monkeypatch.chdir(tmp_path)
    # Restore real math after test to prevent cross-contamination
    monkeypatch.setitem(sys.modules, "math", _real_math)

    loader = ModuleLoader(Context())
    ns = loader.import_module("math")

    # Local file shadows Python stdlib.
    assert ns.sqrt(4) == 999


def test_protocol_py_bypasses_local_shadow(tmp_path, monkeypatch):
    (tmp_path / "math.py").write_text("def sqrt(x):\n    return 999\n")
    monkeypatch.chdir(tmp_path)
    # Ensure importlib finds the real math in sys.modules
    monkeypatch.setitem(sys.modules, "math", _real_math)

    loader = ModuleLoader(Context())
    ns = loader.import_module("math", protocol="py")

    # protocol='py' reaches the real Python module.
    assert ns.sqrt(4) == 2.0


def test_package_entry_cannot_escape_package_dir(tmp_path, monkeypatch):
    pkg_dir = tmp_path / "pkg"
    pkg_dir.mkdir()
    (pkg_dir / "lib.toml").write_text("[lib]\nentry = \"../outside.cat\"\n")
    (tmp_path / "outside.cat").write_text("x = 1\n")
    monkeypatch.chdir(tmp_path)

    loader = ModuleLoader(Context())
    with pytest.raises(ValueError, match="escapes package directory"):
        loader.import_module("pkg")


def test_failed_import_does_not_leave_partial_module_in_sys_modules(tmp_path, monkeypatch):
    module_name = f"badmod_{uuid.uuid4().hex}"
    (tmp_path / f"{module_name}.py").write_text("x = 1\nraise RuntimeError('boom')\n")
    monkeypatch.chdir(tmp_path)

    loader = ModuleLoader(Context())
    with pytest.raises(RuntimeError, match="boom"):
        loader.import_module(module_name)

    assert module_name not in sys.modules
