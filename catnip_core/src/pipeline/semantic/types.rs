// FILE: catnip_core/src/pipeline/semantic/types.rs
//! Finite flat type lattice for local flow-sensitive inference (TH3 step 1).
//!
//! Height-2 lattice: `Bottom` < {concrete types} < `Top`. The concrete layer is
//! the small, finite TH4 set (primitives + nominals); there is no subtyping
//! (`join(Int, Float) = Top`), so the least upper bound is trivial and any merge
//! fixpoint settles in at most two passes. Composites carry their parameters:
//! `list[T]` is `List(T)`, `set[T]` is `Set(T)`, `dict[K, V]` is `Dict(K, V)`
//! (a bare composite carries `Top`), with covariant or invariant parameter
//! checking via `accepts`/`accepts_value`. `tuple[T0, T1, ...]` is
//! `Tuple(Some([..]))`, positional and heterogeneous (one type per position,
//! arity part of the contract); a bare `tuple` is `Tuple(None)` (unknown arity).
//! Generic nominal unions are `Union(name, args)`: `Option[int]` is
//! `Union("Option", [Int])`, a bare `Option` is `Union("Option", [])` (unknown
//! arity, defers); covariant in its arguments (a union is immutable).
//!
//! Representation convention used by the analyzer: a variable's binding in
//! `var_types` always holds a *concrete* `Ty`. `Top` (unknown / conflicting) is
//! encoded as the *absence* of the binding, and `Bottom` only ever appears as
//! the seed of a join fold. So `join_states` inserts a key only when the merged
//! result is concrete, and drops it otherwise.

use std::collections::HashMap;

/// A type in the flat lattice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Int,
    Float,
    Str,
    Bool,
    NoneT,
    /// List, parameterized by element type: `list[T]` carries `T`, a bare `list`
    /// carries `Top` (element unknown). The container itself is always concrete.
    List(Box<Ty>),
    /// Set, parameterized by element type: `set[T]` carries `T`, a bare `set`
    /// carries `Top`. Homogeneous like `List`, but a distinct container -- a set
    /// is never assignable to a list nor the reverse.
    Set(Box<Ty>),
    /// Dict, parameterized by key and value types: `dict[K, V]` carries `K`/`V`,
    /// a bare `dict` carries `Top`/`Top`. The container is always concrete.
    Dict(Box<Ty>, Box<Ty>),
    /// Tuple, a positional heterogeneous composite: `Some([T0, T1, ...])` carries
    /// one type per position and a *known* arity (the empty literal `()` is
    /// `Some([])`, a genuine arity-0 tuple); `None` is an *unknown* arity -- a bare
    /// `tuple` annotation or a join of disjoint arities. The arity is part of the
    /// contract, unlike the homogeneous `List`/`Set`. The container itself is
    /// always concrete. Keeping the unknown-arity sentinel (`None`) outside the
    /// value space -- rather than overloading the empty vector -- lets a provable
    /// arity mismatch against a known-empty tuple be rejected statically (TH6),
    /// while a genuinely unknown arity still defers to the runtime boundary.
    Tuple(Option<Vec<Ty>>),
    /// Plain (C-like) enum, by name.
    Enum(String),
    /// Tagged union (ADT), by name and type arguments. A bare `Option` carries an
    /// empty argument vector (arity unknown -- it defers, like a bare `tuple`);
    /// `Option[int]` carries `[Int]`. A unified representation for the nominal and
    /// the parameterized form: an empty vector on either side of a same-name
    /// comparison is never a provable mismatch. Covariant in its arguments (a
    /// union is immutable -- no mutator -- so there is no alias-mutation hazard,
    /// like `Tuple`); an argument may be `Top` (a known arity with an unknown
    /// parameter type, distinct from the empty vector's unknown arity).
    Union(String, Vec<Ty>),
    /// Struct (nominal record), by name.
    Struct(String),
    /// Union of types (`int | str`, `Point | None`). A declared-only contract:
    /// `infer_type` never produces one in v1, so it flows as a slot type, not a
    /// value type. Members are concrete and deduplicated.
    OneOf(Vec<Ty>),
    /// Function type (`(int, str) -> bool`): parameter types and return type.
    /// The arity is part of the contract (fixed, no variadic form in v1). A
    /// parameter or the return may be `Top` (unannotated lambda parameter,
    /// unknown return) -- never a provable mismatch, like composite parameters.
    /// This is the TH3 answer to first-class functions: a callback's type is
    /// *declared*, not inferred (no higher-order CFA), and the declaration is
    /// what lets inference continue through `cb(x)` in the consuming body.
    Fn(Vec<Ty>, Box<Ty>),
    /// Lattice top: type unknown, or several incompatible types reach here.
    Top,
    /// Lattice bottom: no value flows here yet (join identity).
    Bottom,
}

