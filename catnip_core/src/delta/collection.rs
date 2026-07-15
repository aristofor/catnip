//! Versioned multiset (`Collection`) and signed-diff transitions (`Delta`).
//!
//! The algebra is purely per-key summation: a `Delta` is a compacted set of
//! `(value, diff)` records, a `Collection` is the per-value sum of every diff
//! applied so far. Correctness is the "neutral compaction" property proved in
//! `proof/DeltaCollection.v` (summing diffs per key then dropping zeros does
//! not change any observable sum).

use indexmap::IndexMap;

/// Contract for values flowing through the differential engine.
///
/// `Hash` routes, `Eq` decides: state is indexed by the VALUE (owned), never by
/// its hash alone (collisions). Requiring `Eq` (not just `PartialEq`) is load
/// bearing -- it promises a *reflexive* equivalence (`v == v` for all `v`), which
/// the multiset relies on so that a `-1` cancels its matching `+1`. This is why
/// raw `f64` cannot be a `DeltaValue`: it is only `PartialEq` (NaN != NaN). Any
/// float-carrying instantiation MUST canonicalize NaN and signed zero before
/// producing a `DeltaValue` (one canonical NaN, `-0.0` -> `+0.0`; the concrete
/// canonicalization lives in the VM wrapper, not in this core).
pub trait DeltaValue: Clone + Eq + std::hash::Hash {}

impl<T: Clone + Eq + std::hash::Hash> DeltaValue for T {}

/// A transition: signed-diff records, compacted (one entry per value, no zeros).
///
/// Every public constructor compacts, so a `Delta` is ALWAYS compacted:
/// `len`/`is_empty`/`iter` are exact without rescanning. Compaction preserves
/// first-appearance order (`IndexMap`), so the output is deterministic and
/// stable -- the stability contract sits here (operator output, observed), not
/// on `Collection` (internal state).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Delta<V: DeltaValue> {
    changes: Vec<(V, i64)>,
}

impl<V: DeltaValue> Delta<V> {
    /// The empty delta.
    pub fn new() -> Self {
        Delta { changes: Vec::new() }
    }

    /// Build from raw records, compacting immediately (the only constructor
    /// from raw data: no incremental push exists that could leave a
    /// transiently uncompacted state).
    pub fn from_changes(changes: Vec<(V, i64)>) -> Self {
        let mut d = Delta { changes };
        d.compact();
        d
    }

    /// N-ary union: concatenate every input's records, then compact ONCE
    /// (avoids the quadratic recompaction a fold of binary concats would pay).
    pub fn from_deltas(inputs: &[Delta<V>]) -> Self {
        let total = inputs.iter().map(|d| d.changes.len()).sum();
        let mut changes = Vec::with_capacity(total);
        for d in inputs {
            changes.extend(d.changes.iter().cloned());
        }
        Delta::from_changes(changes)
    }

    /// Sum diffs per value, drop zeros. Idempotent (internal normalization);
    /// kept public but no consumer needs it -- every constructor compacts.
    pub fn compact(&mut self) {
        let mut acc: IndexMap<V, i64> = IndexMap::with_capacity(self.changes.len());
        for (v, c) in self.changes.drain(..) {
            *acc.entry(v).or_insert(0) += c;
        }
        self.changes = acc.into_iter().filter(|(_, c)| *c != 0).collect();
    }

    /// Flip every sign (window retraction, round-trip tests).
    pub fn negate(&self) -> Delta<V> {
        Delta {
            changes: self.changes.iter().map(|(v, c)| (v.clone(), -c)).collect(),
        }
    }

    /// Iterate the compacted records.
    pub fn iter(&self) -> impl Iterator<Item = &(V, i64)> {
        self.changes.iter()
    }

    /// No records (exact: the delta is always compacted).
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Number of records (exact: the delta is always compacted).
    pub fn len(&self) -> usize {
        self.changes.len()
    }
}

/// Versioned multiset: current state is the per-value sum of applied diffs.
/// Invariant: no entry with multiplicity 0 (zeros are pruned on apply).
///
/// The type allows negative multiplicities internally (reserved for the
/// iterative extension); presence (`contains`/`iter`) applies the `> 0`
/// threshold and never exposes them. Multiplicities are raw `i64`: overflow is
/// out of scope (the push count needed to reach `i64::MAX` is not physically
/// realizable), matching the overflow-free `Z` model of the Coq proof.
#[derive(Clone, Debug, Default)]
pub struct Collection<V: DeltaValue> {
    state: IndexMap<V, i64>,
}

impl<V: DeltaValue> Collection<V> {
    /// The empty collection.
    pub fn new() -> Self {
        Collection { state: IndexMap::new() }
    }

    /// Apply a delta; entries whose sum reaches 0 are purged. `swap_remove`
    /// (O(1)) rather than `shift_remove` (O(n)): a `Collection`'s iteration
    /// order is not contractual (stability is `Delta`'s contract).
    pub fn apply(&mut self, delta: &Delta<V>) {
        use indexmap::map::Entry;
        for (v, c) in delta.iter() {
            match self.state.entry(v.clone()) {
                Entry::Occupied(mut e) => {
                    *e.get_mut() += c;
                    if *e.get() == 0 {
                        e.swap_remove();
                    }
                }
                Entry::Vacant(e) => {
                    if *c != 0 {
                        e.insert(*c);
                    }
                }
            }
        }
    }

    /// Current per-value sum (0 when absent).
    pub fn multiplicity(&self, v: &V) -> i64 {
        self.state.get(v).copied().unwrap_or(0)
    }

