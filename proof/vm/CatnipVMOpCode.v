(* FILE: proof/vm/CatnipVMOpCode.v *)
(* VM opcode enumeration and numbering bijection.
 *
 * Source of truth:
 *   catnip_core/src/vm/opcode.rs  (VMOpCode, 83 opcodes, repr(u8) 1..83)
 *
 * Proves:
 *   - Opcode decidable equality
 *   - Numbering bijectivity (VMOpCode <-> u8)
 *   - Range [1..83]
 *)

From Coq Require Import Lia PeanoNat.


(* ================================================================ *)
(* A. VM OpCodes                                                      *)
(*                                                                    *)
(* 83 opcodes, ordered by repr(u8) value (1..83).                    *)
(* Source: catnip_core/src/vm/opcode.rs                              *)
(* ================================================================ *)

Inductive VMOpCode :=
  (* Data movement (1-9) *)
  | LoadConst | LoadLocal | StoreLocal | LoadScope | StoreScope
  | LoadGlobal | PopTop | DupTop | RotTwo
  (* Arithmetic (10-18) *)
  | VAdd | VSub | VMul | VDiv | VFloorDiv | VMod | VPow | VNeg | VPos
  (* Bitwise (19-24) *)
  | VBOr | VBXor | VBAnd | VBNot | VLShift | VRShift
  (* Comparison (25-30) *)
  | VLt | VLe | VGt | VGe | VEq | VNe
  (* Logic (31) *)
  | VNot
  (* Jumps (32-36) *)
  | Jump | JumpIfFalse | JumpIfTrue | JumpIfFalseOrPop | JumpIfTrueOrPop
  (* Iteration (37-39) *)
  | GetIter | ForIter | ForRangeInt
  (* Function (40-44) *)
  | VCall | CallKw | TailCall | VReturn | MakeFunction
  (* Collections (45-49) *)
  | BuildList | BuildTuple | BuildSet | BuildDict | BuildSlice
  (* Access (50-53) *)
  | VGetAttr | VSetAttr | VGetItem | VSetItem
  (* Blocks (54-55) *)
  | PushBlock | PopBlock
  (* Control (56-57) *)
  | VBreak | VContinue
  (* Broadcasting (58) *)
  | VBroadcast
  (* Pattern (59-63) *)
  | MatchPattern | BindMatch | JumpIfNone | UnpackSequence | UnpackEx
  (* Misc (64-65) *)
  | Nop | Halt
  (* ND (66-68) *)
  | NdEmptyTopos | NdRecursion | NdMap
  (* Extended (69-74) *)
  | ForRangeStep | MatchPatternVM | VBreakpoint | MakeStruct | MakeTrait
  | CallMethod
  (* Membership & Identity (75-78) *)
  | VIn | VNotIn | VIs | VIsNot
  (* Conversion (79) *)
  | VToBool
  (* Extended jumps (80) *)
  | JumpIfNotNoneOrPop
  (* Extended match (81-82) *)
  | MatchAssignPatternVM | VMatchFail
  (* Control (83) *)
  | VExit.

Lemma VMOpCode_eq_dec : forall (a b : VMOpCode), {a = b} + {a <> b}.
Proof. decide equality. Defined.


(* ================================================================ *)
(* B. Opcode Numbering                                                *)
(*                                                                    *)
(* Bijection VMOpCode <-> nat (matching repr(u8) in Rust).            *)
(* ================================================================ *)

Definition vm_opcode_to_nat (oc : VMOpCode) : nat :=
  match oc with
  | LoadConst => 1 | LoadLocal => 2 | StoreLocal => 3
  | LoadScope => 4 | StoreScope => 5 | LoadGlobal => 6
  | PopTop => 7 | DupTop => 8 | RotTwo => 9
  | VAdd => 10 | VSub => 11 | VMul => 12 | VDiv => 13
  | VFloorDiv => 14 | VMod => 15 | VPow => 16
  | VNeg => 17 | VPos => 18
  | VBOr => 19 | VBXor => 20 | VBAnd => 21
  | VBNot => 22 | VLShift => 23 | VRShift => 24
  | VLt => 25 | VLe => 26 | VGt => 27 | VGe => 28
  | VEq => 29 | VNe => 30
  | VNot => 31
  | Jump => 32 | JumpIfFalse => 33 | JumpIfTrue => 34
  | JumpIfFalseOrPop => 35 | JumpIfTrueOrPop => 36
  | GetIter => 37 | ForIter => 38 | ForRangeInt => 39
  | VCall => 40 | CallKw => 41 | TailCall => 42
  | VReturn => 43 | MakeFunction => 44
  | BuildList => 45 | BuildTuple => 46
  | BuildSet => 47 | BuildDict => 48 | BuildSlice => 49
  | VGetAttr => 50 | VSetAttr => 51
  | VGetItem => 52 | VSetItem => 53
  | PushBlock => 54 | PopBlock => 55
  | VBreak => 56 | VContinue => 57
  | VBroadcast => 58
  | MatchPattern => 59 | BindMatch => 60 | JumpIfNone => 61
  | UnpackSequence => 62 | UnpackEx => 63
  | Nop => 64 | Halt => 65
  | NdEmptyTopos => 66 | NdRecursion => 67 | NdMap => 68
  | ForRangeStep => 69 | MatchPatternVM => 70
  | VBreakpoint => 71 | MakeStruct => 72 | MakeTrait => 73
  | CallMethod => 74
  | VIn => 75 | VNotIn => 76 | VIs => 77 | VIsNot => 78
  | VToBool => 79
  | JumpIfNotNoneOrPop => 80
  | MatchAssignPatternVM => 81 | VMatchFail => 82
  | VExit => 83
  end.

