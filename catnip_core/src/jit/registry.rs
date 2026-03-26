// FILE: catnip_core/src/jit/registry.rs
//! Registry of pure functions available for inlining.

use super::function_info::JitFunctionInfo;
use std::collections::HashMap;
use std::sync::Arc;

/// Registry of pure functions available for inlining.
pub struct PureFunctionRegistry {
    /// Map func_id → JitFunctionInfo for pure functions
    functions: HashMap<String, Arc<JitFunctionInfo>>,
}

impl PureFunctionRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
        }
    }

    /// Register a pure function for inlining.
    /// Only functions marked as pure will be stored.
    pub fn register(&mut self, func_id: String, info: Arc<JitFunctionInfo>) {
        if info.is_pure {
            self.functions.insert(func_id, info);
        }
    }

    /// Get function info if it's inlineable (small enough).
    /// Returns None if function not found or too complex.
    pub fn get_inlineable(&self, func_id: &str, max_size: usize) -> Option<&JitFunctionInfo> {
        self.functions
            .get(func_id)
            .filter(|c| c.complexity <= max_size)
            .map(|c| &**c)
    }

    /// Check if a function is registered as pure.
    pub fn is_pure(&self, func_id: &str) -> bool {
        self.functions.contains_key(func_id)
    }

    pub fn len(&self) -> usize {
        self.functions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

impl Default for PureFunctionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jit::function_info::JitFunctionInfo;

    #[test]
    fn test_register_pure() {
        let mut registry = PureFunctionRegistry::new();

        let info = JitFunctionInfo {
            instructions: vec![],
            constants: vec![],
            names: vec![],
            nargs: 1,
            complexity: 10,
            is_pure: true,
        };

        registry.register("test_fn".to_string(), Arc::new(info));

        assert!(registry.is_pure("test_fn"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_register_impure() {
        let mut registry = PureFunctionRegistry::new();

        let info = JitFunctionInfo {
            instructions: vec![],
            constants: vec![],
            names: vec![],
            nargs: 0,
            complexity: 10,
            is_pure: false,
        };

        registry.register("impure_fn".to_string(), Arc::new(info));

        assert!(!registry.is_pure("impure_fn"));
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_get_inlineable_size_limit() {
        let mut registry = PureFunctionRegistry::new();

        let info = JitFunctionInfo {
            instructions: vec![],
            constants: vec![],
            names: vec![],
            nargs: 0,
            complexity: 30,
            is_pure: true,
        };

        registry.register("big_fn".to_string(), Arc::new(info));

        assert!(registry.get_inlineable("big_fn", 20).is_none());
        assert!(registry.get_inlineable("big_fn", 40).is_some());
    }

    #[test]
    fn test_get_inlineable_not_found() {
        let registry = PureFunctionRegistry::new();
        assert!(registry.get_inlineable("unknown", 20).is_none());
    }
}
