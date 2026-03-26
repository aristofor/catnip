// FILE: catnip_core/src/ir/mod.rs
//! IR (Intermediate Representation) - pure Rust types and opcodes.

pub mod opcode;
pub mod pure;

#[cfg(test)]
mod tests;

pub use opcode::IROpCode;
pub use pure::{BroadcastType, IR};
