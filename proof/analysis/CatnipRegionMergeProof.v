(* FILE: proof/analysis/CatnipRegionMergeProof.v *)
(* CatnipRegionMergeProof.v - forward-only merge search correctness
 *
 * Source of truth:
 *   catnip_core/src/cfg/region.rs  (find_merge_point, get_all_successors)
 *
 * find_merge_point picks the first block that is (i) reachable from both
 * branches of an if, (ii) dominated by the if header. The Phase 4 property
 * harness showed this is wrong when the walk follows back-edges: from the
 * else branch, the walk re-enters the enclosing loop and reaches the then
 * branch's own blocks, so an inner while header (dominated by the if header)
 * was taken for the merge and the loop was reconstructed outside its branch.
 *
 * The fix cuts back-edges (an edge u -> v with v dominating u) from the
 * reachability walk. This file proves the two halves of that fix:
 *
 *   1. merge_unique_minimal: under the builder's structural invariant (every
 *      forward path from a branch to a common dominated block passes through
 *      the merge), the merge is the ONLY candidate that intercepts all
 *      candidates -- so any first-candidate forward search returns it.
 *      The proof is an infinite-descent argument on path lengths, the same
 *      skeleton as dominance antisymmetry.
 *
 *   2. The bug, concretely: on the minimized counterexample CFG (outer loop,
 *      if, inner while in the then branch), the inner header IS reachable
 *      from the else branch when back-edges are followed, and is NOT
 *      forward-reachable once they are cut.
 *
 * 8 theorems/lemmas + 3 examples, 0 Admitted.
 * Depends on CatnipDominanceProof (CFG model, paths, dominance).
 *)

From Coq Require Import List Arith Lia.
Import ListNotations.
From Catnip Require Import CatnipDominanceProof.


(* ================================================================ *)
(* A. Forward edges and forward reachability                          *)
(*                                                                    *)
(* A back-edge is an edge whose target dominates its source (the     *)
(* is_back_edge test in region.rs). The merge search walks forward   *)
(* edges only.                                                        *)
(* ================================================================ *)

Definition forward_edge (g : CFG) (u v : nat) : Prop :=
  g.(edge) u v /\ ~ dominates g v u.

Definition freach (g : CFG) (s x : nat) : Prop :=
  exists p, path (forward_edge g) s x p.

Lemma freach_refl : forall g s, freach g s s.
Proof.
  intros g s. exists [s]. constructor.
Qed.


(* ================================================================ *)
(* B. Interception antisymmetry (infinite descent)                    *)
(*                                                                    *)
(* If every path s->x contains m and every path s->m contains x,     *)
(* with x <> m, prefix extraction builds ever-shorter paths --        *)
(* impossible. Same skeleton as dom_no_path, over any edge relation.  *)
(* ================================================================ *)

Lemma intercept_no_path : forall (E : nat -> nat -> Prop) s x m n,
  x <> m ->
  (forall p, path E s x p -> In m p) ->
  (forall p, path E s m p -> In x p) ->
  forall p, path E s x p -> length p <= n -> False.
Proof.
  intros E s x m n Hne Hxm Hmx.
  induction n as [|n IHn].
  - intros p Hp Hlen. pose proof (path_length_pos _ _ _ _ Hp). lia.
  - intros p Hp Hlen.
    destruct (path_prefix_strict _ _ _ _ _ Hp (Hxm _ Hp)
                (fun Heq => Hne (eq_sym Heq)))
      as [p1 [Hp1 Hlt1]].
    destruct (path_prefix_strict _ _ _ _ _ Hp1 (Hmx _ Hp1) Hne)
      as [p2 [Hp2 Hlt2]].
    exact (IHn p2 Hp2 ltac:(lia)).
Qed.


