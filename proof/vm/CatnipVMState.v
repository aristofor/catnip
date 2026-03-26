(* FILE: proof/vm/CatnipVMState.v *)
(* Stack effect model, VM state, fixed-effect classification.
 *
 * Defines:
 *   - StackEffect (Fixed | ArgDependent)
 *   - stack_effect classifier for all 83 opcodes
 *   - VMState record (stack, locals, IP)
 *   - Fixed-effect predicates and count lemma
 *
 * Depends on: CatnipVMOpCode.v
 *)

From Coq Require Import List Bool.
Import ListNotations.

From Catnip Require Export CatnipVMOpCode.


(* ================================================================ *)
(* C. Stack Effect Model                                              *)
(*                                                                    *)
(* Each opcode has a stack effect: how many values it pops and        *)
(* pushes. Fixed-effect opcodes have known (pops, pushes).            *)
(* Arg-dependent opcodes have effect parameterized by arg.            *)
(*                                                                    *)
(* Source: VMOpCode::stack_effect() in opcode.rs                      *)
(* ================================================================ *)

(* Stack effect classification *)
Inductive StackEffect :=
  | Fixed (pops pushes : nat)    (* known statically *)
  | ArgDependent.                (* depends on instruction arg *)

Definition stack_effect (oc : VMOpCode) : StackEffect :=
  match oc with
  (* Data movement *)
  | LoadConst      => Fixed 0 1
  | LoadLocal      => Fixed 0 1
  | StoreLocal     => Fixed 1 0
  | LoadScope      => Fixed 0 1
  | StoreScope     => Fixed 1 0
  | LoadGlobal     => Fixed 0 1
  | PopTop         => Fixed 1 0
  | DupTop         => Fixed 1 2
  | RotTwo         => Fixed 2 2
  (* Arithmetic: binary => pop 2 push 1 *)
  | VAdd | VSub | VMul | VDiv | VFloorDiv | VMod | VPow => Fixed 2 1
  (* Arithmetic: unary => pop 1 push 1 *)
  | VNeg | VPos    => Fixed 1 1
  (* Bitwise: binary *)
  | VBOr | VBXor | VBAnd | VLShift | VRShift => Fixed 2 1
  (* Bitwise: unary *)
  | VBNot          => Fixed 1 1
  (* Comparison: binary *)
  | VLt | VLe | VGt | VGe | VEq | VNe => Fixed 2 1
  (* Logic: unary *)
  | VNot           => Fixed 1 1
  (* Jumps *)
  | Jump           => Fixed 0 0
  | JumpIfFalse    => Fixed 1 0
  | JumpIfTrue     => Fixed 1 0
  | JumpIfFalseOrPop => Fixed 1 0
  | JumpIfTrueOrPop  => Fixed 1 0
  (* Iteration *)
  | GetIter        => Fixed 1 1
  | ForIter        => Fixed 0 1
  | ForRangeInt    => Fixed 0 0
  | ForRangeStep   => Fixed 0 0
  (* Function: arg-dependent *)
  | VCall          => ArgDependent
  | CallKw         => ArgDependent
  | TailCall       => ArgDependent
  | CallMethod     => ArgDependent
  (* Function: fixed *)
  | VReturn        => Fixed 1 0
  | MakeFunction   => Fixed 1 1
  (* Collections: arg-dependent *)
  | BuildList      => ArgDependent
  | BuildTuple     => ArgDependent
  | BuildSet       => ArgDependent
  | BuildDict      => ArgDependent
  | BuildSlice     => ArgDependent
  (* Access *)
  | VGetAttr       => Fixed 1 1
  | VSetAttr       => Fixed 2 0
  | VGetItem       => Fixed 2 1
  | VSetItem       => Fixed 3 0
  (* Blocks *)
  | PushBlock       => Fixed 0 0
  | PopBlock        => Fixed 0 0
  (* Control *)
  | VBreak          => Fixed 0 0
  | VContinue       => Fixed 0 0
  (* Broadcasting: arg-dependent *)
  | VBroadcast      => ArgDependent
  (* Pattern *)
  | MatchPattern    => Fixed 1 1
  | BindMatch       => Fixed 1 0
  | JumpIfNone      => Fixed 1 0
  | UnpackSequence  => ArgDependent
  | UnpackEx        => ArgDependent
  | MatchPatternVM  => Fixed 1 1
  (* Misc *)
  | Nop            => Fixed 0 0
  | Halt           => Fixed 0 0
  | VBreakpoint    => Fixed 0 0
  (* ND *)
  | NdEmptyTopos   => Fixed 0 1
  | NdRecursion    => ArgDependent
  | NdMap          => ArgDependent
  (* Struct *)
  | MakeStruct     => Fixed 0 0
  | MakeTrait      => Fixed 0 0
  (* Membership & Identity: binary => pop 2 push 1 *)
  | VIn | VNotIn | VIs | VIsNot => Fixed 2 1
  (* Conversion: unary => pop 1 push 1 *)
  | VToBool        => Fixed 1 1
  (* Extended jumps *)
  | JumpIfNotNoneOrPop => Fixed 1 0
  (* Extended match *)
  | MatchAssignPatternVM => Fixed 1 1
  | VMatchFail     => Fixed 0 0
  (* Exit: arg-dependent (arg=0: pops 0, arg=1: pops 1) *)
  | VExit          => ArgDependent
  end.


