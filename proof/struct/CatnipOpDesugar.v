(* FILE: proof/struct/CatnipOpDesugar.v *)
(* Operator desugaring: symbol x arity -> method name
 *
 * Source of truth:
 *   catnip_rs/src/parser/pure_transforms.rs  (operator_symbol_to_method_name)
 *
 * Proves:
 *   - Injectivity: distinct (symbol, arity) pairs produce distinct names
 *   - Totality: all valid combinations produce a name
 *)

From Coq Require Import List String Bool.
Import ListNotations.

From Catnip Require Import CatnipIR.
From Catnip Require Import CatnipStructBase.

Open Scope string_scope.


(* ================================================================ *)
(* A. Operator Symbols and Arity                                    *)
(*                                                                  *)
(* 19 symbols supported by `op <symbol>` syntax.                    *)
(* ================================================================ *)

Inductive OperatorSymbol :=
  (* Arithmetic *)
  | SymPlus | SymMinus | SymStar | SymSlash | SymFloorDiv
  | SymPercent | SymPow
  (* Comparison *)
  | SymEq | SymNe | SymLt | SymLe | SymGt | SymGe
  (* Bitwise *)
  | SymAmp | SymPipe | SymCaret | SymLShift | SymRShift
  (* Unary only *)
  | SymTilde.

Inductive OpArity := Unary | Binary.

Lemma OperatorSymbol_eq_dec : forall (a b : OperatorSymbol), {a = b} + {a <> b}.
Proof. decide equality. Defined.

Lemma OpArity_eq_dec : forall (a b : OpArity), {a = b} + {a <> b}.
Proof. decide equality. Defined.


(* ================================================================ *)
(* B. Desugaring Function                                           *)
(*                                                                  *)
(* Mirrors operator_symbol_to_method_name(sym, param_count).        *)
(* 21 valid combinations out of 38 possible (19 x 2).              *)
(* ================================================================ *)

Definition desugar_operator (sym : OperatorSymbol) (ar : OpArity) : option string :=
  match sym, ar with
  (* Binary arithmetic *)
  | SymPlus, Binary     => Some "op_add"
  | SymMinus, Binary    => Some "op_sub"
  | SymStar, Binary     => Some "op_mul"
  | SymSlash, Binary    => Some "op_div"
  | SymFloorDiv, Binary => Some "op_floordiv"
  | SymPercent, Binary  => Some "op_mod"
  | SymPow, Binary      => Some "op_pow"
  (* Binary comparison *)
  | SymEq, Binary       => Some "op_eq"
  | SymNe, Binary       => Some "op_ne"
  | SymLt, Binary       => Some "op_lt"
  | SymLe, Binary       => Some "op_le"
  | SymGt, Binary       => Some "op_gt"
  | SymGe, Binary       => Some "op_ge"
  (* Binary bitwise *)
  | SymAmp, Binary      => Some "op_band"
  | SymPipe, Binary     => Some "op_bor"
  | SymCaret, Binary    => Some "op_bxor"
  | SymLShift, Binary   => Some "op_lshift"
  | SymRShift, Binary   => Some "op_rshift"
  (* Unary *)
  | SymMinus, Unary     => Some "op_neg"
  | SymPlus, Unary      => Some "op_pos"
  | SymTilde, Unary     => Some "op_bnot"
  (* Invalid combinations *)
  | _, _                => None
  end.

Definition valid_operator_arity (sym : OperatorSymbol) (ar : OpArity) : bool :=
  match desugar_operator sym ar with
  | Some _ => true
  | None   => false
  end.


(* ================================================================ *)
(* C. Method Name -> IR Opcode                                      *)
(*                                                                  *)
(* Links desugared names to their corresponding IROpCode.           *)
(* ================================================================ *)

Definition desugar_to_opcode (name : string) : option IROpCode :=
  if String.eqb name "op_add" then Some Add
  else if String.eqb name "op_sub" then Some Sub
  else if String.eqb name "op_mul" then Some Mul
  else if String.eqb name "op_div" then Some Div
  else if String.eqb name "op_floordiv" then Some FloorDiv
  else if String.eqb name "op_mod" then Some Mod
  else if String.eqb name "op_pow" then Some Pow
  else if String.eqb name "op_eq" then Some Eq
  else if String.eqb name "op_ne" then Some Ne
  else if String.eqb name "op_lt" then Some Lt
  else if String.eqb name "op_le" then Some Le
  else if String.eqb name "op_gt" then Some Gt
  else if String.eqb name "op_ge" then Some Ge
  else if String.eqb name "op_band" then Some BAnd
  else if String.eqb name "op_bor" then Some BOr
  else if String.eqb name "op_bxor" then Some BXor
  else if String.eqb name "op_lshift" then Some LShift
  else if String.eqb name "op_rshift" then Some RShift
  else if String.eqb name "op_neg" then Some Neg
  else if String.eqb name "op_pos" then Some Pos
  else if String.eqb name "op_bnot" then Some BNot
  else None.


(* ================================================================ *)
(* D. Injectivity                                                   *)
(*                                                                  *)
(* Distinct (symbol, arity) pairs never produce the same name.      *)
(* ================================================================ *)

Theorem desugar_injective : forall s1 a1 s2 a2 n,
  desugar_operator s1 a1 = Some n ->
  desugar_operator s2 a2 = Some n ->
  s1 = s2 /\ a1 = a2.
Proof.
  intros s1 a1 s2 a2 n H1 H2.
  destruct s1, a1; simpl in H1; try discriminate;
    inversion H1; subst;
    destruct s2, a2; simpl in H2; try discriminate;
    auto.
Qed.


(* ================================================================ *)
(* E. Totality                                                      *)
(*                                                                  *)
(* Every valid (symbol, arity) combination produces a method name.  *)
(* ================================================================ *)

Theorem desugar_total : forall s a,
  valid_operator_arity s a = true ->
  exists n, desugar_operator s a = Some n.
Proof.
  intros s a H. unfold valid_operator_arity in H.
  destruct (desugar_operator s a) eqn:Heq.
  - exists s0. reflexivity.
  - discriminate.
Qed.
