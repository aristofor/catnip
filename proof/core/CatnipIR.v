(* FILE: proof/core/CatnipIR.v *)
(* CatnipIR.v — Formal model of Catnip's Intermediate Representation
 *
 * Source of truth:
 *   catnip_rs/src/ir/opcode.rs  (IROpCode, 59 opcodes, repr(u8) 1..59)
 *   catnip_rs/src/ir/pure.rs    (IRPure, full IR node enum)
 *
 * Foundation for all downstream proofs:
 *   CatnipScopeProof, CatnipFunctionProof, CatnipPatternProof,
 *   CatnipOptimProof, CatnipVMProof, CatnipCFGProof.
 *
 * Position metadata (start_byte, end_byte) and kwargs omitted:
 * semantically irrelevant for correctness proofs.
 *)

From Coq Require Import List String ZArith Bool Lia PeanoNat.
Import ListNotations.


(* ================================================================ *)
(* A. IR OpCodes                                                      *)
(*                                                                    *)
(* 59 opcodes, ordered by repr(u8) value (1..59).                     *)
(* ================================================================ *)

Inductive IROpCode :=
  (* Control flow (1-9) *)
  | Nop | OpIf | OpWhile | OpFor | OpMatch
  | OpBlock | OpReturn | OpBreak | OpContinue
  (* Functions (10-13) *)
  | Call | OpLambda | FnDef | SetLocals
  (* Access (14-18) *)
  | GetAttr | SetAttr | GetItem | SetItem | OpSlice
  (* Arithmetic (19-28) *)
  | Add | Sub | Mul | Div | TrueDiv | FloorDiv | Mod | Pow
  | Neg | Pos
  (* Comparison (29-34) *)
  | Eq | Ne | Lt | Le | Gt | Ge
  (* Logical (35-37) *)
  | And | Or | Not
  (* Bitwise (38-43) *)
  | BAnd | BOr | BXor | BNot | LShift | RShift
  (* Broadcasting (44) *)
  | OpBroadcast
  (* Literals (45-48) *)
  | ListLiteral | TupleLiteral | SetLiteral | DictLiteral
  (* Stack (49-51) *)
  | Push | Pop | PushPeek
  (* Misc (52-53) *)
  | Fstring | Pragma
  (* ND (54-56) *)
  | NdRecursion | NdMap | NdEmptyTopos
  (* Debug (57) *)
  | Breakpoint
  (* Structures (58-59) *)
  | OpStruct | TraitDef.

Lemma IROpCode_eq_dec : forall (a b : IROpCode), {a = b} + {a <> b}.
Proof. decide equality. Defined.


(* ================================================================ *)
(* B. Broadcast Type                                                  *)
(* ================================================================ *)

Inductive BroadcastType :=
  | BtBinary | BtUnary | BtIf | BtLambda | BtNDMap | BtNDRecursion.

Lemma BroadcastType_eq_dec : forall (a b : BroadcastType), {a = b} + {a <> b}.
Proof. decide equality. Defined.


(* ================================================================ *)
(* C. IRPure — full IR node type                                      *)
(*                                                                    *)
(* Float abstracted as Z (proofs reason about structure, not IEEE).    *)
(* ================================================================ *)

Inductive IRPure :=
  (* Literals *)
  | IRInt (n : Z)
  | IRFloat (n : Z)
  | IRString (s : string)
  | IRBytes (bs : list nat)
  | IRBool (b : bool)
  | IRNone
  | IRDecimal (s : string)
  | IRImaginary (s : string)
  (* Operation *)
  | IROp (opcode : IROpCode) (args : list IRPure) (tail : bool)
  (* Variables *)
  | IRIdentifier (name : string)
  | IRRef (name : string)
  (* Collections *)
  | IRList (elts : list IRPure)
  | IRProgram (stmts : list IRPure)
  | IRTuple (elts : list IRPure)
  | IRDict (entries : list (IRPure * IRPure))
  | IRSet (elts : list IRPure)
  (* Function call *)
  | IRCall (func : IRPure) (args : list IRPure)
  (* Patterns *)
  | IRPatternLiteral (inner : IRPure)
  | IRPatternVar (name : string)
  | IRPatternWildcard
  | IRPatternOr (pats : list IRPure)
  | IRPatternTuple (pats : list IRPure)
  | IRPatternStruct (name : string) (fields : list string)
  (* Slice *)
  | IRSlice (start stop step : IRPure)
  (* Broadcasting *)
  | IRBroadcast (target : option IRPure) (operator : IRPure)
                (operand : option IRPure) (bt : BroadcastType).


