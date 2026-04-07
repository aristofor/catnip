// FILE: catnip_core/src/policy.rs
//! Module policy core -- mask-based, namespace-aware, deny-wins.
//!
//! Pure Rust policy logic shared between catnip_rs (#[pyclass] wrapper)
//! and catnip_vm (standalone mode).

use xxhash_rust::xxh64;

/// Default policy action when no rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    Allow,
    Deny,
}

/// Module access policy with deny-wins semantics.
#[derive(Debug, Clone)]
pub struct ModulePolicyCore {
    pub default_action: PolicyAction,
    pub allow_rules: Vec<String>,
    pub deny_rules: Vec<String>,
    pub hash: String,
}

/// Check if a module name matches a rule.
///
/// - `"os"` matches `os`, `os.path`, `os.path.join` (boundary at `.`)
/// - `"os.*"` matches `os.path` but NOT `os` itself
/// - `"oslo"` does NOT match `"os"`
pub fn matches_rule(module_name: &str, rule: &str) -> bool {
    if let Some(prefix) = rule.strip_suffix(".*") {
        // Wildcard: match sub-modules only, not the prefix itself
        module_name.starts_with(prefix) && module_name.as_bytes().get(prefix.len()) == Some(&b'.')
    } else {
        // Exact: match module itself + all sub-modules
        module_name == rule || (module_name.starts_with(rule) && module_name.as_bytes().get(rule.len()) == Some(&b'.'))
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

impl ModulePolicyCore {
    /// Create a new policy from action string and rule lists.
    pub fn create(default_action: &str, mut allow: Vec<String>, mut deny: Vec<String>) -> Result<Self, String> {
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

    /// Check if a module is allowed by this policy.
    pub fn check(&self, module_name: &str) -> bool {
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

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        let action = match self.default_action {
            PolicyAction::Allow => "allow",
            PolicyAction::Deny => "deny",
        };
        let mut parts = vec![format!("default={}", action)];
        if !self.allow_rules.is_empty() {
            parts.push(format!("allow={:?}", self.allow_rules));
        }
        if !self.deny_rules.is_empty() {
            parts.push(format!("deny={:?}", self.deny_rules));
        }
        parts.join(", ")
    }
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
        let policy = ModulePolicyCore {
            default_action: PolicyAction::Allow,
            allow_rules: vec!["os".to_string()],
            deny_rules: vec!["os".to_string()],
            hash: String::new(),
        };
        assert!(!policy.check("os"));
    }

    #[test]
    fn test_default_deny() {
        let policy = ModulePolicyCore {
            default_action: PolicyAction::Deny,
            allow_rules: vec![],
            deny_rules: vec![],
            hash: String::new(),
        };
        assert!(!policy.check("anything"));
    }

    #[test]
    fn test_default_allow() {
        let policy = ModulePolicyCore {
            default_action: PolicyAction::Allow,
            allow_rules: vec![],
            deny_rules: vec![],
            hash: String::new(),
        };
        assert!(policy.check("anything"));
    }

    #[test]
    fn test_allow_specific() {
        let policy = ModulePolicyCore {
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
        let policy = ModulePolicyCore {
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
        let h1 = compute_hash(PolicyAction::Deny, &["math".to_string()], &["os".to_string()]);
        let h2 = compute_hash(PolicyAction::Deny, &["math".to_string()], &["os".to_string()]);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn test_hash_changes() {
        let h1 = compute_hash(PolicyAction::Deny, &["math".to_string()], &["os".to_string()]);
        let h2 = compute_hash(PolicyAction::Allow, &["math".to_string()], &["os".to_string()]);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_create_and_check() {
        let policy = ModulePolicyCore::create("deny", vec!["math".into(), "io".into()], vec![]).unwrap();
        assert!(policy.check("math"));
        assert!(policy.check("io"));
        assert!(!policy.check("os"));
    }

    #[test]
    fn test_create_invalid_action() {
        assert!(ModulePolicyCore::create("block", vec![], vec![]).is_err());
    }
}
