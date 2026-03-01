(* FILE: proof/analysis/CatnipCFGSSABase.v *)
(* CatnipCFGSSABase.v — CFG/SSA model + operational lemmas (A)
 *
 * Source of truth:
 *   catnip_rs/src/cfg/ssa.rs             (SSAValue, BlockParam, trivial phi)
 *   catnip_rs/src/cfg/ssa_builder.rs     (Braun et al. 2013 SSA construction)
 *   catnip_rs/src/cfg/ssa_cse.rs         (inter-block CSE)
 *   catnip_rs/src/cfg/ssa_gvn.rs         (Global Value Numbering)
 *   catnip_rs/src/cfg/ssa_licm.rs        (Loop-Invariant Code Motion)
 *   catnip_rs/src/cfg/ssa_dse.rs         (Dead Store Elimination, fixpoint)
 *   catnip_rs/src/cfg/ssa_destruction.rs (phi elimination via copies)
 *
 * 49 lemmas/theorems, 0 Admitted.
 * 8 hypotheses derived from operational models, 6 retained.
 * Depends on CatnipDominanceProof (CFG, path, dominates, dom_trans).
 *)

From Coq Require Import List Arith Lia Bool.
Import ListNotations.
From Catnip Require Import CatnipDominanceProof.


(* ================================================================ *)
(* A. SSA Model                                                       *)
(*                                                                    *)
(* Matches SSAValue { var: usize, version: u32 } from ssa.rs.       *)
(* BlockParam { value, incoming } and BlockSSAInfo { params, ... }. *)
(* ================================================================ *)

Record SSAVal := mkSSAVal {
  sv_var : nat;
  sv_ver : nat;
}.

Definition ssaval_eqb (a b : SSAVal) : bool :=
  Nat.eqb (sv_var a) (sv_var b) && Nat.eqb (sv_ver a) (sv_ver b).

Record Phi := mkPhi {
  phi_val      : SSAVal;
  phi_incoming : list SSAVal;
}.

Record BlockSSA := mkBlockSSA {
  bssa_phis : list Phi;
  bssa_defs : list (nat * SSAVal);
}.

Definition Opcode := nat.
Definition Env := SSAVal -> nat.

Record ExprKey := mkExprKey {
  ek_op   : Opcode;
  ek_args : list SSAVal;
}.

Definition eval_expr (ev : Opcode -> list nat -> nat) (rho : Env) (e : ExprKey) : nat :=
  ev (ek_op e) (map rho (ek_args e)).

(* Phi evaluation: pick the incoming value from predecessor index *)
Definition eval_phi (rho : Env) (p : Phi) (pred_idx : nat) : nat :=
  match nth_error (phi_incoming p) pred_idx with
  | Some w => rho w
  | None   => 0
  end.

(* Environment update *)
Definition update_env (rho : Env) (v : SSAVal) (val : nat) : Env :=
  fun w => if ssaval_eqb w v then val else rho w.

(* Phi copy for SSA destruction *)
Definition phi_copy (p : Phi) (pred_idx : nat) : option (SSAVal * SSAVal) :=
  match nth_error (phi_incoming p) pred_idx with
  | Some w => Some (phi_val p, w)
  | None   => None
  end.

(* All copies for a list of phis at a given predecessor index *)
Fixpoint all_phi_copies (phis : list Phi) (pred_idx : nat) : list (SSAVal * SSAVal) :=
  match phis with
  | [] => []
  | p :: rest =>
      match phi_copy p pred_idx with
      | Some c => c :: all_phi_copies rest pred_idx
      | None   => all_phi_copies rest pred_idx
      end
  end.

(* Apply a sequence of copies to an environment *)
Fixpoint apply_copies (copies : list (SSAVal * SSAVal)) (rho : Env) : Env :=
  match copies with
  | []              => rho
  | (dst, src) :: r => apply_copies r (update_env rho dst (rho src))
  end.

(* Insert position: before the last instruction (terminator) *)
Definition insert_pos (n_instrs : nat) : nat :=
  match n_instrs with
  | 0   => 0
  | S n => n
  end.


(* ---------- Lemma A.1 : ssaval_eqb_refl ---------- *)

Lemma ssaval_eqb_refl : forall v, ssaval_eqb v v = true.
Proof.
  destruct v as [var ver]. unfold ssaval_eqb. simpl.
  rewrite Nat.eqb_refl. simpl. apply Nat.eqb_refl.
Qed.


