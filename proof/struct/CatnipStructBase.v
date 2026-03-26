(* FILE: proof/struct/CatnipStructBase.v *)
(* CatnipStructBase.v - Core definitions, field access, method resolution.
 *
 * Source of truth:
 *   catnip_rs/src/vm/structs.rs   (StructType, StructInstance, SuperProxy)
 *   catnip_rs/src/vm/core.rs      (resolve_member dispatch)
 *   catnip_rs/src/core/method.rs  (BoundCatnipMethod, CatnipMethod)
 *
 * Proves:
 *   - O(1) field access by position (no hash lookup at runtime)
 *   - Method resolution determinism (fields > methods > static)
 *)

From Coq Require Import List ZArith Bool Lia PeanoNat.
From Coq Require Import Arith.Compare_dec.
From Coq Require Import String.
Import ListNotations.

(* Resolve length ambiguity: List.length over String.length *)
Definition length {A : Type} := @List.length A.

(* Override ++ to always mean list append (string_scope also defines ++) *)
Notation "x '++' y" := (@app _ x y) (at level 60, right associativity) : list_scope.
Open Scope string_scope.
Open Scope list_scope.


(* ================================================================ *)
(* A. Field Definitions and Access                                   *)
(*                                                                    *)
(* StructField: name + optional default.                             *)
(* Fields stored positionally in Vec<Value> (StructInstance.fields). *)
(* Type stores field metadata in Vec<StructField> (StructType.fields)*)
(* Access: find field name in type metadata -> index -> instance vec *)
(* ================================================================ *)

Record FieldDef := mkField {
  field_name : string;
  field_has_default : bool;
}.

Definition FieldList := list FieldDef.
Definition FieldValues := list Z.  (* Z abstracts NaN-boxed Value *)

(* Find field position by name. O(n) at type registration,
   but at runtime the compiler emits direct index. *)
Fixpoint find_field_idx (name : string) (fields : FieldList) (idx : nat) : option nat :=
  match fields with
  | [] => None
  | f :: rest =>
      if String.eqb (field_name f) name then Some idx
      else find_field_idx name rest (S idx)
  end.

Definition field_lookup (name : string) (fields : FieldList) : option nat :=
  find_field_idx name fields 0.

(* Read field value by index *)
Definition read_field (values : FieldValues) (idx : nat) : option Z :=
  nth_error values idx.


