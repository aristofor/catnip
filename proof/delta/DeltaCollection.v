(* FILE: proof/delta/DeltaCollection.v *)
(* DeltaCollection.v - neutral compaction of signed-diff transitions
 *
 * Source of truth:
 *   catnip_core/src/delta/collection.rs  (Delta::compact, Collection::apply)
 *
 * Step 1 of the delta dataflow project (wip/DELTA_STEP1_COLLECTION.md):
 * a Delta is a list of (value, diff) records, a Collection is the per-value
 * sum of every applied diff. Compaction folds records into per-key sums
 * (first-appearance order) and drops zeros -- exactly the Rust IndexMap
 * fold followed by the non-zero filter. This file proves compaction is
 * unobservable: no per-key sum changes, hence no applied state changes.
 *
 * The model sums in Z (no overflow), matching the assumed gap documented in
 * the Rust code (i64 overflow out of scope). Equalities on collections are
 * extensional (per key): list-level structural equality depends on
 * representation order, which is Delta's stability contract, not part of
 * the algebra proved here.
 *
 * Theorems: 5 (+ 4 supporting lemmas). 0 Admitted.
 *
 *   compaction_preserves_sum : per-key sums survive compact
 *   compact_idempotent       : compact (compact d) ~ compact d (per key)
 *   apply_compact_eq         : applying compact d = applying d
 *   empty_delta_neutral      : applying [] changes nothing
 *   negate_roundtrip         : apply d then apply (negate d) restores C
 *)

From Coq Require Import List ZArith Lia.
Import ListNotations.
Open Scope Z_scope.

Section DeltaCollection.

  (* Values are abstract with decidable equality -- the Rust `Eq + Hash`
     contract (DeltaValue). Reflexivity of `=` is what the multiset needs
     so a -1 cancels its matching +1 (why raw f64 is not a DeltaValue). *)
  Variable K : Type.
  Hypothesis K_eq_dec : forall a b : K, {a = b} + {a <> b}.

  Definition delta := list (K * Z).
  Definition collection := K -> Z.

  (* Per-key sum of a delta's records: the one observable of the algebra. *)
  Fixpoint sum_diffs (v : K) (d : delta) : Z :=
    match d with
    | [] => 0
    | (k, c) :: rest => (if K_eq_dec v k then c else 0) + sum_diffs v rest
    end.

  (* One step of the Rust fold: IndexMap entry(k) += c, insertion order kept
     (the new key lands at the tail = first-appearance order overall). *)
  Fixpoint add_diff (k : K) (c : Z) (acc : delta) : delta :=
    match acc with
    | [] => [(k, c)]
    | (k', c') :: rest =>
        if K_eq_dec k k' then (k', c' + c) :: rest
        else (k', c') :: add_diff k c rest
    end.

  (* compact = fold every record into the accumulator, then drop zeros --
     the exact shape of Delta::compact (IndexMap fold + non-zero filter). *)
  Definition nonzero (p : K * Z) : bool := negb (Z.eqb (snd p) 0).

  Definition compact (d : delta) : delta :=
    filter nonzero (fold_left (fun acc p => add_diff (fst p) (snd p) acc) d []).

  Definition apply (C : collection) (d : delta) : collection :=
    fun v => C v + sum_diffs v d.

  Definition negate (d : delta) : delta :=
    map (fun p => (fst p, - snd p)) d.

  (* ---------- supporting lemmas ---------- *)

  (* Dropping zero records never changes a per-key sum: a removed record
     contributes exactly 0. *)
  Lemma sum_filter_nonzero :
    forall v d, sum_diffs v (filter nonzero d) = sum_diffs v d.
  Proof.
    intros v d; induction d as [| [k c] rest IH]; simpl.
    - reflexivity.
    - unfold nonzero at 1; simpl.
      destruct (Z.eqb_spec c 0) as [Hc | Hc]; simpl.
      + (* c = 0: the record is dropped and contributed 0 *)
        subst c. destruct (K_eq_dec v k); lia.
      + destruct (K_eq_dec v k); lia.
  Qed.

  (* One fold step adds exactly the record's contribution to the key. *)
  Lemma sum_add_diff :
    forall v k c acc,
      sum_diffs v (add_diff k c acc) =
      sum_diffs v acc + (if K_eq_dec v k then c else 0).
  Proof.
    intros v k c acc; induction acc as [| [k' c'] rest IH]; simpl.
    - destruct (K_eq_dec v k); lia.
    - destruct (K_eq_dec k k') as [Hkk | Hkk]; simpl.
      + (* merged into the existing entry for k' = k *)
        subst k'. destruct (K_eq_dec v k); lia.
      + (* pushed further down, entry k' untouched *)
        destruct (K_eq_dec v k'); rewrite IH; lia.
  Qed.

  (* The whole fold accumulates the delta's sums on top of the accumulator's. *)
  Lemma sum_fold :
    forall v d acc,
      sum_diffs v (fold_left (fun a p => add_diff (fst p) (snd p) a) d acc) =
      sum_diffs v acc + sum_diffs v d.
  Proof.
    intros v d; induction d as [| [k c] rest IH]; intros acc; simpl.
    - lia.
    - rewrite IH, sum_add_diff. destruct (K_eq_dec v k); lia.
  Qed.

  (* Negation flips every per-key sum. *)
  Lemma sum_negate :
    forall v d, sum_diffs v (negate d) = - sum_diffs v d.
  Proof.
    intros v d; induction d as [| [k c] rest IH]; simpl.
    - reflexivity.
    - rewrite IH. destruct (K_eq_dec v k); lia.
  Qed.

  (* ---------- theorems ---------- *)

  (* Compaction is unobservable on per-key sums. *)
  Theorem compaction_preserves_sum :
    forall v d, sum_diffs v (compact d) = sum_diffs v d.
  Proof.
    intros v d. unfold compact.
    rewrite sum_filter_nonzero, sum_fold. simpl. lia.
  Qed.

  (* Idempotence, extensionally per key (the only equality with semantic
     reach -- structural list equality depends on representation order). *)
  Theorem compact_idempotent :
    forall d v, sum_diffs v (compact (compact d)) = sum_diffs v (compact d).
  Proof.
    intros d v. apply compaction_preserves_sum.
  Qed.

  (* Applying a compacted delta is applying the delta. *)
  Theorem apply_compact_eq :
    forall C d v, apply C (compact d) v = apply C d v.
  Proof.
    intros C d v. unfold apply. rewrite compaction_preserves_sum. reflexivity.
  Qed.

  (* The empty delta is neutral. *)
  Theorem empty_delta_neutral :
    forall (C : collection) v, apply C [] v = C v.
  Proof.
    intros C v. unfold apply. simpl. lia.
  Qed.

  (* Applying a delta then its negation restores the collection. *)
  Theorem negate_roundtrip :
    forall C d v, apply (apply C d) (negate d) v = C v.
  Proof.
    intros C d v. unfold apply. rewrite sum_negate. lia.
  Qed.

End DeltaCollection.
