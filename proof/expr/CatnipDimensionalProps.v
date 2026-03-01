(* FILE: proof/expr/CatnipDimensionalProps.v *)
(*                                                                    *)
(* Universality, structural properties, non-trivial algebraic laws,   *)
(* and pipeline algebra for the dimensional calculus.                  *)
(* Split from CatnipDimensional.v to reduce peak memory.              *)

From Coq Require Import List Bool Arith PeanoNat Lia.
Import ListNotations.
From Catnip Require Import CatnipDimensional.


(* ================================================================ *)
(* SECTION G : UNIVERSALITY                                           *)
(* ================================================================ *)

(** An operation [op : Val -> Val] is *elementwise* when, on any
    collection, it produces a collection of the same length whose
    i-th element equals [op] applied to the i-th input element.

    This captures the property that the operation treats each element
    independently, with no cross-element interaction. *)

Definition elementwise (op : Val -> Val) : Prop :=
  forall xs,
    exists ys,
      op (Coll xs) = Coll ys /\
      length ys = length xs /\
      (forall i v, nth_error xs i = Some v ->
                   nth_error ys i = Some (op v)).

(** Auxiliary: two lists that agree pointwise are equal. *)

Lemma list_nth_error_ext : forall {A : Type} (l1 l2 : list A),
  length l1 = length l2 ->
  (forall i, nth_error l1 i = nth_error l2 i) ->
  l1 = l2.
