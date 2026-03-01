# FILE: tests/language/test_module_policy.py
import pytest
import tempfile
from pathlib import Path

from catnip import Catnip
from catnip._rs import ModulePolicy
from catnip.exc import CatnipRuntimeError
from catnip.loader import ModuleLoader


class TestModulePolicyRust:
    """Test ModulePolicy Rust class from Python."""

    def test_deny_blocks(self):
        policy = ModulePolicy("allow", deny=["os", "subprocess"])
        assert not policy.check("os")
        assert not policy.check("os.path")
        assert not policy.check("subprocess")
        assert policy.check("math")

    def test_allow_specific(self):
        policy = ModulePolicy("deny", allow=["math", "json"])
        assert policy.check("math")
        assert policy.check("json")
        assert not policy.check("os")

    def test_wildcard(self):
        policy = ModulePolicy("deny", allow=["numpy.*"])
        assert not policy.check("numpy")
        assert policy.check("numpy.linalg")
        assert policy.check("numpy.linalg.solve")

    def test_deny_wins(self):
        policy = ModulePolicy("allow", allow=["os"], deny=["os"])
        assert not policy.check("os")

    def test_no_oslo_match(self):
        policy = ModulePolicy("allow", deny=["os"])
        assert not policy.check("os")
        assert policy.check("oslo")

    def test_hash_stable(self):
        p1 = ModulePolicy("deny", allow=["math"], deny=["os"])
        p2 = ModulePolicy("deny", allow=["math"], deny=["os"])
        assert p1.policy_hash == p2.policy_hash
        assert len(p1.policy_hash) == 16

    def test_hash_changes(self):
        p1 = ModulePolicy("deny", allow=["math"], deny=["os"])
        p2 = ModulePolicy("allow", allow=["math"], deny=["os"])
        assert p1.policy_hash != p2.policy_hash

    def test_invalid_action(self):
        with pytest.raises(ValueError, match="invalid default_action"):
            ModulePolicy("block")


POLICIES_TOML = """\
[sandbox]
policy = "deny"
allow = ["math", "json", "random"]

[admin]
policy = "allow"
deny = ["subprocess"]

[template]
policy = "deny"
allow = ["math", "json"]
deny = ["os", "sys", "importlib"]
"""


class TestModulePolicyFromFile:
    """Test loading policies from external TOML files."""

    def _write_policy_file(self, tmp_path, content=POLICIES_TOML):
        p = tmp_path / "policies.toml"
        p.write_text(content)
        return p

    def test_load_sandbox_profile(self, tmp_path):
        f = self._write_policy_file(tmp_path)
        policy = ModulePolicy.from_file(f, "sandbox")
        assert policy.check("math")
        assert policy.check("json")
        assert not policy.check("os")

    def test_load_admin_profile(self, tmp_path):
        f = self._write_policy_file(tmp_path)
        policy = ModulePolicy.from_file(f, "admin")
        assert policy.check("os")
        assert policy.check("math")
        assert not policy.check("subprocess")

    def test_load_template_profile(self, tmp_path):
        f = self._write_policy_file(tmp_path)
        policy = ModulePolicy.from_file(f, "template")
        assert policy.check("math")
        assert not policy.check("os")
        assert not policy.check("sys")
        assert not policy.check("importlib")

    def test_profile_not_found(self, tmp_path):
        f = self._write_policy_file(tmp_path)
        with pytest.raises(KeyError, match="nonexistent"):
            ModulePolicy.from_file(f, "nonexistent")

    def test_file_not_found(self):
        with pytest.raises(FileNotFoundError):
            ModulePolicy.from_file("/tmp/does_not_exist_policy.toml", "sandbox")

    def test_invalid_toml(self, tmp_path):
        f = tmp_path / "bad.toml"
        f.write_text("[[[broken")
        with pytest.raises(ValueError, match="invalid TOML"):
            ModulePolicy.from_file(f, "sandbox")

    def test_list_profiles(self, tmp_path):
        f = self._write_policy_file(tmp_path)
        profiles = ModulePolicy.list_profiles(f)
        assert profiles == ["admin", "sandbox", "template"]

    def test_from_file_integration(self, tmp_path):
        """Full round-trip: file -> policy -> loader gate."""
        f = self._write_policy_file(tmp_path)
        policy = ModulePolicy.from_file(f, "sandbox")
        cat = Catnip(module_policy=policy)
        loader = ModuleLoader(cat.context)
        ns = loader.import_module("math")
        assert hasattr(ns, "sqrt")
        with pytest.raises(CatnipRuntimeError, match="blocked by policy"):
            loader.import_module("os")

    def test_hash_differs_between_profiles(self, tmp_path):
        f = self._write_policy_file(tmp_path)
        p1 = ModulePolicy.from_file(f, "sandbox")
        p2 = ModulePolicy.from_file(f, "admin")
        assert p1.policy_hash != p2.policy_hash


class TestModulePolicyIntegration:
    """Test policy integration with Catnip loader."""

    def test_policy_blocks_import(self):
        cat = Catnip()
        cat.context.module_policy = ModulePolicy("allow", deny=["os"])
        loader = ModuleLoader(cat.context)
        with pytest.raises(CatnipRuntimeError, match="module 'os' blocked by policy"):
            loader.import_module("os")

    def test_policy_allows_import(self):
        cat = Catnip()
        cat.context.module_policy = ModulePolicy("deny", allow=["math"])
        loader = ModuleLoader(cat.context)
        ns = loader.import_module("math")
        assert hasattr(ns, "sqrt")

    def test_no_policy_allows_all(self):
        """Backward compatible: no policy means everything is allowed."""
        cat = Catnip()
        assert cat.context.module_policy is None
        loader = ModuleLoader(cat.context)
        ns = loader.import_module("math")
        assert hasattr(ns, "sqrt")

    def test_policy_via_kwarg(self):
        policy = ModulePolicy("allow", deny=["os"])
        cat = Catnip(module_policy=policy)
        assert cat.context.module_policy is not None
        assert not cat.context.module_policy.check("os")
        assert cat.context.module_policy.check("math")

    def test_policy_blocks_submodule(self):
        cat = Catnip()
        cat.context.module_policy = ModulePolicy("allow", deny=["os"])
        loader = ModuleLoader(cat.context)
        with pytest.raises(CatnipRuntimeError, match="blocked by policy"):
            loader.import_module("os.path")

    def test_cached_module_bypasses_policy(self):
        """Once loaded, cached modules bypass policy check."""
        cat = Catnip()
        loader = ModuleLoader(cat.context)
        # Load without policy
        ns = loader.import_module("math")
        # Set restrictive policy
        cat.context.module_policy = ModulePolicy("deny")
        # Cached hit should still work
        ns2 = loader.import_module("math")
        assert ns is ns2
