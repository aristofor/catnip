(* FILE: proof/expr/CatnipBroadcastOverload.v *)
(* Broadcast and ND invariants under operator overloading
 *
 * Source of truth:
 *   catnip_rs/src/core/registry/broadcast/  (apply_broadcast)
 *   catnip_rs/src/vm/core.rs                (try_struct_binop)
 *
 * Key insight: broadcast_map is parametric over f : Val -> Val.
 * A struct operator method is just a function Val -> Val (after
 * partial application of the operand). All shape invariants
 * (length preservation, empty fixpoint, composition, determinism)
 * hold for ANY f, including overloaded operators.
 *
 * This file instantiates the existing parametric proofs with
 * operator overload dispatch functions, producing corollaries.
 *
 * Depends on:
 *   CatnipDimensional.v      (broadcast_map, shape invariants)
 *   CatnipNDRecursion.v      (nd_eval, monotonicity, determinism)
 *   CatnipPurityProof.v      (optim_eligible, struct_op_call)
 *)

From Coq Require Import List Bool Arith PeanoNat Lia String.
Import ListNotations.

From Catnip Require Import CatnipDimensional.
From Catnip Require Import CatnipNDRecursion.
From Catnip Require Import CatnipIR.
From Catnip Require Import CatnipPurityProof.


(* ================================================================ *)
(* A. Operator Overload as broadcast_map argument                   *)
(*                                                                  *)
(* A binary operator overload on a struct is a function             *)
(*   op_method : Val -> Val -> Val                                  *)
(* Broadcasting partially applies the second operand:               *)
(*   broadcast_map (fun x => op_method x operand) collection        *)
(* ================================================================ *)

Section OverloadedBroadcast.

(* An arbitrary struct method: no assumptions on its behavior *)
Variable op_method : Val -> Val -> Val.

Definition overloaded_binop (operand : Val) : Val -> Val :=
  fun x => op_method x operand.

(* Shape preservation: length is preserved regardless of op_method *)
Theorem overloaded_broadcast_preserves_length :
  forall operand xs,
    match broadcast_map (overloaded_binop operand) (Coll xs) with
    | Coll ys => List.length ys = List.length xs
    | _ => False
    end.
Proof.
  intros. apply broadcast_preserves_length.
Qed.

(* Empty fixpoint: broadcasting over @[] yields @[] *)
Theorem overloaded_broadcast_empty :
  forall operand,
    broadcast_map (overloaded_binop operand) empty_topos = empty_topos.
Proof.
  intros. apply broadcast_empty_fixed.
Qed.

(* Composition: chaining two overloaded broadcasts fuses *)
Theorem overloaded_broadcast_composition :
  forall (op1 op2 : Val -> Val -> Val) operand1 operand2 xs,
    broadcast_map (fun x => op2 x operand2)
      (broadcast_map (fun x => op1 x operand1) (Coll xs)) =
    broadcast_map (fun x => op2 (op1 x operand1) operand2) (Coll xs).
Proof.
  intros. apply coherence_composition.
Qed.

(* Determinism: evaluation with overloaded ops is deterministic *)
Theorem overloaded_eval_deterministic :
  forall operand e v1 v2,
    eval (EBroadMap e (overloaded_binop operand)) v1 ->
    eval (EBroadMap e (overloaded_binop operand)) v2 ->
    v1 = v2.
Proof.
  intros. exact (eval_deterministic _ _ _ H H0).
Qed.

End OverloadedBroadcast.


(* ================================================================ *)
(* B. Unary Operator Overload                                       *)
(* ================================================================ *)

Section UnaryOverload.

Variable unary_method : Val -> Val.

Theorem unary_broadcast_preserves_length :
  forall xs,
    match broadcast_map unary_method (Coll xs) with
    | Coll ys => List.length ys = List.length xs
    | _ => False
    end.
Proof.
  intros. apply broadcast_preserves_length.
Qed.

Theorem unary_broadcast_empty :
  broadcast_map unary_method empty_topos = empty_topos.
Proof. apply broadcast_empty_fixed. Qed.

End UnaryOverload.


(* ================================================================ *)
(* C. ND-Recursion with Overloaded Operators                        *)
(*                                                                  *)
(* nd_eval is parametric over base, step_seed, combine.             *)
(* When combine uses an overloaded operator, monotonicity and       *)
(* determinism still hold (they depend only on the recursion        *)
(* structure, not on what combine does).                            *)
(* ================================================================ *)

Section NDOverload.

Variable base : nat -> bool.
Variable base_val : nat -> nat.
Variable step_seed : nat -> nat.
(* combine may use an overloaded operator internally *)
Variable combine : nat -> nat -> nat.

Theorem nd_overloaded_deterministic :
  forall fuel1 fuel2 seed v1 v2,
    nd_eval base base_val step_seed combine fuel1 seed = Some v1 ->
    nd_eval base base_val step_seed combine fuel2 seed = Some v2 ->
    v1 = v2.
Proof.
  exact (nd_eval_deterministic base base_val step_seed combine).
Qed.

Theorem nd_overloaded_monotone :
  forall fuel fuel' seed v,
    nd_eval base base_val step_seed combine fuel seed = Some v ->
    fuel <= fuel' ->
    nd_eval base base_val step_seed combine fuel' seed = Some v.
Proof.
  exact (nd_eval_mono base base_val step_seed combine).
Qed.

End NDOverload.


(* ================================================================ *)
(* D. Optimization Guard: Broadcast over Struct Op                  *)
(*                                                                  *)
(* A broadcast expression whose operator is a struct method call    *)
(* is NOT optimization-eligible (connects to CatnipPurityProof).   *)
(* ================================================================ *)

(* IRBroadcast with a struct op_call as operator *)
Definition broadcast_struct_op (target : IRPure) (method : string)
  (self other : IRPure) : IRPure :=
  IRBroadcast (Some target) (struct_op_call method self other)
              (Some other) BtBinary.

Theorem broadcast_struct_op_inner_not_eligible :
  forall method self other,
    optim_eligible (struct_op_call method self other) = false.
Proof.
  intros. apply struct_op_not_eligible.
Qed.


(* ================================================================ *)
(* E. Concrete Examples                                             *)
(* ================================================================ *)

(* vec2_add applied elementwise to a collection *)
Example broadcast_overloaded_length :
  let op := fun (a b : Val) => Scalar 42 in
  match broadcast_map (overloaded_binop op (Scalar 0))
          (Coll [Scalar 1; Scalar 2; Scalar 3]) with
  | Coll ys => List.length ys = 3
  | _ => False
  end.
Proof. reflexivity. Qed.

(* Empty collection stays empty *)
Example broadcast_overloaded_empty :
  let op := fun (a b : Val) => Scalar 42 in
  broadcast_map (overloaded_binop op (Scalar 0)) empty_topos = empty_topos.
Proof. reflexivity. Qed.

(* ND with overloaded combine: both fuel values produce same result *)
Example nd_overloaded_same_result :
  nd_eval (fun n => Nat.eqb n 0) (fun _ => 1) (fun n => n - 1)
          (fun _ r => r + 1) 3 2 =
  nd_eval (fun n => Nat.eqb n 0) (fun _ => 1) (fun n => n - 1)
          (fun _ r => r + 1) 5 2.
Proof. reflexivity. Qed.
