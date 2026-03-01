(* FILE: proof/expr/CatnipDimensionalDeep.v *)
(*                                                                    *)
(* Deep (recursive) broadcast: definitions and proofs.                *)
(* Complements CatnipDimensional.v (shallow broadcast_map) with       *)
(* broadcast_deep, which traverses nested Val structures and applies  *)
(* f only at Scalar leaves.                                           *)
(*                                                                    *)
(* References:                                                        *)
(*   Chlipala, CPDT ch.3 (nested induction principle)                 *)
(*   Johnstone, Sketches of an Elephant, vol.2, C2.1                  *)

From Coq Require Import List Bool Arith PeanoNat.
Import ListNotations.
From Catnip Require Import CatnipDimensional.
From Catnip Require Import CatnipDimensionalProps.


(* ================================================================ *)
(* SECTION A : NESTED INDUCTION PRINCIPLE FOR Val                     *)
(* ================================================================ *)

(** Coq's auto-generated [Val_ind] is too weak for nested data:
    it provides no induction hypothesis on the elements of the list
    inside [Coll].  We define a strengthened principle via mutual
    fixpoint (Chlipala, CPDT ch.3). *)

Fixpoint Forall_Val (P : Val -> Prop) (xs : list Val) : Prop :=
  match xs with
  | [] => True
  | x :: xs' => P x /\ Forall_Val P xs'
  end.

Fixpoint val_ind_nested (P : Val -> Prop)
  (Hscalar : forall n, P (Scalar n))
  (Hcoll : forall xs, Forall_Val P xs -> P (Coll xs))
  (v : Val) {struct v} : P v :=
  match v return P v with
  | Scalar n => Hscalar n
  | Coll xs =>
    Hcoll xs
      ((fix aux (l : list Val) : Forall_Val P l :=
        match l return Forall_Val P l with
        | [] => I
        | x :: l' => conj (val_ind_nested P Hscalar Hcoll x) (aux l')
        end) xs)
  end.

