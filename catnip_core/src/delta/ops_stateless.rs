//! Stateless operators: map, filter, concat.
//!
//! A stateless operator is fully characterized by "the output delta depends
//! only on the input delta" -- no indexed state, so `commit` is a no-op and
//! `Staged` stays `None`. Every operator emits through `Delta::from_changes`
//! (or `from_deltas`), so the output is systematically compacted: one output
//! shape for all operators, no per-operator reasoning about compactness.

use super::collection::{Delta, DeltaValue};
use super::op::{DeltaError, DeltaHost, DeltaOp, Staged};

/// Unary-op input guard: exactly one upstream delta.
fn single<'a, V: DeltaValue>(inputs: &'a [Delta<V>], op: &'static str) -> Result<&'a Delta<V>, DeltaError> {
    match inputs {
        [d] => Ok(d),
        _ => Err(DeltaError::Arity {
            op,
            expected: 1,
            got: inputs.len(),
        }),
    }
}

/// `map f`: transform every record's value, transporting sign and
/// multiplicity as is. `f` can collide (`f(a) == f(b)`), so the output goes
/// through `from_changes` (compaction merges collided keys).
pub struct Map<V: DeltaValue> {
    func: V,
}

impl<V: DeltaValue> Map<V> {
    pub fn new(func: V) -> Self {
        Map { func }
    }
}

impl<V: DeltaValue> DeltaOp<V> for Map<V> {
    fn arity(&self) -> usize {
        1
    }

    fn compute(&self, inputs: &[Delta<V>], host: &mut dyn DeltaHost<V>) -> Result<(Delta<V>, Staged<V>), DeltaError> {
        let input = single(inputs, "map")?;
        let mut out = Vec::with_capacity(input.len());
        for (v, c) in input.iter() {
            out.push((host.call(&self.func, std::slice::from_ref(v))?, *c));
        }
        Ok((Delta::from_changes(out), Staged::None))
    }

    fn commit(&mut self, staged: Staged<V>) {
        debug_assert!(matches!(staged, Staged::None));
    }
}

/// `filter p`: keep the records whose value satisfies the predicate,
/// diffs untouched (a subset of the input -- no collision possible, the
/// output compaction is the identity, applied for uniformity).
pub struct Filter<V: DeltaValue> {
    func: V,
}

impl<V: DeltaValue> Filter<V> {
    pub fn new(func: V) -> Self {
        Filter { func }
    }
}

impl<V: DeltaValue> DeltaOp<V> for Filter<V> {
    fn arity(&self) -> usize {
        1
    }

    fn compute(&self, inputs: &[Delta<V>], host: &mut dyn DeltaHost<V>) -> Result<(Delta<V>, Staged<V>), DeltaError> {
        let input = single(inputs, "filter")?;
        let mut out = Vec::new();
        for (v, c) in input.iter() {
            let verdict = host.call(&self.func, std::slice::from_ref(v))?;
            if host.truthy(&verdict)? {
                out.push((v.clone(), *c));
            }
        }
        Ok((Delta::from_changes(out), Staged::None))
    }

    fn commit(&mut self, staged: Staged<V>) {
        debug_assert!(matches!(staged, Staged::None));
    }
}

/// `concat`: n-ary multiset union of the upstream deltas. No callback. A
/// record present in two flows gets its multiplicities added (multiset, not
/// set semantics); `from_deltas` compacts ONCE over the concatenation.
pub struct Concat {
    n: usize,
}

impl Concat {
    pub fn new(n: usize) -> Self {
        Concat { n }
    }
}

impl<V: DeltaValue> DeltaOp<V> for Concat {
    fn arity(&self) -> usize {
        self.n
    }

    fn compute(&self, inputs: &[Delta<V>], _host: &mut dyn DeltaHost<V>) -> Result<(Delta<V>, Staged<V>), DeltaError> {
        if inputs.len() != self.n {
            return Err(DeltaError::Arity {
                op: "concat",
                expected: self.n,
                got: inputs.len(),
            });
        }
        Ok((Delta::from_deltas(inputs), Staged::None))
    }

