// FILE: catnip_core/src/jit/executor.rs
//! JIT executor: manages compiled code and execution.

use super::codegen::{CompiledFn, JITCodegen};
use super::detector::HotLoopDetector;
use super::function_info::JitFunctionInfo;
use super::inliner::{InliningConfig, PureInliner};
use super::registry::PureFunctionRegistry;
use super::trace::{Trace, TraceOp, TraceRecorder};
use super::trace_cache::TraceCache;
use crate::constants::JIT_THRESHOLD_DEFAULT;
use std::collections::HashMap;
use std::sync::Arc;

type NameGuard = (String, i64, usize);
type FunctionTrace = (CompiledFn, usize, Vec<NameGuard>);
/// Key for per-loop compiled state: (bytecode_hash, loop_offset). The hash
/// disambiguates loops sharing a loop_offset across distinct code objects -- a
/// second program reusing the same VM, or a function vs the top level -- so a
/// stale trace is never run for the wrong bytecode.
type LoopKey = (u64, usize);

/// JIT executor that manages compilation and execution of hot loops and functions.
pub struct JITExecutor {
    /// Hot loop detector
    pub detector: HotLoopDetector,
    /// Trace recorder
    pub recorder: TraceRecorder,
    /// Code generator
    codegen: Option<JITCodegen>,
    /// Compiled loop traces: (bytecode_hash, loop_offset) -> function pointer
    compiled: HashMap<LoopKey, CompiledFn>,
    /// Guards for each compiled loop trace: key -> Vec<(name, expected_value, slot)>
    guards: HashMap<LoopKey, Vec<NameGuard>>,
    /// Function-identity guards for each compiled loop trace:
    /// key -> Vec<(name, expected_nanbox_bits)>. Check-only at JIT entry
    /// (an inlined scope function must still resolve to the same value).
    func_guards: HashMap<LoopKey, Vec<(String, u64)>>,
    /// Identity guards for inlined functions held in a frame LOCAL slot:
    /// key -> Vec<(slot, expected_nanbox_bits)>. Verified at JIT entry
    /// against frame.locals[slot] (and those slots are excluded from the
    /// numeric-only local scan, since they hold a function, not a number).
    func_slot_guards: HashMap<LoopKey, Vec<(usize, u64)>>,
    /// Highest local slot the compiled loop code addresses: key -> max_slot.
    /// Sizes the locals array at execution time; codegen writes every
    /// `trace.locals_used` slot, not just guard slots.
    loop_max_slot: HashMap<LoopKey, usize>,
    /// Per-slot type contract of the compiled loop: key -> Vec<(slot, is_float)>,
    /// from the trace's GuardInt/GuardFloat (post-inlining, what codegen typed).
    /// Compiled code unboxes each slot with this type and never re-checks at
    /// runtime; the VM must validate it at JIT entry.
    slot_type_guards: HashMap<LoopKey, Vec<(usize, bool)>>,
    /// Compiled function traces: func_id -> (function pointer, num_locals_used, name_guards)
    compiled_functions: HashMap<String, FunctionTrace>,
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
            func_guards: HashMap::new(),
            func_slot_guards: HashMap::new(),
            loop_max_slot: HashMap::new(),
            slot_type_guards: HashMap::new(),
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
        self.detector.set_bytecode_hash(hash);
    }

    /// Try to compile a loop from a cached trace (skip warm-up recording).
    /// Returns true if a cached trace was found and compiled successfully.
    pub fn try_compile_from_cache(&mut self, loop_offset: usize) -> bool {
        if !self.enabled || self.compiled.contains_key(&(self.bytecode_hash, loop_offset)) {
            return false;
        }

        let trace = match self.trace_cache.load(self.bytecode_hash, loop_offset) {
            Some(t) => t,
            None => return false,
        };

        matches!(self.compile_trace_inner(trace), Ok(true))
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
        self.compiled.contains_key(&(self.bytecode_hash, offset))
    }

    /// Get compiled function for a loop.
    #[inline]
    pub fn get_compiled(&self, offset: usize) -> Option<CompiledFn> {
        self.compiled.get(&(self.bytecode_hash, offset)).copied()
    }

    /// Get guards for a compiled loop.
    pub fn get_guards(&self, offset: usize) -> Option<&Vec<(String, i64, usize)>> {
        self.guards.get(&(self.bytecode_hash, offset))
    }

    /// Get function-identity guards for a compiled loop.
    pub fn get_func_guards(&self, offset: usize) -> Option<&Vec<(String, u64)>> {
        self.func_guards.get(&(self.bytecode_hash, offset))
    }

    /// Get local-slot function-identity guards for a compiled loop.
    pub fn get_func_slot_guards(&self, offset: usize) -> Option<&Vec<(usize, u64)>> {
        self.func_slot_guards.get(&(self.bytecode_hash, offset))
    }

    /// Highest local slot the compiled loop code addresses, if compiled.
    #[inline]
    pub fn get_loop_max_slot(&self, offset: usize) -> Option<usize> {
        self.loop_max_slot.get(&(self.bytecode_hash, offset)).copied()
    }

    /// Per-slot type contract of a compiled loop: (slot, is_float) pairs the
    /// VM must validate before entering the compiled code.
    pub fn get_slot_type_guards(&self, offset: usize) -> Option<&Vec<(usize, bool)>> {
        self.slot_type_guards.get(&(self.bytecode_hash, offset))
    }

    /// Check if a compiled version exists for a function.
    #[inline]
    pub fn has_compiled_function(&self, func_id: &str) -> bool {
        self.compiled_functions.contains_key(func_id)
    }

    /// Get compiled function for a function trace.
    /// Returns (function_pointer, max_slot_used, name_guards)
    #[inline]
    pub fn get_compiled_function(&self, func_id: &str) -> Option<(CompiledFn, usize, &[NameGuard])> {
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
        name_guards: Vec<NameGuard>,
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
    pub fn compile_trace(&mut self, trace: Trace) -> Result<bool, String> {
        let offset = trace.loop_offset;
        self.detector.stop_tracing(offset);

        // Cache the trace before inlining (raw recorded form, portable)
        let cache_trace = trace.clone();

        let result = self.compile_trace_inner(trace)?;

        if result {
            self.trace_cache.store(self.bytecode_hash, offset, &cache_trace);
        }

        Ok(result)
    }

    /// Internal compilation logic shared between fresh traces and cached loads.
    fn compile_trace_inner(&mut self, trace: Trace) -> Result<bool, String> {
        let offset = trace.loop_offset;
        let key = (self.bytecode_hash, offset);

        if !trace.is_compilable() {
            return Ok(false);
        }

        // Apply pure function inlining optimization
        let mut trace = trace;
        if !self.pure_registry.is_empty() {
            let config = InliningConfig::default();
            let mut inliner = PureInliner::new(config, &self.pure_registry);
            let _ = inliner.optimize(&mut trace);
        }

        // Compile trace FIRST: the guard maps below must describe the code
        // that actually sits in `compiled`. Overwriting them before a compile
        // that can fail (or bail) leaves stale native code paired with fresh
        // guards -- the VM would then admit values the old code cannot unbox
        // (silent garbage results).
        let hash = self.bytecode_hash;
        let codegen = self.ensure_codegen()?;
        let func = codegen.compile(&trace, hash)?;

        // Store guards for this trace. All inserted unconditionally so a
        // recompile of the same key overwrites any prior entry (an empty
        // func_guards must replace a stale non-empty one, never leave it behind).
        self.guards.insert(key, trace.name_guards.clone());
        self.func_guards.insert(key, trace.func_guards.clone());
        self.func_slot_guards.insert(key, trace.func_slot_guards.clone());

        // Record the highest slot codegen addresses (post-inlining), so the
        // executor can size the locals array even on the warm-start path where
        // frame.locals was never extended by tracing.
        if let Some(max_slot) = trace.locals_used.iter().copied().max() {
            self.loop_max_slot.insert(key, max_slot);
        }

        // Per-slot type contract (post-inlining, mirrors codegen's
        // compute_slot_types last-wins semantics): compiled code unboxes each
        // guarded slot as this type without any runtime re-check, so the VM
        // must enforce it at JIT entry.
        let mut slot_kinds: HashMap<usize, bool> = HashMap::new();
        for op in &trace.ops {
            match op {
                TraceOp::GuardInt(slot) => {
                    slot_kinds.insert(*slot, false);
                }
                TraceOp::GuardFloat(slot) => {
                    slot_kinds.insert(*slot, true);
                }
                _ => {}
            }
        }
        self.slot_type_guards.insert(key, slot_kinds.into_iter().collect());

        self.compiled.insert(key, func);
        self.detector.mark_compiled_offset(offset);

        Ok(true)
    }

    /// Compile a function trace and return (compiled function pointer, max_slot_used, name_guards).
    pub fn compile_function_trace(&mut self, trace: &Trace) -> Result<FunctionTrace, String> {
        if !trace.is_compilable() {
            return Err("Trace is not compilable".into());
        }

        // Apply pure function inlining optimization
        let mut trace = trace.clone();
        if !self.pure_registry.is_empty() {
            let config = InliningConfig::default();
            let mut inliner = PureInliner::new(config, &self.pure_registry);
            let _ = inliner.optimize(&mut trace);
        }

        // Calculate max slot used from trace
        let max_slot = trace.locals_used.iter().copied().max().unwrap_or(0);

        let name_guards = trace.name_guards.clone();

        // Compile function trace
        let hash = self.bytecode_hash;
        let codegen = self.ensure_codegen()?;
        let func = codegen.compile(&trace, hash)?;

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
        let func = self.compiled.get(&(self.bytecode_hash, offset))?;
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
        self.func_guards.clear();
        self.func_slot_guards.clear();
        self.loop_max_slot.clear();
    }

    /// Register a pre-built JitFunctionInfo for potential inlining.
    pub fn register_pure_info(&mut self, func_id: String, info: Arc<JitFunctionInfo>) {
        self.pure_registry.register(func_id, info);
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
