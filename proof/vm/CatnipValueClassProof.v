(* FILE: proof/vm/CatnipValueClassProof.v *)
(* CatnipValueClassProof.v - Canonical tag classification (Scalar/Index/Pointer)
 *
 * Source of truth:
 *   catnip_core/src/nanbox.rs        (shared tag geometry + TagClass)
 *   catnip_rs/src/vm/value.rs        (8-tag PyO3 Value)
 *   catnip_vm/src/value.rs           (16-tag pure Value)
 *
 * Layer A of the value-boundary model. Every NaN-box tag carries a payload
 * of exactly one of three classes:
 *
 *   - Scalar  : inline data, always boundary-safe          (SmallInt, Bool, Nil, Symbol)
 *   - Index   : bounded table handle, safe iff in bounds   (PyObj, Struct, VMFunc)
 *   - Pointer : raw Arc pointer, NOT certifiable from bits  (BigInt, Complex, collections)
 *
 * The per-crate tag sets diverge above tag 7 (catnip_vm native collections vs
 * catnip_rs PyObject), so the classification is parameterized by the tag set.
 * This file proves the canonical catnip_rs 8-tag instance; the boundary
 * theorems (Layer B) depend only on the Scalar set being exactly {0,1,2,3},
 * which holds in both crates.
 *
 * Builds on CatnipNanBoxProof (encode/extract/tag ids/valid_tag).
 *
 * 8 theorems, 0 Admitted.
 *)

From Coq Require Import ZArith Bool Lia.
From Catnip Require Import CatnipNanBoxProof.
Open Scope Z_scope.


(* ================================================================ *)
(* A. The three payload classes                                      *)
(* ================================================================ *)

Inductive TagClass := Scalar | Index | Pointer.


(* ================================================================ *)
(* B. Per-class tag predicates (canonical catnip_rs tag set)         *)
(* ================================================================ *)

Definition is_scalar_tag (t : Z) : bool :=
  (t =? tag_id_SmallInt) || (t =? tag_id_Bool) ||
  (t =? tag_id_Nil) || (t =? tag_id_Symbol).

Definition is_index_tag (t : Z) : bool :=
  (t =? tag_id_PyObj) || (t =? tag_id_Struct) || (t =? tag_id_VMFunc).

Definition is_pointer_tag (t : Z) : bool :=
  t =? tag_id_BigInt.

(* The classification function. Pointer is checked first, then Index;
   every remaining valid tag is Scalar. *)
Definition classify (t : Z) : TagClass :=
  if is_pointer_tag t then Pointer
  else if is_index_tag t then Index
  else Scalar.


(* ================================================================ *)
(* C. Exhaustiveness: every valid tag is covered by one predicate    *)
(* ================================================================ *)

Theorem class_exhaustive : forall t,
  valid_tag t ->
  is_scalar_tag t = true \/ is_index_tag t = true \/ is_pointer_tag t = true.
Proof.
  intros t [Hlo Hhi]. unfold NTAGS in Hhi.
  assert (Ht: t = 0 \/ t = 1 \/ t = 2 \/ t = 3 \/
              t = 4 \/ t = 5 \/ t = 6 \/ t = 7) by lia.
  destruct Ht as [H|[H|[H|[H|[H|[H|[H|H]]]]]]]; subst t; vm_compute;
    ((left; reflexivity) || (right; left; reflexivity)
                          || (right; right; reflexivity)).
Qed.


(* ================================================================ *)
(* D. Pairwise disjointness of the classes                           *)
(* ================================================================ *)

Theorem scalar_pointer_disjoint : forall t,
  is_scalar_tag t = true -> is_pointer_tag t = false.
Proof.
  intros t H. unfold is_pointer_tag, tag_id_BigInt.
  destruct (Z.eqb_spec t 6) as [E|]; [| reflexivity].
  subst t. unfold is_scalar_tag, tag_id_SmallInt, tag_id_Bool,
    tag_id_Nil, tag_id_Symbol in H. vm_compute in H. discriminate H.
Qed.

Theorem index_pointer_disjoint : forall t,
  is_index_tag t = true -> is_pointer_tag t = false.
Proof.
  intros t H. unfold is_pointer_tag, tag_id_BigInt.
  destruct (Z.eqb_spec t 6) as [E|]; [| reflexivity].
  subst t. unfold is_index_tag, tag_id_PyObj, tag_id_Struct,
    tag_id_VMFunc in H. vm_compute in H. discriminate H.
Qed.

Theorem scalar_index_disjoint : forall t,
  is_scalar_tag t = true -> is_index_tag t = false.
Proof.
  intros t H. unfold is_scalar_tag in H.
  repeat (apply orb_prop in H; destruct H as [H|H]);
    apply Z.eqb_eq in H; subst t; reflexivity.
Qed.


(* ================================================================ *)
(* E. classify agrees with the predicates                            *)
(* ================================================================ *)

Theorem classify_pointer : forall t,
  is_pointer_tag t = true -> classify t = Pointer.
Proof.
  intros t H. unfold classify. rewrite H. reflexivity.
Qed.

Theorem classify_index : forall t,
  is_index_tag t = true -> classify t = Index.
Proof.
  intros t H. unfold classify.
  rewrite (index_pointer_disjoint t H), H. reflexivity.
Qed.

Theorem classify_scalar : forall t,
  is_scalar_tag t = true -> classify t = Scalar.
Proof.
  intros t H. unfold classify.
  rewrite (scalar_pointer_disjoint t H), (scalar_index_disjoint t H).
  reflexivity.
Qed.


(* ================================================================ *)
(* F. Totality: classify lands in exactly one class                  *)
(* ================================================================ *)

Theorem classify_total : forall t,
  classify t = Scalar \/ classify t = Index \/ classify t = Pointer.
Proof.
  intro t. unfold classify.
  destruct (is_pointer_tag t); [ auto |].
  destruct (is_index_tag t); auto.
Qed.


(* ================================================================ *)
(* G. Concrete examples                                              *)
(* ================================================================ *)

Example ex_smallint_scalar : classify tag_id_SmallInt = Scalar.
Proof. reflexivity. Qed.

Example ex_symbol_scalar : classify tag_id_Symbol = Scalar.
Proof. reflexivity. Qed.

Example ex_struct_index : classify tag_id_Struct = Index.
Proof. reflexivity. Qed.

Example ex_pyobj_index : classify tag_id_PyObj = Index.
Proof. reflexivity. Qed.

Example ex_vmfunc_index : classify tag_id_VMFunc = Index.
Proof. reflexivity. Qed.

Example ex_bigint_pointer : classify tag_id_BigInt = Pointer.
Proof. reflexivity. Qed.
