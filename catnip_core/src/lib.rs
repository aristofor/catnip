// FILE: catnip_core/src/lib.rs
//! Pure Rust core for Catnip - no PyO3 dependency.
//!
//! Contains types, opcodes, and pure algorithms shared between
//! the Python binding crate (`catnip_rs`) and standalone tools.

pub mod cfg;
pub mod constants;
pub mod exception;
pub mod freeze;
pub mod ir;
pub mod jit;
pub mod loader;
pub mod nanbox;
pub mod parser;
pub mod paths;
pub mod pipeline;
pub mod policy;
pub mod semantic;
pub mod symbols;
pub mod types;
pub mod vm;
