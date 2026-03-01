// FILE: catnip_rs/src/cache/memoization.rs
//! Memoization system for function execution results.

use super::{CacheKey, CacheType, MemoryCache};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::collections::HashMap;

/// Memoization system for function execution results.
///
/// Stores function execution results based on their arguments.
/// Uses MemoryCache backend for storage with function name indexing.
#[pyclass(name = "Memoization")]
pub struct Memoization {
    backend: Py<MemoryCache>,
    enabled: bool,
    // Index: func_name -> list of (content, hash) pairs
    func_index: HashMap<String, Vec<(String, String)>>,
}

#[pymethods]
impl Memoization {
    #[new]
    #[pyo3(signature = (backend=None))]
    fn new(py: Python<'_>, backend: Option<Py<MemoryCache>>) -> PyResult<Self> {
        let backend = backend.unwrap_or_else(|| {
            // Create default MemoryCache
            Py::new(py, MemoryCache::new(None)).unwrap()
        });

        Ok(Self {
            backend,
            enabled: true,
            func_index: HashMap::new(),
        })
    }

    /// Retrieve result from cache.
    fn get(
        &mut self,
        py: Python<'_>,
        func_name: String,
        args: Py<PyAny>,
        kwargs: Py<PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        if !self.enabled {
            return Ok(None);
        }

        let key = self.make_key(py, &func_name, &args, &kwargs)?;
        let mut backend = self.backend.borrow_mut(py);

        match backend.get(py, &key)? {
            Some(entry) => {
                let entry_ref = entry.borrow(py);
                Ok(Some(entry_ref.value.clone_ref(py)))
            }
            None => Ok(None),
        }
    }

    /// Store result in the cache.
    fn set(
        &mut self,
        py: Python<'_>,
        func_name: String,
        args: Py<PyAny>,
        kwargs: Py<PyAny>,
        result: Py<PyAny>,
    ) -> PyResult<()> {
        if !self.enabled {
            return Ok(());
        }

        let key = self.make_key(py, &func_name, &args, &kwargs)?;
        let key_string = key.to_string(py)?;

        // Create metadata dict
        let metadata = PyDict::new(py);
        metadata.set_item("func_name", &func_name)?;

        // Get args length (try as tuple)
        let args_len = if let Ok(tuple) = args.cast_bound::<PyTuple>(py) {
            tuple.len()
        } else {
            0
        };
        metadata.set_item("args_count", args_len)?;

        // Get kwargs keys (try as dict)
        let kwargs_keys: Vec<String> = if let Ok(dict) = kwargs.cast_bound::<PyDict>(py) {
            dict.keys()
                .iter()
                .filter_map(|k| k.extract::<String>().ok())
                .collect()
        } else {
            Vec::new()
        };
        metadata.set_item("kwargs_keys", kwargs_keys)?;

        // Store in backend
        let mut backend = self.backend.borrow_mut(py);
        backend.set(py, &key, result, Some(metadata.into()))?;

        // Update func_name index (store both content and hash)
        self.func_index
            .entry(func_name.clone())
            .or_insert_with(Vec::new)
            .push((key.content.clone(), key_string));

        Ok(())
    }

    /// Invalidate cache entries.
    ///
    /// If func_name is None, invalidates entire cache.
    /// Returns number of entries invalidated.
    #[pyo3(signature = (func_name=None))]
    fn invalidate(&mut self, py: Python<'_>, func_name: Option<String>) -> PyResult<usize> {
        match func_name {
            None => {
                // Invalidate entire cache
                let mut backend = self.backend.borrow_mut(py);
                let stats = backend.stats(py)?;
                let stats_dict = stats.bind(py);
                let size: usize = stats_dict.get_item("size")?.unwrap().extract()?;
                backend.clear();
                self.func_index.clear();
                Ok(size)
            }
            Some(name) => {
                // Invalidate only this function using index
                if let Some(keys) = self.func_index.get(&name) {
                    let mut count = 0;
                    let mut backend = self.backend.borrow_mut(py);

                    for (content, _hash) in keys {
                        // Reconstruct CacheKey with original content
                        let key = CacheKey::new(
                            content.clone(),
                            CacheType::Result,
                            false,
                            false,
                            String::new(),
                        );
                        if backend.delete(py, &key)? {
                            count += 1;
                        }
                    }

                    // Remove from index
                    self.func_index.remove(&name);
                    Ok(count)
                } else {
                    Ok(0)
                }
            }
        }
    }

    /// Invalidate a specific cache entry.
    fn invalidate_key(
        &mut self,
        py: Python<'_>,
        func_name: String,
        args: Py<PyAny>,
        kwargs: Py<PyAny>,
    ) -> PyResult<bool> {
        let key = self.make_key(py, &func_name, &args, &kwargs)?;
        let key_string = key.to_string(py)?;

        let mut backend = self.backend.borrow_mut(py);
        let deleted = backend.delete(py, &key)?;

        // Update index
        if deleted {
            if let Some(keys) = self.func_index.get_mut(&func_name) {
                keys.retain(|(_content, hash)| hash != &key_string);
                // Clean up empty lists
                if keys.is_empty() {
                    self.func_index.remove(&func_name);
                }
            }
        }

        Ok(deleted)
    }

    /// Enable the cache.
    fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable the cache.
    fn disable(&mut self) {
        self.enabled = false;
    }

    /// Return cache statistics.
    fn stats(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let backend = self.backend.borrow(py);
        let base_stats = backend.stats(py)?;
        let stats_dict = base_stats.bind(py);
        stats_dict.set_item("enabled", self.enabled)?;
        Ok(base_stats)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let backend_name = self
            .backend
            .bind(py)
            .get_type()
            .name()
            .map(|n| n.to_string())
            .unwrap_or_else(|_| "Unknown".to_string());
        format!(
            "Memoization(backend={}, enabled={})",
            backend_name, self.enabled
        )
    }
}

impl Memoization {
    /// Create a cache key from function name and arguments.
    fn make_key(
        &self,
        py: Python<'_>,
        func_name: &str,
        args: &Py<PyAny>,
        kwargs: &Py<PyAny>,
    ) -> PyResult<CacheKey> {
        // Serialize arguments to string for hashing
        let args_str = if let Ok(tuple) = args.cast_bound::<PyTuple>(py) {
            let reprs: Vec<String> = tuple
                .iter()
                .map(|arg| arg.repr().unwrap().to_string())
                .collect();
            reprs.join(",")
        } else {
            String::new()
        };

        let kwargs_str = if let Ok(dict) = kwargs.cast_bound::<PyDict>(py) {
            let mut items: Vec<(String, String)> = dict
                .iter()
                .map(|(k, v)| {
                    let key_str: String = k.extract().unwrap();
                    let val_repr = v.repr().unwrap().to_string();
                    (key_str, val_repr)
                })
                .collect();
            items.sort_by(|a, b| a.0.cmp(&b.0));
            items
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(",")
        } else {
            String::new()
        };

        // Combine function + args in content
        let sep = if !args_str.is_empty() && !kwargs_str.is_empty() {
            ","
        } else {
            ""
        };
        let content = format!("{}({}{}{})", func_name, args_str, sep, kwargs_str);

        Ok(CacheKey::new(
            content,
            CacheType::Result,
            false,
            false,
            String::new(),
        ))
    }
}