impl Ty {
    /// A type is concrete when it names an actual type, i.e. neither lattice
    /// endpoint. Verification only fires between two concrete types (TH2-A:
    /// errors are reported only when the mismatch is provable).
    pub fn is_concrete(&self) -> bool {
        !matches!(self, Ty::Top | Ty::Bottom)
    }

    /// Least upper bound on the flat lattice.
    ///
    /// `Bottom` is the identity (a path that produces no value does not
    /// constrain the merge); `Top` absorbs; two equal concrete types join to
    /// themselves; any other disagreement widens to `Top`.
    pub fn join(&self, other: &Ty) -> Ty {
        match (self, other) {
            (Ty::Bottom, t) | (t, Ty::Bottom) => t.clone(),
            (Ty::Top, _) | (_, Ty::Top) => Ty::Top,
            (a, b) if a == b => a.clone(),
            // Composites join element-wise: two lists/dicts join to a list/dict of
            // the joined parameters (disjoint parameters widen to `Top`, i.e. an
            // unknown element), keeping the container fact rather than discarding it.
            (Ty::List(a), Ty::List(b)) => Ty::List(Box::new(a.join(b))),
            (Ty::Set(a), Ty::Set(b)) => Ty::Set(Box::new(a.join(b))),
            (Ty::Dict(ka, va), Ty::Dict(kb, vb)) => Ty::Dict(Box::new(ka.join(kb)), Box::new(va.join(vb))),
            // Two known same-arity tuples join position-wise. A disjoint arity (or
            // either side already unknown) can't be aligned, so it widens to the
            // unknown-arity tuple (`None`), keeping the container fact rather than
            // discarding it (mirrors the list arm widening a disjoint element to
            // `Top`).
            (Ty::Tuple(Some(a)), Ty::Tuple(Some(b))) if a.len() == b.len() => {
                Ty::Tuple(Some(a.iter().zip(b).map(|(x, y)| x.join(y)).collect()))
            }
            (Ty::Tuple(_), Ty::Tuple(_)) => Ty::Tuple(None),
            // Same-name unions join argument-wise when the arities agree (a disjoint
            // argument widens to `Top`, an unknown parameter). Disjoint arities can't
            // be aligned, so they widen to the bare union (empty arguments, unknown
            // arity) -- keeping the union-name fact, mirroring the tuple arm. Distinct
            // names have no common union, so they fall through to `Top`.
            (Ty::Union(na, aa), Ty::Union(nb, ba)) if na == nb && aa.len() == ba.len() => {
                Ty::Union(na.clone(), aa.iter().zip(ba).map(|(x, y)| x.join(y)).collect())
            }
            (Ty::Union(na, _), Ty::Union(nb, _)) if na == nb => Ty::Union(na.clone(), Vec::new()),
            // Two same-arity function types join component-wise (a disjoint
            // component widens to `Top`, an unknown parameter/return). This is
            // sound for what the analyzer reads off a `Fn`: the arity check
            // stays exact, and a widened component simply stops constraining
            // (the callee's own boundary still enforces its real types).
            // Disjoint arities have no common function shape: `Top`.
            (Ty::Fn(pa, ra), Ty::Fn(pb, rb)) if pa.len() == pb.len() => Ty::Fn(
                pa.iter().zip(pb).map(|(x, y)| x.join(y)).collect(),
                Box::new(ra.join(rb)),
            ),
            _ => Ty::Top,
        }
    }

