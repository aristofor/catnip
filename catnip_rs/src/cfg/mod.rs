// FILE: catnip_rs/src/cfg/mod.rs
//! Control Flow Graph (CFG) module.
//!
//! Provides CFG construction and analysis at IR level for semantic optimizations.
//!
//! Main components:
//! - BasicBlock - Linear sequence of instructions
//! - CFGEdge - Control flow edge with type
//! - ControlFlowGraph - Complete CFG with entry/exit nodes
//! - IRCFGBuilder - Constructs CFG from IR OpCode nodes

pub mod analysis;
pub mod basic_block;
pub mod builder_ir;
pub mod edge;
pub mod graph;
pub mod optimization;
pub mod reconstruction;
pub mod region;
pub mod ssa;
pub mod ssa_builder;
pub mod ssa_cse;
pub mod ssa_destruction;
pub mod ssa_dse;
pub mod ssa_gvn;
pub mod ssa_iv;
pub mod ssa_licm;

#[cfg(test)]
mod tests;

pub use basic_block::BasicBlock;
pub use builder_ir::IRCFGBuilder;
pub use edge::{CFGEdge, EdgeType};
pub use graph::ControlFlowGraph;
pub use region::reconstruct_from_cfg;

use pyo3::prelude::*;

/// Register CFG module with Python.
pub fn register_module(py: Python<'_>, parent_module: &Bound<'_, PyModule>) -> PyResult<()> {
    let cfg_module = PyModule::new(py, "cfg")?;

    cfg_module.add_class::<graph::PyControlFlowGraph>()?;
    cfg_module.add_function(wrap_pyfunction!(
        builder_ir::build_cfg_from_ir,
        &cfg_module
    )?)?;
    cfg_module.add_function(wrap_pyfunction!(
        region::py_reconstruct_from_cfg,
        &cfg_module
    )?)?;

    parent_module.add_submodule(&cfg_module)?;
    Ok(())
}
