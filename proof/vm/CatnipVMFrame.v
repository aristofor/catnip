(* FILE: proof/vm/CatnipVMFrame.v *)
(* VM frame model, IP safety, jump safety, block stack, encoding.
 *
 * Source of truth:
 *   catnip_rs/src/vm/frame.rs     (Frame struct, locals, block_stack)
 *   catnip_rs/src/vm/core.rs      (dispatch loop, IP management)
 *   catnip_rs/src/vm/compiler.rs  (ForRangeInt/ForRangeStep encoding)
 *
 * Proves:
 *   - Frame locals invariant (length preservation)
 *   - get/set local roundtrip properties
 *   - IP safety (bounds, fetch, advance, exit)
 *   - Jump target validity
 *   - Block stack push/pop pairing (scope isolation)
 *   - ForRangeInt/ForRangeStep bitpacking roundtrips
 *
 * Depends on: CatnipVMBase.v (VMOpCode, Stack, Locals, Instr)
 *)

From Catnip Require Export CatnipVMBase.
From Coq Require Import List Lia PeanoNat Bool.
Import ListNotations.
Open Scope nat_scope.


(* ================================================================ *)
(* P. Frame Model                                                     *)
(*                                                                    *)
(* Abstract model of a VM frame matching frame.rs:                    *)
(*   - locals: fixed-size vector initialized to nil                   *)
(*   - ip: starts at 0, always within code bounds                     *)
(*   - block_stack: save/restore locals on scope boundaries           *)
(*                                                                    *)
(* Source: catnip_rs/src/vm/frame.rs (Frame struct)                   *)
(* ================================================================ *)

Definition Nil : nat := 0.

Record Frame := mkFrame {
  fr_stack   : Stack;
  fr_locals  : Locals;
  fr_ip      : nat;
  fr_nlocals : nat;       (* code.nlocals: fixed at frame creation *)
  fr_blocks  : list (nat * list nat);  (* block_stack: (slot_start, saved) *)
}.

(* Frame created by Frame::with_code: locals = [Nil; ...; Nil], ip = 0 *)
Definition new_frame (nlocals : nat) : Frame :=
  mkFrame [] (repeat Nil nlocals) 0 nlocals [].

(* Locals length invariant: locals always has nlocals elements *)
Definition locals_wf (f : Frame) : Prop :=
  length (fr_locals f) = fr_nlocals f.

Theorem new_frame_locals_wf : forall n,
  locals_wf (new_frame n).
Proof.
  unfold locals_wf, new_frame. simpl.
  apply repeat_length.
Qed.

(* Safe local access: get_local returns Nil for out-of-bounds *)
Definition get_local (f : Frame) (slot : nat) : nat :=
  nth slot (fr_locals f) Nil.

(* Safe local mutation: set_local is no-op for out-of-bounds *)
Definition set_local (f : Frame) (slot : nat) (v : nat) : Frame :=
  if slot <? fr_nlocals f then
    mkFrame (fr_stack f)
            (firstn slot (fr_locals f) ++ [v] ++ skipn (S slot) (fr_locals f))
            (fr_ip f) (fr_nlocals f) (fr_blocks f)
  else f.

(* set_local preserves locals length *)
Lemma splice_length : forall (l : list nat) n v,
  n < length l ->
  length (firstn n l ++ [v] ++ skipn (S n) l) = length l.
Proof.
  intros l n v Hlt.
  rewrite !length_app.
  rewrite firstn_length_le by lia.
  rewrite length_skipn.
  simpl. lia.
Qed.

Local Opaque firstn skipn app.

Theorem set_local_preserves_wf : forall f slot v,
  locals_wf f ->
  locals_wf (set_local f slot v).
Proof.
  intros [stk locs ip nlocs blks] slot v Hwf.
  unfold set_local, locals_wf in *. simpl in *.
  destruct (slot <? nlocs) eqn:E; [|exact Hwf].
  simpl. apply Nat.ltb_lt in E.
  rewrite splice_length by lia. exact Hwf.
Qed.

Local Transparent firstn skipn app.

(* get after set: same slot returns the written value *)
Theorem get_set_same : forall f slot v,
  locals_wf f ->
  slot < fr_nlocals f ->
  get_local (set_local f slot v) slot = v.
Proof.
  intros f slot v Hwf Hlt.
  unfold get_local, set_local, locals_wf in *.
  destruct (Nat.ltb_spec slot (fr_nlocals f)); [|lia].
  simpl.
  rewrite app_nth2; [|rewrite firstn_length_le by lia; lia].
  rewrite firstn_length_le by lia.
  replace (slot - slot) with 0 by lia.
  reflexivity.
