// FILE: catnip_rs/src/policy/mod.rs
//! Module policy system - mask-based, namespace-aware, deny-wins.
//!
//! Evaluates module access in order:
//! 1. Check deny rules -> match -> block
//! 2. Check allow rules -> match -> allow
//! 3. Fallback -> default_action

use pyo3::prelude::*;
use std::fs;
use std::path::PathBuf;
use xxhash_rust::xxh64;

/// Default policy action when no rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    Allow,
    Deny,
}

/// Module access policy with deny-wins semantics.
#[pyclass(name = "ModulePolicy", module = "catnip._rs", frozen, from_py_object)]
#[derive(Debug, Clone)]
pub struct ModulePolicy {
    default_action: PolicyAction,
    allow_rules: Vec<String>,
    deny_rules: Vec<String>,
    hash: String,
}

/// Check if a module name matches a rule.
///
/// - `"os"` matches `os`, `os.path`, `os.path.join` (boundary at `.`)
/// - `"os.*"` matches `os.path` but NOT `os` itself
/// - `"oslo"` does NOT match `"os"`
fn matches_rule(module_name: &str, rule: &str) -> bool {
    if let Some(prefix) = rule.strip_suffix(".*") {
        // Wildcard: match sub-modules only, not the prefix itself
        module_name.starts_with(prefix) && module_name.as_bytes().get(prefix.len()) == Some(&b'.')
    } else {
        // Exact: match module itself + all sub-modules
        module_name == rule
            || (module_name.starts_with(rule)
                && module_name.as_bytes().get(rule.len()) == Some(&b'.'))
    }
}

fn compute_hash(default_action: PolicyAction, allow: &[String], deny: &[String]) -> String {
    let canonical = format!(
        "default:{}|allow:{}|deny:{}",
        match default_action {
            PolicyAction::Allow => "allow",
            PolicyAction::Deny => "deny",
        },
        allow.join(","),
        deny.join(","),
    );
    format!("{:016x}", xxh64::xxh64(canonical.as_bytes(), 0))
}

impl ModulePolicy {
    /// Rust-side constructor for use from config parsing.
    pub fn create(
        default_action: &str,
        mut allow: Vec<String>,
        mut deny: Vec<String>,
    ) -> Result<Self, String> {
        let action = match default_action {
            "allow" => PolicyAction::Allow,
            "deny" => PolicyAction::Deny,
            other => return Err(format!("invalid default_action: '{}'", other)),
        };
        allow.sort();
        deny.sort();
        let hash = compute_hash(action, &allow, &deny);
        Ok(Self {
            default_action: action,
            allow_rules: allow,
            deny_rules: deny,
            hash,
        })
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
    fn new(default_action: &str, mut allow: Vec<String>, mut deny: Vec<String>) -> PyResult<Self> {
        let action = match default_action {
            "allow" => PolicyAction::Allow,
            "deny" => PolicyAction::Deny,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "invalid default_action: '{}' (expected 'allow' or 'deny')",
                    other
                )));
            }
        };
        allow.sort();
        deny.sort();
        let hash = compute_hash(action, &allow, &deny);
        Ok(Self {
            default_action: action,
            allow_rules: allow,
            deny_rules: deny,
            hash,
        })
    }

    /// Check if a module is allowed by this policy.
    fn check(&self, module_name: &str) -> bool {
        // Deny-wins: check deny first
        for rule in &self.deny_rules {
            if matches_rule(module_name, rule) {
                return false;
            }
        }
        // Then check allow
        for rule in &self.allow_rules {
            if matches_rule(module_name, rule) {
                return true;
            }
        }
        // Fallback
        self.default_action == PolicyAction::Allow
    }

    /// Stable hash for cache invalidation.
    #[getter]
    fn policy_hash(&self) -> &str {
        &self.hash
    }

    /// Load a named policy profile from a TOML file.
    ///
    /// File format: each top-level table is a profile name.
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
        let content = fs::read_to_string(&path).map_err(|e| {
            pyo3::exceptions::PyFileNotFoundError::new_err(format!("{}: {}", path.display(), e))
        })?;
        let data: toml::Table = toml::from_str(&content).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "invalid TOML in {}: {}",
                path.display(),
                e
            ))
        })?;
        let section = data
            .get(profile)
            .and_then(|v| v.as_table())
            .ok_or_else(|| {
                let available: Vec<&String> = data.keys().filter(|k| data[*k].is_table()).collect();
                pyo3::exceptions::PyKeyError::new_err(format!(
                    "profile '{}' not found in {} (available: {:?})",
                    profile,
                    path.display(),
                    available,
                ))
            })?;
        parse_profile(section).map_err(|e| pyo3::exceptions::PyValueError::new_err(e))
    }

    /// List available profile names in a policy file.
    #[staticmethod]
    fn list_profiles(path: PathBuf) -> PyResult<Vec<String>> {
        let content = fs::read_to_string(&path).map_err(|e| {
            pyo3::exceptions::PyFileNotFoundError::new_err(format!("{}: {}", path.display(), e))
        })?;
        let data: toml::Table = toml::from_str(&content).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "invalid TOML in {}: {}",
                path.display(),
                e
            ))
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
        format!(
            "ModulePolicy(default={}, allow={:?}, deny={:?})",
            match self.default_action {
                PolicyAction::Allow => "allow",
                PolicyAction::Deny => "deny",
            },
            self.allow_rules,
            self.deny_rules,
        )
    }
}

