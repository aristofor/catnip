(* FILE: proof/analysis/CatnipLivenessLinear.v *)
(* Linear block liveness analysis, execution model, and local DSE.
 *
 * Scope: linear blocks only (no CFG).
 *
 * Proves:
 *   - USE/DEF sets per instruction
 *   - Backward transfer function monotonicity
 *   - One-pass backward liveness (live_states, live_in)
 *   - Fuel-bounded fixpoint scaffold (iterate_to_fixpoint)
 *   - Small-step execution model (State, eq_on, exec_instr, exec_block)
 *   - Liveness soundness: live_in_sound
 *   - Local DSE correctness: dse_linear_correct
 *
 * Depends on: CatnipVarSet.v
 *)

From Catnip Require Export CatnipVarSet.
From Coq Require Import List Bool Lia Arith.PeanoNat ZArith.
Import ListNotations.


(* ================================================================ *)
(* C. Linear instructions + USE/DEF + transfer                       *)
(* ================================================================ *)

Inductive Instr :=
  | Assign : Var -> VarSet -> Instr     (* x := expr, where expr uses variables *)
  | Effect : VarSet -> Instr.           (* side-effecting instruction that only reads *)

Definition use_set (i : Instr) : VarSet :=
  match i with
  | Assign _ uses => uses
  | Effect uses => uses
  end.

Definition def_set (i : Instr) : VarSet :=
  match i with
  | Assign x _ => [x]
  | Effect _ => []
  end.

Definition transfer (i : Instr) (live_out : VarSet) : VarSet :=
  union (use_set i) (remove_list (def_set i) live_out).

Lemma transfer_monotone : forall i o1 o2,
  subset o1 o2 -> subset (transfer i o1) (transfer i o2).
Proof.
  intros i o1 o2 Hsub.
  unfold transfer.
  apply union_monotone_right.
  apply remove_list_monotone.
  exact Hsub.
Qed.


(* ================================================================ *)
(* D. Backward liveness on a linear block                            *)
(*
 * live_states returns [in_0; in_1; ...; in_n], where in_n = live_out.
 * Length is S (length code).
 *)
(* ================================================================ *)

Fixpoint live_states (code : list Instr) (live_out : VarSet) : list VarSet :=
  match code with
  | [] => [live_out]
  | i :: rest =>
      let states_rest := live_states rest live_out in
      match states_rest with
      | [] => []  (* unreachable: live_states rest is never empty *)
      | in_next :: _ => transfer i in_next :: states_rest
      end
  end.

Definition live_in (code : list Instr) (live_out : VarSet) : VarSet :=
  match live_states code live_out with
  | [] => live_out
  | hd :: _ => hd
  end.

Lemma live_states_nonempty : forall code out,
  live_states code out <> [].
Proof.
  induction code as [|i rest IH]; intros out.
  - simpl. discriminate.
  - simpl.
    destruct (live_states rest out) as [|s ss] eqn:Hls.
    + exfalso. exact (IH out Hls).
    + discriminate.
Qed.

Lemma live_states_length : forall code out,
  length (live_states code out) = S (length code).
Proof.
  induction code as [|i rest IH]; intro out.
  - simpl. reflexivity.
  - simpl.
    destruct (live_states rest out) as [|s ss] eqn:Hls.
    + exfalso. pose proof (live_states_nonempty rest out) as Hne. contradiction.
    + specialize (IH out).
      rewrite Hls in IH.
      simpl in IH.
      simpl. rewrite IH. reflexivity.
Qed.

Lemma live_in_cons : forall i rest out,
  live_in (i :: rest) out = transfer i (live_in rest out).
Proof.
  intros i rest out.
  unfold live_in.
  simpl.
  destruct (live_states rest out) as [|next tail] eqn:Hls.
  - exfalso.
    pose proof (live_states_nonempty rest out) as Hne.
    exact (Hne Hls).
  - reflexivity.
Qed.


(* ================================================================ *)
(* E. Fuel-bounded fixpoint scaffold (for future CFG step)           *)
(* ================================================================ *)

Fixpoint varset_eqb (a b : VarSet) : bool :=
  match a, b with
  | [], [] => true
  | x :: xs, y :: ys => Nat.eqb x y && varset_eqb xs ys
  | _, _ => false
  end.

Fixpoint iterate_to_fixpoint
  (fuel : nat)
  (f : VarSet -> VarSet)
  (s : VarSet) : VarSet :=
  match fuel with
  | 0 => s
  | S fuel' =>
      let s' := f s in
      if varset_eqb s s' then s else iterate_to_fixpoint fuel' f s'
  end.

Lemma varset_eqb_refl : forall s,
  varset_eqb s s = true.
Proof.
  induction s as [|x xs IH].
  - simpl. reflexivity.
  - simpl. rewrite Nat.eqb_refl. rewrite IH. reflexivity.
Qed.

Lemma varset_eqb_eq : forall a b,
  varset_eqb a b = true -> a = b.