(* ================================================================ *)
(* D. Structural Size                                                 *)
(*                                                                    *)
(* Local fixpoints (let fix) for nested types: list IRPure,           *)
(* list (IRPure * IRPure), option IRPure.                             *)
(* Needed for well-founded recursion downstream.                      *)
(* ================================================================ *)

Fixpoint ir_size (ir : IRPure) : nat :=
  let fix list_size (l : list IRPure) : nat :=
    match l with
    | [] => 0
    | x :: xs => ir_size x + list_size xs
    end
  in
  let fix pair_list_size (l : list (IRPure * IRPure)) : nat :=
    match l with
    | [] => 0
    | (k, v) :: xs => ir_size k + ir_size v + pair_list_size xs
    end
  in
  let opt_size (o : option IRPure) : nat :=
    match o with
    | None => 0
    | Some x => ir_size x
    end
  in
  match ir with
  | IRInt _ | IRFloat _ | IRString _ | IRBool _ | IRNone
  | IRDecimal _ | IRImaginary _
  | IRIdentifier _ | IRRef _ | IRPatternVar _
  | IRPatternWildcard | IRPatternStruct _ _ => 1
  | IRBytes bs => 1 + List.length bs
  | IROp _ args _ => 1 + list_size args
  | IRList elts | IRProgram elts | IRTuple elts | IRSet elts
  | IRPatternOr elts | IRPatternTuple elts => 1 + list_size elts
  | IRDict entries => 1 + pair_list_size entries
  | IRCall f args => 1 + ir_size f + list_size args
  | IRPatternLiteral inner => 1 + ir_size inner
  | IRSlice a b c => 1 + ir_size a + ir_size b + ir_size c
  | IRBroadcast tgt op opnd _ =>
      1 + opt_size tgt + ir_size op + opt_size opnd
  end.

Lemma ir_size_pos : forall ir, (ir_size ir >= 1)%nat.
Proof. destruct ir; simpl; lia. Qed.


(* ================================================================ *)
(* E. Basic Predicates                                                *)
(* ================================================================ *)

Definition is_literal (ir : IRPure) : bool :=
  match ir with
  | IRInt _ | IRFloat _ | IRString _ | IRBytes _
  | IRBool _ | IRNone | IRDecimal _ | IRImaginary _ => true
  | _ => false
  end.

Definition is_op (ir : IRPure) : bool :=
  match ir with
  | IROp _ _ _ => true
  | _ => false
  end.

Definition is_call (ir : IRPure) : bool :=
  match ir with
  | IRCall _ _ => true
  | _ => false
  end.

Definition is_pattern (ir : IRPure) : bool :=
  match ir with
  | IRPatternLiteral _ | IRPatternVar _ | IRPatternWildcard
  | IRPatternOr _ | IRPatternTuple _ | IRPatternStruct _ _ => true
  | _ => false
  end.

Definition is_collection (ir : IRPure) : bool :=
  match ir with
  | IRList _ | IRTuple _ | IRDict _ | IRSet _ => true
  | _ => false
  end.

Definition opcode_of (ir : IRPure) : option IROpCode :=
  match ir with
  | IROp oc _ _ => Some oc
  | _ => None
  end.

Definition is_tail (ir : IRPure) : bool :=
  match ir with
  | IROp _ _ true => true
  | _ => false
  end.

Definition mark_tail (ir : IRPure) : IRPure :=
  match ir with
  | IROp oc args _ => IROp oc args true
  | other => other
  end.


(* ================================================================ *)
(* F. Properties                                                      *)
(* ================================================================ *)

Lemma literal_not_op : forall ir,
  is_literal ir = true -> is_op ir = false.
Proof. destruct ir; simpl; intros H; try discriminate; reflexivity. Qed.

Lemma op_not_literal : forall ir,
  is_op ir = true -> is_literal ir = false.
Proof. destruct ir; simpl; intros H; try discriminate; reflexivity. Qed.

Lemma literal_not_pattern : forall ir,
  is_literal ir = true -> is_pattern ir = false.
Proof. destruct ir; simpl; intros H; try discriminate; reflexivity. Qed.

Lemma pattern_not_literal : forall ir,
  is_pattern ir = true -> is_literal ir = false.
Proof. destruct ir; simpl; intros H; try discriminate; reflexivity. Qed.

Lemma opcode_of_some_iff_is_op : forall ir,
  is_op ir = true <-> exists oc, opcode_of ir = Some oc.
Proof.
  split.
  - destruct ir; simpl; intros H; try discriminate.
    eexists. reflexivity.
  - destruct ir; simpl; intros [oc H]; try discriminate. reflexivity.
Qed.

Lemma mark_tail_idempotent : forall ir,
  mark_tail (mark_tail ir) = mark_tail ir.
Proof. destruct ir; simpl; reflexivity. Qed.