/// Parse a policy profile from a TOML table (reused by config and from_file).
pub fn parse_profile(table: &toml::Table) -> Result<ModulePolicy, String> {
    let default_action = table
        .get("policy")
        .and_then(|v| v.as_str())
        .unwrap_or("allow");
    let allow: Vec<String> = table
        .get("allow")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let deny: Vec<String> = table
        .get("deny")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    ModulePolicy::create(default_action, allow, deny)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_exact() {
        assert!(matches_rule("os", "os"));
        assert!(matches_rule("os.path", "os"));
        assert!(matches_rule("os.path.join", "os"));
        assert!(!matches_rule("oslo", "os"));
        assert!(!matches_rule("osm", "os"));
    }

    #[test]
    fn test_matches_wildcard() {
        assert!(matches_rule("os.path", "os.*"));
        assert!(matches_rule("os.path.join", "os.*"));
        assert!(!matches_rule("os", "os.*"));
        assert!(!matches_rule("oslo.utils", "os.*"));
    }

    #[test]
    fn test_matches_deep() {
        assert!(matches_rule("numpy.linalg.solve", "numpy.*"));
        assert!(matches_rule("numpy.linalg", "numpy.*"));
        assert!(!matches_rule("numpy", "numpy.*"));
    }

    #[test]
    fn test_deny_wins() {
        let policy = ModulePolicy {
            default_action: PolicyAction::Allow,
            allow_rules: vec!["os".to_string()],
            deny_rules: vec!["os".to_string()],
            hash: String::new(),
        };
        assert!(!policy.check("os"));
    }

    #[test]
    fn test_default_deny() {
        let policy = ModulePolicy {
            default_action: PolicyAction::Deny,
            allow_rules: vec![],
            deny_rules: vec![],
            hash: String::new(),
        };
        assert!(!policy.check("anything"));
    }

    #[test]
    fn test_default_allow() {
        let policy = ModulePolicy {
            default_action: PolicyAction::Allow,
            allow_rules: vec![],
            deny_rules: vec![],
            hash: String::new(),
        };
        assert!(policy.check("anything"));
    }

    #[test]
    fn test_allow_specific() {
        let policy = ModulePolicy {
            default_action: PolicyAction::Deny,
            allow_rules: vec!["math".to_string(), "json".to_string()],
            deny_rules: vec![],
            hash: String::new(),
        };
        assert!(policy.check("math"));
        assert!(policy.check("json"));
        assert!(!policy.check("os"));
    }

    #[test]
    fn test_deny_specific() {
        let policy = ModulePolicy {
            default_action: PolicyAction::Allow,
            allow_rules: vec![],
            deny_rules: vec!["os".to_string(), "subprocess".to_string()],
            hash: String::new(),
        };
        assert!(!policy.check("os"));
        assert!(!policy.check("os.path"));
        assert!(!policy.check("subprocess"));
        assert!(policy.check("math"));
    }

    #[test]
    fn test_hash_stable() {
        let h1 = compute_hash(
            PolicyAction::Deny,
            &["math".to_string()],
            &["os".to_string()],
        );
        let h2 = compute_hash(
            PolicyAction::Deny,
            &["math".to_string()],
            &["os".to_string()],
        );
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

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
    fn test_hash_changes() {
        let h1 = compute_hash(
            PolicyAction::Deny,
            &["math".to_string()],
            &["os".to_string()],
        );
        let h2 = compute_hash(
            PolicyAction::Allow,
            &["math".to_string()],
            &["os".to_string()],
        );
        assert_ne!(h1, h2);
    }
}
