# FILE: tests/language/test_feature_imports.py
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import pytest

from catnip import Catnip
from catnip.context import Context
from catnip.exc import CatnipRuntimeError, CatnipTypeError


def _catnip_in(tmpdir):
    """Catnip instance with META.file pointing into tmpdir."""
    c = Catnip()
    c.context.globals['META'].file = str(Path(tmpdir) / '__test__.cat')
    return c


class TestModuleImports(unittest.TestCase):
    def test_import_builtin_module(self):
        """Import a Python stdlib module via import() builtin."""
        catnip = Catnip()
        catnip.parse('m = import("math")\nm.sqrt(81)')
        result = catnip.execute()
        self.assertEqual(result, 9)

    def test_import_python_file(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "host.py").write_text("def triple(x):\n    return x * 3\n")
            catnip = _catnip_in(tmpdir)
            catnip.parse('host = import("host")\nhost.triple(4)')
            result = catnip.execute()
            self.assertEqual(result, 12)

    def test_import_catnip_module(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "tools.cat").write_text('double = (x) => { x * 2 }\n_hidden = 99')
            catnip = _catnip_in(tmpdir)
            catnip.parse('tools = import("tools")\ntools.double(5)')
            result = catnip.execute()
            self.assertEqual(result, 10)
            namespace = catnip.context.globals['tools']
            self.assertTrue(hasattr(namespace, 'double'))
            self.assertFalse(hasattr(namespace, '_hidden'))


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

    def test_protocol_prefix_bypasses_priority(self):
        """import("utils", protocol="py") loads the .py even when utils.cat exists."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('answer = () => { 42 }')
            (Path(tmpdir) / "utils.py").write_text('def answer(): return 99')
            catnip = _catnip_in(tmpdir)
            catnip.parse('u = import("utils", protocol="py")\nu.answer()')
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
        meta = catnip.context.globals['META']
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

        ctx = Context(globals={'x': 1})
        self.assertEqual(ctx.globals['x'], 1)
        self.assertIsInstance(ctx.globals['META'], CatnipMeta)

    def test_meta_setattr(self):
        """Code can set attributes on META."""
        catnip = Catnip()
        catnip.parse("META.x = 42\nMETA.x")
        result = catnip.execute()
        self.assertEqual(result, 42)

    def test_meta_exports_priority(self):
        """META.exports takes priority over __all__ and heuristic."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mod.cat").write_text(
                'public = 1\nalso_public = 2\nsecret = 3\n' 'META.exports = list("public", "also_public")'
            )
            catnip = _catnip_in(tmpdir)
            catnip.parse('mod = import("mod")')
            catnip.execute()
            ns = catnip.context.globals['mod']
            self.assertTrue(hasattr(ns, 'public'))
            self.assertTrue(hasattr(ns, 'also_public'))
            self.assertFalse(hasattr(ns, 'secret'))

    def test_meta_exports_over_all(self):
        """META.exports wins when both META.exports and __all__ are set."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mod.cat").write_text(
                'a = 1\nb = 2\nc = 3\n' '__all__ = list("a", "b", "c")\n' 'META.exports = list("a")'
            )
            catnip = _catnip_in(tmpdir)
            catnip.parse('mod = import("mod")')
            catnip.execute()
            ns = catnip.context.globals['mod']
            self.assertTrue(hasattr(ns, 'a'))
            self.assertFalse(hasattr(ns, 'b'))
            self.assertFalse(hasattr(ns, 'c'))

    def test_meta_exports_tuple(self):
        """META.exports accepts a tuple."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mod.cat").write_text('a = 1\nb = 2\nsecret = 3\n' 'META.exports = tuple("a", "b")')
            catnip = _catnip_in(tmpdir)
            catnip.parse('mod = import("mod")')
            catnip.execute()
            ns = catnip.context.globals['mod']
            self.assertTrue(hasattr(ns, 'a'))
            self.assertTrue(hasattr(ns, 'b'))
            self.assertFalse(hasattr(ns, 'secret'))

    def test_meta_exports_set(self):
        """META.exports accepts a set."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mod.cat").write_text('x = 10\ny = 20\nprivate = 30\n' 'META.exports = set("x", "y")')
            catnip = _catnip_in(tmpdir)
            catnip.parse('mod = import("mod")')
            catnip.execute()
            ns = catnip.context.globals['mod']
            self.assertTrue(hasattr(ns, 'x'))
            self.assertTrue(hasattr(ns, 'y'))
            self.assertFalse(hasattr(ns, 'private'))

    def test_meta_exports_type_error(self):
        """Invalid META.exports type fails fast instead of silently falling back."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mod.cat").write_text('a = 1\nMETA.exports = "a"')
            catnip = _catnip_in(tmpdir)
            catnip.parse('mod = import("mod")')
            with self.assertRaises(CatnipTypeError):
                catnip.execute()

    def test_all_fallback(self):
        """__all__ still works when META.exports is not set."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mod.cat").write_text('a = 1\nb = 2\n__all__ = list("a")')
            catnip = _catnip_in(tmpdir)
            catnip.parse('mod = import("mod")')
            catnip.execute()
            ns = catnip.context.globals['mod']
            self.assertTrue(hasattr(ns, 'a'))
            self.assertFalse(hasattr(ns, 'b'))

    def test_heuristic_excludes_meta(self):
        """Heuristic export (no META.exports, no __all__) excludes META itself."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mod.cat").write_text("val = 42")
            catnip = _catnip_in(tmpdir)
            catnip.parse('mod = import("mod")')
            catnip.execute()
            ns = catnip.context.globals['mod']
            self.assertTrue(hasattr(ns, 'val'))
            self.assertFalse(hasattr(ns, 'META'))

    def test_meta_path_in_module(self):
        """META.file is set to the module file path before execution."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mod.cat").write_text("my_path = META.file")
            catnip = _catnip_in(tmpdir)
            catnip.parse('mod = import("mod")')
            catnip.execute()
            ns = catnip.context.globals['mod']
            self.assertEqual(ns.my_path, str((Path(tmpdir) / "mod.cat").resolve()))


class TestSelectiveImports(unittest.TestCase):
    """Tests for selective import with optional aliases."""

    def test_selective_single_name(self):
        catnip = Catnip()
        catnip.parse('import("math", "sqrt")\nsqrt(144)')
        result = catnip.execute()
        self.assertEqual(result, 12.0)

    def test_selective_multiple_names(self):
        catnip = Catnip()
        catnip.parse('import("math", "sqrt", "pi")\nsqrt(pi)')
        result = catnip.execute()
        self.assertAlmostEqual(result, 1.7724538509055159)

    def test_selective_with_alias(self):
        catnip = Catnip()
        catnip.parse('import("math", "pi:p")\np')
        result = catnip.execute()
        self.assertAlmostEqual(result, 3.141592653589793)

    def test_selective_mixed(self):
        catnip = Catnip()
        catnip.parse('import("math", "sqrt", "pi:p")\nsqrt(4) + p')
        result = catnip.execute()
        self.assertAlmostEqual(result, 2.0 + 3.141592653589793)

    def test_selective_returns_none(self):
        catnip = Catnip()
        catnip.parse('r = import("math", "sqrt")\nr')
        result = catnip.execute()
        self.assertIsNone(result)

    def test_selective_name_not_found(self):
        catnip = Catnip()
        catnip.parse('import("math", "nonexistent")')
        with self.assertRaises(AttributeError):
            catnip.execute()

    def test_selective_and_wild_error(self):
        catnip = Catnip()
        catnip.parse('import("math", "sqrt", wild=True)')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_selective_empty_name(self):
        catnip = Catnip()
        catnip.parse('import("math", "")')
        with self.assertRaises(ValueError):
            catnip.execute()

    def test_selective_empty_alias(self):
        catnip = Catnip()
        catnip.parse('import("math", "sqrt:")')
        with self.assertRaises(ValueError):
            catnip.execute()

    def test_selective_catnip_module(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "tools.cat").write_text("double = (x) => { x * 2 }")
            catnip = _catnip_in(tmpdir)
            catnip.parse('import("tools", "double")\ndouble(7)')
            result = catnip.execute()
            self.assertEqual(result, 14)

    def test_selective_alias_catnip_module(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "tools.cat").write_text("double = (x) => { x * 2 }")
            catnip = _catnip_in(tmpdir)
            catnip.parse('import("tools", "double:d")\nd(7)')
            result = catnip.execute()
            self.assertEqual(result, 14)

    def test_selective_multiple_all_aliased(self):
        catnip = Catnip()
        catnip.parse('import("math", "sqrt:s", "pi:p")\ns(4) + p')
        result = catnip.execute()
        self.assertAlmostEqual(result, 2.0 + 3.141592653589793)

    def test_selective_alias_does_not_leak_original(self):
        catnip = Catnip()
        catnip.parse('import("math", "pi:p")\np')
        catnip.execute()
        self.assertNotIn('pi', catnip.context.globals)
        self.assertIn('p', catnip.context.globals)

    def test_selective_non_string_name(self):
        catnip = Catnip()
        catnip.parse('import("math", 42)')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_wild_only_still_works(self):
        catnip = Catnip()
        catnip.parse('import("math", wild=True)\nsqrt(16)')
        result = catnip.execute()
        self.assertEqual(result, 4.0)


class TestNewResolution(unittest.TestCase):
    """Tests for protocol prefixes, dotted names, and migration errors."""

    def test_protocol_prefix_cat(self):
        """import("utils", protocol="cat") finds .cat only."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('val = 42')
            (Path(tmpdir) / "utils.py").write_text('val = 99')
            catnip = _catnip_in(tmpdir)
            catnip.parse('u = import("utils", protocol="cat")\nu.val')
            result = catnip.execute()
            self.assertEqual(result, 42)

    def test_protocol_prefix_py(self):
        """import("utils", protocol="py") finds .py only."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('val = 42')
            (Path(tmpdir) / "utils.py").write_text('val = 99')
            catnip = _catnip_in(tmpdir)
            catnip.parse('u = import("utils", protocol="py")\nu.val')
            result = catnip.execute()
            self.assertEqual(result, 99)

    def test_protocol_prefix_forces_py_over_cat(self):
        """When both exist, protocol="py" forces .py."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "helpers.cat").write_text('x = 1')
            (Path(tmpdir) / "helpers.py").write_text('x = 2')
            catnip = _catnip_in(tmpdir)
            catnip.parse('h = import("helpers", protocol="py")\nh.x')
            result = catnip.execute()
            self.assertEqual(result, 2)

    def test_dotted_name_resolution(self):
        """import("mylib.utils") resolves mylib/utils.cat."""
        with tempfile.TemporaryDirectory() as tmpdir:
            pkg = Path(tmpdir) / "mylib"
            pkg.mkdir()
            (pkg / "utils.cat").write_text('val = 77')
            catnip = _catnip_in(tmpdir)
            catnip.parse('u = import("mylib.utils")\nu.val')
            result = catnip.execute()
            self.assertEqual(result, 77)

    def test_dotted_name_deep(self):
        """import("a.b.c") resolves a/b/c.cat."""
        with tempfile.TemporaryDirectory() as tmpdir:
            deep = Path(tmpdir) / "a" / "b"
            deep.mkdir(parents=True)
            (deep / "c.cat").write_text('val = 123')
            catnip = _catnip_in(tmpdir)
            catnip.parse('m = import("a.b.c")\nm.val')
            result = catnip.execute()
            self.assertEqual(result, 123)

    def test_caller_dir_priority(self):
        """Module in caller_dir is found before CWD."""
        with tempfile.TemporaryDirectory() as caller, tempfile.TemporaryDirectory() as workdir:
            (Path(caller) / "prio.cat").write_text('val = "caller"')
            (Path(workdir) / "prio.cat").write_text('val = "cwd"')
            catnip = _catnip_in(caller)
            with patch("pathlib.Path.cwd", return_value=Path(workdir)):
                catnip.parse('m = import("prio")\nm.val')
                result = catnip.execute()
            self.assertEqual(result, "caller")

    def test_migration_error_relative(self):
        """import("./foo") raises CatnipRuntimeError with migration guidance."""
        catnip = Catnip()
        catnip.parse('import("./foo")')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_migration_error_absolute(self):
        """import("/tmp/foo.cat") raises CatnipRuntimeError."""
        catnip = Catnip()
        catnip.parse('import("/tmp/foo.cat")')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_migration_error_extension(self):
        """import("host.py") raises CatnipRuntimeError."""
        catnip = Catnip()
        catnip.parse('import("host.py")')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_importlib_fallback_dotted(self):
        """import("os.path") works via importlib fallback."""
        catnip = Catnip()
        catnip.parse('p = import("os.path")\np.join("a", "b")')
        result = catnip.execute()
        self.assertEqual(result, "a/b")

    def test_cache_normalized_key(self):
        """import("math", protocol="py") and import("math") share the same cache."""
        catnip = Catnip()
        catnip.parse('a = import("math", protocol="py")\nb = import("math")\na == b')
        result = catnip.execute()
        self.assertTrue(result)

    def test_runtime_import_cache_is_scoped_by_caller_dir(self):
        """The runtime ImportLoader must not reuse a module across different caller dirs."""
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            dir_a = root / "a"
            dir_b = root / "b"
            dir_a.mkdir()
            dir_b.mkdir()
            (dir_a / "helper.py").write_text('VALUE = "A"\n')
            (dir_b / "helper.py").write_text('VALUE = "B"\n')

            catnip = Catnip()
            meta = catnip.context.globals["META"]

            meta.file = str(dir_a / "main.cat")
            catnip._pipeline.inject_globals(catnip.context.globals)
            mod_a = catnip._fixed_import("helper")

            meta.file = str(dir_b / "main.cat")
            catnip._pipeline.inject_globals(catnip.context.globals)
            mod_b = catnip._fixed_import("helper")

            self.assertEqual(mod_a.VALUE, "A")
            self.assertEqual(mod_b.VALUE, "B")
            self.assertIsNot(mod_a, mod_b)