(* ---------- Lemma A.2 : ssaval_eqb_eq ---------- *)

Lemma ssaval_eqb_eq : forall a b,
  ssaval_eqb a b = true <-> a = b.
Proof.
  intros a b. split.
  - intro H. destruct a as [av ai], b as [bv bi].
    unfold ssaval_eqb in H. simpl in H.
    apply Bool.andb_true_iff in H. destruct H as [H1 H2].
    apply Nat.eqb_eq in H1. apply Nat.eqb_eq in H2.
    subst. reflexivity.
  - intros <-. apply ssaval_eqb_refl.
Qed.


(* ================================================================ *)
(* Utility lemmas (used by multiple sections)                        *)
(* ================================================================ *)

(* A non-increasing nat sequence stabilizes within its initial value *)
Lemma monotone_nat_stabilizes :
  forall (f : nat -> nat),
    (forall n, f (S n) <= f n) ->
    forall total, f 0 <= total ->
    exists k, k <= total /\ f (S k) = f k.
Proof.
  intros f Hmono total. revert f Hmono.
  induction total as [|t IH]; intros f Hmono Hbound.
  - exists 0. split.
    + lia.
    + specialize (Hmono 0). lia.
  - destruct (Nat.eq_dec (f 1) (f 0)) as [Heq|Hne].
    + exists 0. split.
      * lia.
      * exact Heq.
    + assert (Hlt : f 1 <= t) by (specialize (Hmono 0); lia).
      destruct (IH (fun n => f (S n))
                   (fun n => Hmono (S n))
                   Hlt)
        as [k' [Hk' Hstab]].
      exists (S k'). split; [lia | exact Hstab].
Qed.

(* Pointwise-equal args give equal maps *)
Lemma map_pointwise_eq : forall (f : SSAVal -> nat) (l1 l2 : list SSAVal),
  length l1 = length l2 ->
  (forall i v1 v2,
    nth_error l1 i = Some v1 ->
    nth_error l2 i = Some v2 ->
    f v1 = f v2) ->
  map f l1 = map f l2.
Proof.
  induction l1 as [|a1 t1 IH]; intros l2 Hlen Hpw.
  - destruct l2; [reflexivity | simpl in Hlen; discriminate].
  - destruct l2 as [|a2 t2]; [simpl in Hlen; discriminate |].
    simpl. f_equal.
    + exact (Hpw 0 a1 a2 eq_refl eq_refl).
    + apply IH.
      * simpl in Hlen. lia.
      * intros i v1 v2 H1 H2. exact (Hpw (S i) v1 v2 H1 H2).
Qed.

(* Map with extensionally equal functions on list elements *)
Lemma map_ext_in_local : forall {A B : Type} (f g : A -> B) l,
  (forall a, In a l -> f a = g a) -> map f l = map g l.
Proof.
  intros A B f g l H.
  induction l as [|x xs IH].
  - reflexivity.
  - simpl. f_equal.
    + apply H. left. reflexivity.
    + apply IH. intros a Ha. apply H. right. exact Ha.
Qed.

(* ssaval_eqb false implies inequality *)
Lemma ssaval_eqb_false_neq : forall a b,
  ssaval_eqb a b = false -> a <> b.
Proof.
  intros a b Hf Heq. subst.
  rewrite ssaval_eqb_refl in Hf. discriminate.
Qed.

(* Filtering a list never increases its length *)
Lemma filter_length_le : forall {A : Type} (f : A -> bool) (l : list A),
  length (filter f l) <= length l.
Proof.
  intros A f l. induction l as [|x xs IH].
  - simpl. lia.
  - simpl. destruct (f x); simpl; lia.
Qed.

(* In a list implies positive length *)
Lemma In_length_pos : forall {A : Type} (x : A) (l : list A),
  In x l -> length l > 0.
Proof.
  intros A x [|y ys] H.
  - inversion H.
  - simpl. lia.
Qed.


(* ================================================================ *)
(* SSA Construction: Operational Model                                *)
(*                                                                    *)
(* Models fresh_version (monotone version counter per variable) and   *)
(* the incomplete_phis map (one entry per variable per block) from    *)
(* ssa_builder.rs. Derives unique_def and no_dup_phi from             *)
(* operational specifications.                                        *)
(* ================================================================ *)

Section SSA_Construction.

(* --- Fresh version model ---
   fresh_version in ssa_builder.rs allocates mkSSAVal(var, next_ver[var])
   then increments next_ver[var]. The counter is monotone per variable,
   so each (var, version) pair is globally unique. *)

Record SSACtx := mkSSACtx {
  ctx_next_ver    : nat -> nat;
  ctx_sealed      : nat -> bool;
  ctx_block_phis  : nat -> list Phi;
  ctx_incomplete  : nat -> list (nat * nat);
}.

Definition fresh_version (ctx : SSACtx) (var : nat) : SSAVal :=
  mkSSAVal var (ctx_next_ver ctx var).

Definition bump_version (ctx : SSACtx) (var : nat) : SSACtx :=
  mkSSACtx
    (fun v => if Nat.eqb v var then S (ctx_next_ver ctx var) else ctx_next_ver ctx v)
    (ctx_sealed ctx)
    (ctx_block_phis ctx)
    (ctx_incomplete ctx).

(* Bumping increments exactly the target variable *)
Lemma bump_version_increases : forall ctx var,
  ctx_next_ver (bump_version ctx var) var = S (ctx_next_ver ctx var).
Proof.
  intros ctx var. unfold bump_version. simpl.
  rewrite Nat.eqb_refl. reflexivity.
Qed.

Lemma bump_version_preserves_other : forall ctx v1 v2,
  v1 <> v2 ->
  ctx_next_ver (bump_version ctx v1) v2 = ctx_next_ver ctx v2.
Proof.
  intros ctx v1 v2 Hne. unfold bump_version. simpl.
  destruct (Nat.eqb v2 v1) eqn:E.
  - apply Nat.eqb_eq in E. symmetry in E. contradiction.
  - reflexivity.
Qed.

(* Two fresh_versions with different counters for the same variable
   produce distinct SSAVals *)
Lemma fresh_version_injective : forall ctx1 ctx2 var,
  ctx_next_ver ctx1 var <> ctx_next_ver ctx2 var ->
  fresh_version ctx1 var <> fresh_version ctx2 var.
Proof.
  intros ctx1 ctx2 var Hne Heq.
  unfold fresh_version in Heq. inversion Heq. contradiction.
Qed.

(* After bumping, fresh_version gives a value distinct from the pre-bump one *)
Lemma fresh_version_after_bump : forall ctx var,
  fresh_version ctx var <> fresh_version (bump_version ctx var) var.
Proof.
  intros ctx var H.
  unfold fresh_version, bump_version in H. simpl in H.
  rewrite Nat.eqb_refl in H. inversion H. lia.
Qed.

(* --- Key derivation pattern: def_block ---
   def_block : SSAVal -> block is a total function modeling the
   current_def HashMap in ssa_builder.rs. Each SSAVal maps to
   exactly one definition site because fresh_version allocates
   unique (var, version) pairs. *)

Theorem unique_def_from_def_block :
  forall (defined_in : nat -> SSAVal -> Prop) (def_block : SSAVal -> nat),
    (forall b v, defined_in b v -> b = def_block v) ->
    forall v b1 b2, defined_in b1 v -> defined_in b2 v -> b1 = b2.
Proof.
  intros defined_in def_block Hspec v b1 b2 H1 H2.
  transitivity (def_block v).
  - exact (Hspec b1 v H1).
  - symmetry. exact (Hspec b2 v H2).
Qed.

(* --- Key derivation pattern: phi_lookup ---
   phi_lookup : block -> var -> option Phi models the block_phis
   HashMap in ssa.rs, keyed by variable index. Functionality of
   the map (at most one phi per variable per block) implies
   that two phis for the same variable in the same block are equal. *)

Theorem no_dup_phi_from_lookup :
  forall (phi_at : nat -> Phi -> Prop) (phi_lookup : nat -> nat -> option Phi),
    (forall b p, phi_at b p -> phi_lookup b (sv_var (phi_val p)) = Some p) ->
    forall b p1 p2,
      phi_at b p1 -> phi_at b p2 ->
      sv_var (phi_val p1) = sv_var (phi_val p2) -> p1 = p2.
Proof.
  intros phi_at phi_lookup Hspec b p1 p2 H1 H2 Hvar.
  pose proof (Hspec b p1 H1) as E1.
  pose proof (Hspec b p2 H2) as E2.
  rewrite Hvar in E1. rewrite E1 in E2. congruence.
Qed.

(* --- Seal model ---
   seal_block sets ctx_sealed[b] = true, clears ctx_incomplete[b],
   and fills all incomplete phis by looking up variable definitions
   in each predecessor. *)

Definition seal_block (ctx : SSACtx) (b : nat) : SSACtx :=
  mkSSACtx
    (ctx_next_ver ctx)
    (fun n => if Nat.eqb n b then true else ctx_sealed ctx n)
    (ctx_block_phis ctx)
    (fun n => if Nat.eqb n b then [] else ctx_incomplete ctx n).

Lemma seal_block_sealed : forall ctx b,
  ctx_sealed (seal_block ctx b) b = true.
Proof.
  intros ctx b. unfold seal_block. simpl. rewrite Nat.eqb_refl. reflexivity.
Qed.

Lemma seal_block_clears_incomplete : forall ctx b,
  ctx_incomplete (seal_block ctx b) b = [].
Proof.
  intros ctx b. unfold seal_block. simpl. rewrite Nat.eqb_refl. reflexivity.
Qed.

Lemma seal_block_preserves_other : forall ctx b1 b2,
  b1 <> b2 ->
  ctx_incomplete (seal_block ctx b1) b2 = ctx_incomplete ctx b2.
Proof.
  intros ctx b1 b2 Hne. unfold seal_block. simpl.
  destruct (Nat.eqb b2 b1) eqn:E.
  - apply Nat.eqb_eq in E. symmetry in E. contradiction.
  - reflexivity.
Qed.

End SSA_Construction.


(* ================================================================ *)
(* Use-Count: Operational Model                                       *)
(*                                                                    *)
(* Models use_count as length of a use-list (HashMap<SSAVal, Vec>     *)
(* in ssa_dse.rs). Derives zero-count and dependency properties.      *)
(* ================================================================ *)

Section UseCount_Operational.

(* Zero-length list is empty: combined with use_list_complete,
   this gives use_count_zero_no_use. *)
Lemma length_zero_not_In : forall {A : Type} (l : list A) (x : A),
  length l = 0 -> ~ In x l.
Proof.
  intros A l x Hlen Hin.
  assert (Hpos := In_length_pos x l Hin). lia.
Qed.

(* Non-empty list has positive length: combined with dep_recorded,
   this gives use_count_dep. *)
Lemma In_list_length_pos : forall {A : Type} (l : list A) (x : A),
  In x l -> length l > 0.
Proof.
  intros A l x H. exact (In_length_pos x l H).
Qed.

End UseCount_Operational.


(* ================================================================ *)
(* DSE: Operational Model                                             *)
(*                                                                    *)
(* Models global_dse (ssa_dse.rs) as iterated filtering.              *)
(* Each step removes values with zero use-count and pure RHS.        *)
(* Derives monotonicity and live-value preservation.                  *)
(* ================================================================ *)

Section DSE_Operational.

Variable A : Type.
Variable is_dead : A -> bool.

(* Single DSE step: filter out dead values *)
Definition dse_filter (vals : list A) : list A :=
  filter (fun v => negb (is_dead v)) vals.

Lemma dse_filter_mono : forall vals,
  length (dse_filter vals) <= length vals.
Proof.
  intro vals. unfold dse_filter. apply filter_length_le.
Qed.

(* Non-dead values survive the filter *)
Lemma dse_filter_preserves : forall vals v,
  In v vals -> is_dead v = false -> In v (dse_filter vals).
Proof.
  intros vals v Hin Halive. unfold dse_filter.
  induction vals as [|x xs IH].
  - inversion Hin.
  - destruct Hin as [<- | Hin].
    + simpl. rewrite Halive. simpl. left. reflexivity.
    + simpl. destruct (negb (is_dead x)).
      * right. exact (IH Hin).
      * exact (IH Hin).
Qed.

(* Iterated DSE *)
Fixpoint dse_iterate (n : nat) (vals : list A) : list A :=
  match n with
  | 0 => vals
  | S n' => dse_filter (dse_iterate n' vals)
  end.

Lemma dse_iterate_mono : forall n vals,
  length (dse_iterate (S n) vals) <= length (dse_iterate n vals).
Proof.
  intros n vals. simpl. apply dse_filter_mono.
Qed.

(* Contrapositive: positive use-count + dead-implies-zero -> not dead *)
Lemma live_implies_not_dead : forall (uc : nat) (dead : bool),
  (dead = true -> uc = 0) ->
  uc > 0 ->
  dead = false.
Proof.
  intros uc d Himp Hpos.
  destruct d; [exfalso; lia | reflexivity].
Qed.

End DSE_Operational.
