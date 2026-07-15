// FILE: catnip_core/src/cfg/mod.rs
//! Control Flow Graph (CFG) module - pure Rust, no PyO3.
//!
//! Provides CFG construction and analysis at IR level for semantic optimizations.
//!
//! Main components:
//! - BasicBlock - Linear sequence of instructions
//! - CFGEdge - Control flow edge with type
//! - ControlFlowGraph - Complete CFG with entry/exit nodes

pub mod analysis;
pub mod basic_block;
pub mod builder_ir;
pub mod edge;
pub mod graph;
pub mod liveness;
pub mod optimization;
pub mod region;
pub mod ssa;
pub mod ssa_builder;
pub mod ssa_cse;
pub mod ssa_destruction;
pub mod ssa_dse;
pub mod ssa_gvn;
pub mod ssa_iv;
pub mod ssa_licm;

pub use basic_block::BasicBlock;
pub use builder_ir::IRCFGBuilder;
pub use edge::{CFGEdge, EdgeType};
pub use graph::ControlFlowGraph;
pub use region::reconstruct_from_cfg;

#[cfg(test)]
mod tests;
