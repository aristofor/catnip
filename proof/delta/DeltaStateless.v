(* FILE: proof/delta/DeltaStateless.v *)
(* DeltaStateless.v - stateless operators: map/filter homomorphisms, concat union
 *
 * Source of truth:
 *   catnip_core/src/delta/ops_stateless.rs  (Map, Filter, Concat)
 *
 * Step 2 of the delta dataflow project (wip/DELTA_STEP2_STATELESS.md).
 * A stateless operator is characterized by "the output delta depends only on
 * the input delta". The theorems make that dependence PER-KEY-SUM only:
 *
 *   - map/filter respect compaction (the operator may run on the compacted
 *     input -- the observable output sums do not change), and distribute
 *     over delta concatenation (processing state-then-delta equals
 *     processing their union): together, the homomorphism property, stated
 *     on the finitary delta representation (collections as functions would
 *     drag in infinite-support sums over preimages, out of model scope);
 *   - concat is multiset union: applying d1 ++ d2 is applying d1 then d2
 *     (which with compaction neutrality covers Delta::from_deltas);
 *   - a stateless commit is the identity on state (anchors the invariant
 *     the step 3 atomicity proof generalizes).
 *
 * `f` and `p` are Coq functions, hence pure and deterministic by
 * construction -- exactly the D8 contract.
 *
 * Theorems: 7 (+ 4 supporting lemmas). 0 Admitted.
 *)

From Coq Require Import List ZArith Lia.
From Catnip Require Import DeltaCollection.
Import ListNotations.
Open Scope Z_scope.

