(* FILE: proof/optim/CatnipPurityProof.v *)
(* Optimization safety under operator overloading
 *
 * Source of truth:
 *   catnip_rs/src/semantic/common_subexpression_elimination.rs  (pure_ops)
 *   catnip_rs/src/vm/core.rs  (try_struct_binop -> method call)
 *
 * Key insight: struct operator overloads desugar to method calls (Call),
 * and Call is never in pure_ops. Therefore CSE/DCE/folding/LICM skip
 * struct operator expressions by construction.
 *
 * Proves:
 *   - pure_op classifies exactly the builtin opcodes
 *   - Call (and other control flow) is not pure
 *   - Struct operator dispatch produces Call, not the builtin opcode
 *   - Struct operator expressions are never optimization-eligible
 *)

From Coq Require Import List String Bool ZArith.
Import ListNotations.

From Catnip Require Import CatnipIR.
From Catnip Require Import CatnipOpDesugar.

Open Scope string_scope.


(* ================================================================ *)
(* A. Pure Opcodes                                                  *)
(*                                                                  *)
(* Mirrors the pure_ops HashSet in                                  *)
(* common_subexpression_elimination.rs:27-63.                       *)
(* An opcode is pure iff it has no side effects and same inputs     *)
(* always produce same outputs.                                     *)
(* ================================================================ *)

Definition pure_op (op : IROpCode) : bool :=
  match op with
  (* Arithmetic *)
  | Add | Sub | Mul | Div | TrueDiv | FloorDiv | Mod | Pow
  | Neg | Pos => true
  (* Comparison *)
  | Eq | Ne | Lt | Le | Gt | Ge => true
  (* Logical *)
  | And | Or | Not => true
  (* Bitwise *)
  | BAnd | BOr | BXor | BNot | LShift | RShift => true
  (* Member access *)
  | GetAttr | GetItem => true
  (* Everything else *)
  | _ => false
  end.


(* ================================================================ *)
(* B. Call Is Not Pure                                               *)
(*                                                                  *)
(* Method calls can have arbitrary side effects.                    *)
(* ================================================================ *)

Lemma call_not_pure : pure_op Call = false.
Proof. reflexivity. Qed.

Lemma fn_def_not_pure : pure_op FnDef = false.
Proof. reflexivity. Qed.

Lemma lambda_not_pure : pure_op OpLambda = false.
Proof. reflexivity. Qed.

Lemma set_locals_not_pure : pure_op SetLocals = false.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* C. Struct Operator Dispatch Model                                *)
(*                                                                  *)
(* When a binary op hits a struct, the VM rewrites it as a method   *)
(* call. The IR node changes from IROp Add [...] to                *)
(* IROp Call [method_ref; args...].                                 *)
(*                                                                  *)
(* This models the transformation in try_struct_binop               *)
(* (vm/core.rs:5823).                                               *)
(* ================================================================ *)

(* An expression is optimization-eligible if its top-level opcode is pure *)
Definition optim_eligible (ir : IRPure) : bool :=
  match ir with
  | IROp op _ _ => pure_op op
  | _ => false
  end.

(* Struct operator call: IROp Call [method_ref, self, other] *)
Definition struct_op_call (method : string) (self other : IRPure) : IRPure :=
  IROp Call [IRRef method; self; other] false.

(* Struct unary call: IROp Call [method_ref, self] *)
Definition struct_unop_call (method : string) (self : IRPure) : IRPure :=
  IROp Call [IRRef method; self] false.

Theorem struct_op_not_eligible : forall method self other,
  optim_eligible (struct_op_call method self other) = false.
Proof. reflexivity. Qed.

Theorem struct_unop_not_eligible : forall method self,
  optim_eligible (struct_unop_call method self) = false.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* D. Builtin vs Struct: Eligibility Split                          *)
(*                                                                  *)
(* Builtin ops (on primitives) ARE eligible.                        *)
(* The same symbol on structs is NOT eligible (dispatched as Call). *)
(* ================================================================ *)

(* Builtin expression: IROp Add [a, b] *)
Definition builtin_binop (op : IROpCode) (a b : IRPure) : IRPure :=
  IROp op [a; b] false.

(* All arithmetic/comparison/bitwise builtins are eligible *)
Theorem builtin_arith_eligible :
  optim_eligible (builtin_binop Add (IRInt 1) (IRInt 2)) = true /\
  optim_eligible (builtin_binop Sub (IRInt 1) (IRInt 2)) = true /\
  optim_eligible (builtin_binop Mul (IRInt 1) (IRInt 2)) = true /\
  optim_eligible (builtin_binop Eq  (IRInt 1) (IRInt 2)) = true /\
  optim_eligible (builtin_binop BAnd (IRInt 1) (IRInt 2)) = true.
Proof. repeat split; reflexivity. Qed.

(* Same symbol on struct -> not eligible *)
Theorem struct_add_not_eligible :
  forall self other,
  optim_eligible (struct_op_call "op_add" self other) = false.
Proof. reflexivity. Qed.

(* General: for any valid operator symbol, the struct dispatch is not eligible *)
Theorem overloaded_op_never_eligible :
  forall sym name self other,
    desugar_operator sym Binary = Some name ->
    optim_eligible (struct_op_call name self other) = false.
Proof.
  intros sym name self other H.
  unfold struct_op_call, optim_eligible, pure_op. reflexivity.
Qed.

Theorem overloaded_unop_never_eligible :
  forall sym name self,
    desugar_operator sym Unary = Some name ->
    optim_eligible (struct_unop_call name self) = false.
Proof.
  intros sym name self H.
  unfold struct_unop_call, optim_eligible, pure_op. reflexivity.
Qed.


(* ================================================================ *)
(* E. Completeness: All Desugared Opcodes Are Pure as Builtins      *)
(*                                                                  *)
(* Every operator symbol that desugars to a method name corresponds *)
(* to a builtin opcode that IS pure. This means the optimization    *)
(* is safe on primitives and correctly skipped on structs.          *)
(* ================================================================ *)

Theorem desugared_builtins_are_pure : forall sym ar name opc,
  desugar_operator sym ar = Some name ->
  desugar_to_opcode name = Some opc ->
  pure_op opc = true.
Proof.
  destruct sym, ar; simpl; intros name opc H1 H2;
    try discriminate; inversion H1; subst; simpl in H2;
    inversion H2; subst; reflexivity.
Qed.


(* ================================================================ *)
(* F. Concrete Examples                                             *)
(* ================================================================ *)

(* Vec2 + Vec2: not eligible (struct dispatch) *)
Example vec2_add_not_eligible :
  optim_eligible (struct_op_call "op_add" (IRInt 0) (IRInt 0)) = false.
Proof. reflexivity. Qed.

(* 1 + 2: eligible (builtin) *)
Example int_add_eligible :
  optim_eligible (builtin_binop Add (IRInt 1) (IRInt 2)) = true.
Proof. reflexivity. Qed.

(* -v (unary neg on struct): not eligible *)
Example struct_neg_not_eligible :
  optim_eligible (struct_unop_call "op_neg" (IRInt 0)) = false.
Proof. reflexivity. Qed.

(* -5 (unary neg on int): eligible *)
Example int_neg_eligible :
  optim_eligible (IROp Neg [IRInt 5] false) = true.
Proof. reflexivity. Qed.