Lemma mark_tail_preserves_opcode : forall ir,
  opcode_of (mark_tail ir) = opcode_of ir.
Proof. destruct ir; simpl; reflexivity. Qed.

Lemma mark_tail_is_tail : forall oc args t,
  is_tail (mark_tail (IROp oc args t)) = true.
Proof. reflexivity. Qed.

Lemma is_literal_size : forall ir,
  is_literal ir = true ->
  match ir with
  | IRBytes bs => ir_size ir = (1 + List.length bs)%nat
  | _ => ir_size ir = 1%nat
  end.
Proof. destruct ir; simpl; intros H; try discriminate; reflexivity. Qed.


(* ================================================================ *)
(* G. Control Flow Classification                                     *)
(*                                                                    *)
(* Arguments passed unevaluated (lazy semantics).                     *)
(* Matches CONTROL_FLOW_OPS in semantic/opcode.py.                    *)
(* ================================================================ *)

Definition is_control_flow_op (oc : IROpCode) : bool :=
  match oc with
  | OpIf | OpWhile | OpFor | OpMatch | OpBlock
  | Call | OpLambda | FnDef | SetLocals => true
  | _ => false
  end.

Definition is_arithmetic_op (oc : IROpCode) : bool :=
  match oc with
  | Add | Sub | Mul | Div | TrueDiv | FloorDiv | Mod | Pow => true
  | _ => false
  end.

Definition is_comparison_op (oc : IROpCode) : bool :=
  match oc with
  | Eq | Ne | Lt | Le | Gt | Ge => true
  | _ => false
  end.

Definition is_logical_op (oc : IROpCode) : bool :=
  match oc with
  | And | Or | Not => true
  | _ => false
  end.

Definition is_bitwise_op (oc : IROpCode) : bool :=
  match oc with
  | BAnd | BOr | BXor | BNot | LShift | RShift => true
  | _ => false
  end.

(* Disjointness: control flow vs arithmetic *)
Lemma control_flow_not_arithmetic : forall oc,
  is_control_flow_op oc = true ->
  is_arithmetic_op oc = false.
Proof. destruct oc; simpl; intros H; try discriminate; reflexivity. Qed.

Lemma arithmetic_not_control_flow : forall oc,
  is_arithmetic_op oc = true ->
  is_control_flow_op oc = false.
Proof. destruct oc; simpl; intros H; try discriminate; reflexivity. Qed.


(* ================================================================ *)
(* H. Opcode Numbering (repr(u8) bijection)                           *)
(* ================================================================ *)

Definition opcode_to_nat (oc : IROpCode) : nat :=
  match oc with
  | Nop => 1 | OpIf => 2 | OpWhile => 3 | OpFor => 4 | OpMatch => 5
  | OpBlock => 6 | OpReturn => 7 | OpBreak => 8 | OpContinue => 9
  | Call => 10 | OpLambda => 11 | FnDef => 12 | SetLocals => 13
  | GetAttr => 14 | SetAttr => 15 | GetItem => 16 | SetItem => 17
  | OpSlice => 18
  | Add => 19 | Sub => 20 | Mul => 21 | Div => 22 | TrueDiv => 23
  | FloorDiv => 24 | Mod => 25 | Pow => 26 | Neg => 27 | Pos => 28
  | Eq => 29 | Ne => 30 | Lt => 31 | Le => 32 | Gt => 33 | Ge => 34
  | And => 35 | Or => 36 | Not => 37
  | BAnd => 38 | BOr => 39 | BXor => 40 | BNot => 41
  | LShift => 42 | RShift => 43
  | OpBroadcast => 44
  | ListLiteral => 45 | TupleLiteral => 46 | SetLiteral => 47
  | DictLiteral => 48
  | Push => 49 | Pop => 50 | PushPeek => 51
  | Fstring => 52 | Pragma => 53
  | NdRecursion => 54 | NdMap => 55 | NdEmptyTopos => 56
  | Breakpoint => 57
  | OpStruct => 58 | TraitDef => 59
  end.

Theorem opcode_to_nat_injective : forall a b,
  opcode_to_nat a = opcode_to_nat b -> a = b.
Proof.
  destruct a; destruct b; simpl; intro H;
    try reflexivity; discriminate.
Qed.

Theorem opcode_to_nat_range : forall oc,
  (1 <= opcode_to_nat oc <= 59)%nat.
Proof. destruct oc; simpl; lia. Qed.

