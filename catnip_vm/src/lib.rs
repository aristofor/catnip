// FILE: catnip_vm/src/lib.rs
//! Pure Rust VM for Catnip -- no PyO3 dependency.
//!
//! Provides: NaN-boxed Value, string operations, VmHost trait, PureHost,
//! and a standalone IR -> bytecode compiler.

// Test float literals (e.g. `3.14`) are sample data, not approximations of math constants.
#![cfg_attr(test, allow(clippy::approx_constant))]
// Every `unsafe` block in production code must carry a `// SAFETY:` justification
// (NaN-box payload derefs, manual refcount, FFI): the invariants are subtle and
// were the source of the refcount/UAF cluster, so they are documented at the site.
// Enforced on non-test code only; test-local `unsafe` is exempt.
#![cfg_attr(not(test), deny(clippy::undocumented_unsafe_blocks))]

pub mod collections;
pub mod compiler;
pub mod error;
pub mod host;
pub mod loader;
pub mod ops;
pub mod pipeline;
pub mod plugin;
pub mod stdlib;
#[cfg(test)]
pub(crate) mod test_support;
pub mod value;
pub mod vm;

pub use error::{VMError, VMResult};
pub use value::Value;

/// Canonical Catnip runtime version, the single source of truth for the
/// language version. Stdlib plugins (e.g. `sys.version`) reference this so they
/// report the Catnip version rather than their own crate version.
///
/// Resolves to the workspace version (`version.workspace = true`) at compile
/// time. Plugins must use this instead of their own `CARGO_PKG_VERSION`, which
/// would diverge if the plugin crate were ever versioned independently.
pub const CATNIP_VERSION: &str = env!("CARGO_PKG_VERSION");
