(* FILE: proof/optim/CatnipBluntCodeProof.v *)
(* Blunt Code - boolean simplifications.
 *
 * Source: catnip_core/src/semantic/passes/blunt_code.rs
 *
 * The live pass keeps only:
 *   - not (a == b) -> a != b and not (a != b) -> a == b (Eq/Ne only:
 *     order inversions are unsound under IEEE 754 NaN)
 *   - And/Or folding when BOTH operands are boolean literals
 *   - complement: x and (not x) -> False, x or (not x) -> True
 *   - if/elif constant-condition pruning
 *
 * Removed from the code (review 2026-06-10), pinned by *_untouched
 * guards below: double negation (not not 5 is True, not 5), boolean
 * equality (5 == True is False, not 5), order comparison inversion
 * (not (a < b) is not a >= b under NaN), idempotence (x && x changes
 * the return type when x is not a bool).
 *
 * Model scope: Expr has no side effects, so the complement rules are
 * proved on pure expressions; the Rust pass drops the evaluation of x
 * in `f() and not f()` (tracked as a review suggestion).
 *
 * Proves:
 *   - invert_cmp: involution, negation correctness (Eq/Ne)
 *   - invert_cmp_order_none: order comparisons have no inversion
 *   - the live rewrites + *_untouched guards
 *   - expr_eqb reflection (expr_eqb_eq)
 *   - simplify_blunt preserves eval_bool (simplify_blunt_bool_sound)
 *
 * Depends on: CatnipIR.v, CatnipExprModel.v
 *
 * 0 Admitted.
 *)

From Coq Require Import List ZArith Bool Lia QArith.
From Catnip Require Import CatnipIR.
From Catnip Require Import CatnipExprModel.
Import ListNotations.


(* ================================================================ *)
(* C. Blunt Code                                                     *)
(* ================================================================ *)

(* Eq/Ne only: NaN breaks the order inversions (not (a < b) is not
   a >= b when either side is NaN). *)
Definition invert_cmp (oc : IROpCode) : option IROpCode :=
  match oc with
  | Eq => Some Ne | Ne => Some Eq
  | _ => None
  end.

Theorem invert_cmp_involution : forall oc oc',
  invert_cmp oc = Some oc' -> invert_cmp oc' = Some oc.
Proof.
  intros oc oc' H;
  destruct oc; simpl in H; try discriminate;
  inversion H; subst; reflexivity.
Qed.

Theorem invert_cmp_self_inverse : forall oc oc' oc'',
  invert_cmp oc = Some oc' -> invert_cmp oc' = Some oc'' -> oc'' = oc.
Proof.
  intros oc oc' oc'' H1 H2;
  destruct oc; simpl in H1; try discriminate;
  inversion H1; subst; simpl in H2; inversion H2; reflexivity.
Qed.

(* Guard: order comparisons are never inverted (NaN) *)
Theorem invert_cmp_order_none :
  invert_cmp Lt = None /\ invert_cmp Le = None /\
  invert_cmp Gt = None /\ invert_cmp Ge = None.
Proof. repeat split. Qed.

(* Literal boolean extraction (mirror of matches!((l, r), (Bool, Bool))) *)
Definition as_bconst (e : Expr) : option bool :=
  match e with BConst b => Some b | _ => None end.

(* And: two boolean literals fold; complement folds regardless of x.
   `orig` is the unmodified expression (returned when no rule fires). *)
Definition blunt_and (e1 e2 orig : Expr) : Expr :=
  match as_bconst e1, as_bconst e2 with
  | Some a, Some b => BConst (andb a b)
  | _, _ =>
      match is_not e1 with
      | Some e1' =>
          if expr_eqb e1' e2 then BConst false
          else match is_not e2 with
               | Some e2' => if expr_eqb e1 e2' then BConst false else orig
               | None => orig
               end
      | None =>
          match is_not e2 with
          | Some e2' => if expr_eqb e1 e2' then BConst false else orig
          | None => orig
          end
      end
  end.

