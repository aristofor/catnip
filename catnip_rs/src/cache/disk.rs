// FILE: catnip_rs/src/cache/disk.rs
//! Disk-based cache backend with TTL and size limits.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CacheEntry, CacheKey, CacheType};
use crate::config::get_cache_dir;

/// Check if cache debug mode is enabled via CATNIP_CACHE_DEBUG env var.
fn is_debug_enabled() -> bool {
    std::env::var("CATNIP_CACHE_DEBUG").is_ok()
}

/// Metadata for a cached entry on disk.
#[derive(Debug, Clone)]
struct EntryMetadata {
    size_bytes: u64,
    created_at: u64,
    accessed_at: u64,
}

/// Disk-based cache with TTL and size management.
#[pyclass(name = "DiskCache")]
pub struct DiskCache {
    directory: PathBuf,
    max_size_bytes: Option<u64>,
    ttl_seconds: Option<u64>,
    hits: u64,
    misses: u64,
}

#[pymethods]
impl DiskCache {
    #[new]
    #[pyo3(signature = (directory=None, max_size_bytes=None, ttl_seconds=None))]
    fn new(
        _py: Python<'_>,
        directory: Option<PathBuf>,
        max_size_bytes: Option<u64>,
        ttl_seconds: Option<u64>,
    ) -> PyResult<Self> {
        let directory = match directory {
            Some(d) => d,
            None => get_cache_dir(),
        };

        // Create directory if it doesn't exist
        if !directory.exists() {
            fs::create_dir_all(&directory)?;
        }

        Ok(Self {
            directory,
            max_size_bytes,
            ttl_seconds,
            hits: 0,
            misses: 0,
        })
    }

    /// Retrieve an entry from the cache.
    fn get(&mut self, py: Python<'_>, key: &CacheKey) -> PyResult<Option<Py<CacheEntry>>> {
        let key_str = key.to_string(py)?;
        let file_path = self.get_file_path(&key_str);

        if !file_path.exists() {
            self.misses += 1;
            if is_debug_enabled() {
                eprintln!("[cache] MISS {}", key_str);
            }
            return Ok(None);
        }

        // Check TTL
        if let Some(ttl) = self.ttl_seconds {
            let metadata = fs::metadata(&file_path)?;
            // Use created time, fallback to modified if not available
            let creation_time = metadata.created().or_else(|_| metadata.modified())?;
            let age = SystemTime::now()
                .duration_since(creation_time)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            if age >= ttl {
                // Expired, delete and return None
                let _ = fs::remove_file(&file_path);
                self.misses += 1;
                if is_debug_enabled() {
                    eprintln!("[cache] MISS (expired) {}", key_str);
                }
                return Ok(None);
            }
        }

        // Read and deserialize with pickle
        let data = fs::read(&file_path)?;
        let pickle = py.import("pickle")?;
        let loads = pickle.getattr("loads")?;
        let py_bytes = PyBytes::new(py, &data);
        let entry_dict: Bound<'_, PyDict> = loads.call1((py_bytes,))?.extract()?;

        // Reconstruct CacheEntry from dict
        let key: String = entry_dict.get_item("key")?.unwrap().extract()?;
        let value: Py<PyAny> = entry_dict.get_item("value")?.unwrap().extract()?;
        let cache_type_str: String = entry_dict.get_item("cache_type")?.unwrap().extract()?;
        let cache_type = match cache_type_str.as_str() {
            "source" => CacheType::Source,
            "ast" => CacheType::Ast,
            "bytecode" => CacheType::Bytecode,
            "result" => CacheType::Result,
            _ => CacheType::Source, // fallback
        };
        let metadata: Py<PyDict> = entry_dict.get_item("metadata")?.unwrap().extract()?;

        let entry = CacheEntry {
            key,
            value,
            cache_type,
            metadata,
        };

        // Update access time
        let _ = fs::File::open(&file_path);

        self.hits += 1;
        if is_debug_enabled() {
            eprintln!("[cache] HIT {}", key_str);
        }
        Ok(Some(Py::new(py, entry)?))
    }

    /// Store an entry in the cache.
    #[pyo3(signature = (key, value, metadata=None))]
    fn set(
        &mut self,
        py: Python<'_>,
        key: &CacheKey,
        value: Py<PyAny>,
        metadata: Option<Py<PyDict>>,
    ) -> PyResult<()> {
        let key_str = key.to_string(py)?;
        let file_path = self.get_file_path(&key_str);

        // Create entry as dict for pickling
        let metadata = metadata.unwrap_or_else(|| PyDict::new(py).into());
        let entry_dict = PyDict::new(py);
        entry_dict.set_item("key", key_str.clone())?;
        entry_dict.set_item("value", value)?;
        // Store cache_type as string for pickle compatibility
        entry_dict.set_item("cache_type", key.cache_type.__str__())?;
        entry_dict.set_item("metadata", metadata)?;

        // Serialize with pickle
        let pickle = py.import("pickle")?;
        let dumps = pickle.getattr("dumps")?;
        let pickled_bytes: Bound<'_, PyBytes> = dumps.call1((entry_dict,))?.extract()?;

        // Atomic write: temp file + rename (safe for concurrent processes)
        let tmp_path = file_path.with_extension("tmp");
        fs::write(&tmp_path, pickled_bytes.as_bytes())?;
        fs::rename(&tmp_path, &file_path)?;

        // Check if we need to prune
        if self.max_size_bytes.is_some() {
            self.prune_if_needed(py)?;
        }

        Ok(())
    }

