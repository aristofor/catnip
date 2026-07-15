// FILE: catnip_core/src/lib.rs
//! Pure Rust core for Catnip - no PyO3 dependency.
//!
//! Contains types, opcodes, and pure algorithms shared between
//! the Python binding crate (`catnip_rs`) and standalone tools.

// Test float literals (e.g. `3.14`) are sample data, not approximations of math constants.
#![cfg_attr(test, allow(clippy::approx_constant))]
// Production `unsafe` blocks must carry a `// SAFETY:` justification; test-local
// unsafe is exempt.
#![cfg_attr(not(test), deny(clippy::undocumented_unsafe_blocks))]

pub mod arith;
pub mod cfg;
pub mod constants;
pub mod delta;
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
pub mod scalar;
pub mod semantic;
pub mod symbols;
pub mod types;
pub mod vm;