class TestPackages(unittest.TestCase):
    """Tests for lib.toml package resolution."""

    def test_package_with_lib_toml(self):
        """Directory with lib.toml is loaded as a package."""
        with tempfile.TemporaryDirectory() as tmpdir:
            pkg = Path(tmpdir) / "mylib"
            pkg.mkdir()
            (pkg / "lib.toml").write_text('[lib]\nname = "mylib"\nversion = "0.1.0"\n')
            (pkg / "main.cat").write_text('greet = () => { "hello" }')
            catnip = _catnip_in(tmpdir)
            catnip.parse('m = import("mylib")\nm.greet()')
            result = catnip.execute()
            self.assertEqual(result, "hello")

    def test_package_custom_entry(self):
        """Custom entry point in lib.toml."""
        with tempfile.TemporaryDirectory() as tmpdir:
            pkg = Path(tmpdir) / "mylib"
            pkg.mkdir()
            (pkg / "lib.toml").write_text('[lib]\nentry = "index.cat"\n')
            (pkg / "index.cat").write_text('val = 99')
            catnip = _catnip_in(tmpdir)
            catnip.parse('m = import("mylib")\nm.val')
            result = catnip.execute()
            self.assertEqual(result, 99)

    def test_package_exports_filter(self):
        """lib.exports.include filters exported names."""
        with tempfile.TemporaryDirectory() as tmpdir:
            pkg = Path(tmpdir) / "mylib"
            pkg.mkdir()
            (pkg / "lib.toml").write_text('[lib]\n[lib.exports]\ninclude = ["pub_fn"]\n')
            (pkg / "main.cat").write_text('pub_fn = () => { 1 }\nsecret = 2')
            catnip = _catnip_in(tmpdir)
            catnip.parse('m = import("mylib")')
            catnip.execute()
            ns = catnip.context.globals['m']
            self.assertTrue(hasattr(ns, 'pub_fn'))
            self.assertFalse(hasattr(ns, 'secret'))

    def test_package_priority_over_file(self):
        """mylib/lib.toml takes priority over mylib.cat."""
        with tempfile.TemporaryDirectory() as tmpdir:
            # Loose file
            (Path(tmpdir) / "mylib.cat").write_text('val = "file"')
            # Package dir
            pkg = Path(tmpdir) / "mylib"
            pkg.mkdir()
            (pkg / "lib.toml").write_text('[lib]\n')
            (pkg / "main.cat").write_text('val = "package"')
            catnip = _catnip_in(tmpdir)
            catnip.parse('m = import("mylib")\nm.val')
            result = catnip.execute()
            self.assertEqual(result, "package")

    def test_package_missing_entry(self):
        """Missing entry point raises clear error."""
        with tempfile.TemporaryDirectory() as tmpdir:
            pkg = Path(tmpdir) / "mylib"
            pkg.mkdir()
            (pkg / "lib.toml").write_text('[lib]\nentry = "missing.cat"\n')
            catnip = _catnip_in(tmpdir)
            catnip.parse('import("mylib")')
            with self.assertRaises((FileNotFoundError, CatnipRuntimeError)):
                catnip.execute()

    def test_dir_without_lib_toml_ignored(self):
        """Directory without lib.toml is not treated as a package."""
        with tempfile.TemporaryDirectory() as tmpdir:
            # Dir exists but no lib.toml
            (Path(tmpdir) / "mylib").mkdir()
            # Loose file should be found instead
            (Path(tmpdir) / "mylib.cat").write_text('val = "file"')
            catnip = _catnip_in(tmpdir)
            catnip.parse('m = import("mylib")\nm.val')
            result = catnip.execute()
            self.assertEqual(result, "file")

    def test_dotted_into_package_dir(self):
        """import("mylib.sub") finds mylib/sub.cat even when mylib is a package."""
        with tempfile.TemporaryDirectory() as tmpdir:
            pkg = Path(tmpdir) / "mylib"
            pkg.mkdir()
            (pkg / "lib.toml").write_text('[lib]\n')
            (pkg / "main.cat").write_text('val = "main"')
            (pkg / "sub.cat").write_text('val = "sub"')
            catnip = _catnip_in(tmpdir)
            catnip.parse('m = import("mylib.sub")\nm.val')
            result = catnip.execute()
            self.assertEqual(result, "sub")


