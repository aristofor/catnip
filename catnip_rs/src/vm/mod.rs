// FILE: catnip_rs/src/vm/mod.rs
//! Catnip Virtual Machine module.
//!
//! Stack-based VM that executes bytecode without growing the Python stack.
//! Uses NaN-boxing for efficient value representation.

pub mod compiler;
pub mod core;
pub mod frame;
pub mod iter;
pub mod mro;
pub mod opcode;
pub mod pattern;
pub mod peephole;
pub mod py_interop;
pub mod structs;
pub mod traits;
pub mod value;

// Re-export main types
pub use compiler::{Compiler, PyCompiler};
pub use core::{
    bench_bigint_ops, get_vm_fallback_stats, reset_vm_fallback_stats, BigIntOpsBenchResult,
    VMError, VMFallbackStats, VM,
};
pub use frame::{
    CodeObject, Frame, FramePool, NativeClosureScope, PyCodeObject, RustClosureScope,
    RustVMFunction,
};
pub use opcode::VMOpCode as OpCode; // Alias for backward compatibility
pub use opcode::{Instruction, VMOpCode};
pub use pattern::{VMPattern, VMPatternElement};
pub use peephole::PeepholeOptimizer;
pub use py_interop::{convert_code_object, PyVMContext};
pub use structs::{CatnipStructProxy, CatnipStructType, StructRegistry, SuperProxy};
pub use traits::TraitRegistry;
pub use value::Value;
