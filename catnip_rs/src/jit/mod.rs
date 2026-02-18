// FILE: catnip_rs/src/jit/mod.rs
//! Trace-based JIT compiler for Catnip VM.
//!
//! Records execution traces of hot loops and compiles them to native code
//! using Cranelift.

pub mod builtin_dispatch;
mod codegen;
mod detector;
mod executor;
mod inliner;
mod memo_cache;
mod registry;
mod trace;
pub mod trace_cache;

pub use builtin_dispatch::catnip_call_builtin;
pub use codegen::JITCodegen;
pub use detector::HotLoopDetector;
pub use executor::JITExecutor;
pub use inliner::{InliningConfig, PureInliner};
pub use memo_cache::{memo_lookup, memo_store};
pub use registry::PureFunctionRegistry;
pub use trace::{Trace, TraceOp, TraceRecorder};
pub use trace_cache::{hash_bytecode, TraceCache};
