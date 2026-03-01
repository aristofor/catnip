(* FILE: proof/optim/CatnipTailRecLoopProof.v *)
(* CatnipTailRecLoopProof.v — Tail recursion to loop transformation
 *
 * Source of truth:
 *   catnip_rs/src/semantic/tail_recursion_to_loop.rs
 *
 * Proves the semantic equivalence between tail-recursive functions
 * and their loop-based transformation. The pass converts:
 *   f(s) = if base(s) then result(s) else f(step(s))
 * into:
 *   f(s) = { while !base(s) { s := step(s) }; result(s) }
 *
 * Also proves the correctness of two-phase rebinding through
 * temporaries, which ensures arguments are evaluated with the
 * original parameter values before reassignment.
 *
 * Standalone: no dependencies on other Catnip proofs.
 *)

From Coq Require Import List PeanoNat Lia Bool.
Import ListNotations.


(* ================================================================ *)
(* A. Abstract Tail-Recursive Function Model                         *)
(*                                                                    *)
(* A tail-recursive function is characterized by:                    *)
(*   - is_base  : St -> bool   (termination predicate)               *)
(*   - base_val : St -> R      (result when base case holds)         *)
(*   - step     : St -> St     (next state for recursive call)       *)
(*                                                                    *)
(* Parametric in state type St and result type R.                    *)
(* ================================================================ *)

Section TailRecToLoop.

Variable St : Type.
Variable R : Type.

Variable is_base  : St -> bool.
Variable base_val : St -> R.
Variable step     : St -> St.


(* ================================================================ *)
(* B. Recursive Evaluation (trampoline model)                        *)
(*                                                                    *)
(* Models the original tail-recursive function:                      *)
(*   f(s) = if base(s) then result(s) else f(step(s))               *)
(*                                                                    *)
(* Each non-base step corresponds to one TailCall signal in the      *)
(* trampoline. Fuel bounds the number of iterations.                 *)
(* ================================================================ *)

Fixpoint eval_rec (fuel : nat) (s : St) : option R :=
  match fuel with
  | 0 => None
  | S n =>
      if is_base s then Some (base_val s)
      else eval_rec n (step s)
  end.


(* ================================================================ *)
(* C. Loop Evaluation (transformed model)                            *)
(*                                                                    *)
(* Models the transformed version with while loop:                   *)
(*   f(s) = { while !base(s) { s := step(s) }; result(s) }          *)
(*                                                                    *)
(* Split into two phases:                                            *)
(*   1. iterate: advance state until base case                       *)
(*   2. apply base_val to the final state                            *)
(* ================================================================ *)

Fixpoint iterate (fuel : nat) (s : St) : option St :=
  match fuel with
  | 0 => None
  | S n =>
      if is_base s then Some s
      else iterate n (step s)
  end.

Definition eval_loop (fuel : nat) (s : St) : option R :=
  match iterate fuel s with
  | Some s_final => Some (base_val s_final)
  | None => None
  end.


(* ================================================================ *)
(* D. Equivalence: recursive = iterative                             *)
(*                                                                    *)
(* Core correctness theorem of the transformation.                   *)
(* ================================================================ *)

Theorem tail_rec_to_loop_correct : forall fuel s,
  eval_rec fuel s = eval_loop fuel s.
Proof.
  induction fuel as [|n IH]; intros s.
  - reflexivity.
  - simpl. destruct (is_base s) eqn:Hbase.
    + unfold eval_loop. simpl. rewrite Hbase. reflexivity.
    + rewrite IH. unfold eval_loop. simpl. rewrite Hbase. reflexivity.
Qed.


(* ================================================================ *)
(* E. Fuel Properties                                                *)
(* ================================================================ *)

(* More fuel preserves the result *)
Theorem eval_rec_fuel_monotone : forall n m s v,
  eval_rec n s = Some v ->
  eval_rec (n + m) s = Some v.
