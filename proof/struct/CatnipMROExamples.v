(* FILE: proof/struct/CatnipMROExamples.v *)
(* Concrete Examples: C3, Methods, Super, Fields
 *
 * Demonstrates all MRO features with concrete examples:
 *   - C3 linearization (linear, diamond, complex, inconsistent)
 *   - Method resolution (first-wins, child override)
 *   - Super chains (cooperative resolution)
 *   - Field operations (dedup, redefinition)
 *)

From Coq Require Import List ZArith Bool Lia PeanoNat.
From Coq Require Import String.
Import ListNotations.

From Catnip Require Import CatnipMROC3Core.
From Catnip Require Import CatnipMROMethods.
From Catnip Require Import CatnipMROSuper.
From Catnip Require Import CatnipMROFields.

Open Scope string_scope.
Open Scope list_scope.


(* ================================================================ *)
(* J. Concrete C3 Examples                                           *)
(* ================================================================ *)

(* --- Linear chain: A -> B -> C --- *)

Example linear_c3 :
  c3_linearize "C" ["B"] [["B"; "A"]] = Some ["C"; "B"; "A"].
Proof. reflexivity. Qed.

(* --- Diamond: D(B, C) where B(A), C(A) --- *)
(* MRO(A)=[A], MRO(B)=[B,A], MRO(C)=[C,A] *)
(* Expected MRO(D) = [D, B, C, A] *)

Example diamond_c3 :
  c3_linearize "D" ["B"; "C"] [["B"; "A"]; ["C"; "A"]] = Some ["D"; "B"; "C"; "A"].
Proof. reflexivity. Qed.

(* --- Complex diamond: E(C, D) where C(A,O), D(B,O), A(O), B(O) --- *)
(* Expected MRO(E) = [E, C, A, D, B, O] *)

Example complex_diamond_c3 :
  c3_linearize "E" ["C"; "D"] [["C"; "A"; "O"]; ["D"; "B"; "O"]]
  = Some ["E"; "C"; "A"; "D"; "B"; "O"].
Proof. reflexivity. Qed.

(* --- Inconsistent hierarchy: D(A, B) where A(X,Y), B(Y,X) --- *)
(* C3 cannot linearize: X before Y (from A) contradicts Y before X (from B) *)

Example inconsistent_c3 :
  c3_linearize "D" ["A"; "B"] [["A"; "X"; "Y"]; ["B"; "Y"; "X"]] = None.
Proof. reflexivity. Qed.

(* --- Single parent: B(A) --- *)

Example single_parent_c3 :
  c3_linearize "B" ["A"] [["A"]] = Some ["B"; "A"].
Proof. reflexivity. Qed.

(* --- Three independent parents: D(A, B, C) --- *)

Example three_parents_c3 :
  c3_linearize "D" ["A"; "B"; "C"] [["A"]; ["B"]; ["C"]]
  = Some ["D"; "A"; "B"; "C"].
Proof. reflexivity. Qed.


(* ================================================================ *)
(* K. Concrete Method Resolution Examples                            *)
(* ================================================================ *)

(* Diamond MRO(D) = [D, B, C, A], all define "value" *)
Definition methods_A := [mkMethod "value" "A" 1].
Definition methods_B := [mkMethod "value" "B" 2].
Definition methods_C := [mkMethod "value" "C" 3].
Definition methods_D : MethodMap := [].