Proof.
  induction l1 as [| a l1' IH]; intros l2 Hlen Hnth.
  - destruct l2; [reflexivity | simpl in Hlen; discriminate].
  - destruct l2 as [| b l2'].
    + simpl in Hlen. discriminate.
    + f_equal.
      * specialize (Hnth 0). simpl in Hnth. injection Hnth. auto.
      * apply IH.
        -- simpl in Hlen. lia.
        -- intros i. specialize (Hnth (S i)). simpl in Hnth. exact Hnth.
Qed.

(** Auxiliary: nth_error commutes with map. *)

Lemma nth_error_map_local : forall (f : Val -> Val) (xs : list Val) (i : nat),
  nth_error (map f xs) i = option_map f (nth_error xs i).
Proof.
  intros f xs. induction xs as [| x xs' IH]; intros i.
  - destruct i; reflexivity.
  - destruct i as [| i']; simpl.
    + reflexivity.
    + apply IH.
Qed.

(** Universality theorem: any elementwise operation on collections
    agrees with broadcast_map.

    This means broadcast_map is the *unique* way to lift a scalar
    operation to collections that preserves length and commutes
    with element extraction.

    Categorically: broadcast_map is the unique natural transformation
    from the identity endofunctor to the list endofunctor that
    extends pointwise application (Johnstone, C2.1). *)

Theorem universality : forall op,
  elementwise op ->
  forall xs, op (Coll xs) = Coll (map op xs).
Proof.
  intros op Hew xs.
  destruct (Hew xs) as [ys [Hop [Hlen Hnth]]].
  rewrite Hop. f_equal.
  apply list_nth_error_ext.
  - rewrite length_map. exact Hlen.
  - intros i. rewrite nth_error_map_local.
    destruct (nth_error xs i) eqn:Hxi.
    + simpl. apply Hnth. exact Hxi.
    + simpl.
      (* i is out of bounds for xs, hence also for ys *)
      assert (Hlen_i : length xs <= i).
      { destruct (Nat.le_gt_cases (length xs) i) as [Hle | Hgt].
        - exact Hle.
        - exfalso.
          apply nth_error_Some in Hgt. rewrite Hxi in Hgt. apply Hgt. reflexivity. }
      assert (Hyi : nth_error ys i = None).
      { apply nth_error_None. lia. }
      rewrite Hyi. reflexivity.
Qed.

(** broadcast_map preserves pointwise equality with its kernel f.
    (Note: broadcast_map f applies f -- not broadcast_map f -- to each
    element.  This one-level semantics matches Catnip's runtime:
    data.[f] maps f over the top-level elements without recursing.) *)

Lemma broadcast_map_nth : forall (f : Val -> Val) (xs : list Val) (i : nat) (v : Val),
  nth_error xs i = Some v ->
  nth_error (map f xs) i = Some (f v).
Proof.
  intros f xs i v Hxi.
  rewrite nth_error_map_local. rewrite Hxi. reflexivity.
Qed.

(** Any elementwise op equals broadcast_map op on all values. *)

Corollary broadcast_unique : forall op,
  elementwise op ->
  forall v, op v = broadcast_map op v.
Proof.
  intros op Hew [n | xs].
  - (* Scalar: broadcast_map op (Scalar n) = op (Scalar n) *)
    simpl. reflexivity.
  - (* Collection: by universality *)
    apply universality. exact Hew.
Qed.

Definition flat (xs : list Val) : Prop :=
  forall v, In v xs -> exists n, v = Scalar n.

(** Minimality on flat collections (all elements are scalars):
    for flat data, the scalar kernel of f uniquely determines
    broadcast_map f.  This is the strongest true form for one-level
    broadcast semantics. *)

Theorem broadcast_minimal : forall f g xs,
  (forall n, f (Scalar n) = g (Scalar n)) ->
  flat xs ->
  broadcast_map f (Coll xs) = broadcast_map g (Coll xs).
Proof.
  intros f g xs Hscalar Hflat. simpl. f_equal.
  induction xs as [| x xs' IH]; simpl.
  - reflexivity.
  - f_equal.
    + destruct (Hflat x (or_introl eq_refl)) as [n Hn].
      subst. apply Hscalar.
    + apply IH. intros v Hv. apply Hflat. right. exact Hv.
Qed.

Corollary broadcast_minimal_flat : forall f g xs,
  (forall n, f (Scalar n) = g (Scalar n)) ->
  flat xs ->
  broadcast_map f (Coll xs) = broadcast_map g (Coll xs).
Proof.
  intros f g xs Hscalar Hflat.
  eapply broadcast_minimal; eauto.
Qed.


(* ================================================================ *)
(* SECTION H : ND-MAP AND BROADCAST EQUIVALENCE                      *)
(* ================================================================ *)

(** @>(data, f) is semantically equivalent to broadcast_map f data.
    This shows that ND-map is subsumed by broadcast. *)

Definition nd_map (f : Val -> Val) (v : Val) : Val :=
  broadcast_map f v.

Theorem nd_map_is_broadcast : forall f v,
  nd_map f v = broadcast_map f v.
Proof. reflexivity. Qed.

(** @>(data, f) composed with @>(_, g) fuses like broadcast. *)

Theorem nd_map_composition : forall (f g : Val -> Val) (xs : list Val),
  nd_map g (nd_map f (Coll xs)) = nd_map (fun x => g (f x)) (Coll xs).
Proof.
  intros. unfold nd_map. apply coherence_composition.
Qed.


(* ================================================================ *)
(* SECTION I : STRUCTURAL PROPERTIES                                  *)
(* ================================================================ *)

(** Type preservation: broadcast_map on a Coll always returns a Coll,
    on a Scalar always returns the direct application. *)

Theorem type_preservation_coll : forall f xs,
  exists ys, broadcast_map f (Coll xs) = Coll ys.
Proof.
  intros. exists (map f xs). reflexivity.
Qed.

(** Broadcast respects structural decomposition:
    broadcasting over a cons is the same as broadcasting the head
    and broadcasting the tail. *)

Theorem broadcast_cons : forall f x xs,
  broadcast_map f (Coll (x :: xs)) =
  match broadcast_map f (Coll [x]), broadcast_map f (Coll xs) with
  | Coll [y], Coll ys => Coll (y :: ys)
  | _, _ => broadcast_map f (Coll (x :: xs))  (* unreachable *)
  end.
Proof.
  intros. simpl. reflexivity.
Qed.

(** Element independence: the i-th output element depends only
    on the i-th input element.  Changing one input element
    affects only the corresponding output element. *)

Theorem element_independence :
  forall (f : Val -> Val) (xs : list Val) (i : nat) (x : Val),
  i < length xs ->
  nth_error (map f (firstn i xs ++ [x] ++ skipn (S i) xs)) i =
  Some (f x).
Proof.
  intros f xs i x Hi.
  rewrite nth_error_map_local.
  assert (Hnth : nth_error (firstn i xs ++ [x] ++ skipn (S i) xs) i = Some x).
  { rewrite nth_error_app2.
    - rewrite length_firstn. rewrite Nat.min_l by lia. rewrite Nat.sub_diag.
      simpl. reflexivity.
    - rewrite length_firstn. lia. }
  rewrite Hnth. reflexivity.
Qed.


(* ================================================================ *)
(* SECTION J : NON-TRIVIAL PROPERTIES                                *)
(* ================================================================ *)

(** Properties requiring structural induction or algebraic hypotheses.
    These extend the coherence/universality framework to cover
    filter-map interaction, boolean masking, fold exchange, and
    shallow broadcast semantics. *)


(* --- J.1 : General filter-map pullback law ----------------------- *)

(** Filtering after mapping equals mapping after filtering with
    the pulled-back predicate [fun x => p (f x)].
    This strengthens [filter_map_commute] (Section D) which requires
    [p] to be invariant under [f].  Here, no hypothesis needed. *)

Theorem filter_map_pullback : forall (f : Val -> Val) (p : Val -> bool) (xs : list Val),
  filter p (map f xs) = map f (filter (fun x => p (f x)) xs).
Proof.
  intros f p xs.
  induction xs as [| x xs' IH]; simpl.
  - reflexivity.
  - destruct (p (f x)) eqn:Hpfx; simpl.
    + f_equal. exact IH.
    + exact IH.
Qed.


(* --- J.2 : Filter absorption ------------------------------------- *)

(** Two successive filters absorb into one with conjoint predicate.
    Proof requires induction with nested case analysis on both
    predicates. *)

Theorem filter_filter : forall (p q : Val -> bool) (xs : list Val),
  filter p (filter q xs) = filter (fun x => q x && p x) xs.
Proof.
  intros p q xs.
  induction xs as [| x xs' IH].
  - reflexivity.
  - simpl. destruct (q x) eqn:Hqx; simpl.
    + destruct (p x) eqn:Hpx; simpl.
      * f_equal. exact IH.
      * exact IH.
    + exact IH.
Qed.


(* --- J.3 : Broadcast filter-map pullback ------------------------- *)

(** At the Val level: filtering a broadcast equals broadcasting a
    filtered input.  Direct corollary of [filter_map_pullback]. *)

Theorem broadcast_filter_map : forall (f : Val -> Val) (p : Val -> bool) (xs : list Val),
  broadcast_filter p (broadcast_map f (Coll xs)) =
  broadcast_map f (broadcast_filter (fun x => p (f x)) (Coll xs)).
Proof.
  intros f p xs. simpl. f_equal. apply filter_map_pullback.
Qed.


(* --- J.4 : Boolean mask selection -------------------------------- *)

(** Boolean mask: keeps elements at positions where the mask is [true].
    Models [data.\[mask\]] in Catnip syntax.
    Unlike [broadcast_filter], the keep/drop decision comes from a
    parallel boolean vector, not from a predicate on the element. *)

Fixpoint mask_select (mask : list bool) (xs : list Val) : list Val :=
  match mask, xs with
  | true :: ms,  x :: xs' => x :: mask_select ms xs'
  | false :: ms, _ :: xs' => mask_select ms xs'
  | _, _ => []
  end.

(** All-true mask is identity. *)

Theorem mask_all_true : forall (xs : list Val),
  mask_select (repeat true (length xs)) xs = xs.
Proof.
  induction xs as [| x xs' IH]; simpl.
  - reflexivity.
  - f_equal. exact IH.
Qed.

(** All-false mask gives empty. *)

Theorem mask_all_false : forall (xs : list Val),
  mask_select (repeat false (length xs)) xs = [].
Proof.
  induction xs as [| x xs' IH]; simpl.
  - reflexivity.
  - exact IH.
Qed.

(** Mask commutes with map: masking then mapping equals
    mapping then masking.  The mask is position-based and does not
    depend on element values, so map cannot disrupt it. *)

Theorem mask_map_commute : forall (f : Val -> Val) (mask : list bool) (xs : list Val),
  map f (mask_select mask xs) = mask_select mask (map f xs).
Proof.
  intros f mask.
  induction mask as [| b ms IH]; intros xs.
  - reflexivity.
  - destruct xs as [| x xs']; [destruct b; reflexivity |].
    destruct b; simpl.
    + f_equal. apply IH.
    + apply IH.
Qed.

(** Mask result is never longer than input. *)

Theorem mask_length_le : forall (mask : list bool) (xs : list Val),
  length (mask_select mask xs) <= length xs.
Proof.
  intros mask.
  induction mask as [| b ms IH]; intros xs.
  - simpl. lia.
  - destruct xs as [| x xs'].
    + destruct b; simpl; lia.
    + destruct b; simpl; specialize (IH xs'); lia.
Qed.


(* --- J.5 : Broadcast distributes over concatenation -------------- *)

(** Broadcast is a list homomorphism: it distributes over [++].
    A collection can be split, broadcast independently, and
    concatenated without affecting the result.
    This is the algebraic foundation for parallel broadcast. *)

Theorem broadcast_concat : forall (f : Val -> Val) (xs ys : list Val),
  broadcast_map f (Coll (xs ++ ys)) = Coll (map f xs ++ map f ys).
Proof.
  intros. simpl. f_equal. apply map_app.
Qed.


(* --- J.6 : Shallow (one-level) broadcast ------------------------- *)

(** Broadcast operates at exactly one level: on nested data,
    [f] receives inner collections as opaque values.
    This formalizes a core Catnip design invariant:
    [data.\[f\]] never recurses into sub-collections. *)

Theorem broadcast_shallow : forall (f : Val -> Val) (xss : list (list Val)),
  broadcast_map f (Coll (map Coll xss)) =
  Coll (map (fun xs => f (Coll xs)) xss).
Proof.
  intros. simpl. f_equal. apply map_map.
Qed.

(** Two-level broadcast requires explicit nesting:
    [data.\[broadcast_map f\]] recurses one more level. *)

Theorem broadcast_two_levels : forall (f : Val -> Val) (xss : list (list Val)),
  broadcast_map (broadcast_map f) (Coll (map Coll xss)) =
  Coll (map (fun xs => Coll (map f xs)) xss).
Proof.
  intros f xss. simpl. f_equal.
  induction xss as [| xs xss' IH]; simpl.
  - reflexivity.
  - f_equal. exact IH.
Qed.


(* --- J.7 : Fold-broadcast exchange (monoid homomorphism) --------- *)

Fixpoint fold_right_val (op : Val -> Val -> Val) (z : Val) (xs : list Val) : Val :=
  match xs with
  | [] => z
  | x :: xs' => op x (fold_right_val op z xs')
  end.

(** When [f] distributes over [op] and fixes the identity [z],
    [f] commutes with [fold_right_val] -- i.e. [f] is a monoid
    homomorphism from [(Val, op, z)] to itself.
    Practical instance: doubling each element then summing equals
    summing then doubling. *)

Theorem fold_broadcast_exchange :
  forall (f : Val -> Val) (op : Val -> Val -> Val) (z : Val) (xs : list Val),
  (forall x y, f (op x y) = op (f x) (f y)) ->
  f z = z ->
  f (fold_right_val op z xs) = fold_right_val op z (map f xs).
Proof.
  intros f op z xs Hdist Hz.
  induction xs as [| x xs' IH]; simpl.
  - exact Hz.
  - rewrite Hdist. f_equal. exact IH.
Qed.

(** Folding over a broadcast is folding with a composed accumulator.
    No algebraic hypothesis needed. *)

Theorem fold_broadcast_map :
  forall (f : Val -> Val) (op : Val -> Val -> Val) (z : Val) (xs : list Val),
  fold_right_val op z (map f xs) =
  fold_right_val (fun x acc => op (f x) acc) z xs.
Proof.
  intros f op z xs.
  induction xs as [| x xs' IH]; simpl.
  - reflexivity.
  - f_equal. exact IH.
Qed.


(* ================================================================ *)
(* SECTION K : PIPELINE ALGEBRA                                      *)
(* ================================================================ *)

(** Pipeline operations in Catnip can be composed, reordered, and
    normalized.  This section proves:
    - Complete fusion: any chain of maps collapses to one map.
    - Transformational equivalence: pipeline rewriting rules
      preserve semantics.
    - Normalization: any mixed map/filter pipeline reduces to a
      canonical filter-then-map form. *)


(* --- K.1 : Complete map-chain fusion ----------------------------- *)

(** A chain of broadcast maps applied successively. *)

Fixpoint apply_map_chain (fs : list (Val -> Val)) (v : Val) : Val :=
  match fs with
  | [] => v
  | f :: fs' => apply_map_chain fs' (broadcast_map f v)
  end.

(** Left-to-right function composition: apply each function
    in sequence. *)

Fixpoint compose_chain (fs : list (Val -> Val)) (x : Val) : Val :=
  match fs with
  | [] => x
  | f :: fs' => compose_chain fs' (f x)
  end.

(** Complete fusion: any chain of n broadcast maps on a collection
    is equivalent to a single broadcast with the composed function.
    The fusion is safe (semantics-preserving) and complete
    (works for any number of maps, not just two). *)

Theorem map_chain_fusion : forall (fs : list (Val -> Val)) (xs : list Val),
  apply_map_chain fs (Coll xs) = Coll (map (compose_chain fs) xs).
Proof.
  induction fs as [| f fs' IH]; intros xs; simpl.
  - f_equal. symmetry. apply map_id_local.
  - rewrite IH. f_equal. rewrite map_map. reflexivity.
Qed.

(** Corollary: fusing n maps preserves collection length. *)

Corollary map_chain_preserves_length : forall (fs : list (Val -> Val)) (xs : list Val),
  exists ys, apply_map_chain fs (Coll xs) = Coll ys /\ length ys = length xs.
Proof.
  intros. exists (map (compose_chain fs) xs). split.
  - apply map_chain_fusion.
  - apply length_map.
Qed.


(* --- K.2 : Transformational equivalence -------------------------- *)

(** Pipeline operations: map or filter. *)

Inductive PipeOp : Type :=
| PMap  : (Val -> Val)  -> PipeOp
| PFilt : (Val -> bool) -> PipeOp.

(** Execute a pipeline left-to-right on a value. *)

Fixpoint run_pipeline (ops : list PipeOp) (v : Val) : Val :=
  match ops with
  | [] => v
  | PMap f  :: ops' => run_pipeline ops' (broadcast_map f v)
  | PFilt p :: ops' => run_pipeline ops' (broadcast_filter p v)
  end.

(** Two pipelines are equivalent when they produce the same result
    on all collections. *)

Definition pipeline_equiv (p1 p2 : list PipeOp) : Prop :=
  forall xs, run_pipeline p1 (Coll xs) = run_pipeline p2 (Coll xs).

(** Rule 1 -- Map-map fusion.
    Two consecutive maps fuse into one. *)

Theorem equiv_map_map : forall f g,
  pipeline_equiv [PMap f; PMap g] [PMap (fun x => g (f x))].
Proof.
  intros f g xs. simpl. f_equal. apply map_map.
Qed.

(** Rule 2 -- Filter-map pullback.
    A map followed by a filter can be reordered:
    the filter is pulled back through the map. *)

Theorem equiv_filter_map_swap : forall f p,
  pipeline_equiv [PMap f; PFilt p] [PFilt (fun x => p (f x)); PMap f].
Proof.
  intros f p xs. simpl. f_equal. apply filter_map_pullback.
Qed.

(** Rule 3 -- Filter-filter absorption.
    Two consecutive filters merge into one. *)

Theorem equiv_filter_filter : forall p q,
  pipeline_equiv [PFilt p; PFilt q] [PFilt (fun x => p x && q x)].
Proof.
  intros p q xs. simpl. f_equal. apply filter_filter.
Qed.


(* --- K.3 : Pipeline normalization -------------------------------- *)

(** Helper: filter with a trivially true predicate is identity. *)

Lemma filter_true : forall (xs : list Val),
  filter (fun _ : Val => true) xs = xs.
Proof.
  induction xs as [| x xs' IH]; simpl.
  - reflexivity.
  - f_equal. exact IH.
Qed.

(** Normalize a pipeline to a (predicate, function) pair
    representing the canonical form: filter then map.
    Every pipeline of maps and filters has this normal form. *)

Fixpoint normalize_pipeline (ops : list PipeOp) : (Val -> bool) * (Val -> Val) :=
  match ops with
  | [] => (fun _ => true, fun x => x)
  | PMap g :: ops' =>
      match normalize_pipeline ops' with
      | (p, f) => (fun x => p (g x), fun x => f (g x))
      end
  | PFilt q :: ops' =>
      match normalize_pipeline ops' with
      | (p, f) => (fun x => q x && p x, f)
      end
  end.

(** Execute the normalized form on a list. *)

Definition run_normal (nf : (Val -> bool) * (Val -> Val)) (xs : list Val) : Val :=
  Coll (map (snd nf) (filter (fst nf) xs)).

(** Normalization theorem: any pipeline of maps and filters on a
    collection is equivalent to a single filter followed by a
    single map.

    This is the pipeline analogue of a normal form theorem:
    [data.\[f\].\[if p\].\[g\]] has a canonical form
    [data.\[if p'\].\[h\]] for some computed [p'] and [h]. *)

Theorem pipeline_normalization : forall (ops : list PipeOp) (xs : list Val),
  run_pipeline ops (Coll xs) = run_normal (normalize_pipeline ops) xs.
Proof.
  induction ops as [| op ops' IH]; intros xs.
  - (* [] *)
    unfold run_normal. simpl.
    f_equal. rewrite filter_true. symmetry. apply map_id_local.
  - destruct op as [g | q].
    + (* PMap g *)
      simpl. rewrite IH. unfold run_normal.
      destruct (normalize_pipeline ops') as [p f]. simpl.
      f_equal. rewrite filter_map_pullback. apply map_map.
    + (* PFilt q *)
      simpl. rewrite IH. unfold run_normal.
      destruct (normalize_pipeline ops') as [p f]. simpl.
      f_equal. rewrite filter_filter. reflexivity.
Qed.

(** Corollary: pipeline equivalence is sound via normalization.
    Two pipelines that agree on the normalized form are equivalent. *)

Corollary normalization_sound : forall (ops1 ops2 : list PipeOp),
  (forall xs, run_normal (normalize_pipeline ops1) xs =
              run_normal (normalize_pipeline ops2) xs) ->
  pipeline_equiv ops1 ops2.
Proof.
  intros ops1 ops2 Hnorm xs.
  rewrite pipeline_normalization. rewrite pipeline_normalization.
  apply Hnorm.
Qed.


(* ================================================================ *)
(* SECTION L : SUMMARY OF RESULTS                                    *)
(* ================================================================ *)

(** The dimensional calculus of Catnip satisfies:

    1. COHERENCE (functor laws)
       - coherence_identity:      v.[id] = v
       - coherence_composition:   v.[f].[g] = v.[g . f]

    2. CONFLUENCE
       - eval_deterministic:  evaluation is a total function
       - eval_fusion:         chained broadcasts fuse

    3. PARTIAL TERMINATION
       - nd_partial_termination:  well-founded ND-recursion terminates
       - nd_eval_mono:            monotonicity of fuel
       - memo_coherence:          lookup hits preserve correctness

    4. UNIVERSALITY
       - universality:         any elementwise op is broadcast_map
       - broadcast_unique:     broadcast_map is the unique such op
       - broadcast_minimal_flat: determined by scalar behavior

    5. NON-TRIVIAL PROPERTIES
       - filter_map_pullback:     filter . map = map . filter (p . f)
       - filter_filter:           filter p . filter q = filter (q && p)
       - broadcast_filter_map:    Val-level pullback
       - mask_map_commute:        mask commutes with map
       - broadcast_concat:        broadcast distributes over ++
       - broadcast_shallow:       one-level semantics
       - broadcast_two_levels:    explicit nesting required for depth
       - fold_broadcast_exchange: monoid homomorphism
       - fold_broadcast_map:      fold . map = fold with composed op

    6. PIPELINE ALGEBRA
       - map_chain_fusion:        n maps fuse to one (complete fusion)
       - equiv_map_map:           map;map -> map
       - equiv_filter_map_swap:   map;filter -> filter;map
       - equiv_filter_filter:     filter;filter -> filter
       - pipeline_normalization:  any pipeline = filter;map
       - normalization_sound:     same normal form => equivalent

    These results formalize the guarantees stated in
    docs/lang/BROADCAST.md: broadcast is a minimal, universal,
    confluent abstraction for dimension-polymorphic computation. *)
