(* FILE: proof/expr/CatnipDimensional.v *)
From Coq Require Import List Bool Arith PeanoNat Lia.
Import ListNotations.

(* ================================================================ *)
(* CatnipDimensional.v                                               *)
(*                                                                    *)
(* Formal model of Catnip's dimensional calculus: core definitions.   *)
(*                                                                    *)
(* Catnip's broadcast notation .[op] lifts scalar operations          *)
(* to collections without conditional branching.  This file           *)
(* formalizes the value domain, broadcast semantics, and proves:      *)
(*                                                                    *)
(*   A. Coherence      -- broadcast satisfies the functor laws        *)
(*   B. Confluence      -- evaluation is deterministic                *)
(*                                                                    *)
(* ND-recursion:     see CatnipNDRecursion.v                          *)
(* Universality etc: see CatnipDimensionalProps.v                     *)
(*                                                                    *)
(* References:                                                        *)
(*   Johnstone, Sketches of an Elephant, vol.2, C2.1                  *)
(*   Braun et al., Simple and Efficient SSA Construction, 2013        *)
(* ================================================================ *)


(* ================================================================ *)
(* SECTION A : VALUE DOMAIN                                           *)
(* ================================================================ *)

(** Values in the dimensional calculus.
    [Scalar] wraps a natural number; [Coll] wraps a list of values.
    The empty topos @[] is modeled as [Coll nil]. *)

Inductive Val : Type :=
| Scalar : nat -> Val
| Coll   : list Val -> Val.

Definition empty_topos : Val := Coll [].

(** Truthiness: non-empty collections and non-zero scalars are truthy;
    empty collections (including @[]) and zero are falsy. *)

Definition val_bool (v : Val) : bool :=
  match v with
  | Scalar 0 => false
  | Scalar _ => true
  | Coll []  => false
  | Coll _   => true
  end.

Theorem empty_topos_falsy : val_bool empty_topos = false.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* SECTION B : BROADCAST MAP                                          *)
(* ================================================================ *)

(** [broadcast_map f v] applies [f] element-wise:
    - On a scalar: direct application.
    - On a collection: map over elements.
    This mirrors [target.\[op\]] in Catnip syntax. *)

Definition broadcast_map (f : Val -> Val) (v : Val) : Val :=
  match v with
  | Scalar n => f (Scalar n)
  | Coll xs  => Coll (map f xs)
  end.


(* ================================================================ *)
(* SECTION C : COHERENCE (FUNCTOR LAWS)                               *)
(* ================================================================ *)

(** broadcast_map forms an endofunctor on Val.
    These two laws guarantee that chained broadcasts
    behave predictably regardless of fusion or evaluation order. *)

Lemma map_id_local : forall (l : list Val),
  map (fun x => x) l = l.
Proof.
  induction l as [| a l' IH]; simpl.
  - reflexivity.
  - f_equal. exact IH.
Qed.

(** Identity law: broadcasting the identity function is a no-op.
    Corresponds to:  v.[id] = v  *)

Theorem coherence_identity : forall v,
  broadcast_map (fun x => x) v = v.
Proof.
  destruct v as [n | xs]; simpl.
  - reflexivity.
  - f_equal. apply map_id_local.
Qed.

(** Composition law on collections: two consecutive broadcasts fuse.
    Corresponds to:  xs.[f].[g] = xs.[(x) => g(f(x))]

    This is the core fusion guarantee: chaining broadcasts on a collection
    is equivalent to broadcasting the composed function.
    (On scalars, composition holds trivially when f preserves type;
    the collection case is the non-trivial structural result.) *)

Theorem coherence_composition : forall (f g : Val -> Val) (xs : list Val),
  broadcast_map g (broadcast_map f (Coll xs)) =
  broadcast_map (fun x => g (f x)) (Coll xs).
Proof.
  intros f g xs. simpl. f_equal. apply map_map.
Qed.

(** Composition on scalars holds when f preserves scalar shape. *)

Theorem coherence_composition_scalar : forall (f g : Val -> Val) (n : nat),
  (exists m, f (Scalar n) = Scalar m) ->
  broadcast_map g (broadcast_map f (Scalar n)) =
  broadcast_map (fun x => g (f x)) (Scalar n).
Proof.
  intros f g n [m Hm]. simpl. rewrite Hm. simpl. reflexivity.
Qed.

(** Broadcast preserves collection length. *)

Theorem broadcast_preserves_length : forall f xs,
  match broadcast_map f (Coll xs) with
  | Coll ys => length ys = length xs
  | _       => False
  end.
Proof.
  intros. simpl. apply length_map.
Qed.

(** @[] is a fixed point of broadcast: broadcasting any function
    over the empty topos yields the empty topos. *)

Theorem broadcast_empty_fixed : forall f,
  broadcast_map f empty_topos = empty_topos.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* SECTION D : BROADCAST FILTER                                       *)
(* ================================================================ *)

(** [broadcast_filter p v] keeps elements satisfying [p]:
    - On a scalar: singleton list if [p v], else empty.
    - On a collection: standard filter. *)

Definition broadcast_filter (p : Val -> bool) (v : Val) : Val :=
  match v with
  | Scalar n =>
      if p (Scalar n) then Coll [Scalar n]
      else Coll []
  | Coll xs => Coll (filter p xs)
  end.

(** Filter is idempotent: filtering twice is the same as filtering once. *)

Theorem filter_idempotent : forall (p : Val -> bool) (xs : list Val),
  filter p (filter p xs) = filter p xs.
Proof.
  induction xs as [| x xs' IH]; simpl.
  - reflexivity.
  - destruct (p x) eqn:Hpx; simpl.
    + rewrite Hpx. f_equal. exact IH.
    + exact IH.
Qed.

(** Filter distributes over append. *)

Theorem filter_app : forall (p : Val -> bool) (xs ys : list Val),
  filter p (xs ++ ys) = filter p xs ++ filter p ys.
Proof.
  induction xs as [| x xs' IH]; intros; simpl.
  - reflexivity.
  - destruct (p x); simpl; f_equal; apply IH.
Qed.

(** Filter-then-map commutes when the predicate is invariant under f. *)

Theorem filter_map_commute : forall (f : Val -> Val) (p : Val -> bool) (xs : list Val),
  (forall x, p (f x) = p x) ->
  map f (filter p xs) = filter p (map f xs).
Proof.
  intros f p xs Hinv.
  induction xs as [| x xs' IH]; simpl.
  - reflexivity.
  - rewrite Hinv. destruct (p x) eqn:Hpx; simpl.
    + f_equal. exact IH.
    + exact IH.
Qed.

(** @[] is a zero for filter. *)

Theorem filter_empty_zero : forall p,
  broadcast_filter p empty_topos = empty_topos.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* SECTION E : EXPRESSION LANGUAGE AND CONFLUENCE                     *)
(* ================================================================ *)

(** Minimal expression language with broadcast-map and broadcast-filter.
    We prove evaluation is deterministic, which gives confluence
    (Church-Rosser for deterministic systems). *)

Inductive Expr : Type :=
| EVal       : Val -> Expr
| EBroadMap  : Expr -> (Val -> Val)  -> Expr
| EBroadFilt : Expr -> (Val -> bool) -> Expr.

(** Big-step evaluation relation. *)

Inductive eval : Expr -> Val -> Prop :=
| eval_val : forall v,
    eval (EVal v) v
| eval_map : forall e v f,
    eval e v ->
    eval (EBroadMap e f) (broadcast_map f v)
| eval_filt : forall e v p,
    eval e v ->
    eval (EBroadFilt e p) (broadcast_filter p v).

(** Totality: every expression evaluates to some value. *)

Theorem eval_total : forall e, exists v, eval e v.
Proof.
  induction e as [v | e IHe f | e IHe p].
  - exists v. constructor.
  - destruct IHe as [v Hv]. exists (broadcast_map f v). constructor. exact Hv.
  - destruct IHe as [v Hv]. exists (broadcast_filter p v). constructor. exact Hv.
Qed.

(** Determinism: evaluation produces exactly one result.
    Combined with totality, this makes the semantics a total function. *)

Theorem eval_deterministic : forall e v1 v2,
  eval e v1 -> eval e v2 -> v1 = v2.
Proof.
  intros e v1 v2 H1. revert v2.
  induction H1 as [v | e v f Hev IH | e v p Hev IH]; intros v2 H2;
    inversion H2; subst.
  - reflexivity.
  - rewrite (IH _ ltac:(eassumption)). reflexivity.
  - rewrite (IH _ ltac:(eassumption)). reflexivity.
Qed.

(** Confluence (Church-Rosser): if e reduces to both v1 and v2,
    they are the same value.  Immediate from determinism. *)

Definition confluent := eval_deterministic.

(** Evaluation commutes with broadcast composition on collections. *)

Theorem eval_fusion : forall e f g xs,
  eval e (Coll xs) ->
  eval (EBroadMap (EBroadMap e f) g)
       (broadcast_map (fun x => g (f x)) (Coll xs)).
Proof.
  intros e f g xs Hev.
  rewrite <- coherence_composition.
  constructor. constructor. exact Hev.
Qed.