Section DeltaStateless.

  Variable K : Type.
  Hypothesis K_eq_dec : forall a b : K, {a = b} + {a <> b}.

  Notation delta := (list (K * Z)).
  Notation sum_diffs := (sum_diffs K K_eq_dec).
  Notation compact := (compact K K_eq_dec).
  Notation add_diff := (add_diff K K_eq_dec).
  Notation apply := (apply K K_eq_dec).

  (* map f transports each record's key through f, diff untouched --
     Map::compute minus the host indirection (f total and pure, D8). *)
  Definition map_delta (f : K -> K) (d : delta) : delta :=
    map (fun p => (f (fst p), snd p)) d.

  (* filter p keeps the records whose key satisfies p, diff untouched. *)
  Definition filter_delta (p : K -> bool) (d : delta) : delta :=
    filter (fun q => p (fst q)) d.

  (* ---------- supporting lemmas ---------- *)

  (* Sums distribute over concatenation (concat's algebra). *)
  Lemma sum_app :
    forall v (d1 d2 : delta),
      sum_diffs v (d1 ++ d2) = sum_diffs v d1 + sum_diffs v d2.
  Proof.
    intros v d1 d2; induction d1 as [| [k c] rest IH]; simpl.
    - reflexivity.
    - rewrite IH. destruct (K_eq_dec v k); lia.
  Qed.

  (* A filtered delta's per-key sum has a closed form: kept keys keep their
     sum, dropped keys sum to zero. *)
  Lemma sum_filter_delta :
    forall p v (d : delta),
      sum_diffs v (filter_delta p d) =
      if p v then sum_diffs v d else 0.
  Proof.
    intros p v d; induction d as [| [k c] rest IH]; simpl.
    - destruct (p v); reflexivity.
    - unfold filter_delta in *; simpl.
      destruct (p k) eqn:Hpk; simpl.
      + destruct (K_eq_dec v k) as [He | He].
        * subst k. rewrite Hpk, IH, Hpk. lia.
        * rewrite IH. destruct (p v); lia.
      + destruct (K_eq_dec v k) as [He | He].
        * subst k. rewrite IH, Hpk. lia.
        * rewrite IH. destruct (p v); lia.
  Qed.

  (* One accumulator step, seen through map: merging (k, c) into the entry
     for k adds c to the mapped sum at f k. *)
  Lemma msum_add_diff :
    forall (f : K -> K) v k c (acc : delta),
      sum_diffs v (map_delta f (add_diff k c acc)) =
      sum_diffs v (map_delta f acc) + (if K_eq_dec v (f k) then c else 0).
  Proof.
    intros f v k c acc; induction acc as [| [k' c'] rest IH]; simpl.
    - destruct (K_eq_dec v (f k)); lia.
    - destruct (K_eq_dec k k') as [Hkk | Hkk]; simpl.
      + subst k'. destruct (K_eq_dec v (f k)); lia.
      + rewrite IH. destruct (K_eq_dec v (f k')); lia.
  Qed.

  (* The whole fold, seen through map. *)
  Lemma msum_fold :
    forall (f : K -> K) v (d acc : delta),
      sum_diffs v (map_delta f
        (fold_left (fun a q => add_diff (fst q) (snd q) a) d acc)) =
      sum_diffs v (map_delta f acc) + sum_diffs v (map_delta f d).
  Proof.
    intros f v d; induction d as [| [k c] rest IH]; intros acc; simpl.
    - lia.
    - rewrite IH, msum_add_diff. destruct (K_eq_dec v (f k)); lia.
  Qed.

  (* Dropping zero records is invisible through map (a dropped record
     contributes 0 to any mapped key). *)
  Lemma msum_filter_nonzero :
    forall (f : K -> K) v (d : delta),
      sum_diffs v (map_delta f (filter (nonzero K) d)) =
      sum_diffs v (map_delta f d).
  Proof.
    intros f v d; induction d as [| [k c] rest IH]; simpl.
    - reflexivity.
    - unfold nonzero at 1; simpl.
      destruct (Z.eqb_spec c 0) as [Hc | Hc]; simpl.
      + subst c. destruct (K_eq_dec v (f k)); lia.
      + destruct (K_eq_dec v (f k)); lia.
  Qed.

  (* ---------- theorems ---------- *)

  (* Map may run on the compacted input: the output sums do not change.
     This is what lets Map::compute consume the always-compacted Delta. *)
  Theorem map_respects_compaction :
    forall (f : K -> K) v (d : delta),
      sum_diffs v (map_delta f (compact d)) = sum_diffs v (map_delta f d).
  Proof.
    intros f v d. unfold compact.
    rewrite msum_filter_nonzero, msum_fold. simpl. lia.
  Qed.

  (* Map distributes over delta concatenation: mapping state-then-delta is
     mapping their union (the delta-representation homomorphism). *)
  Theorem map_additive :
    forall (f : K -> K) v (d1 d2 : delta),
      sum_diffs v (map_delta f (d1 ++ d2)) =
      sum_diffs v (map_delta f d1) + sum_diffs v (map_delta f d2).
  Proof.
    intros f v d1 d2. unfold map_delta. rewrite map_app. apply sum_app.
  Qed.

  (* Same two properties for filter. *)
  Theorem filter_respects_compaction :
    forall (p : K -> bool) v (d : delta),
      sum_diffs v (filter_delta p (compact d)) =
      sum_diffs v (filter_delta p d).
  Proof.
    intros p v d. rewrite !sum_filter_delta.
    destruct (p v).
    - apply compaction_preserves_sum.
    - reflexivity.
  Qed.

  Theorem filter_additive :
    forall (p : K -> bool) v (d1 d2 : delta),
      sum_diffs v (filter_delta p (d1 ++ d2)) =
      sum_diffs v (filter_delta p d1) + sum_diffs v (filter_delta p d2).
  Proof.
    intros p v d1 d2. rewrite !sum_filter_delta.
    destruct (p v).
    - apply sum_app.
    - reflexivity.
  Qed.

  (* Concat is multiset union: applying the concatenation is applying each
     delta in turn. With compaction neutrality (step 1), this covers
     Delta::from_deltas (concatenate everything, compact once). *)
  Theorem concat_union :
    forall (C : K -> Z) (d1 d2 : delta) v,
      apply C (d1 ++ d2) v = apply (apply C d1) d2 v.
  Proof.
    intros C d1 d2 v. unfold DeltaCollection.apply. rewrite sum_app. lia.
  Qed.

  (* from_deltas = compact of the concatenation: same observable as the
     sequential application (ties concat_union to the Rust constructor). *)
  Theorem from_deltas_observable :
    forall (C : K -> Z) (d1 d2 : delta) v,
      apply C (compact (d1 ++ d2)) v = apply (apply C d1) d2 v.
  Proof.
    intros C d1 d2 v. rewrite apply_compact_eq. apply concat_union.
  Qed.

  (* A stateless operator's commit is the identity on state: it has no state
     to write (Staged::None). Anchors the invariant that step 3's atomicity
     proof generalizes to staged writes. *)
  Definition stateless_commit (s : unit) : unit := s.

  Theorem stateless_commit_noop : forall s, stateless_commit s = s.
  Proof. reflexivity. Qed.

End DeltaStateless.
