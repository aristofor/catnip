(* FILE: proof/expr/CatnipNDRecursion.v *)
(*                                                                    *)
(* ND-recursion and partial termination.  Models @@(seed, lambda)     *)
(* with fuel-bounded evaluation, monotonicity, determinism, and       *)
(* memoization coherence.  Independent of the value domain.           *)
(* Split from CatnipDimensional.v to reduce peak memory.              *)

From Coq Require Import Arith PeanoNat Lia.


(** ND-recursion models @@(seed, lambda) where the lambda receives
    (value, recur) and may call recur on a sub-seed.

    We parameterize over:
    - [base]:      predicate identifying base cases
    - [base_val]:  value for base cases
    - [step_seed]: computes the next seed for recursive calls
    - [combine]:   combines the current seed with the recursive result

    This captures the essential structure of @@. *)

Section NDRecursion.

Variable base      : nat -> bool.
Variable base_val  : nat -> nat.
Variable step_seed : nat -> nat.
Variable combine   : nat -> nat -> nat.

(** Fuel-bounded ND evaluation.
    Returns [None] when fuel is exhausted, [Some v] otherwise. *)

Fixpoint nd_eval (fuel : nat) (seed : nat) : option nat :=
  match fuel with
  | 0 => None
  | S fuel' =>
      if base seed then Some (base_val seed)
      else match nd_eval fuel' (step_seed seed) with
           | Some r => Some (combine seed r)
           | None   => None
           end
  end.

(** Unfolding lemma: one step of nd_eval. *)

Lemma nd_eval_unfold : forall fuel seed,
  nd_eval (S fuel) seed =
    if base seed then Some (base_val seed)
    else match nd_eval fuel (step_seed seed) with
         | Some r => Some (combine seed r)
         | None   => None
         end.
Proof. reflexivity. Qed.

(** Monotonicity: more fuel never changes the result.
    Once a computation succeeds, adding fuel preserves the answer. *)

Lemma nd_eval_mono : forall fuel1 fuel2 seed v,
  nd_eval fuel1 seed = Some v ->
  fuel1 <= fuel2 ->
  nd_eval fuel2 seed = Some v.
Proof.
  induction fuel1 as [| f1 IH]; intros fuel2 seed v Heval Hle.
  - simpl in Heval. discriminate.
  - destruct fuel2 as [| f2].
    + lia.
    + rewrite nd_eval_unfold in Heval. rewrite nd_eval_unfold.
      destruct (base seed).
      * exact Heval.
      * destruct (nd_eval f1 (step_seed seed)) eqn:Hrec.
        -- rewrite (IH f2 (step_seed seed) n Hrec ltac:(lia)).
           exact Heval.
        -- discriminate.
Qed.

(** Determinism: if nd_eval succeeds with two different fuel values,
    the results agree. *)

Theorem nd_eval_deterministic : forall fuel1 fuel2 seed v1 v2,
  nd_eval fuel1 seed = Some v1 ->
  nd_eval fuel2 seed = Some v2 ->
  v1 = v2.
Proof.
  intros fuel1 fuel2 seed v1 v2 H1 H2.
  assert (Hmax : fuel1 <= Nat.max fuel1 fuel2) by lia.
  assert (Hmax2 : fuel2 <= Nat.max fuel1 fuel2) by lia.
  assert (E1 := nd_eval_mono _ _ _ _ H1 Hmax).
  assert (E2 := nd_eval_mono _ _ _ _ H2 Hmax2).
  rewrite E1 in E2. injection E2. auto.
Qed.

(** Partial termination: if a measure [mu] strictly decreases at
    each recursive step, then evaluation terminates.

    The required fuel is [S (mu seed)]. *)

Variable mu : nat -> nat.

Hypothesis step_decreases : forall seed,
  base seed = false -> mu (step_seed seed) < mu seed.

Lemma nd_termination_aux : forall n seed,
  mu seed <= n ->
  exists v, nd_eval (S n) seed = Some v.
Proof.
  induction n as [| n' IH]; intros seed Hmu.
  - (* n = 0: mu seed = 0, must be a base case *)
    rewrite nd_eval_unfold.
    destruct (base seed) eqn:Hb.
    + eexists. reflexivity.
    + (* base seed = false contradicts mu seed = 0 *)
      exfalso. specialize (step_decreases seed Hb). lia.
  - (* n = S n' *)
    rewrite nd_eval_unfold.
    destruct (base seed) eqn:Hb.
    + eexists. reflexivity.
    + (* Recursive case: mu (step_seed seed) <= n' *)
      assert (Hmu' : mu (step_seed seed) <= n')
        by (specialize (step_decreases seed Hb); lia).
      destruct (IH (step_seed seed) Hmu') as [rv Hrv].
      rewrite Hrv. eexists. reflexivity.
Qed.

Theorem nd_partial_termination : forall seed,
  exists v, nd_eval (S (mu seed)) seed = Some v.
Proof.
  intro seed. apply nd_termination_aux. lia.
Qed.

(** Lookup coherence: lookup-augmented evaluation agrees with
    standard evaluation.

    Note: [memo] is read-only in this model. [nd_eval_memo] does not
    update the table during recursion; it only short-circuits when a
    pre-filled entry exists. *)

Variable memo : nat -> option nat.

Hypothesis memo_correct : forall seed v,
  memo seed = Some v ->
  exists fuel, nd_eval fuel seed = Some v.

Fixpoint nd_eval_memo (fuel : nat) (seed : nat) : option nat :=
  match memo seed with
  | Some v => Some v
  | None =>
      match fuel with
      | 0 => None
      | S fuel' =>
          if base seed then Some (base_val seed)
          else match nd_eval_memo fuel' (step_seed seed) with
               | Some r => Some (combine seed r)
               | None   => None
               end
      end
  end.

Lemma nd_eval_memo_lookup_hit : forall fuel seed v,
  memo seed = Some v ->
  nd_eval_memo fuel seed = Some v.
Proof.
  intros fuel seed v Hmemo.
  destruct fuel as [|fuel']; simpl; rewrite Hmemo; reflexivity.
Qed.

(** Lookup-augmented evaluation agrees with standard evaluation:
    if memo hits are correct, the result is the same. *)

Theorem memo_coherence : forall fuel seed v,
  nd_eval_memo fuel seed = Some v ->
  exists fuel', nd_eval fuel' seed = Some v.
Proof.
  induction fuel as [| fuel' IH]; intros seed v Heval.
  - (* fuel = 0 *)
    simpl in Heval.
    destruct (memo seed) eqn:Hmemo.
    + injection Heval as ->. apply memo_correct. exact Hmemo.
    + discriminate.
  - simpl in Heval.
    destruct (memo seed) eqn:Hmemo.
    + injection Heval as ->. apply memo_correct. exact Hmemo.
    + destruct (base seed) eqn:Hb.
      * injection Heval as <-.
        exists 1. rewrite nd_eval_unfold. rewrite Hb. reflexivity.
      * destruct (nd_eval_memo fuel' (step_seed seed)) eqn:Hrec.
        -- injection Heval as <-.
           destruct (IH (step_seed seed) n Hrec) as [fuel_rec Hfuel_rec].
           exists (S fuel_rec). rewrite nd_eval_unfold. rewrite Hb.
           rewrite Hfuel_rec. reflexivity.
        -- discriminate.
Qed.

End NDRecursion.
