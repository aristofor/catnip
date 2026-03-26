// FILE: catnip_rs/src/core/broadcast/mod.rs
//
// Rust port of broadcast operations from Cython
// Replaces: catnip/core/broadcast_ops.pyx (194 lines)
//           catnip/core/broadcast_registry.pyx (226 lines)
//
// Provides optimized broadcasting for:
// - Map operations: target.[func]
// - Filter operations: target.[if condition]
// - Boolean mask indexing: target.[mask]
// - Element-wise operations: list(1,2,3).[+ 10]
//
// SIMD fast paths (simd.rs):
// - Listes numeriques homogenes (all i64 ou all f64)
// - Arithmetique (+, -, *, /, //, %, **), comparaisons, filtres
// - Fallback automatique vers chemin Python pour types mixtes

pub mod ops;
pub mod simd;

// Re-export main functions for convenience
pub use ops::{broadcast_binary_op, broadcast_map, filter_by_mask, filter_conditional, is_boolean_mask, nd_map};