class TestAutoImport(unittest.TestCase):
    """Tests for auto kwarg on Catnip()."""

    def test_auto_kwarg(self):
        """auto=['math'] makes math available without explicit import."""
        catnip = Catnip(auto=['math'])
        catnip.parse("math.sqrt(16)")
        result = catnip.execute()
        self.assertEqual(result, 4.0)

    def test_auto_empty(self):
        """auto=[] works without error."""
        catnip = Catnip(auto=[])
        catnip.parse("1 + 1")
        result = catnip.execute()
        self.assertEqual(result, 2)

    def test_auto_nonexistent(self):
        """auto with unknown module does not crash (warning + skip)."""
        catnip = Catnip(auto=["nonexistent_xyz_9999"])
        catnip.parse("1 + 1")
        result = catnip.execute()
        self.assertEqual(result, 2)


class TestRelativeImports(unittest.TestCase):
    """Tests for relative imports with leading dots."""

    def test_single_dot_same_dir(self):
        """import(".utils") loads utils.cat from caller's directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('double = (x) => { x * 2 }')
            catnip = _catnip_in(tmpdir)
            catnip.parse('u = import(".utils")\nu.double(5)')
            result = catnip.execute()
            self.assertEqual(result, 10)

    def test_double_dot_parent(self):
        """import("..utils") loads utils.cat from parent directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            sub = Path(tmpdir) / "sub"
            sub.mkdir()
            (Path(tmpdir) / "utils.cat").write_text('val = 42')
            catnip = _catnip_in(str(sub))
            catnip.parse('u = import("..utils")\nu.val')
            result = catnip.execute()
            self.assertEqual(result, 42)

    def test_triple_dot_grandparent(self):
        """import("...utils") loads from grandparent directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            deep = Path(tmpdir) / "a" / "b"
            deep.mkdir(parents=True)
            (Path(tmpdir) / "utils.cat").write_text('val = 99')
            catnip = _catnip_in(str(deep))
            catnip.parse('u = import("...utils")\nu.val')
            result = catnip.execute()
            self.assertEqual(result, 99)

    def test_relative_dotted_subpath(self):
        """import("..lib.utils") resolves to parent/lib/utils.cat."""
        with tempfile.TemporaryDirectory() as tmpdir:
            sub = Path(tmpdir) / "sub"
            sub.mkdir()
            lib = Path(tmpdir) / "lib"
            lib.mkdir()
            (lib / "utils.cat").write_text('val = 77')
            catnip = _catnip_in(str(sub))
            catnip.parse('u = import("..lib.utils")\nu.val')
            result = catnip.execute()
            self.assertEqual(result, 77)

    def test_relative_protocol_cat(self):
        """Relative import with protocol="cat" only finds .cat files."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('val = 1')
            (Path(tmpdir) / "utils.py").write_text('val = 2')
            catnip = _catnip_in(tmpdir)
            catnip.parse('u = import(".utils", protocol="cat")\nu.val')
            result = catnip.execute()
            self.assertEqual(result, 1)

    def test_relative_protocol_py(self):
        """Relative import with protocol="py" only finds .py files."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('val = 1')
            (Path(tmpdir) / "utils.py").write_text('val = 2')
            catnip = _catnip_in(tmpdir)
            catnip.parse('u = import(".utils", protocol="py")\nu.val')
            result = catnip.execute()
            self.assertEqual(result, 2)

    def test_relative_cache_same_caller(self):
        """Two relative imports from the same caller return the same object."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('val = 1')
            catnip = _catnip_in(tmpdir)
            catnip.parse('a = import(".utils")\nb = import(".utils")\na == b')
            result = catnip.execute()
            self.assertTrue(result)

    def test_relative_cache_different_callers(self):
        """Same relative spec from different callers resolving to different files."""
        with tempfile.TemporaryDirectory() as tmpdir:
            dir_a = Path(tmpdir) / "a"
            dir_b = Path(tmpdir) / "b"
            dir_a.mkdir()
            dir_b.mkdir()
            (dir_a / "utils.cat").write_text('val = 1')
            (dir_b / "utils.cat").write_text('val = 2')
            catnip_a = _catnip_in(str(dir_a))
            catnip_a.parse('u = import(".utils")\nu.val')
            result_a = catnip_a.execute()
            catnip_b = _catnip_in(str(dir_b))
            catnip_b.parse('u = import(".utils")\nu.val')
            result_b = catnip_b.execute()
            self.assertEqual(result_a, 1)
            self.assertEqual(result_b, 2)

    def test_relative_no_caller_dir(self):
        """Relative import without META.file raises CatnipRuntimeError."""
        catnip = Catnip()
        catnip.parse('import(".utils")')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_dot_only_error(self):
        """import(".") with no module name raises CatnipRuntimeError."""
        catnip = Catnip()
        catnip.parse('import(".")')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_dots_only_error(self):
        """import("..") with no module name raises CatnipRuntimeError."""
        catnip = Catnip()
        catnip.parse('import("..")')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_relative_not_found(self):
        """import(".nonexistent") raises an error."""
        with tempfile.TemporaryDirectory() as tmpdir:
            catnip = _catnip_in(tmpdir)
            catnip.parse('import(".nonexistent")')
            with self.assertRaises((FileNotFoundError, CatnipRuntimeError)):
                catnip.execute()

    def test_dot_slash_still_rejected(self):
        """import("./foo") still raises migration error."""
        catnip = Catnip()
        catnip.parse('import("./foo")')
        with self.assertRaises(CatnipRuntimeError):
            catnip.execute()

    def test_relative_selective(self):
        """Selective import from relative spec."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('double = (x) => { x * 2 }\ntriple = (x) => { x * 3 }')
            catnip = _catnip_in(tmpdir)
            catnip.parse('import(".utils", "double")\ndouble(7)')
            result = catnip.execute()
            self.assertEqual(result, 14)

    def test_relative_wild(self):
        """Wild import from relative spec injects into globals."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('double = (x) => { x * 2 }')
            catnip = _catnip_in(tmpdir)
            catnip.parse('import(".utils", wild=True)\ndouble(5)')
            result = catnip.execute()
            self.assertEqual(result, 10)

    def test_relative_package(self):
        """Relative import of a package with lib.toml."""
        with tempfile.TemporaryDirectory() as tmpdir:
            pkg = Path(tmpdir) / "mylib"
            pkg.mkdir()
            (pkg / "lib.toml").write_text('[lib]\n')
            (pkg / "main.cat").write_text('greet = () => { "hello" }')
            catnip = _catnip_in(tmpdir)
            catnip.parse('m = import(".mylib")\nm.greet()')
            result = catnip.execute()
            self.assertEqual(result, "hello")


