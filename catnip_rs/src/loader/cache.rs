// FILE: catnip_rs/src/loader/cache.rs
use pyo3::prelude::*;
use std::collections::HashMap;

/// Module cache: bare names and resolved paths → loaded namespace.
#[derive(Default)]
pub struct ModuleCache {
    modules: HashMap<String, Py<PyAny>>,
}

impl ModuleCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a cached module by key.
    pub fn get(&self, key: &str) -> Option<&Py<PyAny>> {
        self.modules.get(key)
    }

    /// Insert a module into the cache.
    pub fn insert(&mut self, key: String, namespace: Py<PyAny>) {
        self.modules.insert(key, namespace);
    }

    /// Check if a key exists.
    pub fn contains(&self, key: &str) -> bool {
        self.modules.contains_key(key)
    }
}
