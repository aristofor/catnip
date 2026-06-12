// FILE: catnip_rs/src/cache/disk.rs
//! Disk-based cache backend with TTL and size limits.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CacheEntry, CacheKey, CacheType};
use crate::config::get_cache_dir;

/// Minimum staleness before a read rewrites the stored access time. Avoids a
/// write on every cache hit while keeping LRU recency roughly accurate.
const ACCESS_BUMP_SECS: u64 = 60;

/// Check if cache debug mode is enabled via CATNIP_CACHE_DEBUG env var.
fn is_debug_enabled() -> bool {
    std::env::var("CATNIP_CACHE_DEBUG").is_ok()
}

/// Current Unix time in seconds (0 if the clock is before the epoch).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Filesystem creation time (fallback modified) in Unix seconds, for legacy
/// entries written before timestamps were embedded in the pickle.
fn fs_time_secs(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let t = meta.created().or_else(|_| meta.modified()).ok()?;
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
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
            return Ok(self.record_miss(&key_str, "MISS"));
        }

        // Load defensively: a truncated/corrupt/incompatible file is treated as
        // a miss (evict + None), never a hard error propagated to the caller.
        let entry_dict = match self.load_entry_dict(py, &file_path) {
            Ok(d) => d,
            Err(_) => {
                let _ = fs::remove_file(&file_path);
                return Ok(self.record_miss(&key_str, "MISS (corrupt)"));
            }
        };

        let now = now_secs();

        // TTL from the timestamp stored inside the entry. Filesystem birth/mtime
        // is unavailable on some filesystems and mutable by unrelated tooling.
        if let Some(ttl) = self.ttl_seconds {
            let created = Self::dict_u64(&entry_dict, "created_at").or_else(|| fs_time_secs(&file_path));
            if let Some(created) = created {
                if now.saturating_sub(created) >= ttl {
                    let _ = fs::remove_file(&file_path);
                    return Ok(self.record_miss(&key_str, "MISS (expired)"));
                }
            }
        }

        // Reconstruct the entry; a missing/mistyped field is corruption -> miss.
        let entry = match Self::build_entry(&entry_dict) {
            Some(e) => e,
            None => {
                let _ = fs::remove_file(&file_path);
                return Ok(self.record_miss(&key_str, "MISS (corrupt)"));
            }
        };

        // LRU: refresh the stored access time, gated so repeated reads don't
        // rewrite the file every time. prune() sorts evictions by this value.
        let accessed = Self::dict_u64(&entry_dict, "accessed_at").unwrap_or(0);
        if now.saturating_sub(accessed) >= ACCESS_BUMP_SECS && entry_dict.set_item("accessed_at", now).is_ok() {
            let _ = self.rewrite_entry(py, &file_path, &entry_dict);
        }

        self.hits += 1;
        if is_debug_enabled() {
            eprintln!("[cache] HIT {}", key_str);
        }
        Ok(Some(Py::new(py, entry)?))
    }

    /// Store an entry in the cache.
    #[pyo3(signature = (key, value, metadata=None))]
    fn set(&mut self, py: Python<'_>, key: &CacheKey, value: Py<PyAny>, metadata: Option<Py<PyDict>>) -> PyResult<()> {
        let key_str = key.to_string(py)?;
        let file_path = self.get_file_path(&key_str);

        // Create entry as dict for pickling
        let metadata = metadata.unwrap_or_else(|| PyDict::new(py).into());
        let now = now_secs();
        let entry_dict = PyDict::new(py);
        entry_dict.set_item("key", key_str.clone())?;
        entry_dict.set_item("value", value)?;
        // Store cache_type as string for pickle compatibility
        entry_dict.set_item("cache_type", key.cache_type.__str__())?;
        entry_dict.set_item("metadata", metadata)?;
        // Embed timestamps so TTL/LRU don't depend on fragile filesystem times.
        entry_dict.set_item("created_at", now)?;
        entry_dict.set_item("accessed_at", now)?;

        // Serialize and write atomically (temp file + rename).
        self.rewrite_entry(py, &file_path, &entry_dict)?;

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

    /// Clear the entire cache (only files owned by this backend).
    fn clear(&mut self) -> PyResult<()> {
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && Self::is_owned_file(&path) {
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
        dict.set_item("volume_mb", format!("{:.2}", total_size as f64 / (1024.0 * 1024.0)))?;

        if let Some(max_size) = self.max_size_bytes {
            dict.set_item("max_size_mb", format!("{:.2}", max_size as f64 / (1024.0 * 1024.0)))?;
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
    fn prune(&mut self, py: Python<'_>) -> PyResult<u64> {
        let mut removed_count = 0u64;
        let now = now_secs();

        // Collect all entries with metadata
        let mut entries: Vec<(PathBuf, EntryMetadata)> = Vec::new();

        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();

            // Only prune files owned by DiskCache
            if !path.is_file() || !Self::is_owned_file(&path) {
                continue;
            }

            let size = fs::metadata(&path)?.len();

            // Read timestamps from the entry itself (consistent with get()).
            // A file that no longer deserializes is corrupt -> evict now.
            let (created_at, accessed_at) = match self.load_entry_dict(py, &path) {
                Ok(d) => (
                    Self::dict_u64(&d, "created_at")
                        .or_else(|| fs_time_secs(&path))
                        .unwrap_or(0),
                    Self::dict_u64(&d, "accessed_at").unwrap_or(0),
                ),
                Err(_) => {
                    let _ = fs::remove_file(&path);
                    removed_count += 1;
                    continue;
                }
            };

            entries.push((
                path.clone(),
                EntryMetadata {
                    size_bytes: size,
                    created_at,
                    accessed_at,
                },
            ));
        }

        // Remove expired entries (TTL). Same boundary as get(): age >= ttl.
        if let Some(ttl) = self.ttl_seconds {
            entries.retain(|(path, meta)| {
                let age = now.saturating_sub(meta.created_at);
                if age >= ttl {
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
    /// Record a miss (counter + optional debug log) and return `None`.
    fn record_miss(&mut self, key_str: &str, reason: &str) -> Option<Py<CacheEntry>> {
        self.misses += 1;
        if is_debug_enabled() {
            eprintln!("[cache] {} {}", reason, key_str);
        }
        None
    }

    /// Read and unpickle the entry dict. Errors (I/O, unpickling, wrong type)
    /// propagate so the caller can treat the file as corrupt and evict it.
    ///
    /// Note: entries are pickled, so the cache directory is a trust boundary —
    /// only this user should be able to write to it (XDG cache, user-owned).
    /// A writable-by-others cache dir would allow arbitrary code execution on
    /// load; the key signature guards correctness, not safety.
    fn load_entry_dict<'py>(&self, py: Python<'py>, file_path: &Path) -> PyResult<Bound<'py, PyDict>> {
        let data = fs::read(file_path)?;
        let pickle = py.import("pickle")?;
        let loads = pickle.getattr("loads")?;
        let py_bytes = PyBytes::new(py, &data);
        Ok(loads.call1((py_bytes,))?.extract()?)
    }

    /// Serialize an entry dict and write it atomically (temp file + rename).
    fn rewrite_entry(&self, py: Python<'_>, file_path: &Path, entry_dict: &Bound<'_, PyDict>) -> PyResult<()> {
        let pickle = py.import("pickle")?;
        let dumps = pickle.getattr("dumps")?;
        let pickled_bytes: Bound<'_, PyBytes> = dumps.call1((entry_dict,))?.extract()?;
        let tmp_path = file_path.with_extension("tmp");
        fs::write(&tmp_path, pickled_bytes.as_bytes())?;
        fs::rename(&tmp_path, file_path)?;
        Ok(())
    }

    /// Extract a `u64` field from an entry dict, or `None` if absent/mistyped.
    fn dict_u64(d: &Bound<'_, PyDict>, key: &str) -> Option<u64> {
        d.get_item(key).ok().flatten().and_then(|v| v.extract::<u64>().ok())
    }

    /// Reconstruct a `CacheEntry` from an entry dict; `None` if any required
    /// field is missing or has the wrong type (treated as corruption).
    fn build_entry(d: &Bound<'_, PyDict>) -> Option<CacheEntry> {
        let key: String = d.get_item("key").ok().flatten()?.extract().ok()?;
        let value: Py<PyAny> = d.get_item("value").ok().flatten()?.extract().ok()?;
        let cache_type_str: String = d.get_item("cache_type").ok().flatten()?.extract().ok()?;
        let cache_type = match cache_type_str.as_str() {
            "source" => CacheType::Source,
            "ast" => CacheType::Ast,
            "bytecode" => CacheType::Bytecode,
            "result" => CacheType::Result,
            _ => CacheType::Source,
        };
        let metadata: Py<PyDict> = d.get_item("metadata").ok().flatten()?.extract().ok()?;
        Some(CacheEntry {
            key,
            value,
            cache_type,
            metadata,
        })
    }

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
