// FILE: catnip_rs/src/cache/mod.rs
//! Cache system for Catnip - Rust implementation.
//!
//! Provides:
//! - CacheType enum
//! - CacheKey with xxhash-based key generation
//! - CacheEntry with metadata
//! - MemoryCache backend with hit/miss statistics
//! - DiskCache backend with TTL and size management

mod backend;
mod disk;
mod memoization;

use crate::constants::*;
use indexmap::IndexMap;
use pyo3::prelude::*;
use pyo3::types::PyDict;

pub use backend::CatnipCache;
pub use disk::DiskCache;
pub use memoization::Memoization;

/// Type of content to cache.
#[pyclass(name = "CacheType", from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheType {
    #[pyo3(name = "SOURCE")]
    Source = 1,
    #[pyo3(name = "AST")]
    Ast = 2,
    #[pyo3(name = "BYTECODE")]
    Bytecode = 3,
    #[pyo3(name = "RESULT")]
    Result = 4,
}

#[pymethods]
impl CacheType {
    // Enum variants as class attributes
    #[classattr]
    const SOURCE: CacheType = CacheType::Source;
    #[classattr]
    const AST: CacheType = CacheType::Ast;
    #[classattr]
    const BYTECODE: CacheType = CacheType::Bytecode;
    #[classattr]
    const RESULT: CacheType = CacheType::Result;

    fn __repr__(&self) -> String {
        match self {
            CacheType::Source => "CacheType.SOURCE".to_string(),
            CacheType::Ast => "CacheType.AST".to_string(),
            CacheType::Bytecode => "CacheType.BYTECODE".to_string(),
            CacheType::Result => "CacheType.RESULT".to_string(),
        }
    }

    fn __str__(&self) -> &'static str {
        match self {
            CacheType::Source => "source",
            CacheType::Ast => "ast",
            CacheType::Bytecode => "bytecode",
            CacheType::Result => "result",
        }
    }

    fn __hash__(&self) -> u64 {
        *self as u64
    }

    fn __richcmp__(&self, other: &Self, op: pyo3::pyclass::CompareOp) -> bool {
        match op {
            pyo3::pyclass::CompareOp::Eq => self == other,
            pyo3::pyclass::CompareOp::Ne => self != other,
            _ => false,
        }
    }

    #[getter]
    fn value(&self) -> &'static str {
        self.__str__()
    }

    /// Make CacheType iterable for Python (for cache_type in CacheType:)
    fn __iter__(_slf: PyRef<'_, Self>) -> CacheTypeIterator {
        CacheTypeIterator { index: 0 }
    }
}

/// Iterator for CacheType enum
#[pyclass]
struct CacheTypeIterator {
    index: usize,
}

#[pymethods]
impl CacheTypeIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<CacheType> {
        let result = match slf.index {
            0 => Some(CacheType::Source),
            1 => Some(CacheType::Ast),
            2 => Some(CacheType::Bytecode),
            3 => Some(CacheType::Result),
            _ => None,
        };
        slf.index += 1;
        result
    }
}

/// Cache key with fine-grained control.
#[pyclass(name = "CacheKey", from_py_object)]
#[derive(Clone)]
pub struct CacheKey {
    #[pyo3(get, set)]
    pub content: String,
    #[pyo3(get, set)]
    pub cache_type: CacheType,
    #[pyo3(get, set)]
    pub optimize: bool,
    #[pyo3(get, set)]
    pub tco_enabled: bool,
    #[pyo3(get, set)]
    pub module_policy_hash: String,
}

#[pymethods]
impl CacheKey {
    #[new]
    #[pyo3(signature = (content, cache_type, optimize=true, tco_enabled=true, module_policy_hash=String::new()))]
    fn new(
        content: String,
        cache_type: CacheType,
        optimize: bool,
        tco_enabled: bool,
        module_policy_hash: String,
    ) -> Self {
        Self {
            content,
            cache_type,
            optimize,
            tco_enabled,
            module_policy_hash,
        }
    }

    /// Generate a unique cache key with xxhash.
    fn to_string(&self, py: Python<'_>) -> PyResult<String> {
        // Import version info from Python
        let version_module = py.import(PY_MOD_VERSION)?;
        let lang_id: String = version_module.getattr("__lang_id__")?.extract()?;
        let version: String = version_module.getattr("__version__")?.extract()?;

        // Get build date
        let get_build_date = version_module.getattr("_get_build_date")?;
        let build_date_opt: Option<String> = get_build_date.call0()?.extract()?;
        let build_date = build_date_opt.unwrap_or_else(|| "0000-00-00-00:00:00".to_string());

        // Catnip signature for automatic cache invalidation
        let catnip_signature = format!("{}:{}:{}", lang_id, version, build_date);

        // Combine signature + content + options
        let combined = format!(
            "{}|{}|{}|{}|{}|{}",
            catnip_signature,
            self.content,
            self.cache_type.__str__(),
            self.optimize,
            self.tco_enabled,
            self.module_policy_hash,
        );

        // Compute xxhash
        let utils_module = py.import(PY_MOD_UTILS)?;
        let compute_signature = utils_module.getattr("compute_signature")?;
        let hash_value: String = compute_signature.call1((combined,))?.extract()?;

        Ok(format!("catnip:{}:{}", self.cache_type.__str__(), hash_value))
    }

