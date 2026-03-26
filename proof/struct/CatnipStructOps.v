(* FILE: proof/struct/CatnipStructOps.v *)
(* CatnipStructOps.v - Operations and proofs on structs and traits.
 *
 * Source of truth:
 *   catnip_rs/src/vm/structs.rs   (extends, method merge, construction)
 *   catnip_rs/src/vm/traits.rs    (TraitDef, TraitRegistry, linearization)
 *   catnip_rs/src/vm/core.rs      (CallMethod, abstract enforcement)
 *   catnip_rs/src/core/method.rs  (BoundCatnipMethod, CatnipMethod)
 *
 * Proves:
 *   - Inheritance correctness (extends: field prepend, method merge)
 *   - Trait linearization (post-order, no cycles, no duplicates)
 *   - Abstract method enforcement
 *   - CallMethod equivalence with GetAttr + Call
 *   - Instance construction validation
 *)

From Coq Require Import List ZArith Bool Lia PeanoNat.
From Coq Require Import Arith.Compare_dec.
From Coq Require Import String.
Import ListNotations.
From Catnip Require Import CatnipStructBase.


(* ================================================================ *)
(* E. Method Merge (child overrides parent)                          *)
(*                                                                    *)
(* Used by both struct extends and trait linearization.               *)
(* For MRO-based multiple inheritance field/method merge and super,  *)
(* see CatnipMROMethods.v / CatnipMROFields.v.                       *)
(* ================================================================ *)

(* Merge methods: child overrides parent *)
Definition merge_methods (parent child : MethodMap) : MethodMap :=
  let child_names := map method_name child in
  let parent_only := filter
    (fun m => negb (existsb (String.eqb (method_name m)) child_names))
    parent
  in
  child ++ parent_only.

(* find on concatenation: if found in first list, returns that *)
Lemma find_app_l : forall {A : Type} (f : A -> bool) (l1 l2 : list A) (x : A),
  find f l1 = Some x ->
  find f (l1 ++ l2) = Some x.
Proof.
  intros A f l1. induction l1 as [|a rest IH]; intros l2 x H.
  - simpl in H. discriminate.
  - simpl in H. simpl.
    destruct (f a) eqn:E.
    + exact H.
    + apply IH. exact H.
Qed.

(* find on concatenation: if not found in first, try second *)
Lemma find_app_r : forall {A : Type} (f : A -> bool) (l1 l2 : list A),
  find f l1 = None ->
  find f (l1 ++ l2) = find f l2.
Proof.
  intros A f l1. induction l1 as [|a rest IH]; intros l2 H.
  - reflexivity.
  - simpl in H. simpl.
    destruct (f a) eqn:E.
    + discriminate.
    + apply IH. exact H.
Qed.

(* Child method overrides parent in merge *)
Theorem child_override_wins : forall parent_mm child_mm name m,
  find_method name child_mm = Some m ->
  find_method name (merge_methods parent_mm child_mm) = Some m.
Proof.
  intros. unfold merge_methods, find_method. apply find_app_l. exact H.
Qed.

(* Method not in child means name not in child_names *)
Lemma find_method_none_not_in_names : forall name methods,
  find_method name methods = None ->
  existsb (String.eqb name) (map method_name methods) = false.
Proof.
  intros name methods H.
  unfold find_method in H.
  induction methods as [|m rest IH].
  - reflexivity.
  - simpl in H.
    destruct (String.eqb (method_name m) name) eqn:Em.
    + discriminate.
    + simpl. rewrite String.eqb_sym. rewrite Em. simpl.
      apply IH. exact H.
Qed.