Definition nat_to_vm_opcode (n : nat) : option VMOpCode :=
  match n with
  | 1 => Some LoadConst | 2 => Some LoadLocal | 3 => Some StoreLocal
  | 4 => Some LoadScope | 5 => Some StoreScope | 6 => Some LoadGlobal
  | 7 => Some PopTop | 8 => Some DupTop | 9 => Some RotTwo
  | 10 => Some VAdd | 11 => Some VSub | 12 => Some VMul | 13 => Some VDiv
  | 14 => Some VFloorDiv | 15 => Some VMod | 16 => Some VPow
  | 17 => Some VNeg | 18 => Some VPos
  | 19 => Some VBOr | 20 => Some VBXor | 21 => Some VBAnd
  | 22 => Some VBNot | 23 => Some VLShift | 24 => Some VRShift
  | 25 => Some VLt | 26 => Some VLe | 27 => Some VGt | 28 => Some VGe
  | 29 => Some VEq | 30 => Some VNe
  | 31 => Some VNot
  | 32 => Some Jump | 33 => Some JumpIfFalse | 34 => Some JumpIfTrue
  | 35 => Some JumpIfFalseOrPop | 36 => Some JumpIfTrueOrPop
  | 37 => Some GetIter | 38 => Some ForIter | 39 => Some ForRangeInt
  | 40 => Some VCall | 41 => Some CallKw | 42 => Some TailCall
  | 43 => Some VReturn | 44 => Some MakeFunction
  | 45 => Some BuildList | 46 => Some BuildTuple
  | 47 => Some BuildSet | 48 => Some BuildDict | 49 => Some BuildSlice
  | 50 => Some VGetAttr | 51 => Some VSetAttr
  | 52 => Some VGetItem | 53 => Some VSetItem
  | 54 => Some PushBlock | 55 => Some PopBlock
  | 56 => Some VBreak | 57 => Some VContinue
  | 58 => Some VBroadcast
  | 59 => Some MatchPattern | 60 => Some BindMatch | 61 => Some JumpIfNone
  | 62 => Some UnpackSequence | 63 => Some UnpackEx
  | 64 => Some Nop | 65 => Some Halt
  | 66 => Some NdEmptyTopos | 67 => Some NdRecursion | 68 => Some NdMap
  | 69 => Some ForRangeStep | 70 => Some MatchPatternVM
  | 71 => Some VBreakpoint | 72 => Some MakeStruct | 73 => Some MakeTrait
  | 74 => Some CallMethod
  | 75 => Some VIn | 76 => Some VNotIn | 77 => Some VIs | 78 => Some VIsNot
  | 79 => Some VToBool
  | 80 => Some JumpIfNotNoneOrPop
  | 81 => Some MatchAssignPatternVM | 82 => Some VMatchFail
  | 83 => Some VExit
  | _ => None
  end.

Theorem vm_opcode_to_nat_injective : forall a b,
  vm_opcode_to_nat a = vm_opcode_to_nat b -> a = b.
Proof.
  destruct a; destruct b; simpl; intro H;
    try reflexivity; discriminate.
Qed.

Theorem vm_opcode_to_nat_range : forall oc,
  (1 <= vm_opcode_to_nat oc <= 83).
Proof. destruct oc; simpl; lia. Qed.

Theorem vm_opcode_roundtrip : forall oc,
  nat_to_vm_opcode (vm_opcode_to_nat oc) = Some oc.
Proof. destruct oc; reflexivity. Qed.

Theorem nat_to_vm_opcode_roundtrip : forall n oc,
  nat_to_vm_opcode n = Some oc -> vm_opcode_to_nat oc = n.
Proof.
  intros n oc H.
  destruct n as [|n']; simpl in H; [discriminate|].
  do 83 (destruct n' as [|n']; [inversion H; reflexivity|]).
  simpl in H. discriminate.
Qed.