(* ================================================================ *)
(* A'. Field Name Decidability                                       *)
(* ================================================================ *)

Lemma string_eqb_refl : forall s, String.eqb s s = true.
Proof.
  induction s; simpl.
  - reflexivity.
  - rewrite Ascii.eqb_refl. exact IHs.
Qed.

Lemma string_eqb_eq : forall s1 s2,
  String.eqb s1 s2 = true -> s1 = s2.
Proof. intros. apply String.eqb_eq. exact H. Qed.

Lemma string_eqb_neq : forall s1 s2,
  String.eqb s1 s2 = false -> s1 <> s2.
Proof.
  intros s1 s2 H Heq. subst.
  rewrite string_eqb_refl in H. discriminate.
Qed.


(* ================================================================ *)
(* B. Field Access Correctness                                       *)
(*                                                                    *)
(* If a field exists at position k, reading values[k] returns the    *)
(* correct value.                                                     *)
(* ================================================================ *)

(* No duplicate field names *)
Definition fields_unique (fields : FieldList) : Prop :=
  forall i j fi fj,
    nth_error fields i = Some fi ->
    nth_error fields j = Some fj ->
    field_name fi = field_name fj ->
    i = j.

(* Field values match field definitions in length *)
Definition fields_match (fields : FieldList) (values : FieldValues) : Prop :=
  length fields = length values.

(* Alternative field lookup returning a relative index *)
Fixpoint find_field_rel (name : string) (fields : FieldList) : option nat :=
  match fields with
  | [] => None
  | f :: rest =>
      if String.eqb (field_name f) name then Some 0
      else match find_field_rel name rest with
           | Some n => Some (S n)
           | None => None
           end
  end.

(* find_field_rel agrees with field_lookup *)
Lemma find_field_rel_equiv_aux : forall name fields base,
  find_field_idx name fields base =
  match find_field_rel name fields with
  | Some n => Some (base + n)
  | None => None
  end.
Proof.
  intros name fields.
  induction fields as [|f rest IH]; intro base; simpl.
  - reflexivity.
  - destruct (String.eqb (field_name f) name) eqn:E.
    + f_equal. lia.
    + rewrite IH.
      destruct (find_field_rel name rest); [f_equal; lia | reflexivity].
Qed.

Lemma find_field_rel_equiv : forall name fields,
  field_lookup name fields = find_field_rel name fields.
Proof.
  intros. unfold field_lookup.
  rewrite find_field_rel_equiv_aux.
  destruct (find_field_rel name fields); [f_equal; lia | reflexivity].
Qed.

Theorem field_access_correct : forall name fields idx,
  field_lookup name fields = Some idx ->
  exists fd, nth_error fields idx = Some fd /\
             field_name fd = name.
Proof.
  intros name fields idx Hlookup.
  rewrite find_field_rel_equiv in Hlookup.
  revert idx Hlookup.
  induction fields as [|f rest IH]; intros idx H.
  - simpl in H. discriminate.
  - simpl in H.
    destruct (String.eqb (field_name f) name) eqn:Heq.
    + inversion H; subst. exists f. split.
      * reflexivity.
      * apply string_eqb_eq. exact Heq.
    + destruct (find_field_rel name rest) eqn:Efr; [|discriminate].
      inversion H; subst.
      specialize (IH n eq_refl).
      destruct IH as [fd [Hnth Hname]].
      exists fd. split.
      * simpl. exact Hnth.
      * exact Hname.
Qed.

(* find_field_idx returns a valid index *)
Lemma find_field_idx_bound : forall name fields base idx,
  find_field_idx name fields base = Some idx ->
  (base <= idx < base + length fields)%nat.
Proof.
  intros name fields.
  induction fields as [|f rest IH]; intros base idx H.
  - simpl in H. discriminate.
  - simpl in H.
    destruct (String.eqb (field_name f) name) eqn:Heq.
    + inversion H; subst. simpl. lia.
    + apply IH in H. simpl. lia.
Qed.

Theorem field_lookup_bound : forall name fields idx,
  field_lookup name fields = Some idx ->
  (idx < length fields)%nat.
Proof.
  intros name fields idx H.
  unfold field_lookup in H.
  apply find_field_idx_bound in H. lia.
Qed.

(* Field lookup is deterministic *)
Theorem field_lookup_deterministic : forall name fields idx1 idx2,
  field_lookup name fields = Some idx1 ->
  field_lookup name fields = Some idx2 ->
  idx1 = idx2.
Proof.
  intros name fields idx1 idx2 H1 H2.
  rewrite H1 in H2. inversion H2. reflexivity.
Qed.

(* Two different field names map to different indices *)
Lemma find_field_idx_injective : forall fields base n1 n2 idx1 idx2,
  fields_unique fields ->
  find_field_idx n1 fields base = Some idx1 ->
  find_field_idx n2 fields base = Some idx2 ->
  n1 <> n2 ->
  idx1 <> idx2.
Proof.
  intros fields.
  induction fields as [|f rest IH]; intros base n1 n2 idx1 idx2 Huniq H1 H2 Hne.
  - simpl in H1. discriminate.
  - simpl in H1, H2.
    destruct (String.eqb (field_name f) n1) eqn:E1;
    destruct (String.eqb (field_name f) n2) eqn:E2.
    + (* Both match f -> n1 = n2, contradiction *)
      apply string_eqb_eq in E1. apply string_eqb_eq in E2.
      exfalso. apply Hne. congruence.
    + (* n1 matches f at base, n2 somewhere in rest *)
      inversion H1; subst.
      apply find_field_idx_bound in H2. lia.
    + (* n2 matches f at base, n1 somewhere in rest *)
      inversion H2; subst.
      apply find_field_idx_bound in H1. lia.
    + (* Both in rest *)
      assert (Huniq' : fields_unique rest).
      { unfold fields_unique in *. intros i j fi fj Hi Hj Hfn.
        assert (S i = S j) as Heq.
        { apply (Huniq (S i) (S j) fi fj); simpl; assumption. }
        lia. }
      exact (IH (S base) n1 n2 idx1 idx2 Huniq' H1 H2 Hne).
Qed.

Theorem field_lookup_injective : forall fields n1 n2 idx1 idx2,
  fields_unique fields ->
  field_lookup n1 fields = Some idx1 ->
  field_lookup n2 fields = Some idx2 ->
  n1 <> n2 ->
  idx1 <> idx2.
Proof.
  intros. unfold field_lookup in *.
  exact (find_field_idx_injective fields 0 n1 n2 idx1 idx2 H H0 H1 H2).
Qed.


(* ================================================================ *)
(* C. Method Resolution                                              *)
(*                                                                    *)
(* Three-level priority: fields > instance methods > static methods. *)
(* Models the CallMethod dispatch in core.rs:2757-2932.              *)
(* ================================================================ *)

Inductive MethodKind :=
  | MkInstance   (* Receives self as first arg *)
  | MkStatic.    (* No self binding *)

Record MethodEntry := mkMethod {
  method_name : string;
  method_kind : MethodKind;
  method_id : nat;         (* opaque callable identifier *)
}.

Definition MethodMap := list MethodEntry.

Definition find_method (name : string) (methods : MethodMap) : option MethodEntry :=
  find (fun m => String.eqb (method_name m) name) methods.

(* Resolution result *)
Inductive ResolveResult :=
  | ResField (idx : nat) (value : Z)     (* Callable field *)
  | ResMethod (m : MethodEntry)           (* Instance method *)
  | ResStatic (m : MethodEntry)           (* Static method *)
  | ResNotFound.

(* Full resolution: try field, then method, then static *)
Definition resolve_member
  (name : string) (fields : FieldList) (values : FieldValues)
  (methods : MethodMap) (statics : MethodMap) : ResolveResult :=
  match field_lookup name fields with
  | Some idx =>
      match read_field values idx with
      | Some v => ResField idx v
      | None => ResNotFound  (* should not happen if fields_match *)
      end
  | None =>
      match find_method name methods with
      | Some m => ResMethod m
      | None =>
          match find_method name statics with
          | Some m => ResStatic m
          | None => ResNotFound
          end
      end
  end.

(* Fields shadow methods *)
Theorem field_shadows_method : forall name fields values methods statics idx v,
  field_lookup name fields = Some idx ->
  read_field values idx = Some v ->
  resolve_member name fields values methods statics = ResField idx v.
Proof.
  intros. unfold resolve_member. rewrite H. rewrite H0. reflexivity.
Qed.

(* Methods shadow static methods *)
Theorem method_shadows_static : forall name fields values methods statics m,
  field_lookup name fields = None ->
  find_method name methods = Some m ->
  resolve_member name fields values methods statics = ResMethod m.
Proof.
  intros. unfold resolve_member. rewrite H. rewrite H0. reflexivity.
Qed.

(* Resolution is deterministic *)
Theorem resolve_deterministic : forall name fields values methods statics,
  forall r1 r2,
  r1 = resolve_member name fields values methods statics ->
  r2 = resolve_member name fields values methods statics ->
  r1 = r2.
Proof. intros. subst. reflexivity. Qed.


(* ================================================================ *)
(* D. Struct Type                                                     *)
(*                                                                    *)
(* Models StructType from structs.rs:37-55.                          *)
(* Since multiple inheritance: parent_names replaces parent,          *)
(* mro stores the C3-linearized method resolution order.             *)
(* Super resolution is MRO-based (see CatnipMROSuper.v).            *)
(* ================================================================ *)

Record StructType := mkStructType {
  st_id : nat;
  st_name : string;
  st_fields : FieldList;
  st_methods : MethodMap;
  st_statics : MethodMap;
  st_parent_names : list string;    (* direct parent type names *)
  st_mro : list string;            (* C3 linearized MRO *)
  st_implements : list string;       (* trait names *)
  st_abstract : list string;         (* unimplemented abstract method names *)
}.

Record StructInstance := mkInstance {
  si_type_id : nat;
  si_fields : FieldValues;
}.
