// FILE: catnip_core/src/jit/detector.rs
//! Hot loop detection for JIT compilation.
//!
//! Tracks execution counts of loop headers and triggers compilation
//! when a threshold is reached.

use std::collections::{HashMap, HashSet};

use crate::constants::JIT_THRESHOLD_DEFAULT;

/// Detects hot loops by tracking execution counts at loop headers.
pub struct HotLoopDetector {
    /// Execution count threshold before triggering compilation
    threshold: u32,
    /// Execution counts per function (identified by func_id string)
    counts: HashMap<String, u32>,
    /// Functions that have reached the threshold
    hot_functions: HashSet<String>,
    /// Functions that have been compiled
    compiled_functions: HashSet<String>,
    /// Functions currently being traced (for VM use)
    tracing_loops: HashSet<u64>,
}

impl HotLoopDetector {
    /// Create a new detector with the given threshold.
    pub fn new(threshold: u32) -> Self {
        Self {
            threshold,
            counts: HashMap::new(),
            hot_functions: HashSet::new(),
            compiled_functions: HashSet::new(),
            tracing_loops: HashSet::new(),
        }
    }

    /// Record a loop header execution (VM use). Returns true if loop just became hot.
    #[inline]
    pub fn record_loop_header(&mut self, offset: usize) -> bool {
        let func_id = format!("offset_{}", offset);
        self.record_call_internal(&func_id)
    }

    /// Record a function call (public for VM use). Returns true if function just became hot.
    pub fn record_call_internal(&mut self, func_id: &str) -> bool {
        // Skip if already compiled
        if self.compiled_functions.contains(func_id) {
            return false;
        }

        let count = self.counts.entry(func_id.to_string()).or_insert(0);
        *count += 1;

        if *count >= self.threshold && !self.hot_functions.contains(func_id) {
            self.hot_functions.insert(func_id.to_string());
            return true;
        }

        false
    }

    /// Mark a function as compiled (public for VM use).
    pub fn mark_compiled_internal(&mut self, func_id: &str) {
        self.compiled_functions.insert(func_id.to_string());
        self.hot_functions.remove(func_id);
    }

    /// Mark a loop as compiled (VM use with offset).
    pub fn mark_compiled_offset(&mut self, offset: usize) {
        let func_id = format!("offset_{}", offset);
        self.mark_compiled_internal(&func_id);
    }

    /// Check if a function is currently hot (public for VM use).
    pub fn is_hot_internal(&self, func_id: &str) -> bool {
        self.hot_functions.contains(func_id)
    }

    /// Check if a function has been compiled (public for VM use).
    pub fn is_compiled_internal(&self, func_id: &str) -> bool {
        self.compiled_functions.contains(func_id)
    }

    /// Mark a loop as currently being traced.
    pub fn start_tracing(&mut self, offset: usize) {
        self.tracing_loops.insert(offset as u64);
    }

    /// Mark a loop as no longer being traced.
    pub fn stop_tracing(&mut self, offset: usize) {
        self.tracing_loops.remove(&(offset as u64));
    }

    /// Reset all profiling data.
    pub fn reset(&mut self) {
        self.counts.clear();
        self.hot_functions.clear();
        self.compiled_functions.clear();
        self.tracing_loops.clear();
    }

    /// Get statistics.
    pub fn stats(&self) -> DetectorStats {
        DetectorStats {
            total_loops_tracked: self.counts.len(),
            hot_loops: self.hot_functions.len(),
            compiled_loops: self.compiled_functions.len(),
            tracing_loops: self.tracing_loops.len(),
        }
    }
}

impl Default for HotLoopDetector {
    fn default() -> Self {
        Self::new(JIT_THRESHOLD_DEFAULT)
    }
}

/// Statistics from the detector.
#[derive(Debug, Clone)]
pub struct DetectorStats {
    pub total_loops_tracked: usize,
    pub hot_loops: usize,
    pub compiled_loops: usize,
    pub tracing_loops: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detection_threshold() {
        let mut detector = HotLoopDetector::new(3);

        assert!(!detector.record_loop_header(100)); // count = 1
        assert!(!detector.record_loop_header(100)); // count = 2
        assert!(detector.record_loop_header(100)); // count = 3, HOT!
        assert!(!detector.record_loop_header(100)); // already hot

        assert!(detector.is_hot_internal("offset_100"));
    }

    #[test]
    fn test_compiled_skipped() {
        let mut detector = HotLoopDetector::new(2);

        detector.record_loop_header(100);
        detector.record_loop_header(100); // HOT
        assert!(detector.is_hot_internal("offset_100"));

        detector.mark_compiled_offset(100);
        assert!(!detector.is_hot_internal("offset_100"));
        assert!(detector.is_compiled_internal("offset_100"));

        // Should not increment count anymore
        assert!(!detector.record_loop_header(100));
    }
}
