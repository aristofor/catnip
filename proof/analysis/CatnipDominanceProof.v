(* FILE: proof/analysis/CatnipDominanceProof.v *)
(* CatnipDominanceProof.v - CFG dominance analysis correctness
 *
 * Source of truth:
 *   catnip_rs/src/cfg/analysis.rs   (compute_dominators, idom, frontiers)
 *   catnip_rs/src/cfg/graph.rs      (ControlFlowGraph, BasicBlock)
 *
 * Models a finite directed graph with entry node and proves
 * structural properties of the dominance relation:
 *
 *   1. Reflexivity: every reachable node dominates itself
 *   2. Entry dominates all reachable nodes
 *   3. Transitivity via path prefix extraction
 *   4. Antisymmetry via path descent argument
 *   5. Immediate dominator uniqueness and tree structure
 *   6. Dominance frontier characterization (Cytron et al. 1991)
 *
 * Standalone: no dependencies on other Catnip proofs.
 *)

From Coq Require Import List Arith Lia.
Import ListNotations.


(* ================================================================ *)
(* A. CFG Model                                                       *)
(*                                                                    *)
(* Parametric in edge relation. Nodes are nat, matching              *)
(* ControlFlowGraph's usize block IDs.                               *)
(* ================================================================ *)

Record CFG := mkCFG {
  entry : nat;
  edge  : nat -> nat -> Prop;
}.


(* ================================================================ *)
(* B. Paths                                                           *)
(*                                                                    *)
(* A path from s to t is a non-empty node list [s; ...; t]           *)
(* following edges at each step.                                     *)
(* ================================================================ *)

Inductive path (E : nat -> nat -> Prop) : nat -> nat -> list nat -> Prop :=
| path_single : forall x, path E x x [x]
| path_cons   : forall x y z p,
    E x y -> path E y z p -> path E x z (x :: p).


(* ================================================================ *)
(* C. Definitions                                                     *)
(* ================================================================ *)

Definition reachable (g : CFG) (y : nat) : Prop :=
  exists p, path g.(edge) g.(entry) y p.

(* X dominates Y iff X appears on every path from entry to Y *)
Definition dominates (g : CFG) (x y : nat) : Prop :=
  forall p, path g.(edge) g.(entry) y p -> In x p.

Definition strict_dom (g : CFG) (x y : nat) : Prop :=
  dominates g x y /\ x <> y.


(* ================================================================ *)
(* D. Path Structural Lemmas                                          *)
(* ================================================================ *)

Lemma path_end_in : forall E s t p,
  path E s t p -> In t p.
Proof.
  intros E s t p H. induction H; simpl; auto.
Qed.

Lemma path_start_in : forall E s t p,
  path E s t p -> In s p.
Proof.
  intros E s t p H. destruct H; simpl; auto.
Qed.

Lemma path_length_pos : forall E s t p,
  path E s t p -> length p >= 1.
Proof.
  intros E s t p H. destruct H; simpl; lia.
Qed.

Lemma entry_reachable : forall g, reachable g g.(entry).
Proof.
  intro g. exists [g.(entry)]. constructor.
Qed.


(* ================================================================ *)
(* E. Reflexivity and Entry                                           *)
(* ================================================================ *)

Theorem dom_refl : forall g y,
  reachable g y -> dominates g y y.
Proof.
  intros g y _ p Hp. exact (path_end_in _ _ _ _ Hp).
Qed.

Theorem entry_dom_all : forall g y,
  reachable g y -> dominates g g.(entry) y.
Proof.
  intros g y _ p Hp. exact (path_start_in _ _ _ _ Hp).
Qed.


(* ================================================================ *)
(* F. Path Prefix Extraction                                          *)
(*                                                                    *)
(* If m is on a path from s to t, there exists a sub-path from      *)
(* s to m whose nodes are all on the original path.                  *)
(* ================================================================ *)

Lemma path_prefix : forall E s t p m,
  path E s t p -> In m p ->
  exists p1, path E s m p1 /\ (forall x, In x p1 -> In x p).
Proof.
  intros E s t p m H.
  induction H as [s' | s' next t' p' Hedge Hpath IH]; intro Hin.
  - destruct Hin as [<-|[]].
    exists [s']. split; [constructor | auto].
  - destruct Hin as [<-|Hin'].
    + exists [s']. split; [constructor |].
      intros a Ha. destruct Ha as [<-|[]]. simpl. auto.
    + destruct (IH Hin') as [p1 [Hp1 Hsub]].
      exists (s' :: p1). split.
      * econstructor; eassumption.
      * intros a Ha. destruct Ha as [<-|Ha'].
        -- simpl. auto.
        -- simpl. right. exact (Hsub _ Ha').
Qed.


(* ================================================================ *)
(* G. Transitivity                                                    *)
(* ================================================================ *)

Theorem dom_trans : forall g x y z,
  dominates g x y -> dominates g y z -> dominates g x z.
Proof.
  intros g x y z Hxy Hyz p Hp.
  assert (Hy : In y p) by (apply Hyz; exact Hp).
  destruct (path_prefix _ _ _ _ _ Hp Hy) as [p1 [Hp1 Hsub]].
  apply Hsub. apply Hxy. exact Hp1.
Qed.

Lemma dom_reachable : forall g x y,
  dominates g x y -> reachable g y -> reachable g x.
Proof.
  intros g x y Hdom [p Hp].
  destruct (path_prefix _ _ _ _ _ Hp (Hdom _ Hp)) as [p1 [Hp1 _]].
  exists p1. exact Hp1.
Qed.


(* ================================================================ *)
(* H. Antisymmetry                                                    *)
(*                                                                    *)
(* If dom(x,y) and dom(y,x) with x <> y, both must appear on       *)
(* every path to the other, creating shorter and shorter paths.      *)
(* This infinite descent on nat is impossible.                       *)
(* ================================================================ *)

Lemma path_prefix_strict : forall E s t p m,
  path E s t p -> In m p -> m <> t ->
  exists p1, path E s m p1 /\ length p1 < length p.
Proof.
  intros E s t p m H.
  induction H as [s' | s' next t' p' Hedge Hpath IH]; intros Hin Hne.
  - destruct Hin as [<-|[]].
    exfalso. exact (Hne eq_refl).
  - destruct Hin as [<-|Hin'].
    + exists [s']. split; [constructor |].
      simpl. pose proof (path_length_pos _ _ _ _ Hpath). lia.
    + destruct (IH Hin' Hne) as [p1 [Hp1 Hlt]].
      exists (s' :: p1). split.
      * econstructor; eassumption.
      * simpl. lia.
Qed.

Lemma dom_no_path : forall g x y n,
  x <> y ->
  dominates g x y -> dominates g y x ->
  forall p, path g.(edge) g.(entry) y p -> length p <= n -> False.
Proof.
  intros g x y n Hne Hxy Hyx.
  induction n as [|n IHn].
  - intros p Hp Hlen. pose proof (path_length_pos _ _ _ _ Hp). lia.
  - intros p Hp Hlen.
    destruct (path_prefix_strict _ _ _ _ _ Hp (Hxy _ Hp) Hne)
      as [p1 [Hp1 Hlt1]].
    assert (Hne' : y <> x).
    { intro Heq. exact (Hne (eq_sym Heq)). }
    destruct (path_prefix_strict _ _ _ _ _ Hp1 (Hyx _ Hp1) Hne')
      as [p2 [Hp2 Hlt2]].
    exact (IHn p2 Hp2 ltac:(lia)).
Qed.

Theorem dom_antisym : forall g x y,
  reachable g y ->
  dominates g x y -> dominates g y x -> x = y.
Proof.
  intros g x y [p Hp] Hxy Hyx.
  destruct (Nat.eq_dec x y) as [|Hne]; [assumption|].
  exfalso.
  exact (dom_no_path g x y (length p) Hne Hxy Hyx p Hp ltac:(lia)).
Qed.


(* ================================================================ *)
(* I. Immediate Dominator                                             *)
(*                                                                    *)
(* idom(y) is the unique strict dominator closest to y, dominated   *)
(* by all other strict dominators of y.                              *)
(* Matches compute_immediate_dominators in analysis.rs.              *)
(* ================================================================ *)

Definition is_idom (g : CFG) (d y : nat) : Prop :=
  strict_dom g d y /\
  forall z, strict_dom g z y -> dominates g z d.

Theorem idom_unique : forall g d1 d2 y,
  reachable g y ->
  is_idom g d1 y -> is_idom g d2 y -> d1 = d2.
Proof.
  intros g d1 d2 y Hy [[Hd1 Hne1] Hc1] [[Hd2 Hne2] Hc2].
  apply (dom_antisym g d1 d2 (dom_reachable _ _ _ Hd2 Hy)).
  - apply Hc2. split; assumption.
  - apply Hc1. split; assumption.
Qed.

Theorem idom_dominates : forall g d y,
  is_idom g d y -> dominates g d y.
Proof.
  intros g d y [[Hdom _] _]. exact Hdom.
Qed.

(* Entry has no idom: it is the root of the dominator tree *)
Theorem entry_no_idom : forall g d,
  ~ is_idom g d g.(entry).
Proof.
  intros g d [[Hdom Hne] _].
  apply Hne.
  apply (dom_antisym g d g.(entry) (entry_reachable g) Hdom).
  apply entry_dom_all.
  exact (dom_reachable _ _ _ Hdom (entry_reachable g)).
Qed.

(* No two-cycle in idom relation *)
Theorem idom_no_cycle : forall g d y,
  reachable g y ->
  is_idom g d y -> ~ is_idom g y d.
Proof.
  intros g d y Hy [[Hdy Hned] _] [[Hyd _] _].
  exact (Hned (dom_antisym g d y Hy Hdy Hyd)).
Qed.


(* ================================================================ *)
(* J. Dominance Frontier                                              *)
(*                                                                    *)
(* DF(x) = { y | ∃ pred p of y with x dom p, but x ¬sdom y }       *)
(* Cytron et al. 1991, SSA construction paper.                       *)
(* Matches compute_dominance_frontiers in analysis.rs.               *)
(* ================================================================ *)

Definition in_dom_frontier (g : CFG) (x y : nat) : Prop :=
  (exists p, g.(edge) p y /\ dominates g x p) /\
  ~ strict_dom g x y.

(* Entry has empty frontier for all non-entry nodes *)
Theorem entry_frontier_empty : forall g y,
  reachable g y -> y <> g.(entry) ->
  ~ in_dom_frontier g g.(entry) y.
Proof.
  intros g y Hreach Hne [_ Hnsdom].
  apply Hnsdom. split.
  - apply entry_dom_all. exact Hreach.
  - intro Heq. exact (Hne (eq_sym Heq)).
Qed.

(* Strict domination implies not in own frontier *)
Theorem sdom_not_in_frontier : forall g x y,
  strict_dom g x y -> ~ in_dom_frontier g x y.
Proof.
  intros g x y Hsdom [_ Hnsdom]. exact (Hnsdom Hsdom).
Qed.


(* ================================================================ *)
(* K. Concrete Examples                                               *)
(*                                                                    *)
(* Diamond CFG (if/else merge):                                      *)
(*                                                                    *)
(*     0 (entry)                                                     *)
(*    / \                                                            *)
(*   1   2                                                           *)
(*    \ /                                                            *)
(*     3 (merge)                                                     *)
(* ================================================================ *)

Definition diamond_edge (x y : nat) : Prop :=
  (x = 0 /\ y = 1) \/ (x = 0 /\ y = 2) \/
  (x = 1 /\ y = 3) \/ (x = 2 /\ y = 3).

Definition diamond := mkCFG 0 diamond_edge.

Example diamond_reachable_3 : reachable diamond 3.
Proof.
  exists [0; 1; 3].
  apply path_cons with 1.
  - left. auto.
  - apply path_cons with 3.
    + right. right. left. auto.
    + constructor.
Qed.

Example diamond_entry_dom_3 :
  dominates diamond 0 3.
Proof.
  intros p Hp. exact (path_start_in _ _ _ _ Hp).
Qed.

(* Branch 1 does NOT dominate merge: path 0→2→3 bypasses 1 *)
Example diamond_1_not_dom_3 :
  ~ dominates diamond 1 3.
Proof.
  intro H.
  assert (Hp : path diamond_edge 0 3 [0; 2; 3]).
  { apply path_cons with 2.
    - right. left. auto.
    - apply path_cons with 3.
      + right. right. right. auto.
      + constructor. }
  specialize (H _ Hp). simpl in H.
  destruct H as [H|[H|[H|[]]]]; discriminate.
Qed.

(* Merge (3) is in the dominance frontier of branch 1 *)
Example diamond_3_in_df_1 :
  in_dom_frontier diamond 1 3.
Proof.
  split.
  - exists 1. split.
    + right. right. left. auto.
    + intros p Hp. exact (path_end_in _ _ _ _ Hp).
  - intro Hsdom. exact (diamond_1_not_dom_3 (proj1 Hsdom)).
Qed.

Example diamond_entry_sdom_1 : strict_dom diamond 0 1.
Proof.
  split.
  - intros p Hp. exact (path_start_in _ _ _ _ Hp).
  - discriminate.
Qed.