    /// Assignability: does a value of type `value` satisfy a `self`-typed slot?
    ///
    /// This is the relation that drives E300 (a mismatch is reported only when
    /// the slot does *not* accept the value), kept separate from the lattice
    /// `join`: the lattice is flat with no subtyping, but the type *contract*
    /// follows the PEP 484 numeric tower -- `bool` is a subtype of `int`, and an
    /// `int` is acceptable where a `float` is declared. Without this, a `float`
    /// param fed an `int` literal (a standard widening) would be rejected, which
    /// is a false positive now that E300 is fatal. Narrowing (`float` into an
    /// `int` slot) is *not* accepted, matching the tower's one-way direction.
    pub fn accepts(&self, value: &Ty) -> bool {
        if self == value {
            return true;
        }
        match (self, value) {
            // Numeric tower: bool <: int <: float.
            (Ty::Float, Ty::Int) | (Ty::Float, Ty::Bool) | (Ty::Int, Ty::Bool) => true,
            // Composites are covariant in their parameters. A non-concrete
            // parameter on either side (`list` with unknown element, or a value
            // whose element type couldn't be inferred) is not a provable mismatch,
            // so it is accepted (TH2-A). Invariance for mutation soundness is a
            // separate decision (variance, deferred).
            (Ty::List(u), Ty::List(v)) => param_accepts(u, v),
            (Ty::Set(u), Ty::Set(v)) => param_accepts(u, v),
            (Ty::Dict(ku, vu), Ty::Dict(kv, vv)) => param_accepts(ku, kv) && param_accepts(vu, vv),
            // A tuple is positional: an unknown-arity slot (bare `tuple`, `None`)
            // accepts any tuple, and an unknown-arity value is not a provable
            // mismatch (deferred to the runtime boundary). Two *known* arities must
            // match exactly and each position accept covariantly -- so a known
            // arity-0 `()` against a fixed-arity slot is a provable mismatch (TH6),
            // no longer silently deferred.
            (Ty::Tuple(None), Ty::Tuple(_)) | (Ty::Tuple(_), Ty::Tuple(None)) => true,
            (Ty::Tuple(Some(us)), Ty::Tuple(Some(vs))) => {
                us.len() == vs.len() && us.iter().zip(vs).all(|(u, v)| param_accepts(u, v))
            }
            // A parameterized union is covariant in its arguments (immutable, like a
            // tuple). The names must match; a bare slot (empty arguments) accepts any
            // same-name union, and a bare *value* (empty arguments, unknown arity) is
            // never a provable mismatch -- both defer to the runtime boundary. Two
            // known arities must match and each argument accept covariantly (numeric
            // tower via `param_accepts`), so `Option[str]` into `Option[int]` is a
            // provable mismatch, while `Option[int]` into a bare `Option` is accepted.
            (Ty::Union(ns, sa), Ty::Union(nv, va)) => {
                ns == nv
                    && (sa.is_empty()
                        || va.is_empty()
                        || (sa.len() == va.len() && sa.iter().zip(va).all(|(u, v)| param_accepts(u, v))))
            }
            // Function types: the arity is exact, parameters are CONTRAvariant
            // (a callback declared to take `float` serves a slot expecting an
            // `int`-taker: it will happily receive ints) and the return is
            // covariant. An unknown component on either side (`Top`: an
            // unannotated lambda parameter, an unknown return) is never a
            // provable mismatch (TH2-A), via the same `param_accepts` rule as
            // composites -- note the swapped operands on the parameter side.
            (Ty::Fn(sp, sr), Ty::Fn(vp, vr)) => {
                sp.len() == vp.len() && sp.iter().zip(vp).all(|(s, v)| param_accepts(v, s)) && param_accepts(sr, vr)
            }
            // A union *value* is assignable only if every member it could hold is
            // (robustness: `infer_type` produces no union in v1, so this is unused
            // for now). Tested before the slot-union arm so `OneOf vs OneOf` here.
            (_, Ty::OneOf(vs)) => vs.iter().all(|v| self.accepts(v)),
            // A union *slot* accepts a value assignable to any one of its members.
            (Ty::OneOf(ms), _) => ms.iter().any(|m| m.accepts(value)),
            _ => false,
        }
    }

    /// Assignability with explicit container variance, for the typed-boundary
    /// check. `covariant` is set when the value is a freshly built composite (a
    /// literal): its parameters then follow the numeric tower (a `list[int]`
    /// literal satisfies `list[float]`). For an already-typed value (a variable,
    /// a call result) `covariant` is false and the parameters of a *mutable*
    /// composite (`list`/`set`/`dict`) are invariant (a `list[int]` variable does
    /// not satisfy `list[float]` -- a mutation through the alias would be unsound).
    /// A `tuple` is immutable, so it carries no alias-mutation hazard and stays
    /// covariant regardless of `covariant` (PEP 484's covariant `Tuple`); it has
    /// no arm here and falls through to `accepts`. Non-composite types ignore
    /// `covariant` (their `accepts` relation carries no container variance);
    /// unknown parameters (`Top`) are never a provable mismatch.
    pub fn accepts_value(&self, value: &Ty, covariant: bool) -> bool {
        if self == value {
            return true;
        }
        match (self, value) {
            (Ty::List(u), Ty::List(v)) => param_accepts_v(u, v, covariant),
            (Ty::Set(u), Ty::Set(v)) => param_accepts_v(u, v, covariant),
            (Ty::Dict(ku, vu), Ty::Dict(kv, vv)) => {
                param_accepts_v(ku, kv, covariant) && param_accepts_v(vu, vv, covariant)
            }
            // No `Tuple` or `Union` arm: both immutable -> covariant -> fall through
            // to `accepts` (a union carries no alias-mutation hazard, so its arguments
            // stay covariant regardless of `covariant`, like `Tuple`).
            (_, Ty::OneOf(vs)) => vs.iter().all(|v| self.accepts_value(v, covariant)),
            (Ty::OneOf(ms), _) => ms.iter().any(|m| m.accepts_value(value, covariant)),
            _ => self.accepts(value),
        }
    }
}

/// Accept relation for one composite type parameter (covariant). A provable
/// mismatch requires both sides concrete and the slot not accepting the value;
/// an unknown parameter (`Top` on either side -- a bare `list` slot, or a value
/// whose element type couldn't be inferred) is never a provable mismatch, so it
/// is accepted. Mirrors the call-site rule "verify only between concrete types".
fn param_accepts(slot: &Ty, value: &Ty) -> bool {
    !slot.is_concrete() || !value.is_concrete() || slot.accepts(value)
}

