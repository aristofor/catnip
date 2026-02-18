// FILE: catnip_rs/src/semantic/mod.rs
//! Semantic analysis and IR optimization
//!
//! Port of catnip/semantic/ from Cython to Rust

pub mod analyzer;
pub mod base;
pub mod block_flattening;
pub mod blunt_code;
pub mod common_subexpression_elimination;
pub mod constant_folding;
pub mod constant_propagation;
pub mod copy_propagation;
pub mod dead_code_elimination;
pub mod dead_store_elimination;
pub mod function_inlining;
pub mod opcode;
pub mod optimizer;
pub mod strength_reduction;
pub mod tail_recursion_to_loop;

// Re-exports for convenience
pub use analyzer::Semantic;
pub use base::{OptimizationPassBase, Optimizer};
pub use block_flattening::BlockFlatteningPass;
pub use blunt_code::BluntCodePass;
pub use common_subexpression_elimination::CommonSubexpressionEliminationPass;
pub use constant_folding::ConstantFoldingPass;
pub use constant_propagation::ConstantPropagationPass;
pub use copy_propagation::CopyPropagationPass;
pub use dead_code_elimination::DeadCodeEliminationPass;
pub use dead_store_elimination::DeadStoreEliminationPass;
pub use function_inlining::FunctionInliningPass;
pub use opcode::OpCode;
pub use optimizer::OptimizationPass;
pub use strength_reduction::StrengthReductionPass;
pub use tail_recursion_to_loop::TailRecursionToLoopPass;

#[cfg(test)]
mod tests;
