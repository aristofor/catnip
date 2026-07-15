(* FILE: proof/vm/CatnipOwnershipProof.v *)
(* CatnipOwnershipProof.v - Struct slot refcount ownership across VM boundary
 *
 * Source of truth:
 *   catnip_rs/src/vm/structs.rs  (StructRegistry refcount, transplant_to_parent,
 *                                 instance_to_pyobject, CatnipStructProxy)
 *   catnip_rs/src/vm/py_interop.rs (portabilize_struct_values: ownership transfer)
 *
 * Layer C of the value-boundary model. A struct lives in a registry slot with a
 * refcount. Two kinds of owner hold a reference: the VM itself (an internal
 * handle, bounded by the VM's lifetime) and each Python proxy that escaped to
 * the host. The invariant is `refcount = number of live owners`.
 *
 * The documented cross-VM leak is a refcount-arithmetic fact: a broadcast child
 * creates a struct (one VM-internal owner), k proxies escape, then the child
 * ends. `transplant_to_parent` currently COPIES the count into the parent,
 * carrying the child's VM-internal owner that no longer exists -- a phantom that
 * no decref will ever release. This file proves that copy-transplant leaves a
 * stuck refcount of 1, while a transfer-transplant (release the VM-internal
 * owner at the result boundary, move the slot) returns to 0 and is freeable.
 *
 * This is the specification for the Phase 5 refcount refactor.
 *
 * Sections G/H cover the residual sibling phantom: a struct created in the child
 * and referenced only by proxies that escaped to the host (no VM-internal owner
 * survives the callback). Those proxies stay bound to the child id, which dies
 * with the child, so their decref no-ops and the transplanted count is stuck (G).
 * The fix re-anchors the proxies of TRANSPLANTED slots onto the parent. A
 * pass-through slot (already in the parent) must NOT be re-anchored: its proxy's
 * incref lived on the child and never reached the parent, so re-anchoring would
 * over-decref the parent -- an underflow below its real owners (H).
 *
 * Standalone: refcount arithmetic over nat.
 *
 * 13 theorems, 0 Admitted.
 *)

From Coq Require Import Arith Lia.


(* ================================================================ *)
(* A. Refcount primitives                                            *)
(* ================================================================ *)

Definition Refcount := nat.

Definition incref (rc : Refcount) : Refcount := S rc.
Definition decref (rc : Refcount) : Refcount := pred rc.

(* A slot is freeable exactly when no owner remains. *)
Definition freeable (rc : Refcount) : Prop := rc = 0.

(* Every incref is undone by exactly one decref. *)
Theorem incref_decref_balanced : forall rc, decref (incref rc) = rc.
Proof. intro rc. unfold decref, incref. simpl. reflexivity. Qed.


(* ================================================================ *)
(* B. Ownership invariant                                            *)
(*                                                                    *)
(* refcount = number of live owners (VM-internal handle + proxies).  *)
(* ================================================================ *)

Definition live_owners (vm_internal proxies : nat) : nat := vm_internal + proxies.


(* ================================================================ *)
(* C. Slot lifecycle across a broadcast child -> parent transplant   *)
(* ================================================================ *)

(* The child creates the struct: one VM-internal owner. *)
Definition created : Refcount := 1.

(* k Python proxies escape to the host (each increfs). *)
Definition with_proxies (k : nat) : Refcount := 1 + k.

(* After the child VM is gone, the k proxies are dropped by Python GC (each
   decrefs its own registry slot). *)
Definition drop_proxies (k : nat) (rc : Refcount) : Refcount := rc - k.

(* COPY transplant (current behaviour): the parent inherits the child's whole
   count, including the VM-internal owner that died with the child. *)
Definition copy_residual (k : nat) : Refcount :=
  drop_proxies k (with_proxies k).

(* TRANSFER transplant (the fix): the VM-internal owner is released at the
   result boundary (portabilize_struct_values style) before the slot moves to
   the parent, so only the proxy ownership transfers. *)
Definition transfer_residual (k : nat) : Refcount :=
  drop_proxies k (decref (with_proxies k)).


(* ================================================================ *)
(* D. The leak, and its fix                                          *)
(* ================================================================ *)

(* Copy-transplant leaves a phantom owner: the slot is stuck at 1 forever. *)
Theorem copy_leaks : forall k, copy_residual k = 1.
Proof. intro k. unfold copy_residual, drop_proxies, with_proxies. lia. Qed.

(* Transfer-transplant returns the slot to zero once all proxies drop. *)
Theorem transfer_no_leak : forall k, transfer_residual k = 0.
Proof.
  intro k. unfold transfer_residual, drop_proxies, decref, with_proxies. lia.
Qed.

(* Equivalently, phrased as freeability. *)
Theorem transfer_freeable : forall k, freeable (transfer_residual k).
Proof. intro k. unfold freeable. apply transfer_no_leak. Qed.

Theorem copy_not_freeable : forall k, ~ freeable (copy_residual k).
Proof. intro k. unfold freeable. rewrite copy_leaks. discriminate. Qed.


(* ================================================================ *)
(* E. Why: the invariant is preserved by transfer, broken by copy    *)
(* ================================================================ *)

(* After transfer, the VM-internal owner is gone, so the refcount equals the
   real surviving owners (the proxies). *)
Theorem transfer_preserves_invariant : forall k,
  decref (with_proxies k) = live_owners 0 k.
Proof. intro k. unfold decref, with_proxies, live_owners. simpl. lia. Qed.

(* Copy keeps a refcount that counts a VM-internal owner which no longer
   exists: it does not match the real surviving owners. *)
Theorem copy_breaks_invariant : forall k,
  with_proxies k = live_owners 1 k /\ live_owners 1 k <> live_owners 0 k.
Proof.
  intro k. split.
  - unfold with_proxies, live_owners. lia.
  - unfold live_owners. lia.
Qed.


(* ================================================================ *)
(* F. Concrete examples                                             *)
(* ================================================================ *)

(* One struct, one escaped proxy: copy leaks, transfer frees. *)
Example ex_copy_leaks_one_proxy : copy_residual 1 = 1.
Proof. reflexivity. Qed.

Example ex_transfer_frees_one_proxy : transfer_residual 1 = 0.
Proof. reflexivity. Qed.

(* No escaped proxy: copy still leaks the VM-internal phantom, transfer is 0. *)
Example ex_copy_leaks_no_proxy : copy_residual 0 = 1.
Proof. reflexivity. Qed.

Example ex_transfer_frees_no_proxy : transfer_residual 0 = 0.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* G. Escaped non-result proxy: the anchor must follow the slot       *)
(*                                                                    *)
(* A struct created in the child and referenced only by k proxies     *)
(* that escaped to the host (the VM-internal owner was released at     *)
(* frame teardown before transplant). transplant_to_parent copies the *)
(* surviving count k into the parent. The question is which registry   *)
(* those k proxies decref when Python finally drops them.             *)
(* ================================================================ *)

