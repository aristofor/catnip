(* FILE: proof/optim/CatnipDCEFlattenProof.v *)
(* Dead Code Elimination, Block Flattening, Pass Composition.
 *
 * Source: catnip_rs/src/semantic/dead_code_elimination.rs
 *         catnip_rs/src/semantic/block_flattening.rs
 *
 * Proves:
 *   - DCE: if-true/false elimination, while-false, empty/singleton block
 *   - Block flattening: flatten_one, idempotence, flatten_block_sound
 *   - Pass composition: compose_preserves_eval, compose_two_idempotent
 *   - Lowering to IRPure (sr_mul_one_r_ir, sr_add_zero_r_ir, blunt_double_neg_ir)
 *
 * Depends on: CatnipIR.v, CatnipExprModel.v,
 *             CatnipStrengthRedProof.v, CatnipBluntCodeProof.v
 *
 * 0 Admitted.
 *)

From Coq Require Import List ZArith Bool Lia QArith.
From Catnip Require Import CatnipIR.
From Catnip Require Import CatnipExprModel.
From Catnip Require Import CatnipStrengthRedProof.
From Catnip Require Import CatnipBluntCodeProof.
Import ListNotations.


(* ================================================================ *)
(* D. Dead Code Elimination                                          *)
(*                                                                    *)
(* From dead_code_elimination.rs.                                     *)
(* None = expression eliminated entirely.                             *)
(* ================================================================ *)

