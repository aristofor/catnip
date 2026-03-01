// FILE: catnip_rs/src/cache/backend.rs
//! CatnipCache adapter for compilation caching.
//!
//! Provides a high-level adapter that wraps any Python cache backend
//! (MemoryCache, DiskCache, or custom implementations).
//!
//! Custom backends must implement the CacheBackend protocol (see catnip/cachesys/base.py).

use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::{CacheEntry, CacheKey, CacheType, MemoryCache};

/// Smart cache adapter for Catnip compilation.
///
/// Provides fine-grained control over what is cached (source, AST, bytecode)
/// and automatically handles invalidation based on compilation options.
#[pyclass(name = "CatnipCache")]
pub struct CatnipCache {
    backend: Py<PyAny>,
    cache_source: bool,
    cache_ast: bool,
    cache_bytecode: bool,
}

#[pymethods]
impl CatnipCache {
    #[new]
    #[pyo3(signature = (backend=None, cache_source=true, cache_ast=true, cache_bytecode=true))]
    fn new(
        py: Python<'_>,
        backend: Option<Py<PyAny>>,
        cache_source: bool,
        cache_ast: bool,
        cache_bytecode: bool,
    ) -> PyResult<Self> {
        let backend = match backend {
            Some(b) => b,
            None => Py::new(py, MemoryCache::new(None))?.into_any(),
        };

        Ok(Self {
            backend,
            cache_source,
            cache_ast,
            cache_bytecode,
        })
    }

    /// Retrieve AST from cache.
    #[pyo3(signature = (source, optimize=true, tco_enabled=true))]
    fn get_parsed(
        &self,
        py: Python<'_>,
        source: String,
        optimize: bool,
        tco_enabled: bool,
    ) -> PyResult<Option<Py<PyAny>>> {
        if !self.cache_ast {
            return Ok(None);
        }

        let key = Py::new(
            py,
            CacheKey::new(source, CacheType::Ast, optimize, tco_enabled, String::new()),
        )?;
        let backend = self.backend.bind(py);
        let result: Option<Py<CacheEntry>> = backend.call_method1("get", (key,))?.extract()?;

        Ok(result.map(|entry| {
            let entry_ref = entry.bind(py);
            entry_ref.getattr("value").unwrap().into()
        }))
    }

    /// Store AST in cache.
    #[pyo3(signature = (source, ast, optimize=true, tco_enabled=true, metadata=None))]
    fn set_parsed(
        &self,
        py: Python<'_>,
        source: String,
        ast: Py<PyAny>,
        optimize: bool,
        tco_enabled: bool,
        metadata: Option<Py<PyDict>>,
    ) -> PyResult<()> {
        if !self.cache_ast {
            return Ok(());
        }

        let key = Py::new(
            py,
            CacheKey::new(source, CacheType::Ast, optimize, tco_enabled, String::new()),
        )?;
        let backend = self.backend.bind(py);
        backend.call_method1("set", (key, ast, metadata))?;
        Ok(())
    }

    /// Retrieve bytecode from cache.
    #[pyo3(signature = (source, optimize=true, tco_enabled=true))]
    fn get_bytecode(
        &self,
        py: Python<'_>,
        source: String,
        optimize: bool,
        tco_enabled: bool,
    ) -> PyResult<Option<Py<PyAny>>> {
        if !self.cache_bytecode {
            return Ok(None);
        }

        let key = Py::new(
            py,
            CacheKey::new(
                source,
                CacheType::Bytecode,
                optimize,
                tco_enabled,
                String::new(),
            ),
        )?;
        let backend = self.backend.bind(py);
        let result: Option<Py<CacheEntry>> = backend.call_method1("get", (key,))?.extract()?;

        Ok(result.map(|entry| {
            let entry_ref = entry.bind(py);
            entry_ref.getattr("value").unwrap().into()
        }))
    }

    /// Store bytecode in cache.
    #[pyo3(signature = (source, bytecode, optimize=true, tco_enabled=true, metadata=None))]
    fn set_bytecode(
        &self,
        py: Python<'_>,
        source: String,
        bytecode: Py<PyAny>,
        optimize: bool,
        tco_enabled: bool,
        metadata: Option<Py<PyDict>>,
    ) -> PyResult<()> {
        if !self.cache_bytecode {
            return Ok(());
        }

        let key = Py::new(
            py,
            CacheKey::new(
                source,
                CacheType::Bytecode,
                optimize,
                tco_enabled,
                String::new(),
            ),
        )?;
        let backend = self.backend.bind(py);
        backend.call_method1("set", (key, bytecode, metadata))?;
        Ok(())
    }

    /// Invalidate all cache entries for a given source.
    fn invalidate_all(&self, py: Python<'_>, source: String) -> PyResult<()> {
        let cache_types = vec![
            CacheType::Source,
            CacheType::Ast,
            CacheType::Bytecode,
            CacheType::Result,
        ];

        for cache_type in cache_types {
            for optimize in [true, false] {
                for tco_enabled in [true, false] {
                    let key = Py::new(
                        py,
                        CacheKey::new(
                            source.clone(),
                            cache_type,
                            optimize,
                            tco_enabled,
                            String::new(),
                        ),
                    )?;
                    let backend = self.backend.bind(py);
                    backend.call_method1("delete", (key,))?;
                }
            }
        }

        Ok(())
    }

    /// Clear the entire cache.
    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        let backend = self.backend.bind(py);
        backend.call_method0("clear")?;
        Ok(())
    }

    /// Return cache statistics.
    fn stats(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let backend = self.backend.bind(py);
        let base_stats: Py<PyDict> = backend.call_method0("stats")?.extract()?;

        let config = PyDict::new(py);
        config.set_item("cache_source", self.cache_source)?;
        config.set_item("cache_ast", self.cache_ast)?;
        config.set_item("cache_bytecode", self.cache_bytecode)?;

        base_stats.bind(py).set_item("cache_config", config)?;
        Ok(base_stats)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let backend = self.backend.bind(py);
        let backend_class = backend.get_type().qualname()?;
        Ok(format!(
            "CatnipCache(backend={}, source={}, ast={}, bytecode={})",
            backend_class, self.cache_source, self.cache_ast, self.cache_bytecode
        ))
    }
}