class TestImportStatement(unittest.TestCase):
    """Tests for the import statement syntax: import('spec') with auto-binding."""

    def test_import_statement_basic(self):
        """import('math') binds math in scope."""
        catnip = Catnip()
        catnip.parse("import('math')\nmath.sqrt(81)")
        result = catnip.execute()
        self.assertEqual(result, 9)

    def test_import_statement_double_quotes(self):
        """import("math") with double quotes."""
        catnip = Catnip()
        catnip.parse('import("math")\nmath.pi')
        result = catnip.execute()
        self.assertAlmostEqual(result, 3.141592653589793)

    def test_import_statement_relative(self):
        """import('.utils') binds utils from caller dir."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "utils.cat").write_text('double = (x) => { x * 2 }')
            catnip = _catnip_in(tmpdir)
            catnip.parse("import('.utils')\nutils.double(5)")
            result = catnip.execute()
            self.assertEqual(result, 10)

    def test_import_statement_dotted_no_binding(self):
        """import('os.path') in statement form does not auto-bind (ambiguous name)."""
        catnip = Catnip()
        catnip.parse("import('os.path')")
        catnip.execute()
        # No auto-binding: neither 'os' nor 'path' should be in globals
        self.assertNotIn('os', catnip.context.globals)
        self.assertNotIn('path', catnip.context.globals)

    def test_import_expression_unchanged(self):
        """m = import('math') still works as expression."""
        catnip = Catnip()
        catnip.parse('m = import("math")\nm.sqrt(81)')
        result = catnip.execute()
        self.assertEqual(result, 9)

    def test_import_selective_unchanged(self):
        """import('math', 'sqrt') still works as function call."""
        catnip = Catnip()
        catnip.parse('import("math", "sqrt")\nsqrt(144)')
        result = catnip.execute()
        self.assertEqual(result, 12.0)

    def test_import_statement_catnip_module(self):
        """import('mod') binds a .cat module."""
        with tempfile.TemporaryDirectory() as tmpdir:
            (Path(tmpdir) / "mymod.cat").write_text('add1 = (x) => { x + 1 }')
            catnip = _catnip_in(tmpdir)
            catnip.parse("import('.mymod')\nmymod.add1(9)")
            result = catnip.execute()
            self.assertEqual(result, 10)


class TestImportStatementNoDesugar(unittest.TestCase):
    """Verify that import() with non-literal args does NOT auto-bind."""

    def test_dynamic_spec_no_binding(self):
        """import(spec("math")) should not auto-bind math."""
        catnip = Catnip()
        catnip.parse('spec = (x) => { x }; import(spec("math"))')
        catnip.execute()
        self.assertNotIn('math', catnip.context.globals)

    def test_concat_spec_no_binding(self):
        """import("ma" + "th") should not auto-bind."""
        catnip = Catnip()
        catnip.parse('import("ma" + "th")')
        catnip.execute()
        self.assertNotIn('math', catnip.context.globals)

    def test_fstring_spec_no_binding(self):
        """import(f"math") should not auto-bind."""
        catnip = Catnip()
        catnip.parse('x = "math"; import(f"{x}")')
        catnip.execute()
        self.assertNotIn('math', catnip.context.globals)


if __name__ == "__main__":
    unittest.main()
