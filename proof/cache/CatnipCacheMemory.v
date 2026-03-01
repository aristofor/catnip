(* FILE: proof/cache/CatnipCacheMemory.v *)
(* MemoryCache model: FIFO eviction, key uniqueness, round-trip,
 * size invariant, FIFO order, counter monotonicity.
 *
 * Source: catnip_rs/src/cache/mod.rs (MemoryCache, IndexMap FIFO)
 *
 * Proves:
 *   - mc_set preserves key uniqueness (Theorem 6)
 *   - mc_set/mc_get round-trip (Theorem 7)
 *   - Size invariant under mc_set (Theorem 9)
 *   - Clear empties cache (Theorems 10-11)
 *   - Hit/miss counter semantics (Theorems 11-12)
 *   - FIFO eviction removes oldest entry (Theorem 30)
 *   - Counter total = number of get calls (Theorem 36)
 *   - Set preserves counters (Theorem 37)
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
(* B. MemoryCache model (FIFO with max_size)                         *)
(*                                                                    *)
(* IndexMap in Rust = list of (key, value) pairs preserving           *)
(* insertion order.                                                   *)
(* ================================================================ *)

Definition Entry := (Z * Z)%type.  (* key, value *)

Record MemoryCache := mk_mem_cache {
  mc_entries : list Entry;
  mc_max_size : option nat;
  mc_hits : nat;
  mc_misses : nat;
}.

Definition mc_empty (max_size : option nat) : MemoryCache :=
  mk_mem_cache [] max_size 0 0.

(* Lookup by key *)
Fixpoint mc_find (key : Z) (entries : list Entry) : option Z :=
  match entries with
  | [] => None
  | (k, v) :: rest => if Z.eqb k key then Some v else mc_find key rest
  end.

(* Remove by key *)
Fixpoint mc_remove (key : Z) (entries : list Entry) : list Entry :=
  match entries with
  | [] => []
  | (k, v) :: rest =>
    if Z.eqb k key then rest else (k, v) :: mc_remove key rest
  end.

(* Check membership *)
Fixpoint mc_mem (key : Z) (entries : list Entry) : bool :=
  match entries with
  | [] => false
  | (k, _) :: rest => if Z.eqb k key then true else mc_mem key rest
  end.

(* FIFO eviction: remove first element if at capacity *)
Definition mc_evict (entries : list Entry) (max_size : option nat) (key : Z)
  : list Entry :=
  match max_size with
  | None => entries
  | Some ms =>
    if (Nat.leb ms (length entries)) && negb (mc_mem key entries)
    then tl entries
    else entries
  end.

(* Set operation: evict if needed, then insert/update *)
Definition mc_set (cache : MemoryCache) (key val : Z) : MemoryCache :=
  let entries' := mc_evict (mc_entries cache) (mc_max_size cache) key in
  let cleaned := mc_remove key entries' in
  mk_mem_cache (cleaned ++ [(key, val)]) (mc_max_size cache)
               (mc_hits cache) (mc_misses cache).

(* Get operation: lookup + update hit/miss counters *)
Definition mc_get (cache : MemoryCache) (key : Z) : (option Z * MemoryCache) :=
  match mc_find key (mc_entries cache) with
  | Some v => (Some v, mk_mem_cache (mc_entries cache) (mc_max_size cache)
                                    (S (mc_hits cache)) (mc_misses cache))
  | None => (None, mk_mem_cache (mc_entries cache) (mc_max_size cache)
                                (mc_hits cache) (S (mc_misses cache)))
  end.

Definition mc_clear (cache : MemoryCache) : MemoryCache :=
  mk_mem_cache [] (mc_max_size cache) 0 0.

Definition mc_size (cache : MemoryCache) : nat := length (mc_entries cache).

(* --- Helper lemmas --- *)

Lemma mc_find_app_last : forall key val entries,
  mc_find key entries = None ->
  mc_find key (entries ++ [(key, val)]) = Some val.
Proof.
  intros key val. induction entries as [| [k v] rest IH].
  - simpl. intros _. rewrite Z.eqb_refl. reflexivity.
  - simpl. destruct (Z.eqb k key) eqn:Hk.
    + intros H. discriminate.
    + intros H. apply IH. assumption.
Qed.

(* Key uniqueness invariant *)
Definition keys_of (entries : list Entry) : list Z :=
  map fst entries.

Definition keys_unique (entries : list Entry) : Prop :=
  NoDup (keys_of entries).

Lemma mc_find_not_in : forall key entries,
  ~ In key (keys_of entries) ->
  mc_find key entries = None.
Proof.
  intros key entries Hnotin.
  induction entries as [| [k v] rest IH].
  - reflexivity.
  - simpl. destruct (Z.eqb k key) eqn:Hk.
    + apply Z.eqb_eq in Hk. subst.
      exfalso. apply Hnotin. simpl. left. reflexivity.
    + apply IH. intro Hin. apply Hnotin. simpl. right. assumption.
Qed.

Lemma mc_remove_removes : forall key entries,
  keys_unique entries ->
  mc_find key (mc_remove key entries) = None.
Proof.
  intros key entries Huniq.
  induction entries as [| [k v] rest IH].
  - reflexivity.
  - simpl. destruct (Z.eqb k key) eqn:Hk.
    + apply Z.eqb_eq in Hk. subst.
      unfold keys_unique in Huniq. simpl in Huniq.
      inversion Huniq as [| x xs Hnotin Hnd']. subst.
      apply mc_find_not_in. assumption.
    + simpl. rewrite Hk. apply IH.
      unfold keys_unique in *. simpl in Huniq.
      inversion Huniq. assumption.
Qed.

Lemma mc_remove_keys_subset : forall key k entries,
  In k (keys_of (mc_remove key entries)) -> In k (keys_of entries).
Proof.
  intros key k entries.
  induction entries as [| [k' v'] rest IH].
  - simpl. auto.
  - simpl. destruct (Z.eqb k' key) eqn:Hk.
    + intro Hin. right. assumption.
    + simpl. intros [Heq | Hin].
      * left. assumption.
      * right. apply IH. assumption.
Qed.

Lemma mc_remove_preserves_unique : forall key entries,
  keys_unique entries -> keys_unique (mc_remove key entries).
Proof.
  intros key entries Huniq.
  induction entries as [| [k v] rest IH].
  - simpl. constructor.
  - simpl. destruct (Z.eqb k key) eqn:Hk.
    + unfold keys_unique in Huniq. simpl in Huniq. inversion Huniq. assumption.
    + unfold keys_unique. simpl.
      constructor.
      * unfold keys_unique in Huniq. simpl in Huniq.
        inversion Huniq as [| x xs Hnotin Hnd]. subst.
        intro Hin. apply Hnotin.
        apply mc_remove_keys_subset in Hin. assumption.
      * apply IH. unfold keys_unique in *. simpl in Huniq.
        inversion Huniq. assumption.
Qed.

Lemma mc_find_none_remove : forall key k entries,
  mc_find key entries = None -> mc_find key (mc_remove k entries) = None.
Proof.
  intros key k entries Hnone.
  induction entries as [| [k' v'] rest IH].
  - reflexivity.
  - simpl. destruct (Z.eqb k' k) eqn:Hk.
    + simpl in Hnone. destruct (Z.eqb k' key) eqn:Hkey; [discriminate | ].
      exact Hnone.
    + simpl. simpl in Hnone.
      destruct (Z.eqb k' key) eqn:Hkey; [discriminate | ].
      apply IH. exact Hnone.
Qed.

Lemma mc_find_none_fold_remove : forall key ks backend,
  mc_find key (mc_entries backend) = None ->
  mc_find key (mc_entries
    (fold_left
      (fun cache k => mk_mem_cache (mc_remove k (mc_entries cache))
                                   (mc_max_size cache)
                                   (mc_hits cache) (mc_misses cache))
      ks backend)) = None.
Proof.
  intros key ks. revert key.
  induction ks as [| k' rest IH]; intros key backend Hnone.
  - exact Hnone.
  - simpl. apply IH. simpl. apply mc_find_none_remove. exact Hnone.
Qed.

Lemma mc_find_not_mem : forall key entries,
  mc_mem key entries = false -> mc_find key entries = None.
Proof.
  intros. induction entries as [| [k v] rest IH].
  - reflexivity.
  - simpl in *. destruct (Z.eqb k key); [discriminate | apply IH; assumption].
Qed.

Lemma app_preserves_unique_new : forall key val entries,
  keys_unique entries ->
  mc_find key entries = None ->
  keys_unique (entries ++ [(key, val)]).
Proof.
  intros key val entries Huniq Hfind.
  induction entries as [| [k v] rest IH].
  - simpl. unfold keys_unique. simpl.
    constructor. { intro H. contradiction. } constructor.
  - unfold keys_unique in *. simpl in *.
    destruct (Z.eqb k key) eqn:Hk.
    + discriminate.
    + inversion Huniq as [| x xs Hnotin_k Hnd_rest]. subst.
      constructor.
      * intro Hin. unfold keys_of in Hin. rewrite map_app in Hin. simpl in Hin.
        apply in_app_iff in Hin. destruct Hin as [Hin | Hin].
        -- apply Hnotin_k. assumption.
        -- simpl in Hin. destruct Hin as [Hin | Hin].
           ++ subst. rewrite Z.eqb_refl in Hk. discriminate.
           ++ contradiction.
      * apply IH.
        -- assumption.
        -- assumption.
Qed.

(* Eviction preserves uniqueness *)
Lemma mc_evict_preserves_unique : forall entries max_size key,
  keys_unique entries ->
  keys_unique (mc_evict entries max_size key).
Proof.
  intros entries max_size key Huniq.
  unfold mc_evict.
  destruct max_size.
  - destruct (andb (Nat.leb n (length entries)) (negb (mc_mem key entries))).
    + destruct entries; [constructor | inversion Huniq; assumption].
    + assumption.
  - assumption.
Qed.

(* Theorem 6: mc_set preserves key uniqueness *)
Theorem mc_set_preserves_unique : forall cache key val,
  keys_unique (mc_entries cache) ->
  keys_unique (mc_entries (mc_set cache key val)).
Proof.
  intros cache key val Huniq.
  unfold mc_set. simpl.
  apply app_preserves_unique_new.
  - apply mc_remove_preserves_unique.
    apply mc_evict_preserves_unique. assumption.
  - apply mc_remove_removes.
    apply mc_evict_preserves_unique. assumption.
Qed.

(* Theorem 7: Round-trip - set then get returns the value *)
Theorem mc_set_get_same : forall cache key val,
  keys_unique (mc_entries cache) ->
  fst (mc_get (mc_set cache key val) key) = Some val.
Proof.
  intros cache key val Huniq.
  unfold mc_get, mc_set. simpl.
  rewrite mc_find_app_last.
  - reflexivity.
  - apply mc_remove_removes.
    apply mc_evict_preserves_unique. assumption.
Qed.

(* Theorem 8: Get for different key is unaffected by set *)
Lemma mc_find_remove_other : forall key1 key2 entries,
  key1 <> key2 ->
  mc_find key1 (mc_remove key2 entries) = mc_find key1 entries.
Proof.
  intros key1 key2 entries Hneq.
  induction entries as [| [k v] rest IH].
  - reflexivity.
  - simpl. destruct (Z.eqb k key2) eqn:Hk2.
    + apply Z.eqb_eq in Hk2. subst.
      simpl. destruct (Z.eqb key2 key1) eqn:Hk1.
      * apply Z.eqb_eq in Hk1. symmetry in Hk1. contradiction.
      * reflexivity.
    + simpl. destruct (Z.eqb k key1); [reflexivity | apply IH].
Qed.

Lemma mc_find_app_other : forall key1 key2 val entries,
  key1 <> key2 ->
  mc_find key1 (entries ++ [(key2, val)]) = mc_find key1 entries.
Proof.
  intros. induction entries as [| [k v] rest IH].
  - simpl. destruct (Z.eqb key2 key1) eqn:Hk.
    + apply Z.eqb_eq in Hk. symmetry in Hk. contradiction.
    + reflexivity.
  - simpl. destruct (Z.eqb k key1); [reflexivity | apply IH].
Qed.

(* Theorem 9: Size invariant - cache never exceeds max_size *)
Lemma mc_remove_length : forall key entries,
  (length (mc_remove key entries) <= length entries)%nat.
Proof.
  intros. induction entries as [| [k v] rest IH].
  - simpl. lia.
  - simpl. destruct (Z.eqb k key).
    + lia.
    + simpl. lia.
Qed.

Lemma mc_remove_length_mem : forall key entries,
  keys_unique entries ->
  mc_mem key entries = true ->
  length (mc_remove key entries) = Nat.pred (length entries).
Proof.
  intros key entries Huniq Hmem.
  induction entries as [| [k v] rest IH].
  - simpl in Hmem. discriminate.
  - simpl in *. destruct (Z.eqb k key) eqn:Hk.
    + lia.
    + simpl.
      assert (IHr := IH).
      specialize (IHr ltac:(unfold keys_unique in *; simpl in Huniq; inversion Huniq; assumption) Hmem).
      destruct rest as [| [k' v'] rest'].
      * simpl in Hmem. discriminate.
      * simpl in IHr. simpl. lia.
Qed.

Lemma tl_length : forall {A : Type} (l : list A),
  (length (tl l) <= length l)%nat.
Proof.
  intros. destruct l; simpl; lia.
Qed.

Lemma tl_length_pos : forall {A : Type} (l : list A),
  l <> [] -> length (tl l) = Nat.pred (length l).
Proof.
  intros. destruct l; [contradiction | simpl; lia].
Qed.

Theorem mc_set_size_invariant : forall cache key val ms,
  keys_unique (mc_entries cache) ->
  mc_max_size cache = Some ms ->
  (ms > 0)%nat ->
  (mc_size cache <= ms)%nat ->
  (mc_size (mc_set cache key val) <= ms)%nat.
Proof.
  intros cache key val ms Huniq Hmax Hms Hsize.
  unfold mc_size, mc_set in *. simpl.
  rewrite length_app. simpl.
  unfold mc_evict. rewrite Hmax.
  destruct (mc_mem key (mc_entries cache)) eqn:Hmem;
  destruct (Nat.leb ms (length (mc_entries cache))) eqn:Hcap;
  simpl;
  pose proof (mc_remove_length key (mc_entries cache));
  try (apply Nat.leb_le in Hcap);
  try (apply Nat.leb_gt in Hcap).
  - (* key in cache, at capacity *)
    rewrite mc_remove_length_mem; [|assumption|assumption].
    destruct (mc_entries cache); simpl in *; lia.
  - (* key in cache, under capacity *)
    rewrite mc_remove_length_mem; [|assumption|assumption].
    destruct (mc_entries cache); simpl in *; lia.
  - (* key not in cache, at capacity: evict first *)
    destruct (mc_entries cache) eqn:He; simpl in *.
    + lia.
    + pose proof (mc_remove_length key l). lia.
  - (* key not in cache, under capacity *)
    lia.
Qed.

(* Theorem 10: Clear empties the cache *)
Theorem mc_clear_empty : forall cache,
  mc_size (mc_clear cache) = 0%nat.
Proof.
  intros. unfold mc_size, mc_clear. simpl. reflexivity.
Qed.

Theorem mc_clear_get_none : forall cache key,
  fst (mc_get (mc_clear cache) key) = None.
Proof.
  intros. unfold mc_get, mc_clear. simpl. reflexivity.
Qed.

(* Theorem 11: Hit counter increases on cache hit *)
Theorem mc_get_hit_increments : forall cache key v cache',
  mc_find key (mc_entries cache) = Some v ->
  mc_get cache key = (Some v, cache') ->
  mc_hits cache' = S (mc_hits cache).
Proof.
  intros cache key v cache' Hfind Hget.
  unfold mc_get in Hget. rewrite Hfind in Hget.
  inversion Hget. reflexivity.
Qed.

(* Theorem 12: Miss counter increases on cache miss *)
Theorem mc_get_miss_increments : forall cache key cache',
  mc_find key (mc_entries cache) = None ->
  mc_get cache key = (None, cache') ->
  mc_misses cache' = S (mc_misses cache).
Proof.
  intros cache key cache' Hfind Hget.
  unfold mc_get in Hget. rewrite Hfind in Hget.
  inversion Hget. reflexivity.
Qed.


(* ================================================================ *)
(* F. FIFO order property                                            *)
(*                                                                    *)
(* mc_set maintains insertion order and FIFO eviction removes the    *)
(* oldest entry.                                                      *)
(* ================================================================ *)

(* The first entry in the list is the oldest *)
Definition mc_oldest (cache : MemoryCache) : option Z :=
  match mc_entries cache with
  | [] => None
  | (k, _) :: _ => Some k
  end.

(* Theorem 30: FIFO eviction removes the oldest entry *)
Theorem mc_fifo_evicts_oldest : forall cache key val ms oldest_key oldest_val rest,
  mc_max_size cache = Some ms ->
  mc_entries cache = (oldest_key, oldest_val) :: rest ->
  keys_unique (mc_entries cache) ->
  (length (mc_entries cache) >= ms)%nat ->
  mc_mem key (mc_entries cache) = false ->
  mc_find oldest_key (mc_entries (mc_set cache key val)) = None.
Proof.
  intros cache key val ms oldest_key oldest_val rest Hmax Hentries Huniq Hlen Hmem.
  rewrite Hentries in Hmem. simpl in Hmem.
  apply Bool.orb_false_iff in Hmem. destruct Hmem as [Hok_neq Hmem_rest].
  rewrite Hentries in Huniq. unfold keys_unique in Huniq.
  simpl in Huniq. inversion Huniq as [| x xs Hnotin Hnd]. subst.
  rewrite Hentries in Hlen. simpl in Hlen.
  (* Prove via sufficient conditions *)
  assert (Hmem_full : mc_mem key ((oldest_key, oldest_val) :: rest) = false).
  { simpl. rewrite Hok_neq. exact Hmem_rest. }
  (* Prove the result on the entries directly *)
  assert (Hresult : mc_entries (mc_set cache key val) =
                    mc_remove key rest ++ [(key, val)]).
  { unfold mc_set. rewrite Hentries.
    assert (Hevict : mc_evict ((oldest_key, oldest_val) :: rest) (mc_max_size cache) key
                     = rest).
    { unfold mc_evict. rewrite Hmax.
      rewrite Hmem_full.
      change (negb false) with true. rewrite Bool.andb_true_r.
      change (length ((oldest_key, oldest_val) :: rest)) with (S (length rest)).
      assert (Hcap : Nat.leb ms (S (length rest)) = true) by (apply Nat.leb_le; exact Hlen).
      rewrite Hcap.
      change (tl ((oldest_key, oldest_val) :: rest)) with rest.
      reflexivity. }
    rewrite Hevict. reflexivity. }
  rewrite Hresult.
  rewrite mc_find_app_other.
  - apply mc_find_none_remove.
    apply mc_find_not_in. exact Hnotin.
  - apply Z.eqb_neq. exact Hok_neq.
Qed.


(* ================================================================ *)
(* I. Monotonicity of counters                                       *)
(* ================================================================ *)

(* Theorem 36: hits + misses = total number of get calls *)
Theorem mc_get_counter_total : forall cache key,
  let (_, cache') := mc_get cache key in
  (mc_hits cache' + mc_misses cache' = S (mc_hits cache + mc_misses cache))%nat.
Proof.
  intros. unfold mc_get.
  destruct (mc_find key (mc_entries cache)); simpl; lia.
Qed.

(* Theorem 37: Set does not change counters *)
Theorem mc_set_preserves_counters : forall cache key val,
  mc_hits (mc_set cache key val) = mc_hits cache /\
  mc_misses (mc_set cache key val) = mc_misses cache.
Proof.
  intros. unfold mc_set. simpl. split; reflexivity.
Qed.

(* Theorem 32: Empty cache has 0% hit rate *)
Theorem empty_cache_zero_hits : forall ms,
  mc_hits (mc_empty ms) = 0%nat /\ mc_misses (mc_empty ms) = 0%nat.
Proof.
  intros. split; reflexivity.
Qed.