Proof.
  induction a as [|x xs IH]; intros [|y ys] Heq; simpl in Heq;
    try discriminate; try reflexivity.
  apply andb_true_iff in Heq as [Hxy Htl].
  apply Nat.eqb_eq in Hxy. subst.
  f_equal. apply IH. exact Htl.
Qed.

Lemma iterate_to_fixpoint_stable : forall fuel f s,
  f s = s -> iterate_to_fixpoint fuel f s = s.
Proof.
  induction fuel as [|fuel IH]; intros f s Hstable.
  - simpl. reflexivity.
  - simpl. rewrite Hstable. rewrite varset_eqb_refl. reflexivity.
Qed.

(* When the fixpoint check passes, the result is genuinely stable *)
Lemma iterate_to_fixpoint_is_fixpoint : forall fuel f s,
  let r := iterate_to_fixpoint fuel f s in
  varset_eqb r (f r) = true ->
  f r = r.
Proof.
  intros fuel f s r Heq.
  apply varset_eqb_eq in Heq. symmetry. exact Heq.
Qed.


(* ================================================================ *)
(* F. Small-step execution model (linear blocks)                     *)
(* ================================================================ *)

Definition State := Var -> Z.

Definition eq_on (live : VarSet) (s1 s2 : State) : Prop :=
  forall x, In x live -> s1 x = s2 x.

Definition update (st : State) (x : Var) (v : Z) : State :=
  fun y => if Nat.eqb y x then v else st y.

Fixpoint eval_uses (uses : VarSet) (st : State) : Z :=
  match uses with
  | [] => 0%Z
  | x :: xs => (st x + eval_uses xs st)%Z
  end.

Definition exec_instr (st : State) (i : Instr) : State :=
  match i with
  | Assign x uses => update st x (eval_uses uses st)
  | Effect _ => st
  end.

Fixpoint exec_block (code : list Instr) (st : State) : State :=
  match code with
  | [] => st
  | i :: rest => exec_block rest (exec_instr st i)
  end.

Lemma eq_on_refl : forall live st,
  eq_on live st st.
Proof.
  intros live st x Hx. reflexivity.
Qed.

Lemma eq_on_sym : forall live s1 s2,
  eq_on live s1 s2 -> eq_on live s2 s1.
Proof.
  intros live s1 s2 Heq x Hx.
  symmetry. apply Heq. exact Hx.
Qed.

Lemma eq_on_trans : forall live s1 s2 s3,
  eq_on live s1 s2 ->
  eq_on live s2 s3 ->
  eq_on live s1 s3.
Proof.
  intros live s1 s2 s3 H12 H23 x Hx.
  rewrite H12 by exact Hx.
  apply H23. exact Hx.
Qed.

Lemma eq_on_subset : forall a b s1 s2,
  subset a b ->
  eq_on b s1 s2 ->
  eq_on a s1 s2.
Proof.
  intros a b s1 s2 Hsub Heq x Hx.
  apply Heq. apply Hsub. exact Hx.
Qed.

Lemma update_eq : forall st x v,
  update st x v x = v.
Proof.
  intros st x v.
  unfold update.
  rewrite Nat.eqb_refl.
  reflexivity.
Qed.

Lemma update_neq : forall st x y v,
  x <> y ->
  update st x v y = st y.
Proof.
  intros st x y v Hneq.
  unfold update.
  rewrite Nat.eqb_sym.
  apply Nat.eqb_neq in Hneq.
  rewrite Hneq.
  reflexivity.
Qed.

Lemma eval_uses_eq : forall uses s1 s2,
  eq_on uses s1 s2 ->
  eval_uses uses s1 = eval_uses uses s2.
Proof.
  intros uses s1 s2 Heq.
  induction uses as [|x xs IH].
  - reflexivity.
  - simpl.
    rewrite Heq by (left; reflexivity).
    rewrite IH.
    reflexivity.
    intros y Hy. apply Heq. right. exact Hy.
Qed.

Lemma eq_on_update_notin : forall live st x v,
  ~ In x live ->
  eq_on live (update st x v) st.
Proof.
  intros live st x v Hnot y Hy.
  unfold update.
  destruct (Nat.eqb y x) eqn:Hyx.
  - apply Nat.eqb_eq in Hyx. subst y. exfalso. apply Hnot. exact Hy.
  - reflexivity.
Qed.

Lemma eq_on_empty : forall s1 s2,
  eq_on [] s1 s2.
Proof.
  intros s1 s2 x Hx. inversion Hx.
Qed.

Lemma transfer_sound_instr : forall i live_out s1 s2,
  eq_on (transfer i live_out) s1 s2 ->
  eq_on live_out (exec_instr s1 i) (exec_instr s2 i).