Definition nat_to_opcode (n : nat) : option IROpCode :=
  match n with
  | 1 => Some Nop | 2 => Some OpIf | 3 => Some OpWhile
  | 4 => Some OpFor | 5 => Some OpMatch | 6 => Some OpBlock
  | 7 => Some OpReturn | 8 => Some OpBreak | 9 => Some OpContinue
  | 10 => Some Call | 11 => Some OpLambda | 12 => Some FnDef
  | 13 => Some SetLocals | 14 => Some GetAttr | 15 => Some SetAttr
  | 16 => Some GetItem | 17 => Some SetItem | 18 => Some OpSlice
  | 19 => Some Add | 20 => Some Sub | 21 => Some Mul
  | 22 => Some Div | 23 => Some TrueDiv | 24 => Some FloorDiv
  | 25 => Some Mod | 26 => Some Pow | 27 => Some Neg | 28 => Some Pos
  | 29 => Some Eq | 30 => Some Ne | 31 => Some Lt
  | 32 => Some Le | 33 => Some Gt | 34 => Some Ge
  | 35 => Some And | 36 => Some Or | 37 => Some Not
  | 38 => Some BAnd | 39 => Some BOr | 40 => Some BXor
  | 41 => Some BNot | 42 => Some LShift | 43 => Some RShift
  | 44 => Some OpBroadcast
  | 45 => Some ListLiteral | 46 => Some TupleLiteral
  | 47 => Some SetLiteral | 48 => Some DictLiteral
  | 49 => Some Push | 50 => Some Pop | 51 => Some PushPeek
  | 52 => Some Fstring | 53 => Some Pragma
  | 54 => Some NdRecursion | 55 => Some NdMap | 56 => Some NdEmptyTopos
  | 57 => Some Breakpoint
  | 58 => Some OpStruct | 59 => Some TraitDef
  | _ => None
  end.

Theorem opcode_roundtrip : forall oc,
  nat_to_opcode (opcode_to_nat oc) = Some oc.
Proof. destruct oc; reflexivity. Qed.

Theorem nat_to_opcode_roundtrip : forall n oc,
  nat_to_opcode n = Some oc -> opcode_to_nat oc = n.
Proof.
  intros n oc H.
  destruct n as [|n']; simpl in H; try discriminate.
  do 59 (destruct n' as [|n']; [inversion H; reflexivity|]).
  simpl in H. discriminate.
Qed.


(* ================================================================ *)
(* I. Smart Constructors                                              *)
(*                                                                    *)
(* Match IRPure::op() and IRPure::call() from pure.rs.                *)
(* ================================================================ *)

Definition ir_op (oc : IROpCode) (args : list IRPure) : IRPure :=
  IROp oc args false.

Definition ir_call (f : IRPure) (args : list IRPure) : IRPure :=
  IRCall f args.

Definition ir_binop (oc : IROpCode) (l r : IRPure) : IRPure :=
  ir_op oc [IRTuple [l; r]].

Lemma ir_op_not_tail : forall oc args,
  is_tail (ir_op oc args) = false.
Proof. reflexivity. Qed.

Lemma ir_op_opcode : forall oc args,
  opcode_of (ir_op oc args) = Some oc.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* J. Well-formedness                                                 *)
(*                                                                    *)
(* Structural invariants on IR nodes. A well-formed IR is one that    *)
(* could have been produced by the parser + semantic analyzer.        *)
(* ================================================================ *)

(* Arity: expected number of args per opcode.
   None = variadic (any arity accepted). *)
Definition opcode_arity (oc : IROpCode) : option nat :=
  match oc with
  (* Unary *)
  | Neg | Pos | BNot | Not | OpReturn | OpBreak => Some 1
  (* Binary (args wrapped in Tuple) *)
  | Add | Sub | Mul | Div | TrueDiv | FloorDiv | Mod | Pow
  | Eq | Ne | Lt | Le | Gt | Ge
  | And | Or | BAnd | BOr | BXor | LShift | RShift
  | GetAttr | SetAttr | GetItem | SetItem => Some 1
  (* Ternary *)
  | OpIf => Some 3
  | OpSlice => Some 3
  (* Control flow: variadic *)
  | OpWhile | OpFor | OpMatch | OpBlock
  | Call | OpLambda | FnDef | SetLocals => None
  (* Literals: variadic *)
  | ListLiteral | TupleLiteral | SetLiteral | DictLiteral => None
  (* Misc: variadic *)
  | _ => None
  end.

Definition arity_ok (ir : IRPure) : bool :=
  match ir with
  | IROp oc args _ =>
      match opcode_arity oc with
      | Some n => Nat.eqb (List.length args) n
      | None => true
      end
  | _ => true
  end.

Lemma arity_ok_variadic : forall oc args t,
  opcode_arity oc = None ->
  arity_ok (IROp oc args t) = true.
Proof.
  intros oc args t H. unfold arity_ok. rewrite H. reflexivity.
Qed.
