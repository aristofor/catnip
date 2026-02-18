// FILE: catnip_rs/src/jit/executor.rs
//! JIT executor: manages compiled code and execution.

use super::codegen::{CompiledFn, JITCodegen};
use super::detector::HotLoopDetector;
use super::inliner::{InliningConfig, PureInliner};
use super::registry::PureFunctionRegistry;
use super::trace::TraceRecorder;
use super::trace_cache::TraceCache;
use crate::constants::JIT_THRESHOLD_DEFAULT;
use crate::vm::value::Value;
use std::collections::HashMap;

/// Unbox a NaN-boxed integer value (called from JIT compiled code).
/// Returns the raw i64 value, or 0 if not an integer.
/// Special case: -1 (guard failure) is passed through as-is.
#[no_mangle]
pub extern "C" fn catnip_unbox_int(boxed: i64) -> i64 {
    // Guard failure code: pass through
    if boxed == -1 {
        return -1;
    }
    let value = Value::from_raw(boxed as u64);
    value.as_int().unwrap_or(0)
}

/// Unbox a NaN-boxed float value (called from JIT compiled code).
/// Returns the raw f64 value, or 0.0 if not a float.
#[no_mangle]
pub extern "C" fn catnip_unbox_float(boxed: i64) -> f64 {
    let value = Value::from_raw(boxed as u64);
    value.as_float().unwrap_or(0.0)
}

/// JIT executor that manages compilation and execution of hot loops and functions.
pub struct JITExecutor {
    /// Hot loop detector
    pub detector: HotLoopDetector,
    /// Trace recorder
    pub recorder: TraceRecorder,
    /// Code generator
    codegen: Option<JITCodegen>,
    /// Compiled loop traces: loop_offset -> function pointer
    compiled: HashMap<usize, CompiledFn>,
    /// Guards for each compiled loop trace: loop_offset -> Vec<(name, expected_value, slot)>
    guards: HashMap<usize, Vec<(String, i64, usize)>>,
    /// Compiled function traces: func_id -> (function pointer, num_locals_used, name_guards)
    compiled_functions: HashMap<String, (CompiledFn, usize, Vec<(String, i64, usize)>)>,
    /// Registry of pure functions available for inlining
    pure_registry: PureFunctionRegistry,
    /// JIT enabled flag
    enabled: bool,
    /// Compilation threshold (overrides detector)
    threshold: u32,
    /// Persistent trace cache (disk-backed, process-safe)
    trace_cache: TraceCache,
    /// Bytecode hash for the current code object (set before execution)
    bytecode_hash: u64,
}

impl JITExecutor {
    /// Create a new JIT executor.
    pub fn new(threshold: u32) -> Self {
        Self {
            detector: HotLoopDetector::new(threshold),
            recorder: TraceRecorder::new(),
            codegen: None,
            compiled: HashMap::new(),
            guards: HashMap::new(),
            compiled_functions: HashMap::new(),
            pure_registry: PureFunctionRegistry::new(),
            enabled: true,
            threshold,
            trace_cache: TraceCache::new(),
            bytecode_hash: 0,
        }
    }

    /// Enable or disable JIT.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if JIT is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Initialize codegen lazily (expensive to create).
    fn ensure_codegen(&mut self) -> Result<&mut JITCodegen, String> {
        if self.codegen.is_none() {
            self.codegen = Some(JITCodegen::new()?);
        }
        Ok(self.codegen.as_mut().unwrap())
    }

    /// Set bytecode hash for trace cache key generation.
    /// Must be called before execution with the hash of the current code object.
    pub fn set_bytecode_hash(&mut self, hash: u64) {
        self.bytecode_hash = hash;
    }

    /// Try to compile a loop from a cached trace (skip warm-up recording).
    /// Returns true if a cached trace was found and compiled successfully.
    pub fn try_compile_from_cache(&mut self, loop_offset: usize) -> bool {
        if !self.enabled || self.compiled.contains_key(&loop_offset) {
            return false;
        }

        let trace = match self.trace_cache.load(self.bytecode_hash, loop_offset) {
            Some(t) => t,
            None => return false,
        };

        // Compile the cached trace
        match self.compile_trace_inner(trace) {
            Ok(true) => true,
            _ => false,
        }
    }

