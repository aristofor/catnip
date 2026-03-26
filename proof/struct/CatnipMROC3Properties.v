(* FILE: proof/struct/CatnipMROC3Properties.v *)
(* C3 Properties: Local Precedence and Monotonicity
 *
 * Proves that C3 linearization preserves:
 *   - Local precedence order (parent declaration order)
 *   - Monotonicity (within-parent order)
 *
 * These are the defining properties of C3 (vs other linearizations).
 *
 * Reference: "The Python 2.3 Method Resolution Order", Michele
 * Simionato, 2003.
 *)

From Coq Require Import List ZArith Bool Lia PeanoNat.
From Coq Require Import String.
Import ListNotations.

From Catnip Require Import CatnipMROC3Core.

Definition length {A : Type} := @List.length A.

Notation "x '++' y" := (@app _ x y) (at level 60, right associativity) : list_scope.
Open Scope string_scope.
Open Scope list_scope.

(* Position of an element in a list *)
Fixpoint list_position (x : string) (l : list string) : option nat :=
  match l with
  | [] => None
  | h :: rest =>
      if String.eqb h x then Some 0
      else match list_position x rest with
           | Some n => Some (S n)
           | None => None
           end
  end.

(* "x appears before y in l" *)
Definition before_in (x y : string) (l : list string) : Prop :=
  exists px py, list_position x l = Some px /\
                list_position y l = Some py /\
                (px < py)%nat.

(* --- Helpers for order preservation proofs --- *)

Lemma neq_string_eqb : forall s1 s2, s1 <> s2 -> String.eqb s1 s2 = false.
Proof.
  intros s1 s2 H. destruct (String.eqb s1 s2) eqn:E; [|reflexivity].
  exfalso. apply H. apply String.eqb_eq. exact E.
Qed.

Lemma before_in_neq : forall x y l, before_in x y l -> x <> y.
Proof.
  intros x y l [px [py [Hx [Hy Hlt]]]] Heq.
  subst. rewrite Hx in Hy. inversion Hy. lia.
Qed.

Lemma list_position_In : forall x l n, list_position x l = Some n -> In x l.
Proof.
  induction l as [|h rest IH]; simpl; intros n Hp; [discriminate|].
  destruct (String.eqb h x) eqn:E.
  - left. apply String.eqb_eq in E. auto.
  - right. destruct (list_position x rest); [|discriminate].
    eapply IH. reflexivity.
Qed.

