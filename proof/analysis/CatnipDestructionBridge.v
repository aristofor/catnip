(* FILE: proof/analysis/CatnipDestructionBridge.v *)
(* CatnipDestructionBridge.v - parallel-copy solver meets the phi semantics
 *
 * Source of truth:
 *   catnip_core/src/cfg/ssa_destruction.rs
 *     (sequentialize_parallel_copies, materialize_phis)
 *
 * CatnipParallelCopyProof establishes the solver's identity on an abstract
 * state (loc = nat): breaking a cycle with one scratch realizes the parallel
 * rotation. CatnipCFGSSACorrectness section H establishes the per-copy facts
 * on the SSA environment (Env = SSAVal -> nat): one copy implements one phi.
 * The two models were not formally connected -- the compositional statement
 * ("the sequentialized batch of one edge realizes ALL the join's phis
 * simultaneously") lived in neither.
 *
 * This file closes that gap. The solver skeleton (chain, cycle break) is
 * restated on Env/SSAVal -- apply_copies from CatnipCFGSSABase is exactly the
 * sequential execution the solver emits -- and the bridge theorems land in
 * the phi vocabulary:
 *
 *   cycle_batch_realizes_phi   : after executing the broken cycle, every
 *                                phi target holds eval_phi of the INITIAL
 *                                environment (the parallel join semantics)
 *   cycle_batch_preserves_others : locations outside the batch (and not the
 *                                scratch) keep their value
 *
 * Proves (12 lemmas/theorems + 1 example, 0 Admitted):
 *   - upd_env_same, upd_env_other, ssaval_eq_dec  (Env copy semantics)
 *   - apply_copies_cons                            (unfolding)
 *   - in_skipn_sv                                  (list helper)
 *   - chain_env_untouched, chain_env_pos           (chain body, universal)
 *   - cycle_break_env_correct, cycle_break_env_outside (n-cycle on Env)
 *   - cycle_batch_realizes_phi                     (the bridge)
 *   - cycle_batch_preserves_others                 (the frame)
 *   - swap_join_demo                               (2-phi join instance)
 *
 * Depends on: CatnipCFGSSABase.
 *)

From Coq Require Import List Arith Lia Bool.
Import ListNotations.
From Catnip Require Import CatnipCFGSSABase.


(* ================================================================ *)
(* A. Copy semantics on Env                                           *)
(*                                                                    *)
(* apply_copies (CatnipCFGSSABase) executes copies left to right,    *)
(* each reading the CURRENT environment -- the same sequential        *)
(* semantics as the solver's emitted list.                            *)
(* ================================================================ *)

Definition sv0 : SSAVal := mkSSAVal 0 0.

Lemma ssaval_eq_dec : forall a b : SSAVal, {a = b} + {a <> b}.
Proof.
  intros a b. destruct (ssaval_eqb a b) eqn:He.
  - left. apply ssaval_eqb_eq. exact He.
  - right. intro Heq. subst. rewrite ssaval_eqb_refl in He. discriminate.
Qed.

Lemma upd_env_same : forall rho v x, update_env rho v x v = x.
Proof.
  intros. unfold update_env. rewrite ssaval_eqb_refl. reflexivity.
Qed.

Lemma upd_env_other : forall rho v x w, w <> v -> update_env rho v x w = rho w.
Proof.
  intros rho v x w H. unfold update_env.
  destruct (ssaval_eqb w v) eqn:He.
  - exfalso. apply H. apply ssaval_eqb_eq. exact He.
  - reflexivity.
Qed.

Lemma apply_copies_cons : forall d s copies rho,
  apply_copies ((d, s) :: copies) rho
    = apply_copies copies (update_env rho d (rho s)).
Proof. reflexivity. Qed.


(* ================================================================ *)
(* B. List helper (SSAVal instance of in_skipn)                       *)
(* ================================================================ *)

Lemma in_skipn_sv : forall n (l : list SSAVal) a,
  In a (skipn n l) -> In a l.
Proof.
  induction n as [|n IH]; intros l a H.
  - simpl in H. exact H.
  - destruct l as [|x l'].
    + simpl in H. contradiction.
    + simpl in H. right. apply (IH l' a H).
Qed.


(* ================================================================ *)
(* C. The solver's cycle break, on Env                                *)
(*                                                                    *)
(* A cycle over distinct SSA values xs with a fresh scratch t is      *)
(* serialized as (t, x0) :: chain xs t, mirroring                     *)
(* sequentialize_parallel_copies. Same skeleton as the nat model,     *)
(* transposed onto update_env / apply_copies.                         *)
(* ================================================================ *)

Fixpoint chain_env (xs : list SSAVal) (t : SSAVal) : list (SSAVal * SSAVal) :=
  match xs with
  | [] => []
  | x :: xs' => (x, hd t xs') :: chain_env xs' t
  end.

Lemma chain_env_untouched : forall xs t rho z,
  ~ In z xs -> apply_copies (chain_env xs t) rho z = rho z.
Proof.
  induction xs as [|x xs' IH]; intros t rho z Hz.
  - simpl. reflexivity.
  - assert (z <> x /\ ~ In z xs') as [Hzx Hz'].
    { split.
      - intro H. apply Hz. left. symmetry. exact H.
      - intro H. apply Hz. right. exact H. }
    cbn [chain_env]. rewrite apply_copies_cons.
    rewrite (IH t (update_env rho x (rho (hd t xs'))) z Hz').
    apply upd_env_other. exact Hzx.
Qed.

Lemma chain_env_pos : forall xs t rho i,
  NoDup xs -> ~ In t xs -> i < length xs ->
  apply_copies (chain_env xs t) rho (nth i xs sv0)
    = rho (hd t (skipn (S i) xs)).
Proof.
  intros xs. induction xs as [|x xs' IH]; intros t rho i Hnd Ht Hi.
  - simpl in Hi. lia.
  - apply NoDup_cons_iff in Hnd. destruct Hnd as [Hxni Hnd'].
    assert (Htx : t <> x).
    { intro H. apply Ht. left. symmetry. exact H. }
    assert (Htxs : ~ In t xs').
    { intro H. apply Ht. right. exact H. }
    cbn [chain_env]. rewrite apply_copies_cons.
    destruct i as [|j].
    + cbn [nth skipn].
      rewrite (chain_env_untouched xs' t
                 (update_env rho x (rho (hd t xs'))) x Hxni).
      rewrite upd_env_same. reflexivity.
    + cbn [nth]. cbn [length] in Hi.
      assert (Hj : j < length xs') by lia.
      rewrite (IH t (update_env rho x (rho (hd t xs'))) j Hnd' Htxs Hj).
      change (skipn (S (S j)) (x :: xs')) with (skipn (S j) xs').
      apply upd_env_other.
      destruct (skipn (S j) xs') as [|y ys] eqn:Hsk.
      * cbn [hd]. exact Htx.
      * cbn [hd]. intro Hyx. subst y. apply Hxni.
        apply (in_skipn_sv (S j) xs' x). rewrite Hsk. left. reflexivity.
Qed.

(* The n-cycle break on Env: position i ends holding the INITIAL value of
   its successor, the last wrapping to x0 through the scratch. *)
Theorem cycle_break_env_correct : forall xs t rho,
  NoDup xs -> ~ In t xs ->
  forall i, i < length xs ->
    apply_copies ((t, hd sv0 xs) :: chain_env xs t) rho (nth i xs sv0)
      = rho (hd (hd sv0 xs) (skipn (S i) xs)).
Proof.
  intros xs t rho Hnd Ht i Hi.
  rewrite apply_copies_cons.
  set (rho1 := update_env rho t (rho (hd sv0 xs))).
  transitivity (rho1 (hd t (skipn (S i) xs))).
  { apply (chain_env_pos xs t rho1 i Hnd Ht Hi). }
  destruct (skipn (S i) xs) as [|y ys] eqn:Hsk.
  - cbn [hd]. unfold rho1. rewrite upd_env_same. reflexivity.
  - cbn [hd]. unfold rho1. apply upd_env_other.
    intro Hyt. subst y. apply Ht.
    apply (in_skipn_sv (S i) xs t). rewrite Hsk. left. reflexivity.
Qed.

Theorem cycle_break_env_outside : forall xs t rho z,
  ~ In z xs -> z <> t ->
  apply_copies ((t, hd sv0 xs) :: chain_env xs t) rho z = rho z.
Proof.
  intros xs t rho z Hz Hzt.
  rewrite apply_copies_cons.
  transitivity (update_env rho t (rho (hd sv0 xs)) z).
  { apply (chain_env_untouched xs t _ z Hz). }
  apply upd_env_other. exact Hzt.
Qed.


(* ================================================================ *)
(* D. The bridge: batch execution realizes the phi semantics          *)
(*                                                                    *)
(* An edge batch that forms a cycle: the i-th phi's target is xs[i]   *)
(* and its incoming value at this predecessor is xs[i+1] (the last    *)
(* wrapping to xs[0]) -- exactly the shape whose naive serialization  *)
(* loses a value. After executing the broken sequence, every phi      *)
(* target holds eval_phi of the INITIAL environment: the sequential   *)
(* list implements the parallel join semantics.                       *)
(* ================================================================ *)

Theorem cycle_batch_realizes_phi : forall xs t rho p pred_idx i,
  NoDup xs -> ~ In t xs -> i < length xs ->
  phi_val p = nth i xs sv0 ->
  nth_error (phi_incoming p) pred_idx
    = Some (hd (hd sv0 xs) (skipn (S i) xs)) ->
  apply_copies ((t, hd sv0 xs) :: chain_env xs t) rho (phi_val p)
    = eval_phi rho p pred_idx.
Proof.
  intros xs t rho p pred_idx i Hnd Ht Hi Hval Hinc.
  rewrite Hval.
  rewrite (cycle_break_env_correct xs t rho Hnd Ht i Hi).
  unfold eval_phi. rewrite Hinc. reflexivity.
Qed.

(* SSA values outside the batch (and not the scratch) are untouched:
   the join rewrites exactly its phi targets, nothing else. *)
Theorem cycle_batch_preserves_others : forall xs t rho z,
  ~ In z xs -> z <> t ->
  apply_copies ((t, hd sv0 xs) :: chain_env xs t) rho z = rho z.
Proof.
  exact cycle_break_env_outside.
Qed.


(* ================================================================ *)
(* E. Instance: a two-phi swap join                                   *)
(*                                                                    *)
(* Join with phis a := phi(..., b, ...) and b := phi(..., a, ...):    *)
(* the edge batch {a <- b, b <- a} is the swap. Broken with a fresh   *)
(* scratch, both phis receive their parallel value.                   *)
(* ================================================================ *)

Example swap_join_demo :
  forall (rho : Env) (a b t : SSAVal) (pa pb : Phi),
  a <> b -> t <> a -> t <> b ->
  phi_val pa = a -> phi_incoming pa = [b] ->
  phi_val pb = b -> phi_incoming pb = [a] ->
  let seq := (t, a) :: chain_env [a; b] t in
  apply_copies seq rho (phi_val pa) = eval_phi rho pa 0
  /\ apply_copies seq rho (phi_val pb) = eval_phi rho pb 0.
Proof.
  intros rho a b t pa pb Hab Hta Htb Hva Hia Hvb Hib seq.
  assert (Hnd : NoDup [a; b]).
  { constructor.
    - intros [H|[]]. exact (Hab (eq_sym H)).
    - constructor; [intros []|constructor]. }
  assert (Ht : ~ In t [a; b]).
  { intros [H|[H|[]]]; [exact (Hta (eq_sym H))|exact (Htb (eq_sym H))]. }
  split.
  - apply (cycle_batch_realizes_phi [a; b] t rho pa 0 0 Hnd Ht).
    + simpl; lia.
    + simpl; exact Hva.
    + rewrite Hia. reflexivity.
  - apply (cycle_batch_realizes_phi [a; b] t rho pb 0 1 Hnd Ht).
    + simpl; lia.
    + simpl; exact Hvb.
    + rewrite Hib. reflexivity.
Qed.
