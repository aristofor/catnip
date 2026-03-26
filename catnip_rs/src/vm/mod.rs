// FILE: catnip_rs/src/vm/mod.rs
//! Catnip Virtual Machine module.
//!
//! Stack-based VM that executes bytecode without growing the Python stack.
//! Uses NaN-boxing for efficient value representation.

pub mod compiler;
pub mod compiler_core;
pub mod compiler_input;
pub mod core;
pub mod frame;
pub mod host;
pub mod iter;
pub mod pattern;
pub mod py_interop;
pub mod structs;
pub mod traits;
pub mod unified_compiler;
pub mod value;

// ForRange constants -- re-exported from catnip_core
pub use catnip_core::vm::{
    FOR_RANGE_JUMP_MASK, FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_SLOT_MASK, FOR_RANGE_SLOT_STOP_SHIFT,
    FOR_RANGE_STEP_BYTE_MASK, FOR_RANGE_STEP_JUMP_MASK, FOR_RANGE_STEP_SHIFT, FOR_RANGE_STEP_SIGN_SHIFT,
};

// Re-exported from catnip_core (pure Rust, no PyO3)
pub use catnip_core::vm::{memory, mro, opcode, peephole};

// Re-export main types
pub use compiler::PyCompiler;
pub use core::{
    BigIntOpsBenchResult, VM, VMError, VMFallbackStats, bench_bigint_ops, get_vm_fallback_stats,
    reset_vm_fallback_stats,
};
pub use frame::{
    ClosureParent, ClosureScope, CodeObject, Frame, FramePool, Globals, NativeClosureScope, PyCodeObject, VMFunction,
};
pub use host::VMHost;
pub use opcode::VMOpCode as OpCode; // Alias for backward compatibility
pub use opcode::{Instruction, VMOpCode};
pub use pattern::{VMPattern, VMPatternElement};
pub use peephole::PeepholeOptimizer;
pub use py_interop::{PyVMContext, convert_code_object, convert_pure_compile_output};
pub use structs::{CatnipStructProxy, CatnipStructType, StructRegistry, SuperProxy};
pub use traits::TraitRegistry;
pub use unified_compiler::UnifiedCompiler;
pub use value::Value;
