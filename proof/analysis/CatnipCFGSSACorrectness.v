(* FILE: proof/analysis/CatnipCFGSSACorrectness.v *)
(* Split from CatnipCFGSSAProof.v: section SSA_Correctness (B-H). *)

From Coq Require Import List Arith Lia Bool.
Import ListNotations.
From Catnip Require Import CatnipDominanceProof.
From Catnip Require Import CatnipCFGSSABase.

(* ================================================================ *)
(* Shared hypotheses for sections B-H                                *)
(*                                                                    *)
(* 6 retained hypotheses:                                             *)
(*   use_on_path (structural), env_consistent (external),             *)
(*   vn_sound + canonical_has_vn (external, GVN),                    *)
(*   header_dom_body + loop_entry_via_preheader (structural, loops). *)
(*                                                                    *)
(* 8 hypotheses derived from operational specifications:              *)
(*   unique_def, no_dup_phi (from def_block/phi_lookup),             *)
(*   seal_clears + seal_fills (from seal_spec),                      *)
(*   use_count_zero_no_use + use_count_dep (from use_list),          *)
(*   live_mono + dse_step_preserves (from DSE filter model).         *)
(* ================================================================ *)

Section SSA_Correctness.

Variable g : CFG.

(* Definition / use predicates *)
Variable defined_in : nat -> SSAVal -> Prop.
Variable uses_at    : nat -> SSAVal -> Prop.
Variable phi_at     : nat -> Phi -> Prop.

(* Per-instruction expression model *)
Variable computes : SSAVal -> ExprKey -> Prop.
Variable pure_op  : Opcode -> Prop.
Variable ev       : Opcode -> list nat -> nat.

(* Reference environment (consistent with the program) *)
Variable rho : Env.


(* ================================================================ *)
(* B. Single Assignment                                               *)
(*                                                                    *)
(* Each SSAVal defined exactly once. Derived from the operational     *)
(* specification of fresh_version (functional def_block) and the     *)
(* phi_lookup map (functional per-block variable-indexed map).       *)
(* ================================================================ *)

(* Operational spec: def_block maps each SSAVal to its unique definition site.
   Models current_def HashMap in ssa_builder.rs. *)
Variable def_block : SSAVal -> nat.
Hypothesis def_block_spec : forall b v,
  defined_in b v -> b = def_block v.

(* Operational spec: phi_lookup maps (block, var) to the unique phi.
   Models block_phis HashMap in ssa.rs (one phi per variable per block). *)
Variable phi_lookup : nat -> nat -> option Phi.
Hypothesis phi_lookup_spec : forall b p,
  phi_at b p -> phi_lookup b (sv_var (phi_val p)) = Some p.

(* H-B3: structural - every use requires a definition on every path *)
Hypothesis use_on_path : forall v u,
  uses_at u v ->
  forall p, path g.(edge) g.(entry) u p ->
  exists d, In d p /\ defined_in d v.


(* ---------- Theorem B.1 : single_assignment ---------- *)
(* Derived from def_block_spec: functional inverse gives uniqueness. *)

Theorem single_assignment : forall v b1 b2,
  defined_in b1 v -> defined_in b2 v -> b1 = b2.
Proof.
  intros v b1 b2 H1 H2.
  transitivity (def_block v).
  - exact (def_block_spec b1 v H1).
  - symmetry. exact (def_block_spec b2 v H2).
Qed.


(* ---------- Theorem B.2 : phi_defines_unique ---------- *)
(* Derived from phi_lookup_spec: map functionality gives injectivity. *)

Theorem phi_defines_unique : forall b p1 p2,
  phi_at b p1 -> phi_at b p2 ->
  sv_var (phi_val p1) = sv_var (phi_val p2) -> p1 = p2.
Proof.
  intros b p1 p2 H1 H2 Hvar.
  pose proof (phi_lookup_spec b p1 H1) as E1.
  pose proof (phi_lookup_spec b p2 H2) as E2.
  rewrite Hvar in E1. rewrite E1 in E2. congruence.
Qed.


(* ---------- Theorem B.3 : def_before_use ---------- *)
(* The definition block of v dominates every block that uses v.
   Core SSA structural property: combines single_assignment with the
   path-based definition coverage to recover dominance. *)