Qed.

(* get after set: different slot is unaffected *)
Theorem get_set_other : forall f slot1 slot2 v,
  locals_wf f ->
  slot1 < fr_nlocals f ->
  slot2 < fr_nlocals f ->
  slot1 <> slot2 ->
  get_local (set_local f slot1 v) slot2 = get_local f slot2.
Proof.
  intros f slot1 slot2 v Hwf Hlt1 Hlt2 Hne.
  unfold get_local, set_local, locals_wf in *.
  destruct (Nat.ltb_spec slot1 (fr_nlocals f)); [|lia].
  cbn [fr_locals].
  destruct (Nat.lt_ge_cases slot2 slot1).
  - rewrite app_nth1; [|rewrite firstn_length_le by lia; lia].
    rewrite nth_firstn.
    replace (_ <? _) with true by (symmetry; apply Nat.ltb_lt; lia).
    reflexivity.
  - rewrite app_nth2; [|rewrite firstn_length_le by lia; lia].
    rewrite firstn_length_le by lia.
    rewrite app_nth2 by (simpl; lia).
    change (length [v]) with 1.
    rewrite nth_skipn. f_equal. lia.
Qed.

(* New frame: all locals are Nil *)
Theorem new_frame_all_nil : forall n slot,
  slot < n -> get_local (new_frame n) slot = Nil.
Proof.
  intros n slot Hlt. unfold get_local, new_frame. simpl.
  apply nth_repeat.
Qed.


(* ================================================================ *)
(* Q. IP Safety                                                       *)
(*                                                                    *)
(* The instruction pointer must stay within code bounds.              *)
(* After fetch, IP is incremented before dispatch (pre-increment).    *)
(*                                                                    *)
(* Source: core.rs dispatch loop                                      *)
(*   - frame.ip starts at 0                                           *)
(*   - each cycle: fetch instr at ip, ip += 1, dispatch               *)
(*   - jumps set ip directly to target                                *)
(*   - exit when ip >= code.instructions.len()                        *)
(* ================================================================ *)

Record CodeObject := mkCode {
  co_instrs  : list Instr;
  co_nlocals : nat;
  co_nargs   : nat;
}.

(* IP is valid if it can fetch the next instruction *)
Definition ip_valid (ip : nat) (code : CodeObject) : Prop :=
  ip < length (co_instrs code).

(* IP is in bounds (includes the "exit" position) *)
Definition ip_in_bounds (ip : nat) (code : CodeObject) : Prop :=
  ip <= length (co_instrs code).

(* New frame starts at ip = 0, which is valid for non-empty code *)
Theorem ip_initial_valid : forall code,
  0 < length (co_instrs code) ->
  ip_valid 0 code.
Proof. unfold ip_valid. lia. Qed.

(* After incrementing IP, it stays in bounds *)
Theorem ip_advance_in_bounds : forall ip code,
  ip_valid ip code ->
  ip_in_bounds (ip + 1) code.
Proof. unfold ip_valid, ip_in_bounds. lia. Qed.

(* Fetching at a valid IP succeeds *)
Theorem ip_fetch_some : forall ip code,
  ip_valid ip code ->
  exists instr, nth_error (co_instrs code) ip = Some instr.
Proof.
  intros ip code H.
  destruct (nth_error (co_instrs code) ip) eqn:E.
  - eexists. reflexivity.
  - apply nth_error_None in E. unfold ip_valid in H. lia.
Qed.

(* Non-valid IP means exit condition *)
Theorem ip_exit : forall ip code,
  ~ ip_valid ip code ->
  ip_in_bounds ip code ->
  ip = length (co_instrs code).
Proof. unfold ip_valid, ip_in_bounds. lia. Qed.


(* ================================================================ *)
(* R. Jump Safety                                                     *)
(*                                                                    *)
(* Jump targets must be valid instruction addresses.                  *)
(* A well-formed code object has all jump targets in bounds.          *)
(*                                                                    *)
(* Source: compiler.rs emit/patch, core.rs Jump/JumpIf* handlers      *)
(* ================================================================ *)

Definition is_jump_op (oc : VMOpCode) : bool :=
  match oc with
  | Jump | JumpIfFalse | JumpIfTrue
  | JumpIfFalseOrPop | JumpIfTrueOrPop
  | JumpIfNone => true
  | _ => false
  end.