    fn commit(&mut self, staged: Staged<V>) {
        debug_assert!(matches!(staged, Staged::None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    type ToyFn = fn(&str) -> Result<String, DeltaError>;

    /// Toy host on `String`: callbacks are named Rust functions, mirroring the
    /// VM model (func = a VMFunc value the host resolves and runs). Truthiness
    /// is "the string is not empty and not '0'/'false'".
    struct ToyHost {
        funcs: HashMap<String, ToyFn>,
    }

    impl ToyHost {
        fn new() -> Self {
            let mut funcs: HashMap<String, ToyFn> = HashMap::new();
            funcs.insert("inc".to_string(), |s| Ok((s.parse::<i64>().unwrap() + 1).to_string()));
            funcs.insert("const0".to_string(), |_| Ok("0".to_string()));
            funcs.insert("is_even".to_string(), |s| {
                Ok((s.parse::<i64>().unwrap() % 2 == 0).to_string())
            });
            funcs.insert("boom".to_string(), |_| Err(DeltaError::Host("boom".to_string())));
            ToyHost { funcs }
        }
    }

    impl DeltaHost<String> for ToyHost {
        fn call(&mut self, func: &String, args: &[String]) -> Result<String, DeltaError> {
            let f = self
                .funcs
                .get(func)
                .ok_or_else(|| DeltaError::Host(format!("unknown fn {func}")))?;
            f(&args[0])
        }

        fn truthy(&self, v: &String) -> Result<bool, DeltaError> {
            Ok(!v.is_empty() && v != "0" && v != "false")
        }
    }

    fn d(changes: &[(&str, i64)]) -> Delta<String> {
        Delta::from_changes(changes.iter().map(|(v, c)| (v.to_string(), *c)).collect())
    }

    fn entries(delta: &Delta<String>) -> Vec<(String, i64)> {
        delta.iter().cloned().collect()
    }

    #[test]
    fn map_transports_sign_and_multiplicity() {
        let mut host = ToyHost::new();
        let op = Map::new("inc".to_string());
        let (out, _) = op.compute(&[d(&[("2", 3), ("5", -1)])], &mut host).unwrap();
        assert_eq!(entries(&out), vec![("3".to_string(), 3), ("6".to_string(), -1)]);
    }

    #[test]
    fn map_collision_compacts_output() {
        let mut host = ToyHost::new();
        let op = Map::new("const0".to_string());
        let (out, _) = op.compute(&[d(&[("a", 1), ("b", 1)])], &mut host).unwrap();
        assert_eq!(entries(&out), vec![("0".to_string(), 2)]);
    }

    #[test]
    fn filter_keeps_subset_with_diffs() {
        let mut host = ToyHost::new();
        let op = Filter::new("is_even".to_string());
        let (out, _) = op.compute(&[d(&[("1", 1), ("2", 1), ("3", 1)])], &mut host).unwrap();
        assert_eq!(entries(&out), vec![("2".to_string(), 1)]);
        let (out, _) = op.compute(&[d(&[("2", 3), ("4", -2), ("1", 1)])], &mut host).unwrap();
        assert_eq!(entries(&out), vec![("2".to_string(), 3), ("4".to_string(), -2)]);
    }

    #[test]
    fn concat_is_multiset_union() {
        let mut host = ToyHost::new();
        let op = Concat::new(2);
        let (out, _) = op
            .compute(&[d(&[("a", 1)]), d(&[("a", 1), ("b", 1)])], &mut host)
            .unwrap();
        assert_eq!(entries(&out), vec![("a".to_string(), 2), ("b".to_string(), 1)]);
    }

    #[test]
    fn arity_guards_reject_wrong_wiring() {
        let mut host = ToyHost::new();
        let map = Map::new("inc".to_string());
        assert!(matches!(
            map.compute(&[], &mut host),
            Err(DeltaError::Arity {
                op: "map",
                expected: 1,
                got: 0
            })
        ));
        assert!(matches!(
            map.compute(&[d(&[]), d(&[])], &mut host),
            Err(DeltaError::Arity {
                op: "map",
                expected: 1,
                got: 2
            })
        ));
        let filter = Filter::new("is_even".to_string());
        assert!(matches!(
            filter.compute(&[], &mut host),
            Err(DeltaError::Arity { op: "filter", .. })
        ));
        let concat = Concat::new(2);
        assert!(matches!(
            <Concat as DeltaOp<String>>::compute(&concat, &[d(&[])], &mut host),
            Err(DeltaError::Arity {
                op: "concat",
                expected: 2,
                got: 1
            })
        ));
        assert!(matches!(
            <Concat as DeltaOp<String>>::compute(&concat, &[d(&[]), d(&[]), d(&[])], &mut host),
            Err(DeltaError::Arity {
                op: "concat",
                expected: 2,
                got: 3
            })
        ));
    }

    #[test]
    fn host_error_surfaces_without_partial_output() {
        let mut host = ToyHost::new();
        let op = Map::new("boom".to_string());
        let r = op.compute(&[d(&[("1", 1), ("2", 1)])], &mut host);
        assert_eq!(r.err(), Some(DeltaError::Host("boom".to_string())));
    }

    #[test]
    fn stateless_commit_is_a_noop() {
        let mut op = Map::new("inc".to_string());
        op.commit(Staged::None);
        let mut filter = Filter::new("is_even".to_string());
        filter.commit(Staged::None);
        let mut concat = Concat::new(1);
        <Concat as DeltaOp<String>>::commit(&mut concat, Staged::None);
    }

    #[test]
    fn map_homomorphism_on_a_concrete_case() {
        // map f (C after D) == (map f C) after (map f D): the bridge to the
        // Coq theorem, checked empirically on one case with a collision.
        use crate::delta::Collection;
        let mut host = ToyHost::new();
        let op = Map::new("const0".to_string());

        // State route: build C, apply D, then map the resulting state.
        let d0 = d(&[("a", 2), ("b", 1)]);
        let dd = d(&[("b", 1), ("c", 3)]);
        let mut c: Collection<String> = Collection::new();
        c.apply(&d0);
        c.apply(&dd);
        let state_delta = c.to_delta();
        let (mapped_state, _) = op.compute(&[state_delta], &mut host).unwrap();

        // Delta route: map C's raw delta and D separately, apply both.
        let (mapped_c, _) = op.compute(&[d0], &mut host).unwrap();
        let (mapped_d, _) = op.compute(&[dd], &mut host).unwrap();
        let mut via_deltas: Collection<String> = Collection::new();
        via_deltas.apply(&mapped_c);
        via_deltas.apply(&mapped_d);

        let mut via_state: Collection<String> = Collection::new();
        via_state.apply(&mapped_state);
        assert_eq!(
            via_state.multiplicity(&"0".to_string()),
            via_deltas.multiplicity(&"0".to_string())
        );
        assert_eq!(via_state.distinct_count(), via_deltas.distinct_count());
    }
}
