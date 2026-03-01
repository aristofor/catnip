(* FILE: proof/analysis/CatnipLivenessCFG.v *)
(* CFG-level liveness analysis, fixpoint iteration, CFG DSE,
 * and path-level semantic preservation.
 *
 * Proves:
 *   - Mini-CFG liveness model (LiveMap, step_in, iterate_cfg)
 *   - step_in monotonicity
 *   - Fixpoint verification (iterate_cfg_is_fixpoint)
 *   - NoDup preservation (transfer_sets, collect_succ_in)
 *   - CFG-local DSE criterion (is_dead_assign_cfg, dse_instr_cfg)
 *   - DSE soundness per instruction (dse_instr_cfg_sound)
 *   - Path-level composition (exec_path_sound)
 *
 * Depends on: CatnipLivenessLinear.v (CatnipVarSet.v transitively)
 *)

From Catnip Require Export CatnipLivenessLinear.
From Coq Require Import List Bool Lia Arith.PeanoNat ZArith.
Import ListNotations.


(* ================================================================ *)
(* H. Mini-CFG liveness + fuel-bounded fixpoint                      *)
(* ================================================================ *)

Definition LiveMap := list VarSet.

Fixpoint lmap_get (i : nat) (m : LiveMap) : VarSet :=
  match i, m with
  | 0, x :: _ => x
  | S k, _ :: tl => lmap_get k tl
  | _, _ => []
  end.

Definition lmap_subset (m1 m2 : LiveMap) : Prop :=
  forall i x, In x (lmap_get i m1) -> In x (lmap_get i m2).

Fixpoint collect_succ_in (succs : list nat) (in_map : LiveMap) : VarSet :=
  match succs with
  | [] => []
  | s :: tl => union (lmap_get s in_map) (collect_succ_in tl in_map)
  end.

Definition transfer_sets (uses defs live_out : VarSet) : VarSet :=
  union uses (remove_list defs live_out).

Fixpoint step_in
  (uses_tbl defs_tbl : list VarSet)
  (succ_tbl : list (list nat))
  (in_map : LiveMap) : LiveMap :=
  match uses_tbl, defs_tbl, succ_tbl with
  | uses :: ur, defs :: dr, succs :: sr =>
      let out_n := collect_succ_in succs in_map in
      transfer_sets uses defs out_n :: step_in ur dr sr in_map
  | _, _, _ => []
  end.

Fixpoint lmap_eqb (m1 m2 : LiveMap) : bool :=
  match m1, m2 with
  | [], [] => true
  | s1 :: t1, s2 :: t2 => varset_eqb s1 s2 && lmap_eqb t1 t2
  | _, _ => false
  end.

Fixpoint iterate_cfg
  (fuel : nat)
  (uses_tbl defs_tbl : list VarSet)
  (succ_tbl : list (list nat))
  (in_map : LiveMap) : LiveMap :=
  match fuel with
  | 0 => in_map
  | S fuel' =>
      let next := step_in uses_tbl defs_tbl succ_tbl in_map in
      if lmap_eqb in_map next then in_map
      else iterate_cfg fuel' uses_tbl defs_tbl succ_tbl next
  end.

Lemma transfer_sets_monotone : forall uses defs o1 o2,
  subset o1 o2 ->
  subset (transfer_sets uses defs o1) (transfer_sets uses defs o2).
Proof.
  intros uses defs o1 o2 Hsub.
  unfold transfer_sets.
  apply union_monotone_right.
  apply remove_list_monotone.
  exact Hsub.
Qed.

Lemma collect_succ_in_monotone : forall succs m1 m2,
  lmap_subset m1 m2 ->
  subset (collect_succ_in succs m1) (collect_succ_in succs m2).
Proof.
  induction succs as [|s tl IH]; intros m1 m2 Hsub.
  - intros x Hx. inversion Hx.
  - intros x Hx.
    simpl in Hx |- *.
    rewrite union_spec in Hx.
    rewrite union_spec.
    destruct Hx as [Hin_head | Hin_tail].
    + left. apply Hsub. exact Hin_head.
    + right. eapply IH.
      * exact Hsub.
      * exact Hin_tail.
