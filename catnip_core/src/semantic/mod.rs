// FILE: catnip_core/src/semantic/mod.rs
//! Semantic analysis - pure Rust, no PyO3.

pub mod opcode;
pub mod passes;

pub use opcode::OpCode;
pub use passes::PureOptimizer;