Theorem is_jump_op_enumerated : forall oc,
  is_jump_op oc = true <->
  oc = Jump \/ oc = JumpIfFalse \/ oc = JumpIfTrue \/
  oc = JumpIfFalseOrPop \/ oc = JumpIfTrueOrPop \/
  oc = JumpIfNone.
Proof.
  split.
  - destruct oc; simpl; intro H; try discriminate; intuition.
  - intros [H|[H|[H|[H|[H|H]]]]]; subst; reflexivity.
Qed.

(* Well-formed code: all jump targets are in bounds *)
Definition jumps_wf (code : CodeObject) : Prop :=
  forall ip instr,
    nth_error (co_instrs code) ip = Some instr ->
    is_jump_op (instr_op instr) = true ->
    instr_arg instr <= length (co_instrs code).

(* Jump preserves IP in bounds *)
Theorem jump_preserves_bounds : forall code ip instr target,
  jumps_wf code ->
  nth_error (co_instrs code) ip = Some instr ->
  is_jump_op (instr_op instr) = true ->
  target = instr_arg instr ->
  ip_in_bounds target code.
Proof.
  intros code ip instr target Hwf Hnth Hjump Htgt.
  unfold ip_in_bounds. subst target.
  exact (Hwf ip instr Hnth Hjump).
Qed.

(* Non-jump opcodes just advance IP by 1 *)
Theorem non_jump_advances : forall ip code,
  ip_valid ip code ->
  ip_in_bounds (ip + 1) code.
Proof. unfold ip_valid, ip_in_bounds. lia. Qed.

(* All 6 jump opcodes have fixed stack effect *)
Theorem jump_ops_fixed_effect :
  stack_effect Jump = Fixed 0 0 /\
  stack_effect JumpIfFalse = Fixed 1 0 /\
  stack_effect JumpIfTrue = Fixed 1 0 /\
  stack_effect JumpIfFalseOrPop = Fixed 1 0 /\
  stack_effect JumpIfTrueOrPop = Fixed 1 0 /\
  stack_effect JumpIfNone = Fixed 1 0.
Proof. repeat split; reflexivity. Qed.


(* ================================================================ *)
(* S. Block Stack Invariants                                          *)
(*                                                                    *)
(* PushBlock saves locals[slot_start..] onto the block stack.         *)
(* PopBlock restores them. The pairing ensures scope isolation.       *)
(*                                                                    *)
(* Source: frame.rs push_block/pop_block                              *)
(* ================================================================ *)

(* Push block: save locals from slot_start *)
Definition push_block (f : Frame) (slot_start : nat) : Frame :=
  let saved := skipn slot_start (fr_locals f) in
  mkFrame (fr_stack f) (fr_locals f) (fr_ip f) (fr_nlocals f)
          ((slot_start, saved) :: fr_blocks f).

(* Pop block: restore saved locals *)
Definition pop_block (f : Frame) : Frame :=
  match fr_blocks f with
  | [] => f
  | (slot_start, saved) :: rest =>
      let restored := firstn slot_start (fr_locals f) ++
                      saved ++
                      repeat Nil (length (fr_locals f) - slot_start - length saved) in
      mkFrame (fr_stack f) restored (fr_ip f) (fr_nlocals f) rest
  end.

(* Block stack well-formedness: saved region fits in locals *)
Definition blocks_wf (f : Frame) : Prop :=
  forall ss saved,
    In (ss, saved) (fr_blocks f) ->
    ss + length saved <= fr_nlocals f.

(* Push preserves well-formedness *)
Theorem push_block_wf : forall f ss,
  locals_wf f ->
  blocks_wf f ->
  ss <= fr_nlocals f ->
  blocks_wf (push_block f ss).
Proof.
  intros f ss Hlwf Hbwf Hle.
  unfold blocks_wf, push_block. simpl.
  intros ss' saved' [Heq | Hin].
  - inversion Heq. subst ss' saved'.
    rewrite length_skipn.
    unfold locals_wf in Hlwf. lia.
  - exact (Hbwf ss' saved' Hin).
Qed.

(* Push then pop restores the original frame (for the saved region) *)
Theorem push_pop_restores : forall f ss,
  locals_wf f ->
  ss <= fr_nlocals f ->
  forall slot, slot < ss ->
  get_local (pop_block (push_block f ss)) slot = get_local f slot.
