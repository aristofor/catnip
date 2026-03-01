(* FILE: proof/struct/CatnipOpDesugarProps.v *)
(* Operator desugaring properties
 *
 * Proves:
 *   - Arity disambiguation for + and -
 *   - All 21 output names are pairwise distinct
 *   - Invalid combinations return None
 *   - Desugared methods are resolvable via find_method
 *   - Consistency with IR opcodes
 *   - All names share the "op_" prefix
 *)

From Coq Require Import List String Bool Ascii.
Import ListNotations.

From Catnip Require Import CatnipIR.
From Catnip Require Import CatnipStructBase.
From Catnip Require Import CatnipOpDesugar.

Open Scope string_scope.


(* ================================================================ *)
(* A. Arity Disambiguation                                          *)
(*                                                                  *)
(* + and - are the only symbols valid as both unary and binary.     *)
(* They always produce different method names.                      *)
(* ================================================================ *)

Theorem arity_disambiguation_minus :
  desugar_operator SymMinus Unary <> desugar_operator SymMinus Binary.
Proof. simpl. discriminate. Qed.

Theorem arity_disambiguation_plus :
  desugar_operator SymPlus Unary <> desugar_operator SymPlus Binary.
Proof. simpl. discriminate. Qed.


(* ================================================================ *)
(* B. Pairwise Distinctness                                         *)
(*                                                                  *)
(* Direct corollary of injectivity: different inputs, different     *)
(* outputs.                                                         *)
(* ================================================================ *)

Theorem desugar_names_distinct : forall s1 a1 s2 a2 n1 n2,
  desugar_operator s1 a1 = Some n1 ->
  desugar_operator s2 a2 = Some n2 ->
  (s1 <> s2 \/ a1 <> a2) ->
  n1 <> n2.
Proof.
  intros s1 a1 s2 a2 n1 n2 H1 H2 Hneq Heq. subst n2.
  destruct (desugar_injective _ _ _ _ _ H1 H2) as [Hs Ha].
  destruct Hneq as [Hns | Hna]; contradiction.
Qed.


(* ================================================================ *)
(* C. Invalid Combinations                                          *)
(* ================================================================ *)

Theorem invalid_combinations_fail :
  desugar_operator SymEq Unary = None /\
  desugar_operator SymNe Unary = None /\
  desugar_operator SymLt Unary = None /\
  desugar_operator SymLe Unary = None /\
  desugar_operator SymGt Unary = None /\
  desugar_operator SymGe Unary = None /\
  desugar_operator SymStar Unary = None /\
  desugar_operator SymSlash Unary = None /\
  desugar_operator SymFloorDiv Unary = None /\
  desugar_operator SymPercent Unary = None /\
  desugar_operator SymPow Unary = None /\
  desugar_operator SymAmp Unary = None /\
  desugar_operator SymPipe Unary = None /\
  desugar_operator SymCaret Unary = None /\
  desugar_operator SymLShift Unary = None /\
  desugar_operator SymRShift Unary = None /\
  desugar_operator SymTilde Binary = None.
Proof. repeat split; reflexivity. Qed.


(* ================================================================ *)
(* D. Method Resolvability                                          *)
(*                                                                  *)
(* If a struct has a method with the desugared name,                *)
(* find_method (from CatnipStructBase) retrieves it.                *)
(* ================================================================ *)

Lemma find_method_exists : forall name kind id methods,
  In (mkMethod name kind id) methods ->
  exists m, find_method name methods = Some m.
Proof.
  induction methods as [|x rest IH]; intro Hin.
  - contradiction.
  - destruct Hin as [-> | Hin'].
    + exists (mkMethod name kind id). unfold find_method. simpl.
      rewrite string_eqb_refl. reflexivity.
    + specialize (IH Hin'). destruct IH as [m Hm].
      unfold find_method in *. simpl.
      destruct (String.eqb (method_name x) name) eqn:E.
      * eexists. reflexivity.
      * exists m. exact Hm.
Qed.

Theorem desugar_method_resolvable : forall sym ar name kind id methods,
  desugar_operator sym ar = Some name ->
  In (mkMethod name kind id) methods ->
  exists m, find_method name methods = Some m.
Proof.
  intros sym ar name kind id methods _ Hin.
  exact (find_method_exists name kind id methods Hin).
Qed.


(* ================================================================ *)
(* E. Opcode Consistency                                            *)
(*                                                                  *)
(* The desugared name maps to the expected IR opcode.               *)
(* ================================================================ *)

Definition expected_opcode (sym : OperatorSymbol) (ar : OpArity) : option IROpCode :=
  match sym, ar with
  | SymPlus, Binary     => Some Add
  | SymMinus, Binary    => Some Sub
  | SymStar, Binary     => Some Mul
  | SymSlash, Binary    => Some Div
  | SymFloorDiv, Binary => Some FloorDiv
  | SymPercent, Binary  => Some Mod
  | SymPow, Binary      => Some Pow
  | SymEq, Binary       => Some Eq
  | SymNe, Binary       => Some Ne
  | SymLt, Binary       => Some Lt
  | SymLe, Binary       => Some Le
  | SymGt, Binary       => Some Gt
  | SymGe, Binary       => Some Ge
  | SymAmp, Binary      => Some BAnd
  | SymPipe, Binary     => Some BOr
  | SymCaret, Binary    => Some BXor
  | SymLShift, Binary   => Some LShift
  | SymRShift, Binary   => Some RShift
  | SymMinus, Unary     => Some Neg
  | SymPlus, Unary      => Some Pos
  | SymTilde, Unary     => Some BNot
  | _, _                => None
  end.

Theorem desugar_opcode_consistent : forall sym ar name,
  desugar_operator sym ar = Some name ->
  desugar_to_opcode name = expected_opcode sym ar.
Proof.
  destruct sym, ar; simpl; intro name; intro H; try discriminate;
    inversion H; subst; reflexivity.
Qed.


(* ================================================================ *)
(* F. Prefix Invariant                                              *)
(*                                                                  *)
(* All desugared names start with "op_".                            *)
(* ================================================================ *)

Definition has_op_prefix (s : string) : bool :=
  match s with
  | String "o"%char (String "p"%char (String "_"%char _)) => true
  | _ => false
  end.

Lemma op_prefix_preserved : forall sym ar name,
  desugar_operator sym ar = Some name ->
  has_op_prefix name = true.
Proof.
  destruct sym, ar; simpl; intro name; intro H; try discriminate;
    inversion H; subst; reflexivity.
Qed.
