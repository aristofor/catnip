(* FILE: proof/analysis/CatnipParallelCopyProof.v *)
(* Parallel-copy sequentialization for SSA destruction.
 *
 * Source: catnip_core/src/cfg/ssa_destruction.rs
 *         (sequentialize_parallel_copies)
 *
 * A phi at a CFG join imposes a set of *parallel* copies on each incoming
 * edge: every destination is written from the *initial* value of its source,
 * simultaneously. Emitting them one after another corrupts a value when the
 * copies form a cycle -- the swap / lost-copy problem (Briggs et al. 1998;
 * Boissinot et al. 2009). The Rust solver serializes a batch, spilling one
 * cycle node to a scratch temporary to break each cycle.
 *
 * This file proves the load-bearing identity: breaking a cycle of *any* length
 * with a single scratch temporary realizes the parallel rotation exactly, on
 * every location. The swap and the 3-cycle follow as concrete instances; a
 * companion example pins *why* the scratch is needed (the naive two-copy swap
 * loses a value), and the acyclic chain shows the leaf-first order needs no
 * scratch.
 *
 * Proves:
 *   - upd_same, upd_other, exec_cons          (copy semantics)
 *   - in_skipn_l                              (list helper)
 *   - chain_untouched, chain_pos              (chain body, universal)
 *   - cycle_break_correct, cycle_break_outside (n-cycle, universal)
 *   - swap_demo, three_cycle_demo             (instances of the theorem)
 *   - naive_swap_corrupts                     (necessity of the scratch)
 *   - acyclic_chain_demo                      (acyclic case, no scratch)
 *   - exec_frame_off, dead_copy_invisible     (liveness gate: dead copy invisible)
 *
 * Depends on: Coq stdlib only.
 *
 * 0 Admitted.
 *)

From Coq Require Import List Arith Lia Bool.
Import ListNotations.

