// FILE: catnip_rs/src/semantic/tests/mod.rs
//! Unit tests for semantic analysis and optimization passes.
//!
//! These tests validate the semantic analyzer and optimization passes
//! without the overhead of Python interop. They test IR transformations
//! directly, making them 10-100x faster than end-to-end Python tests.

#[cfg(test)]
mod helpers;

#[cfg(test)]
mod test_blunt_code;

#[cfg(test)]
mod test_constant_folding;

#[cfg(test)]
mod test_constant_propagation;

#[cfg(test)]
mod test_cse;

#[cfg(test)]
mod test_optimizer;