/// Accept relation for one composite parameter with explicit variance (used by
/// [`Ty::accepts_value`]). An unknown parameter (`Top` on either side) is never a
/// provable mismatch. A covariant parameter (the value is a fresh literal) follows
/// the numeric tower recursively; an invariant parameter (the value is already
/// typed) demands structural equivalence -- tolerant of unknown components
/// (see [`invariant_compatible`]).
fn param_accepts_v(slot: &Ty, value: &Ty, covariant: bool) -> bool {
    if !slot.is_concrete() || !value.is_concrete() {
        return true;
    }
    if covariant {
        slot.accepts_value(value, true)
    } else {
        invariant_compatible(slot, value)
    }
}

/// Structural equivalence for the invariant path, tolerant of unknowns: a
/// `Top` component on either side is never a provable mismatch (TH2-A), so it
/// is compatible; concrete components must match recursively. Plain equality
/// here was a fatal false rejection: `Fn([Int], Top)` (an unannotated lambda's
/// inferred type) passed through a variable never equals the declared
/// `Fn([Int], Int)`, and a `List(Top)` (empty-literal element) never equals
/// `List(Int)`, even though neither mismatch is provable.
fn invariant_compatible(slot: &Ty, value: &Ty) -> bool {
    if !slot.is_concrete() || !value.is_concrete() {
        return true;
    }
    match (slot, value) {
        (Ty::List(a), Ty::List(b)) | (Ty::Set(a), Ty::Set(b)) => invariant_compatible(a, b),
        (Ty::Dict(ka, va), Ty::Dict(kb, vb)) => invariant_compatible(ka, kb) && invariant_compatible(va, vb),
        (Ty::Tuple(None), Ty::Tuple(_)) | (Ty::Tuple(_), Ty::Tuple(None)) => true,
        (Ty::Tuple(Some(a)), Ty::Tuple(Some(b))) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| invariant_compatible(x, y))
        }
        (Ty::Union(na, aa), Ty::Union(nb, ab)) => {
            na == nb
                && (aa.is_empty()
                    || ab.is_empty()
                    || (aa.len() == ab.len() && aa.iter().zip(ab).all(|(x, y)| invariant_compatible(x, y))))
        }
        (Ty::Fn(pa, ra), Ty::Fn(pb, rb)) => {
            pa.len() == pb.len()
                && pa.iter().zip(pb).all(|(x, y)| invariant_compatible(x, y))
                && invariant_compatible(ra, rb)
        }
        _ => slot == value,
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ty::Int => f.write_str("int"),
            Ty::Float => f.write_str("float"),
            Ty::Str => f.write_str("str"),
            Ty::Bool => f.write_str("bool"),
            Ty::NoneT => f.write_str("None"),
            Ty::List(e) => {
                if e.is_concrete() {
                    write!(f, "list[{e}]")
                } else {
                    f.write_str("list")
                }
            }
            Ty::Set(e) => {
                if e.is_concrete() {
                    write!(f, "set[{e}]")
                } else {
                    f.write_str("set")
                }
            }
            Ty::Dict(k, v) => {
                if k.is_concrete() || v.is_concrete() {
                    write!(f, "dict[{k}, {v}]")
                } else {
                    f.write_str("dict")
                }
            }
            Ty::Tuple(None) => f.write_str("tuple"),
            Ty::Tuple(Some(ps)) => {
                // A known arity, including the empty tuple `()` -> `tuple[]`, which
                // a reader distinguishes from the unknown-arity bare `tuple`.
                let parts: Vec<String> = ps.iter().map(|p| p.to_string()).collect();
                write!(f, "tuple[{}]", parts.join(", "))
            }
            Ty::Enum(n) | Ty::Struct(n) => f.write_str(n),
            Ty::Union(n, args) => {
                if args.is_empty() {
                    f.write_str(n)
                } else {
                    let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                    write!(f, "{n}[{}]", parts.join(", "))
                }
            }
            Ty::OneOf(tys) => {
                let parts: Vec<String> = tys.iter().map(|t| t.to_string()).collect();
                f.write_str(&parts.join(" | "))
            }
            Ty::Fn(params, ret) => {
                let parts: Vec<String> = params.iter().map(|p| p.to_string()).collect();
                write!(f, "({}) -> {ret}", parts.join(", "))
            }
            Ty::Top => f.write_str("?"),
            Ty::Bottom => f.write_str("!"),
        }
    }
}

