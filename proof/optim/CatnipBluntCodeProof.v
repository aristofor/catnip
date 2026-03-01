(* FILE: proof/optim/CatnipBluntCodeProof.v *)
(* Blunt Code — Boolean algebra simplifications.
 *
 * Source: catnip_rs/src/semantic/blunt_code.rs
 *
 * Proves:
 *   - Comparison inversion (invert_cmp_involution, invert_cmp_negates)
 *   - Double negation, NOT on comparisons
 *   - Boolean equality simplification (eq_true/false)
 *   - Idempotence (And/Or x x = x)
 *   - Complement (And x (Not x) = false, Or x (Not x) = true)
 *   - If constant branch elimination
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
(* C. Blunt Code — Boolean Algebra                                   *)
(*                                                                    *)
(* Simplifications from blunt_code.rs: double negation, comparison    *)
(* inversion, boolean equality, idempotence, complement.              *)
(* ================================================================ *)

Definition invert_cmp (oc : IROpCode) : option IROpCode :=
  match oc with
  | Eq => Some Ne | Ne => Some Eq
  | Lt => Some Ge | Le => Some Gt
  | Gt => Some Le | Ge => Some Lt
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

Theorem invert_cmp_domain : forall oc,
  is_comparison_op oc = true -> exists oc', invert_cmp oc = Some oc'.
Proof.
  intros oc H;
  destruct oc; simpl in H; try discriminate; simpl; eauto.
Qed.

(* Blunt code simplification — nested matches for Coq compatibility *)
Definition simplify_blunt (e : Expr) : Expr :=
  match e with
  | UnOp Not inner =>
      match inner with
      (* Double negation *)
      | UnOp Not x => x
      (* NOT on comparisons *)
      | BinOp op a b =>
          match invert_cmp op with
          | Some op' => BinOp op' a b
          | None => e
          end
      | _ => e
      end
  | BinOp Eq e1 e2 =>
      match e2 with
      | BConst true => e1
      | BConst false => UnOp Not e1
      | _ => match e1 with
             | BConst true => e2
             | BConst false => UnOp Not e2
             | _ => e
             end
      end
  | BinOp And e1 e2 =>
      if expr_eqb e1 e2 then e1
      else match is_not e1 with
           | Some e1' => if expr_eqb e1' e2 then BConst false
                         else match is_not e2 with
                              | Some e2' => if expr_eqb e1 e2' then BConst false else e
                              | None => e
                              end
           | None =>
             match is_not e2 with
             | Some e2' => if expr_eqb e1 e2' then BConst false else e
             | None => e
             end
           end
  | BinOp Or e1 e2 =>
      if expr_eqb e1 e2 then e1
      else match is_not e1 with
           | Some e1' => if expr_eqb e1' e2 then BConst true
                         else match is_not e2 with
                              | Some e2' => if expr_eqb e1 e2' then BConst true else e
                              | None => e
                              end
           | None =>
             match is_not e2 with
             | Some e2' => if expr_eqb e1 e2' then BConst true else e
             | None => e
             end
           end
  | IfExpr c t f =>
      match c with
      | BConst true => t
      | BConst false => f
      | _ => e
      end
  | _ => e
  end.

(* --- Blunt code correctness theorems --- *)

Theorem blunt_double_neg : forall x,
  simplify_blunt (UnOp Not (UnOp Not x)) = x.
Proof. reflexivity. Qed.

Theorem blunt_not_eq : forall a b,
  simplify_blunt (UnOp Not (BinOp Eq a b)) = BinOp Ne a b.
Proof. reflexivity. Qed.

Theorem blunt_not_ne : forall a b,
  simplify_blunt (UnOp Not (BinOp Ne a b)) = BinOp Eq a b.
Proof. reflexivity. Qed.

Theorem blunt_not_lt : forall a b,
  simplify_blunt (UnOp Not (BinOp Lt a b)) = BinOp Ge a b.
Proof. reflexivity. Qed.

Theorem blunt_not_le : forall a b,
  simplify_blunt (UnOp Not (BinOp Le a b)) = BinOp Gt a b.
Proof. reflexivity. Qed.

Theorem blunt_not_gt : forall a b,
  simplify_blunt (UnOp Not (BinOp Gt a b)) = BinOp Le a b.
Proof. reflexivity. Qed.

Theorem blunt_not_ge : forall a b,
  simplify_blunt (UnOp Not (BinOp Ge a b)) = BinOp Lt a b.
Proof. reflexivity. Qed.

Theorem blunt_eq_true_r : forall x,
  simplify_blunt (BinOp Eq x (BConst true)) = x.
Proof. reflexivity. Qed.

Theorem blunt_eq_false_r : forall x,
  simplify_blunt (BinOp Eq x (BConst false)) = UnOp Not x.
Proof. reflexivity. Qed.

(* _l variants hold when e2 is not a BConst (no overlap with _r rules) *)
Theorem blunt_eq_true_l : forall x,
  (forall b, x <> BConst b) ->
  simplify_blunt (BinOp Eq (BConst true) x) = x.
Proof.
  intros x Hx; destruct x; simpl; try reflexivity.
  exfalso; apply (Hx b); reflexivity.
Qed.

Theorem blunt_eq_false_l : forall x,
  (forall b, x <> BConst b) ->
  simplify_blunt (BinOp Eq (BConst false) x) = UnOp Not x.
Proof.
  intros x Hx; destruct x; simpl; try reflexivity.
  exfalso; apply (Hx b); reflexivity.
Qed.

Theorem blunt_and_idempotent : forall x,
  simplify_blunt (BinOp And x x) = x.
Proof. intro x; simpl; rewrite expr_eqb_refl; reflexivity. Qed.

Theorem blunt_or_idempotent : forall x,
  simplify_blunt (BinOp Or x x) = x.
Proof. intro x; simpl; rewrite expr_eqb_refl; reflexivity. Qed.

(* Helper: if b then x else x = x *)
Lemma if_same {A : Type} (b : bool) (x : A) : (if b then x else x) = x.
Proof. destruct b; reflexivity. Qed.

(* Complement: x && not(x) = False, not(x) && x = False
   Proof strategy: unfold + rewrite without simpl to avoid
   expansion of IROpCode_eq_dec. *)

Theorem blunt_and_complement : forall x,
  simplify_blunt (BinOp And x (UnOp Not x)) = BConst false.
Proof.
  intro x. cbn -[expr_eqb]. rewrite expr_eqb_not_self.
  destruct (is_not x) as [e1'|].
  - destruct (expr_eqb e1' (UnOp Not x)); [reflexivity|].
    rewrite expr_eqb_refl. reflexivity.
  - rewrite expr_eqb_refl. reflexivity.
Qed.

Theorem blunt_and_complement_l : forall x,
  simplify_blunt (BinOp And (UnOp Not x) x) = BConst false.
Proof.
  intro x. cbn -[expr_eqb]. rewrite expr_eqb_unop_self.
  rewrite expr_eqb_refl. reflexivity.
Qed.

Theorem blunt_or_complement : forall x,
  simplify_blunt (BinOp Or x (UnOp Not x)) = BConst true.
Proof.
  intro x. cbn -[expr_eqb]. rewrite expr_eqb_not_self.
  destruct (is_not x) as [e1'|].
  - destruct (expr_eqb e1' (UnOp Not x)); [reflexivity|].
    rewrite expr_eqb_refl. reflexivity.
  - rewrite expr_eqb_refl. reflexivity.
Qed.

Theorem blunt_or_complement_l : forall x,
  simplify_blunt (BinOp Or (UnOp Not x) x) = BConst true.
Proof.
  intro x. cbn -[expr_eqb]. rewrite expr_eqb_unop_self.
  rewrite expr_eqb_refl. reflexivity.
Qed.

Theorem blunt_if_true : forall t f,
  simplify_blunt (IfExpr (BConst true) t f) = t.
Proof. reflexivity. Qed.

Theorem blunt_if_false : forall t f,
  simplify_blunt (IfExpr (BConst false) t f) = f.
Proof. reflexivity. Qed.

(* --- Blunt code semantic preservation --- *)

(* Double negation preserves boolean semantics *)
Theorem blunt_double_neg_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (UnOp Not (UnOp Not x)) rho = Some b.
Proof.
  intros x rho b Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  destruct (v =? 0)%Z; simpl; exact Hx.
Qed.

(* Comparison semantics for inversion proofs *)
Definition eval_cmp (op : IROpCode) (a b : Z) : option bool :=
  match op with
  | Eq => Some (Z.eqb a b)
  | Ne => Some (negb (Z.eqb a b))
  | Lt => Some (Z.ltb a b)
  | Le => Some (Z.leb a b)
  | Gt => Some (Z.ltb b a)
  | Ge => Some (Z.leb b a)
  | _ => None
  end.

Lemma leb_negb_ltb : forall n m, Z.leb n m = negb (Z.ltb m n).
Proof.
  intros. unfold Z.leb, Z.ltb.
  rewrite (Z.compare_antisym n m).
  destruct (n ?= m)%Z; reflexivity.
Qed.

Lemma ltb_negb_leb : forall n m, Z.ltb n m = negb (Z.leb m n).
Proof.
  intros. unfold Z.ltb, Z.leb.
  rewrite (Z.compare_antisym n m).
  destruct (n ?= m)%Z; reflexivity.
Qed.

(* Comparison inversion negates the result — all 6 ops at once *)
Theorem invert_cmp_negates : forall op op' a b c,
  invert_cmp op = Some op' ->
  eval_cmp op a b = Some c ->
  eval_cmp op' a b = Some (negb c).
Proof.
  intros op op' a b c Hinv Heval.
  destruct op; simpl in Hinv; try discriminate;
  injection Hinv as <-; simpl in Heval |- *;
  injection Heval as <-; f_equal.
  (* Eq -> Ne solved by f_equal *)
  - (* Ne -> Eq *) rewrite negb_involutive. reflexivity.
  - (* Lt -> Ge *) apply leb_negb_ltb.
  - (* Le -> Gt *) apply ltb_negb_leb.
  - (* Gt -> Le *) apply leb_negb_ltb.
  - (* Ge -> Lt *) apply ltb_negb_leb.
Qed.

(* And idempotence preserves boolean semantics *)
Theorem blunt_and_idempotent_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp And x x) rho = Some b.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  destruct (v =? 0)%Z eqn:Hv; simpl.
  - exact Hx.
  - rewrite Hv. exact Hx.
Qed.

(* Or idempotence preserves boolean semantics *)
Theorem blunt_or_idempotent_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp Or x x) rho = Some b.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  destruct (v =? 0)%Z eqn:Hv; simpl; rewrite Hv; exact Hx.
Qed.

(* If-true/false preserve eval_expr semantics *)
Theorem blunt_if_true_sem : forall t f rho v,
  eval_expr t rho = Some v ->
  eval_expr (IfExpr (BConst true) t f) rho = Some v.
Proof. intros. simpl. exact H. Qed.

Theorem blunt_if_false_sem : forall t f rho v,
  eval_expr f rho = Some v ->
  eval_expr (IfExpr (BConst false) t f) rho = Some v.
Proof. intros. simpl. exact H. Qed.


(* --- Structural equality reflection --- *)

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

(* --- Semantic soundness: simplify_blunt preserves eval_bool --- *)

(* Helper: is_not e = Some e' implies e = UnOp Not e' *)
Local Lemma is_not_Some : forall e e', is_not e = Some e' -> e = UnOp Not e'.
Proof.
  intros e e' H. destruct e; simpl in H; try discriminate.
  destruct i; try discriminate. injection H as <-. reflexivity.
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
    + (* Eq: eval_expr = None *) simpl in H. discriminate.
    + (* And *)
      cbn [simplify_blunt].
      destruct (expr_eqb l r) eqn:Heq.
      * (* idempotent *)
        apply expr_eqb_eq in Heq; subst.
        simpl in H.
        destruct (eval_expr r rho) eqn:Hr; simpl in H; [|discriminate].
        destruct z; simpl in *; congruence.
      * (* non-idempotent *)
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
      cbn [simplify_blunt].
      destruct (expr_eqb l r) eqn:Heq.
      * (* idempotent *)
        apply expr_eqb_eq in Heq; subst.
        simpl in H.
        destruct (eval_expr r rho) eqn:Hr; simpl in H; [|discriminate].
        destruct z; simpl in *; congruence.
      * (* non-idempotent *)
        destruct (is_not l) eqn:Hnl.
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
    destruct u as [| | | |iu lu ru|iu xu| | | |]; try exact H.
    + (* Not (BinOp iu lu ru) *)
      simpl. destruct (invert_cmp iu) eqn:Hcmp; try exact H.
      destruct iu; simpl in Hcmp; try discriminate; simpl in *; exact H.
    + (* Not (UnOp iu xu) *)
      destruct iu; try exact H.
      simpl in *. destruct (eval_expr xu rho); simpl in *; try discriminate.
      destruct (z =? 0)%Z; simpl in *; exact H.
  - (* IfExpr *)
    destruct c; try exact H. destruct b; exact H.
Qed.
