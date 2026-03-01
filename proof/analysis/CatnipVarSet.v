(* FILE: proof/analysis/CatnipVarSet.v *)
(* Finite set operations on nat-indexed variables (list-based).
 *
 * Reusable library for all liveness and dataflow proofs.
 *
 * Provides:
 *   - VarSet = list nat with add, union, remove_var, remove_list
 *   - Specification lemmas (add_spec, union_spec, in_remove_var_iff)
 *   - Monotonicity (add, union, remove_var, remove_list)
 *   - NoDup preservation (add, union, remove_var, remove_list)
 *   - Subset inclusion into union (subset_union_left, subset_union_right)
 *   - Membership decision (mem_true_in, mem_false_notin)
 *)

From Coq Require Import List Bool Lia Arith.PeanoNat ZArith.
Import ListNotations.


(* ================================================================ *)
(* A. Variables and finite sets (list-based)                        *)
(* ================================================================ *)

Definition Var := nat.
Definition VarSet := list Var.

Fixpoint mem (x : Var) (s : VarSet) : bool :=
  match s with
  | [] => false
  | y :: ys => if Nat.eqb x y then true else mem x ys
  end.

Definition add (x : Var) (s : VarSet) : VarSet :=
  if mem x s then s else x :: s.

Fixpoint union (a b : VarSet) : VarSet :=
  match a with
  | [] => b
  | x :: xs => union xs (add x b)
  end.

Fixpoint remove_var (x : Var) (s : VarSet) : VarSet :=
  match s with
  | [] => []
  | y :: ys => if Nat.eqb x y then remove_var x ys else y :: remove_var x ys
  end.

Fixpoint remove_list (xs : VarSet) (s : VarSet) : VarSet :=
  match xs with
  | [] => s
  | x :: tl => remove_list tl (remove_var x s)
  end.

Definition subset (a b : VarSet) : Prop :=
  forall x, In x a -> In x b.


(* ================================================================ *)
(* B. Basic set lemmas                                               *)
(* ================================================================ *)

Lemma add_spec : forall x s y,
  In y (add x s) <-> y = x \/ In y s.
Proof.
  intros x s y.
  unfold add.
  destruct (mem x s) eqn:Hm.
  - split.
    + intro Hy. right. exact Hy.
    + intros [Hy | Hy].
      * subst.
        induction s as [|u us IH].
        { simpl in Hm. discriminate. }
        simpl in Hm.
        destruct (Nat.eqb x u) eqn:Hu.
        { apply Nat.eqb_eq in Hu. left. symmetry. exact Hu. }
        { right. apply IH. exact Hm. }
      * exact Hy.
  - split.
    + simpl. intros [Hy | Hy].
      * left. symmetry. exact Hy.
      * right. exact Hy.
    + simpl. intros [Hy | Hy].
      * left. symmetry. exact Hy.
      * right. exact Hy.
Qed.

Lemma add_monotone : forall x a b,
  subset a b -> subset (add x a) (add x b).
Proof.
  intros x a b Hsub y Hy.
  apply add_spec in Hy.
  apply add_spec.
  destruct Hy as [-> | Hin].
  - left. reflexivity.
  - right. apply Hsub. exact Hin.
Qed.

Lemma union_monotone_right : forall a b1 b2,
  subset b1 b2 -> subset (union a b1) (union a b2).
Proof.
  induction a as [|x xs IH]; intros b1 b2 Hsub y Hy.
  - simpl in *. apply Hsub. exact Hy.
  - simpl in *. eapply IH.
    + apply add_monotone. exact Hsub.
    + exact Hy.
Qed.

Lemma union_spec : forall a b x,
  In x (union a b) <-> In x a \/ In x b.
Proof.
  induction a as [|u us IH]; intros b x.
  - simpl. tauto.
  - simpl.
    rewrite IH.
    rewrite add_spec.
    split.
    + intros [Hin_us | [Hxu | Hin_b]].
      * left. right. exact Hin_us.
      * left. left. symmetry. exact Hxu.
      * right. exact Hin_b.
    + intros [[Hux | Hin_us] | Hin_b].
      * right. left. symmetry. exact Hux.
      * left. exact Hin_us.
      * right. right. exact Hin_b.
Qed.

Lemma in_remove_var_iff : forall x s y,
  In y (remove_var x s) <-> In y s /\ y <> x.
Proof.
  intros x s.
  induction s as [|u us IH]; intro y.
  - simpl. split.
    + intro H. contradiction.
    + intros [Hin _]. contradiction.
  - simpl.
    destruct (Nat.eqb x u) eqn:Hu.
    + split.
      * intro H.
        apply IH in H as [Hin Hneq].
        split.
        { right. exact Hin. }
        { exact Hneq. }
      * intros [[Hy | Hin] Hneq].
        { subst.
          apply Nat.eqb_eq in Hu.
          exfalso. apply Hneq. symmetry. exact Hu. }
        { apply IH. split; assumption. }
    + split.
      * intro H.
        destruct H as [Hy | Hrest].
        { split.
          { left. exact Hy. }
          { intros Hxy. subst.
            apply Nat.eqb_neq in Hu. apply Hu. reflexivity. } }
        { apply IH in Hrest as [Hin Hneq].
          split.
          { right. exact Hin. }
          { exact Hneq. } }
      * intros [[Hy | Hin] Hneq].
        { left. exact Hy. }
        { right. apply IH. split; assumption. }
Qed.

Lemma remove_var_monotone : forall x a b,
  subset a b -> subset (remove_var x a) (remove_var x b).
