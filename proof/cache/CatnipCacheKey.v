(* FILE: proof/cache/CatnipCacheKey.v *)
(* CacheType and CacheKey model: encoding, injectivity, disjointness.
 *
 * Source: catnip_rs/src/cache/mod.rs (CacheType, CacheKey)
 *
 * Proves:
 *   - CacheType equality decision
 *   - CacheKey equality decision
 *   - cache_key_to_z injectivity (Theorem 5)
 *   - Type / optimize / TCO disjointness (Theorems 2-4)
 *
 * 0 Admitted.
 *)

Require Import Coq.Lists.List.
Require Import Coq.Arith.Arith.
Require Import Coq.Bool.Bool.
Require Import Coq.ZArith.ZArith.
Require Import Lia.
Import ListNotations.

Open Scope Z_scope.


(* ================================================================ *)
(* A. CacheType and CacheKey model                                   *)
(* ================================================================ *)

Inductive CacheType : Type :=
  | CT_Source
  | CT_AST
  | CT_Bytecode
  | CT_Result.

Definition cache_type_eqb (a b : CacheType) : bool :=
  match a, b with
  | CT_Source, CT_Source => true
  | CT_AST, CT_AST => true
  | CT_Bytecode, CT_Bytecode => true
  | CT_Result, CT_Result => true
  | _, _ => false
  end.

Record CacheKey := mk_cache_key {
  ck_content : Z;       (* hash of source code *)
  ck_type : CacheType;
  ck_optimize : bool;
  ck_tco : bool;
}.

Definition bool_eqb (a b : bool) : bool :=
  match a, b with
  | true, true => true
  | false, false => true
  | _, _ => false
  end.

Definition cache_key_eqb (a b : CacheKey) : bool :=
  andb (andb (andb (Z.eqb (ck_content a) (ck_content b))
                    (cache_type_eqb (ck_type a) (ck_type b)))
             (bool_eqb (ck_optimize a) (ck_optimize b)))
       (bool_eqb (ck_tco a) (ck_tco b)).

(* Key to string: deterministic mapping *)
Definition cache_key_to_z (k : CacheKey) : Z :=
  let type_id := match ck_type k with
    | CT_Source => 0 | CT_AST => 1 | CT_Bytecode => 2 | CT_Result => 3
  end in
  let opt_bit := if ck_optimize k then 1 else 0 in
  let tco_bit := if ck_tco k then 1 else 0 in
  ck_content k * 16 + type_id * 4 + opt_bit * 2 + tco_bit.

Lemma cache_type_eqb_refl : forall t, cache_type_eqb t t = true.
Proof. destruct t; reflexivity. Qed.

Lemma cache_type_eqb_eq : forall a b, cache_type_eqb a b = true <-> a = b.
Proof.
  destruct a, b; simpl; split; intros; try reflexivity; try discriminate.
Qed.

Lemma bool_eqb_refl : forall b, bool_eqb b b = true.
Proof. destruct b; reflexivity. Qed.

Lemma bool_eqb_eq : forall a b, bool_eqb a b = true <-> a = b.
Proof. destruct a, b; simpl; split; intros; try reflexivity; discriminate. Qed.

Lemma cache_key_eqb_refl : forall k, cache_key_eqb k k = true.
Proof.
  intros. unfold cache_key_eqb.
  rewrite Z.eqb_refl, cache_type_eqb_refl, bool_eqb_refl, bool_eqb_refl.
  reflexivity.
Qed.

Lemma cache_key_eqb_eq : forall a b,
  cache_key_eqb a b = true <-> a = b.
Proof.
  intros [c1 t1 o1 tc1] [c2 t2 o2 tc2].
  split; intros H.
  - unfold cache_key_eqb in H.
    destruct o1, o2, tc1, tc2, t1, t2; simpl in H; try discriminate;
    rewrite ?andb_true_r in H;
    try discriminate;
    (destruct (Z.eqb c1 c2) eqn:Hc; simpl in H; try discriminate;
     apply Z.eqb_eq in Hc; subst; reflexivity).
  - inversion H. subst. apply cache_key_eqb_refl.
Qed.

(* Theorem 1: Same inputs produce same key encoding *)
Theorem cache_key_deterministic : forall k,
  cache_key_to_z k = cache_key_to_z k.
Proof. reflexivity. Qed.

(* Theorem 2: Different cache types produce different keys for same content *)
Theorem cache_key_type_disjoint : forall content opt tco t1 t2,
  t1 <> t2 ->
  cache_key_to_z (mk_cache_key content t1 opt tco) <>
  cache_key_to_z (mk_cache_key content t2 opt tco).
Proof.
  intros. unfold cache_key_to_z. simpl.
  destruct t1, t2; try contradiction; lia.
Qed.

(* Theorem 3: Different optimization flags produce different keys *)
Theorem cache_key_optimize_disjoint : forall content ct tco,
  cache_key_to_z (mk_cache_key content ct true tco) <>
  cache_key_to_z (mk_cache_key content ct false tco).
Proof.
  intros. unfold cache_key_to_z. simpl.
  destruct ct, tco; simpl; lia.
Qed.

(* Theorem 4: Different TCO flags produce different keys *)
Theorem cache_key_tco_disjoint : forall content ct opt,
  cache_key_to_z (mk_cache_key content ct opt true) <>
  cache_key_to_z (mk_cache_key content ct opt false).
Proof.
  intros. unfold cache_key_to_z. simpl.
  destruct ct, opt; simpl; lia.
Qed.

(* Theorem 5: Encoding is injective *)
Theorem cache_key_injective : forall a b,
  cache_key_to_z a = cache_key_to_z b -> a = b.
Proof.
  intros [c1 t1 o1 tc1] [c2 t2 o2 tc2].
  unfold cache_key_to_z. simpl.
  destruct t1, t2, o1, o2, tc1, tc2; simpl; intros; try lia;
  f_equal; lia.
Qed.
