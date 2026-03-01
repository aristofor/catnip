(* FILE: proof/struct/CatnipOpDesugarExamples.v *)
(* Concrete Examples: Operator Desugaring
 *
 * Demonstrates:
 *   - Basic symbol -> name mappings
 *   - Struct with operator methods (Vec2)
 *   - Disambiguation of shared symbols (- as unary/binary)
 *   - Invalid combination rejection
 *)

From Coq Require Import List String Bool.
Import ListNotations.

From Catnip Require Import CatnipStructBase.
From Catnip Require Import CatnipOpDesugar.

Open Scope string_scope.


(* ================================================================ *)
(* A. Basic Mapping Examples                                        *)
(* ================================================================ *)

Example desugar_plus_binary :
  desugar_operator SymPlus Binary = Some "op_add".
Proof. reflexivity. Qed.

Example desugar_minus_binary :
  desugar_operator SymMinus Binary = Some "op_sub".
Proof. reflexivity. Qed.

Example desugar_star_binary :
  desugar_operator SymStar Binary = Some "op_mul".
Proof. reflexivity. Qed.

Example desugar_pow_binary :
  desugar_operator SymPow Binary = Some "op_pow".
Proof. reflexivity. Qed.

Example desugar_eq_binary :
  desugar_operator SymEq Binary = Some "op_eq".
Proof. reflexivity. Qed.

Example desugar_lshift_binary :
  desugar_operator SymLShift Binary = Some "op_lshift".
Proof. reflexivity. Qed.

Example desugar_minus_unary :
  desugar_operator SymMinus Unary = Some "op_neg".
Proof. reflexivity. Qed.

Example desugar_plus_unary :
  desugar_operator SymPlus Unary = Some "op_pos".
Proof. reflexivity. Qed.

Example desugar_tilde_unary :
  desugar_operator SymTilde Unary = Some "op_bnot".
Proof. reflexivity. Qed.


(* ================================================================ *)
(* B. Struct with Operator Methods (Vec2)                           *)
(*                                                                  *)
(* A Vec2 struct implementing add, neg, and eq.                     *)
(* find_method retrieves each operator method.                      *)
(* ================================================================ *)

Definition vec2_methods : MethodMap :=
  [mkMethod "op_add" MkInstance 1;
   mkMethod "op_neg" MkInstance 2;
   mkMethod "op_eq"  MkInstance 3].

Example vec2_find_add :
  find_method "op_add" vec2_methods = Some (mkMethod "op_add" MkInstance 1).
Proof. reflexivity. Qed.

Example vec2_find_neg :
  find_method "op_neg" vec2_methods = Some (mkMethod "op_neg" MkInstance 2).
Proof. reflexivity. Qed.

Example vec2_find_eq :
  find_method "op_eq" vec2_methods = Some (mkMethod "op_eq" MkInstance 3).
Proof. reflexivity. Qed.

Example vec2_no_sub :
  find_method "op_sub" vec2_methods = None.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* C. Disambiguation: Minus Unary vs Binary                         *)
(*                                                                  *)
(* Same struct, both op_sub and op_neg, resolved independently.     *)
(* ================================================================ *)

Definition num_methods : MethodMap :=
  [mkMethod "op_add" MkInstance 1;
   mkMethod "op_sub" MkInstance 2;
   mkMethod "op_neg" MkInstance 3].

Example minus_as_binary :
  desugar_operator SymMinus Binary = Some "op_sub" /\
  find_method "op_sub" num_methods = Some (mkMethod "op_sub" MkInstance 2).
Proof. split; reflexivity. Qed.

Example minus_as_unary :
  desugar_operator SymMinus Unary = Some "op_neg" /\
  find_method "op_neg" num_methods = Some (mkMethod "op_neg" MkInstance 3).
Proof. split; reflexivity. Qed.


(* ================================================================ *)
(* D. Invalid Combinations                                          *)
(* ================================================================ *)

Example eq_not_unary :
  desugar_operator SymEq Unary = None.
Proof. reflexivity. Qed.

Example tilde_not_binary :
  desugar_operator SymTilde Binary = None.
Proof. reflexivity. Qed.

Example star_not_unary :
  desugar_operator SymStar Unary = None.
Proof. reflexivity. Qed.
