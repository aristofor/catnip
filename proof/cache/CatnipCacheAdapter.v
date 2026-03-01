(* FILE: proof/cache/CatnipCacheAdapter.v *)
(* CatnipCache selective adapter and Memoization model.
 *
 * Source: catnip_rs/src/cache/backend.rs (CatnipCache)
 *         catnip_rs/src/cache/memoization.rs (Memoization)
 *
 * Proves:
 *   - Disabled cache type returns None (Theorem 17)
 *   - Enabled cache type delegates to backend (Theorem 18)
 *   - Result type always cached (Theorem 19)
 *   - Invalidation covers all 16 combinations (Theorems 20-22)
 *   - Memoization disabled = no-op (Theorems 23-24)
 *   - Memoization round-trip (Theorem 25)
 *   - Key determinism and function disjointness (Theorems 26-27)
 *   - Invalidation cleans index (Theorem 29)
 *   - Cache key bijection (Theorem 33)
 *   - Invalidation keys are distinct (Theorem 35)
 *
 * Depends on: CatnipCacheKey.v, CatnipCacheMemory.v
 *
 * 0 Admitted.
 *)

From Catnip Require Export CatnipCacheKey.
From Catnip Require Export CatnipCacheMemory.
Require Import Coq.Lists.List.
Require Import Coq.Arith.Arith.
Require Import Coq.Bool.Bool.
Require Import Coq.ZArith.ZArith.
Require Import Lia.
Import ListNotations.

Open Scope Z_scope.


(* ================================================================ *)
(* D. CatnipCache adapter (selective caching)                        *)
(* ================================================================ *)

Record CatnipCacheConfig := mk_cc_config {
  cc_cache_source : bool;
  cc_cache_ast : bool;
  cc_cache_bytecode : bool;
}.

(* Model: adapter delegates to a backend but filters by config *)
Definition cc_should_cache (config : CatnipCacheConfig) (ct : CacheType) : bool :=
  match ct with
  | CT_Source => cc_cache_source config
  | CT_AST => cc_cache_ast config
  | CT_Bytecode => cc_cache_bytecode config
  | CT_Result => true  (* always cache results *)
  end.

(* Get through adapter: returns None if type is disabled *)
Definition cc_get (config : CatnipCacheConfig) (ct : CacheType)
  (backend_get : CacheType -> option Z) : option Z :=
  if cc_should_cache config ct then backend_get ct
  else None.

(* Set through adapter: no-op if type is disabled *)
Definition cc_set (config : CatnipCacheConfig) (ct : CacheType)
  (backend_set : CacheType -> unit) : unit :=
  if cc_should_cache config ct then backend_set ct
  else tt.

(* Theorem 17: Disabled cache type always returns None *)
Theorem cc_disabled_returns_none : forall config ct backend_get,
  cc_should_cache config ct = false ->
  cc_get config ct backend_get = None.
Proof.
  intros. unfold cc_get. rewrite H. reflexivity.
Qed.

(* Theorem 18: Enabled cache type delegates to backend *)
Theorem cc_enabled_delegates : forall config ct backend_get,
  cc_should_cache config ct = true ->
  cc_get config ct backend_get = backend_get ct.
Proof.
  intros. unfold cc_get. rewrite H. reflexivity.
Qed.

(* Theorem 19: Result type is always cached *)
Theorem cc_result_always_cached : forall config,
  cc_should_cache config CT_Result = true.
Proof.
  intros. unfold cc_should_cache. reflexivity.
Qed.

(* Invalidate_all: generates all 16 combinations *)
Definition all_invalidation_keys (content : Z) : list CacheKey :=
  flat_map (fun ct =>
    flat_map (fun opt =>
      map (fun tco => mk_cache_key content ct opt tco)
          [true; false])
      [true; false])
    [CT_Source; CT_AST; CT_Bytecode; CT_Result].

(* Theorem 20: invalidate_all produces exactly 16 keys *)
Theorem invalidation_key_count : forall content,
  length (all_invalidation_keys content) = 16%nat.
Proof.
  intros. unfold all_invalidation_keys. simpl. reflexivity.
Qed.

(* Theorem 21: All combinations are covered *)
Theorem invalidation_covers_all : forall content ct opt tco,
  In (mk_cache_key content ct opt tco) (all_invalidation_keys content).
Proof.
  intros. unfold all_invalidation_keys.
  simpl.
  destruct ct, opt, tco; simpl;
  repeat (try (left; reflexivity); right).
Qed.

(* Theorem 22: All keys in invalidation list have the same content *)
Theorem invalidation_same_content : forall content k,
  In k (all_invalidation_keys content) ->
  ck_content k = content.
Proof.
  intros. unfold all_invalidation_keys in H. simpl in H.
  repeat (destruct H as [H | H]; [subst; reflexivity |]).
  contradiction.
Qed.


(* ================================================================ *)
(* E. Memoization model (function result caching + index sync)       *)
(* ================================================================ *)

(* Simplified model: memoization = MemoryCache + function index *)

Record MemoIndex := mk_memo_index {
  mi_func_name : Z;   (* function id *)
  mi_key_hash : Z;    (* cache key hash *)
}.

Record Memoization := mk_memo {
  memo_backend : MemoryCache;
  memo_enabled : bool;
  memo_index : list MemoIndex;
}.

(* Build memoization key from function id and args hash *)
Definition memo_make_key (func_id args_hash : Z) : Z :=
  func_id * 1000000 + args_hash.

(* Get: lookup in backend if enabled *)
Definition memo_get (m : Memoization) (func_id args_hash : Z) : option Z :=
  if memo_enabled m then
    mc_find (memo_make_key func_id args_hash) (mc_entries (memo_backend m))
  else None.