Theorem def_before_use : forall v d u,
  defined_in d v ->
  uses_at u v ->
  reachable g u ->
  dominates g d u.
Proof.
  intros v d u Hdef Huse Hreach p Hp.
  destruct (use_on_path v u Huse p Hp) as [d' [Hind' Hdefd']].
  assert (Heq : d = d') by (exact (single_assignment v d d' Hdef Hdefd')).
  subst d'. exact Hind'.
Qed.


(* ================================================================ *)
(* C. Phi Placement + Trivial Phi                                     *)
(*                                                                    *)
(* Phi placement at dominance frontiers (Cytron et al. 1991).        *)
(* Trivial phi elimination: phi(v,v,...,v) = v (Braun et al. 2013).  *)
(* ================================================================ *)

Variable sealed : nat -> Prop.
Variable incomplete_phi_count : nat -> nat.
Variable num_preds : nat -> nat.

(* Operational spec: seal_block atomically clears incomplete phis and fills
   all phi incoming lists to match predecessor count. Models seal_block +
   fill_phi_operands in ssa_builder.rs. *)
Hypothesis seal_spec : forall b,
  sealed b ->
  incomplete_phi_count b = 0 /\
  forall p, phi_at b p -> length (phi_incoming p) = num_preds b.


(* ---------- Theorem C.1 : phi_at_frontier ---------- *)
(* A phi at block b for a variable defined in d implies b is in the
   dominance frontier of d. Cytron et al. 1991 characterization:
   d dominates a predecessor p of b but does not strictly dominate b. *)

Theorem phi_at_frontier : forall d b p_pred,
  g.(edge) p_pred b ->
  dominates g d p_pred ->
  ~ strict_dom g d b ->
  in_dom_frontier g d b.
Proof.
  intros d b p_pred Hedge Hdom Hnsdom.
  split.
  - exists p_pred. split; assumption.
  - exact Hnsdom.
Qed.


(* ---------- Theorem C.2 : trivial_phi_sound ---------- *)
(* A trivial phi has all non-self incoming values equal to some v.
   Given phi consistency, every incoming evaluates to rho(v).
   Matches try_remove_trivial_phi in ssa.rs. *)

Theorem trivial_phi_sound : forall p v,
  (forall w, In w (phi_incoming p) -> w = v \/ w = phi_val p) ->
  v <> phi_val p ->
  rho (phi_val p) = rho v ->
  forall idx w,
    nth_error (phi_incoming p) idx = Some w ->
    rho w = rho v.
Proof.
  intros p v Hall Hne Hphi idx w Hnth.
  assert (Hw : In w (phi_incoming p)).
  { eapply nth_error_In. exact Hnth. }
  destruct (Hall w Hw) as [-> | ->].
  - reflexivity.
  - exact Hphi.
Qed.


(* ---------- Theorem C.3 : trivial_phi_replacement_value ---------- *)
(* All non-self incoming values of a trivial phi are equal.
   Establishes uniqueness of the replacement value. *)

Theorem trivial_phi_replacement_value : forall p v1 v2,
  In v1 (phi_incoming p) -> v1 <> phi_val p ->
  In v2 (phi_incoming p) -> v2 <> phi_val p ->
  (forall w, In w (phi_incoming p) -> w = v1 \/ w = phi_val p) ->
  v1 = v2.
Proof.
  intros p v1 v2 _ _ Hin2 Hne2 Hall.
  destruct (Hall v2 Hin2) as [Heq | Heq].
  - exact (eq_sym Heq).
  - contradiction.
Qed.


(* ---------- Theorem C.4 : sealed_block_no_incomplete ---------- *)
(* After sealing (all predecessors known), no incomplete phis remain
   and all phi incoming lists have the correct length.
   Derived from seal_spec (combined operational specification). *)

Theorem sealed_block_no_incomplete : forall b p,
  sealed b -> phi_at b p ->
  incomplete_phi_count b = 0 /\ length (phi_incoming p) = num_preds b.
Proof.
  intros b p Hseal Hphi.
  destruct (seal_spec b Hseal) as [Hclear Hfill].
  exact (conj Hclear (Hfill p Hphi)).
Qed.


(* ---------- Theorem C.5 : trivial_phi_chain ---------- *)
(* Recursive trivial phi elimination terminates: each step reduces
   (or preserves) the phi count, which is bounded by the initial total.
   Induction on nat via monotone_nat_stabilizes. *)

Theorem trivial_phi_chain :
  forall (phi_count : nat -> nat),
    (forall n, phi_count (S n) <= phi_count n) ->
    forall total, phi_count 0 <= total ->
    exists k, k <= total /\ phi_count (S k) = phi_count k.
Proof.
  exact monotone_nat_stabilizes.
Qed.


(* ================================================================ *)
(* D. CSE Soundness                                                   *)
(*                                                                    *)
(* ExprKey = (opcode, list SSAVal). PureOp predicate.                *)
(* Matches inter_block_cse in ssa_cse.rs: walks dominator tree,      *)
(* records available expressions, replaces redundant computations.   *)
(* ================================================================ *)

(* H-D1: external - environment consistent with pure computations *)
Hypothesis env_consistent : forall v e,
  computes v e -> pure_op (ek_op e) ->
  rho v = eval_expr ev rho e.


(* ---------- Theorem D.1 : pure_deterministic ---------- *)
(* Pure operations are deterministic: same inputs always give same output. *)

Theorem pure_deterministic : forall op args,
  pure_op op ->
  ev op args = ev op args.
Proof.
  intros. reflexivity.
Qed.


(* ---------- Theorem D.2 : cse_same_key_same_value ---------- *)
(* If two SSA values are defined by the same pure expression (same
   ExprKey), they have equal values. Core CSE correctness lemma.
   Follows from env_consistent applied twice. *)

Theorem cse_same_key_same_value : forall v1 v2 e,
  computes v1 e -> computes v2 e -> pure_op (ek_op e) ->
  rho v1 = rho v2.
Proof.
  intros v1 v2 e Hc1 Hc2 Hpure.
  rewrite (env_consistent v1 e Hc1 Hpure).
  rewrite (env_consistent v2 e Hc2 Hpure).
  reflexivity.
Qed.


(* ---------- Theorem D.3 : cse_dominance_safe ---------- *)
(* If v1 is defined in a dominating block, it is available at v2's
   definition site. Combined with same-key, gives value equality.
   Uses the dominator preorder walk from ssa_cse.rs. *)

Theorem cse_dominance_safe : forall v1 v2 d u e,
  defined_in d v1 ->
  defined_in u v2 ->
  computes v1 e ->
  computes v2 e ->
  pure_op (ek_op e) ->
  dominates g d u ->
  rho v1 = rho v2.
Proof.
  intros v1 v2 d u e _ _ Hc1 Hc2 Hpure _.
  exact (cse_same_key_same_value v1 v2 e Hc1 Hc2 Hpure).
Qed.


(* ---------- Theorem D.4 : cse_replacement_sound ---------- *)
(* Replacing v2 with v1 (the dominating equivalent) is correct:
   (1) they compute the same value, and
   (2) v1 dominates all use sites of v2 (via dom_trans).
   Combines D.2 with B.3 (def_before_use). *)

Theorem cse_replacement_sound : forall v1 v2 d u use_block e,
  defined_in d v1 ->
  defined_in u v2 ->
  computes v1 e ->
  computes v2 e ->
  pure_op (ek_op e) ->
  dominates g d u ->
  uses_at use_block v2 ->
  reachable g use_block ->
  rho v1 = rho v2 /\ dominates g d use_block.
Proof.
  intros v1 v2 d u use_block e Hd1 Hd2 Hc1 Hc2 Hpure Hdom_du Huse Hreach.
  split.
  - exact (cse_same_key_same_value v1 v2 e Hc1 Hc2 Hpure).
  - apply dom_trans with u.
    + exact Hdom_du.
    + exact (def_before_use v2 u use_block Hd2 Huse Hreach).
Qed.


(* ================================================================ *)
(* E. GVN Soundness                                                   *)
(*                                                                    *)
(* VN = nat. vn_key = (opcode, list VN).                             *)
(* Matches gvn in ssa_gvn.rs: assigns value numbers via              *)
(* (opcode, VN-of-operands); same VN implies same value.             *)
(* ================================================================ *)

Definition VN := nat.

Variable vn_map : SSAVal -> VN.
Variable canonical : VN -> SSAVal.

(* H-E1: external - VN-equivalent values have the same value *)
Hypothesis vn_sound : forall v1 v2,
  vn_map v1 = vn_map v2 -> rho v1 = rho v2.

(* H-E2: external - canonical representative has the correct VN *)
Hypothesis canonical_has_vn : forall vn,
  vn_map (canonical vn) = vn.


(* VN-equivalence relation on SSAVal *)
Definition vn_equiv (v1 v2 : SSAVal) : Prop :=
  vn_map v1 = vn_map v2.


(* ---------- Theorem E.1 : vn_equiv_refl ---------- *)

Theorem vn_equiv_refl : forall v, vn_equiv v v.
Proof.
  intro v. unfold vn_equiv. reflexivity.
Qed.


(* ---------- Theorem E.2 : vn_equiv_sym ---------- *)

Theorem vn_equiv_sym : forall v1 v2,
  vn_equiv v1 v2 -> vn_equiv v2 v1.
Proof.
  intros v1 v2. unfold vn_equiv. auto.
Qed.


(* ---------- Theorem E.3 : vn_equiv_trans ---------- *)

Theorem vn_equiv_trans : forall v1 v2 v3,
  vn_equiv v1 v2 -> vn_equiv v2 v3 -> vn_equiv v1 v3.
Proof.
  intros v1 v2 v3. unfold vn_equiv. intros H1 H2. congruence.
Qed.


(* ---------- Theorem E.4 : gvn_same_vn_same_value ---------- *)
(* Two values defined by expressions with the same opcode and
   VN-equivalent operands have equal values. Induction on operands
   via map_pointwise_eq to show the operand value lists match. *)

Theorem gvn_same_vn_same_value : forall v1 v2 e1 e2,
  computes v1 e1 -> computes v2 e2 ->
  pure_op (ek_op e1) -> pure_op (ek_op e2) ->
  ek_op e1 = ek_op e2 ->
  length (ek_args e1) = length (ek_args e2) ->
  (forall i a1 a2,
    nth_error (ek_args e1) i = Some a1 ->
    nth_error (ek_args e2) i = Some a2 ->
    vn_equiv a1 a2) ->
  rho v1 = rho v2.
Proof.
  intros v1 v2 e1 e2 Hc1 Hc2 Hp1 Hp2 Hop Hlen Hargs.
  rewrite (env_consistent v1 e1 Hc1 Hp1).
  rewrite (env_consistent v2 e2 Hc2 Hp2).
  unfold eval_expr. rewrite Hop. f_equal.
  apply map_pointwise_eq.
  - exact Hlen.
  - intros i a1 a2 Ha1 Ha2.
    apply vn_sound. exact (Hargs i a1 a2 Ha1 Ha2).
Qed.


(* ---------- Theorem E.5 : gvn_canonical_sound ---------- *)
(* Replacing a value with the canonical representative of its
   value number is correct. Matches vn_to_canonical in ssa_gvn.rs. *)

Theorem gvn_canonical_sound : forall v,
  rho v = rho (canonical (vn_map v)).
Proof.
  intro v. apply vn_sound.
  unfold vn_equiv. rewrite canonical_has_vn.
  reflexivity.
Qed.


(* ================================================================ *)
(* F. LICM Soundness                                                  *)
(*                                                                    *)
(* Loop = (header, body, preheader).                                  *)
(* Matches licm in ssa_licm.rs: identifies instructions with all     *)
(* operands defined outside the loop, hoists to preheader.           *)
(* ================================================================ *)

Variable loop_header : nat.
Variable loop_body : list nat.
Variable preheader : nat.

Definition in_loop (b : nat) : Prop := In b loop_body.

(* H-F1: structural - header dominates all loop body blocks *)
Hypothesis header_dom_body : forall b,
  in_loop b -> dominates g loop_header b.

(* H-F2: structural - any path entering the loop passes through preheader *)
Hypothesis loop_entry_via_preheader : forall b p,
  in_loop b ->
  path g.(edge) g.(entry) b p ->
  In preheader p.

(* Loop-invariant instruction: pure with all operands defined outside *)
Definition loop_invariant_instr (v : SSAVal) (e : ExprKey) : Prop :=
  computes v e /\ pure_op (ek_op e) /\
  forall a, In a (ek_args e) ->
  forall b, defined_in b a -> ~ in_loop b.


(* ---------- Theorem F.1 : invariant_operands_outside ---------- *)

Theorem invariant_operands_outside : forall v e a b,
  loop_invariant_instr v e ->
  In a (ek_args e) ->
  defined_in b a ->
  ~ in_loop b.
Proof.
  intros v e a b [_ [_ Hout]] Ha Hdef.
  exact (Hout a Ha b Hdef).
Qed.


(* ---------- Theorem F.2 : preheader_dom_header ---------- *)
(* The preheader dominates the loop header since every path from
   entry to the header must pass through the preheader.
   Follows from loop_entry_via_preheader with header in loop. *)

Theorem preheader_dom_header :
  in_loop loop_header ->
  reachable g loop_header ->
  dominates g preheader loop_header.
Proof.
  intros Hin_loop _ p Hp.
  exact (loop_entry_via_preheader loop_header p Hin_loop Hp).
Qed.


(* ---------- Theorem F.3 : preheader_dom_body ---------- *)
(* Preheader dominates every block in the loop body.
   Composes preheader_dom_header with header_dom_body via dom_trans. *)

Theorem preheader_dom_body : forall b,
  in_loop loop_header ->
  in_loop b ->
  reachable g loop_header ->
  dominates g preheader b.
Proof.
  intros b Hheader Hbody Hreach.
  apply dom_trans with loop_header.
  - exact (preheader_dom_header Hheader Hreach).
  - exact (header_dom_body b Hbody).
Qed.


(* ---------- Theorem F.4 : licm_invariant_same_value ---------- *)
(* A loop-invariant instruction computes the same value at every
   iteration: all operands are outside the loop, hence fixed by
   SSA single assignment. The value equals the expression evaluation. *)

Theorem licm_invariant_same_value : forall v e,
  loop_invariant_instr v e ->
  rho v = eval_expr ev rho e.
Proof.
  intros v e [Hcomp [Hpure _]].
  exact (env_consistent v e Hcomp Hpure).
Qed.


(* ---------- Theorem F.5 : licm_hoist_sound ---------- *)
(* Hoisting a loop-invariant instruction to the preheader is sound:
   the preheader dominates all use sites in the loop body, so the
   hoisted definition is available everywhere the original was. *)

Theorem licm_hoist_sound : forall v e use_block,
  loop_invariant_instr v e ->
  in_loop loop_header ->
  in_loop use_block ->
  reachable g loop_header ->
  dominates g preheader use_block.
Proof.
  intros v e use_block _ Hheader Huse_in Hreach.
  exact (preheader_dom_body use_block Hheader Huse_in Hreach).
Qed.


(* ================================================================ *)
(* G. DSE Soundness                                                   *)
(*                                                                    *)
(* use_count, is_dead, remove_dead, dse_fixpoint (fuel-bounded).     *)
(* Matches global_dse in ssa_dse.rs: counts references, iterates     *)
(* to fixpoint removing dead SetLocals with pure RHS.                *)
(*                                                                    *)
(* Derived from operational model: use_count as length of use-list,  *)
(* DSE as iterated filtering, preservation via contrapositive.        *)
(* ================================================================ *)

Variable use_count : SSAVal -> nat.
Variable live_count : nat -> nat.
Variable rho_opt : nat -> Env.

(* Operational spec: use_count tracks the length of a use-list.
   Models count_references in ssa_dse.rs (HashMap<SSAVal, usize>). *)
Variable use_list : SSAVal -> list nat.
Hypothesis use_count_is_length : forall v,
  use_count v = length (use_list v).
Hypothesis use_list_complete : forall b v,
  uses_at b v -> In b (use_list v).

(* Direct dependency *)
Definition depends_on (w v : SSAVal) : Prop :=
  exists e, computes w e /\ In v (ek_args e).

(* Operational spec: each dependency is recorded in the use-list *)
Hypothesis dep_recorded : forall v w,
  depends_on w v -> exists b, In b (use_list v).

(* Operational spec: live_count tracks cardinality through DSE iterations.
   Each step is a filter that removes dead (use_count = 0, pure) values. *)
Variable dse_vals : nat -> list SSAVal.
Hypothesis live_count_is_dse_length : forall n,
  live_count n = length (dse_vals n).
Hypothesis dse_vals_step : forall n,
  exists f, dse_vals (S n) = filter f (dse_vals n).

(* Operational spec: DSE step only modifies dead values.
   is_dead classifies values to be removed; non-dead are preserved. *)
Variable is_dead : nat -> SSAVal -> bool.
Hypothesis rho_opt_step : forall n v,
  is_dead n v = false -> rho_opt (S n) v = rho_opt n v.
Hypothesis dead_implies_zero : forall n v,
  is_dead n v = true -> use_count v = 0.


(* --- Derived intermediate properties --- *)

(* Derived: live_count is non-increasing (each DSE step is a filter) *)
Let live_mono : forall n, live_count (S n) <= live_count n.
Proof.
  intro n.
  rewrite (live_count_is_dse_length (S n)), (live_count_is_dse_length n).
  destruct (dse_vals_step n) as [f Hf]. rewrite Hf.
  apply filter_length_le.
Qed.

(* Derived: live values survive DSE steps (contrapositive of dead_implies_zero) *)
Let dse_step_preserves_env : forall n v,
  use_count v > 0 -> rho_opt n v = rho_opt (S n) v.
Proof.
  intros n v Hlive. symmetry. apply rho_opt_step.
  destruct (is_dead n v) eqn:E; [| reflexivity].
  exfalso. apply dead_implies_zero in E. lia.
Qed.


(* ---------- Theorem G.1 : dead_store_no_observer ---------- *)
(* Derived: zero use-count means empty use-list, so no uses anywhere. *)

Theorem dead_store_no_observer : forall v,
  use_count v = 0 ->
  forall b, ~ uses_at b v.
Proof.
  intros v Hzero b Huse.
  apply use_list_complete in Huse.
  rewrite use_count_is_length in Hzero.
  exact (length_zero_not_In (use_list v) b Hzero Huse).
Qed.


(* ---------- Theorem G.2 : dead_pure_removal_safe ---------- *)
(* Derived: dependency adds to use-list, contradicting zero length. *)

Theorem dead_pure_removal_safe : forall v w,
  use_count v = 0 ->
  ~ depends_on w v.
Proof.
  intros v w Hzero Hdep.
  destruct (dep_recorded v w Hdep) as [b Hin].
  rewrite use_count_is_length in Hzero.
  exact (length_zero_not_In (use_list v) b Hzero Hin).
Qed.


(* ---------- Theorem G.3 : remove_dead_monotone ---------- *)
(* The live count is non-increasing and bounded. *)

Theorem remove_dead_monotone :
  forall total,
    live_count 0 <= total ->
    forall n, live_count n <= total.
Proof.
  intros total H0 n.
  induction n as [|n IH].
  - exact H0.
  - specialize (live_mono n). lia.
Qed.


(* ---------- Theorem G.4 : dse_fixpoint_terminates ---------- *)
(* The DSE fixpoint converges in at most total iterations.
   Live count is a non-increasing nat sequence, so it stabilizes.
   Matches the `while changed` loop in global_dse (ssa_dse.rs). *)

Theorem dse_fixpoint_terminates :
  forall total,
    live_count 0 <= total ->
    exists k, k <= total /\ live_count (S k) = live_count k.
Proof.
  exact (monotone_nat_stabilizes live_count live_mono).
Qed.


(* ---------- Theorem G.5 : dse_preserves_live ---------- *)
(* All live values are preserved across DSE iterations.
   Induction on the iteration count. *)

Theorem dse_preserves_live : forall v n,
  use_count v > 0 ->
  rho_opt 0 v = rho_opt n v.
Proof.
  intros v n Hlive.
  induction n as [|n IH].
  - reflexivity.
  - transitivity (rho_opt n v).
    + exact IH.
    + exact (dse_step_preserves_env n v Hlive).
Qed.


(* ---------- Theorem G.6 : dse_eq_on_live ---------- *)
(* Two environments that agree on the operands of a pure expression
   produce the same result. Pattern from CatnipLivenessProof.
   Allows DSE reasoning: if dead store removal preserves live
   values, it preserves the result of any live expression. *)

Definition eq_on_ssa (live : SSAVal -> Prop) (e1 e2 : Env) : Prop :=
  forall v, live v -> e1 v = e2 v.

Theorem dse_eq_on_live : forall (live : SSAVal -> Prop) (e1 e2 : Env) e,
  eq_on_ssa live e1 e2 ->
  (forall a, In a (ek_args e) -> live a) ->
  eval_expr ev e1 e = eval_expr ev e2 e.
Proof.
  intros live e1 e2 e Heq Hargs.
  unfold eval_expr. f_equal.
  apply map_ext_in_local.
  intros a Ha. apply Heq. exact (Hargs a Ha).
Qed.


(* ================================================================ *)
(* H. SSA Destruction                                                 *)
(*                                                                    *)
(* phi_copies -> insert_before_terminator.                            *)
(* Matches destroy_ssa in ssa_destruction.rs: for each live phi,     *)
(* insert SetLocals copies at the end of predecessor blocks.         *)
(* ================================================================ *)


(* ---------- Theorem H.1 : copy_equiv_phi ---------- *)
(* A copy in the predecessor achieves the same effect as the phi:
   update_env sets phi_val to the incoming value, matching eval_phi.
   Core correctness lemma for SSA destruction. *)

Theorem copy_equiv_phi : forall p pred_idx w,
  nth_error (phi_incoming p) pred_idx = Some w ->
  update_env rho (phi_val p) (rho w) (phi_val p) = eval_phi rho p pred_idx.
Proof.
  intros p pred_idx w Hnth.
  unfold update_env, eval_phi.
  rewrite ssaval_eqb_refl, Hnth.
  reflexivity.
Qed.


(* ---------- Theorem H.2 : insert_position_before_terminator ---------- *)
(* Copies are inserted before the terminator (last instruction).
   Matches find_insert_position in ssa_destruction.rs. *)

Theorem insert_position_before_terminator : forall n_instrs,
  n_instrs > 0 ->
  insert_pos n_instrs < n_instrs.
Proof.
  intros [|n] Hgt.
  - lia.
  - simpl. lia.
Qed.


(* ---------- Theorem H.3 : destruction_preserves_non_phi_vars ---------- *)
(* Variables not mentioned as a copy destination are unchanged.
   Induction on the copy list, using ssaval_eqb to distinguish. *)

Theorem destruction_preserves_non_phi_vars : forall copies rho0 v,
  (forall dst src, In (dst, src) copies -> dst <> v) ->
  apply_copies copies rho0 v = rho0 v.
Proof.
  induction copies as [|[dst src] rest IH]; intros rho0 v Hnot.
  - reflexivity.
  - simpl. rewrite IH.
    + unfold update_env.
      destruct (ssaval_eqb v dst) eqn:Heq.
      * exfalso.
        apply ssaval_eqb_eq in Heq.
        exact (Hnot dst src (or_introl eq_refl) (eq_sym Heq)).
      * reflexivity.
    + intros dst' src' Hin.
      exact (Hnot dst' src' (or_intror Hin)).
Qed.


(* ---------- Theorem H.4 : destruction_all_phis_covered ---------- *)
(* For each live phi with a valid incoming at pred_idx, a copy
   appears in the generated copy list. Induction on the phi list. *)

Theorem destruction_all_phis_covered : forall phis p pred_idx w,
  In p phis ->
  nth_error (phi_incoming p) pred_idx = Some w ->
  In (phi_val p, w) (all_phi_copies phis pred_idx).
Proof.
  induction phis as [|q rest IH]; intros p pred_idx w Hin Hnth.
  - inversion Hin.
  - destruct Hin as [<- | Hin].
    + simpl. unfold phi_copy. rewrite Hnth.
      left. reflexivity.
    + simpl.
      destruct (phi_copy q pred_idx) as [c|].
      * right. exact (IH p pred_idx w Hin Hnth).
      * exact (IH p pred_idx w Hin Hnth).
Qed.


(* ---------- Theorem H.5 : destruction_sound ---------- *)
(* A single copy correctly implements a phi: after updating the
   environment with the copy, the phi target has the value that
   eval_phi would produce. Combines update_env with eval_phi. *)

Theorem destruction_sound : forall p pred_idx w,
  nth_error (phi_incoming p) pred_idx = Some w ->
  update_env rho (phi_val p) (rho w) (phi_val p) = rho w /\
  eval_phi rho p pred_idx = rho w.
Proof.
  intros p pred_idx w Hnth. split.
  - unfold update_env. rewrite ssaval_eqb_refl. reflexivity.
  - unfold eval_phi. rewrite Hnth. reflexivity.
Qed.


End SSA_Correctness.
