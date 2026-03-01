# FILE: tests/language/test_feature_imports.py
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from catnip import Catnip
from catnip.context import Context
from catnip.exc import CatnipTypeError


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


class TestMeta(unittest.TestCase):
    """Tests for META injection and META.exports in modules."""

    def test_meta_exists_in_globals(self):
        """META is a CatnipMeta available in every context."""
        from catnip._rs import CatnipMeta

        catnip = Catnip()
        meta = catnip.context.globals["META"]
        self.assertIsInstance(meta, CatnipMeta)

    def test_meta_accessible_from_code(self):
        """Code can read META as a global."""
        from catnip._rs import CatnipMeta

        catnip = Catnip()
        catnip.parse("META")
        result = catnip.execute()
        self.assertIsInstance(result, CatnipMeta)

    def test_meta_exists_with_custom_globals(self):
        """META is injected even when Context receives custom globals."""
        from catnip._rs import CatnipMeta

        ctx = Context(globals={"x": 1})
        self.assertEqual(ctx.globals["x"], 1)
        self.assertIsInstance(ctx.globals["META"], CatnipMeta)

    def test_meta_setattr(self):
        """Code can set attributes on META."""
        catnip = Catnip()
        catnip.parse("META.x = 42\nMETA.x")
        result = catnip.execute()
        self.assertEqual(result, 42)

    def test_meta_exports_priority(self):
        """META.exports takes priority over __all__ and heuristic."""
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "mod.cat"
            module_path.write_text(
                "\n".join(
                    [
                        "public = 1",
                        "also_public = 2",
                        "secret = 3",
                        'META.exports = list("public", "also_public")',
                    ]
                )
            )

            script = f'mod = import("{module_path.as_posix()}")'
            catnip = Catnip()
            catnip.parse(script)
            catnip.execute()

            ns = catnip.context.globals["mod"]
            self.assertTrue(hasattr(ns, "public"))
            self.assertTrue(hasattr(ns, "also_public"))
            self.assertFalse(hasattr(ns, "secret"))

    def test_meta_exports_over_all(self):
        """META.exports wins when both META.exports and __all__ are set."""
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "mod.cat"
            module_path.write_text(
                "\n".join(
                    [
                        "a = 1",
                        "b = 2",
                        "c = 3",
                        '__all__ = list("a", "b", "c")',
                        'META.exports = list("a")',
                    ]
                )
            )

            script = f'mod = import("{module_path.as_posix()}")'
            catnip = Catnip()
            catnip.parse(script)
            catnip.execute()

            ns = catnip.context.globals["mod"]
            self.assertTrue(hasattr(ns, "a"))
            self.assertFalse(hasattr(ns, "b"))
            self.assertFalse(hasattr(ns, "c"))

    def test_meta_exports_tuple(self):
        """META.exports accepts a tuple."""
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "mod.cat"
            module_path.write_text(
                "\n".join(
                    [
                        "a = 1",
                        "b = 2",
                        "secret = 3",
                        'META.exports = tuple("a", "b")',
                    ]
                )
            )

            script = f'mod = import("{module_path.as_posix()}")'
            catnip = Catnip()
            catnip.parse(script)
            catnip.execute()

            ns = catnip.context.globals["mod"]
            self.assertTrue(hasattr(ns, "a"))
            self.assertTrue(hasattr(ns, "b"))
            self.assertFalse(hasattr(ns, "secret"))

    def test_meta_exports_set(self):
        """META.exports accepts a set."""
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "mod.cat"
            module_path.write_text(
                "\n".join(
                    [
                        "x = 10",
                        "y = 20",
                        "private = 30",
                        'META.exports = set("x", "y")',
                    ]
                )
            )

            script = f'mod = import("{module_path.as_posix()}")'
            catnip = Catnip()
            catnip.parse(script)
            catnip.execute()

            ns = catnip.context.globals["mod"]
            self.assertTrue(hasattr(ns, "x"))
            self.assertTrue(hasattr(ns, "y"))
            self.assertFalse(hasattr(ns, "private"))

    def test_meta_exports_type_error(self):
        """Invalid META.exports type fails fast instead of silently falling back."""
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "mod.cat"
            module_path.write_text(
                "\n".join(
                    [
                        "a = 1",
                        'META.exports = "a"',
                    ]
                )
            )

            script = f'mod = import("{module_path.as_posix()}")'
            catnip = Catnip()
            catnip.parse(script)

            with self.assertRaises(CatnipTypeError):
                catnip.execute()

    def test_all_fallback(self):
        """__all__ still works when META.exports is not set."""
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "mod.cat"
            module_path.write_text(
                "\n".join(
                    [
                        "a = 1",
                        "b = 2",
                        '__all__ = list("a")',
                    ]
                )
            )

            script = f'mod = import("{module_path.as_posix()}")'
            catnip = Catnip()
            catnip.parse(script)
            catnip.execute()

            ns = catnip.context.globals["mod"]
            self.assertTrue(hasattr(ns, "a"))
            self.assertFalse(hasattr(ns, "b"))

    def test_heuristic_excludes_meta(self):
        """Heuristic export (no META.exports, no __all__) excludes META itself."""
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "mod.cat"
            module_path.write_text("val = 42")

            script = f'mod = import("{module_path.as_posix()}")'
            catnip = Catnip()
            catnip.parse(script)
            catnip.execute()

            ns = catnip.context.globals["mod"]
            self.assertTrue(hasattr(ns, "val"))
            self.assertFalse(hasattr(ns, "META"))

    def test_meta_path_in_module(self):
        """META.path is set to the module file path before execution."""
        with tempfile.TemporaryDirectory() as tmpdir:
            module_path = Path(tmpdir) / "mod.cat"
            module_path.write_text("my_path = META.path")

            script = f'mod = import("{module_path.as_posix()}")'
            catnip = Catnip()
            catnip.parse(script)
            catnip.execute()

            ns = catnip.context.globals["mod"]
            self.assertEqual(ns.my_path, str(module_path.resolve()))


if __name__ == "__main__":
    unittest.main()
