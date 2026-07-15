(* FILE: proof/vm/CatnipNanBoxProof.v *)
(* CatnipNanBoxProof.v - NaN-boxing value representation correctness
 *
 * Source of truth:
 *   catnip_rs/src/vm/value.rs (PyO3 tags 4-7)
 *   catnip_core/src/nanbox.rs + catnip_core/src/scalar.rs (shared
 *   layout, ScalarValue accessors and constructors -- Phase 5)
 *
 * Proves tag discrimination, encoding injectivity, and round-trip
 * correctness for the VM's NaN-boxed value representation:
 *
 *   [Sign:1][Exponent:11=0x7FF][Quiet:1][Tag:4][Payload:47]
 *     63        62-52              51     50-47    46-0
 *
 * 8 tags: SmallInt(0), Bool(1), Nil(2), Symbol(3),
 *         PyObj(4), Struct(5), BigInt(6), VMFunc(7).
 *
 * Complete proofs for all 8 tags (0-7), including BigInt
 * promotion/demotion invariants (SmallInt <-> BigInt) and the
 * Phase 5 promotion lemmas: to_bigint totality/exactness and
 * fast-path/slow-path coherence of the generic int arithmetic
 * (catnip_core::arith).
 *
 * Models bitwise encoding as arithmetic (tag * 2^47 + payload)
 * since Z arithmetic is cleaner for Coq proofs than Z.land/Z.lor
 * on 64-bit constants.
 *
 * Standalone: no dependencies on other Catnip proofs.
 *
 * 37 theorems + 24 examples, 0 Admitted.
 *)

From Coq Require Import ZArith Bool Lia.
Open Scope Z_scope.


(* ================================================================ *)
(* A. Constants                                                      *)
(*                                                                    *)
(* QNAN_BASE = 0x7FF8 * 2^47 (bits 63-52 + quiet NaN bit).         *)
(* Tag occupies bits 50-47 = multiplier of 2^47.                     *)
(* Payload occupies bits 46-0.                                       *)
(* ================================================================ *)

Definition W := 2 ^ 47.     (* payload width *)
Definition QNAN := 65520.   (* 0x7FF8 * 2: shifted for 4-bit tags *)
Definition NTAGS := 8.

Definition tag_id_SmallInt := 0.
Definition tag_id_Bool     := 1.
Definition tag_id_Nil      := 2.
Definition tag_id_Symbol   := 3.
Definition tag_id_PyObj    := 4.
Definition tag_id_Struct   := 5.
Definition tag_id_BigInt   := 6.
Definition tag_id_VMFunc   := 7.


(* ================================================================ *)
(* B. Encoding / Decoding                                            *)
(*                                                                    *)
(* value = (QNAN + tag_id) * W + payload                            *)
(* ================================================================ *)

Definition valid_tag (t : Z) : Prop := 0 <= t < NTAGS.
Definition valid_payload (p : Z) : Prop := 0 <= p < W.

Definition encode (tag payload : Z) : Z :=
  (QNAN + tag) * W + payload.

Definition extract_tag (v : Z) : Z :=
  (v / W) - QNAN.

Definition extract_payload (v : Z) : Z :=
  v mod W.


(* ================================================================ *)
(* C. Tag Extraction                                                  *)
(* ================================================================ *)

Lemma W_pos : 0 < W.
Proof. unfold W. lia. Qed.

Theorem extract_tag_correct : forall tag payload,
  valid_tag tag -> valid_payload payload ->
  extract_tag (encode tag payload) = tag.
Proof.
  intros tag payload [Htlo Hthi] [Hplo Hphi].
  unfold extract_tag, encode.
  rewrite Z.div_add_l by (pose proof W_pos; lia).
  rewrite Z.div_small by lia.
  lia.
Qed.

Theorem extract_payload_correct : forall tag payload,
  valid_tag tag -> valid_payload payload ->
  extract_payload (encode tag payload) = payload.
Proof.
  intros tag payload [Htlo Hthi] [Hplo Hphi].
  unfold extract_payload, encode.
  rewrite Z.add_comm.
  rewrite Z.mod_add by (pose proof W_pos; lia).
  apply Z.mod_small. lia.
Qed.


(* ================================================================ *)
(* D. Encoding Injectivity                                           *)
(* ================================================================ *)

