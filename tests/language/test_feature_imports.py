# FILE: tests/language/test_feature_imports.py
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from catnip import Catnip


class TestModuleImports(unittest.TestCase):
    def test_import_builtin_module(self):
        """Import a Python stdlib module via import() builtin."""
        catnip = Catnip()
        catnip.parse('m = import("math")\nm.sqrt(81)')
        result = catnip.execute()
        self.assertEqual(result, 9)

    def test_import_python_file(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "host.py"
            module_path.write_text(
                "\n".join(
                    [
                        "def triple(x):",
                        "    return x * 3",
                    ]
                )
            )

            script = f'host = import("{module_path.as_posix()}")\nhost.triple(4)'
            catnip = Catnip()
            catnip.parse(script)
            result = catnip.execute()

            self.assertEqual(result, 12)

    def test_import_catnip_module(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "tools.cat"
            module_path.write_text(
                "\n".join(
                    [
                        "double = (x) => { x * 2 }",
                        "_hidden = 99",
                    ]
                )
            )

            script = f'tools = import("{module_path.as_posix()}")\ntools.double(5)'
            catnip = Catnip()
            catnip.parse(script)
            result = catnip.execute()

            self.assertEqual(result, 10)
            namespace = catnip.context.globals["tools"]
            self.assertTrue(hasattr(namespace, "double"))
            self.assertFalse(hasattr(namespace, "_hidden"))


class TestModuleResolution(unittest.TestCase):
    """Tests for bare-name resolution priority and search path."""

    def test_bare_name_cat_over_py(self):
        """When both utils.cat and utils.py exist, bare import("utils") loads .cat."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('answer = () => { 42 }')
            (Path(tmpdir) / "utils.py").write_text('def answer(): return 99')
            with patch("pathlib.Path.cwd", return_value=Path(tmpdir)):
                catnip = Catnip()
                catnip.parse('u = import("utils")\nu.answer()')
                result = catnip.execute()
            self.assertEqual(result, 42)

    def test_bare_name_fallback_py(self):
        """When only utils.py exists, bare import("utils") loads the .py."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.py").write_text('def answer(): return 99')
            with patch("pathlib.Path.cwd", return_value=Path(tmpdir)):
                catnip = Catnip()
                catnip.parse('u = import("utils")\nu.answer()')
                result = catnip.execute()
            self.assertEqual(result, 99)

    def test_cache_hit(self):
        """Second import("math") returns the same cached object."""
        catnip = Catnip()
        catnip.parse('a = import("math")\nb = import("math")\na == b')
        result = catnip.execute()
        self.assertTrue(result)

    def test_explicit_path_bypasses_priority(self):
        """import("./utils.py") loads the .py even when utils.cat exists in CWD."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('answer = () => { 42 }')
            py_path = Path(tmpdir) / "utils.py"
            py_path.write_text('def answer(): return 99')
            script = f'u = import("{py_path.as_posix()}")\nu.answer()'
            catnip = Catnip()
            catnip.parse(script)
            result = catnip.execute()
            self.assertEqual(result, 99)

    def test_catnip_path_env(self):
        """Module in a CATNIP_PATH directory is resolved for bare names."""
        with tempfile.TemporaryDirectory() as libdir, tempfile.TemporaryDirectory() as workdir:
            (Path(libdir) / "mylib.cat").write_text('greet = () => { "hello" }')
            env = {**os.environ, "CATNIP_PATH": libdir}
            with patch("pathlib.Path.cwd", return_value=Path(workdir)), patch.dict(os.environ, env):
                catnip = Catnip()
                catnip.parse('m = import("mylib")\nm.greet()')
                result = catnip.execute()
            self.assertEqual(result, "hello")


if __name__ == "__main__":
    unittest.main()
