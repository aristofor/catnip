// FILE: catnip_rs/src/ir/mod.rs
//! IR (Intermediate Representation) opcodes module.
//!
//! Contains the IROpCode enum representing high-level semantic opcodes
//! that are compiled down to VM bytecode by the compiler.

pub mod json;
pub mod opcode;
pub mod pure;
pub mod to_python;

#[cfg(test)]
mod tests;

pub use json::{
    ir_from_json, ir_to_json, ir_to_json_compact, ir_to_json_compact_pretty, ir_to_json_pretty,
};
pub use opcode::IROpCode;
pub use pure::{BroadcastType, IRPure};
pub use to_python::ir_pure_to_python;