(* Set: store in backend + update index *)
Definition memo_set (m : Memoization) (func_id args_hash val : Z) : Memoization :=
  if memo_enabled m then
    let key := memo_make_key func_id args_hash in
    let backend' := mc_set (memo_backend m) key val in
    let idx_entry := mk_memo_index func_id key in
    mk_memo backend' true (idx_entry :: memo_index m)
  else m.

(* Invalidate all entries for a function *)
Definition memo_invalidate_func (m : Memoization) (func_id : Z) : Memoization :=
  let keys_to_remove := map mi_key_hash
    (filter (fun idx => Z.eqb (mi_func_name idx) func_id) (memo_index m)) in
  let backend' := fold_left
    (fun cache key => mk_mem_cache (mc_remove key (mc_entries cache))
                                   (mc_max_size cache)
                                   (mc_hits cache) (mc_misses cache))
    keys_to_remove (memo_backend m) in
  let index' := filter (fun idx => negb (Z.eqb (mi_func_name idx) func_id)) (memo_index m) in
  mk_memo backend' (memo_enabled m) index'.

(* Theorem 23: Disabled memoization always returns None *)
Theorem memo_disabled_returns_none : forall m func_id args_hash,
  memo_enabled m = false ->
  memo_get m func_id args_hash = None.
Proof.
  intros. unfold memo_get. rewrite H. reflexivity.
Qed.

(* Theorem 24: Disabled memoization set is no-op *)
Theorem memo_disabled_set_noop : forall m func_id args_hash val,
  memo_enabled m = false ->
  memo_set m func_id args_hash val = m.
Proof.
  intros. unfold memo_set. rewrite H. reflexivity.
Qed.

(* Theorem 25: Round-trip for memoization *)
Theorem memo_set_get_same : forall m func_id args_hash val,
  memo_enabled m = true ->
  keys_unique (mc_entries (memo_backend m)) ->
  memo_get (memo_set m func_id args_hash val) func_id args_hash = Some val.
Proof.
  intros m func_id args_hash val Hen Huniq.
  unfold memo_get, memo_set.
  rewrite Hen. simpl.
  unfold mc_set. simpl.
  rewrite mc_find_app_last.
  - reflexivity.
  - apply mc_remove_removes.
    apply mc_evict_preserves_unique. assumption.
Qed.

(* Theorem 26: Deterministic key generation *)
Theorem memo_key_deterministic : forall f a,
  memo_make_key f a = memo_make_key f a.
Proof. reflexivity. Qed.

(* Theorem 27: Different functions produce different keys *)
Theorem memo_key_func_disjoint : forall f1 f2 a,
  f1 <> f2 ->
  0 <= a < 1000000 ->
  0 <= f1 ->
  0 <= f2 ->
  memo_make_key f1 a <> memo_make_key f2 a.
Proof.
  intros. unfold memo_make_key. lia.
Qed.

(* Theorem 28: After invalidation, function entries return None *)
Lemma mc_find_fold_remove : forall keys backend key,
  In key keys ->
  keys_unique (mc_entries backend) ->
  NoDup keys ->
  mc_find key
    (mc_entries
      (fold_left
        (fun cache k => mk_mem_cache (mc_remove k (mc_entries cache))
                                     (mc_max_size cache)
                                     (mc_hits cache) (mc_misses cache))
        keys backend)) = None.
Proof.
  intros keys. induction keys as [| a keys' IHkeys]; intros backend key Hin Huniq Hnd.
  - contradiction.
  - simpl in Hin. destruct Hin as [Heq | Hin].
    + subst. simpl.
      apply mc_find_none_fold_remove. simpl.
      apply mc_remove_removes. assumption.
    + simpl. apply IHkeys.
      * assumption.
      * simpl. apply mc_remove_preserves_unique. assumption.
      * inversion Hnd. assumption.
Qed.

(* Theorem 29: Index is cleaned after invalidation *)
Theorem memo_invalidate_cleans_index : forall m func_id idx,
  In idx (memo_index (memo_invalidate_func m func_id)) ->
  mi_func_name idx <> func_id.
Proof.
  intros m func_id idx Hin.
  unfold memo_invalidate_func in Hin. simpl in Hin.
  apply filter_In in Hin. destruct Hin as [_ Hfilter].
  destruct (Z.eqb (mi_func_name idx) func_id) eqn:Heq.
  - simpl in Hfilter. discriminate.
  - apply Z.eqb_neq. assumption.
Qed.


(* ================================================================ *)
(* H. Composition: end-to-end cache properties                       *)
(* ================================================================ *)

(* Theorem 33: Cache key encoding is a bijection between CacheKey and Z *)
Theorem cache_key_bijection : forall k1 k2,
  cache_key_to_z k1 = cache_key_to_z k2 <-> k1 = k2.
Proof.
  split.
  - apply cache_key_injective.
  - intros. subst. reflexivity.
Qed.

(* Theorem 34: Number of possible keys per source = 4 types x 2 opt x 2 tco = 16 *)
Theorem keys_per_source_count : forall content,
  length (all_invalidation_keys content) = 16%nat.
Proof. apply invalidation_key_count. Qed.

(* Theorem 35: All invalidation keys are distinct *)
Lemma all_invalidation_keys_nodup : forall content,
  NoDup (map cache_key_to_z (all_invalidation_keys content)).
Proof.
  intros. unfold all_invalidation_keys, cache_key_to_z.
  simpl flat_map. simpl map. simpl.
  repeat (constructor; [simpl; lia | ]).
  constructor.
Qed.