Proof.
  intros x a b Hsub y Hy.
  apply in_remove_var_iff in Hy as [Hina Hneq].
  apply in_remove_var_iff.
  split.
  - apply Hsub. exact Hina.
  - exact Hneq.
Qed.

Lemma remove_list_monotone : forall xs a b,
  subset a b -> subset (remove_list xs a) (remove_list xs b).
Proof.
  induction xs as [|x tl IH]; intros a b Hsub.
  - simpl. exact Hsub.
  - simpl. apply IH. apply remove_var_monotone. exact Hsub.
Qed.


(* ================================================================ *)
(* B.2 NoDup invariants                                             *)
(*                                                                    *)
(* VarSet operations preserve the NoDup (no duplicates) invariant.   *)
(* Foundation for size-based reasoning and convergence analysis.      *)
(* ================================================================ *)

Lemma add_NoDup : forall x s, NoDup s -> NoDup (add x s).
Proof.
  intros x s Hnd. unfold add.
  destruct (mem x s) eqn:Hm.
  - exact Hnd.
  - constructor.
    + (* mem x s = false -> ~ In x s *)
      intro Hin. induction s as [|y ys IH].
      * inversion Hin.
      * simpl in Hm.
        destruct (Nat.eqb x y) eqn:Hxy.
        { discriminate. }
        { destruct Hin as [Hy | Hy].
          - subst. rewrite Nat.eqb_refl in Hxy. discriminate.
          - apply IH. inversion Hnd; assumption. exact Hm. exact Hy. }
    + exact Hnd.
Qed.

Lemma union_NoDup : forall a b, NoDup b -> NoDup (union a b).
Proof.
  induction a as [|x xs IH]; intros b Hnd.
  - simpl. exact Hnd.
  - simpl. apply IH. apply add_NoDup. exact Hnd.
Qed.

Lemma remove_var_NoDup : forall x s, NoDup s -> NoDup (remove_var x s).
Proof.
  intros x s. induction s as [|y ys IH]; intro Hnd.
  - simpl. constructor.
  - inversion Hnd as [|? ? Hnin Hnd']; subst.
    simpl. destruct (Nat.eqb x y) eqn:Hxy.
    + apply IH. exact Hnd'.
    + constructor.
      * intro Hin. apply Hnin.
        apply in_remove_var_iff in Hin as [Hin _]. exact Hin.
      * apply IH. exact Hnd'.
Qed.

Lemma remove_list_NoDup : forall xs s, NoDup s -> NoDup (remove_list xs s).
Proof.
  induction xs as [|x tl IH]; intros s Hnd.
  - simpl. exact Hnd.
  - simpl. apply IH. apply remove_var_NoDup. exact Hnd.
Qed.

Lemma mem_true_in : forall x s,
  mem x s = true -> In x s.
Proof.
  intros x s.
  induction s as [|y ys IH]; intro Hm.
  - simpl in Hm. discriminate.
  - simpl in Hm.
    destruct (Nat.eqb x y) eqn:Heq.
    + apply Nat.eqb_eq in Heq. left. symmetry. exact Heq.
    + right. apply IH. exact Hm.
Qed.

Lemma mem_false_notin : forall x s,
  mem x s = false -> ~ In x s.
Proof.
  intros x s.
  induction s as [|y ys IH]; intro Hm.
  - simpl. intro Hin. contradiction.
  - simpl in Hm.
    destruct (Nat.eqb x y) eqn:Heq.
    + discriminate.
    + intro Hin.
      destruct Hin as [Hin | Hin].
      * subst. apply Nat.eqb_neq in Heq. contradiction.
      * apply IH in Hm. contradiction.
Qed.

Lemma subset_union_right : forall a b,
  subset b (union a b).
Proof.
  induction a as [|x xs IH]; intros b y Hy.
  - simpl. exact Hy.
  - simpl.
    apply IH.
    apply (proj2 (add_spec x b y)).
    right. exact Hy.
Qed.

Lemma subset_union_left : forall a b,
  subset a (union a b).
Proof.
  induction a as [|x xs IH]; intros b y Hy.
  - inversion Hy.
  - simpl in Hy.
    simpl.
    destruct Hy as [Hy | Hy].
    + subst y.
      apply subset_union_right with (a := xs) (b := add x b).
      apply (proj2 (add_spec x b x)).
      left. reflexivity.
    + apply IH with (b := add x b). exact Hy.
Qed.

Lemma in_remove_var_intro : forall x y s,
  In y s ->
  y <> x ->
  In y (remove_var x s).
Proof.
  intros x y s.
  induction s as [|u us IH]; intros Hin Hneq.
  - inversion Hin.
  - simpl in Hin.
    simpl.
    destruct (Nat.eqb x u) eqn:Hu.
    + destruct Hin as [Hy | Hy].
      * subst.
        apply Nat.eqb_eq in Hu.
        exfalso. apply Hneq. symmetry. exact Hu.
      * apply IH; assumption.
    + destruct Hin as [Hy | Hy].
      * subst. left. reflexivity.
      * right. apply IH; assumption.
Qed.

Lemma remove_var_notin : forall x s,
  ~ In x s ->
  remove_var x s = s.
Proof.
  intros x s Hnot.
  induction s as [|y ys IH].
  - reflexivity.
  - simpl.
    destruct (Nat.eqb x y) eqn:Heq.
    + apply Nat.eqb_eq in Heq. subst y.
      exfalso. apply Hnot. left. reflexivity.
    + f_equal.
      apply IH.
      intro Hin. apply Hnot. right. exact Hin.
Qed.