Definition blunt_or (e1 e2 orig : Expr) : Expr :=
  match as_bconst e1, as_bconst e2 with
  | Some a, Some b => BConst (orb a b)
  | _, _ =>
      match is_not e1 with
      | Some e1' =>
          if expr_eqb e1' e2 then BConst true
          else match is_not e2 with
               | Some e2' => if expr_eqb e1 e2' then BConst true else orig
               | None => orig
               end
      | None =>
          match is_not e2 with
          | Some e2' => if expr_eqb e1 e2' then BConst true else orig
          | None => orig
          end
      end
  end.

Definition simplify_blunt (e : Expr) : Expr :=
  match e with
  | UnOp Not (BinOp op a b) =>
      match invert_cmp op with
      | Some op' => BinOp op' a b
      | None => e
      end
  | BinOp And e1 e2 => blunt_and e1 e2 e
  | BinOp Or e1 e2 => blunt_or e1 e2 e
  | IfExpr (BConst true) t _ => t
  | IfExpr (BConst false) _ f => f
  | _ => e
  end.

(* --- Live rewrites --- *)

Theorem blunt_not_eq : forall a b,
  simplify_blunt (UnOp Not (BinOp Eq a b)) = BinOp Ne a b.
Proof. reflexivity. Qed.

Theorem blunt_not_ne : forall a b,
  simplify_blunt (UnOp Not (BinOp Ne a b)) = BinOp Eq a b.
Proof. reflexivity. Qed.

Theorem blunt_and_bools : forall a b,
  simplify_blunt (BinOp And (BConst a) (BConst b)) = BConst (andb a b).
Proof. reflexivity. Qed.

Theorem blunt_or_bools : forall a b,
  simplify_blunt (BinOp Or (BConst a) (BConst b)) = BConst (orb a b).
Proof. reflexivity. Qed.

Theorem blunt_if_true : forall t f,
  simplify_blunt (IfExpr (BConst true) t f) = t.
Proof. reflexivity. Qed.

Theorem blunt_if_false : forall t f,
  simplify_blunt (IfExpr (BConst false) t f) = f.
Proof. reflexivity. Qed.

(* --- Guards: removed rules never fire --- *)

Theorem blunt_double_neg_untouched : forall x,
  (forall op a b, x <> BinOp op a b) ->
  simplify_blunt (UnOp Not (UnOp Not x)) = UnOp Not (UnOp Not x).
Proof. reflexivity. Qed.

Theorem blunt_not_lt_untouched : forall a b,
  simplify_blunt (UnOp Not (BinOp Lt a b)) = UnOp Not (BinOp Lt a b).
Proof. reflexivity. Qed.

Theorem blunt_not_le_untouched : forall a b,
  simplify_blunt (UnOp Not (BinOp Le a b)) = UnOp Not (BinOp Le a b).
Proof. reflexivity. Qed.

Theorem blunt_not_gt_untouched : forall a b,
  simplify_blunt (UnOp Not (BinOp Gt a b)) = UnOp Not (BinOp Gt a b).
Proof. reflexivity. Qed.

Theorem blunt_not_ge_untouched : forall a b,
  simplify_blunt (UnOp Not (BinOp Ge a b)) = UnOp Not (BinOp Ge a b).
Proof. reflexivity. Qed.

Theorem blunt_eq_true_untouched : forall x,
  simplify_blunt (BinOp Eq x (BConst true)) = BinOp Eq x (BConst true).
Proof. reflexivity. Qed.

Theorem blunt_eq_false_untouched : forall x,
  simplify_blunt (BinOp Eq x (BConst false)) = BinOp Eq x (BConst false).
Proof. reflexivity. Qed.

Theorem blunt_and_idempotent_untouched : forall n,
  simplify_blunt (BinOp And (Var n) (Var n)) = BinOp And (Var n) (Var n).
Proof. reflexivity. Qed.

Theorem blunt_or_idempotent_untouched : forall n,
  simplify_blunt (BinOp Or (Var n) (Var n)) = BinOp Or (Var n) (Var n).
Proof. reflexivity. Qed.

(* --- Complement --- *)

Theorem blunt_and_complement : forall x,
  simplify_blunt (BinOp And x (UnOp Not x)) = BConst false.
