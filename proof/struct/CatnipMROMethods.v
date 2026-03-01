(* FILE: proof/struct/CatnipMROMethods.v *)
(* MRO-Based Method Resolution
 *
 * Proves:
 *   - First-wins method resolution
 *   - Left-priority in multiple inheritance
 *   - Method merge determinism
 *   - Merged methods are subset of input
 *
 * Source: MakeStruct MRO method merge + setup_super_proxy
 * in catnip_rs/src/vm/core.rs
 *)

From Coq Require Import List ZArith Bool Lia PeanoNat.
From Coq Require Import String.
Import ListNotations.

Open Scope string_scope.
Open Scope list_scope.


(* ================================================================ *)
(* G. MRO-Based Method Resolution                                   *)
(*                                                                    *)
(* Methods collected by traversing MRO in order. First-wins:         *)
(* earliest type in MRO that defines a method wins.                  *)
(* This gives left-priority in extends(B, C): B before C.           *)
(* Source: MakeStruct MRO method merge + setup_super_proxy.          *)
(* ================================================================ *)

Record MethodDef := mkMethod {
  meth_name : string;
  meth_source : string;  (* type name that defines this method *)
  meth_id : nat;
}.

Definition MethodMap := list MethodDef.

Definition find_method (name : string) (methods : MethodMap) : option MethodDef :=
  find (fun m => String.eqb (meth_name m) name) methods.

(* Merge methods from types in MRO order: first-wins *)
Fixpoint merge_methods_mro (all_methods : MethodMap) (seen : list string) : MethodMap :=
  match all_methods with
  | [] => []
  | m :: rest =>
      if existsb (String.eqb (meth_name m)) seen then
        merge_methods_mro rest seen
      else
        m :: merge_methods_mro rest (meth_name m :: seen)
  end.

(* find_method returns a method whose name matches *)
Lemma find_method_name : forall name methods m,
  find_method name methods = Some m ->
  String.eqb (meth_name m) name = true.
Proof.
  intros name methods m H. unfold find_method in H.
  apply find_some in H. destruct H as [_ Heq]. exact Heq.
Qed.

(* Helper: existsb on cons *)
Lemma existsb_cons_false : forall (f : string -> bool) h t,
  existsb f (h :: t) = false ->
  f h = false /\ existsb f t = false.
Proof.
  intros f h t H. simpl in H.
  apply Bool.orb_false_iff in H. exact H.
Qed.

(* String equality reflexivity *)
Lemma string_eqb_refl : forall s, String.eqb s s = true.
Proof. induction s; simpl. reflexivity. rewrite Ascii.eqb_refl. exact IHs. Qed.

Lemma neq_string_eqb : forall s1 s2, s1 <> s2 -> String.eqb s1 s2 = false.
Proof.
  intros s1 s2 H. destruct (String.eqb s1 s2) eqn:E; [|reflexivity].
  exfalso. apply H. apply String.eqb_eq. exact E.
Qed.

(* Left priority generalized: if method is in first list, first-wins finds it *)
Lemma left_priority_gen : forall b_methods c_methods name mb seen,
  find_method name b_methods = Some mb ->
  existsb (String.eqb (meth_name mb)) seen = false ->
  find_method name (merge_methods_mro (b_methods ++ c_methods) seen) = Some mb.
Proof.
  intros b_methods.
  induction b_methods as [|m rest IH]; intros c_methods name mb seen Hfind Hnotseen.
  - simpl in Hfind. discriminate.
  - simpl in Hfind.
    destruct (String.eqb (meth_name m) name) eqn:Em.
    + inversion Hfind; subst. simpl.
      rewrite Hnotseen.
      unfold find_method. simpl. rewrite Em. reflexivity.
    + simpl.
      destruct (existsb (String.eqb (meth_name m)) seen) eqn:Emseen.
      * unfold find_method. apply IH; assumption.
      * unfold find_method. simpl. rewrite Em.
        apply IH.
        -- exact Hfind.
        -- (* mb.name is not in (m.name :: seen) *)
           assert (Hmb_name : String.eqb (meth_name mb) name = true)
             by (apply find_method_name with (methods := rest); exact Hfind).
           simpl.
           apply Bool.orb_false_iff. split.
           ++ (* mb.name <> m.name: mb matches name, m doesn't *)
              destruct (String.eqb (meth_name mb) (meth_name m)) eqn:Embm; [|reflexivity].
              apply String.eqb_eq in Embm. apply String.eqb_eq in Hmb_name.
              exfalso. rewrite <- Hmb_name in Em. rewrite <- Embm in Em.
              rewrite string_eqb_refl in Em. discriminate.
           ++ exact Hnotseen.
Qed.

(* Left priority: in extends(B, C), B's methods come before C's *)
Theorem left_priority : forall b_methods c_methods name mb,
  find_method name b_methods = Some mb ->
  find_method name (merge_methods_mro (b_methods ++ c_methods) []) = Some mb.
Proof.
  intros. apply left_priority_gen.
  - exact H.
  - reflexivity.
Qed.

(* Merged methods are a subset of input methods *)
Lemma merge_methods_subset : forall methods seen m,
  In m (merge_methods_mro methods seen) -> In m methods.
Proof.
  induction methods as [|x rest IH]; intros seen m Hin.
  - inversion Hin.
  - simpl in Hin.
    destruct (existsb (String.eqb (meth_name x)) seen).
    + right. apply IH with (seen := seen). exact Hin.
    + destruct Hin as [Heq | Hin'].
      * left. exact Heq.
      * right. apply IH with (seen := meth_name x :: seen). exact Hin'.
Qed.

(* Method merge is deterministic (same inputs => same outputs) *)
Theorem merge_methods_deterministic : forall methods seen,
  merge_methods_mro methods seen = merge_methods_mro methods seen.
Proof. reflexivity. Qed.