Proof.
  induction n as [|n IH]; intros m s v H.
  - simpl in H. discriminate.
  - simpl in H. destruct (is_base s) eqn:Hbase.
    + inversion H. subst. simpl. rewrite Hbase. reflexivity.
    + simpl. rewrite Hbase. exact (IH m _ v H).
Qed.

Theorem iterate_fuel_monotone : forall n m s sf,
  iterate n s = Some sf ->
  iterate (n + m) s = Some sf.
Proof.
  induction n as [|n IH]; intros m s sf H.
  - simpl in H. discriminate.
  - simpl in H. destruct (is_base s) eqn:Hbase.
    + inversion H. subst. simpl. rewrite Hbase. reflexivity.
    + simpl. rewrite Hbase. exact (IH m _ sf H).
Qed.

(* Sufficient fuel also works for eval_loop *)
Corollary eval_loop_fuel_monotone : forall n m s v,
  eval_loop n s = Some v ->
  eval_loop (n + m) s = Some v.
Proof.
  intros n m s v H.
  rewrite <- tail_rec_to_loop_correct in H.
  rewrite <- tail_rec_to_loop_correct.
  exact (eval_rec_fuel_monotone n m s v H).
Qed.

(* iterate always returns a base state *)
Theorem iterate_finds_base : forall fuel s s_final,
  iterate fuel s = Some s_final ->
  is_base s_final = true.
Proof.
  induction fuel as [|n IH]; intros s s_final H.
  - simpl in H. discriminate.
  - simpl in H. destruct (is_base s) eqn:Hbase.
    + inversion H. subst. exact Hbase.
    + exact (IH _ s_final H).
Qed.

(* If already at base, one fuel unit suffices *)
Lemma eval_rec_base_immediate : forall fuel s,
  is_base s = true ->
  eval_rec (S fuel) s = Some (base_val s).
Proof.
  intros fuel s Hbase. simpl. rewrite Hbase. reflexivity.
Qed.

(* If not at base, reduce by one step *)
Lemma eval_rec_step : forall fuel s,
  is_base s = false ->
  eval_rec (S fuel) s = eval_rec fuel (step s).
Proof.
  intros fuel s Hbase. simpl. rewrite Hbase. reflexivity.
Qed.

(* Number of iterations = number of steps to reach base *)
Fixpoint steps_to_base (fuel : nat) (s : St) : option nat :=
  match fuel with
  | 0 => None
  | S n =>
      if is_base s then Some 0
      else match steps_to_base n (step s) with
           | Some k => Some (S k)
           | None => None
           end
  end.

(* If we know how many steps, the result is deterministic *)
Theorem eval_rec_steps_deterministic : forall fuel1 fuel2 s v1 v2,
  eval_rec fuel1 s = Some v1 ->
  eval_rec fuel2 s = Some v2 ->
  v1 = v2.
Proof.
  induction fuel1 as [|n1 IH]; intros fuel2 s v1 v2 H1 H2.
  - simpl in H1. discriminate.
  - simpl in H1. destruct (is_base s) eqn:Hbase.
    + inversion H1. subst. clear H1.
      destruct fuel2 as [|n2]; simpl in H2; [discriminate|].
      rewrite Hbase in H2. inversion H2. reflexivity.
    + destruct fuel2 as [|n2]; simpl in H2; [discriminate|].
      rewrite Hbase in H2. exact (IH n2 _ v1 v2 H1 H2).
Qed.

End TailRecToLoop.


(* ================================================================ *)
(* F. Two-Phase Rebinding                                            *)
(*                                                                    *)
(* The pass rebinds parameters through temporaries:                  *)
(*   Phase 1: tmp_i := eval(arg_i, env)    (all see original env)   *)
(*   Phase 2: param_i := tmp_i             (copy from temps)        *)
(*                                                                    *)
(* This implements simultaneous assignment. Sequential assignment     *)
(* without temporaries would let later arguments see already-modified *)
(* parameters, breaking correctness.                                  *)
(*                                                                    *)
(* We prove: two-phase rebinding = simultaneous evaluation.          *)
(* ================================================================ *)