(* First-wins: B's "value" is used (B before C in MRO) *)
Example diamond_method_resolution :
  find_method "value"
    (merge_methods_mro (methods_D ++ methods_B ++ methods_C ++ methods_A) [])
  = Some (mkMethod "value" "B" 2).
Proof. reflexivity. Qed.

(* Child override wins over all parents *)
Definition methods_D_override := [mkMethod "value" "D" 4].

Example child_override_wins :
  find_method "value"
    (merge_methods_mro (methods_D_override ++ methods_B ++ methods_C ++ methods_A) [])
  = Some (mkMethod "value" "D" 4).
Proof. reflexivity. Qed.


(* ================================================================ *)
(* L. Concrete Super Chain Examples                                  *)
(* ================================================================ *)

(* MRO = [D, B, C, A], type methods: *)
Definition diamond_tm : TypeMethods :=
  [("D", methods_D); ("B", methods_B); ("C", methods_C); ("A", methods_A)].

(* D.value -> super -> B.value (pos 0 -> skip to pos 1) *)
Example super_from_D :
  find_method "value" (super_at_position ["D"; "B"; "C"; "A"] 0 diamond_tm)
  = Some (mkMethod "value" "B" 2).
Proof. reflexivity. Qed.

(* B.value -> super -> C.value (pos 1 -> skip to pos 2)
   This is the key cooperative super property: B.super goes to C, not A! *)
Example super_from_B_in_diamond :
  find_method "value" (super_at_position ["D"; "B"; "C"; "A"] 1 diamond_tm)
  = Some (mkMethod "value" "C" 3).
Proof. reflexivity. Qed.

(* C.value -> super -> A.value (pos 2 -> skip to pos 3) *)
Example super_from_C_in_diamond :
  find_method "value" (super_at_position ["D"; "B"; "C"; "A"] 2 diamond_tm)
  = Some (mkMethod "value" "A" 1).
Proof. reflexivity. Qed.

(* A.value -> super -> None (pos 3 = last, nothing left) *)
Example super_from_A_none :
  find_method "value" (super_at_position ["D"; "B"; "C"; "A"] 3 diamond_tm)
  = None.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* M. Concrete Init Chaining Example                                 *)
(*                                                                    *)
(* D.init -> super.init -> B.init -> super.init -> C.init ->         *)
(* super.init -> A.init. Trace = "ACBD".                             *)
(* ================================================================ *)

Definition init_A := [mkMethod "init" "A" 10].
Definition init_B := [mkMethod "init" "B" 11].
Definition init_C := [mkMethod "init" "C" 12].
Definition init_D := [mkMethod "init" "D" 13].

Definition init_tm : TypeMethods :=
  [("D", init_D); ("B", init_B); ("C", init_C); ("A", init_A)].

Example init_chain_step_0 :
  find_method "init" (super_at_position ["D"; "B"; "C"; "A"] 0 init_tm)
  = Some (mkMethod "init" "B" 11).
Proof. reflexivity. Qed.

Example init_chain_step_1 :
  find_method "init" (super_at_position ["D"; "B"; "C"; "A"] 1 init_tm)
  = Some (mkMethod "init" "C" 12).
Proof. reflexivity. Qed.

Example init_chain_step_2 :
  find_method "init" (super_at_position ["D"; "B"; "C"; "A"] 2 init_tm)
  = Some (mkMethod "init" "A" 10).
Proof. reflexivity. Qed.

Example init_chain_step_3 :
  find_method "init" (super_at_position ["D"; "B"; "C"; "A"] 3 init_tm)
  = None.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* N. Concrete Field Redefinition Examples                           *)
(* ================================================================ *)

Definition field_x := mkField "x" false.
Definition field_y := mkField "y" false.
Definition field_z := mkField "z" false.
Definition field_w := mkField "w" false.

Example redefinition_caught :
  find (fun f => has_field_name (field_name f) [field_x])
    [mkField "x" false] = Some (mkField "x" false).
Proof. reflexivity. Qed.

Example no_redefinition_ok :
  find (fun f => has_field_name (field_name f) [field_x])
    [field_y; field_z] = None.
Proof. reflexivity. Qed.

(* Diamond field dedup example *)
(* B inherits x from A, C inherits x from A.
   Raw fields in MRO order [D, B, C, A]: w, (x,y), (x,z), x *)
Definition diamond_raw_fields : list FieldList :=
  [[field_w]; [field_x; field_y]; [field_x; field_z]; [field_x]].

Example diamond_dedup_correct :
  collect_mro_fields diamond_raw_fields =
  [field_w; field_x; field_y; field_z].
Proof. reflexivity. Qed.

Example diamond_x_once :
  count_field_name "x" (collect_mro_fields diamond_raw_fields) = 1.
Proof. reflexivity. Qed.