(* ================================================================ *)
(* A. Copy semantics                                                 *)
(*                                                                    *)
(* A location holds a value; a copy (d, s) writes s's current value  *)
(* into d. A sequence executes left to right (fold_left). Mirrors    *)
(* run_seq in ssa_destruction.rs's tests.                            *)
(* ================================================================ *)

Definition loc := nat.
Definition state := loc -> nat.

Definition upd (s : state) (d : loc) (v : nat) : state :=
  fun x => if Nat.eqb x d then v else s x.

Lemma upd_same : forall s d v, upd s d v d = v.
Proof. intros. unfold upd. rewrite Nat.eqb_refl. reflexivity. Qed.

Lemma upd_other : forall s d v x, x <> d -> upd s d v x = s x.
Proof.
  intros s d v x H. unfold upd.
  apply Nat.eqb_neq in H. rewrite H. reflexivity.
Qed.

Definition do_copy (s : state) (c : loc * loc) : state :=
  upd s (fst c) (s (snd c)).

Definition exec (seq : list (loc * loc)) (s : state) : state :=
  fold_left do_copy seq s.

Lemma exec_cons : forall c seq s, exec (c :: seq) s = exec seq (do_copy s c).
Proof. intros. unfold exec. reflexivity. Qed.

(* ================================================================ *)
(* B. List helper                                                    *)
(* ================================================================ *)

Lemma in_skipn_l : forall (A : Type) n (l : list A) a,
  In a (skipn n l) -> In a l.
Proof.
  induction n as [|n IH]; intros l a H.
  - simpl in H. exact H.
  - destruct l as [|x l'].
    + simpl in H. contradiction.
    + simpl in H. right. apply (IH l' a H).
Qed.

(* ================================================================ *)
(* C. Cycle break                                                    *)
(*                                                                    *)
(* A cycle over distinct locations xs = [x0; ...; x_{k-1}] with a     *)
(* fresh scratch t is serialized as                                  *)
(*     (t, x0) :: chain xs t                                          *)
(* where chain xs t = [(x0,x1); (x1,x2); ...; (x_{k-1}, t)].          *)
(* The initial save spills x0 into t; the chain shifts every node    *)
(* into its predecessor and the last node reads the saved value.     *)
(* ================================================================ *)

Fixpoint chain (xs : list loc) (t : loc) : list (loc * loc) :=
  match xs with
  | [] => []
  | x :: xs' => (x, hd t xs') :: chain xs' t
  end.

(* A location outside the chain's destinations is left untouched. *)
Lemma chain_untouched : forall xs t s z,
  ~ In z xs -> exec (chain xs t) s z = s z.
Proof.
  induction xs as [|x xs' IH]; intros t s z Hz.
  - simpl. reflexivity.
  - assert (z <> x /\ ~ In z xs') as [Hzx Hz'].
    { split.
      - intro H. apply Hz. left. symmetry. exact H.
      - intro H. apply Hz. right. exact H. }
    cbn [chain]. rewrite exec_cons.
    rewrite (IH t (do_copy s (x, hd t xs')) z Hz').
    unfold do_copy. cbn [fst snd]. apply upd_other. exact Hzx.
Qed.

(* Position i of the chain, run on any state s, receives the value that s
   held at the head of the (i+1)-suffix of xs (its successor), and the last
   position reads s at the scratch t. Universal over distinct xs. *)
Lemma chain_pos : forall xs t s i,
  NoDup xs -> ~ In t xs -> i < length xs ->
  exec (chain xs t) s (nth i xs 0) = s (hd t (skipn (S i) xs)).
Proof.
  intros xs. induction xs as [|x xs' IH]; intros t s i Hnd Ht Hi.
  - simpl in Hi. lia.
  - apply NoDup_cons_iff in Hnd. destruct Hnd as [Hxni Hnd'].
    assert (Htx : t <> x).
    { intro H. apply Ht. left. symmetry. exact H. }
    assert (Htxs : ~ In t xs').
    { intro H. apply Ht. right. exact H. }
    cbn [chain]. rewrite exec_cons.
    destruct i as [|j].
    + cbn [nth skipn].
      rewrite (chain_untouched xs' t (do_copy s (x, hd t xs')) x Hxni).
      unfold do_copy. cbn [fst snd]. rewrite upd_same. reflexivity.
    + cbn [nth]. cbn [length] in Hi.
      assert (Hj : j < length xs') by lia.
      rewrite (IH t (do_copy s (x, hd t xs')) j Hnd' Htxs Hj).
      change (skipn (S (S j)) (x :: xs')) with (skipn (S j) xs').
      unfold do_copy. cbn [fst snd]. apply upd_other.
      destruct (skipn (S j) xs') as [|y ys] eqn:Hsk.
      * cbn [hd]. exact Htx.
      * cbn [hd]. intro Hyx. subst y. apply Hxni.
        apply (in_skipn_l _ (S j) xs' x). rewrite Hsk. left. reflexivity.
Qed.

(* The headline theorem. For distinct xs and a fresh scratch t, the broken
   sequence realizes the parallel rotation: node at position i ends holding
   the *initial* value of its successor (the last node wraps to x0, carried
   through the scratch). Universal over cycle length. *)
Theorem cycle_break_correct : forall xs t s,
  NoDup xs -> ~ In t xs ->
  forall i, i < length xs ->
    exec ((t, hd 0 xs) :: chain xs t) s (nth i xs 0)
      = s (hd (hd 0 xs) (skipn (S i) xs)).
Proof.
  intros xs t s Hnd Ht i Hi.
  set (s1 := do_copy s (t, hd 0 xs)).
  assert (Hstate : exec ((t, hd 0 xs) :: chain xs t) s = exec (chain xs t) s1)
    by (unfold s1; apply exec_cons).
  rewrite Hstate.
  transitivity (s1 (hd t (skipn (S i) xs))).
  { apply (chain_pos xs t s1 i Hnd Ht Hi). }
  destruct (skipn (S i) xs) as [|y ys] eqn:Hsk.
  - cbn [hd]. unfold s1, do_copy. cbn [fst snd]. rewrite upd_same. reflexivity.
  - cbn [hd]. unfold s1, do_copy. cbn [fst snd]. apply upd_other.
    intro Hyt. subst y. apply Ht.
    apply (in_skipn_l _ (S i) xs t). rewrite Hsk. left. reflexivity.
Qed.

(* Locations outside the cycle (and distinct from the scratch) are untouched. *)
Theorem cycle_break_outside : forall xs t s z,
  ~ In z xs -> z <> t ->
  exec ((t, hd 0 xs) :: chain xs t) s z = s z.
Proof.
  intros xs t s z Hz Hzt.
  set (s1 := do_copy s (t, hd 0 xs)).
  assert (Hstate : exec ((t, hd 0 xs) :: chain xs t) s = exec (chain xs t) s1)
    by (unfold s1; apply exec_cons).
  rewrite Hstate.
  transitivity (s1 z).
  { apply (chain_untouched xs t s1 z Hz). }
  unfold s1, do_copy. cbn [fst snd]. apply upd_other. exact Hzt.
Qed.

(* ================================================================ *)
(* D. Instances and the necessity of the scratch                     *)
(* ================================================================ *)

(* Swap {1 <- 2, 2 <- 1} broken with scratch 0: [(0,1);(1,2);(2,0)]. *)
Example swap_demo : forall s : state,
  exec [(0, 1); (1, 2); (2, 0)] s 1 = s 2
  /\ exec [(0, 1); (1, 2); (2, 0)] s 2 = s 1.
Proof.
  intro s.
  assert (Hnd : NoDup [1; 2]).
  { constructor.
    - intros [H | H]; [ discriminate H | exact H ].
    - constructor; [ intros H; exact H | constructor ]. }
  assert (Ht : ~ In 0 [1; 2]).
  { intros [H | [H | H]]; [ discriminate H | discriminate H | exact H ]. }
  split.
  - assert (H := cycle_break_correct [1; 2] 0 s Hnd Ht 0 (ltac:(simpl; lia))).
    cbn in H. exact H.
  - assert (H := cycle_break_correct [1; 2] 0 s Hnd Ht 1 (ltac:(simpl; lia))).
    cbn in H. exact H.
Qed.

(* 3-cycle {1<-2, 2<-3, 3<-1} broken with scratch 0. *)
Example three_cycle_demo : forall s : state,
  exec [(0, 1); (1, 2); (2, 3); (3, 0)] s 1 = s 2
  /\ exec [(0, 1); (1, 2); (2, 3); (3, 0)] s 2 = s 3
  /\ exec [(0, 1); (1, 2); (2, 3); (3, 0)] s 3 = s 1.
Proof.
  intro s.
  assert (Hnd : NoDup [1; 2; 3]).
  { constructor.
    - intros [H | [H | H]]; [ discriminate H | discriminate H | exact H ].
    - constructor.
      + intros [H | H]; [ discriminate H | exact H ].
      + constructor; [ intros H; exact H | constructor ]. }
  assert (Ht : ~ In 0 [1; 2; 3]).
  { intros [H | [H | [H | H]]];
      [ discriminate H | discriminate H | discriminate H | exact H ]. }
  split; [| split].
  - assert (H := cycle_break_correct [1;2;3] 0 s Hnd Ht 0 (ltac:(simpl; lia))).
    cbn in H. exact H.
  - assert (H := cycle_break_correct [1;2;3] 0 s Hnd Ht 1 (ltac:(simpl; lia))).
    cbn in H. exact H.
  - assert (H := cycle_break_correct [1;2;3] 0 s Hnd Ht 2 (ltac:(simpl; lia))).
    cbn in H. exact H.
Qed.

(* Necessity: the naive swap [(1,2);(2,1)] -- no scratch -- loses location 1's
   value. Both locations end holding the old value of 2. Justifies the spill. *)
Example naive_swap_corrupts : forall s : state,
  exec [(1, 2); (2, 1)] s 1 = s 2
  /\ exec [(1, 2); (2, 1)] s 2 = s 2.
Proof.
  intro s. split; unfold exec, do_copy, upd; cbn; reflexivity.
Qed.

(* Acyclic batch {1 <- 2, 2 <- 3} serialized leaf-first as [(1,2);(2,3)] needs
   no scratch: 1 reads the old 2 before 2 is overwritten. *)
Example acyclic_chain_demo : forall s : state,
  exec [(1, 2); (2, 3)] s 1 = s 2
  /\ exec [(1, 2); (2, 3)] s 2 = s 3.
Proof.
  intro s. split; unfold exec, do_copy, upd; cbn; reflexivity.
Qed.

(* ================================================================ *)
(* E. Liveness gate: a dead copy is observationally invisible        *)
(*                                                                    *)
(* Justifies materialize_phis skipping a phi whose variable is not    *)
(* live-in: its materialized copy writes a location nothing reads     *)
(* afterwards, so omitting it changes no other location's value.      *)
(* ================================================================ *)

(* A copy sequence never reads location `x` as a source. *)
Definition not_reads (seq : list (loc * loc)) (x : loc) : Prop :=
  forall c, In c seq -> snd c <> x.

(* Executing a sequence that never reads `x` is insensitive to `x`'s value: two
   states agreeing everywhere off `x` keep agreeing everywhere off `x`. *)
Lemma exec_frame_off : forall seq x s1 s2,
  not_reads seq x ->
  (forall z, z <> x -> s1 z = s2 z) ->
  forall z, z <> x -> exec seq s1 z = exec seq s2 z.
Proof.
  induction seq as [| c seq IH]; intros x s1 s2 Hnr Hagree z Hz.
  - apply Hagree; exact Hz.
  - rewrite !exec_cons.
    apply (IH x (do_copy s1 c) (do_copy s2 c)).
    + intros c' Hin. apply Hnr. right. exact Hin.
    + intros w Hw. unfold do_copy.
      destruct (Nat.eq_dec w (fst c)) as [-> | Hne].
      * rewrite !upd_same. apply Hagree. apply Hnr. left. reflexivity.
      * rewrite (upd_other s1 (fst c) (s1 (snd c)) w Hne).
        rewrite (upd_other s2 (fst c) (s2 (snd c)) w Hne).
        apply Hagree. exact Hw.
    + exact Hz.
Qed.

(* Omitting a copy whose destination `d` is never read afterwards leaves every
   other location unchanged -- exactly the dead materialization the gate drops. *)
Theorem dead_copy_invisible : forall d s rest sigma,
  not_reads rest d ->
  forall z, z <> d -> exec ((d, s) :: rest) sigma z = exec rest sigma z.
Proof.
  intros d s rest sigma Hnr z Hz.
  rewrite exec_cons.
  apply (exec_frame_off rest d (do_copy sigma (d, s)) sigma Hnr).
  - intros w Hw. unfold do_copy. cbn [fst snd]. apply upd_other. exact Hw.
  - exact Hz.
Qed.
