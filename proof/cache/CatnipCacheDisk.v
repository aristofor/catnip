(* FILE: proof/cache/CatnipCacheDisk.v *)
(* DiskCache model: LRU eviction, TTL enforcement, atomic writes.
 *
 * Source: catnip_rs/src/cache/disk.rs (DiskCache, LRU, TTL, atomic)
 *
 * Proves:
 *   - TTL enforcement: expired entries never returned (Theorem 13)
 *   - Fresh entries returned correctly (Theorem 14)
 *   - Prune removes all expired entries (Theorem 15)
 *   - LRU eviction respects size limit (Theorem 16)
 *   - find_min_access correctness (minimality + membership)
 *   - Atomic write: no partial state (Theorems 38-40)
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
(* C. DiskCache model (LRU + TTL)                                    *)
(* ================================================================ *)

Record DiskEntry := mk_disk_entry {
  de_key : Z;
  de_value : Z;
  de_size_bytes : Z;
  de_created_at : Z;
  de_accessed_at : Z;
}.

Record DiskCache := mk_disk_cache {
  dc_entries : list DiskEntry;
  dc_max_size_bytes : option Z;
  dc_ttl_seconds : option Z;
  dc_hits : nat;
  dc_misses : nat;
}.

Definition dc_total_size (entries : list DiskEntry) : Z :=
  fold_right (fun e acc => de_size_bytes e + acc) 0 entries.

Definition dc_is_expired (e : DiskEntry) (now : Z) (ttl : option Z) : bool :=
  match ttl with
  | None => false
  | Some t => Z.leb t (now - de_created_at e)
  end.

(* Phase 1 of prune: remove expired entries *)
Definition dc_remove_expired (entries : list DiskEntry) (now : Z) (ttl : option Z)
  : list DiskEntry :=
  filter (fun e => negb (dc_is_expired e now ttl)) entries.

(* Sort by accessed_at for LRU *)
(* We model "sort and take" as: remove entries with smallest accessed_at until under limit *)
Fixpoint dc_find_min_access (entries : list DiskEntry) : option DiskEntry :=
  match entries with
  | [] => None
  | [e] => Some e
  | e :: rest =>
    match dc_find_min_access rest with
    | None => Some e
    | Some m => if Z.leb (de_accessed_at e) (de_accessed_at m)
                then Some e else Some m
    end
  end.

Fixpoint dc_remove_entry (key : Z) (entries : list DiskEntry) : list DiskEntry :=
  match entries with
  | [] => []
  | e :: rest =>
    if Z.eqb (de_key e) key then rest
    else e :: dc_remove_entry key rest
  end.

(* Phase 2: LRU eviction until total size <= max *)
Fixpoint dc_lru_evict (entries : list DiskEntry) (max_bytes : Z) (fuel : nat)
  : list DiskEntry :=
  match fuel with
  | O => entries
  | S fuel' =>
    if Z.leb (dc_total_size entries) max_bytes then entries
    else match dc_find_min_access entries with
         | None => entries
         | Some victim => dc_lru_evict (dc_remove_entry (de_key victim) entries) max_bytes fuel'
         end
  end.

(* Full prune: TTL then LRU *)
Definition dc_prune (cache : DiskCache) (now : Z) : list DiskEntry :=
  let after_ttl := dc_remove_expired (dc_entries cache) now (dc_ttl_seconds cache) in
  match dc_max_size_bytes cache with
  | None => after_ttl
  | Some max => dc_lru_evict after_ttl max (length after_ttl)
  end.

(* Get with TTL check *)
Definition dc_get (cache : DiskCache) (key now : Z) : option Z :=
  match find (fun e => Z.eqb (de_key e) key) (dc_entries cache) with
  | None => None
  | Some e =>
    if dc_is_expired e now (dc_ttl_seconds cache) then None
    else Some (de_value e)
  end.

(* Theorem 13: Expired entries are never returned by get *)
Theorem dc_get_ttl_enforcement : forall cache key now ttl e,
  dc_ttl_seconds cache = Some ttl ->
  find (fun e' => Z.eqb (de_key e') key) (dc_entries cache) = Some e ->
  ttl <= now - de_created_at e ->
  dc_get cache key now = None.
Proof.
  intros cache key now ttl e Httl Hfind Hage.
  unfold dc_get. rewrite Hfind.
  unfold dc_is_expired. rewrite Httl.
  apply Z.leb_le in Hage. rewrite Hage.
  reflexivity.
Qed.

(* Theorem 14: Non-expired entries are returned by get *)
Theorem dc_get_fresh_returns : forall cache key now ttl e,
  dc_ttl_seconds cache = Some ttl ->
  find (fun e' => Z.eqb (de_key e') key) (dc_entries cache) = Some e ->
  now - de_created_at e < ttl ->
  dc_get cache key now = Some (de_value e).
