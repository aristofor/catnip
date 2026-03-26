// FILE: catnip_core/src/jit/mod.rs
//! JIT types - pure Rust, no PyO3.

pub mod builtin_dispatch;
pub mod codegen;
pub mod detector;
pub mod executor;
pub mod function_info;
pub mod inliner;
pub mod memo_cache;
pub mod registry;
pub mod trace;
pub mod trace_cache;

pub use builtin_dispatch::catnip_call_builtin;
pub use codegen::JITCodegen;
pub use detector::{DetectorStats, HotLoopDetector};
pub use executor::{JITExecutor, JITStats};
pub use function_info::{JitConstant, JitFunctionInfo};
pub use inliner::{InliningConfig, PureInliner};
pub use memo_cache::{memo_lookup, memo_store};
pub use registry::PureFunctionRegistry;
pub use trace::{Trace, TraceOp, TraceRecorder};
pub use trace_cache::{NativeCodeCache, TraceCache, hash_bytecode};