Proof.
  intros f ss Hwf Hle slot Hlt.
  unfold pop_block, push_block, get_local.
  cbn [fr_locals fr_blocks fr_stack fr_ip fr_nlocals].
  rewrite app_nth1; [|rewrite firstn_length_le by (unfold locals_wf in Hwf; lia); lia].
  rewrite nth_firstn.
  replace (_ <? _) with true by (symmetry; apply Nat.ltb_lt; lia).
  reflexivity.
Qed.

(* Push then pop: saved region (slot >= slot_start) is restored *)
Theorem push_pop_saved_region : forall f ss,
  locals_wf f ->
  ss <= fr_nlocals f ->
  forall slot, ss <= slot -> slot < fr_nlocals f ->
  get_local (pop_block (push_block f ss)) slot = get_local f slot.
Proof.
  intros f ss Hwf Hle slot Hge Hlt.
  unfold pop_block, push_block, get_local.
  cbn [fr_locals fr_blocks fr_stack fr_ip fr_nlocals].
  rewrite app_nth2; [|rewrite firstn_length_le by (unfold locals_wf in Hwf; lia); lia].
  rewrite firstn_length_le by (unfold locals_wf in Hwf; lia).
  rewrite app_nth1.
  - rewrite nth_skipn. f_equal. lia.
  - rewrite length_skipn. unfold locals_wf in Hwf. lia.
Qed.

(* Block stack depth increases by 1 on push *)
Theorem push_block_depth : forall f ss,
  length (fr_blocks (push_block f ss)) = S (length (fr_blocks f)).
Proof. intros. reflexivity. Qed.

(* Block stack depth decreases by 1 on pop (if non-empty) *)
Theorem pop_block_depth : forall f ss saved rest,
  fr_blocks f = (ss, saved) :: rest ->
  length (fr_blocks (pop_block f)) = length (fr_blocks f) - 1.
Proof.
  intros f ss saved rest Hblk.
  unfold pop_block. rewrite Hblk. simpl. lia.
Qed.


(* ================================================================ *)
(* T. ForRangeInt Encoding                                            *)
(*                                                                    *)
(* Packs 4 fields into a single u32:                                  *)
(*   (slot_i << 24) | (slot_stop << 16) | (step_sign << 15) | off   *)
(*                                                                    *)
(* Constraints: slot_i, slot_stop < 256, offset < 32768.             *)
(*                                                                    *)
(* Source: compiler.rs encode_for_range_args,                         *)
(*         core.rs ForRangeInt handler (decode)                       *)
(* ================================================================ *)

(* Bit operation model using Coq naturals *)
Import Nat.
Local Transparent pow.
Local Lemma pow2_nonzero : forall n, 2 ^ n <> 0.
Proof. intro n. apply Nat.pow_nonzero. lia. Qed.

(* Algebraic power decompositions (avoids computing large Peano nats) *)
Local Fact pow16_double15 : 2 ^ 16 = 2 * 2 ^ 15.
Proof. change 16 with (1 + 15). rewrite Nat.pow_add_r. change (2^1) with 2. lia. Qed.

Local Fact pow24_256x16 : 2 ^ 24 = 256 * 2 ^ 16.
Proof. change 256 with (2^8). rewrite <- Nat.pow_add_r. reflexivity. Qed.

(* Solves power-inequality goals by algebraic decomposition + nia *)
Ltac depower :=
  rewrite ?pow24_256x16; rewrite ?pow16_double15;
  pose proof (pow2_nonzero 15); nia.

(* Encode *)
Definition encode_for_range (slot_i slot_stop : nat) (step_positive : bool)
                            (jump_offset : nat) : nat :=
  let step_bit := if step_positive then 0 else 1 in
  slot_i * (2^24) + slot_stop * (2^16) + step_bit * (2^15) +
  (jump_offset mod (2^15)).

(* Decode *)
Definition decode_slot_i (arg : nat) : nat := arg / (2^24).
Definition decode_slot_stop (arg : nat) : nat := (arg / (2^16)) mod 256.
Definition decode_step_positive (arg : nat) : bool :=
  ((arg / (2^15)) mod 2 =? 0).
Definition decode_jump_offset (arg : nat) : nat := arg mod (2^15).

(* Roundtrip: encode then decode recovers all fields *)
Theorem for_range_slot_i_roundtrip : forall si ss sp off,
  si < 256 -> ss < 256 -> off < 2^15 ->
  decode_slot_i (encode_for_range si ss sp off) = si.