Proof.
  intros cache key now ttl e Httl Hfind Hfresh.
  unfold dc_get. rewrite Hfind.
  unfold dc_is_expired. rewrite Httl.
  apply Z.leb_gt in Hfresh. rewrite Hfresh.
  reflexivity.
Qed.

(* Theorem 15: Prune removes all expired entries *)
Lemma filter_all_satisfy : forall {A} (f : A -> bool) (l : list A),
  forall x, In x (filter f l) -> f x = true.
Proof.
  intros A f l. induction l; intros x Hin.
  - simpl in Hin. contradiction.
  - simpl in Hin. destruct (f a) eqn:Ha.
    + destruct Hin. { subst. assumption. } apply IHl. assumption.
    + apply IHl. assumption.
Qed.

Theorem dc_prune_removes_expired : forall cache now e,
  In e (dc_remove_expired (dc_entries cache) now (dc_ttl_seconds cache)) ->
  dc_is_expired e now (dc_ttl_seconds cache) = false.
Proof.
  intros.
  unfold dc_remove_expired in H.
  apply filter_all_satisfy in H.
  destruct (dc_is_expired e now (dc_ttl_seconds cache)); simpl in H;
  [discriminate | reflexivity].
Qed.

(* Helper: total_size is nonneg when all entries have nonneg sizes *)
Lemma dc_total_size_nonneg : forall entries,
  (forall e, In e entries -> de_size_bytes e >= 0) ->
  dc_total_size entries >= 0.
Proof.
  intros entries Hpos.
  induction entries as [| a rest IH]; simpl; [lia |].
  assert (de_size_bytes a >= 0) by (apply Hpos; left; reflexivity).
  assert (forall e, In e rest -> de_size_bytes e >= 0) by
    (intros; apply Hpos; right; assumption).
  specialize (IH H0). lia.
Qed.

Lemma dc_remove_entry_subset : forall key e entries,
  In e (dc_remove_entry key entries) -> In e entries.
Proof.
  intros key e entries. induction entries as [| a rest IH]; simpl; [auto |].
  destruct (Z.eqb (de_key a) key).
  - intros. right. assumption.
  - intros [Heq | Hin]; [left; assumption | right; apply IH; assumption].
Qed.

Lemma dc_remove_entry_length : forall key entries,
  (length (dc_remove_entry key entries) <= length entries)%nat.
Proof.
  intros. induction entries as [| a rest IH]; simpl; [lia |].
  destruct (Z.eqb (de_key a) key); simpl; lia.
Qed.

Lemma dc_remove_entry_length_in : forall e entries,
  In e entries ->
  (length (dc_remove_entry (de_key e) entries) < length entries)%nat.
Proof.
  intros e entries Hin.
  induction entries as [| a rest IH]; [contradiction |].
  simpl. destruct (Z.eqb (de_key a) (de_key e)) eqn:Hk.
  - simpl. lia.
  - simpl. apply -> Nat.succ_lt_mono. apply IH.
    destruct Hin as [Heq | Hin]; [| assumption].
    subst. rewrite Z.eqb_refl in Hk. discriminate.
Qed.

(* Helper: dc_remove_entry preserves non-negative sizes *)
Lemma dc_remove_total_size_nonneg : forall key entries,
  (forall e, In e entries -> de_size_bytes e >= 0) ->
  dc_total_size (dc_remove_entry key entries) >= 0.
Proof.
  intros. apply dc_total_size_nonneg.
  intros. apply H. apply dc_remove_entry_subset in H0. assumption.
Qed.

(* dc_find_min_access returns Some for non-empty lists *)
Lemma dc_find_min_access_some : forall entries,
  entries <> [] -> exists e, dc_find_min_access entries = Some e.