Section Rebinding.

Variable V : Type.

(* Environment: association list, head = most recent *)
Definition Env := list (nat * V).

Fixpoint env_lookup (env : Env) (x : nat) : option V :=
  match env with
  | [] => None
  | (k, v) :: rest => if Nat.eqb k x then Some v else env_lookup rest x
  end.

Definition env_set (env : Env) (x : nat) (v : V) : Env := (x, v) :: env.

(* Simple expressions: variable references or constants *)
Inductive RExpr :=
  | RVar   : nat -> RExpr
  | RConst : V -> RExpr.

Definition eval_rexpr (env : Env) (e : RExpr) : option V :=
  match e with
  | RVar x => env_lookup env x
  | RConst v => Some v
  end.


(* --- Simultaneous assignment --- *)
(* Evaluate all exprs with original env, collect values *)
Fixpoint eval_all (env : Env) (exprs : list RExpr) : option (list V) :=
  match exprs with
  | [] => Some []
  | e :: es =>
      match eval_rexpr env e, eval_all env es with
      | Some v, Some vs => Some (v :: vs)
      | _, _ => None
      end
  end.

(* Bind a list of values to a list of target variables *)
Fixpoint bind_targets (env : Env) (targets : list nat) (vals : list V) : option Env :=
  match targets, vals with
  | [], [] => Some env
  | t :: ts, v :: vs =>
      match bind_targets env ts vs with
      | Some env' => Some (env_set env' t v)
      | None => None
      end
  | _, _ => None
  end.

(* Two-phase: evaluate all, then bind all *)
Definition two_phase (env : Env) (targets : list nat) (exprs : list RExpr) : option Env :=
  match eval_all env exprs with
  | Some vals => bind_targets env targets vals
  | None => None
  end.


(* --- Sequential assignment (no temporaries, INCORRECT for dependent args) --- *)
(* Each expression is evaluated with the CURRENT (already modified) env *)
Fixpoint sequential (env : Env) (targets : list nat) (exprs : list RExpr) : option Env :=
  match targets, exprs with
  | [], [] => Some env
  | t :: ts, e :: es =>
      match eval_rexpr env e with
      | Some v => sequential (env_set env t v) ts es
      | None => None
      end
  | _, _ => None
  end.


(* --- Properties --- *)

(* eval_all succeeds iff every expression evaluates *)
Lemma eval_all_cons : forall env e es v vs,
  eval_rexpr env e = Some v ->
  eval_all env es = Some vs ->
  eval_all env (e :: es) = Some (v :: vs).
Proof.
  intros env e es v vs He Hes. simpl. rewrite He, Hes. reflexivity.
Qed.

(* bind_targets length must match *)
Lemma bind_targets_length : forall env targets vals env',
  bind_targets env targets vals = Some env' ->
  length targets = length vals.
Proof.
  induction targets as [|t ts IH]; intros [|v vs] env' H;
  simpl in H; try discriminate.
  - reflexivity.
  - destruct (bind_targets env ts vs) eqn:Hb; [|discriminate].
    inversion H. simpl. f_equal. exact (IH _ _ Hb).
Qed.

(* Two-phase and sequential agree when expressions are independent.
   For a SINGLE assignment, both are trivially identical. *)
Theorem single_assign_equiv : forall env t e v,
  eval_rexpr env e = Some v ->
  two_phase env [t] [e] = Some (env_set env t v).
Proof.
  intros env t e v He. unfold two_phase. simpl. rewrite He. simpl. reflexivity.
Qed.

(* Two-phase correctly evaluates all expressions with original env *)
Theorem two_phase_uses_original_env : forall env targets exprs vals env',
  eval_all env exprs = Some vals ->
  bind_targets env targets vals = Some env' ->
  two_phase env targets exprs = Some env'.
Proof.
  intros env targets exprs vals env' Heval Hbind.
  unfold two_phase. rewrite Heval. exact Hbind.
Qed.