    /// Record loop header execution, return true if should start tracing.
    #[inline]
    pub fn record_loop(&mut self, offset: usize) -> bool {
        if !self.enabled {
            return false;
        }
        self.detector.record_loop_header(offset)
    }

    /// Check if a compiled version exists for a loop.
    #[inline]
    pub fn has_compiled(&self, offset: usize) -> bool {
        self.compiled.contains_key(&offset)
    }

    /// Get compiled function for a loop.
    #[inline]
    pub fn get_compiled(&self, offset: usize) -> Option<CompiledFn> {
        self.compiled.get(&offset).copied()
    }

    /// Get guards for a compiled loop.
    pub fn get_guards(&self, offset: usize) -> Option<&Vec<(String, i64, usize)>> {
        self.guards.get(&offset)
    }

    /// Check if a compiled version exists for a function.
    #[inline]
    pub fn has_compiled_function(&self, func_id: &str) -> bool {
        self.compiled_functions.contains_key(func_id)
    }

    /// Get compiled function for a function trace.
    /// Returns (function_pointer, max_slot_used, name_guards)
    #[inline]
    pub fn get_compiled_function(
        &self,
        func_id: &str,
    ) -> Option<(CompiledFn, usize, &[(String, i64, usize)])> {
        self.compiled_functions
            .get(func_id)
            .map(|(f, s, g)| (*f, *s, g.as_slice()))
    }

    /// Store a compiled function trace.
    pub fn store_compiled_function(
        &mut self,
        func_id: String,
        compiled: CompiledFn,
        max_slot: usize,
        name_guards: Vec<(String, i64, usize)>,
    ) {
        self.compiled_functions
            .insert(func_id, (compiled, max_slot, name_guards));
    }

    /// Start tracing a hot loop.
    pub fn start_tracing(&mut self, offset: usize, num_locals: usize) {
        self.detector.start_tracing(offset);
        self.recorder.start(offset, num_locals);
    }

    /// Check if currently tracing.
    #[inline]
    pub fn is_tracing(&self) -> bool {
        self.recorder.is_recording()
    }

    /// Stop tracing and compile if successful.
    pub fn finish_tracing(&mut self) -> Result<bool, String> {
        let trace = match self.recorder.stop() {
            Some(t) => t,
            None => return Ok(false),
        };

        self.compile_trace(trace)
    }

    /// Compile an externally-recorded trace (for loops).
    /// Caches the trace to disk after successful compilation.
    pub fn compile_trace(&mut self, trace: super::trace::Trace) -> Result<bool, String> {
        let offset = trace.loop_offset;
        self.detector.stop_tracing(offset);

        // Cache the trace before inlining (raw recorded form, portable)
        let cache_trace = trace.clone();

        let result = self.compile_trace_inner(trace)?;

        if result {
            // Store to disk cache for future runs / worker processes
            self.trace_cache
                .store(self.bytecode_hash, offset, &cache_trace);
        }

        Ok(result)
    }

    /// Internal compilation logic shared between fresh traces and cached loads.
    fn compile_trace_inner(&mut self, trace: super::trace::Trace) -> Result<bool, String> {
        let offset = trace.loop_offset;

        if !trace.is_compilable() {
            return Ok(false);
        }

        // Store guards for this trace
        self.guards.insert(offset, trace.name_guards.clone());

        // Apply pure function inlining optimization
        let mut trace = trace;
        if !self.pure_registry.is_empty() {
            let config = InliningConfig::default();
            let mut inliner = PureInliner::new(config, &self.pure_registry);
            let _ = inliner.optimize(&mut trace);
        }

        // Compile trace
        let codegen = self.ensure_codegen()?;
        let func = codegen.compile(&trace)?;

        self.compiled.insert(offset, func);
        self.detector.mark_compiled_offset(offset);

        Ok(true)
    }