(* ================================================================ *)
(* C. The merge search, abstractly                                    *)
(*                                                                    *)
(* h = if header, t/e = branch entries, m = the merge block the      *)
(* builder created. A candidate is what find_merge_point accepts:    *)
(* forward-reachable from both branches and dominated by the header. *)
(*                                                                    *)
(* The structural hypotheses are what the builder guarantees for an  *)
(* if region: the merge is itself a candidate, and every forward     *)
(* path from a branch entry to ANY candidate passes through it (a    *)
(* branch's only structured exits are the merge, or leaving the      *)
(* header's dominance region entirely -- break/continue/return --    *)
(* which the dominance test in the candidate definition rejects).    *)
(* ================================================================ *)

Section MergeSearch.
  Variable g : CFG.
  Variables h t e m : nat.

  Definition candidate (x : nat) : Prop :=
    freach g t x /\ freach g e x /\ dominates g h x.

  Hypothesis merge_candidate : candidate m.
  Hypothesis merge_intercepts : forall x p,
    candidate x -> path (forward_edge g) t x p -> In m p.

  (* Every candidate is discovered through m: on any forward path from
     the then entry to a candidate, m appears. Immediate from the
     structural hypothesis; stated as the search-facing corollary. *)
  Theorem candidate_behind_merge : forall x p,
    candidate x -> path (forward_edge g) t x p -> In m p.
  Proof.
    exact merge_intercepts.
  Qed.

  (* The merge is the unique candidate that intercepts all candidates.
     Any first-candidate search along forward edges therefore returns m:
     whatever candidate x it finds first, x intercepts the candidates
     found through it -- and interception both ways forces x = m. *)
  Theorem merge_unique_minimal : forall x,
    candidate x ->
    (forall c p, candidate c -> path (forward_edge g) t c p -> In x p) ->
    x = m.
  Proof.
    intros x Hx Hint.
    destruct (Nat.eq_dec x m) as [|Hne]; [assumption|].
    exfalso.
    (* a forward path t -> m exists (m is a candidate); x sits on it,
       so a forward path t -> x exists too -- seed of the descent *)
    destruct merge_candidate as [[pm Hpm] _].
    destruct (path_prefix _ _ _ _ _ Hpm (Hint m pm merge_candidate Hpm))
      as [px [Hpx _]].
    exact (intercept_no_path (forward_edge g) t x m (length px) Hne
             (fun p Hp => merge_intercepts x p Hx Hp)
             (fun p Hp => Hint m p merge_candidate Hp)
             px Hpx ltac:(lia)).
  Qed.

End MergeSearch.


(* ================================================================ *)
(* D. The counterexample CFG, concretely                              *)
(*                                                                    *)
(* The minimized property-harness case:                              *)
(*                                                                    *)
(*   0 entry                                                          *)
(*   1 outer while header      0->1, 1->2 (body), 1->9 (exit)        *)
(*   2 if header               2->3 (then), 2->6 (else)              *)
(*   3 then entry              3->4                                   *)
(*   4 inner while header      4->5 (body), 4->7 (inner exit)        *)
(*   5 inner body              5->4 (back-edge)                       *)
(*   6 else                    6->7                                   *)
(*   7 if merge                7->1 (outer back-edge)                 *)
(*   9 loop exit                                                      *)
(*                                                                    *)
(* Followed naively, 6 reaches 4 (through the outer back-edge and    *)
(* the then branch); forward-only it reaches exactly {6, 7}.          *)
(* ================================================================ *)

Definition nested_edge (x y : nat) : Prop :=
  (x = 0 /\ y = 1) \/ (x = 1 /\ y = 2) \/ (x = 1 /\ y = 9) \/
  (x = 2 /\ y = 3) \/ (x = 2 /\ y = 6) \/
  (x = 3 /\ y = 4) \/ (x = 4 /\ y = 5) \/ (x = 4 /\ y = 7) \/
  (x = 5 /\ y = 4) \/ (x = 6 /\ y = 7) \/ (x = 7 /\ y = 1).

Definition nested := mkCFG 0 nested_edge.

(* Followed naively (all edges), the else branch reaches the inner
   while header 4 that lives inside the THEN branch: this is the walk
   that made find_merge_point pick 4 for the merge. *)
Example naive_walk_reaches_inner_header :
  exists p, path nested_edge 6 4 p.
Proof.
  exists [6; 7; 1; 2; 3; 4].
  apply path_cons with 7. { unfold nested_edge; lia. }
  apply path_cons with 1. { unfold nested_edge; lia. }
  apply path_cons with 2. { unfold nested_edge; lia. }
  apply path_cons with 3. { unfold nested_edge; lia. }
  apply path_cons with 4. { unfold nested_edge; lia. }
  constructor.
Qed.

(* Every path from the entry to 7 goes through 1: the only edge out of
   the entry is 0 -> 1. So the outer back-edge 7 -> 1 is a back-edge.
   (The single-path case is dismissed by inversion: 0 <> 7.) *)
Lemma dominates_1_7 : dominates nested 1 7.
Proof.
  intros p Hp. simpl in Hp.
  inversion Hp as [| x y z p' Hedge Hpath]; subst.
  assert (Hy : y = 1) by (unfold nested_edge in Hedge; lia).
  subst y. simpl. right.
  exact (path_start_in _ _ _ _ Hpath).
Qed.

(* Forward-reachability from the else branch stops at the merge: the
   only edge out of 7 is the outer back-edge, which is cut. *)
Lemma freach_from_else : forall x p,
  path (forward_edge nested) 6 x p -> x = 6 \/ x = 7.
Proof.
  intros x p Hp.
  inversion Hp as [| a y z p' Hedge Hpath]; subst.
  - auto.
  - (* first forward edge out of 6: only 6 -> 7 *)
    destruct Hedge as [He _].
    assert (Hy : y = 7) by (simpl in He; unfold nested_edge in He; lia).
    subst y. right.
    (* from 7, the only edge is 7 -> 1, a back-edge: no forward step *)
    inversion Hpath as [| b w z' p'' Hedge' Hpath']; subst.
    + reflexivity.
    + exfalso.
      destruct Hedge' as [He' Hnb].
      assert (Hw : w = 1) by (simpl in He'; unfold nested_edge in He'; lia).
      subst w.
      exact (Hnb dominates_1_7).
Qed.

Example forward_walk_stops_at_merge : ~ freach nested 6 4.
Proof.
  intros [p Hp].
  destruct (freach_from_else _ _ Hp) as [H|H]; discriminate.
Qed.

(* The then branch still reaches the merge forward: 3 -> 4 -> 7, and
   neither hop is a back-edge (4 does not dominate 3: the else path
   0->1->2->6->7 never visits 4 -- likewise 7 does not dominate 4). *)
Lemma not_dominates_4_3 : ~ dominates nested 4 3.
Proof.
  intro H.
  assert (Hp : path nested_edge 0 3 [0; 1; 2; 3]).
  { apply path_cons with 1. { unfold nested_edge; lia. }
    apply path_cons with 2. { unfold nested_edge; lia. }
    apply path_cons with 3. { unfold nested_edge; lia. }
    constructor. }
  specialize (H _ Hp). simpl in H. lia.
Qed.

Lemma not_dominates_7_4 : ~ dominates nested 7 4.
Proof.
  intro H.
  assert (Hp : path nested_edge 0 4 [0; 1; 2; 3; 4]).
  { apply path_cons with 1. { unfold nested_edge; lia. }
    apply path_cons with 2. { unfold nested_edge; lia. }
    apply path_cons with 3. { unfold nested_edge; lia. }
    apply path_cons with 4. { unfold nested_edge; lia. }
    constructor. }
  specialize (H _ Hp). simpl in H. lia.
Qed.

Example then_branch_reaches_merge_forward : freach nested 3 7.
Proof.
  exists [3; 4; 7].
  apply path_cons with 4.
  { split.
    - simpl; unfold nested_edge; lia.
    - exact not_dominates_4_3. }
  apply path_cons with 7.
  { split.
    - simpl; unfold nested_edge; lia.
    - exact not_dominates_7_4. }
  constructor.
Qed.