(* Count carried into the parent by transplant: the k escaped proxies. *)
Definition escaped_transplanted (k : nat) : Refcount := k.

(* STALE anchor (the residual bug): the proxies stay bound to the child id,
   unregistered when the child dies, so their decref no-ops against the parent
   and the copied count is never reduced. *)
Definition stale_anchor_residual (k : nat) : Refcount := escaped_transplanted k.

(* RE-ANCHORED (the fix): each proxy is retargeted to the parent at transplant,
   so its drop decrefs the parent slot the count was copied into. *)
Definition reanchored_residual (k : nat) : Refcount :=
  drop_proxies k (escaped_transplanted k).

(* The stale anchor leaks the entire escaped count (k proxies -> k phantom). *)
Theorem stale_anchor_leaks : forall k, stale_anchor_residual k = k.
Proof. intro k. unfold stale_anchor_residual, escaped_transplanted. reflexivity. Qed.

(* With at least one escaped proxy the stale anchor is not freeable. *)
Theorem stale_anchor_not_freeable : forall k, k <> 0 -> ~ freeable (stale_anchor_residual k).
Proof. intros k Hk. unfold freeable. rewrite stale_anchor_leaks. exact Hk. Qed.

(* Re-anchoring returns the slot to zero once all proxies drop. *)
Theorem reanchor_no_leak : forall k, reanchored_residual k = 0.
Proof. intro k. unfold reanchored_residual, drop_proxies, escaped_transplanted. lia. Qed.

Theorem reanchor_freeable : forall k, freeable (reanchored_residual k).
Proof. intro k. unfold freeable. apply reanchor_no_leak. Qed.


(* ================================================================ *)
(* H. Pass-through must NOT be re-anchored (over-decref guard)        *)
(*                                                                    *)
(* A pass-through struct already lives in the parent with p > 0 real  *)
(* owners. A proxy materialized for it in the child increfs the CHILD  *)
(* slot only, and transplant does not copy a pass-through slot, so the *)
(* parent count stays p. Leaving the proxy bound to the child (no-op   *)
(* on death) preserves p. Re-anchoring it would decref the parent for  *)
(* an incref it never received -- driving the count below the p owners *)
(* that still hold the slot (a use-after-free).                       *)
(* ================================================================ *)

(* Parent's own count for a pass-through slot; the callback's child-side
   increfs/decrefs die with the child and never reach it. *)
Definition passthrough_parent (p : nat) : Refcount := p.

(* Leaving the child-anchored proxies as no-ops preserves the parent count. *)
Definition passthrough_noop (p : nat) : Refcount := passthrough_parent p.

(* Wrongly re-anchoring q proxies onto the parent decrefs it q times. *)
Definition passthrough_reanchored (p q : nat) : Refcount :=
  drop_proxies q (passthrough_parent p).

(* No-op preserves the parent's real owner count. *)
Theorem passthrough_noop_preserves : forall p, passthrough_noop p = p.
Proof. intro p. unfold passthrough_noop, passthrough_parent. reflexivity. Qed.

(* Re-anchoring a pass-through loses an owner the no-op path keeps: with a live
   parent (p > 0) and at least one wrongly-re-anchored proxy, the residual drops
   strictly below the parent's real owner count. *)
Theorem passthrough_reanchor_underflows : forall p q,
  0 < p -> 0 < q -> passthrough_reanchored p q < passthrough_parent p.
Proof.
  intros p q Hp Hq. unfold passthrough_reanchored, drop_proxies, passthrough_parent. lia.
Qed.