Theorem encode_injective : forall t1 t2 p1 p2,
  valid_tag t1 -> valid_tag t2 ->
  valid_payload p1 -> valid_payload p2 ->
  encode t1 p1 = encode t2 p2 ->
  t1 = t2 /\ p1 = p2.
Proof.
  intros t1 t2 p1 p2 Ht1 Ht2 Hp1 Hp2 Henc.
  assert (Htag : extract_tag (encode t1 p1) = extract_tag (encode t2 p2))
    by (rewrite Henc; reflexivity).
  rewrite extract_tag_correct in Htag by assumption.
  rewrite extract_tag_correct in Htag by assumption.
  assert (Hpay : extract_payload (encode t1 p1) = extract_payload (encode t2 p2))
    by (rewrite Henc; reflexivity).
  rewrite extract_payload_correct in Hpay by assumption.
  rewrite extract_payload_correct in Hpay by assumption.
  split; assumption.
Qed.


(* ================================================================ *)
(* E. Tag Discrimination                                              *)
(*                                                                    *)
(* Values with different tags are always different.                   *)
(* ================================================================ *)

Theorem different_tags_different_values : forall t1 t2 p1 p2,
  valid_tag t1 -> valid_tag t2 ->
  valid_payload p1 -> valid_payload p2 ->
  t1 <> t2 ->
  encode t1 p1 <> encode t2 p2.
Proof.
  intros t1 t2 p1 p2 Ht1 Ht2 Hp1 Hp2 Hne Habs.
  apply encode_injective in Habs; try assumption.
  destruct Habs as [Heq _]. contradiction.
Qed.


(* ================================================================ *)
(* F. Bool Round-Trip                                                *)
(* ================================================================ *)

Definition encode_bool (b : bool) : Z :=
  encode tag_id_Bool (if b then 1 else 0).

Definition decode_bool (v : Z) : bool :=
  negb (Z.eqb (extract_payload v) 0).

Theorem bool_roundtrip : forall b,
  decode_bool (encode_bool b) = b.
Proof.
  intro b.
  unfold decode_bool, encode_bool.
  rewrite extract_payload_correct.
  - destruct b; reflexivity.
  - unfold valid_tag, tag_id_Bool, NTAGS. lia.
  - unfold valid_payload, W. destruct b; lia.
Qed.


(* ================================================================ *)
(* G. SmallInt Round-Trip (non-negative)                             *)
(*                                                                    *)
(* Non-negative integers in [0, 2^46) round-trip directly.           *)
(* Negative integers require sign extension (modeled separately).     *)
(* ================================================================ *)

Definition encode_int (i : Z) : Z :=
  encode tag_id_SmallInt (i mod W).

Theorem int_roundtrip_nonneg : forall i,
  0 <= i < W ->
  extract_payload (encode_int i) = i.
Proof.
  intros i [Hlo Hhi].
  unfold encode_int.
  rewrite extract_payload_correct.
  - apply Z.mod_small. lia.
  - unfold valid_tag, tag_id_SmallInt, NTAGS. lia.
  - unfold valid_payload. split.
    + apply Z.mod_pos_bound. pose proof W_pos. lia.
    + apply Z.mod_pos_bound. pose proof W_pos. lia.
Qed.

(* SmallInt range: -2^46 to 2^46-1 *)
Definition HALF_W := 2 ^ 46.

Definition sign_extend (p : Z) : Z :=
  if p <? HALF_W then p else p - W.

Theorem int_roundtrip_signed : forall i,
  -HALF_W <= i < HALF_W ->
  sign_extend (extract_payload (encode_int i)) = i.
Proof.
  intros i [Hlo Hhi].
  unfold encode_int.
  rewrite extract_payload_correct.
  2: { unfold valid_tag, tag_id_SmallInt, NTAGS. lia. }
  2: { unfold valid_payload. split;
       apply Z.mod_pos_bound; pose proof W_pos; lia. }
  unfold sign_extend, HALF_W, W in *.
  destruct (Z.ltb_spec (i mod 2^47) (2^46)).
  - (* i >= 0: mod is identity *)
    assert (Hi0 : 0 <= i).
    { destruct (Z.lt_ge_cases i 0) as [Hn|]; [|lia].
      exfalso.
      assert (Hdiv : i / 2^47 = -1)
        by (symmetry; apply (Z.div_unique i (2^47) (-1) (i + 2^47)); lia).
      pose proof (Z.div_mod i (2^47) ltac:(lia)) as Heq.
      rewrite Hdiv in Heq. lia. }
    rewrite Z.mod_small by lia. lia.
  - (* i < 0: mod = i + 2^47 *)
    assert (Hi_neg : i < 0).
    { destruct (Z.lt_ge_cases i 0); [lia|].
      assert (i mod 2^47 = i) by (apply Z.mod_small; lia).
      lia. }
    assert (Hdiv : i / 2^47 = -1)
      by (symmetry; apply (Z.div_unique i (2^47) (-1) (i + 2^47)); lia).
    pose proof (Z.div_mod i (2^47) ltac:(lia)) as Hmod.
    rewrite Hdiv in Hmod. lia.
