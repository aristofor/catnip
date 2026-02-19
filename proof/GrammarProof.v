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

Lemma step_S : step [N S] [N A; N B].
Proof.
  change (step ([] ++ [N S] ++ []) ([] ++ [N A; N B] ++ [])).
  apply step_prod.
  apply P_S_AB.
Qed.

Lemma step_A : step [N A] [Tm ta].
Proof.
  change (step ([] ++ [N A] ++ []) ([] ++ [Tm ta] ++ [])).
  apply step_prod.
  apply P_A_a.
Qed.

Lemma step_B : step [N B] [Tm tb].
Proof.
  change (step ([] ++ [N B] ++ []) ([] ++ [Tm tb] ++ [])).
  apply step_prod.
  apply P_B_b.
Qed.

Lemma step_A_then_Bctx : step [N A; N B] [Tm ta; N B].
Proof.
  change (step ([] ++ [N A] ++ [N B]) ([] ++ [Tm ta] ++ [N B])).
  apply step_prod.
  apply P_A_a.
Qed.

Lemma step_B_after_A : step [Tm ta; N B] [Tm ta; Tm tb].
Proof.
  change (step ([Tm ta] ++ [N B] ++ []) ([Tm ta] ++ [Tm tb] ++ [])).
  apply step_prod.
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
