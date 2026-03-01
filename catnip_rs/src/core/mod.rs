// FILE: catnip_rs/src/core/mod.rs
//! Catnip core components shared between VM and AST execution modes.
//!
//! This module contains the fundamental building blocks used by both
//! the Rust VM and the Python/Cython AST interpreter.

pub mod function;
pub mod meta;
pub mod method;
pub mod nodes;
pub mod op;
pub mod pattern;
pub mod registry;
pub mod scope;

pub use function::{RustFunction, RustLambda};
pub use meta::CatnipMeta;
pub use method::{BoundCatnipMethod, CatnipMethod};
pub use nodes::{Ref, TailCall};
pub use op::Op;
pub use pattern::{
    PatternLiteral, PatternOr, PatternStruct, PatternTuple, PatternVar, PatternWildcard,
};
pub use registry::Registry;
pub use scope::Scope;