Qed.

Lemma step_in_monotone : forall uses_tbl defs_tbl succ_tbl m1 m2,
  lmap_subset m1 m2 ->
  lmap_subset (step_in uses_tbl defs_tbl succ_tbl m1)
              (step_in uses_tbl defs_tbl succ_tbl m2).
Proof.
  induction uses_tbl as [|uses ur IH];
    intros defs_tbl succ_tbl m1 m2 Hsub i x Hx.
  - destruct i; simpl in Hx; contradiction.
  - destruct defs_tbl as [|defs dr].
    + destruct i; simpl in Hx; contradiction.
    + destruct succ_tbl as [|succs sr].
      * destruct i; simpl in Hx; contradiction.
      * simpl in Hx.
        destruct i as [|k].
        { eapply transfer_sets_monotone.
          - apply collect_succ_in_monotone. exact Hsub.
          - exact Hx. }
        { eapply IH.
          - exact Hsub.
          - exact Hx. }
Qed.

Lemma lmap_eqb_refl : forall m,
  lmap_eqb m m = true.
Proof.
  induction m as [|s tl IH].
  - reflexivity.
  - simpl. rewrite varset_eqb_refl. rewrite IH. reflexivity.
Qed.

Lemma lmap_eqb_eq : forall m1 m2,
  lmap_eqb m1 m2 = true -> m1 = m2.
Proof.
  induction m1 as [|s1 t1 IH]; intros [|s2 t2] Heq; simpl in Heq;
    try discriminate; try reflexivity.
  apply andb_true_iff in Heq as [Hs Ht].
  apply varset_eqb_eq in Hs. subst.
  f_equal. apply IH. exact Ht.
Qed.

Lemma iterate_cfg_stable : forall fuel uses_tbl defs_tbl succ_tbl in_map,
  step_in uses_tbl defs_tbl succ_tbl in_map = in_map ->
  iterate_cfg fuel uses_tbl defs_tbl succ_tbl in_map = in_map.
Proof.
  induction fuel as [|fuel IH]; intros uses_tbl defs_tbl succ_tbl in_map Hstable.
  - reflexivity.
  - simpl. rewrite Hstable. rewrite lmap_eqb_refl. reflexivity.
Qed.

Lemma step_in_equation_head : forall uses defs succs ur dr sr in_map,
  step_in (uses :: ur) (defs :: dr) (succs :: sr) in_map =
    transfer_sets uses defs (collect_succ_in succs in_map) ::
    step_in ur dr sr in_map.
Proof.
  reflexivity.
Qed.


