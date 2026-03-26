(* FILE: proof/vm/CatnipVMExamples.v *)
(* Concrete execution examples and classification completeness.
 *
 * Proves:
 *   - 10 concrete instruction sequence examples (reflexivity)
 *   - effect_total: every opcode is Fixed or ArgDependent
 *   - arg_dependent_opcodes: exhaustive enumeration of the 15
 *
 * Depends on: CatnipVMStackSafety.v
 *)

From Coq Require Import List.
Import ListNotations.

From Catnip Require Export CatnipVMStackSafety.


(* ================================================================ *)
(* N. Concrete Examples                                               *)
(* ================================================================ *)

(* Example: 2 + 3
   LoadConst, LoadConst, Add -> stack depth 1 *)
Example ex_add_2_3 :
  exec_seq [mkInstr LoadConst 0; mkInstr LoadConst 1; mkInstr VAdd 0] [] =
  Some [0].
Proof. reflexivity. Qed.

(* Example: x = 42
   LoadConst, StoreLocal -> stack depth 0 *)
Example ex_assign :
  exec_seq [mkInstr LoadConst 0; mkInstr StoreLocal 0] [] =
  Some [].
Proof. reflexivity. Qed.

(* Example: -x
   LoadLocal, Neg -> stack depth 1 *)
Example ex_neg :
  exec_seq [mkInstr LoadLocal 0; mkInstr VNeg 0] [] =
  Some [0].
Proof. reflexivity. Qed.

(* Example: nested expression (2 + 3) * 4
   LoadConst, LoadConst, Add, LoadConst, Mul -> stack depth 1 *)
Example ex_nested_expr :
  exec_seq [mkInstr LoadConst 0; mkInstr LoadConst 1; mkInstr VAdd 0;
            mkInstr LoadConst 2; mkInstr VMul 0] [] =
  Some [0].
Proof. reflexivity. Qed.

(* Example: empty sequence is identity *)
Example ex_empty_seq :
  forall stk, exec_seq [] stk = Some stk.
Proof. reflexivity. Qed.

(* Example: Nop doesn't change stack depth *)
Example ex_nop :
  forall stk, exec_seq [mkInstr Nop 0] stk = Some stk.
Proof. reflexivity. Qed.

(* Example: x in [1,2,3]
   LoadLocal, LoadLocal, In -> stack depth 1 *)
Example ex_membership :
  exec_seq [mkInstr LoadLocal 0; mkInstr LoadLocal 1; mkInstr VIn 0] [] =
  Some [0].
Proof. reflexivity. Qed.

(* Example: bool(x)
   LoadLocal, ToBool -> stack depth 1 *)
Example ex_to_bool :
  exec_seq [mkInstr LoadLocal 0; mkInstr VToBool 0] [] =
  Some [0].
Proof. reflexivity. Qed.

(* Example: MatchFail is a no-op signal *)
Example ex_match_fail :
  forall stk, exec_seq [mkInstr VMatchFail 0] stk = Some stk.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* O. Classification Completeness                                     *)
(*                                                                    *)
(* Every VM opcode is either fixed-effect or arg-dependent.           *)
(* This is trivially true by construction of StackEffect,             *)
(* but we state it explicitly for documentation.                      *)
(* ================================================================ *)

Theorem effect_total : forall oc,
  (exists p q, stack_effect oc = Fixed p q) \/
  stack_effect oc = ArgDependent.
Proof.
  destruct oc; simpl;
    try (left; eexists; eexists; reflexivity);
    right; reflexivity.
Qed.

(* Arg-dependent opcodes enumerated *)
Theorem arg_dependent_opcodes : forall oc,
  stack_effect oc = ArgDependent <->
  oc = VCall \/ oc = CallKw \/ oc = TailCall \/ oc = CallMethod \/
  oc = BuildList \/ oc = BuildTuple \/ oc = BuildSet \/
  oc = BuildDict \/ oc = BuildSlice \/
  oc = VBroadcast \/
  oc = UnpackSequence \/ oc = UnpackEx \/
  oc = NdRecursion \/ oc = NdMap \/
  oc = VExit.
Proof.
  split.
  - destruct oc; simpl; intro H; try discriminate;
    intuition.
  - intros [H|[H|[H|[H|[H|[H|[H|[H|[H|[H|[H|[H|[H|[H|H]]]]]]]]]]]]]];
    subst; reflexivity.
Qed.
