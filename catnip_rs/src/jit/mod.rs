// FILE: catnip_rs/src/jit/mod.rs
//! Trace-based JIT compiler for Catnip VM.
//!
//! Records execution traces of hot loops and compiles them to native code
//! using Cranelift.

mod detector;
pub mod executor;

// Re-exported from catnip_core (pure Rust, no PyO3)
pub use catnip_core::jit::{builtin_dispatch, codegen, inliner, memo_cache, registry, trace, trace_cache};

pub use catnip_core::jit::{
    HotLoopDetector, JITExecutor, JitConstant, JitFunctionInfo, NativeCodeCache, PureFunctionRegistry, PureInliner,
    Trace, TraceCache, TraceOp, TraceRecorder, catnip_call_builtin, hash_bytecode, memo_lookup, memo_store,
};
pub use detector::PyHotLoopDetector;