Proof.
  intro x. cbn -[expr_eqb]. unfold blunt_and. cbn -[expr_eqb].
  destruct (as_bconst x) as [a|]; cbn -[expr_eqb];
  (destruct (is_not x) as [x'|];
   [ destruct (expr_eqb x' (UnOp Not x)); [reflexivity|];
     rewrite expr_eqb_refl; reflexivity
   | rewrite expr_eqb_refl; reflexivity ]).
Qed.

Theorem blunt_and_complement_l : forall x,
  simplify_blunt (BinOp And (UnOp Not x) x) = BConst false.
Proof.
  intro x. cbn -[expr_eqb]. unfold blunt_and. cbn -[expr_eqb].
  destruct (as_bconst x) as [a|]; cbn -[expr_eqb];
  rewrite expr_eqb_refl; reflexivity.
Qed.

Theorem blunt_or_complement : forall x,
  simplify_blunt (BinOp Or x (UnOp Not x)) = BConst true.
Proof.
  intro x. cbn -[expr_eqb]. unfold blunt_or. cbn -[expr_eqb].
  destruct (as_bconst x) as [a|]; cbn -[expr_eqb];
  (destruct (is_not x) as [x'|];
   [ destruct (expr_eqb x' (UnOp Not x)); [reflexivity|];
     rewrite expr_eqb_refl; reflexivity
   | rewrite expr_eqb_refl; reflexivity ]).
Qed.

Theorem blunt_or_complement_l : forall x,
  simplify_blunt (BinOp Or (UnOp Not x) x) = BConst true.
Proof.
  intro x. cbn -[expr_eqb]. unfold blunt_or. cbn -[expr_eqb].
  destruct (as_bconst x) as [a|]; cbn -[expr_eqb];
  rewrite expr_eqb_refl; reflexivity.
Qed.

(* --- Semantics of comparison inversion --- *)

Definition eval_cmp (op : IROpCode) (a b : Z) : option bool :=
  match op with
  | Eq => Some (Z.eqb a b)
  | Ne => Some (negb (Z.eqb a b))
  | _ => None
  end.

(* Eq/Ne inversion negates the result. Order comparisons are excluded
   from invert_cmp (their inversion is unsound under NaN, which Z does
   not model -- proving them here would certify the removed rules). *)
Theorem invert_cmp_negates : forall op op' a b c,
  invert_cmp op = Some op' ->
  eval_cmp op a b = Some c ->
  eval_cmp op' a b = Some (negb c).
Proof.
  intros op op' a b c Hinv Heval.
  destruct op; simpl in Hinv; try discriminate;
  injection Hinv as <-; simpl in Heval |- *;
  injection Heval as <-; f_equal.
  (* Ne -> Eq *) rewrite negb_involutive. reflexivity.
Qed.


(* ================================================================ *)
(* Structural equality reflection                                    *)
(* ================================================================ *)

Lemma expr_eqb_eq : forall a b, expr_eqb a b = true -> a = b.
Proof.
  fix IH 1. intros a b Heq.
  destruct a, b; simpl in Heq; try discriminate.
  - apply Z.eqb_eq in Heq; subst; reflexivity.
  - destruct b0, b; simpl in Heq; try discriminate; reflexivity.
  - apply andb_true_iff in Heq; destruct Heq as [Hz Hp].
    apply Z.eqb_eq in Hz. apply Pos.eqb_eq in Hp.
    destruct q, q0; simpl in *; subst; reflexivity.
  - apply Nat.eqb_eq in Heq; subst; reflexivity.
  - destruct (IROpCode_eq_dec i i0) as [->|]; [|discriminate].
    apply andb_true_iff in Heq; destruct Heq as [H1 H2].
    f_equal; apply IH; assumption.
  - destruct (IROpCode_eq_dec i i0) as [->|]; [|discriminate].
    f_equal; apply IH; assumption.
  - apply andb_true_iff in Heq; destruct Heq as [H12 H3].
    apply andb_true_iff in H12; destruct H12 as [H1 H2].
    f_equal; apply IH; assumption.
  - apply andb_true_iff in Heq; destruct Heq as [H1 H2].
    f_equal; apply IH; assumption.
  - f_equal. simpl in Heq.
    revert l0 Heq.
    induction l as [|x xs IHl]; intros [|y ys] Heq; simpl in Heq;
    try discriminate; [reflexivity|].
    apply andb_true_iff in Heq; destruct Heq as [Hh Ht].
    f_equal; [apply IH; exact Hh | apply IHl; exact Ht].
  - apply andb_true_iff in Heq; destruct Heq as [Hs Hcs].
    f_equal; [apply IH; exact Hs|].
    simpl in Hcs. revert l0 Hcs.
    induction l as [|[pa pb] xs IHl]; intros [|[qa qb] ys] Hcs;
    simpl in Hcs; try discriminate; [reflexivity|].
    apply andb_true_iff in Hcs; destruct Hcs as [Hab Ht].
    apply andb_true_iff in Hab; destruct Hab as [Ha Hb].
    f_equal; [f_equal; apply IH; assumption | apply IHl; exact Ht].
Qed.


(* ================================================================ *)
(* Semantic soundness: simplify_blunt preserves eval_bool            *)
(* ================================================================ *)

(* Helper: is_not e = Some e' implies e = UnOp Not e' *)
Local Lemma is_not_Some : forall e e', is_not e = Some e' -> e = UnOp Not e'.
Proof.
  intros e e' H. destruct e; simpl in H; try discriminate.
  destruct i; try discriminate. injection H as <-. reflexivity.
Qed.

(* Helper: as_bconst e = Some b implies e = BConst b *)
Local Lemma as_bconst_Some : forall e b, as_bconst e = Some b -> e = BConst b.
Proof.
  intros e b H. destruct e; simpl in H; try discriminate.
  injection H as <-. reflexivity.
Qed.

(* And complement: And (Not x) x always evaluates to false *)
Local Lemma and_not_l : forall x rho bv,
  (match eval_expr (BinOp And (UnOp Not x) x) rho with
   | Some v => Some (negb (v =? 0)%Z) | None => None end) = Some bv ->
  bv = false.
Proof.
  intros x rho bv H. simpl in H.
  destruct (eval_expr x rho); simpl in H; [|discriminate].
  destruct z; simpl in H; congruence.
Qed.

(* And complement: And x (Not x) always evaluates to false *)
Local Lemma and_not_r : forall x rho bv,
  (match eval_expr (BinOp And x (UnOp Not x)) rho with
   | Some v => Some (negb (v =? 0)%Z) | None => None end) = Some bv ->
  bv = false.
Proof.
  intros x rho bv H. simpl in H.
  destruct (eval_expr x rho); simpl in H; [|discriminate].
  destruct z; simpl in H; congruence.
Qed.

(* Or complement: Or (Not x) x always evaluates to true *)
Local Lemma or_not_l : forall x rho bv,
  (match eval_expr (BinOp Or (UnOp Not x) x) rho with
   | Some v => Some (negb (v =? 0)%Z) | None => None end) = Some bv ->
  bv = true.
Proof.
  intros x rho bv H. simpl in H.
  destruct (eval_expr x rho); simpl in H; [|discriminate].
  destruct z; simpl in H; congruence.
Qed.

(* Or complement: Or x (Not x) always evaluates to true *)
Local Lemma or_not_r : forall x rho bv,
  (match eval_expr (BinOp Or x (UnOp Not x)) rho with
   | Some v => Some (negb (v =? 0)%Z) | None => None end) = Some bv ->
  bv = true.
Proof.
  intros x rho bv H. simpl in H.
  destruct (eval_expr x rho); simpl in H; [|discriminate].
  destruct z; simpl in H; congruence.
Qed.

Theorem simplify_blunt_bool_sound : forall e rho bv,
  eval_bool e rho = Some bv ->
  eval_bool (simplify_blunt e) rho = Some bv.
Proof.
  unfold eval_bool. intros e rho bv H.
  destruct e as [| | | |op l r|op u|c t f| | |]; try exact H.
  - (* BinOp *)
    destruct op; try exact H.
    + (* And *)
      cbn [simplify_blunt]. unfold blunt_and.
      destruct (as_bconst l) as [a|] eqn:Hal;
      [destruct (as_bconst r) as [b|] eqn:Har|].
      * (* two boolean literals *)
        apply as_bconst_Some in Hal. apply as_bconst_Some in Har. subst.
        destruct a, b; simpl in *; exact H.
      * (* l literal, r not: complement fallback *)
        apply as_bconst_Some in Hal. subst.
        destruct (is_not (BConst a)) eqn:Hnl; [discriminate Hnl|].
        destruct (is_not r) eqn:Hnr; [|exact H].
        destruct (expr_eqb (BConst a) e) eqn:Hle; [|exact H].
        apply is_not_Some in Hnr. apply expr_eqb_eq in Hle. subst. simpl.
        assert (bv = false) by (eapply and_not_r; exact H). subst. reflexivity.
      * (* l not a literal *)
        destruct (is_not l) eqn:Hnl.
        ** destruct (expr_eqb e r) eqn:Her.
           { apply is_not_Some in Hnl. apply expr_eqb_eq in Her. subst. simpl.
             assert (bv = false) by (eapply and_not_l; exact H). subst. reflexivity. }
           { destruct (is_not r) eqn:Hnr; [|exact H].
             destruct (expr_eqb l e0) eqn:Hle0; [|exact H].
             apply is_not_Some in Hnr. apply expr_eqb_eq in Hle0. subst. simpl.
             assert (bv = false) by (eapply and_not_r; exact H). subst. reflexivity. }
        ** destruct (is_not r) eqn:Hnr; [|exact H].
           destruct (expr_eqb l e) eqn:Hle; [|exact H].
           apply is_not_Some in Hnr. apply expr_eqb_eq in Hle. subst. simpl.
           assert (bv = false) by (eapply and_not_r; exact H). subst. reflexivity.
    + (* Or *)
      cbn [simplify_blunt]. unfold blunt_or.
      destruct (as_bconst l) as [a|] eqn:Hal;
      [destruct (as_bconst r) as [b|] eqn:Har|].
      * apply as_bconst_Some in Hal. apply as_bconst_Some in Har. subst.
        destruct a, b; simpl in *; exact H.
      * apply as_bconst_Some in Hal. subst.
        destruct (is_not (BConst a)) eqn:Hnl; [discriminate Hnl|].
        destruct (is_not r) eqn:Hnr; [|exact H].
        destruct (expr_eqb (BConst a) e) eqn:Hle; [|exact H].
        apply is_not_Some in Hnr. apply expr_eqb_eq in Hle. subst. simpl.
        assert (bv = true) by (eapply or_not_r; exact H). subst. reflexivity.
      * destruct (is_not l) eqn:Hnl.
        ** destruct (expr_eqb e r) eqn:Her.
           { apply is_not_Some in Hnl. apply expr_eqb_eq in Her. subst. simpl.
             assert (bv = true) by (eapply or_not_l; exact H). subst. reflexivity. }
           { destruct (is_not r) eqn:Hnr; [|exact H].
             destruct (expr_eqb l e0) eqn:Hle0; [|exact H].
             apply is_not_Some in Hnr. apply expr_eqb_eq in Hle0. subst. simpl.
             assert (bv = true) by (eapply or_not_r; exact H). subst. reflexivity. }
        ** destruct (is_not r) eqn:Hnr; [|exact H].
           destruct (expr_eqb l e) eqn:Hle; [|exact H].
           apply is_not_Some in Hnr. apply expr_eqb_eq in Hle. subst. simpl.
           assert (bv = true) by (eapply or_not_r; exact H). subst. reflexivity.
  - (* UnOp Not *)
    destruct op; try exact H.
    destruct u as [| | | |iu lu ru| | | | |]; try exact H.
    (* Not (BinOp iu lu ru): Eq/Ne are not evaluated by the model
       (eval_expr has no comparison case), so preservation is vacuous;
       other ops keep the original expression. *)
    simpl. destruct (invert_cmp iu) eqn:Hcmp; try exact H.
    destruct iu; simpl in Hcmp; try discriminate; simpl in *; exact H.
  - (* IfExpr *)
    destruct c; try exact H. destruct b; exact H.
Qed.
