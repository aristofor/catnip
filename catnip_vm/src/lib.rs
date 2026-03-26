// FILE: catnip_vm/src/lib.rs
//! Pure Rust VM for Catnip -- no PyO3 dependency.
//!
//! Provides: NaN-boxed Value, string operations, VmHost trait, PureHost,
//! and a standalone IR -> bytecode compiler.

pub mod collections;
pub mod compiler;
pub mod error;
pub mod host;
pub mod ops;
pub mod pipeline;
pub mod value;
pub mod vm;

pub use error::{VMError, VMResult};
pub use value::Value;