Proof.
  induction entries as [| a [| b rest'] IH]; intro Hne.
  - contradiction.
  - simpl. eauto.
  - assert (Hne2 : b :: rest' <> @nil DiskEntry) by discriminate.
    specialize (IH Hne2). destruct IH as [m Hm].
    change (exists e, (match dc_find_min_access (b :: rest') with
      | None => Some a
      | Some m' => if de_accessed_at a <=? de_accessed_at m' then Some a else Some m'
      end) = Some e).
    rewrite Hm.
    destruct (de_accessed_at a <=? de_accessed_at m); eauto.
Qed.

(* dc_find_min_access returns an element with minimal accessed_at *)
Lemma dc_find_min_access_spec : forall entries e,
  dc_find_min_access entries = Some e ->
  In e entries /\
  forall e', In e' entries -> de_accessed_at e <= de_accessed_at e'.
Proof.
  induction entries as [| a rest IH]; intros e Hmin.
  - discriminate.
  - destruct rest as [| b rest'].
    + simpl in Hmin. inversion Hmin. subst. split.
      * left. reflexivity.
      * intros e' [Heq | []]. subst. lia.
    + (* Unfold one level without reducing recursive call *)
      change (dc_find_min_access (a :: b :: rest')) with
        (match dc_find_min_access (b :: rest') with
         | None => Some a
         | Some m => if de_accessed_at a <=? de_accessed_at m
                     then Some a else Some m
         end) in Hmin.
      (* Case analysis on recursive call *)
      destruct (dc_find_min_access (b :: rest')) as [d |] eqn:Hrec.
      * (* d is min of tail *)
        specialize (IH d eq_refl). destruct IH as [Hdin Hmin_d].
        destruct (de_accessed_at a <=? de_accessed_at d) eqn:Hle.
        -- inversion Hmin. subst. split.
           ++ left. reflexivity.
           ++ intros e' [Heq | Hin'].
              ** subst. lia.
              ** apply Z.leb_le in Hle. specialize (Hmin_d e' Hin'). lia.
        -- inversion Hmin. subst. split.
           ++ right. assumption.
           ++ intros e' [Heq | Hin'].
              ** subst. apply Z.leb_gt in Hle. lia.
              ** apply Hmin_d. assumption.
      * exfalso.
        assert (Hne : b :: rest' <> @nil DiskEntry) by discriminate.
        destruct (dc_find_min_access_some _ Hne) as [m Hm].
        rewrite Hm in Hrec. discriminate.
Qed.

(* Theorem 16: LRU eviction respects size limit (when fuel is sufficient) *)
Lemma dc_lru_evict_size : forall entries max_bytes fuel,
  max_bytes >= 0 ->
  (forall e, In e entries -> de_size_bytes e >= 0) ->
  (fuel >= length entries)%nat ->
  dc_total_size (dc_lru_evict entries max_bytes fuel) <= max_bytes \/
  dc_lru_evict entries max_bytes fuel = [].
Proof.
  intros entries max_bytes fuel Hmax Hpos Hfuel.
  revert entries Hpos Hfuel.
  induction fuel; intros.
  - destruct entries; [right; reflexivity | simpl in Hfuel; lia].
  - simpl.
    destruct (Z.leb (dc_total_size entries) max_bytes) eqn:Hle.
    + left. apply Z.leb_le. assumption.
    + destruct (dc_find_min_access entries) eqn:Hmin.
      * apply IHfuel.
        -- intros. apply Hpos. apply dc_remove_entry_subset in H. assumption.
        -- apply dc_find_min_access_spec in Hmin. destruct Hmin as [Hdin _].
           assert (Hstrict := dc_remove_entry_length_in d entries Hdin).
           apply Nat.lt_succ_r.
           apply Nat.lt_le_trans with (length entries).
           ++ exact Hstrict.
           ++ exact Hfuel.
      * destruct entries as [| x xs].
        -- right. reflexivity.
        -- exfalso.
           assert (Hne : x :: xs <> @nil DiskEntry) by discriminate.
           apply dc_find_min_access_some in Hne. destruct Hne as [m Hm].
           rewrite Hm in Hmin. discriminate.
Qed.


(* ================================================================ *)
(* J. Atomic write safety (modeled as all-or-nothing)                *)
(* ================================================================ *)

(* Model: write is a function that either succeeds completely or fails *)
Inductive WriteResult : Type :=
  | WriteSuccess : Z -> WriteResult   (* new value written *)
  | WriteFail : WriteResult.          (* crash, nothing changed *)

Definition atomic_write (old_val new_val : option Z) (result : WriteResult)
  : option Z :=
  match result with
  | WriteSuccess v => Some v
  | WriteFail => old_val  (* crash: old value preserved *)
  end.

(* Theorem 38: Atomic write never produces partial state *)
Theorem atomic_write_no_partial : forall old new_v result,
  atomic_write old new_v result = old \/
  exists v, atomic_write old new_v result = Some v.
Proof.
  intros. destruct result.
  - right. exists z. reflexivity.
  - left. reflexivity.
Qed.

(* Theorem 39: Successful write stores the intended value *)
Theorem atomic_write_success : forall old v,
  atomic_write old (Some v) (WriteSuccess v) = Some v.
Proof.
  reflexivity.
Qed.

(* Theorem 40: Failed write preserves old value *)
Theorem atomic_write_fail_preserves : forall old new_v,
  atomic_write old new_v WriteFail = old.
Proof.
  reflexivity.
Qed.