    /// Compile a function trace and return (compiled function pointer, max_slot_used, name_guards).
    pub fn compile_function_trace(
        &mut self,
        trace: &super::trace::Trace,
    ) -> Result<(CompiledFn, usize, Vec<(String, i64, usize)>), String> {
        if !trace.is_compilable() {
            return Err("Trace is not compilable".into());
        }

        // Apply pure function inlining optimization
        let mut trace = trace.clone();
        if !self.pure_registry.is_empty() {
            let config = InliningConfig::default();
            let mut inliner = PureInliner::new(config, &self.pure_registry);
            // Ignore inlining errors - just continue without inlining
            let _ = inliner.optimize(&mut trace);
        }

        // Calculate max slot used from trace
        let max_slot = trace.locals_used.iter().copied().max().unwrap_or(0);

        let name_guards = trace.name_guards.clone();

        // Compile function trace
        let codegen = self.ensure_codegen()?;
        let func = codegen.compile(&trace)?;

        Ok((func, max_slot, name_guards))
    }

    /// Abort current tracing.
    pub fn abort_tracing(&mut self) {
        if let Some(trace) = self.recorder.stop() {
            self.detector.stop_tracing(trace.loop_offset);
        }
    }

    /// Execute compiled code for a loop.
    ///
    /// # Safety
    /// Caller must ensure locals array has enough elements.
    #[inline]
    pub unsafe fn execute(&self, offset: usize, locals: &mut [i64]) -> Option<i64> {
        let func = self.compiled.get(&offset)?;
        // Phase 3: Pass depth=0 for initial call (loops don't recurse)
        Some(func(locals.as_mut_ptr(), 0))
    }

    /// Get statistics.
    pub fn stats(&self) -> JITStats {
        let detector_stats = self.detector.stats();
        JITStats {
            enabled: self.enabled,
            threshold: self.threshold,
            compiled_traces: self.compiled.len(),
            hot_loops: detector_stats.hot_loops,
            tracing_loops: detector_stats.tracing_loops,
            total_loops_tracked: detector_stats.total_loops_tracked,
            cached_traces: self.trace_cache.len(),
        }
    }

    /// Access to the trace cache (for clearing, stats, etc.)
    pub fn trace_cache(&self) -> &TraceCache {
        &self.trace_cache
    }

    /// Mutable access to the trace cache.
    pub fn trace_cache_mut(&mut self) -> &mut TraceCache {
        &mut self.trace_cache
    }

    /// Reset all JIT state.
    pub fn reset(&mut self) {
        self.detector.reset();
        self.recorder.abort();
        self.compiled.clear();
        self.guards.clear();
    }

    /// Register a pure function for potential inlining.
    /// Only stores if function is marked as pure.
    pub fn register_pure_function(&mut self, func_id: String, code: crate::vm::frame::CodeObject) {
        self.pure_registry.register(func_id, code);
    }

    /// Check if a function is registered as pure.
    pub fn is_pure_function(&self, func_id: &str) -> bool {
        self.pure_registry.is_pure(func_id)
    }

    /// Get reference to pure function registry (for inliner).
    pub fn pure_registry(&self) -> &PureFunctionRegistry {
        &self.pure_registry
    }
}

impl Default for JITExecutor {
    fn default() -> Self {
        Self::new(JIT_THRESHOLD_DEFAULT)
    }
}

/// JIT statistics.
#[derive(Debug, Clone)]
pub struct JITStats {
    pub enabled: bool,
    pub threshold: u32,
    pub compiled_traces: usize,
    pub hot_loops: usize,
    pub tracing_loops: usize,
    pub total_loops_tracked: usize,
    pub cached_traces: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_creation() {
        let executor = JITExecutor::new(100);
        assert!(executor.is_enabled());
        assert_eq!(executor.stats().threshold, 100);
    }

    #[test]
    fn test_hot_loop_detection() {
        let mut executor = JITExecutor::new(3);

        assert!(!executor.record_loop(100));
        assert!(!executor.record_loop(100));
        assert!(executor.record_loop(100)); // Hot!
        assert!(!executor.record_loop(100)); // Already hot
    }
}
