//! Operator abstractions: error type, host bridge, staged writes, two-phase op.
//!
//! Two-phase contract (D9): `compute` runs the callbacks WITHOUT mutating the
//! operator and returns the output delta plus staged state writes; `commit`
//! applies the writes without running any callback, so it cannot fail. A
//! Catnip exception therefore surfaces at the `push` site with the whole
//! graph observationally unchanged (no half-applied state).

use super::collection::{Delta, DeltaValue};

/// Errors raised while propagating a delta through the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeltaError {
    /// A Catnip callback raised; carries the host-side message. The engine
    /// never propagates the exception VALUE (it lives host-side): the message
    /// is enough to surface at the push site, and the concrete exception type
    /// is rebuilt by the VM wrapper.
    Host(String),
    /// An operator received the wrong number of input deltas (graph wiring bug).
    Arity {
        op: &'static str,
        expected: usize,
        got: usize,
    },
}

/// Bridge to the host runtime: executes a Catnip callable (the `f` of `map f`).
/// The callable is identified by a value `V` (a VMFunc in the real VMs, a key in
/// the toy host); the host resolves and runs it. Must be deterministic for a
/// given delta (purity contract, D8) -- enforced later by the linter.
///
/// `&mut self`: the real host (a VM) mutates its execution state during the
/// call; the D8 purity contract is about the Catnip OBSERVABLE, not about the
/// VM's internals.
pub trait DeltaHost<V: DeltaValue> {
    fn call(&mut self, func: &V, args: &[V]) -> Result<V, DeltaError>;

    /// Truthiness of a host value (the `filter` predicate's verdict). Lives on
    /// the host, not on `V`: `DeltaValue` stays a pure bound alias (blanket
    /// impl preserved), and truthiness is a runtime operation -- the VM host
    /// wires its own `ToBool` here.
    fn truthy(&self, v: &V) -> Result<bool, DeltaError>;
}

/// State writes computed but not yet applied (D9). Bounded enum over the
/// stateful ops (an associated type would not be object-safe). Step 2 needs
/// only the stateless case.
///
/// Each variant belongs to a SINGLE operator family: the graph never feeds a
/// `Staged` produced by one node to another node's `commit`. Receiving a
/// foreign variant is a graph/dispatch bug (programmer error), guarded by
/// `debug_assert!`, never a runtime error branch.
#[derive(Debug)]
pub enum Staged<V: DeltaValue> {
    /// Stateless operator: nothing to commit.
    None,
    // Step 3 adds: Accumulator { updates: Vec<(V, i64)> }, etc.
    #[doc(hidden)]
    _Phantom(std::marker::PhantomData<V>),
}

/// One operator node. Two-phase (D9): `compute` runs callbacks without
/// mutating self and returns the output delta plus the staged state writes;
/// `commit` applies the writes without any callback, so it cannot fail
/// (a `Result` here would reopen a post-callback failure path -- exactly what
/// the two-phase design removes).
pub trait DeltaOp<V: DeltaValue> {
    /// Number of upstream inputs this op consumes (1 for map/filter, n for
    /// concat, 2 for join). The graph validates wiring against it.
    fn arity(&self) -> usize;

    fn compute(&self, inputs: &[Delta<V>], host: &mut dyn DeltaHost<V>) -> Result<(Delta<V>, Staged<V>), DeltaError>;

    fn commit(&mut self, staged: Staged<V>);
}
