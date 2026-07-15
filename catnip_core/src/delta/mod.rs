//! Differential dataflow core: versioned multiset and signed-diff transitions.
//!
//! Generic over the value type (no VM dependency); see `wip/DELTA.md`.
//! Step 1 (this module): the collection model. Operators and the DAG live in
//! sibling modules added by later steps.

pub mod collection;
pub mod op;
pub mod ops_stateless;

pub use collection::{Collection, Delta, DeltaValue};
pub use op::{DeltaError, DeltaHost, DeltaOp, Staged};
pub use ops_stateless::{Concat, Filter, Map};
