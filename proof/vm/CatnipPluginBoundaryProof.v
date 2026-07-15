(* FILE: proof/vm/CatnipPluginBoundaryProof.v *)
(* CatnipPluginBoundaryProof.v - Native plugin FFI boundary (ABI v4 lock)
 *
 * Source of truth:
 *   catnip_vm/src/plugin.rs  (admit_plugin_scalar, interpret_plugin_result,
 *                             PLUGIN_RESULT_OBJECT / PLUGIN_RESULT_HOSTVALUE)
 *
 * A native plugin returns a `PluginResult` carrying a `u64` value and flags.
 * The host admits it through exactly one of three channels:
 *
 *   - OBJECT    : an opaque handle, never dereferenced as a value.
 *   - HOSTVALUE : a value the host itself built via the PluginHostApi builder
 *                 callbacks, so its pointer lives in the host heap (trusted).
 *   - (none)    : must be an inline scalar -- routed through `from_raw_scalar`.
 *
 * This file proves the ABI v4 lock: an unflagged result of Pointer or Index
 * class is rejected, and every value admitted through the unflagged channel is
 * Scalar-class. A plugin therefore cannot hand the host a raw pointer into its
 * own heap unless it goes through the host builders (HOSTVALUE) -- the host
 * never dereferences a plugin-owned Arc.
 *
 * Builds on CatnipBoundaryProof (RawValue, from_raw_scalar, rv_class).
 *
 * 5 theorems, 0 Admitted.
 *)

From Coq Require Import ZArith Bool Lia.
From Catnip Require Import CatnipNanBoxProof.
From Catnip Require Import CatnipValueClassProof.
From Catnip Require Import CatnipBoundaryProof.
Open Scope Z_scope.


(* ================================================================ *)
(* A. Plugin result admission                                        *)
(* ================================================================ *)

Inductive PluginFlag :=
  | FNone        (* no flag: must be an inline scalar *)
  | FObject      (* PLUGIN_RESULT_OBJECT: opaque handle *)
  | FHostValue.  (* PLUGIN_RESULT_HOSTVALUE: host-built, trusted *)

Inductive Admission :=
  | AdmObject (handle : Z)   (* opaque handle, payload never reconstructed *)
  | AdmValue (v : RawValue)  (* a value admitted into the VM *)
  | Rejected.

(* Mirrors interpret_plugin_result's non-error path + admit_plugin_scalar. *)
Definition admit_plugin (flag : PluginFlag) (v : RawValue) : Admission :=
  match flag with
  | FObject => AdmObject (rv_payload v)
  | FHostValue => AdmValue v
  | FNone =>
      match from_raw_scalar v with
      | Some v' => AdmValue v'
      | None => Rejected
      end
  end.


(* ================================================================ *)
(* B. The unflagged channel is scalar-only                           *)
(* ================================================================ *)

(* A plugin cannot hand the host a raw pointer without the host-builder flag. *)
Theorem unflagged_pointer_rejected : forall v,
  rv_class v = Pointer -> admit_plugin FNone v = Rejected.
Proof.
  intros v H. simpl. rewrite (from_raw_scalar_rejects_pointer v H). reflexivity.
Qed.

(* Likewise an unvalidated index handle is rejected on the unflagged channel. *)
Theorem unflagged_index_rejected : forall v,
  rv_class v = Index -> admit_plugin FNone v = Rejected.
Proof.
  intros v H. simpl. rewrite (from_raw_scalar_rejects_index v H). reflexivity.
Qed.

(* Everything admitted through the unflagged channel is Scalar-class. *)
Theorem unflagged_admits_only_scalar : forall v v',
  admit_plugin FNone v = AdmValue v' -> rv_class v' = Scalar.
Proof.
  intros v v' H. simpl in H.
  destruct (from_raw_scalar v) as [r |] eqn:E; [| discriminate H].
  injection H as H'. subst r.
  exact (from_raw_scalar_class_scalar v v' E).
Qed.


(* ================================================================ *)
(* C. The sanctioned channels                                        *)
(* ================================================================ *)

(* An OBJECT result is admitted as an opaque handle: its payload is never
   reconstructed into a value, so a pointer payload is never dereferenced. *)
Theorem object_is_opaque_handle : forall v,
  admit_plugin FObject v = AdmObject (rv_payload v).
Proof. intro v. reflexivity. Qed.

(* The HOSTVALUE channel is the only way a non-scalar value crosses, and it
   carries the host's own allocation (built via PluginHostApi). The model
   admits it as-is: trust here is the host-built invariant, established by
   construction, not certifiable from bits (cf. Pointer in CatnipBoundaryProof). *)
Theorem hostvalue_admitted : forall v,
  admit_plugin FHostValue v = AdmValue v.
Proof. intro v. reflexivity. Qed.


(* ================================================================ *)
(* D. Concrete examples                                              *)
(* ================================================================ *)

(* A forged BigInt-tagged result with no flag is rejected. *)
Example ex_unflagged_bigint_rejected :
  admit_plugin FNone (RTagged tag_id_BigInt 4242) = Rejected.
Proof. reflexivity. Qed.

(* A scalar result with no flag is admitted. *)
Example ex_unflagged_smallint_admitted :
  admit_plugin FNone (RTagged tag_id_SmallInt 7) = AdmValue (RTagged tag_id_SmallInt 7).
Proof. reflexivity. Qed.

(* The same BigInt is admitted only through the host-builder channel. *)
Example ex_hostvalue_bigint_admitted :
  admit_plugin FHostValue (RTagged tag_id_BigInt 4242)
  = AdmValue (RTagged tag_id_BigInt 4242).
Proof. reflexivity. Qed.