Proof.
  intros si ss sp off Hsi Hss Hoff.
  pose proof pow24_256x16. pose proof pow16_double15.
  pose proof (Nat.mod_upper_bound off (2^15) (pow2_nonzero 15)).
  unfold decode_slot_i, encode_for_range.
  destruct sp; cbn [encode_for_range];
    rewrite <- !Nat.add_assoc;
    rewrite Nat.div_add_l by (apply pow2_nonzero);
    rewrite Nat.div_small; (lia || nia).
Qed.

Theorem for_range_slot_stop_roundtrip : forall si ss sp off,
  si < 256 -> ss < 256 -> off < 2^15 ->
  decode_slot_stop (encode_for_range si ss sp off) = ss.
Proof.
  intros si ss sp off Hsi Hss Hoff.
  pose proof pow24_256x16. pose proof pow16_double15.
  pose proof (Nat.mod_upper_bound off (2^15) (pow2_nonzero 15)).
  unfold decode_slot_stop, encode_for_range.
  destruct sp; cbn [encode_for_range].
  - replace (_ + off mod _)
      with ((si * 256 + ss) * 2 ^ 16 + off mod 2 ^ 15) by nia.
    rewrite Nat.div_add_l by (apply pow2_nonzero).
    assert (off mod 2 ^ 15 < 2 ^ 16) by nia.
    rewrite Nat.div_small by lia. rewrite Nat.add_0_r.
    rewrite Nat.add_comm. rewrite Nat.Div0.mod_add.
    rewrite Nat.mod_small by lia. reflexivity.
  - replace (_ + off mod _)
      with ((si * 256 + ss) * 2 ^ 16 + (2 ^ 15 + off mod 2 ^ 15)) by nia.
    rewrite Nat.div_add_l by (apply pow2_nonzero).
    assert (2 ^ 15 + off mod 2 ^ 15 < 2 ^ 16) by nia.
    rewrite Nat.div_small by lia. rewrite Nat.add_0_r.
    rewrite Nat.add_comm. rewrite Nat.Div0.mod_add.
    rewrite Nat.mod_small by lia. reflexivity.
Qed.

Theorem for_range_step_sign_roundtrip : forall si ss sp off,
  si < 256 -> ss < 256 -> off < 2^15 ->
  decode_step_positive (encode_for_range si ss sp off) = sp.
Proof.
  intros si ss sp off Hsi Hss Hoff.
  pose proof pow24_256x16. pose proof pow16_double15.
  pose proof (Nat.mod_upper_bound off (2^15) (pow2_nonzero 15)) as Hmod.
  unfold decode_step_positive, encode_for_range.
  destruct sp; cbn [encode_for_range].
  - (* step_positive = true: (si*512 + ss*2) is even, mod 2 = 0 *)
    replace (_ + off mod _)
      with ((si * 256 + ss) * 2 * 2 ^ 15 + off mod 2 ^ 15) by nia.
    rewrite Nat.div_add_l by (apply pow2_nonzero).
    rewrite Nat.div_small by lia. rewrite Nat.add_0_r.
    rewrite Nat.Div0.mod_mul. reflexivity.
  - (* step_positive = false: (si*512 + ss*2 + 1) is odd, mod 2 = 1 *)
    replace (_ + off mod _)
      with (((si * 256 + ss) * 2 + 1) * 2 ^ 15 + off mod 2 ^ 15) by nia.
    rewrite Nat.div_add_l by (apply pow2_nonzero).
    rewrite Nat.div_small by lia. rewrite Nat.add_0_r.
    replace ((si * 256 + ss) * 2 + 1) with (1 + (si * 256 + ss) * 2) by lia.
    rewrite Nat.Div0.mod_add. reflexivity.
Qed.

Theorem for_range_offset_roundtrip : forall si ss sp off,
  si < 256 -> ss < 256 -> off < 2^15 ->
  decode_jump_offset (encode_for_range si ss sp off) = off.
Proof.
  intros si ss sp off Hsi Hss Hoff.
  pose proof pow24_256x16. pose proof pow16_double15.
  pose proof (Nat.mod_upper_bound off (2^15) (pow2_nonzero 15)) as Hmod.
  unfold decode_jump_offset, encode_for_range.
  destruct sp; cbn [encode_for_range].
  - replace (_ + off mod _)
      with (off mod 2 ^ 15 + (si * 512 + ss * 2) * 2 ^ 15) by nia.
    rewrite Nat.Div0.mod_add. rewrite !Nat.mod_small by lia. reflexivity.
  - replace (_ + off mod _)
      with (off mod 2 ^ 15 + (si * 512 + ss * 2 + 1) * 2 ^ 15) by nia.
    rewrite Nat.Div0.mod_add. rewrite !Nat.mod_small by lia. reflexivity.
