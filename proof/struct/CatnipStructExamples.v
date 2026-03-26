(* FILE: proof/struct/CatnipStructExamples.v *)
(* CatnipStructExamples.v - Concrete examples for struct/trait system. *)

From Coq Require Import List ZArith Bool String.
Import ListNotations.
From Catnip Require Import CatnipStructBase.
From Catnip Require Import CatnipStructOps.

Open Scope string_scope.
Open Scope list_scope.


(* ================================================================ *)
(* L. Concrete Examples                                              *)
(* ================================================================ *)

(* Helper constructors *)
Definition mk_field (n : string) : FieldDef :=
  mkField n false.

Definition mk_field_default (n : string) : FieldDef :=
  mkField n true.

Definition mk_instance_method (n : string) (id : nat) : MethodEntry :=
  mkMethod n MkInstance id.

Definition mk_static_method (n : string) (id : nat) : MethodEntry :=
  mkMethod n MkStatic id.

(* Example: struct Point { x; y; } *)
Definition point_fields : FieldList :=
  [mk_field "x"; mk_field "y"].

Example ex_point_x_idx :
  field_lookup "x" point_fields = Some 0.
Proof. reflexivity. Qed.

Example ex_point_y_idx :
  field_lookup "y" point_fields = Some 1.
Proof. reflexivity. Qed.

Example ex_point_z_none :
  field_lookup "z" point_fields = None.
Proof. reflexivity. Qed.

(* Example: field access on Point(3, 4) *)
Definition point_values : FieldValues := [3%Z; 4%Z].

Example ex_read_x :
  read_field point_values 0 = Some 3%Z.
Proof. reflexivity. Qed.

Example ex_read_y :
  read_field point_values 1 = Some 4%Z.
Proof. reflexivity. Qed.

(* Example: method resolution priority *)
Definition counter_fields : FieldList := [mk_field "n"].
Definition counter_methods : MethodMap := [mk_instance_method "inc" 1].
Definition counter_statics : MethodMap := [mk_static_method "zero" 2].

Example ex_field_over_method :
  resolve_member "n" counter_fields [42%Z] counter_methods counter_statics
  = ResField 0 42.
Proof. reflexivity. Qed.

Example ex_method_found :
  resolve_member "inc" counter_fields [42%Z] counter_methods counter_statics
  = ResMethod (mk_instance_method "inc" 1).
Proof. reflexivity. Qed.

Example ex_static_found :
  resolve_member "zero" counter_fields [42%Z] counter_methods counter_statics
  = ResStatic (mk_static_method "zero" 2).
Proof. reflexivity. Qed.

(* Example: single inheritance - Child(Base) *)
Definition base_type : StructType :=
  mkStructType 0 "Base" [mk_field "a"]
    [mk_instance_method "show" 10] [] [] ["Base"] [] [].

Definition child_type : StructType :=
  mkStructType 1 "Child" [mk_field "a"; mk_field "b"]
    [mk_instance_method "show" 20] [] ["Base"] ["Child"; "Base"] [] [].

Example ex_child_fields :
  length (st_fields child_type) = 2.
Proof. reflexivity. Qed.

Example ex_child_parent_field_first :
  field_name (hd (mk_field "") (st_fields child_type)) = "a".
Proof. reflexivity. Qed.

Example ex_child_override :
  find_method "show" (st_methods child_type) =
  Some (mk_instance_method "show" 20).
Proof. reflexivity. Qed.

Example ex_child_mro :
  st_mro child_type = ["Child"; "Base"].
Proof. reflexivity. Qed.

(* Example: multiple inheritance - D(B, C) diamond *)
Definition diamond_D : StructType :=
  mkStructType 4 "D" [mk_field "x"; mk_field "y"; mk_field "z"; mk_field "w"]
    [] [] ["B"; "C"] ["D"; "B"; "C"; "A"] [] [].

Example ex_diamond_mro :
  st_mro diamond_D = ["D"; "B"; "C"; "A"].
Proof. reflexivity. Qed.

Example ex_diamond_parents :
  st_parent_names diamond_D = ["B"; "C"].
Proof. reflexivity. Qed.

(* Example: construction *)
Example ex_can_construct_exact :
  can_construct point_fields 2 = true.
Proof. reflexivity. Qed.

Example ex_cannot_construct_zero :
  can_construct point_fields 0 = false.
Proof. reflexivity. Qed.

Example ex_cannot_construct_three :
  can_construct point_fields 3 = false.
Proof. reflexivity. Qed.

(* Example with defaults: struct Config { host; port = 8080; } *)
Definition config_fields : FieldList :=
  [mk_field "host"; mk_field_default "port"].

Example ex_config_one_arg :
  can_construct config_fields 1 = true.
Proof. reflexivity. Qed.

Example ex_config_two_args :
  can_construct config_fields 2 = true.
Proof. reflexivity. Qed.

(* Example: CallMethod equivalence *)
Example ex_callmethod_equiv_method :
  call_method "inc"
    (mkStructType 0 "C" counter_fields counter_methods counter_statics [] ["C"] [] [])
    (mkInstance 0 [42%Z])
    99%Z [1%Z; 2%Z]
  =
  getattr_then_call "inc"
    (mkStructType 0 "C" counter_fields counter_methods counter_statics [] ["C"] [] [])
    (mkInstance 0 [42%Z])
    99%Z [1%Z; 2%Z].
Proof. reflexivity. Qed.

(* Example: abstract method enforcement *)
Definition abstract_type : StructType :=
  mkStructType 2 "Shape" [] [] [] [] ["Shape"] [] ["area"].

Example ex_abstract_blocks :
  has_abstract abstract_type.
Proof. unfold has_abstract. simpl. discriminate. Qed.

Definition concrete_type : StructType :=
  mkStructType 3 "Circle" [mk_field "r"]
    [mk_instance_method "area" 30] [] [] ["Circle"] [] [].

Example ex_concrete_ok :
  can_instantiate concrete_type.
Proof. unfold can_instantiate. reflexivity. Qed.
