// FILE: catnip_core/src/vm/mod.rs
//! VM types - pure Rust opcodes and utilities.

pub mod memory;
pub mod mro;
pub mod opcode;
pub mod peephole;

pub use opcode::{Instruction, VMOpCode};
pub use peephole::PeepholeOptimizer;

// ForRangeInt bit-packing: (slot_i << 24) | (slot_stop << 16) | (step_sign << 15) | jump_offset
pub const FOR_RANGE_SLOT_I_SHIFT: u32 = 24;
pub const FOR_RANGE_SLOT_STOP_SHIFT: u32 = 16;
pub const FOR_RANGE_STEP_SIGN_SHIFT: u32 = 15;
pub const FOR_RANGE_SLOT_MASK: u32 = 0xFF;
pub const FOR_RANGE_JUMP_MASK: u32 = 0x7FFF;

// ForRangeStep: (slot_i << 24) | (step_i8 << 16) | jump_target
pub const FOR_RANGE_STEP_SHIFT: u32 = 16;
pub const FOR_RANGE_STEP_BYTE_MASK: u32 = 0xFF;
pub const FOR_RANGE_STEP_JUMP_MASK: u32 = 0xFFFF;