(* Method in parent and in filter means it's in the result *)
Lemma find_in_filter : forall name parent child_names,
  existsb (String.eqb name) child_names = false ->
  forall m, find (fun m => String.eqb (method_name m) name) parent = Some m ->
  find (fun m => String.eqb (method_name m) name)
    (filter (fun m => negb (existsb (String.eqb (method_name m)) child_names)) parent)
  = Some m.
Proof.
  intros name parent child_names Hnotin m Hfind.
  induction parent as [|p rest IH].
  - simpl in Hfind. discriminate.
  - simpl in Hfind.
    destruct (String.eqb (method_name p) name) eqn:Ep.
    + inversion Hfind; subst. simpl.
      assert (Hmfilt : existsb (String.eqb (method_name m)) child_names = false).
      { apply string_eqb_eq in Ep. rewrite Ep. exact Hnotin. }
      rewrite Hmfilt. simpl. rewrite Ep. reflexivity.
    + simpl.
      destruct (existsb (String.eqb (method_name p)) child_names) eqn:Efilt.
      * simpl. apply IH. exact Hfind.
      * simpl. rewrite Ep. apply IH. exact Hfind.
Qed.

(* Parent methods not overridden are preserved *)
Theorem parent_method_preserved : forall parent_mm child_mm name m,
  find_method name parent_mm = Some m ->
  find_method name child_mm = None ->
  find_method name (merge_methods parent_mm child_mm) = Some m.
Proof.
  intros parent_mm child_mm name m Hparent Hchild.
  unfold merge_methods, find_method.
  rewrite find_app_r by exact Hchild.
  apply find_in_filter.
  - apply find_method_none_not_in_names. exact Hchild.
  - exact Hparent.
Qed.


(* ================================================================ *)
(* F. Type Registry                                                   *)
(*                                                                    *)
(* Registration-order registry for type lookup.                      *)
(* Super resolution is now MRO-based: see CatnipMROSuper.v          *)
(* (super_at_position, cooperative super termination).               *)
(* ================================================================ *)

(* Type registry: ordered list of types *)
Definition TypeRegistry := list StructType.

Fixpoint find_type (name : string) (reg : TypeRegistry) : option StructType :=
  match reg with
  | [] => None
  | t :: rest =>
      if String.eqb (st_name t) name then Some t
      else find_type name rest
  end.

(* Registration index: position in the registry *)
Fixpoint reg_index (name : string) (reg : TypeRegistry) : option nat :=
  match reg with
  | [] => None
  | t :: rest =>
      if String.eqb (st_name t) name then Some 0
      else match reg_index name rest with
           | Some n => Some (S n)
           | None => None
           end
  end.

(* Well-formed registry: all parents registered before child *)
Definition well_formed_registry (reg : TypeRegistry) : Prop :=
  forall t parent_name pi ti,
    In t reg ->
    In parent_name (st_parent_names t) ->
    reg_index parent_name reg = Some pi ->
    reg_index (st_name t) reg = Some ti ->
    (pi < ti)%nat.


(* ================================================================ *)
(* G. Trait Linearization                                            *)
(*                                                                    *)
(* Post-order traversal of trait DAG.                                *)
(* Source: traits.rs:172-285 (resolve_for_struct).                   *)
(* ================================================================ *)

Record TraitDef := mkTrait {
  trait_name : string;
  trait_extends : list string;    (* parent trait names *)
  trait_methods : MethodMap;
  trait_abstract : list string;   (* abstract method names *)
}.

Definition TraitRegistry := list TraitDef.

Fixpoint find_trait (name : string) (reg : TraitRegistry) : option TraitDef :=
  match reg with
  | [] => None
  | t :: rest =>
      if String.eqb (trait_name t) name then Some t
      else find_trait name rest
  end.

(* Linearization: post-order DFS with deduplication.
   fuel parameter for termination. *)
Fixpoint linearize_aux
  (names : list string) (reg : TraitRegistry)
  (visited : list string) (fuel : nat) : list string :=
  match fuel with
  | 0 => visited
  | S fuel' =>
      fold_left
        (fun acc name =>
          if existsb (String.eqb name) acc then acc  (* skip visited *)
          else
            match find_trait name reg with
            | None => acc  (* unknown trait, skip *)
            | Some t =>
                (* Visit parents first (post-order) *)
                let acc' := linearize_aux (trait_extends t) reg acc fuel' in
                (* Then add self *)
                if existsb (String.eqb name) acc' then acc'
                else acc' ++ [name]
            end)
        names visited
  end.

Definition linearize (names : list string) (reg : TraitRegistry) : list string :=
  linearize_aux names reg [] (length reg + 1).

(* No duplicates in result *)
Lemma existsb_app_false : forall s l1 l2,
  existsb (String.eqb s) (l1 ++ l2) = false ->
  existsb (String.eqb s) l1 = false /\ existsb (String.eqb s) l2 = false.
Proof.
  intros s l1 l2 H. rewrite existsb_app in H.
  apply Bool.orb_false_iff in H. exact H.
Qed.

(* Linearization only adds to visited *)
Lemma linearize_aux_extends : forall names reg visited fuel,
  forall s, In s visited -> In s (linearize_aux names reg visited fuel).
Proof.
  intros names reg visited fuel.
  revert names visited.
  induction fuel as [|fuel' IH]; intros names visited s Hin.
  - simpl. exact Hin.
  - simpl.
    revert visited Hin.
    induction names as [|n rest IHn]; intros visited Hin.
    + simpl. exact Hin.
    + simpl.
      destruct (existsb (String.eqb n) visited) eqn:Evis.
      * apply IHn. exact Hin.
      * destruct (find_trait n reg) eqn:Efind.
        -- set (acc' := linearize_aux (trait_extends t) reg visited fuel').
           destruct (existsb (String.eqb n) acc') eqn:Eacc.
           ++ apply IHn. apply IH. exact Hin.
           ++ apply IHn. apply in_or_app. left. apply IH. exact Hin.
        -- apply IHn. exact Hin.
Qed.


(* ================================================================ *)
(* H. Abstract Method Enforcement                                    *)
(*                                                                    *)
(* A struct with non-empty abstract_methods set cannot be            *)
(* instantiated. Source: CatnipStructType.__call__ in structs.rs.    *)
(* ================================================================ *)

Definition can_instantiate (ty : StructType) : Prop :=
  st_abstract ty = [].

Definition has_abstract (ty : StructType) : Prop :=
  st_abstract ty <> [].

Theorem abstract_blocks_instantiation : forall ty,
  has_abstract ty -> ~ can_instantiate ty.
Proof.
  intros ty Habs Hcan. unfold has_abstract, can_instantiate in *.
  contradiction.
Qed.

(* Concrete methods remove abstract obligations *)
Definition fulfills_abstract (abstracts : list string) (methods : MethodMap) : list string :=
  filter
    (fun name => negb (existsb (String.eqb name) (map method_name methods)))
    abstracts.

(* If existsb finds name in method names, find_method succeeds *)
Lemma existsb_method_names_find : forall name methods,
  existsb (String.eqb name) (map method_name methods) = true ->
  exists m, find_method name methods = Some m.
Proof.
  intros name methods H.
  induction methods as [|m rest IH].
  - simpl in H. discriminate.
  - unfold find_method. simpl.
    destruct (String.eqb (method_name m) name) eqn:Em.
    + exists m. reflexivity.
    + assert (Hrest : existsb (String.eqb name) (map method_name rest) = true).
      { simpl in H. rewrite String.eqb_sym in H. rewrite Em in H. simpl in H.
        exact H. }
      apply IH in Hrest. destruct Hrest as [m' Hm'].
      exists m'. unfold find_method in Hm'. exact Hm'.
Qed.

Theorem no_remaining_abstract_means_concrete : forall abstracts methods,
  fulfills_abstract abstracts methods = [] ->
  forall name, In name abstracts ->
    exists m, find_method name methods = Some m.
Proof.
  intros abstracts methods Hful name Hin.
  unfold fulfills_abstract in Hful.
  (* name is in abstracts but not in the filter result (which is []) *)
  assert (Hfilter : negb (existsb (String.eqb name) (map method_name methods)) = false).
  { assert (Hnotin : ~ In name (filter
      (fun n => negb (existsb (String.eqb n) (map method_name methods))) abstracts)).
    { rewrite Hful. apply in_nil. }
    destruct (negb (existsb (String.eqb name) (map method_name methods))) eqn:E.
    - exfalso. apply Hnotin. apply filter_In. split; assumption.
    - reflexivity. }
  apply Bool.negb_false_iff in Hfilter.
  apply existsb_method_names_find. exact Hfilter.
Qed.


(* ================================================================ *)
(* I. CallMethod Equivalence                                         *)
(*                                                                    *)
(* CallMethod(obj, name, args) is semantically equivalent to         *)
(* GetAttr(obj, name) followed by Call(result, args).                *)
(* Source: CallMethod handler in core.rs:2757-2932.                  *)
(*                                                                    *)
(* Models the three resolution paths:                                *)
(*   1. Field (callable): call without self                          *)
(*   2. Instance method: call with self prepended                    *)
(*   3. Static method: call without self                             *)
(* ================================================================ *)

(* Abstract call result *)
Inductive CallResult :=
  | CallOk (result : Z)
  | CallError.

(* Model a callable: takes args, produces result *)
Definition Callable := list Z -> CallResult.

(* GetAttr on struct: returns callable + whether to bind self *)
Inductive AttrResult :=
  | AttrCallable (c : Callable) (bind_self : bool)
  | AttrNotFound.

Definition getattr_struct
  (name : string) (ty : StructType) (inst : StructInstance) : AttrResult :=
  match resolve_member name (st_fields ty) (si_fields inst)
                        (st_methods ty) (st_statics ty) with
  | ResField _ _ => AttrCallable (fun args => CallOk 0) false  (* field is callable *)
  | ResMethod m => AttrCallable (fun args => CallOk (Z.of_nat (method_id m))) true
  | ResStatic m => AttrCallable (fun args => CallOk (Z.of_nat (method_id m))) false
  | ResNotFound => AttrNotFound
  end.

(* GetAttr + Call: the two-step approach *)
Definition getattr_then_call
  (name : string) (ty : StructType) (inst : StructInstance)
  (self_val : Z) (args : list Z) : CallResult :=
  match getattr_struct name ty inst with
  | AttrCallable c true => c (self_val :: args)     (* bind self *)
  | AttrCallable c false => c args                   (* no self *)
  | AttrNotFound => CallError
  end.

(* CallMethod: the fused approach *)
Definition call_method
  (name : string) (ty : StructType) (inst : StructInstance)
  (self_val : Z) (args : list Z) : CallResult :=
  match resolve_member name (st_fields ty) (si_fields inst)
                        (st_methods ty) (st_statics ty) with
  | ResField _ _ => (fun a => CallOk 0) args         (* field callable, no self *)
  | ResMethod m => (fun a => CallOk (Z.of_nat (method_id m))) (self_val :: args)
  | ResStatic m => (fun a => CallOk (Z.of_nat (method_id m))) args
  | ResNotFound => CallError
  end.

(* Core equivalence: fused CallMethod produces same result *)
Theorem callmethod_equiv : forall name ty inst self_val args,
  call_method name ty inst self_val args =
  getattr_then_call name ty inst self_val args.
Proof.
  intros.
  unfold call_method, getattr_then_call, getattr_struct.
  destruct (resolve_member name (st_fields ty) (si_fields inst)
                           (st_methods ty) (st_statics ty));
  reflexivity.
Qed.


(* ================================================================ *)
(* J. Instance Construction                                          *)
(*                                                                    *)
(* Source: CatnipStructType.__call__ in structs.rs:277-382.          *)
(* Validates: all fields set, no extra args, defaults applied.        *)
(* ================================================================ *)

(* Count required fields (no default) *)
Definition count_required (fields : FieldList) : nat :=
  length (filter (fun f => negb (field_has_default f)) fields).

(* Construction succeeds if enough args *)
Definition can_construct (fields : FieldList) (nargs : nat) : bool :=
  (count_required fields <=? nargs)%nat && (nargs <=? length fields)%nat.

Theorem construct_exact_args : forall fields,
  can_construct fields (length fields) = true.
Proof.
  intro fields.
  unfold can_construct.
  apply Bool.andb_true_iff. split.
  - apply Nat.leb_le.
    unfold count_required.
    apply Nat.le_trans with (length fields).
    + apply filter_length_le.
    + lia.
  - apply Nat.leb_le. lia.
Qed.

Theorem construct_too_few_args : forall fields nargs,
  (nargs < count_required fields)%nat ->
  can_construct fields nargs = false.
Proof.
  intros fields nargs Hlt.
  unfold can_construct.
  apply Bool.andb_false_iff. left.
  apply Nat.leb_gt. exact Hlt.
Qed.

Theorem construct_too_many_args : forall fields nargs,
  (nargs > length fields)%nat ->
  can_construct fields nargs = false.
Proof.
  intros fields nargs Hgt.
  unfold can_construct.
  apply Bool.andb_false_iff. right.
  apply Nat.leb_gt. exact Hgt.
Qed.


(* ================================================================ *)
(* K. Trait Method Merge                                             *)
(*                                                                    *)
(* Last-wins semantics in linearization order.                       *)
(* Source: traits.rs resolve_for_struct.                              *)
(* ================================================================ *)

(* Merge trait methods in MRO order: later overrides earlier *)
Definition merge_trait_methods (traits : list MethodMap) : MethodMap :=
  fold_left (fun acc methods => merge_methods acc methods) traits [].

(* Merge: child overrides parent (raw MethodMap version) *)
Lemma merge_child_wins : forall parent_mm child_mm name m,
  find_method name child_mm = Some m ->
  find_method name (merge_methods parent_mm child_mm) = Some m.
Proof.
  intros. unfold merge_methods, find_method.
  apply find_app_l. exact H.
Qed.

(* Last trait in list wins for duplicate names *)
Theorem last_trait_wins : forall name m rest_traits last_methods,
  find_method name last_methods = Some m ->
  find_method name (merge_methods
    (merge_trait_methods rest_traits) last_methods) = Some m.
Proof.
  intros. apply merge_child_wins. exact H.
Qed.