(* ================================================================ *)
(* H.2 Fixpoint verification                                        *)
(*                                                                    *)
(* When iterate_cfg's boolean check passes, the result is a genuine  *)
(* fixpoint of step_in.                                               *)
(* Note: unconditional convergence requires a canonical VarSet       *)
(* representation (sorted or hash-set) to avoid list-order           *)
(* oscillation. The current list-based model can oscillate between   *)
(* set-equivalent lists in different order, preventing lmap_eqb      *)
(* from detecting the fixpoint.                                       *)
(* ================================================================ *)

(* Total size metric for convergence analysis *)
Definition lmap_total_size (m : LiveMap) : nat :=
  fold_right (fun s acc => length s + acc) 0 m.

(* When iterate_cfg's eqb check passes, the result is a genuine fixpoint *)
Theorem iterate_cfg_is_fixpoint : forall fuel uses_tbl defs_tbl succ_tbl in_map,
  let result := iterate_cfg fuel uses_tbl defs_tbl succ_tbl in_map in
  lmap_eqb result (step_in uses_tbl defs_tbl succ_tbl result) = true ->
  step_in uses_tbl defs_tbl succ_tbl result = result.
Proof.
  intros fuel uses_tbl defs_tbl succ_tbl in_map result Heq.
  apply lmap_eqb_eq in Heq. symmetry. exact Heq.
Qed.

(* lmap_get on empty map is always [] *)
Lemma lmap_get_nil : forall i, lmap_get i [] = [].
Proof. destruct i; reflexivity. Qed.

(* collect_succ_in from empty map returns [] *)
Lemma collect_succ_in_nil : forall succs,
  collect_succ_in succs [] = [].
Proof.
  induction succs as [|s tl IH].
  - reflexivity.
  - simpl. rewrite lmap_get_nil. simpl. exact IH.
Qed.

(* Empty LiveMap is a subset of any LiveMap *)
Lemma lmap_subset_nil : forall m, lmap_subset [] m.
Proof.
  intros m i x Hx. rewrite lmap_get_nil in Hx. inversion Hx.
Qed.

(* lmap_total_size is 0 for empty map *)
Lemma lmap_total_size_nil : lmap_total_size [] = 0.
Proof. reflexivity. Qed.

(* lmap_total_size distributes over cons *)
Lemma lmap_total_size_cons : forall s m,
  lmap_total_size (s :: m) = length s + lmap_total_size m.
Proof. reflexivity. Qed.

(* NoDup preservation for transfer_sets *)
Lemma transfer_sets_NoDup : forall uses defs out,
  NoDup out -> NoDup (transfer_sets uses defs out).
Proof.
  intros uses defs out Hnd.
  unfold transfer_sets.
  apply union_NoDup.
  apply remove_list_NoDup.
  exact Hnd.
Qed.

(* NoDup preservation for collect_succ_in *)
Lemma collect_succ_in_NoDup : forall succs m,
  (forall i, NoDup (lmap_get i m)) ->
  NoDup (collect_succ_in succs m).
Proof.
  induction succs as [|s tl IH]; intros m Hnd.
  - simpl. constructor.
  - simpl. apply union_NoDup. apply IH. exact Hnd.
Qed.


(* ================================================================ *)
(* I. CFG-local DSE criterion + local semantic preservation          *)
(* ================================================================ *)

Definition out_of_node (succs : list nat) (in_map : LiveMap) : VarSet :=
  collect_succ_in succs in_map.

Definition is_dead_assign_cfg (i : Instr) (succs : list nat) (in_map : LiveMap) : bool :=
  match i with
  | Assign x _ => negb (mem x (out_of_node succs in_map))
  | Effect _ => false
  end.

Definition dse_instr_cfg (i : Instr) (succs : list nat) (in_map : LiveMap) : Instr :=
  if is_dead_assign_cfg i succs in_map then Effect [] else i.

Lemma dead_assign_exec_preserves_out : forall st x uses out,
  ~ In x out ->
  eq_on out (exec_instr st (Assign x uses)) st.
Proof.
  intros st x uses out Hnot.
  unfold exec_instr.
  apply eq_on_update_notin.
  exact Hnot.
Qed.

Lemma dse_instr_cfg_sound : forall st i succs in_map,
  eq_on (out_of_node succs in_map)
        (exec_instr st i)
        (exec_instr st (dse_instr_cfg i succs in_map)).
Proof.
  intros st i succs in_map.
  unfold dse_instr_cfg.
  destruct i as [x uses | uses].
  - simpl.
    destruct (negb (mem x (out_of_node succs in_map))) eqn:Hdead.
    + apply negb_true_iff in Hdead.
      apply mem_false_notin in Hdead.
      apply dead_assign_exec_preserves_out.
      exact Hdead.
    + apply eq_on_refl.
  - simpl. apply eq_on_refl.
Qed.

Fixpoint dse_cfg
  (code : list Instr)
  (succ_tbl : list (list nat))
  (in_map : LiveMap) : list Instr :=
  match code, succ_tbl with
  | i :: rest, succs :: sr => dse_instr_cfg i succs in_map :: dse_cfg rest sr in_map
  | _, _ => []
  end.

Lemma dse_cfg_length : forall code succ_tbl in_map,
  length (dse_cfg code succ_tbl in_map) = Nat.min (length code) (length succ_tbl).
Proof.
  induction code as [|i rest IH]; intros succ_tbl in_map.
  - reflexivity.
  - destruct succ_tbl as [|s sr].
    + reflexivity.
    + simpl. rewrite IH. reflexivity.
Qed.

Lemma dse_cfg_sound_head : forall st i rest succs sr in_map,
  eq_on (out_of_node succs in_map)
        (exec_instr st i)
        (exec_instr st (hd (Effect []) (dse_cfg (i :: rest) (succs :: sr) in_map))).
Proof.
  intros st i rest succs sr in_map.
  simpl.
  apply dse_instr_cfg_sound.
Qed.


(* ================================================================ *)
(* J. Path-level composition (mini CFG semantics)                    *)
(* ================================================================ *)

Definition instr_at (code : list Instr) (n : nat) : Instr :=
  nth n code (Effect []).

Definition succs_at (succ_tbl : list (list nat)) (n : nat) : list nat :=
  nth n succ_tbl [].

Definition out_at (succ_tbl : list (list nat)) (in_map : LiveMap) (n : nat) : VarSet :=
  out_of_node (succs_at succ_tbl n) in_map.

Definition live_at (in_map : LiveMap) (n : nat) : VarSet :=
  lmap_get n in_map.

Inductive PathOK (succ_tbl : list (list nat)) : list nat -> Prop :=
  | path_ok_nil : PathOK succ_tbl []
  | path_ok_single : forall n, PathOK succ_tbl [n]
  | path_ok_cons : forall n m tl,
      In m (succs_at succ_tbl n) ->
      PathOK succ_tbl (m :: tl) ->
      PathOK succ_tbl (n :: m :: tl).

Fixpoint exec_path_orig (code : list Instr) (path : list nat) (st : State) : State :=
  match path with
  | [] => st
  | n :: tl => exec_path_orig code tl (exec_instr st (instr_at code n))
  end.

Fixpoint exec_path_opt
  (code : list Instr)
  (succ_tbl : list (list nat))
  (in_map : LiveMap)
  (path : list nat)
  (st : State) : State :=
  match path with
  | [] => st
  | n :: tl =>
      let i := instr_at code n in
      let succs := succs_at succ_tbl n in
      exec_path_opt code succ_tbl in_map tl (exec_instr st (dse_instr_cfg i succs in_map))
  end.

Fixpoint path_target_live
  (succ_tbl : list (list nat))
  (in_map : LiveMap)
  (path : list nat) : VarSet :=
  match path with
  | [] => []
  | [n] => out_at succ_tbl in_map n
  | _ :: tl => path_target_live succ_tbl in_map tl
  end.

Lemma out_subset_live_dead_assign : forall x uses out,
  ~ In x out ->
  subset out (transfer (Assign x uses) out).
Proof.
  intros x uses out Hnot y Hy.
  unfold transfer. simpl.
  rewrite remove_var_notin by exact Hnot.
  apply subset_union_right with (a := uses) (b := out).
  exact Hy.
Qed.

Lemma lmap_get_in_collect : forall n succs in_map,
  In n succs ->
  subset (lmap_get n in_map) (collect_succ_in succs in_map).
Proof.
  intros n succs.
  induction succs as [|s tl IH]; intros in_map Hin x Hx.
  - inversion Hin.
  - simpl.
    rewrite union_spec.
    destruct Hin as [Hn | Hn].
    + subst s. left. exact Hx.
    + right. apply IH; assumption.
Qed.

Lemma dse_instr_cfg_sound_rel : forall i succs in_map s1 s2 live_in_n,
  live_in_n = transfer i (out_of_node succs in_map) ->
  eq_on live_in_n s1 s2 ->
  eq_on (out_of_node succs in_map)
        (exec_instr s1 i)
        (exec_instr s2 (dse_instr_cfg i succs in_map)).
Proof.
  intros i succs in_map s1 s2 live_in_n Hlive Heq_in.
  unfold dse_instr_cfg.
  destruct i as [x uses | uses].
  - simpl in Hlive.
    simpl.
    destruct (negb (mem x (out_of_node succs in_map))) eqn:Hdead.
    + (* dead assignment eliminated *)
      apply negb_true_iff in Hdead.
      apply mem_false_notin in Hdead.
      assert (Hs1s2_out : eq_on (out_of_node succs in_map) s1 s2).
      { apply eq_on_subset with (b := live_in_n).
        - rewrite Hlive.
          apply out_subset_live_dead_assign.
          exact Hdead.
        - exact Heq_in. }
      apply eq_on_trans with (s2 := s1).
      * apply dead_assign_exec_preserves_out. exact Hdead.
      * exact Hs1s2_out.
    + (* assignment kept *)
      change (eq_on (out_of_node succs in_map)
                    (exec_instr s1 (Assign x uses))
                    (exec_instr s2 (Assign x uses))).
      apply transfer_sound_instr.
      rewrite <- Hlive.
      exact Heq_in.
  - (* effect instruction *)
    simpl in Hlive.
    simpl.
    change (eq_on (out_of_node succs in_map)
                  (exec_instr s1 (Effect uses))
                  (exec_instr s2 (Effect uses))).
    apply transfer_sound_instr.
    rewrite <- Hlive.
    exact Heq_in.
Qed.

Theorem exec_path_sound_from : forall code succ_tbl in_map path s1 s2,
  PathOK succ_tbl path ->
  (forall n, In n path ->
    live_at in_map n = transfer (instr_at code n) (out_at succ_tbl in_map n)) ->
  eq_on (match path with | [] => [] | n :: _ => live_at in_map n end) s1 s2 ->
  eq_on (path_target_live succ_tbl in_map path)
        (exec_path_orig code path s1)
        (exec_path_opt code succ_tbl in_map path s2).
Proof.
  intros code succ_tbl in_map path.
  induction path as [|n tl IH]; intros s1 s2 Hpath Hnode Heq.
  - simpl. apply eq_on_empty.
  - destruct tl as [|m tl2].
    + (* singleton path *)
      inversion Hpath; subst.
      simpl in *.
      apply dse_instr_cfg_sound_rel with (live_in_n := live_at in_map n).
      * apply Hnode. left. reflexivity.
      * exact Heq.
    + (* at least two nodes *)
      inversion Hpath as [| |n' m' tl' Hedge Htail]; subst.
      simpl in *.
      assert (Hnode_n :
                live_at in_map n =
                transfer (instr_at code n) (out_at succ_tbl in_map n)).
      { apply Hnode. left. reflexivity. }
      assert (Hstep_out :
                eq_on (out_at succ_tbl in_map n)
                      (exec_instr s1 (instr_at code n))
                      (exec_instr s2 (dse_instr_cfg (instr_at code n) (succs_at succ_tbl n) in_map))).
      { apply dse_instr_cfg_sound_rel with (live_in_n := live_at in_map n).
        - exact Hnode_n.
        - exact Heq. }
      assert (Hout_to_live_m :
                subset (live_at in_map m) (out_at succ_tbl in_map n)).
      { unfold live_at, out_at, out_of_node.
        apply lmap_get_in_collect.
        exact Hedge. }
      assert (Hnext :
                eq_on (live_at in_map m)
                      (exec_instr s1 (instr_at code n))
                      (exec_instr s2 (dse_instr_cfg (instr_at code n) (succs_at succ_tbl n) in_map))).
      { apply eq_on_subset with (b := out_at succ_tbl in_map n).
        - exact Hout_to_live_m.
        - exact Hstep_out. }
      apply IH.
      * exact Htail.
      * intros k Hink. apply Hnode. right. exact Hink.
      * exact Hnext.
Qed.

Theorem exec_path_sound : forall code succ_tbl in_map path st,
  PathOK succ_tbl path ->
  (forall n, In n path ->
    live_at in_map n = transfer (instr_at code n) (out_at succ_tbl in_map n)) ->
  eq_on (path_target_live succ_tbl in_map path)
        (exec_path_orig code path st)
        (exec_path_opt code succ_tbl in_map path st).
Proof.
  intros code succ_tbl in_map path st Hpath Hnode.
  apply exec_path_sound_from with (s1 := st) (s2 := st).
  - exact Hpath.
  - exact Hnode.
  - destruct path as [|n tl].
    + apply eq_on_empty.
    + apply eq_on_refl.
Qed.