(* ================================================================ *)
(* D. VM State Model                                                  *)
(*                                                                    *)
(* Minimal abstract model of a VM frame:                              *)
(*   - operand stack (list of abstract values)                        *)
(*   - locals (vector indexed by slot)                                *)
(*   - instruction pointer                                            *)
(*                                                                    *)
(* Values abstracted as nat (tag discrimination proven in             *)
(* CatnipNanBoxProof.v; here we only care about stack structure).    *)
(* ================================================================ *)

Definition Stack := list nat.
Definition Locals := list nat.

Record VMState := mkVMState {
  vm_stack  : Stack;
  vm_locals : Locals;
  vm_ip     : nat;
}.

Definition stack_depth (s : VMState) : nat :=
  length (vm_stack s).


(* ================================================================ *)
(* E. Fixed-Effect Classification                                     *)
(*                                                                    *)
(* Predicate: opcode has a statically known stack effect.             *)
(* ================================================================ *)

Definition is_fixed_effect (oc : VMOpCode) : bool :=
  match stack_effect oc with
  | Fixed _ _ => true
  | ArgDependent => false
  end.

Definition get_pops (oc : VMOpCode) : option nat :=
  match stack_effect oc with
  | Fixed p _ => Some p
  | ArgDependent => None
  end.

Definition get_pushes (oc : VMOpCode) : option nat :=
  match stack_effect oc with
  | Fixed _ p => Some p
  | ArgDependent => None
  end.

(* 68 opcodes have fixed effects, 15 are arg-dependent *)
Lemma fixed_effect_count :
  length (filter (fun oc => is_fixed_effect oc)
    [LoadConst; LoadLocal; StoreLocal; LoadScope; StoreScope;
     LoadGlobal; PopTop; DupTop; RotTwo;
     VAdd; VSub; VMul; VDiv; VFloorDiv; VMod; VPow; VNeg; VPos;
     VBOr; VBXor; VBAnd; VBNot; VLShift; VRShift;
     VLt; VLe; VGt; VGe; VEq; VNe;
     VNot;
     Jump; JumpIfFalse; JumpIfTrue; JumpIfFalseOrPop; JumpIfTrueOrPop;
     GetIter; ForIter; ForRangeInt;
     VReturn; MakeFunction;
     VGetAttr; VSetAttr; VGetItem; VSetItem;
     PushBlock; PopBlock; VBreak; VContinue;
     MatchPattern; BindMatch; JumpIfNone; MatchPatternVM;
     Nop; Halt; VBreakpoint;
     NdEmptyTopos; ForRangeStep;
     MakeStruct; MakeTrait;
     VIn; VNotIn; VIs; VIsNot;
     VToBool; JumpIfNotNoneOrPop;
     MatchAssignPatternVM; VMatchFail]) = 68.
Proof. reflexivity. Qed.
