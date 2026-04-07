// FILE: catnip_rs/src/policy/mod.rs
//! Module policy system -- PyO3 wrapper around catnip_core::policy.
//!
//! Adds TOML file loading, Python API, and profile management.

use pyo3::prelude::*;
use std::fs;
use std::path::PathBuf;

pub use catnip_core::policy::{ModulePolicyCore, PolicyAction, matches_rule};

/// Module access policy with deny-wins semantics.
#[pyclass(name = "ModulePolicy", module = "catnip._rs", frozen, from_py_object)]
#[derive(Debug, Clone)]
pub struct ModulePolicy {
    pub inner: ModulePolicyCore,
}

impl ModulePolicy {
    /// Rust-side constructor for use from config parsing.
    pub fn create(default_action: &str, allow: Vec<String>, deny: Vec<String>) -> Result<Self, String> {
        Ok(Self {
            inner: ModulePolicyCore::create(default_action, allow, deny)?,
        })
    }

    pub fn _summary(&self) -> String {
        self.inner.summary()
    }

    /// Load a named policy profile from a TOML config file (Rust API).
    ///
    /// Profiles live under `[modules.policies.<name>]` in the config.
    pub fn load_profile(path: std::path::PathBuf, profile: &str) -> Result<Self, String> {
        let content = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let data: toml::Table =
            toml::from_str(&content).map_err(|e| format!("invalid TOML in {}: {}", path.display(), e))?;
        let policies =
            policies_table(&data).ok_or_else(|| format!("no [modules.policies] section in {}", path.display()))?;
        let section = policies.get(profile).and_then(|v| v.as_table()).ok_or_else(|| {
            let available: Vec<&String> = policies.keys().collect();
            format!("profile '{}' not found (available: {:?})", profile, available,)
        })?;
        parse_profile(section)
    }
}

#[pymethods]
impl ModulePolicy {
    /// Create a new module policy.
    ///
    /// Args:
    ///     default_action: "allow" or "deny" (fallback when no rule matches)
    ///     allow: list of allowed module patterns
    ///     deny: list of denied module patterns
    #[new]
    #[pyo3(signature = (default_action, allow=vec![], deny=vec![]))]
    fn new(default_action: &str, allow: Vec<String>, deny: Vec<String>) -> PyResult<Self> {
        Self::create(default_action, allow, deny).map_err(pyo3::exceptions::PyValueError::new_err)
    }

    /// Check if a module is allowed by this policy.
    fn check(&self, module_name: &str) -> bool {
        self.inner.check(module_name)
    }

    /// Stable hash for cache invalidation.
    #[getter]
    fn policy_hash(&self) -> &str {
        &self.inner.hash
    }

    /// Load a named policy profile from a dedicated policy TOML file.
    ///
    /// Each top-level table is a profile name:
    /// ```toml
    /// [sandbox]
    /// policy = "deny"
    /// allow = ["math", "json"]
    ///
    /// [admin]
    /// policy = "allow"
    /// deny = ["subprocess"]
    /// ```
    #[staticmethod]
    fn from_file(path: PathBuf, profile: &str) -> PyResult<Self> {
        let content = fs::read_to_string(&path)
            .map_err(|e| pyo3::exceptions::PyFileNotFoundError::new_err(format!("{}: {}", path.display(), e)))?;
        let data: toml::Table = toml::from_str(&content).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("invalid TOML in {}: {}", path.display(), e))
        })?;
        let section = data.get(profile).and_then(|v| v.as_table()).ok_or_else(|| {
            let available: Vec<&String> = data.keys().filter(|k| data[*k].is_table()).collect();
            pyo3::exceptions::PyKeyError::new_err(format!(
                "profile '{}' not found in {} (available: {:?})",
                profile,
                path.display(),
                available,
            ))
        })?;
        parse_profile(section).map_err(pyo3::exceptions::PyValueError::new_err)
    }

    /// List available profile names in a policy file (top-level tables).
    #[staticmethod]
    fn list_profiles(path: PathBuf) -> PyResult<Vec<String>> {
        let content = fs::read_to_string(&path)
            .map_err(|e| pyo3::exceptions::PyFileNotFoundError::new_err(format!("{}: {}", path.display(), e)))?;
        let data: toml::Table = toml::from_str(&content).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("invalid TOML in {}: {}", path.display(), e))
        })?;
        let mut profiles: Vec<String> = data
            .iter()
            .filter(|(_, v)| v.is_table())
            .map(|(k, _)| k.clone())
            .collect();
        profiles.sort();
        Ok(profiles)
    }

    fn __repr__(&self) -> String {
        format!("ModulePolicy({})", self.inner.summary())
    }

    fn summary(&self) -> String {
        self.inner.summary()
    }
}

/// Navigate to [modules.policies] subtable in a config TOML.
fn policies_table(data: &toml::Table) -> Option<&toml::Table> {
    data.get("modules")?.as_table()?.get("policies")?.as_table()
}

/// Parse a policy profile from a TOML table (reused by config and from_file).
pub fn parse_profile(table: &toml::Table) -> Result<ModulePolicy, String> {
    let default_action = table.get("policy").and_then(|v| v.as_str()).unwrap_or("allow");
    let allow: Vec<String> = table
        .get("allow")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let deny: Vec<String> = table
        .get("deny")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    ModulePolicy::create(default_action, allow, deny)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_profile() {
        let toml_str = r#"
            policy = "deny"
            allow = ["math", "json"]
            deny = ["os"]
        "#;
        let table: toml::Table = toml::from_str(toml_str).unwrap();
        let policy = parse_profile(&table).unwrap();
        assert!(policy.check("math"));
        assert!(policy.check("json"));
        assert!(!policy.check("os"));
        assert!(!policy.check("subprocess"));
    }

    #[test]
    fn test_parse_profile_default_allow() {
        let table: toml::Table = toml::from_str("").unwrap();
        let policy = parse_profile(&table).unwrap();
        assert!(policy.check("anything"));
    }

    #[test]
    fn test_parse_profile_ignores_extra_keys() {
        let toml_str = r#"
            policy = "deny"
            allow = ["math", "io"]
        "#;
        let table: toml::Table = toml::from_str(toml_str).unwrap();
        let policy = parse_profile(&table).unwrap();
        assert!(policy.check("math"));
        assert!(policy.check("io"));
        assert!(!policy.check("os"));
    }

    #[test]
    fn test_load_profile_nested_toml() {
        let dir = std::env::temp_dir().join("catnip_policy_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("catnip.toml");
        std::fs::write(
            &path,
            r#"
            [modules.policies.sandbox]
            policy = "deny"
            allow = ["math", "json"]

            [modules.policies.admin]
            policy = "allow"
            deny = ["subprocess"]
        "#,
        )
        .unwrap();

        let policy = ModulePolicy::load_profile(path.clone(), "sandbox").unwrap();
        assert!(policy.check("math"));
        assert!(!policy.check("os"));

        let policy = ModulePolicy::load_profile(path.clone(), "admin").unwrap();
        assert!(policy.check("os"));
        assert!(!policy.check("subprocess"));

        assert!(ModulePolicy::load_profile(path, "missing").is_err());
    }

    #[test]
    fn test_policies_table_missing() {
        let data: toml::Table = toml::from_str("[format]\nindent = 4").unwrap();
        assert!(policies_table(&data).is_none());
    }

    // Core logic tests (matches_rule, deny_wins, etc.) are in catnip_core::policy::tests
}