    /// Delete an entry from the cache.
    fn delete(&mut self, py: Python<'_>, key: &CacheKey) -> PyResult<bool> {
        let key_str = key.to_string(py)?;
        let file_path = self.get_file_path(&key_str);

        if file_path.exists() {
            fs::remove_file(&file_path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Clear the entire cache.
    fn clear(&mut self) -> PyResult<()> {
        // Remove all files in the cache directory
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                fs::remove_file(path)?;
            }
        }
        // Reset counters
        self.hits = 0;
        self.misses = 0;
        Ok(())
    }

    /// Check if a key exists in the cache.
    fn exists(&self, py: Python<'_>, key: &CacheKey) -> PyResult<bool> {
        let key_str = key.to_string(py)?;
        let file_path = self.get_file_path(&key_str);
        Ok(file_path.exists())
    }

    /// Return cache statistics.
    fn stats(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("backend", "disk")?;
        dict.set_item("directory", self.directory.to_string_lossy().to_string())?;

        // Count files and total size
        let (count, total_size) = self.get_cache_size()?;
        dict.set_item("size", count)?;
        dict.set_item("volume_bytes", total_size)?;
        dict.set_item(
            "volume_mb",
            format!("{:.2}", total_size as f64 / (1024.0 * 1024.0)),
        )?;

        if let Some(max_size) = self.max_size_bytes {
            dict.set_item(
                "max_size_mb",
                format!("{:.2}", max_size as f64 / (1024.0 * 1024.0)),
            )?;
        } else {
            dict.set_item("max_size_mb", py.None())?;
        }

        if let Some(ttl) = self.ttl_seconds {
            dict.set_item("ttl_seconds", ttl)?;
        } else {
            dict.set_item("ttl_seconds", py.None())?;
        }

        // Add hit/miss statistics
        dict.set_item("hits", self.hits)?;
        dict.set_item("misses", self.misses)?;

        // Calculate hit rate
        let total = self.hits + self.misses;
        let hit_rate = if total == 0 {
            "0.0%".to_string()
        } else {
            format!("{:.1}%", (self.hits as f64 / total as f64) * 100.0)
        };
        dict.set_item("hit_rate", hit_rate)?;

        Ok(dict.into())
    }

    /// Prune expired entries and enforce size limit.
    fn prune(&mut self, _py: Python<'_>) -> PyResult<u64> {
        let mut removed_count = 0u64;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Collect all entries with metadata
        let mut entries: Vec<(PathBuf, EntryMetadata)> = Vec::new();

        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();

            // Only prune files owned by DiskCache
            if !path.is_file() || !Self::is_owned_file(&path) {
                continue;
            }

            let metadata = fs::metadata(&path)?;
            let size = metadata.len();

            let created_at = metadata
                .created()
                .or_else(|_| metadata.modified())
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap().as_secs())
                .unwrap_or(0);

            let accessed_at = metadata
                .accessed()
                .or_else(|_| metadata.modified())
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap().as_secs())
                .unwrap_or(0);

            entries.push((
                path.clone(),
                EntryMetadata {
                    size_bytes: size,
                    created_at,
                    accessed_at,
                },
            ));
        }

        // Remove expired entries (TTL)
        if let Some(ttl) = self.ttl_seconds {
            entries.retain(|(path, meta)| {
                let age = now.saturating_sub(meta.created_at);
                if age > ttl {
                    let _ = fs::remove_file(path);
                    removed_count += 1;
                    false
                } else {
                    true
                }
            });
        }

        // Enforce size limit (remove oldest accessed first)
        if let Some(max_size) = self.max_size_bytes {
            let total_size: u64 = entries.iter().map(|(_, m)| m.size_bytes).sum();

            if total_size > max_size {
                // Sort by accessed_at (oldest first)
                entries.sort_by_key(|(_, m)| m.accessed_at);

                let mut current_size = total_size;
                for (path, meta) in entries.iter() {
                    if current_size <= max_size {
                        break;
                    }

                    let _ = fs::remove_file(path);
                    current_size -= meta.size_bytes;
                    removed_count += 1;
                }
            }
        }

        Ok(removed_count)
    }

    #[getter]
    fn directory(&self) -> String {
        self.directory.to_string_lossy().to_string()
    }

    #[getter]
    fn max_size_bytes(&self) -> Option<u64> {
        self.max_size_bytes
    }

    #[getter]
    fn ttl_seconds(&self) -> Option<u64> {
        self.ttl_seconds
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

/// Prefix for all DiskCache-owned files (from CacheKey: "catnip:type:hash" -> "catnip_type_hash").
const DISK_CACHE_PREFIX: &str = "catnip_";

impl DiskCache {
    /// Get the file path for a cache key.
    fn get_file_path(&self, key: &str) -> PathBuf {
        // Use the key directly as filename (it's already a hash)
        // Replace colons with underscores for filesystem compatibility
        let filename = key.replace(':', "_");
        self.directory.join(filename)
    }

    /// Check if a file belongs to the DiskCache (vs JIT or other subsystems).
    fn is_owned_file(path: &std::path::Path) -> bool {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(DISK_CACHE_PREFIX))
    }

    /// Get total cache size and file count (DiskCache-owned files only).
    fn get_cache_size(&self) -> PyResult<(u64, u64)> {
        let mut count = 0u64;
        let mut total_size = 0u64;

        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && Self::is_owned_file(&path) {
                count += 1;
                total_size += fs::metadata(&path)?.len();
            }
        }

        Ok((count, total_size))
    }

    /// Prune if needed (called after set).
    fn prune_if_needed(&mut self, py: Python<'_>) -> PyResult<()> {
        if let Some(max_size) = self.max_size_bytes {
            let (_, total_size) = self.get_cache_size()?;
            if total_size > max_size {
                self.prune(py)?;
            }
        }
        Ok(())
    }
}