Lemma Forall_Val_iff : forall P xs, Forall_Val P xs <-> Forall P xs.
Proof.
  intros P xs. induction xs as [| x xs' IH].
  - split; [intros _; constructor | intros _; exact I].
  - split.
    + intros [Hx Hxs']. constructor; [exact Hx | apply IH; exact Hxs'].
    + intros H. inversion_clear H. split; [assumption | apply IH; assumption].
Qed.


(* ================================================================ *)
(* SECTION B : DEEP BROADCAST                                         *)
(* ================================================================ *)

(** [broadcast_deep f v] traverses the entire Val tree and applies
    [f] only at [Scalar] leaves.  Unlike [broadcast_map] which
    operates at one level, [broadcast_deep] recurses into nested
    [Coll] structures automatically. *)

Fixpoint broadcast_deep (f : Val -> Val) (v : Val) {struct v} : Val :=
  match v with
  | Scalar n => f (Scalar n)
  | Coll xs => Coll (map (broadcast_deep f) xs)
  end.

(** Non-recursive wrapper exposing the list-level operation. *)

Definition broadcast_deep_list (f : Val -> Val) (xs : list Val) : list Val :=
  map (broadcast_deep f) xs.

Lemma broadcast_deep_list_is_map : forall f xs,
  broadcast_deep_list f xs = map (broadcast_deep f) xs.
Proof. reflexivity. Qed.

Lemma broadcast_deep_coll : forall f xs,
  broadcast_deep f (Coll xs) = Coll (map (broadcast_deep f) xs).
Proof. reflexivity. Qed.


(* ================================================================ *)
(* SECTION C : FUNCTOR LAWS (IDENTITY, COMPOSITION)                   *)
(* ================================================================ *)

(** A function is scalar-preserving when it maps scalars to scalars.
    This is the minimal hypothesis for the composition law. *)

Definition scalar_preserving (f : Val -> Val) : Prop :=
  forall n, exists m, f (Scalar n) = Scalar m.

(** Auxiliary: map with a pointwise-identity function is identity. *)

Lemma map_ext_id : forall (f : Val -> Val) (xs : list Val),
  Forall_Val (fun x => f x = x) xs -> map f xs = xs.
Proof.
  intros f xs. induction xs as [| x xs' IH]; intros H.
  - reflexivity.
  - simpl. destruct H as [Hx Hxs']. f_equal; [exact Hx | exact (IH Hxs')].
Qed.

(** Auxiliary: pointwise-equal functions produce equal maps. *)

Lemma map_ext_Forall : forall {B : Type} (f g : Val -> B) (xs : list Val),
  Forall_Val (fun x => f x = g x) xs -> map f xs = map g xs.
Proof.
  intros B f g xs. induction xs as [| x xs' IH]; intros H.
  - reflexivity.
  - simpl. destruct H as [Hx Hxs']. f_equal; [exact Hx | exact (IH Hxs')].
Qed.

(** Identity law: deep broadcasting id is a no-op on any Val tree. *)

Theorem deep_coherence_identity : forall v,
  broadcast_deep id v = v.
Proof.
  apply (val_ind_nested (fun v => broadcast_deep id v = v)).
  - intros n. reflexivity.
  - intros xs IH. rewrite broadcast_deep_coll. f_equal.
    apply map_ext_id. exact IH.
Qed.

(** Composition law: under scalar_preserving, deep broadcast
    commutes with function composition.  This is the non-trivial
    functor law for the recursive case. *)

Theorem deep_coherence_composition : forall f g v,
  scalar_preserving f ->
  broadcast_deep g (broadcast_deep f v) =
  broadcast_deep (fun x => g (f x)) v.
Proof.
  intros f g v Hsp. revert v.
  apply (val_ind_nested
    (fun v => broadcast_deep g (broadcast_deep f v) =
              broadcast_deep (fun x => g (f x)) v)).
  - intros n. simpl. destruct (Hsp n) as [m Hm]. rewrite Hm. simpl. reflexivity.
  - intros xs IH.
    rewrite broadcast_deep_coll.
    rewrite broadcast_deep_coll.
    rewrite broadcast_deep_coll.
    f_equal. rewrite map_map.
    apply map_ext_Forall. exact IH.
Qed.

(** Counterexample: without scalar_preserving, composition fails.
    Here f sends Scalar to Coll, breaking the precondition. *)

Example deep_composition_counterexample :
  let f := fun v : Val => match v with Scalar _ => Coll [v] | x => x end in
  let g := fun v : Val => match v with Scalar _ => Scalar 1 | Coll _ => Scalar 2 end in
  broadcast_deep g (broadcast_deep f (Scalar 0)) <>
  broadcast_deep (fun x => g (f x)) (Scalar 0).
Proof. cbv. intro H. discriminate H. Qed.


(* ================================================================ *)
(* SECTION D : SHAPE PRESERVATION                                     *)
(* ================================================================ *)

(** Shape: the tree skeleton of a Val, ignoring scalar payloads. *)

Inductive Shape : Type :=
| SScalar : Shape
| SColl   : list Shape -> Shape.

Fixpoint val_shape (v : Val) : Shape :=
  match v with
  | Scalar _ => SScalar
  | Coll xs  => SColl (map val_shape xs)
  end.

(** scalar_preserving functions preserve the shape of any Val tree. *)

Theorem deep_preserves_shape : forall f v,
  scalar_preserving f ->
  val_shape (broadcast_deep f v) = val_shape v.
Proof.
  intros f v Hsp. revert v.
  apply (val_ind_nested
    (fun v => val_shape (broadcast_deep f v) = val_shape v)).
  - intros n. simpl. destruct (Hsp n) as [m Hm]. rewrite Hm. reflexivity.
  - intros xs IH. rewrite broadcast_deep_coll. simpl. f_equal.
    rewrite map_map. apply map_ext_Forall. exact IH.
Qed.

(** Depth: maximum nesting level of a Val tree. *)

Fixpoint val_depth (v : Val) : nat :=
  match v with
  | Scalar _ => 0
  | Coll xs  => S (fold_right Nat.max 0 (map val_depth xs))
  end.

Definition list_max_depth (xs : list Val) : nat :=
  fold_right Nat.max 0 (map val_depth xs).

(** Helper: list_max_depth is invariant under pointwise
    depth-preserving maps. *)

Lemma list_max_depth_ext : forall (f : Val -> Val) xs,
  Forall_Val (fun v => val_depth (f v) = val_depth v) xs ->
  list_max_depth (map f xs) = list_max_depth xs.
Proof.
  intros f xs H. unfold list_max_depth. f_equal.
  rewrite map_map. apply map_ext_Forall. exact H.
Qed.

(** scalar_preserving functions preserve depth. *)

Corollary deep_preserves_depth : forall f v,
  scalar_preserving f ->
  val_depth (broadcast_deep f v) = val_depth v.
Proof.
  intros f v Hsp. revert v.
  apply (val_ind_nested
    (fun v => val_depth (broadcast_deep f v) = val_depth v)).
  - intros n. simpl. destruct (Hsp n) as [m Hm]. rewrite Hm. reflexivity.
  - intros xs IH. rewrite broadcast_deep_coll.
    simpl. f_equal.
    change (fold_right Nat.max 0 (map val_depth (map (broadcast_deep f) xs)) =
            fold_right Nat.max 0 (map val_depth xs)).
    f_equal. rewrite map_map. apply map_ext_Forall. exact IH.
Qed.

(** Deep broadcast preserves collection length (immediate from map). *)

Theorem deep_preserves_length : forall f xs,
  length (map (broadcast_deep f) xs) = length xs.
Proof. intros. apply length_map. Qed.


(* ================================================================ *)
(* SECTION E : EQUIVALENCE WITH SHALLOW BROADCAST                     *)
(* ================================================================ *)

(** On flat data (all elements are scalars), deep and shallow
    broadcast produce the same result: no nested structure to
    recurse into. *)

Theorem deep_eq_shallow_flat : forall f xs,
  flat xs ->
  broadcast_deep f (Coll xs) = broadcast_map f (Coll xs).
Proof.
  intros f xs Hflat. rewrite broadcast_deep_coll. simpl. f_equal.
  induction xs as [| x xs' IH].
  - reflexivity.
  - simpl. f_equal.
    + destruct (Hflat x (or_introl eq_refl)) as [n Hn]. subst. reflexivity.
    + apply IH. intros v Hv. apply Hflat. right. exact Hv.
Qed.

(** Deep broadcast is one step of shallow broadcast composed with
    recursive descent: on Scalar apply f, on Coll map recursively. *)

Theorem deep_as_iterated_shallow : forall f v,
  broadcast_deep f v =
  match v with
  | Scalar _ => f v
  | Coll _   => broadcast_map (broadcast_deep f) v
  end.
Proof.
  intros f [n | xs].
  - reflexivity.
  - rewrite broadcast_deep_coll. reflexivity.
Qed.

(** On two-level data (list of flat lists), deep broadcast equals
    two nested shallow broadcasts. *)

Theorem deep_eq_two_levels : forall f xss,
  (forall xs, In xs xss -> flat xs) ->
  broadcast_deep f (Coll (map Coll xss)) =
  broadcast_map (broadcast_map f) (Coll (map Coll xss)).
Proof.
  intros f xss Hflat.
  rewrite broadcast_deep_coll. rewrite broadcast_two_levels. f_equal.
  rewrite map_map. revert Hflat.
  induction xss as [| xs xss' IH]; intros Hflat.
  - reflexivity.
  - cbn [map]. f_equal.
    + rewrite (deep_eq_shallow_flat f xs (Hflat xs (or_introl eq_refl))).
      reflexivity.
    + apply IH. intros ys Hys. apply Hflat. right. exact Hys.
Qed.

(** Uniform depth: all root-to-leaf paths have exactly [d] levels
    of Coll nesting before reaching a Scalar. *)

Fixpoint uniform_depth (d : nat) (v : Val) : Prop :=
  match d, v with
  | 0,    Scalar _ => True
  | S d', Coll xs  => forall x, In x xs -> uniform_depth d' x
  | _,    _        => False
  end.

(** N-fold iterated shallow broadcast: apply [broadcast_map] d times,
    then [f] at the leaves. *)

Fixpoint broadcast_iter (d : nat) (f : Val -> Val) (v : Val) : Val :=
  match d with
  | 0    => f v
  | S d' => broadcast_map (broadcast_iter d' f) v
  end.

(** For uniform-depth data, deep broadcast equals n-fold iterated
    shallow broadcast.  This is the general equivalence theorem. *)

Theorem deep_eq_n_levels : forall d f v,
  uniform_depth d v ->
  broadcast_deep f v = broadcast_iter d f v.
Proof.
  induction d as [| d' IHd]; intros f v Hu.
  - destruct v as [n | xs]; [reflexivity | destruct Hu].
  - destruct v as [n | xs]; [destruct Hu |].
    rewrite broadcast_deep_coll. simpl. f_equal.
    simpl in Hu. revert Hu.
    induction xs as [| x xs' IHxs]; intros Hu.
    + reflexivity.
    + simpl. f_equal.
      * apply IHd. apply Hu. left. reflexivity.
      * apply IHxs. intros y Hy. apply Hu. right. exact Hy.
Qed.


(* ================================================================ *)
(* SECTION F : CONCRETE EXAMPLES                                      *)
(* ================================================================ *)

Definition double (v : Val) : Val :=
  match v with Scalar n => Scalar (2 * n) | x => x end.

(** Flat list: deep = shallow (no nesting to recurse into). *)

Example ex_deep_flat :
  broadcast_deep double (Coll [Scalar 1; Scalar 2; Scalar 3]) =
  Coll [Scalar 2; Scalar 4; Scalar 6].
Proof. reflexivity. Qed.

(** Nested list: deep penetrates into sub-collections. *)

Example ex_deep_nested :
  broadcast_deep double (Coll [Scalar 1; Coll [Scalar 2; Scalar 3]]) =
  Coll [Scalar 2; Coll [Scalar 4; Scalar 6]].
Proof. reflexivity. Qed.

(** Identity on arbitrarily nested data. *)

Example ex_deep_identity :
  broadcast_deep id (Coll [Scalar 1; Coll [Scalar 2; Coll [Scalar 3]]]) =
  Coll [Scalar 1; Coll [Scalar 2; Coll [Scalar 3]]].
Proof. reflexivity. Qed.

(** Shape is preserved by a scalar_preserving function. *)

Example ex_deep_shape :
  val_shape (broadcast_deep double (Coll [Scalar 1; Coll [Scalar 2]])) =
  val_shape (Coll [Scalar 1; Coll [Scalar 2]]).
Proof. reflexivity. Qed.

(** Empty topos is a fixed point of deep broadcast. *)

Example ex_deep_empty :
  broadcast_deep double empty_topos = empty_topos.
Proof. reflexivity. Qed.

(** Counterexample: shallow does NOT equal deep on nested data.
    Shallow treats inner Coll as opaque; deep recurses into it. *)

Example ex_shallow_vs_deep :
  broadcast_deep double (Coll [Coll [Scalar 1]]) <>
  broadcast_map double (Coll [Coll [Scalar 1]]).
Proof. intro H. discriminate H. Qed.