Lemma In_list_position : forall x l, In x l -> exists n, list_position x l = Some n.
Proof.
  induction l as [|h rest IH]; intros Hin; [inversion Hin|].
  simpl. destruct (String.eqb h x) eqn:E.
  - exists 0. reflexivity.
  - destruct Hin as [Heq|Hin'].
    + subst. rewrite string_eqb_refl in E. discriminate.
    + apply IH in Hin'. destruct Hin' as [n Hn]. rewrite Hn. eauto.
Qed.

Lemma In_existsb_eqb : forall x l, In x l -> existsb (String.eqb x) l = true.
Proof.
  induction l as [|h rest IH]; intros Hin; [inversion Hin|].
  simpl. destruct Hin as [Heq|Hin'].
  - subst. rewrite string_eqb_refl. reflexivity.
  - apply Bool.orb_true_iff. right. apply IH. exact Hin'.
Qed.

Lemma before_in_head : forall x y l, x <> y -> In y l -> before_in x y (x :: l).
Proof.
  intros x y l Hneq Hin.
  apply In_list_position in Hin. destruct Hin as [py Hpy].
  exists 0, (S py). repeat split.
  - simpl. rewrite string_eqb_refl. reflexivity.
  - simpl. rewrite (neq_string_eqb x y Hneq). rewrite Hpy. reflexivity.
  - lia.
Qed.

Lemma before_in_cons_neq : forall x y h l,
  before_in x y l -> x <> h -> y <> h -> before_in x y (h :: l).
Proof.
  intros x y h l [px [py [Hx [Hy Hlt]]]] Hxh Hyh.
  exists (S px), (S py). repeat split.
  - simpl. rewrite (neq_string_eqb h x ltac:(auto)). rewrite Hx. reflexivity.
  - simpl. rewrite (neq_string_eqb h y ltac:(auto)). rewrite Hy. reflexivity.
  - lia.
Qed.

Lemma before_in_remove_cons : forall x y h l,
  before_in x y (h :: l) -> x <> h -> y <> h -> before_in x y l.
Proof.
  intros x y h l [px [py [Hx [Hy Hlt]]]] Hxh Hyh.
  simpl in Hx. rewrite (neq_string_eqb h x ltac:(auto)) in Hx.
  simpl in Hy. rewrite (neq_string_eqb h y ltac:(auto)) in Hy.
  destruct (list_position x l) eqn:Epx; [|discriminate].
  destruct (list_position y l) eqn:Epy; [|discriminate].
  inversion Hx; subst. inversion Hy; subst.
  exists n, n0. repeat split; [exact Epx|exact Epy|lia].
Qed.

Lemma before_in_in_tail : forall x y seq,
  before_in x y seq -> in_tail y seq = true.
Proof.
  intros x y seq [px [py [Hx [Hy Hlt]]]].
  destruct seq as [|s0 s_rest]; [simpl in Hx; discriminate|].
  simpl. simpl in Hy.
  destruct (String.eqb s0 y) eqn:E.
  - inversion Hy; subst. lia.
  - destruct (list_position y s_rest) eqn:Er; [|discriminate].
    apply In_existsb_eqb. eapply list_position_In. exact Er.
Qed.

Lemma in_any_tail_from_in_tail : forall y seq seqs,
  in_tail y seq = true -> In seq seqs -> in_any_tail y seqs = true.
Proof.
  intros y seq seqs Ht Hin. unfold in_any_tail.
  induction seqs as [|s rest IH]; [inversion Hin|].
  simpl. destruct Hin as [Heq|Hin'].
  - subst. rewrite Ht. reflexivity.
  - apply Bool.orb_true_iff. right. apply IH. exact Hin'.
Qed.

Lemma find_good_head_not_in_tail : forall seqs all_seqs h,
  find_good_head seqs all_seqs = Some h -> in_any_tail h all_seqs = false.
Proof.
  induction seqs as [|s rest IH]; simpl; intros all_seqs h Hf; [discriminate|].
  destruct s as [|c s_rest].
  - apply IH. exact Hf.
  - destruct (in_any_tail c all_seqs) eqn:E.
    + apply IH. exact Hf.
    + inversion Hf; subst. exact E.
Qed.

Lemma In_remove_empty : forall (seq : list string) seqs,
  In seq seqs -> seq <> [] -> In seq (remove_empty seqs).
Proof.
  intros seq seqs Hin Hne. unfold remove_empty.
  apply filter_In. split; [exact Hin|].
  destruct seq; [contradiction|reflexivity].
Qed.

Lemma In_remove_head_from : forall head (seq : list string) seqs,
  In seq seqs ->
  In (match seq with h :: rest => if String.eqb h head then rest else seq | [] => [] end)
     (remove_head_from head seqs).
Proof.
  intros head seq seqs Hin. induction seqs as [|s rest IH]; [inversion Hin|].
  destruct Hin as [Heq|Hin'].
  - subst. simpl. left. reflexivity.
  - simpl. right. apply IH. exact Hin'.
Qed.

(* One-step unfolding of c3_merge *)
Lemma c3_merge_unfold : forall seqs fuel',
  c3_merge seqs (S fuel') =
    let seqs' := remove_empty seqs in
    match seqs' with
    | [] => Some []
    | _ :: _ =>
        match find_good_head seqs' seqs' with
        | None => None
        | Some h =>
            match c3_merge (remove_head_from h seqs') fuel' with
            | None => None
            | Some rest => Some (h :: rest)
            end
        end
    end.
Proof. reflexivity. Qed.

(* Core: c3_merge preserves within-sequence order *)
Lemma c3_merge_preserves_order : forall fuel seqs result,
  c3_merge seqs fuel = Some result ->
  forall seq x y,
  In seq seqs ->
  before_in x y seq ->
  In x result -> In y result ->
  before_in x y result.
Proof.
  induction fuel as [|fuel' IH]; intros seqs result Hmerge seq x y Hseq Hbefore Hinx Hiny.
  - simpl in Hmerge. inversion Hmerge; subst. inversion Hinx.
  - rewrite c3_merge_unfold in Hmerge. cbv zeta in Hmerge.
    revert Hmerge.
    destruct (remove_empty seqs) as [|s0 seqs_tl] eqn:Eseqs'; intro Hmerge.
    + inversion Hmerge; subst. inversion Hinx.
    + destruct (find_good_head (s0 :: seqs_tl) (s0 :: seqs_tl)) as [head|] eqn:Efgh;
        [|discriminate].
      destruct (c3_merge (remove_head_from head (s0 :: seqs_tl)) fuel')
        as [rest|] eqn:Erec; [|discriminate].
      inversion Hmerge; subst. clear Hmerge.
      assert (Hseq_ne : seq <> []).
      { intro Habs. subst. destruct Hbefore as [? [? [H _]]].
        simpl in H. discriminate. }
      assert (Hseq' : In seq (s0 :: seqs_tl)).
      { rewrite <- Eseqs'. apply In_remove_empty; assumption. }
      assert (Hneq : x <> y) by (eapply before_in_neq; exact Hbefore).
      destruct (String.eqb x head) eqn:Exh.
      * (* x = head: x at position 0, y after *)
        apply String.eqb_eq in Exh. subst.
        apply before_in_head; [exact Hneq|].
        destruct Hiny as [Heq|Hin']; [subst; contradiction|exact Hin'].
      * destruct (String.eqb y head) eqn:Eyh.
        -- (* y = head: contradiction - y in tail of seq but head is good *)
           apply String.eqb_eq in Eyh. subst. exfalso.
           assert (Htail : in_tail head seq = true)
             by (eapply before_in_in_tail; exact Hbefore).
           assert (Hiat : in_any_tail head (s0 :: seqs_tl) = true)
             by (eapply in_any_tail_from_in_tail; eassumption).
           pose proof (find_good_head_not_in_tail _ _ _ Efgh) as Hniat.
           congruence.
        -- (* x <> head, y <> head: use IH on modified sequences *)
           assert (Hxh : x <> head)
             by (intro H; subst; rewrite string_eqb_refl in Exh; discriminate).
           assert (Hyh : y <> head)
             by (intro H; subst; rewrite string_eqb_refl in Eyh; discriminate).
           apply before_in_cons_neq; [|exact Hxh|exact Hyh].
           assert (Hxr : In x rest)
             by (destruct Hinx as [Heq|]; [subst; contradiction|assumption]).
           assert (Hyr : In y rest)
             by (destruct Hiny as [Heq|]; [subst; contradiction|assumption]).
           set (seq' := match seq with
                        | h :: r => if String.eqb h head then r else seq
                        | [] => []
                        end).
           apply (IH _ _ Erec seq' x y).
           ++ apply In_remove_head_from. exact Hseq'.
           ++ unfold seq'. destruct seq as [|sh sr]; [contradiction|].
              destruct (String.eqb sh head) eqn:Esh.
              ** apply String.eqb_eq in Esh. subst.
                 apply before_in_remove_cons with head; assumption.
              ** exact Hbefore.
           ++ exact Hxr.
           ++ exact Hyr.
Qed.

(* C3 preserves parent declaration order.
   Hypothesis p2 <> name: c3_linearize prepends name, which could
   disrupt ordering if p2 happened to equal the type's own name. *)
Theorem c3_preserves_local_precedence :
  forall name parents parent_mros mro p1 p2,
  c3_linearize name parents parent_mros = Some mro ->
  before_in p1 p2 parents ->
  In p1 mro -> In p2 mro ->
  p2 <> name ->
  before_in p1 p2 mro.
Proof.
  intros name parents parent_mros mro p1 p2 Hlin Hbefore Hp1 Hp2 Hp2n.
  assert (Hneq : p1 <> p2) by (eapply before_in_neq; exact Hbefore).
  destruct parents as [|p ps].
  - destruct Hbefore as [? [? [H _]]]. simpl in H. discriminate.
  - unfold c3_linearize in Hlin. cbv beta match zeta in Hlin.
    match type of Hlin with context [c3_merge ?s ?f] =>
      revert Hlin;
      destruct (c3_merge s f) as [merged|] eqn:Emerge;
      intro Hlin
    end; [|discriminate].
    inversion Hlin; subst. clear Hlin.
    destruct (String.eqb p1 name) eqn:Ep1.
    + apply String.eqb_eq in Ep1. subst.
      apply before_in_head; [exact Hneq|].
      destruct Hp2 as [Heq|Hin']; [subst; contradiction|exact Hin'].
    + assert (Hp1n : p1 <> name)
        by (intro H; subst; rewrite string_eqb_refl in Ep1; discriminate).
      assert (Hp1m : In p1 merged)
        by (destruct Hp1 as [Heq|]; [subst; contradiction|assumption]).
      assert (Hp2m : In p2 merged)
        by (destruct Hp2 as [Heq|]; [subst; contradiction|assumption]).
      apply before_in_cons_neq; [|exact Hp1n|exact Hp2n].
      apply (c3_merge_preserves_order _ _ _ Emerge (p :: ps) p1 p2).
      * apply in_or_app. right. simpl. left. reflexivity.
      * exact Hbefore.
      * exact Hp1m.
      * exact Hp2m.
Qed.

(* C3 monotonicity: child MRO preserves order from each parent's MRO.
   Hypothesis y <> name: same reason as local precedence. *)
Theorem c3_monotonicity :
  forall name parents parent_mros mro parent_mro x y,
  c3_linearize name parents parent_mros = Some mro ->
  In parent_mro parent_mros ->
  before_in x y parent_mro ->
  In x mro -> In y mro ->
  y <> name ->
  before_in x y mro.
Proof.
  intros name parents parent_mros mro parent_mro x y
    Hlin Hpmro Hbefore Hx Hy Hyn.
  assert (Hneq : x <> y) by (eapply before_in_neq; exact Hbefore).
  destruct parents as [|p ps].
  - unfold c3_linearize in Hlin. simpl in Hlin.
    inversion Hlin; subst.
    simpl in Hx. destruct Hx as [Heq|[]]. subst.
    simpl in Hy. destruct Hy as [Heq|[]]. subst.
    exfalso. exact (Hneq eq_refl).
  - unfold c3_linearize in Hlin. cbv beta match zeta in Hlin.
    match type of Hlin with context [c3_merge ?s ?f] =>
      revert Hlin;
      destruct (c3_merge s f) as [merged|] eqn:Emerge;
      intro Hlin
    end; [|discriminate].
    inversion Hlin; subst. clear Hlin.
    destruct (String.eqb x name) eqn:Exn.
    + apply String.eqb_eq in Exn. subst.
      apply before_in_head; [exact Hneq|].
      destruct Hy as [Heq|Hin']; [subst; contradiction|exact Hin'].
    + assert (Hxn : x <> name)
        by (intro H; subst; rewrite string_eqb_refl in Exn; discriminate).
      assert (Hxm : In x merged)
        by (destruct Hx as [Heq|]; [subst; contradiction|assumption]).
      assert (Hym : In y merged)
        by (destruct Hy as [Heq|]; [subst; contradiction|assumption]).
      apply before_in_cons_neq; [|exact Hxn|exact Hyn].
      apply (c3_merge_preserves_order _ _ _ Emerge parent_mro x y).
      * apply in_or_app. left. exact Hpmro.
      * exact Hbefore.
      * exact Hxm.
      * exact Hym.
Qed.
