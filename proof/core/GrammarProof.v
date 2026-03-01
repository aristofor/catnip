(* FILE: proof/core/GrammarProof.v *)
From Coq Require Import List Relation_Operators Program.Equality.
Import ListNotations.

(* Example grammar:
   S -> A B
   A -> "a"
   B -> "b" *)

Inductive NT := S | A | B.
Inductive T := ta | tb.

Inductive symbol :=
| N (x : NT)
| Tm (a : T).

Inductive prod : NT -> list symbol -> Prop :=
| P_S_AB : prod S [N A; N B]
| P_A_a  : prod A [Tm ta]
| P_B_b  : prod B [Tm tb].

(* One-step rewriting in context. *)
Inductive step : list symbol -> list symbol -> Prop :=
| step_prod : forall l r x rhs,
    prod x rhs ->
    step (l ++ [N x] ++ r) (l ++ rhs ++ r).

Lemma step_from_prod : forall l r x rhs,
  prod x rhs ->
  step (l ++ [N x] ++ r) (l ++ rhs ++ r).
Proof.
  intros l r x rhs Hprod.
  now apply step_prod.
Qed.

Lemma step_S : step [N S] [N A; N B].
Proof.
  apply (step_from_prod [] [] S [N A; N B]).
  apply P_S_AB.
Qed.

Lemma step_A : step [N A] [Tm ta].
Proof.
  apply (step_from_prod [] [] A [Tm ta]).
  apply P_A_a.
Qed.

Lemma step_B : step [N B] [Tm tb].
Proof.
  apply (step_from_prod [] [] B [Tm tb]).
  apply P_B_b.
Qed.

Lemma step_A_then_Bctx : step [N A; N B] [Tm ta; N B].
Proof.
  apply (step_from_prod [] [N B] A [Tm ta]).
  apply P_A_a.
Qed.

Lemma step_B_after_A : step [Tm ta; N B] [Tm ta; Tm tb].
Proof.
  apply (step_from_prod [Tm ta] [] B [Tm tb]).
  apply P_B_b.
Qed.

(* Reflexive-transitive closure of one-step rewriting. *)
Definition derives := clos_refl_trans (list symbol) step.

Definition start : list symbol := [N S].

Definition terminal_form (w : list symbol) : Prop :=
  Forall (fun s => match s with Tm _ => True | N _ => False end) w.

Definition generates_symbols (w : list symbol) : Prop :=
  derives start w /\ terminal_form w.

Definition generates (w : list T) : Prop :=
  derives start (map Tm w).

(* Parse trees, indexed by the non-terminal at the root. *)
Inductive tree : NT -> Type :=
| TrS : tree A -> tree B -> tree S
| TrA : tree A
| TrB : tree B.

Fixpoint yield_tree {x : NT} (t : tree x) : list T :=
  match t with
  | TrS ta_tree tb_tree => yield_tree ta_tree ++ yield_tree tb_tree
  | TrA => [ta]
  | TrB => [tb]
  end.

Definition leaves {x : NT} (t : tree x) : list symbol :=
  map Tm (yield_tree t).

Lemma yield_A_singleton : forall t : tree A, yield_tree t = [ta].
Proof.
  intros t.
  dependent destruction t.
  reflexivity.
Qed.

Lemma yield_B_singleton : forall t : tree B, yield_tree t = [tb].
Proof.
  intros t.
  dependent destruction t.
  reflexivity.
Qed.

Lemma tree_sound_A : forall t : tree A, derives [N A] (leaves t).
Proof.
  intros t.
  unfold leaves.
  rewrite yield_A_singleton.
  simpl.
  apply rt_step.
  apply step_A.
Qed.

Lemma tree_sound_B : forall t : tree B, derives [N B] (leaves t).
Proof.
  intros t.
  unfold leaves.
  rewrite yield_B_singleton.
  simpl.
  apply rt_step.
  apply step_B.
Qed.

Theorem tree_sound : forall x (t : tree x), derives [N x] (leaves t).
Proof.
  intros x t.
  destruct t as [ta_tree tb_tree | |].
  - unfold leaves.
    simpl.
    rewrite (yield_A_singleton ta_tree).
    rewrite (yield_B_singleton tb_tree).
    simpl.
    eapply rt_trans.
    + apply rt_step.
      apply step_S.
    + eapply rt_trans.
      * apply rt_step.
        apply step_A_then_Bctx.
      * apply rt_step.
        apply step_B_after_A.
  - apply tree_sound_A.
  - apply tree_sound_B.
Qed.

Theorem generates_example_ab : generates [ta; tb].
Proof.
  unfold generates, start.
  eapply rt_trans.
  - apply rt_step.
    apply step_S.
  - eapply rt_trans.
    + apply rt_step.
      apply step_A_then_Bctx.
    + apply rt_step.
      apply step_B_after_A.
Qed.

(* Non-ambiguity at the tree level for this grammar:
   there is exactly one tree shape for S. *)
Theorem tree_A_unique : forall t1 t2 : tree A, t1 = t2.
Proof.
  intros t1 t2.
  dependent destruction t1.
  dependent destruction t2.
  reflexivity.
Qed.

Theorem tree_B_unique : forall t1 t2 : tree B, t1 = t2.
Proof.
  intros t1 t2.
  dependent destruction t1.
  dependent destruction t2.
  reflexivity.
Qed.

Theorem grammar_unambiguous_S : forall t1 t2 : tree S, t1 = t2.
Proof.
  intros t1 t2.
  dependent destruction t1.
  dependent destruction t2.
  f_equal.
  - apply tree_A_unique.
  - apply tree_B_unique.
Qed.


(* ================================================================ *)
(* NON-AMBIGUITY VIA YIELD                                            *)
(*                                                                    *)
(* Standard formulation: if two parse trees for the same              *)
(* non-terminal produce the same terminal string, the trees are       *)
(* identical.  This is a stronger result than tree-level uniqueness   *)
(* because it is conditioned on the observable output.                *)
(* ================================================================ *)

Definition unambiguous (x : NT) : Prop :=
  forall (w : list T) (t1 t2 : tree x),
    yield_tree t1 = w -> yield_tree t2 = w -> t1 = t2.

(* Yield-injectivity implies unambiguity. *)

Theorem yield_injective : forall (x : NT) (t1 t2 : tree x),
  yield_tree t1 = yield_tree t2 -> t1 = t2.
Proof.
  intros x t1 t2 _.
  destruct x.
  - apply grammar_unambiguous_S.
  - apply tree_A_unique.
  - apply tree_B_unique.
Qed.

Theorem grammar_unambiguous : forall x, unambiguous x.
Proof.
  intros x w t1 t2 H1 H2.
  apply yield_injective.
  congruence.
Qed.

(* The unique word generated by S. *)

Theorem yield_S_unique : forall (t : tree S), yield_tree t = [ta; tb].
Proof.
  intros t.
  dependent destruction t.
  simpl.
  rewrite yield_A_singleton, yield_B_singleton.
  reflexivity.
Qed.

(* Completeness: any tree for S generates [ta; tb] and derives it. *)

Theorem tree_complete_S : forall (t : tree S),
  generates (yield_tree t).
Proof.
  intros t.
  rewrite yield_S_unique.
  exact generates_example_ab.
Qed.