Definition eliminate_dead (e : Expr) : option Expr :=
  match e with
  | IfExpr (BConst true) t _ => Some t
  | IfExpr (BConst false) _ f => Some f
  | WhileExpr (BConst false) _ => None
  | Block [] => None
  | Block [e'] => Some e'
  | other => Some other
  end.

Theorem dce_if_true : forall t f,
  eliminate_dead (IfExpr (BConst true) t f) = Some t.
Proof. reflexivity. Qed.

Theorem dce_if_false : forall t f,
  eliminate_dead (IfExpr (BConst false) t f) = Some f.
Proof. reflexivity. Qed.

Theorem dce_while_false : forall body,
  eliminate_dead (WhileExpr (BConst false) body) = None.
Proof. reflexivity. Qed.

Theorem dce_empty_block : eliminate_dead (Block []) = None.
Proof. reflexivity. Qed.

Theorem dce_singleton_block : forall e,
  eliminate_dead (Block [e]) = Some e.
Proof. reflexivity. Qed.

Theorem dce_if_true_sem : forall t f rho v,
  eval_expr t rho = Some v ->
  eval_expr (IfExpr (BConst true) t f) rho = Some v.
Proof. intros. simpl. assumption. Qed.

Theorem dce_if_false_sem : forall t f rho v,
  eval_expr f rho = Some v ->
  eval_expr (IfExpr (BConst false) t f) rho = Some v.
Proof. intros. simpl. assumption. Qed.


(* ================================================================ *)
(* E. Block Flattening                                               *)
(*                                                                    *)
(* From block_flattening.rs.                                          *)
(* Block [s1, Block [s2, s3], s4] -> Block [s1, s2, s3, s4]          *)
(* ================================================================ *)

(* flatten_one on Expr passes Coq's guard checker via nested fix. *)
Fixpoint flatten_one (e : Expr) : list Expr :=
  let fix go (l : list Expr) : list Expr :=
    match l with
    | [] => []
    | x :: xs => flatten_one x ++ go xs
    end
  in
  match e with
  | Block stmts => go stmts
  | _ => [e]
  end.

Definition flatten_stmts (stmts : list Expr) : list Expr :=
  let fix go (l : list Expr) : list Expr :=
    match l with
    | [] => []
    | x :: xs => flatten_one x ++ go xs
    end
  in go stmts.

Definition flatten_block (e : Expr) : Expr :=
  match e with
  | Block stmts => Block (flatten_stmts stmts)
  | other => other
  end.

(* flatten_one on non-Block is singleton *)
Lemma flatten_one_non_block : forall e,
  (forall l, e <> Block l) -> flatten_one e = [e].
Proof.
  intros e H; destruct e; simpl; try reflexivity.
  exfalso; apply (H l); reflexivity.
Qed.

(* flatten_stmts distributes over append *)
Theorem flatten_stmts_app : forall l1 l2,
  flatten_stmts (l1 ++ l2) = flatten_stmts l1 ++ flatten_stmts l2.
Proof.
  induction l1 as [|s l1' IH]; intros l2; simpl.
  - reflexivity.
  - rewrite IH. rewrite app_assoc. reflexivity.
Qed.

(* Output of flatten_one never contains top-level Blocks *)
Lemma flatten_one_no_blocks : forall e y,
  In y (flatten_one e) -> forall l, y <> Block l.
Proof.
  fix IH 1. intro e.
  destruct e; simpl; intros y Hin l' Heq.
  - destruct Hin as [Hin|[]]; congruence.
  - destruct Hin as [Hin|[]]; congruence.
  - destruct Hin as [Hin|[]]; congruence.
  - destruct Hin as [Hin|[]]; congruence.
  - destruct Hin as [Hin|[]]; congruence.
  - destruct Hin as [Hin|[]]; congruence.
  - destruct Hin as [Hin|[]]; congruence.
  - destruct Hin as [Hin|[]]; congruence.
  - (* Block case: recurse through the list *)
    induction l as [|x xs IHl]; simpl in Hin.
    + contradiction.
    + apply in_app_or in Hin. destruct Hin as [Hin|Hin].
      * exact (IH x y Hin l' Heq).
      * exact (IHl Hin).
  - destruct Hin as [Hin|[]]; congruence.
Qed.

(* flatten_stmts is identity on Block-free lists *)
Lemma flatten_stmts_no_blocks : forall stmts,
  (forall s, In s stmts -> forall l, s <> Block l) ->
  flatten_stmts stmts = stmts.
Proof.
  induction stmts as [|s rest IH]; intros Hno; simpl.
  - reflexivity.
  - assert (Hs : forall l, s <> Block l)
      by (intros; apply Hno; left; reflexivity).
    rewrite flatten_one_non_block by assumption.
    simpl. f_equal. apply IH. intros s' Hin.
    apply Hno. right. assumption.
Qed.

(* Idempotence: flatten_one output is already flat *)
Lemma flatten_stmts_flat : forall stmts,
  (forall s, In s stmts -> forall l, s <> Block l) ->
  flatten_stmts stmts = stmts.
Proof. exact flatten_stmts_no_blocks. Qed.

(* Key: output of flatten_stmts has no top-level Blocks *)
Lemma flatten_stmts_output_no_blocks : forall stmts y,
  In y (flatten_stmts stmts) -> forall l, y <> Block l.
Proof.
  induction stmts as [|x xs IH]; simpl; intros y Hin l Heq.
  - contradiction.
  - apply in_app_or in Hin. destruct Hin as [Hin|Hin].
    + exact (flatten_one_no_blocks x y Hin l Heq).
    + exact (IH y Hin l Heq).
Qed.

Theorem flatten_stmts_idempotent : forall stmts,
  flatten_stmts (flatten_stmts stmts) = flatten_stmts stmts.
Proof.
  intro stmts. apply flatten_stmts_no_blocks.
  intros s Hin l. exact (flatten_stmts_output_no_blocks stmts s Hin l).
Qed.

Theorem flatten_block_idempotent : forall e,
  flatten_block (flatten_block e) = flatten_block e.
Proof.
  intro e; destruct e; simpl; try reflexivity.
  f_equal. apply flatten_stmts_idempotent.
Qed.

Example flatten_nested :
  flatten_block (Block [Const 1; Block [Const 2; Const 3]; Const 4])
  = Block [Const 1; Const 2; Const 3; Const 4].
Proof. reflexivity. Qed.

Example flatten_deep :
  flatten_block (Block [Block [Block [Const 1]]; Const 2])
  = Block [Const 1; Const 2].
Proof. reflexivity. Qed.

(* --- Semantic soundness: flatten_block preserves eval_expr --- *)

Lemma eval_block_flatten_one : forall e rho v,
  eval_expr e rho = Some v ->
  eval_expr (Block (flatten_one e)) rho = Some v.
Proof.
  fix IH 1. intros e rho v H.
  destruct e; try (simpl; exact H).
  (* Block l *)
  destruct l as [|a [|b rest]].
  - simpl. exact H.
  - simpl in H. simpl. rewrite app_nil_r. apply IH. exact H.
  - simpl in H. discriminate.
Qed.

Theorem flatten_block_sound : forall e rho v,
  eval_expr e rho = Some v ->
  eval_expr (flatten_block e) rho = Some v.
Proof.
  intros e rho v H.
  destruct e; try exact H.
  (* Block l *)
  simpl. destruct l as [|a [|b rest]].
  - exact H.
  - simpl in H. simpl. rewrite app_nil_r.
    apply eval_block_flatten_one. exact H.
  - simpl in H. discriminate.
Qed.


(* ================================================================ *)
(* F. Pass Composition                                               *)
(* ================================================================ *)

Definition pass := Expr -> Expr.

Definition compose_passes (passes : list pass) (e : Expr) : Expr :=
  fold_left (fun acc f => f acc) passes e.

Definition preserves_eval (f : pass) : Prop :=
  forall e rho v,
    eval_expr e rho = Some v ->
    eval_expr (f e) rho = Some v.

Theorem compose_preserves_eval : forall passes,
  Forall preserves_eval passes ->
  preserves_eval (compose_passes passes).
Proof.
  unfold preserves_eval, compose_passes.
  induction passes as [|f rest IH]; intros Hall e rho v Heval.
  - simpl. assumption.
  - simpl. inversion Hall; subst. apply IH; auto.
Qed.

Definition idempotent (f : pass) : Prop :=
  forall e, f (f e) = f e.

Theorem compose_two_idempotent : forall f g,
  idempotent f -> idempotent g ->
  (forall e, f (g e) = g e) ->
  (forall e, g (f e) = f e) ->
  idempotent (fun e => g (f e)).
Proof.
  unfold idempotent. intros f g Hf Hg Hfg Hgf e.
  simpl. rewrite Hgf. apply Hfg.
Qed.


(* ================================================================ *)
(* G. Concrete Examples                                              *)
(* ================================================================ *)

Example ex_sr_mul :
  strength_reduce (BinOp Mul (Const 3) (Const 1)) = Const 3.
Proof. reflexivity. Qed.

Example ex_blunt_double_neg :
  simplify_blunt (UnOp Not (UnOp Not (Const 5))) = Const 5.
Proof. reflexivity. Qed.

Example ex_dce_if :
  eliminate_dead (IfExpr (BConst true) (Const 42) (Const 0)) = Some (Const 42).
Proof. reflexivity. Qed.

Example ex_flatten :
  flatten_block (Block [Block [Var 0; Var 1]; Var 2])
  = Block [Var 0; Var 1; Var 2].
Proof. reflexivity. Qed.

(* Pipeline: blunt then strength on not(not(x * 1)) *)
Example ex_pipeline :
  let e := UnOp Not (UnOp Not (BinOp Mul (Var 0) (Const 1))) in
  let step1 := simplify_blunt e in
  let step2 := strength_reduce step1 in
  step1 = BinOp Mul (Var 0) (Const 1) /\
  step2 = Var 0.
Proof. split; reflexivity. Qed.


(* ================================================================ *)
(* H. Lowering to IRPure                                             *)
(*                                                                    *)
(* expr_to_ir is defined in CatnipExprModel.v.                       *)
(* ================================================================ *)

Theorem sr_mul_one_r_ir : forall x,
  expr_to_ir (strength_reduce (BinOp Mul x (Const 1))) = expr_to_ir x.
Proof. reflexivity. Qed.

Theorem sr_add_zero_r_ir : forall x,
  expr_to_ir (strength_reduce (BinOp Add x (Const 0))) = expr_to_ir x.
Proof. reflexivity. Qed.

Theorem blunt_double_neg_ir : forall x,
  expr_to_ir (simplify_blunt (UnOp Not (UnOp Not x))) = expr_to_ir x.
Proof. reflexivity. Qed.
