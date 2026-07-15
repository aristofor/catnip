(* FILE: proof/vm/CatnipBoundaryProof.v *)
(* CatnipBoundaryProof.v - Value-boundary safety (the from_raw lock)
 *
 * Source of truth:
 *   catnip_core/src/nanbox.rs        (from_raw_scalar, TagClass)
 *   catnip_rs/src/vm/value.rs        (from_raw, JIT restore)
 *   catnip_rs/src/jit/executor.rs    (catnip_unbox_int/float)
 *   catnip_rs/src/loader/native_plugin.rs (plugin FFI boundary)
 *
 * Layer B of the value-boundary model. A raw u64 crossing a trust boundary
 * (JIT codegen, plugin FFI) is decoded into a value. The danger is a POINTER
 * tag (BigInt/Complex/Native): its payload is a raw Arc pointer that gets
 * dereferenced -- it CANNOT be certified from bits alone.
 *
 * This file proves that the scalar boundary constructor `from_raw_scalar`
 * never yields a Pointer- or Index-class value: a raw-bits source can only
 * produce inline scalars (floats + SmallInt/Bool/Nil/Symbol). Index handles
 * must instead go through `validate_index`, which proves the bound.
 *
 * The decoded value is modeled as a sum (RFloat | RTagged) so the float case
 * -- an inline IEEE-754 double with no pointer payload -- is explicit, while
 * the tagged case carries a 4-bit tag classified by Layer A.
 *
 * Builds on CatnipNanBoxProof and CatnipValueClassProof.
 *
 * 8 theorems, 0 Admitted.
 *)

From Coq Require Import ZArith Bool Lia.
From Catnip Require Import CatnipNanBoxProof.
From Catnip Require Import CatnipValueClassProof.
Open Scope Z_scope.


(* ================================================================ *)
(* A. Decoded boundary value                                         *)
(*                                                                    *)
(* A u64 is either a non-QNAN IEEE-754 double (RFloat) or a          *)
(* quiet-NaN-tagged value (RTagged tag payload). The Rust `is_float` *)
(* check short-circuits before tag classification; floats carry no   *)
(* pointer payload, so they are inline scalars by construction.      *)
(* ================================================================ *)

Inductive RawValue :=
  | RFloat (f : Z)
  | RTagged (tag payload : Z).

Definition rv_class (v : RawValue) : TagClass :=
  match v with
  | RFloat _ => Scalar
  | RTagged tag _ => classify tag
  end.

Definition rv_payload (v : RawValue) : Z :=
  match v with
  | RFloat _ => 0
  | RTagged _ p => p
  end.


(* ================================================================ *)
(* B. Class never-scalar helpers                                     *)
(* ================================================================ *)

Lemma classify_pointer_not_scalar : forall t,
  classify t = Pointer -> is_scalar_tag t = false.
Proof.
  intros t H. destruct (is_scalar_tag t) eqn:E; [| reflexivity].
  rewrite (classify_scalar t E) in H. discriminate H.
Qed.

Lemma classify_index_not_scalar : forall t,
  classify t = Index -> is_scalar_tag t = false.
Proof.
  intros t H. destruct (is_scalar_tag t) eqn:E; [| reflexivity].
  rewrite (classify_scalar t E) in H. discriminate H.
Qed.


(* ================================================================ *)
(* C. The scalar boundary constructor                                *)
(*                                                                    *)
(* Models catnip_core::nanbox::from_raw_scalar: a float passes       *)
(* through; a tagged value is accepted only if its tag is Scalar,    *)
(* otherwise it is rejected (Rust returns INVALID).                  *)
(* ================================================================ *)

Definition from_raw_scalar (v : RawValue) : option RawValue :=
  match v with
  | RFloat f => Some (RFloat f)
  | RTagged tag payload =>
      if is_scalar_tag tag then Some (RTagged tag payload) else None
  end.

(* THE LOCK: anything from_raw_scalar accepts is Scalar-class.        *)
(* No Pointer/Index value is reachable through the raw-bits path,     *)
(* so no unvalidated pointer is ever dereferenced.                    *)
Theorem from_raw_scalar_class_scalar : forall v v',
  from_raw_scalar v = Some v' -> rv_class v' = Scalar.
