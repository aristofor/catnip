// FILE: catnip_rs/src/jit/trace_cache.rs
//! Persistent trace cache for JIT compilation.
//!
//! Serializes compiled traces to disk so subsequent runs (and ND worker
//! processes) can skip the warm-up recording phase. Each process
//! recompiles from the cached trace independently — no shared machine
//! code, no cross-process pointers.
//!
//! Concurrency: writes use temp-file + atomic rename (POSIX).
//! Multiple readers/writers on the same key converge to a valid trace
//! (last writer wins, all values equivalent for same bytecode).

use super::trace::Trace;
use crate::config::get_cache_dir;
use std::fs;
use std::path::PathBuf;

/// Subdirectory inside the XDG cache for JIT traces.
const JIT_CACHE_DIR: &str = "jit";

/// Cache version — bump when Trace format changes.
const CACHE_VERSION: u32 = 2;

/// Wrapper that includes version metadata for safe deserialization.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedTrace {
    version: u32,
    catnip_version: String,
    trace: Trace,
}

/// Persistent JIT trace cache backed by the filesystem.
pub struct TraceCache {
    directory: PathBuf,
    catnip_version: String,
    enabled: bool,
}

impl TraceCache {
    /// Create a new trace cache. Lazily creates the directory on first write.
    pub fn new() -> Self {
        let directory = get_cache_dir().join(JIT_CACHE_DIR);
        // Best-effort version detection; fallback keeps cache functional
        let catnip_version = env!("CARGO_PKG_VERSION").to_string();
        Self {
            directory,
            catnip_version,
            enabled: true,
        }
    }

    /// Enable or disable the cache.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Build a cache key from bytecode around the loop offset.
    /// The key is deterministic: same bytecode + offset = same key.
    fn cache_key(&self, bytecode_hash: u64, loop_offset: usize) -> String {
        format!(
            "v{}_{:016x}_{:06x}",
            CACHE_VERSION, bytecode_hash, loop_offset
        )
    }

    /// File path for a given cache key.
    fn file_path(&self, key: &str) -> PathBuf {
        self.directory.join(key)
    }

    /// Try to load a cached trace for a loop.
    /// Returns `None` on cache miss or deserialization error.
    pub fn load(&self, bytecode_hash: u64, loop_offset: usize) -> Option<Trace> {
        if !self.enabled {
            return None;
        }

        let key = self.cache_key(bytecode_hash, loop_offset);
        let path = self.file_path(&key);

        let data = fs::read(&path).ok()?;
        let cached: CachedTrace = bincode::deserialize(&data).ok()?;

        // Validate version — stale entries silently ignored
        if cached.version != CACHE_VERSION || cached.catnip_version != self.catnip_version {
            // Stale entry, remove it
            let _ = fs::remove_file(&path);
            return None;
        }

        Some(cached.trace)
    }

    /// Store a trace in the cache. Atomic write via temp + rename.
    pub fn store(&self, bytecode_hash: u64, loop_offset: usize, trace: &Trace) {
        if !self.enabled {
            return;
        }

        // Ensure directory exists (lazy init)
        if !self.directory.exists() {
            if fs::create_dir_all(&self.directory).is_err() {
                return;
            }
        }

        let cached = CachedTrace {
            version: CACHE_VERSION,
            catnip_version: self.catnip_version.clone(),
            trace: trace.clone(),
        };

        let data = match bincode::serialize(&cached) {
            Ok(d) => d,
            Err(_) => return,
        };

        let key = self.cache_key(bytecode_hash, loop_offset);
        let path = self.file_path(&key);
        let tmp_path = path.with_extension("tmp");

        // Atomic: write tmp, rename over final path
        if fs::write(&tmp_path, &data).is_ok() {
            let _ = fs::rename(&tmp_path, &path);
        }
    }

    /// Clear all cached JIT traces.
    pub fn clear(&self) -> u64 {
        let mut removed = 0u64;
        if let Ok(entries) = fs::read_dir(&self.directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && !path.extension().is_some_and(|e| e == "tmp") {
                    if fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
        removed
    }

    /// Count cached traces.
    pub fn len(&self) -> usize {
        fs::read_dir(&self.directory)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| {
                        e.path().is_file() && !e.path().extension().is_some_and(|ext| ext == "tmp")
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for TraceCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a stable hash of a bytecode slice (for cache key generation).
/// Uses FNV-1a for speed — no crypto requirement here.
pub fn hash_bytecode(code: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
    for &byte in code {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3); // FNV prime
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jit::trace::{Trace, TraceOp, TraceType};
    use std::env;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_cache() -> TraceCache {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("catnip_jit_test_{}_{}", std::process::id(), id));
        let _ = fs::create_dir_all(&dir);
        TraceCache {
            directory: dir,
            catnip_version: "test".to_string(),
            enabled: true,
        }
    }

    fn cleanup(cache: &TraceCache) {
        let _ = fs::remove_dir_all(&cache.directory);
    }

    fn sample_trace() -> Trace {
        let mut t = Trace::new(42);
        t.ops.push(TraceOp::LoadConstInt(10));
        t.ops.push(TraceOp::StoreLocal(0));
        t.ops.push(TraceOp::LoadLocal(0));
        t.ops.push(TraceOp::LoadConstInt(1));
        t.ops.push(TraceOp::AddInt);
        t.ops.push(TraceOp::StoreLocal(0));
        t.ops.push(TraceOp::LoopBack);
        t.iterations = 1;
        t
    }

    #[test]
    fn test_store_and_load() {
        let cache = temp_cache();
        let trace = sample_trace();
        let hash = 0xDEADBEEF;

        cache.store(hash, 42, &trace);
        let loaded = cache.load(hash, 42);
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.loop_offset, 42);
        assert_eq!(loaded.ops.len(), trace.ops.len());
        assert_eq!(loaded.trace_type, TraceType::Loop);

        cleanup(&cache);
    }

    #[test]
    fn test_cache_miss() {
        let cache = temp_cache();
        assert!(cache.load(0x1234, 99).is_none());
        cleanup(&cache);
    }

    #[test]
    fn test_version_mismatch() {
        let cache = temp_cache();
        let trace = sample_trace();
        cache.store(0xABCD, 10, &trace);

        // Change version — should miss
        let cache2 = TraceCache {
            directory: cache.directory.clone(),
            catnip_version: "other_version".to_string(),
            enabled: true,
        };
        assert!(cache2.load(0xABCD, 10).is_none());

        cleanup(&cache);
    }

    #[test]
    fn test_clear() {
        let cache = temp_cache();
        cache.store(0x1, 1, &sample_trace());
        cache.store(0x2, 2, &sample_trace());
        assert_eq!(cache.len(), 2);

        let removed = cache.clear();
        assert_eq!(removed, 2);
        assert_eq!(cache.len(), 0);

        cleanup(&cache);
    }

    #[test]
    fn test_disabled() {
        let mut cache = temp_cache();
        cache.set_enabled(false);
        cache.store(0x1, 1, &sample_trace());
        assert!(cache.load(0x1, 1).is_none());
        assert_eq!(cache.len(), 0);
        cleanup(&cache);
    }

    #[test]
    fn test_hash_bytecode_deterministic() {
        let bc = &[1, 2, 3, 4, 5];
        assert_eq!(hash_bytecode(bc), hash_bytecode(bc));
    }

    #[test]
    fn test_hash_bytecode_different() {
        assert_ne!(hash_bytecode(&[1, 2, 3]), hash_bytecode(&[1, 2, 4]));
    }
}