Qed.


(* ================================================================ *)
(* H. Encoding Positivity and Bounds                                 *)
(* ================================================================ *)

Theorem encode_positive : forall tag payload,
  valid_tag tag -> valid_payload payload ->
  0 < encode tag payload.
Proof.
  intros tag payload [Htlo Hthi] [Hplo Hphi].
  unfold encode, QNAN, W, NTAGS in *. lia.
Qed.


(* ================================================================ *)
(* I. Nil / Symbol / PyObj / Struct Round-Trips                      *)
(*                                                                    *)
(* These types carry an unsigned payload (pointer or ID) that        *)
(* round-trips via extract_payload directly.                          *)
(* ================================================================ *)

Definition encode_nil : Z :=
  encode tag_id_Nil 0.

Definition encode_symbol (id : Z) : Z :=
  encode tag_id_Symbol id.

Definition encode_pyobj (ptr : Z) : Z :=
  encode tag_id_PyObj ptr.

Definition encode_struct (ptr : Z) : Z :=
  encode tag_id_Struct ptr.

(* Generic round-trip for any tag with unsigned payload *)
Lemma roundtrip_tag_payload : forall tag payload,
  valid_tag tag -> valid_payload payload ->
  extract_tag (encode tag payload) = tag /\
  extract_payload (encode tag payload) = payload.
Proof.
  intros tag payload Ht Hp. split.
  - exact (extract_tag_correct tag payload Ht Hp).
  - exact (extract_payload_correct tag payload Ht Hp).
Qed.

Theorem nil_roundtrip :
  extract_tag encode_nil = tag_id_Nil /\
  extract_payload encode_nil = 0.
Proof.
  apply roundtrip_tag_payload.
  - unfold valid_tag, tag_id_Nil, NTAGS. lia.
  - unfold valid_payload, W. lia.
Qed.

Theorem symbol_roundtrip : forall id,
  valid_payload id ->
  extract_tag (encode_symbol id) = tag_id_Symbol /\
  extract_payload (encode_symbol id) = id.
Proof.
  intros id Hp. apply roundtrip_tag_payload.
  - unfold valid_tag, tag_id_Symbol, NTAGS. lia.
  - exact Hp.
Qed.

Theorem pyobj_roundtrip : forall ptr,
  valid_payload ptr ->
  extract_tag (encode_pyobj ptr) = tag_id_PyObj /\
  extract_payload (encode_pyobj ptr) = ptr.
Proof.
  intros ptr Hp. apply roundtrip_tag_payload.
  - unfold valid_tag, tag_id_PyObj, NTAGS. lia.
  - exact Hp.
Qed.

Theorem struct_roundtrip : forall ptr,
  valid_payload ptr ->
  extract_tag (encode_struct ptr) = tag_id_Struct /\
  extract_payload (encode_struct ptr) = ptr.
Proof.
  intros ptr Hp. apply roundtrip_tag_payload.
  - unfold valid_tag, tag_id_Struct, NTAGS. lia.
  - exact Hp.
Qed.


