// FILE: catnip_core/src/parser/mod.rs
//! Parser types - pure Rust tree-sitter transforms, no PyO3.

pub mod pure_transforms;
pub mod utils;

pub use pure_transforms::transform as transform_pure;