Proof.
  intros i live_out s1 s2 Heq.
  destruct i as [x uses | uses].
  - simpl. intros y Hy.
    destruct (Nat.eqb y x) eqn:Hyx.
    + apply Nat.eqb_eq in Hyx. subst y.
      rewrite update_eq.
      rewrite update_eq.
      apply eval_uses_eq.
      apply eq_on_subset with (b := transfer (Assign x uses) live_out).
      * unfold transfer.
        apply subset_union_left.
      * exact Heq.
    + apply Nat.eqb_neq in Hyx.
      rewrite update_neq by (intro H; apply Hyx; symmetry; exact H).
      rewrite update_neq by (intro H; apply Hyx; symmetry; exact H).
      apply Heq.
      unfold transfer.
      apply subset_union_right with (a := uses) (b := remove_var x live_out).
      apply in_remove_var_intro; assumption.
  - simpl. intros y Hy.
    unfold exec_instr.
    apply Heq.
    unfold transfer.
    simpl.
    apply subset_union_right with (a := uses) (b := live_out).
    exact Hy.
Qed.

Theorem live_in_sound : forall code live_out s1 s2,
  eq_on (live_in code live_out) s1 s2 ->
  eq_on live_out (exec_block code s1) (exec_block code s2).
Proof.
  induction code as [|i rest IH]; intros live_out s1 s2 Heq.
  - simpl in *. exact Heq.
  - simpl in *.
    remember (live_states rest live_out) as tail eqn:Htail.
    destruct tail as [|next tail'].
    + exfalso.
      pose proof (live_states_nonempty rest live_out) as Hne.
      exact (Hne (eq_sym Htail)).
    + assert (Hlin_rest : live_in rest live_out = next).
      { unfold live_in. rewrite <- Htail. reflexivity. }
      assert (Hlin_cur : live_in (i :: rest) live_out = transfer i next).
      { unfold live_in. simpl. rewrite <- Htail. reflexivity. }
      apply IH.
      rewrite Hlin_rest.
      apply transfer_sound_instr.
      rewrite Hlin_cur in Heq.
      exact Heq.
Qed.


(* ================================================================ *)
(* G. Local DSE + correctness on live-out variables                  *)
(* ================================================================ *)

Fixpoint dse_linear (code : list Instr) (live_out : VarSet) : VarSet * list Instr :=
  match code with
  | [] => (live_out, [])
  | i :: rest =>
      let '(live_rest, rest') := dse_linear rest live_out in
      let live_here := transfer i live_rest in
      match i with
      | Assign x _ =>
          if mem x live_rest then (live_here, i :: rest')
          else (live_here, rest')
      | Effect _ => (live_here, i :: rest')
      end
  end.

Lemma dse_linear_fst_live_in : forall code live_out,
  fst (dse_linear code live_out) = live_in code live_out.
Proof.
  induction code as [|i rest IH]; intros live_out.
  - reflexivity.
  - simpl.
    destruct (dse_linear rest live_out) as [live_rest rest'] eqn:Hdse.
    simpl.
    specialize (IH live_out).
    rewrite Hdse in IH.
    simpl in IH.
    rewrite IH.
    rewrite live_in_cons.
    destruct i as [x uses | uses]; simpl.
    + destruct (mem x (live_in rest live_out)); reflexivity.
    + reflexivity.
Qed.

Theorem dse_linear_correct : forall code live_out st,
  eq_on live_out (exec_block code st) (exec_block (snd (dse_linear code live_out)) st).
Proof.
  induction code as [|i rest IH]; intros live_out st.
  - simpl. apply eq_on_refl.
  - simpl.
    destruct (dse_linear rest live_out) as [live_rest rest'] eqn:Hdse.
    destruct i as [x uses | uses].
    + simpl.
      destruct (mem x live_rest) eqn:Hmem.
      * (* kept assign *)
        specialize (IH live_out (exec_instr st (Assign x uses))).
        simpl in IH.
        rewrite Hdse in IH.
        exact IH.
      * (* removed assign *)
        assert (Hlive_rest : live_rest = live_in rest live_out).
        { rewrite <- dse_linear_fst_live_in.
          rewrite Hdse.
          reflexivity. }
        assert (Heq_in : eq_on live_rest (exec_instr st (Assign x uses)) st).
        { rewrite Hlive_rest.
          apply eq_on_update_notin.
          rewrite Hlive_rest in Hmem.
          apply mem_false_notin. exact Hmem. }
        assert (Hrest_same_live :
                  eq_on live_out
                    (exec_block rest (exec_instr st (Assign x uses)))
                    (exec_block rest st)).
        { apply live_in_sound.
          rewrite <- Hlive_rest.
          exact Heq_in. }
        assert (Hrest_opt_live :
                  eq_on live_out (exec_block rest st) (exec_block rest' st)).
        { specialize (IH live_out st).
          simpl in IH.
          rewrite Hdse in IH.
          exact IH. }
        apply eq_on_trans with (s2 := exec_block rest st).
        { exact Hrest_same_live. }
        { exact Hrest_opt_live. }
    + simpl.
      specialize (IH live_out (exec_instr st (Effect uses))).
      simpl in IH.
      rewrite Hdse in IH.
      exact IH.
Qed.