    /// Present iff the multiplicity is strictly positive.
    pub fn contains(&self, v: &V) -> bool {
        self.multiplicity(v) > 0
    }

    /// Iterate the PRESENT entries (multiplicity > 0; internal negative
    /// multiplicities are never exposed as present).
    pub fn iter(&self) -> impl Iterator<Item = (&V, i64)> {
        self.state.iter().filter(|(_, c)| **c > 0).map(|(v, c)| (v, *c))
    }

    /// Number of present values.
    pub fn distinct_count(&self) -> usize {
        self.iter().count()
    }

    /// No present value.
    pub fn is_empty(&self) -> bool {
        self.distinct_count() == 0
    }

    /// The raw state as a delta from the empty collection: EVERY entry
    /// (`mult != 0`, negatives included), so that applying it to an empty
    /// collection reconstructs the exact state -- unlike `iter`, which only
    /// yields the present ones (`mult > 0`).
    pub fn to_delta(&self) -> Delta<V> {
        Delta::from_changes(self.state.iter().map(|(v, c)| (v.clone(), *c)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(changes: &[(&str, i64)]) -> Delta<String> {
        Delta::from_changes(changes.iter().map(|(v, c)| (v.to_string(), *c)).collect())
    }

    fn entries(delta: &Delta<String>) -> Vec<(String, i64)> {
        delta.iter().cloned().collect()
    }

    #[test]
    fn compaction_sums_per_key() {
        let delta = d(&[("a", 1), ("a", 1), ("a", -1)]);
        assert_eq!(entries(&delta), vec![("a".to_string(), 1)]);
    }

    #[test]
    fn compaction_keeps_first_appearance_order() {
        let delta = d(&[("b", 1), ("a", 1), ("b", 1)]);
        assert_eq!(entries(&delta), vec![("b".to_string(), 2), ("a".to_string(), 1)]);
    }

    #[test]
    fn compaction_drops_zeros() {
        let delta = d(&[("a", 1), ("a", -1)]);
        assert!(delta.is_empty());
        assert_eq!(delta.len(), 0);
    }

    #[test]
    fn compact_is_idempotent() {
        let mut delta = d(&[("b", 1), ("a", 2), ("b", -3)]);
        let once = entries(&delta);
        delta.compact();
        // IndexMap order is deterministic, so structural equality holds here
        // (the Coq theorem is the per-key extensional version).
        assert_eq!(entries(&delta), once);
    }

    #[test]
    fn apply_purges_zero_crossings() {
        let mut c: Collection<i64> = Collection::new();
        c.apply(&Delta::from_changes(vec![(7, 1)]));
        assert!(c.contains(&7));
        c.apply(&Delta::from_changes(vec![(7, -1)]));
        assert!(!c.contains(&7));
        assert!(c.is_empty());
        assert_eq!(c.multiplicity(&7), 0);
    }

    #[test]
    fn multiplicities_accumulate_and_retract() {
        let mut c: Collection<i64> = Collection::new();
        c.apply(&Delta::from_changes(vec![(1, 3), (2, 1)]));
        assert_eq!(c.multiplicity(&1), 3);
        assert_eq!(c.distinct_count(), 2);
        c.apply(&Delta::from_changes(vec![(1, -3)]));
        assert!(!c.contains(&1));
        assert!(c.contains(&2));
        assert_eq!(c.distinct_count(), 1);
    }

    #[test]
    fn empty_delta_is_neutral() {
        let mut c: Collection<i64> = Collection::new();
        c.apply(&Delta::from_changes(vec![(1, 2)]));
        let before: Vec<(i64, i64)> = c.iter().map(|(v, m)| (*v, m)).collect();
        c.apply(&Delta::new());
        let after: Vec<(i64, i64)> = c.iter().map(|(v, m)| (*v, m)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn from_deltas_unions_with_one_compaction() {
        let inputs = [d(&[("a", 1)]), d(&[("a", 1), ("b", 1)]), d(&[("b", -1)])];
        let union = Delta::from_deltas(&inputs);
        assert_eq!(entries(&union), vec![("a".to_string(), 2)]);
    }

    #[test]
    fn negate_flips_signs_and_round_trips() {
        let delta = d(&[("a", 2), ("b", -1)]);
        assert_eq!(
            entries(&delta.negate()),
            vec![("a".to_string(), -2), ("b".to_string(), 1)]
        );
        let mut c: Collection<String> = Collection::new();
        c.apply(&d(&[("x", 5)]));
        let snapshot: Vec<(String, i64)> = c.iter().map(|(v, m)| (v.clone(), m)).collect();
        c.apply(&delta);
        c.apply(&delta.negate());
        let back: Vec<(String, i64)> = c.iter().map(|(v, m)| (v.clone(), m)).collect();
        assert_eq!(snapshot, back);
    }

    #[test]
    fn to_delta_reconstructs_state() {
        let mut c: Collection<i64> = Collection::new();
        c.apply(&Delta::from_changes(vec![(1, 3), (2, 1), (3, 2)]));
        c.apply(&Delta::from_changes(vec![(3, -2)]));
        let mut rebuilt: Collection<i64> = Collection::new();
        rebuilt.apply(&c.to_delta());
        assert_eq!(rebuilt.multiplicity(&1), 3);
        assert_eq!(rebuilt.multiplicity(&2), 1);
        assert_eq!(rebuilt.multiplicity(&3), 0);
        assert_eq!(rebuilt.distinct_count(), c.distinct_count());
    }
}