(* ================================================================ *)
(* I'. BigInt Pointer Round-Trip                                     *)
(*                                                                    *)
(* BigInt uses tag 6 with a 47-bit Arc<BigInt> pointer payload.     *)
(* Same encoding scheme as PyObj (tag 4) and Struct (tag 5).         *)
(* Source: Value::from_bigint, as_bigint_ref (value.rs)              *)
(* ================================================================ *)

Definition encode_bigint (ptr : Z) : Z :=
  encode tag_id_BigInt ptr.

Theorem bigint_roundtrip : forall ptr,
  valid_payload ptr ->
  extract_tag (encode_bigint ptr) = tag_id_BigInt /\
  extract_payload (encode_bigint ptr) = ptr.
Proof.
  intros ptr Hp. apply roundtrip_tag_payload.
  - unfold valid_tag, tag_id_BigInt, NTAGS. lia.
  - exact Hp.
Qed.


(* ================================================================ *)
(* I''. SmallInt / BigInt Promotion and Demotion                     *)
(*                                                                    *)
(* Source: Value::from_bigint_or_demote (value.rs:173-182)           *)
(*         bigint_binop, binary_add (core.rs)                        *)
(*                                                                    *)
(* SmallInt range: [-2^46, 2^46).                                    *)
(* from_bigint_or_demote(n):                                         *)
(*   if n fits SmallInt -> encode as SmallInt (demotion)             *)
(*   otherwise          -> keep as BigInt (no demotion)              *)
(* Arithmetic overflow: SmallInt result out of range -> promote.     *)
(* ================================================================ *)

Definition is_small (n : Z) : Prop :=
  -HALF_W <= n < HALF_W.

Definition needs_bigint (n : Z) : bool :=
  negb ((-HALF_W <=? n) && (n <? HALF_W)).

(* Decision function agrees with predicate *)
Theorem needs_bigint_false_iff_small : forall n,
  needs_bigint n = false <-> is_small n.
Proof.
  intro n. unfold needs_bigint, is_small.
  rewrite Bool.negb_false_iff, Bool.andb_true_iff.
  rewrite Z.leb_le, Z.ltb_lt. tauto.
Qed.

Theorem needs_bigint_true_iff_not_small : forall n,
  needs_bigint n = true <-> ~is_small n.
Proof.
  intro n. split.
  - intros Hn Hs. apply needs_bigint_false_iff_small in Hs. congruence.
  - intro Hns. destruct (needs_bigint n) eqn:E; [reflexivity|].
    exfalso. apply Hns. apply needs_bigint_false_iff_small. exact E.
Qed.

(* Demotion preserves mathematical value *)
Theorem demotion_preserves_value : forall n,
  is_small n ->
  sign_extend (extract_payload (encode_int n)) = n.
Proof. exact int_roundtrip_signed. Qed.

(* Promotion is necessary: out-of-range values cannot be demoted *)
Theorem promotion_necessary : forall n,
  needs_bigint n = true -> ~is_small n.
Proof.
  intros n Hn. apply needs_bigint_true_iff_not_small. exact Hn.
Qed.

(* Addition can overflow SmallInt range *)
Theorem add_can_overflow :
  exists a b,
  is_small a /\ is_small b /\ needs_bigint (a + b) = true.
Proof.
  exists (HALF_W - 1), 1. repeat split;
  unfold is_small, needs_bigint, HALF_W; try lia; reflexivity.
Qed.

(* Multiplication can overflow SmallInt range *)
Theorem mul_can_overflow :
  exists a b,
  is_small a /\ is_small b /\ needs_bigint (a * b) = true.
Proof.
  exists (HALF_W - 1), 2. repeat split;
  unfold is_small, needs_bigint, HALF_W; try lia; reflexivity.
Qed.

(* Within-range result stays SmallInt *)
Theorem add_in_range : forall a b,
  is_small a -> is_small b -> is_small (a + b) ->
  needs_bigint (a + b) = false.
Proof.
  intros a b _ _ Hab.
  apply needs_bigint_false_iff_small. exact Hab.
Qed.


(* ================================================================ *)
(* I'''. BigInt Truthiness                                           *)
(*                                                                    *)
(* Source: Value::is_truthy (value.rs:368-387)                       *)
(*                                                                    *)
(* Truthiness depends only on the mathematical value, not            *)
(* the representation (SmallInt or BigInt).                           *)
(* BigInt(0) is falsy. BigInt(n) for n <> 0 is truthy.              *)
(* ================================================================ *)

Definition int_truthy (n : Z) : bool :=
  negb (Z.eqb n 0).

Theorem truthy_zero : int_truthy 0 = false.
Proof. reflexivity. Qed.

Theorem truthy_nonzero : forall n, n <> 0 -> int_truthy n = true.
Proof.
  intros n Hn. unfold int_truthy.
  destruct (Z.eqb_spec n 0); [contradiction | reflexivity].
Qed.

(* Truthiness is representation-independent for small integers *)
Theorem truthy_invariant : forall n,
  is_small n ->
  int_truthy n = int_truthy (sign_extend (extract_payload (encode_int n))).
Proof.
  intros n Hs. rewrite demotion_preserves_value by exact Hs. reflexivity.
Qed.


(* ================================================================ *)
(* I''''. Promotion Totality and Path Coherence (Phase 5)            *)
(*                                                                    *)
(* Source: catnip_core/src/arith.rs (generic numeric_* int path)     *)
(*         catnip_core/src/scalar.rs (scalar_from_bigint_or_demote,  *)
(*         scalar_as_int with sign extension)                        *)
(*                                                                    *)
(* The generic int path of numeric_add/sub/mul has two branches:     *)
(*   fast: i64 checked_op, then scalar_try_from_int (small only)     *)
(*   slow: exact Z op, then scalar_from_bigint_or_demote             *)
(* Both must denote the same mathematical value and pick the same    *)
(* representation (small iff the value fits the 47-bit range), so    *)
(* the result never depends on which branch ran. bigint_binop        *)
(* (mixed small/big operands) is the slow path applied directly.     *)
(* ================================================================ *)

(* Integer result representation: payload smallint or heap bigint. *)
Inductive IntRepr : Type :=
  | RSmall (n : Z)
  | RBig (n : Z).

Definition denote (r : IntRepr) : Z :=
  match r with RSmall n => n | RBig n => n end.

(* scalar_from_bigint_or_demote: demote iff the value is small. *)
Definition from_bigint_or_demote (n : Z) : IntRepr :=
  if needs_bigint n then RBig n else RSmall n.

(* The shared constructor never changes the mathematical value. *)
Theorem from_bigint_or_demote_preserves : forall n,
  denote (from_bigint_or_demote n) = n.
Proof.
  intro n. unfold from_bigint_or_demote.
  destruct (needs_bigint n); reflexivity.
Qed.

(* The representation it picks is canonical: small iff in range. *)
Theorem from_bigint_or_demote_small_iff : forall n,
  (exists m, from_bigint_or_demote n = RSmall m) <-> is_small n.
Proof.
  intro n. unfold from_bigint_or_demote. split.
  - intros [m Hm]. destruct (needs_bigint n) eqn:E; [discriminate|].
    apply needs_bigint_false_iff_small. exact E.
  - intro Hs. apply needs_bigint_false_iff_small in Hs. rewrite Hs.
    exists n. reflexivity.
Qed.

(* to_bigint (arith.rs) is total and exact on the small form: it     *)
(* reads the payload back through sign extension (scalar_as_int).    *)
(* On the big form it borrows the heap Integer directly (identity).  *)
Definition to_bigint_small (v : Z) : option Z :=
  Some (sign_extend (extract_payload v)).

Theorem to_bigint_small_exact : forall n,
  is_small n -> to_bigint_small (encode_int n) = Some n.
Proof.
  intros n Hs. unfold to_bigint_small.
  rewrite int_roundtrip_signed by exact Hs. reflexivity.
Qed.

(* i64 domain of the fast path (checked_add/checked_sub/checked_mul). *)
Definition I64_MIN := -(2 ^ 63).
Definition I64_MAX := 2 ^ 63 - 1.

Definition in_i64 (n : Z) : bool := (I64_MIN <=? n) && (n <=? I64_MAX).

(* Fast path on the mathematical result v = op a b: checked_op       *)
(* succeeds iff v fits i64, scalar_try_from_int accepts iff v is     *)
(* small; anything else falls through to the slow path (None).       *)
Definition int_fast_path (v : Z) : option IntRepr :=
  if in_i64 v
  then (if needs_bigint v then None else Some (RSmall v))
  else None.

(* Agreement: whenever the fast path answers, the slow path would    *)
(* have produced the same representation (value and form).           *)
Theorem int_paths_agree : forall v r,
  int_fast_path v = Some r -> from_bigint_or_demote v = r.
Proof.
  intros v r H. unfold int_fast_path in H.
  destruct (in_i64 v); [|discriminate].
  destruct (needs_bigint v) eqn:E; [discriminate|].
  injection H as <-. unfold from_bigint_or_demote. rewrite E. reflexivity.
Qed.

(* Completeness on the common domain: a small result never escapes   *)
(* to the slow path (2^46 < 2^63, a small value always fits i64).    *)
Theorem int_fast_path_complete_on_small : forall v,
  is_small v -> int_fast_path v = Some (RSmall v).
Proof.
  intros v Hs.
  assert (Hrange : -HALF_W <= v < HALF_W) by exact Hs.
  unfold int_fast_path.
  assert (Hi : in_i64 v = true).
  { unfold in_i64, I64_MIN, I64_MAX. apply Bool.andb_true_iff.
    unfold HALF_W in Hrange.
    split; apply Z.leb_le; lia. }
  rewrite Hi.
  apply needs_bigint_false_iff_small in Hs. rewrite Hs. reflexivity.
Qed.

(* An i64 overflow can never demote: after a failed checked_op the   *)
(* slow path always keeps the bigint form (|v| > 2^63-1 > 2^46-1).   *)
Theorem i64_overflow_never_demotes : forall v,
  in_i64 v = false -> needs_bigint v = true.
Proof.
  intros v H. apply needs_bigint_true_iff_not_small.
  intro Hs. unfold is_small, HALF_W in Hs.
  unfold in_i64, I64_MIN, I64_MAX in H.
  apply Bool.andb_false_iff in H.
  destruct H as [H|H]; apply Z.leb_gt in H; lia.
Qed.


(* ================================================================ *)
(* J. Decode Completeness                                            *)
(*                                                                    *)
(* Any encoded value can be decomposed back to its tag and payload. *)
(* ================================================================ *)

Definition in_encoded_range (v : Z) : Prop :=
  exists tag payload,
    valid_tag tag /\ valid_payload payload /\
    v = encode tag payload.

Theorem encode_in_range : forall tag payload,
  valid_tag tag -> valid_payload payload ->
  in_encoded_range (encode tag payload).
Proof.
  intros tag payload Ht Hp.
  exists tag, payload. auto.
Qed.

Theorem decode_tag_valid : forall v,
  in_encoded_range v -> valid_tag (extract_tag v).
Proof.
  intros v [tag [payload [Ht [Hp Hv]]]].
  subst v. rewrite extract_tag_correct by assumption.
  exact Ht.
Qed.

Theorem decode_payload_valid : forall v,
  in_encoded_range v -> valid_payload (extract_payload v).
Proof.
  intros v [tag [payload [Ht [Hp Hv]]]].
  subst v. rewrite extract_payload_correct by assumption.
  exact Hp.
Qed.

Theorem encode_decode : forall v,
  in_encoded_range v ->
  encode (extract_tag v) (extract_payload v) = v.
Proof.
  intros v [tag [payload [Ht [Hp Hv]]]].
  subst v.
  rewrite extract_tag_correct by assumption.
  rewrite extract_payload_correct by assumption.
  reflexivity.
Qed.


(* ================================================================ *)
(* K. Tag Classification                                             *)
(*                                                                    *)
(* Every encoded value has exactly one of the 8 tags.               *)
(* ================================================================ *)

Theorem tag_exhaustive : forall v,
  in_encoded_range v ->
  let t := extract_tag v in
  t = tag_id_SmallInt \/ t = tag_id_Bool \/ t = tag_id_Nil \/
  t = tag_id_Symbol \/ t = tag_id_PyObj \/ t = tag_id_Struct \/
  t = tag_id_BigInt \/ t = tag_id_VMFunc.
Proof.
  intros v Hv.
  destruct (decode_tag_valid v Hv) as [Hlo Hhi].
  unfold tag_id_SmallInt, tag_id_Bool, tag_id_Nil,
         tag_id_Symbol, tag_id_PyObj, tag_id_Struct,
         tag_id_BigInt, tag_id_VMFunc, NTAGS in *.
  lia.
Qed.

(* Nil is unique: only one nil value exists *)
Theorem nil_unique : forall p,
  valid_payload p ->
  extract_tag (encode tag_id_Nil p) = tag_id_Nil ->
  encode tag_id_Nil p = encode tag_id_Nil 0 ->
  p = 0.
Proof.
  intros p Hp _ Henc.
  apply encode_injective in Henc.
  - destruct Henc as [_ Heq]. exact Heq.
  - unfold valid_tag, tag_id_Nil, NTAGS. lia.
  - unfold valid_tag, tag_id_Nil, NTAGS. lia.
  - exact Hp.
  - unfold valid_payload, W. lia.
Qed.


(* ================================================================ *)
(* L. Concrete Examples                                              *)
(* ================================================================ *)

Example ex_encode_zero :
  encode tag_id_SmallInt 0 = QNAN * W.
Proof. unfold encode, tag_id_SmallInt. lia. Qed.

Example ex_tag_nil :
  extract_tag (encode tag_id_Nil 0) = tag_id_Nil.
Proof. reflexivity. Qed.

Example ex_tag_symbol_42 :
  extract_tag (encode tag_id_Symbol 42) = tag_id_Symbol.
Proof. reflexivity. Qed.

Example ex_payload_symbol_42 :
  extract_payload (encode tag_id_Symbol 42) = 42.
Proof. reflexivity. Qed.

Example ex_bool_true :
  decode_bool (encode_bool true) = true.
Proof. reflexivity. Qed.

Example ex_bool_false :
  decode_bool (encode_bool false) = false.
Proof. reflexivity. Qed.

Example ex_int_positive :
  extract_payload (encode_int 255) = 255.
Proof. reflexivity. Qed.

Example ex_int_signed_neg :
  sign_extend (extract_payload (encode_int (-1))) = -1.
Proof. reflexivity. Qed.

Example ex_int_signed_min :
  sign_extend (extract_payload (encode_int (-70368744177664))) = -70368744177664.
Proof. reflexivity. Qed.

Example ex_nil_tag :
  extract_tag encode_nil = tag_id_Nil.
Proof. reflexivity. Qed.

Example ex_nil_payload :
  extract_payload encode_nil = 0.
Proof. reflexivity. Qed.

Example ex_symbol_100 :
  extract_tag (encode_symbol 100) = tag_id_Symbol /\
  extract_payload (encode_symbol 100) = 100.
Proof. split; reflexivity. Qed.

Example ex_pyobj_ptr :
  extract_tag (encode_pyobj 12345) = tag_id_PyObj /\
  extract_payload (encode_pyobj 12345) = 12345.
Proof. split; reflexivity. Qed.

Example ex_struct_ptr :
  extract_tag (encode_struct 999) = tag_id_Struct /\
  extract_payload (encode_struct 999) = 999.
Proof. split; reflexivity. Qed.

(* Different tags always produce different values *)
Example ex_disjoint_bool_nil :
  encode_bool true <> encode_nil.
Proof.
  apply different_tags_different_values.
  - unfold valid_tag, tag_id_Bool, NTAGS. lia.
  - unfold valid_tag, tag_id_Nil, NTAGS. lia.
  - unfold valid_payload, W. lia.
  - unfold valid_payload, W. lia.
  - unfold tag_id_Bool, tag_id_Nil. lia.
Qed.

Example ex_disjoint_int_symbol :
  encode_int 42 <> encode_symbol 42.
Proof.
  unfold encode_int, encode_symbol.
  apply different_tags_different_values.
  - unfold valid_tag, tag_id_SmallInt, NTAGS. lia.
  - unfold valid_tag, tag_id_Symbol, NTAGS. lia.
  - unfold valid_payload, W. split;
    apply Z.mod_pos_bound; pose proof W_pos; lia.
  - unfold valid_payload, W. lia.
  - unfold tag_id_SmallInt, tag_id_Symbol. lia.
Qed.

(* Round-trip: encode then decode recovers original *)
Example ex_roundtrip_struct :
  encode (extract_tag (encode_struct 777))
         (extract_payload (encode_struct 777))
  = encode_struct 777.
Proof. reflexivity. Qed.

Example ex_bigint_tag :
  extract_tag (encode_bigint 54321) = tag_id_BigInt.
Proof. reflexivity. Qed.

Example ex_bigint_payload :
  extract_payload (encode_bigint 54321) = 54321.
Proof. reflexivity. Qed.

Example ex_bigint_roundtrip_encode_decode :
  encode (extract_tag (encode_bigint 88888))
         (extract_payload (encode_bigint 88888))
  = encode_bigint 88888.
Proof. reflexivity. Qed.

Example ex_disjoint_bigint_int :
  encode_bigint 42 <> encode tag_id_SmallInt 42.
Proof.
  apply different_tags_different_values.
  - unfold valid_tag, tag_id_BigInt, NTAGS. lia.
  - unfold valid_tag, tag_id_SmallInt, NTAGS. lia.
  - unfold valid_payload, W. lia.
  - unfold valid_payload, W. lia.
  - unfold tag_id_BigInt, tag_id_SmallInt. lia.
Qed.

Example ex_demotion_42 :
  needs_bigint 42 = false.
Proof. reflexivity. Qed.

Example ex_promotion_boundary :
  needs_bigint HALF_W = true.
Proof. reflexivity. Qed.

Example ex_truthy_nonzero :
  int_truthy 999 = true.
Proof. reflexivity. Qed.