(* Key invariant: in two-phase, eval_all sees a frozen snapshot *)
Theorem eval_all_frozen : forall env exprs vals,
  eval_all env exprs = Some vals ->
  forall i e, nth_error exprs i = Some e ->
  exists v, nth_error vals i = Some v /\ eval_rexpr env e = Some v.
Proof.
  intros env exprs. revert env.
  induction exprs as [|e es IH]; intros env vals Heval i ei Hi.
  - destruct i; simpl in Hi; discriminate.
  - simpl in Heval.
    destruct (eval_rexpr env e) eqn:He; [|discriminate].
    destruct (eval_all env es) eqn:Hes; [|discriminate].
    inversion Heval. subst vals.
    destruct i as [|i'].
    + simpl in Hi. inversion Hi. subst ei. exists v. split; [reflexivity | exact He].
    + simpl in Hi. simpl. exact (IH env l Hes i' ei Hi).
Qed.

End Rebinding.


(* ================================================================ *)
(* G. Concrete Examples (St = nat, R = nat)                          *)
(* ================================================================ *)

(* --- Factorial: fact(n, acc) = if n <= 0 then acc else fact(n-1, n*acc) --- *)

(* State = (n, acc) encoded as nat * nat *)
Definition fact_is_base (s : nat * nat) : bool := Nat.eqb (fst s) 0.
Definition fact_base_val (s : nat * nat) : nat := snd s.
Definition fact_step (s : nat * nat) : nat * nat :=
  (fst s - 1, fst s * snd s).

Example ex_fact_rec_5 :
  eval_rec _ _ fact_is_base fact_base_val fact_step 6 (5, 1) = Some 120.
Proof. reflexivity. Qed.

Example ex_fact_loop_5 :
  eval_loop _ _ fact_is_base fact_base_val fact_step 6 (5, 1) = Some 120.
Proof. reflexivity. Qed.

(* Both agree (instance of the general theorem) *)
Example ex_fact_equiv :
  eval_rec _ _ fact_is_base fact_base_val fact_step 6 (5, 1) =
  eval_loop _ _ fact_is_base fact_base_val fact_step 6 (5, 1).
Proof. reflexivity. Qed.


(* --- Countdown: count(n) = if n = 0 then 0 else count(n-1) --- *)

Definition cd_is_base (n : nat) : bool := Nat.eqb n 0.
Definition cd_base_val (n : nat) : nat := 0.
Definition cd_step (n : nat) : nat := n - 1.

Example ex_cd_rec_10 :
  eval_rec _ _ cd_is_base cd_base_val cd_step 11 10 = Some 0.
Proof. reflexivity. Qed.

Example ex_cd_loop_10 :
  eval_loop _ _ cd_is_base cd_base_val cd_step 11 10 = Some 0.
Proof. reflexivity. Qed.


(* --- Rebinding example: swap x and y --- *)

(* env: x=0 -> 1, x=1 -> 2 *)
Definition swap_env : Env nat := [(0, 1); (1, 2)].

(* Two-phase swap: evaluate both with original env, then bind *)
(* x(0) gets y's original value (2), y(1) gets x's original value (1) *)
Example ex_two_phase_swap :
  two_phase nat swap_env [0; 1] [RVar nat 1; RVar nat 0] =
    Some [(0, 2); (1, 1); (0, 1); (1, 2)].
Proof. reflexivity. Qed.

(* Sequential swap: x := y updates x first, then y := x sees new x *)
(* Both end up with value 2 — the swap is broken *)
Example ex_sequential_swap :
  sequential nat swap_env [0; 1] [RVar nat 1; RVar nat 0] =
    Some [(1, 2); (0, 2); (0, 1); (1, 2)].
Proof. reflexivity. Qed.

(* Results differ: two-phase correctly swaps, sequential doesn't *)
Example ex_swap_diverge :
  two_phase nat swap_env [0; 1] [RVar nat 1; RVar nat 0] <>
  sequential nat swap_env [0; 1] [RVar nat 1; RVar nat 0].
Proof. discriminate. Qed.