/// Merge per-branch exit states at a control-flow join point.
///
/// A key absent from a given state means that state knows nothing about it
/// (`Top` for that variable on that path), so a binding survives only when
/// every merged state agrees on the same concrete type. The result holds only
/// concrete bindings: `Top`/`Bottom` outcomes are dropped (encoded as absence).
pub fn join_states(states: &[HashMap<String, Ty>]) -> HashMap<String, Ty> {
    let mut result = HashMap::new();
    if states.is_empty() {
        return result;
    }
    let mut keys: Vec<&String> = states.iter().flat_map(|s| s.keys()).collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let mut acc = Ty::Bottom;
        for state in states {
            match state.get(key) {
                Some(t) => acc = acc.join(t),
                None => acc = acc.join(&Ty::Top),
            }
            if acc == Ty::Top {
                break;
            }
        }
        if acc != Ty::Top && acc != Ty::Bottom {
            result.insert(key.clone(), acc);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_equal_is_identity() {
        assert_eq!(Ty::Int.join(&Ty::Int), Ty::Int);
        assert_eq!(
            Ty::Enum("Color".into()).join(&Ty::Enum("Color".into())),
            Ty::Enum("Color".into())
        );
    }

    #[test]
    fn join_disagreement_widens_to_top() {
        assert_eq!(Ty::Int.join(&Ty::Float), Ty::Top);
        assert_eq!(Ty::Int.join(&Ty::Enum("Color".into())), Ty::Top);
        assert_eq!(Ty::Enum("Color".into()).join(&Ty::Enum("Suit".into())), Ty::Top);
    }

    #[test]
    fn bottom_is_join_identity_and_top_absorbs() {
        assert_eq!(Ty::Bottom.join(&Ty::Str), Ty::Str);
        assert_eq!(Ty::Str.join(&Ty::Bottom), Ty::Str);
        assert_eq!(Ty::Top.join(&Ty::Int), Ty::Top);
        assert_eq!(Ty::Int.join(&Ty::Top), Ty::Top);
    }

    #[test]
    fn join_states_keeps_agreeing_concrete_binding() {
        let a = HashMap::from([("x".to_string(), Ty::Int)]);
        let b = HashMap::from([("x".to_string(), Ty::Int)]);
        let merged = join_states(&[a, b]);
        assert_eq!(merged.get("x"), Some(&Ty::Int));
    }

    #[test]
    fn join_states_drops_conflicting_binding() {
        let a = HashMap::from([("x".to_string(), Ty::Int)]);
        let b = HashMap::from([("x".to_string(), Ty::Enum("Color".into()))]);
        assert!(!join_states(&[a, b]).contains_key("x"));
    }

    #[test]
    fn accepts_is_reflexive() {
        assert!(Ty::Int.accepts(&Ty::Int));
        assert!(Ty::Str.accepts(&Ty::Str));
        assert!(Ty::Struct("P".into()).accepts(&Ty::Struct("P".into())));
    }

    #[test]
    fn accepts_follows_numeric_tower() {
        // bool <: int <: float (PEP 484): widening is accepted.
        assert!(Ty::Float.accepts(&Ty::Int));
        assert!(Ty::Float.accepts(&Ty::Bool));
        assert!(Ty::Int.accepts(&Ty::Bool));
    }

    #[test]
    fn accepts_rejects_narrowing_and_disjoint() {
        // Narrowing is one-way: float into an int slot is refused.
        assert!(!Ty::Int.accepts(&Ty::Float));
        assert!(!Ty::Bool.accepts(&Ty::Int));
        // Disjoint primitives and distinct nominals never match.
        assert!(!Ty::Int.accepts(&Ty::Str));
        assert!(!Ty::Struct("A".into()).accepts(&Ty::Struct("B".into())));
    }

    #[test]
    fn join_states_drops_binding_missing_from_one_branch() {
        // Present in one path, unknown (absent = Top) in the other -> dropped.
        let a = HashMap::from([("x".to_string(), Ty::Int)]);
        let b = HashMap::new();
        assert!(!join_states(&[a, b]).contains_key("x"));
    }

    #[test]
    fn oneof_slot_accepts_any_member() {
        let u = Ty::OneOf(vec![Ty::Int, Ty::Str]);
        assert!(u.accepts(&Ty::Int));
        assert!(u.accepts(&Ty::Str));
        assert!(!u.accepts(&Ty::Float)); // disjoint from both members
        let opt = Ty::OneOf(vec![Ty::Struct("Point".into()), Ty::NoneT]);
        assert!(opt.accepts(&Ty::Struct("Point".into())));
        assert!(opt.accepts(&Ty::NoneT));
        assert!(!opt.accepts(&Ty::Str));
    }

    #[test]
    fn oneof_slot_follows_numeric_tower() {
        // A `bool` flows into an `int` member (bool <: int).
        assert!(Ty::OneOf(vec![Ty::Int, Ty::Str]).accepts(&Ty::Bool));
        // An `int` flows into a `float` member (int <: float).
        assert!(Ty::OneOf(vec![Ty::Float, Ty::Str]).accepts(&Ty::Int));
    }

    #[test]
    fn oneof_value_assignable_only_when_every_member_fits() {
        // Robustness arm (unused in v1): a union *value* satisfies a slot only if
        // each of its members does.
        assert!(Ty::Float.accepts(&Ty::OneOf(vec![Ty::Int, Ty::Bool])));
        assert!(!Ty::Int.accepts(&Ty::OneOf(vec![Ty::Int, Ty::Str])));
    }

    fn list(e: Ty) -> Ty {
        Ty::List(Box::new(e))
    }
    fn set(e: Ty) -> Ty {
        Ty::Set(Box::new(e))
    }
    fn dict(k: Ty, v: Ty) -> Ty {
        Ty::Dict(Box::new(k), Box::new(v))
    }
    fn bare_list() -> Ty {
        list(Ty::Top)
    }
    fn bare_set() -> Ty {
        set(Ty::Top)
    }
    fn bare_dict() -> Ty {
        dict(Ty::Top, Ty::Top)
    }
    fn tuple(ps: Vec<Ty>) -> Ty {
        Ty::Tuple(Some(ps))
    }
    fn bare_tuple() -> Ty {
        Ty::Tuple(None)
    }

    #[test]
    fn composites_are_constructor_level() {
        // The container is always concrete, parameterized or not.
        assert!(bare_list().is_concrete());
        assert!(bare_dict().is_concrete());
        assert!(list(Ty::Int).is_concrete());
        // Reflexive accept, no cross-accept between composites, no tower.
        assert!(bare_list().accepts(&bare_list()));
        assert!(!bare_list().accepts(&bare_dict()));
        assert!(!bare_list().accepts(&Ty::Int));
        assert!(!Ty::Int.accepts(&bare_list()));
        // join: equal -> self, disjoint container -> Top.
        assert_eq!(bare_list().join(&bare_list()), bare_list());
        assert_eq!(bare_list().join(&bare_dict()), Ty::Top);
        // display: bare vs parameterized.
        assert_eq!(bare_list().to_string(), "list");
        assert_eq!(bare_dict().to_string(), "dict");
        assert_eq!(list(Ty::Int).to_string(), "list[int]");
        assert_eq!(dict(Ty::Str, Ty::Int).to_string(), "dict[str, int]");
    }

    #[test]
    fn composite_params_accept_covariantly() {
        // A bare-element slot accepts any element; a concrete element slot rejects
        // a provably-incompatible concrete element, accepts the numeric tower, and
        // accepts an unknown (Top) element (not a provable mismatch).
        assert!(list(Ty::Str).accepts(&list(Ty::Str)));
        assert!(!list(Ty::Str).accepts(&list(Ty::Int)));
        assert!(list(Ty::Float).accepts(&list(Ty::Int))); // int <: float
        assert!(bare_list().accepts(&list(Ty::Int)));
        assert!(list(Ty::Str).accepts(&bare_list())); // unknown element -> accepted
        // dict: both parameters covariant.
        assert!(dict(Ty::Str, Ty::Int).accepts(&dict(Ty::Str, Ty::Int)));
        assert!(!dict(Ty::Str, Ty::Int).accepts(&dict(Ty::Str, Ty::Str)));
        assert!(!dict(Ty::Str, Ty::Int).accepts(&dict(Ty::Int, Ty::Int)));
        // join element-wise: disjoint elements widen the parameter, keep container.
        assert_eq!(list(Ty::Int).join(&list(Ty::Str)), bare_list());
        assert_eq!(list(Ty::Int).join(&list(Ty::Int)), list(Ty::Int));
    }

    #[test]
    fn accepts_value_hybrid_variance() {
        // Covariant (fresh literal): list[int] satisfies list[float] via the tower.
        assert!(list(Ty::Float).accepts_value(&list(Ty::Int), true));
        // Invariant (already-typed value): list[int] does NOT satisfy list[float].
        assert!(!list(Ty::Float).accepts_value(&list(Ty::Int), false));
        // Equal accepted under both; an unknown (Top) element is never provable.
        assert!(list(Ty::Int).accepts_value(&list(Ty::Int), false));
        assert!(list(Ty::Int).accepts_value(&bare_list(), false));
        // dict: invariant on both parameters when the value is typed.
        assert!(!dict(Ty::Str, Ty::Float).accepts_value(&dict(Ty::Str, Ty::Int), false));
        assert!(dict(Ty::Str, Ty::Float).accepts_value(&dict(Ty::Str, Ty::Int), true));
        // Nested invariance: list[list[int]] != list[list[float]] when typed.
        assert!(!list(list(Ty::Float)).accepts_value(&list(list(Ty::Int)), false));
        assert!(list(list(Ty::Float)).accepts_value(&list(list(Ty::Int)), true));
        // Non-composite ignores the flag (the numeric tower always applies).
        assert!(Ty::Float.accepts_value(&Ty::Int, false));
        assert!(Ty::Float.accepts_value(&Ty::Int, true));
    }

    #[test]
    fn composite_member_in_union() {
        let u = Ty::OneOf(vec![Ty::Int, bare_list()]);
        assert!(u.accepts(&Ty::Int));
        assert!(u.accepts(&bare_list()));
        assert!(u.accepts(&list(Ty::Int))); // covariant element under the union
        assert!(!u.accepts(&Ty::Str));
        assert!(!u.accepts(&bare_dict()));
        assert_eq!(u.to_string(), "int | list");
    }

    #[test]
    fn set_mirrors_list_but_is_a_distinct_container() {
        // Homogeneous like list: concrete container, covariant element, tower,
        // unknown element accepted, element-wise join, bare vs parameterized display.
        assert!(bare_set().is_concrete());
        assert!(set(Ty::Str).accepts(&set(Ty::Str)));
        assert!(!set(Ty::Str).accepts(&set(Ty::Int)));
        assert!(set(Ty::Float).accepts(&set(Ty::Int))); // int <: float
        assert!(set(Ty::Str).accepts(&bare_set())); // unknown element -> accepted
        assert_eq!(set(Ty::Int).join(&set(Ty::Str)), bare_set());
        assert_eq!(bare_set().to_string(), "set");
        assert_eq!(set(Ty::Int).to_string(), "set[int]");
        // Hybrid variance, same as list.
        assert!(set(Ty::Float).accepts_value(&set(Ty::Int), true));
        assert!(!set(Ty::Float).accepts_value(&set(Ty::Int), false));
        // Distinct container: no cross-assignability with list, either direction.
        assert!(!bare_set().accepts(&bare_list()));
        assert!(!bare_list().accepts(&bare_set()));
        assert!(!set(Ty::Int).accepts(&list(Ty::Int)));
        assert_eq!(set(Ty::Int).join(&list(Ty::Int)), Ty::Top);
        // Set as a union member, covariant element under the union.
        let u = Ty::OneOf(vec![Ty::Int, bare_set()]);
        assert!(u.accepts(&set(Ty::Int)));
        assert!(!u.accepts(&bare_list()));
    }

    #[test]
    fn tuple_is_positional_and_arity_checked() {
        // The container is always concrete, bare or parameterized.
        assert!(bare_tuple().is_concrete());
        assert!(tuple(vec![Ty::Int, Ty::Str]).is_concrete());
        // Same arity, position-wise accept with the numeric tower; provable
        // per-position mismatch rejected.
        assert!(tuple(vec![Ty::Int, Ty::Str]).accepts(&tuple(vec![Ty::Int, Ty::Str])));
        assert!(tuple(vec![Ty::Float, Ty::Str]).accepts(&tuple(vec![Ty::Int, Ty::Str]))); // int <: float at pos 0
        assert!(!tuple(vec![Ty::Int, Ty::Str]).accepts(&tuple(vec![Ty::Int, Ty::Int]))); // str vs int at pos 1
        // Arity is part of the contract: a different length is a provable mismatch.
        assert!(!tuple(vec![Ty::Int, Ty::Str]).accepts(&tuple(vec![Ty::Int])));
        assert!(!tuple(vec![Ty::Int]).accepts(&tuple(vec![Ty::Int, Ty::Str])));
        // An unknown-arity slot (bare `tuple`, `None`) accepts any tuple, and an
        // unknown-arity value is not a provable mismatch (deferred to runtime).
        assert!(bare_tuple().accepts(&tuple(vec![Ty::Int, Ty::Str])));
        assert!(tuple(vec![Ty::Int, Ty::Str]).accepts(&bare_tuple()));
        // But a *known* empty arity `()` (`Some([])`, not the bare `None`) IS a
        // provable mismatch against a fixed-arity slot -- rejected statically, the
        // distinction the `Option` arity buys (TH6). The bare tuple still defers.
        assert!(!tuple(vec![Ty::Int, Ty::Str]).accepts(&tuple(vec![])));
        assert!(tuple(vec![]).accepts(&tuple(vec![]))); // empty satisfies empty
        // An unknown (Top) position is never a provable mismatch.
        assert!(tuple(vec![Ty::Int, Ty::Str]).accepts(&tuple(vec![Ty::Int, Ty::Top])));
        // join: same arity -> position-wise; disjoint arity -> unknown-arity tuple.
        assert_eq!(
            tuple(vec![Ty::Int, Ty::Str]).join(&tuple(vec![Ty::Int, Ty::Int])),
            tuple(vec![Ty::Int, Ty::Top])
        );
        assert_eq!(tuple(vec![Ty::Int]).join(&tuple(vec![Ty::Int, Ty::Str])), bare_tuple());
        // Immutable -> covariant in every position, regardless of the variance
        // flag (no alias-mutation hazard, unlike the mutable list/set/dict).
        assert!(tuple(vec![Ty::Float]).accepts_value(&tuple(vec![Ty::Int]), true));
        assert!(tuple(vec![Ty::Float]).accepts_value(&tuple(vec![Ty::Int]), false));
        // Distinct container: no cross-assignability with list, either direction.
        assert!(!bare_tuple().accepts(&bare_list()));
        assert!(!bare_list().accepts(&bare_tuple()));
        assert_eq!(bare_tuple().join(&bare_list()), Ty::Top);
        // Display: unknown-arity bare (`tuple`) vs known arity (`tuple[..]`, a Top
        // position prints as `?`, the empty arity prints as `tuple[]`).
        assert_eq!(bare_tuple().to_string(), "tuple");
        assert_eq!(tuple(vec![Ty::Int, Ty::Str]).to_string(), "tuple[int, str]");
        assert_eq!(tuple(vec![Ty::Int, Ty::Top]).to_string(), "tuple[int, ?]");
        assert_eq!(tuple(vec![]).to_string(), "tuple[]");
        // Tuple as a union member, positional under the union.
        let u = Ty::OneOf(vec![Ty::Int, bare_tuple()]);
        assert!(u.accepts(&tuple(vec![Ty::Int, Ty::Str])));
        assert!(!u.accepts(&bare_list()));
    }

    #[test]
    fn oneof_displays_pipe_separated() {
        assert_eq!(Ty::OneOf(vec![Ty::Int, Ty::Str]).to_string(), "int | str");
        assert_eq!(
            Ty::OneOf(vec![Ty::Struct("Point".into()), Ty::NoneT]).to_string(),
            "Point | None"
        );
    }

    fn union(name: &str, args: Vec<Ty>) -> Ty {
        Ty::Union(name.into(), args)
    }
    fn bare_union(name: &str) -> Ty {
        union(name, vec![])
    }

    #[test]
    fn union_is_covariant_and_arity_checked() {
        // The container is always concrete, bare or parameterized.
        assert!(bare_union("Option").is_concrete());
        assert!(union("Option", vec![Ty::Int]).is_concrete());
        // Same name, same arity: covariant argument accept with the numeric tower;
        // a provable argument mismatch is rejected.
        assert!(union("Option", vec![Ty::Int]).accepts(&union("Option", vec![Ty::Int])));
        assert!(union("Option", vec![Ty::Float]).accepts(&union("Option", vec![Ty::Int]))); // int <: float
        assert!(!union("Option", vec![Ty::Int]).accepts(&union("Option", vec![Ty::Str])));
        // Multi-argument (Result[T, E]), position-wise.
        assert!(union("Result", vec![Ty::Float, Ty::Str]).accepts(&union("Result", vec![Ty::Int, Ty::Str])));
        assert!(!union("Result", vec![Ty::Int, Ty::Str]).accepts(&union("Result", vec![Ty::Int, Ty::Int])));
        // Distinct names never match.
        assert!(!union("Option", vec![Ty::Int]).accepts(&union("Result", vec![Ty::Int])));
        // A bare slot accepts any same-name union; a bare value defers (unknown
        // arity is never a provable mismatch) -- both directions accept.
        assert!(bare_union("Option").accepts(&union("Option", vec![Ty::Int])));
        assert!(union("Option", vec![Ty::Int]).accepts(&bare_union("Option")));
        // An unknown (Top) argument is never a provable mismatch (known arity,
        // unknown parameter -- distinct from the empty-argument unknown arity).
        assert!(union("Option", vec![Ty::Int]).accepts(&union("Option", vec![Ty::Top])));
        // Immutable -> covariant regardless of the variance flag (like tuple).
        assert!(union("Option", vec![Ty::Float]).accepts_value(&union("Option", vec![Ty::Int]), true));
        assert!(union("Option", vec![Ty::Float]).accepts_value(&union("Option", vec![Ty::Int]), false));
    }

    #[test]
    fn union_joins_argument_wise() {
        // Same name and arity: argument-wise join (disjoint argument -> Top).
        assert_eq!(
            union("Option", vec![Ty::Int]).join(&union("Option", vec![Ty::Int])),
            union("Option", vec![Ty::Int])
        );
        assert_eq!(
            union("Option", vec![Ty::Int]).join(&union("Option", vec![Ty::Str])),
            union("Option", vec![Ty::Top])
        );
        // Disjoint arity (one bare) widens to the bare union, keeping the name.
        assert_eq!(
            union("Option", vec![Ty::Int]).join(&bare_union("Option")),
            bare_union("Option")
        );
        // Distinct names have no common union -> Top.
        assert_eq!(
            union("Option", vec![Ty::Int]).join(&union("Result", vec![Ty::Int])),
            Ty::Top
        );
    }

    #[test]
    fn union_display_and_membership() {
        assert_eq!(bare_union("Option").to_string(), "Option");
        assert_eq!(union("Option", vec![Ty::Int]).to_string(), "Option[int]");
        assert_eq!(union("Result", vec![Ty::Int, Ty::Str]).to_string(), "Result[int, str]");
        assert_eq!(union("Option", vec![Ty::Top]).to_string(), "Option[?]");
        // A parameterized union as a union-of-types member (covariant argument).
        let u = Ty::OneOf(vec![Ty::Int, union("Option", vec![Ty::Int])]);
        assert!(u.accepts(&union("Option", vec![Ty::Int])));
        assert!(u.accepts(&union("Option", vec![Ty::Bool]))); // bool <: int under the arg
        assert!(!u.accepts(&union("Result", vec![Ty::Int])));
        assert_eq!(u.to_string(), "int | Option[int]");
    }
}
