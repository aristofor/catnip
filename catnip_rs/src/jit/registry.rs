// FILE: catnip_rs/src/jit/registry.rs
//! Registry of pure functions available for inlining.

use crate::vm::frame::CodeObject;
use std::collections::HashMap;
use std::sync::Arc;

/// Registry of pure functions available for inlining.
pub struct PureFunctionRegistry {
    /// Map func_id → CodeObject for pure functions
    functions: HashMap<String, Arc<CodeObject>>,
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
    pub fn register(&mut self, func_id: String, code: Arc<CodeObject>) {
        if code.is_pure {
            self.functions.insert(func_id, code);
        }
    }

    /// Get function CodeObject if it's inlineable (small enough).
    /// Returns None if function not found or too complex.
    pub fn get_inlineable(&self, func_id: &str, max_size: usize) -> Option<&CodeObject> {
        self.functions
            .get(func_id)
            .filter(|c| c.complexity <= max_size)
            .map(|c| &**c)
    }

    /// Check if a function is registered as pure.
    pub fn is_pure(&self, func_id: &str) -> bool {
        self.functions.contains_key(func_id)
    }

    /// Get total number of registered pure functions.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    /// Check if registry is empty.
    #[allow(dead_code)]
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

    #[test]
    fn test_register_pure() {
        let mut registry = PureFunctionRegistry::new();

        let mut code = CodeObject::new("test_fn");
        code.is_pure = true;
        code.complexity = 10;
        code.nargs = 1;

        registry.register("test_fn".to_string(), Arc::new(code));

        assert!(registry.is_pure("test_fn"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_register_impure() {
        let mut registry = PureFunctionRegistry::new();

        let mut code = CodeObject::new("impure_fn");
        code.is_pure = false;
        code.complexity = 10;

        registry.register("impure_fn".to_string(), Arc::new(code));

        assert!(!registry.is_pure("impure_fn"));
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_get_inlineable_size_limit() {
        let mut registry = PureFunctionRegistry::new();

        let mut code = CodeObject::new("big_fn");
        code.is_pure = true;
        code.complexity = 30;

        registry.register("big_fn".to_string(), Arc::new(code));

        assert!(registry.get_inlineable("big_fn", 20).is_none());
        assert!(registry.get_inlineable("big_fn", 40).is_some());
    }

    #[test]
    fn test_get_inlineable_not_found() {
        let registry = PureFunctionRegistry::new();
        assert!(registry.get_inlineable("unknown", 20).is_none());
    }
}