Proof.
  intros [f | tag p] v' H; simpl in H.
  - inversion H; subst. reflexivity.
  - destruct (is_scalar_tag tag) eqn:E; [| discriminate H].
    inversion H; subst. simpl. apply classify_scalar. exact E.
Qed.

Theorem from_raw_scalar_rejects_pointer : forall v,
  rv_class v = Pointer -> from_raw_scalar v = None.
Proof.
  intros [f | tag p] H; simpl in *.
  - discriminate H.
  - rewrite (classify_pointer_not_scalar tag H). reflexivity.
Qed.

Theorem from_raw_scalar_rejects_index : forall v,
  rv_class v = Index -> from_raw_scalar v = None.
Proof.
  intros [f | tag p] H; simpl in *.
  - discriminate H.
  - rewrite (classify_index_not_scalar tag H). reflexivity.
Qed.


(* ================================================================ *)
(* D. Boundary safety predicate                                      *)
(*                                                                    *)
(* A value is boundary-safe iff it is Scalar, or an Index handle     *)
(* proven within its table bound. Pointer is never boundary-safe.    *)
(* ================================================================ *)

Definition boundary_safe (v : RawValue) (bound : Z) : Prop :=
  rv_class v = Scalar \/ (rv_class v = Index /\ rv_payload v < bound).

Theorem pointer_not_boundary_safe : forall v bound,
  rv_class v = Pointer -> ~ boundary_safe v bound.
Proof.
  intros v bound HP [HS | [HI _]]; rewrite HP in *; discriminate.
Qed.

Theorem from_raw_scalar_boundary_safe : forall v v' bound,
  from_raw_scalar v = Some v' -> boundary_safe v' bound.
Proof.
  intros v v' bound H. left.
  exact (from_raw_scalar_class_scalar v v' H).
Qed.


(* ================================================================ *)
(* E. Validated index handles                                        *)
(*                                                                    *)
(* The only sanctioned way to admit an Index value: prove the bound. *)
(* Mirrors a bounds-checked table access at the boundary.            *)
(* ================================================================ *)

Definition validate_index (v : RawValue) (bound : Z) : option RawValue :=
  match v with
  | RTagged tag payload =>
      if is_index_tag tag && (payload <? bound) then Some v else None
  | RFloat _ => None
  end.

Theorem validate_index_sound : forall v v' bound,
  validate_index v bound = Some v' ->
  rv_class v' = Index /\ rv_payload v' < bound.
Proof.
  intros [f | tag p] v' bound H; simpl in H; [ discriminate H |].
  destruct (is_index_tag tag) eqn:Ei; simpl in H; [| discriminate H].
  destruct (p <? bound) eqn:Eb; [| discriminate H].
  inversion H; subst. simpl. split.
  - apply classify_index. exact Ei.
  - apply Z.ltb_lt. exact Eb.
Qed.


(* ================================================================ *)
(* F. Concrete examples                                              *)
(* ================================================================ *)

(* A forged BigInt-tagged word is rejected at the boundary. *)
Example ex_reject_forged_bigint :
  from_raw_scalar (RTagged tag_id_BigInt 123456) = None.
Proof. reflexivity. Qed.

(* A SmallInt word passes. *)
Example ex_accept_smallint :
  from_raw_scalar (RTagged tag_id_SmallInt 42) = Some (RTagged tag_id_SmallInt 42).
Proof. reflexivity. Qed.

(* A float passes (inline scalar). *)
Example ex_accept_float :
  from_raw_scalar (RFloat 1024) = Some (RFloat 1024).
Proof. reflexivity. Qed.

(* A struct handle is rejected by from_raw_scalar... *)
Example ex_reject_struct_scalar :
  from_raw_scalar (RTagged tag_id_Struct 7) = None.
Proof. reflexivity. Qed.

(* ...but admitted by validate_index once the bound is shown. *)
Example ex_validate_struct_in_bounds :
  validate_index (RTagged tag_id_Struct 7) 16
  = Some (RTagged tag_id_Struct 7).
Proof. reflexivity. Qed.

Example ex_validate_struct_out_of_bounds :
  validate_index (RTagged tag_id_Struct 99) 16 = None.
Proof. reflexivity. Qed.