Qed.

(* Combined: all 4 fields survive the roundtrip *)
Theorem for_range_full_roundtrip : forall si ss sp off,
  si < 256 -> ss < 256 -> off < 2^15 ->
  let encoded := encode_for_range si ss sp off in
  decode_slot_i encoded = si /\
  decode_slot_stop encoded = ss /\
  decode_step_positive encoded = sp /\
  decode_jump_offset encoded = off.
Proof.
  intros si ss sp off Hsi Hss Hoff. simpl.
  repeat split.
  - apply for_range_slot_i_roundtrip; assumption.
  - apply for_range_slot_stop_roundtrip; assumption.
  - apply for_range_step_sign_roundtrip; assumption.
  - apply for_range_offset_roundtrip; assumption.
Qed.


(* ================================================================ *)
(* U. ForRangeStep Encoding                                           *)
(*                                                                    *)
(* Packs 3 fields into a single u32:                                  *)
(*   (slot_i << 24) | (step_u8 << 16) | jump_target                 *)
(*                                                                    *)
(* Source: compiler.rs encode_for_range_step,                         *)
(*         core.rs ForRangeStep handler                               *)
(* ================================================================ *)

Definition encode_for_range_step (slot_i : nat) (step_u8 : nat)
                                  (jump_target : nat) : nat :=
  slot_i * (2^24) + step_u8 * (2^16) + (jump_target mod (2^16)).

Definition decode_step_slot_i (arg : nat) : nat := arg / (2^24).
Definition decode_step_u8 (arg : nat) : nat := (arg / (2^16)) mod 256.
Definition decode_step_target (arg : nat) : nat := arg mod (2^16).

Theorem for_range_step_slot_roundtrip : forall si step tgt,
  si < 256 -> step < 256 -> tgt < 2^16 ->
  decode_step_slot_i (encode_for_range_step si step tgt) = si.
Proof.
  intros si step tgt Hsi Hstep Htgt.
  unfold decode_step_slot_i, encode_for_range_step.
  rewrite <- !Nat.add_assoc.
  rewrite Nat.div_add_l by (apply pow2_nonzero).
  assert (step * 2 ^ 16 + tgt mod 2 ^ 16 < 2 ^ 24) by
    (pose proof (Nat.mod_upper_bound tgt (2^16) (pow2_nonzero 16));
     rewrite pow24_256x16; pose proof (pow2_nonzero 16); nia).
  rewrite Nat.div_small by lia. lia.
Qed.

Theorem for_range_step_step_roundtrip : forall si step tgt,
  si < 256 -> step < 256 -> tgt < 2^16 ->
  decode_step_u8 (encode_for_range_step si step tgt) = step.
Proof.
  intros si step tgt Hsi Hstep Htgt.
  pose proof pow24_256x16.
  unfold decode_step_u8, encode_for_range_step.
  replace (si * 2 ^ 24 + step * 2 ^ 16 + tgt mod 2 ^ 16)
    with ((si * 256 + step) * 2 ^ 16 + tgt mod 2 ^ 16) by nia.
  rewrite Nat.div_add_l by (apply pow2_nonzero).
  assert (tgt mod 2 ^ 16 < 2 ^ 16) by
    (apply Nat.mod_upper_bound; apply pow2_nonzero).
  rewrite Nat.div_small by lia.
  rewrite Nat.add_0_r.
  rewrite Nat.add_comm. rewrite Nat.Div0.mod_add.
  rewrite Nat.mod_small by lia. reflexivity.
Qed.

Theorem for_range_step_target_roundtrip : forall si step tgt,
  si < 256 -> step < 256 -> tgt < 2^16 ->
  decode_step_target (encode_for_range_step si step tgt) = tgt.
Proof.
  intros si step tgt Hsi Hstep Htgt.
  pose proof pow24_256x16.
  pose proof (Nat.mod_upper_bound tgt (2^16) (pow2_nonzero 16)).
  unfold decode_step_target, encode_for_range_step.
  replace (si * 2 ^ 24 + step * 2 ^ 16 + tgt mod 2 ^ 16)
    with ((si * 256 + step) * 2 ^ 16 + tgt mod 2 ^ 16) by nia.
  rewrite Nat.add_comm. rewrite Nat.Div0.mod_add.
  rewrite !Nat.mod_small by lia. reflexivity.
Qed.