    fn __repr__(&self) -> String {
        format!(
            "CacheKey(content={:?}, cache_type={:?}, optimize={}, tco_enabled={})",
            &self.content[..self.content.len().min(30)],
            self.cache_type,
            self.optimize,
            self.tco_enabled
        )
    }
}

/// Cache entry with metadata.
#[pyclass(name = "CacheEntry")]
pub struct CacheEntry {
    #[pyo3(get, set)]
    pub key: String,
    #[pyo3(get, set)]
    pub value: Py<PyAny>,
    #[pyo3(get, set)]
    pub cache_type: CacheType,
    #[pyo3(get, set)]
    pub metadata: Py<PyDict>,
}

#[pymethods]
impl CacheEntry {
    #[new]
    #[pyo3(signature = (key, value, cache_type, metadata=None))]
    fn new(py: Python<'_>, key: String, value: Py<PyAny>, cache_type: CacheType, metadata: Option<Py<PyDict>>) -> Self {
        let metadata = metadata.unwrap_or_else(|| PyDict::new(py).into());
        Self {
            key,
            value,
            cache_type,
            metadata,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "CacheEntry(key={:?}, cache_type={:?})",
            &self.key[..self.key.len().min(30)],
            self.cache_type
        )
    }
}

/// In-memory cache with hit/miss statistics.
#[pyclass(name = "MemoryCache")]
pub struct MemoryCache {
    cache: IndexMap<String, CacheEntry>,
    max_size: Option<usize>,
    hits: u64,
    misses: u64,
}

#[pymethods]
impl MemoryCache {
    #[new]
    #[pyo3(signature = (max_size=None))]
    fn new(max_size: Option<usize>) -> Self {
        Self {
            cache: IndexMap::new(),
            max_size,
            hits: 0,
            misses: 0,
        }
    }

    /// Retrieve an entry from the cache.
    fn get(&mut self, py: Python<'_>, key: &CacheKey) -> PyResult<Option<Py<CacheEntry>>> {
        let key_str = key.to_string(py)?;

        if let Some(entry) = self.cache.get(&key_str) {
            self.hits += 1;
            // Clone entry for Python
            let new_entry = CacheEntry {
                key: entry.key.clone(),
                value: entry.value.clone_ref(py),
                cache_type: entry.cache_type,
                metadata: entry.metadata.clone_ref(py),
            };
            Ok(Some(Py::new(py, new_entry)?))
        } else {
            self.misses += 1;
            Ok(None)
        }
    }

    /// Store an entry in the cache.
    #[pyo3(signature = (key, value, metadata=None))]
    fn set(&mut self, py: Python<'_>, key: &CacheKey, value: Py<PyAny>, metadata: Option<Py<PyDict>>) -> PyResult<()> {
        let key_str = key.to_string(py)?;

        // If max_size is defined, check limit
        if let Some(max) = self.max_size {
            if self.cache.len() >= max && !self.cache.contains_key(&key_str) {
                // FIFO eviction - remove first key and shift remaining
                if let Some(first_key) = self.cache.keys().next().cloned() {
                    self.cache.shift_remove(&first_key);
                }
            }
        }

        let metadata = metadata.unwrap_or_else(|| PyDict::new(py).into());
        let entry = CacheEntry {
            key: key_str.clone(),
            value,
            cache_type: key.cache_type,
            metadata,
        };
        self.cache.insert(key_str, entry);
        Ok(())
    }

    /// Delete an entry from the cache.
    fn delete(&mut self, py: Python<'_>, key: &CacheKey) -> PyResult<bool> {
        let key_str = key.to_string(py)?;
        Ok(self.cache.shift_remove(&key_str).is_some())
    }

    /// Clear the entire cache.
    fn clear(&mut self) {
        self.cache.clear();
        self.hits = 0;
        self.misses = 0;
    }

    /// Check if a key exists in the cache.
    fn exists(&self, py: Python<'_>, key: &CacheKey) -> PyResult<bool> {
        let key_str = key.to_string(py)?;
        Ok(self.cache.contains_key(&key_str))
    }

    /// Return cache statistics.
    fn stats(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("backend", "memory")?;
        dict.set_item("size", self.cache.len())?;
        dict.set_item("max_size", self.max_size)?;
        dict.set_item("hits", self.hits)?;
        dict.set_item("misses", self.misses)?;

        let total = self.hits + self.misses;
        let hit_rate = if total > 0 {
            (self.hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        dict.set_item("hit_rate", format!("{:.1}%", hit_rate))?;

        Ok(dict.into())
    }

    #[getter]
    fn max_size(&self) -> Option<usize> {
        self.max_size
    }

    #[getter]
    fn hits(&self) -> u64 {
        self.hits
    }

    #[getter]
    fn misses(&self) -> u64 {
        self.misses
    }
}

/// Register cache module functions and classes.
pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<CacheType>()?;
    m.add_class::<CacheKey>()?;
    m.add_class::<CacheEntry>()?;
    m.add_class::<MemoryCache>()?;
    m.add_class::<DiskCache>()?;
    m.add_class::<CatnipCache>()?;
    m.add_class::<Memoization>()?;
    Ok(())
}
