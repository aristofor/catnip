(* FILE: proof/struct/CatnipMROFields.v *)
(* MRO-Based Field Operations
 *
 * Proves:
 *   - Field merge (first-seen dedup)
 *   - Diamond field deduplication
 *   - Field redefinition detection
 *
 * Source: MakeStruct in catnip_rs/src/vm/core.rs
 *)

From Coq Require Import List ZArith Bool Lia PeanoNat.
From Coq Require Import String.
Import ListNotations.

Definition length {A : Type} := @List.length A.

Notation "x '++' y" := (@app _ x y) (at level 60, right associativity) : list_scope.
Open Scope string_scope.
Open Scope list_scope.


(* ================================================================ *)
(* D. MRO-Based Field Merge                                          *)
(*                                                                    *)
(* Fields collected by traversing the MRO. First-seen wins: a field  *)
(* name from an earlier MRO position is kept, later duplicates       *)
(* are skipped. Source: MakeStruct in core.rs.                       *)
(* ================================================================ *)

Record FieldDef := mkField {
  field_name : string;
  field_has_default : bool;
}.

Definition FieldList := list FieldDef.

(* Merge fields from MRO: first-seen wins (deduplication by name) *)
Fixpoint merge_fields_dedup (fields : FieldList) (seen : list string) : FieldList :=
  match fields with
  | [] => []
  | f :: rest =>
      if existsb (String.eqb (field_name f)) seen then
        merge_fields_dedup rest seen
      else
        f :: merge_fields_dedup rest (field_name f :: seen)
  end.

(* Collect all fields from types in MRO order *)
Definition collect_mro_fields (type_fields : list FieldList) : FieldList :=
  merge_fields_dedup (List.concat type_fields) [].

(* Dedup never adds new field names *)
Lemma merge_fields_dedup_subset : forall fields seen f,
  In f (merge_fields_dedup fields seen) -> In f fields.
Proof.
  induction fields as [|x rest IH]; intros seen f Hin.
  - inversion Hin.
  - simpl in Hin.
    destruct (existsb (String.eqb (field_name x)) seen).
    + right. apply IH with (seen := seen). exact Hin.
    + destruct Hin as [Heq | Hin'].
      * left. exact Heq.
      * right. apply IH with (seen := field_name x :: seen). exact Hin'.
Qed.


(* ================================================================ *)
(* E. Diamond Field Deduplication                                    *)
(*                                                                    *)
(* In diamond D(B,C) where B(A) and C(A), field "x" from A appears *)
(* only once. MRO = [D, B, C, A].                                   *)
(* ================================================================ *)

(* Count occurrences of a field name *)
Fixpoint count_field_name (name : string) (fields : FieldList) : nat :=
  match fields with
  | [] => 0
  | f :: rest =>
      (if String.eqb (field_name f) name then 1 else 0) + count_field_name name rest
  end.

(* Helper: string equality reflexivity *)
Lemma string_eqb_refl : forall s, String.eqb s s = true.
Proof. induction s; simpl. reflexivity. rewrite Ascii.eqb_refl. exact IHs. Qed.

(* After dedup, each name appears at most once.
   Proof by generalized induction on (fields, seen). *)
Theorem dedup_at_most_once : forall fields name,
  count_field_name name (merge_fields_dedup fields []) <= 1.
Proof.
  intros fields name.
  enough (H : forall seen,
    (existsb (String.eqb name) seen = true ->
      count_field_name name (merge_fields_dedup fields seen) = 0) /\
    (existsb (String.eqb name) seen = false ->
      count_field_name name (merge_fields_dedup fields seen) <= 1)).
  { specialize (H []). destruct H as [_ Hr]. apply Hr. reflexivity. }
  induction fields as [|f rest IH]; intro seen.
  - split; intros _; simpl; lia.
  - split; intro Hseen; simpl.
    + (* name already in seen *)
      destruct (existsb (String.eqb (field_name f)) seen) eqn:Efseen.
      * apply (proj1 (IH seen)). exact Hseen.
      * simpl.
        destruct (String.eqb (field_name f) name) eqn:Efn.
        -- apply String.eqb_eq in Efn. rewrite Efn in Efseen.
           rewrite Efseen in Hseen. discriminate.
        -- apply (proj1 (IH (field_name f :: seen))).
           simpl. rewrite String.eqb_sym. rewrite Efn. simpl. exact Hseen.
    + (* name not in seen *)
      destruct (existsb (String.eqb (field_name f)) seen) eqn:Efseen.
      * apply (proj2 (IH seen)). exact Hseen.
      * simpl.
        destruct (String.eqb (field_name f) name) eqn:Efn.
        -- simpl.
           apply String.eqb_eq in Efn.
           assert (Hin : existsb (String.eqb name) (field_name f :: seen) = true).
           { simpl. rewrite String.eqb_sym. rewrite Efn.
             rewrite string_eqb_refl. reflexivity.
           }
           specialize (proj1 (IH (field_name f :: seen)) Hin). lia.
        -- simpl.
           apply (proj2 (IH (field_name f :: seen))).
           simpl. rewrite String.eqb_sym. rewrite Efn. simpl. exact Hseen.
Qed.


(* ================================================================ *)
(* F. Field Redefinition Detection                                   *)
(*                                                                    *)
(* A child struct cannot declare a field with the same name as an    *)
(* inherited field. Source: MakeStruct in core.rs.                   *)
(* ================================================================ *)

Definition has_field_name (name : string) (fields : FieldList) : bool :=
  existsb (fun f => String.eqb (field_name f) name) fields.

(* If no child field name clashes with inherited fields, check returns None *)
Theorem no_redefinition_correct : forall child_fields inherited_fields,
  find (fun f => has_field_name (field_name f) inherited_fields) child_fields = None ->
  forall f, In f child_fields ->
    has_field_name (field_name f) inherited_fields = false.
Proof.
  intros child_fields inherited_fields Hfind f Hin.
  induction child_fields as [|c rest IH].
  - inversion Hin.
  - simpl in Hfind.
    destruct (has_field_name (field_name c) inherited_fields) eqn:Ec.
    + discriminate.
    + destruct Hin as [Heq | Hin'].
      * subst. exact Ec.
      * apply IH; assumption.
Qed.

(* If check finds a clash, it returns a field that exists in both *)
Theorem redefinition_detected : forall child_fields inherited_fields f,
  find (fun f => has_field_name (field_name f) inherited_fields) child_fields = Some f ->
  has_field_name (field_name f) inherited_fields = true /\ In f child_fields.
Proof.
  intros child_fields inherited_fields f Hfind.
  apply find_some in Hfind. destruct Hfind as [Hin Hhas].
  split; assumption.
Qed.
