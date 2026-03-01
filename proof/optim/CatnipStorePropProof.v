(* FILE: proof/optim/CatnipStorePropProof.v *)
(* CatnipStorePropProof.v — Store model + constant/copy propagation proofs
 *
 * Source of truth:
 *   catnip_rs/src/semantic/constant_propagation.rs
 *   catnip_rs/src/semantic/copy_propagation.rs
 *   catnip_rs/src/semantic/common_subexpression_elimination.rs
 *
 * This file introduces a mutable store (variable → value) that evolves
 * statement-by-statement through a block. The store enables proofs for
 * the stateful optimization passes: constant propagation, copy propagation,
 * and common subexpression elimination.
 *
 * The store is an association list (most recent binding wins), reusing
 * the same structure proven correct in CatnipScopeProof.v.
 *
 * Depends on:
 *   CatnipIR.v         (IROpCode, SetLocals)
 *   CatnipExprModel.v  (Expr, Var, Const, BinOp, env, eval_expr, expr_eqb)
 *)

From Coq Require Import List ZArith Bool Lia PeanoNat.
From Catnip Require Import CatnipIR.
From Catnip Require Import CatnipExprModel.
Import ListNotations.


(* ================================================================ *)
(* A. Store                                                          *)
(*                                                                    *)
(* Association list: variable id (nat) → Expr.                       *)
(* Head = most recent binding. Mirrors HashMap in Rust passes.        *)
(* ================================================================ *)

Definition Store := list (nat * Expr).

Fixpoint store_lookup (s : Store) (x : nat) : option Expr :=
  match s with
  | [] => None
  | (k, v) :: rest => if Nat.eqb k x then Some v else store_lookup rest x
  end.

Definition store_set (s : Store) (x : nat) (e : Expr) : Store := (x, e) :: s.

Definition store_empty : Store := [].


(* ================================================================ *)
(* B. Store Properties                                               *)
(* ================================================================ *)

Lemma store_lookup_empty : forall x,
  store_lookup store_empty x = None.
Proof. reflexivity. Qed.

Lemma store_lookup_set_same : forall s x e,
  store_lookup (store_set s x e) x = Some e.
Proof.
  intros s x e. simpl. rewrite Nat.eqb_refl. reflexivity.
Qed.

Lemma store_lookup_set_other : forall s x y e,
  x <> y ->
  store_lookup (store_set s x e) y = store_lookup s y.
Proof.
  intros s x y e Hne. simpl.
  destruct (Nat.eqb x y) eqn:Heq.
  - apply Nat.eqb_eq in Heq. contradiction.
  - reflexivity.
Qed.


(* ================================================================ *)
(* C. Store-Environment Compatibility                                *)
(*                                                                    *)
(* A store is compatible with an environment rho when every           *)
(* constant tracked in the store evaluates to the same value          *)
(* that rho assigns to the variable.                                  *)
(* ================================================================ *)

Definition is_const (e : Expr) : bool :=
  match e with
  | Const _ | BConst _ | QConst _ => true
  | _ => false
  end.

Definition store_compatible (s : Store) (rho : env) : Prop :=
  forall x c,
    store_lookup s x = Some c ->
    is_const c = true ->
    eval_expr c rho = Some (rho x).

Lemma store_compatible_empty : forall rho,
  store_compatible store_empty rho.
Proof.
  unfold store_compatible. intros rho x c H. simpl in H. discriminate.
Qed.

Lemma store_compatible_set : forall s rho x c,
  store_compatible s rho ->
  is_const c = true ->
  eval_expr c rho = Some (rho x) ->
  store_compatible (store_set s x c) rho.
Proof.
  unfold store_compatible. intros s rho x c Hcomp Hconst Heval y d Hlookup Hd.
  simpl in Hlookup.
  destruct (Nat.eqb x y) eqn:Heq.
  - apply Nat.eqb_eq in Heq. subst. inversion Hlookup. subst. exact Heval.
  - apply Hcomp; assumption.
Qed.


(* ================================================================ *)
(* D. Constant Propagation                                           *)
(*                                                                    *)
(* Replace Var x by its constant value when x is in the store.       *)
(* Mirrors visit_ref in constant_propagation.rs.                     *)
(* ================================================================ *)

Fixpoint const_prop (s : Store) (e : Expr) : Expr :=
  match e with
  | Var x =>
      match store_lookup s x with
      | Some c => if is_const c then c else e
      | None => e
      end
  | BinOp op l r => BinOp op (const_prop s l) (const_prop s r)
  | UnOp op x => UnOp op (const_prop s x)
  | IfExpr c t f => IfExpr (const_prop s c) (const_prop s t) (const_prop s f)
  | _ => e
  end.

Theorem const_prop_correct : forall s rho e v,
  store_compatible s rho ->
  eval_expr e rho = Some v ->
  eval_expr (const_prop s e) rho = Some v.
Proof.
  intros s rho e. revert s rho.
  induction e; intros s rho v Hcomp Heval; simpl; try exact Heval.
  - (* Var *)
    simpl in Heval. injection Heval as Hv. subst v.
    destruct (store_lookup s n) as [c|] eqn:Hlookup.
    + destruct (is_const c) eqn:Hconst.
      * exact (Hcomp n c Hlookup Hconst).
      * reflexivity.
    + reflexivity.
  - (* BinOp *)
    simpl in Heval.
    destruct i; simpl in Heval |- *;
    try (destruct (eval_expr e1 rho) eqn:He1; [|discriminate];
         destruct (eval_expr e2 rho) eqn:He2; [|discriminate];
         rewrite (IHe1 s rho z Hcomp He1);
         rewrite (IHe2 s rho z0 Hcomp He2);
         exact Heval);
    try exact Heval.
  - (* UnOp *)
    simpl in Heval.
    destruct i; simpl in Heval |- *;
    try (destruct (eval_expr e rho) eqn:He; [|discriminate];
         rewrite (IHe s rho z Hcomp He);
         exact Heval);
    try exact Heval.
  - (* IfExpr *)
    simpl in Heval.
    destruct (eval_expr e1 rho) eqn:He1; [|discriminate].
    rewrite (IHe1 s rho z Hcomp He1).
    destruct (Z.eqb z 0).
    + apply IHe3; assumption.
    + apply IHe2; assumption.
Qed.


(* ================================================================ *)
(* E. Block-level Constant Propagation                               *)
(*                                                                    *)
(* Models the Rust pass traversing a Block [stmt1; stmt2; ...]       *)
(* and accumulating known constants in the store.                    *)
(*                                                                    *)
(* Assign := (var_id, value_expr). Simplified model of SET_LOCALS.   *)
(* ================================================================ *)

Inductive Stmt :=
  | SAssign : nat -> Expr -> Stmt     (* SET_LOCALS x = expr *)
  | SExpr   : Expr -> Stmt.           (* any other statement *)

Definition stmt_propagate (s : Store) (st : Stmt) : Store * Stmt :=
  match st with
  | SAssign x e =>
      let e' := const_prop s e in
      let s' := if is_const e' then store_set s x e' else s in
      (s', SAssign x e')
  | SExpr e => (s, SExpr (const_prop s e))
  end.

Fixpoint block_propagate (s : Store) (stmts : list Stmt) : Store * list Stmt :=
  match stmts with
  | [] => (s, [])
  | st :: rest =>
      let '(s', st') := stmt_propagate s st in
      let '(s_final, rest') := block_propagate s' rest in
      (s_final, st' :: rest')
  end.

Lemma block_propagate_single_const : forall s x e,
  is_const (const_prop s e) = true ->
  block_propagate s [SAssign x e] =
    (store_set s x (const_prop s e), [SAssign x (const_prop s e)]).
Proof.
  intros s x e Hconst. simpl.
  rewrite Hconst. reflexivity.
Qed.


(* ================================================================ *)
(* F. Copy Propagation                                               *)
(*                                                                    *)
(* Store maps variable → source variable (nat → nat).                *)
(* Mirrors copy_propagation.rs: track x = y, replace Ref(x) by y.   *)
(* ================================================================ *)

Definition CopyStore := list (nat * nat).

Fixpoint copy_lookup (s : CopyStore) (x : nat) : option nat :=
  match s with
  | [] => None
  | (k, v) :: rest => if Nat.eqb k x then Some v else copy_lookup rest x
  end.

Definition copy_set (s : CopyStore) (x : nat) (src : nat) : CopyStore :=
  (x, src) :: s.

(* Follow copy chain: x → y → z → ... until no more mapping *)
Fixpoint copy_resolve (s : CopyStore) (x : nat) (fuel : nat) : nat :=
  match fuel with
  | 0 => x
  | S fuel' =>
      match copy_lookup s x with
      | Some y => copy_resolve s y fuel'
      | None => x
      end
  end.

Fixpoint copy_prop (s : CopyStore) (fuel : nat) (e : Expr) : Expr :=
  match e with
  | Var x => Var (copy_resolve s x fuel)
  | BinOp op l r => BinOp op (copy_prop s fuel l) (copy_prop s fuel r)
  | UnOp op x => UnOp op (copy_prop s fuel x)
  | IfExpr c t f => IfExpr (copy_prop s fuel c) (copy_prop s fuel t) (copy_prop s fuel f)
  | _ => e
  end.

(* Copy-store compatibility: if x maps to y, then rho x = rho y *)
Definition copy_compatible (s : CopyStore) (rho : env) : Prop :=
  forall x y,
    copy_lookup s x = Some y ->
    rho x = rho y.

Lemma copy_compatible_empty : forall rho,
  copy_compatible [] rho.
Proof.
  unfold copy_compatible. intros rho x y H. simpl in H. discriminate.
Qed.

Lemma copy_resolve_correct : forall s rho fuel x,
  copy_compatible s rho ->
  rho (copy_resolve s x fuel) = rho x.
Proof.
  intros s rho fuel. induction fuel as [|fuel' IH]; intros x Hcomp.
  - simpl. reflexivity.
  - simpl. destruct (copy_lookup s x) as [y|] eqn:Hlookup.
    + rewrite IH by exact Hcomp.
      apply Hcomp in Hlookup. symmetry. exact Hlookup.
    + reflexivity.
Qed.

Theorem copy_prop_correct : forall s rho fuel e v,
  copy_compatible s rho ->
  eval_expr e rho = Some v ->
  eval_expr (copy_prop s fuel e) rho = Some v.
Proof.
  intros s rho fuel e. revert s rho fuel.
  induction e; intros s rho fuel v Hcomp Heval; simpl; try exact Heval.
  - (* Var *)
    simpl in Heval |- *. injection Heval as Hv. subst v.
    f_equal. apply copy_resolve_correct. exact Hcomp.
  - (* BinOp *)
    simpl in Heval.
    destruct i; simpl in Heval |- *;
    try (destruct (eval_expr e1 rho) eqn:He1; [|discriminate];
         destruct (eval_expr e2 rho) eqn:He2; [|discriminate];
         rewrite (IHe1 s rho fuel z Hcomp He1);
         rewrite (IHe2 s rho fuel z0 Hcomp He2);
         exact Heval);
    try exact Heval.
  - (* UnOp *)
    simpl in Heval.
    destruct i; simpl in Heval |- *;
    try (destruct (eval_expr e rho) eqn:He; [|discriminate];
         rewrite (IHe s rho fuel z Hcomp He);
         exact Heval);
    try exact Heval.
  - (* IfExpr *)
    simpl in Heval.
    destruct (eval_expr e1 rho) eqn:He1; [|discriminate].
    rewrite (IHe1 s rho fuel z Hcomp He1).
    destruct (Z.eqb z 0).
    + apply IHe3; assumption.
    + apply IHe2; assumption.
Qed.


(* ================================================================ *)
(* G. Common Subexpression Elimination (CSE)                         *)
(*                                                                    *)
(* Store maps expression → variable (Expr → nat).                    *)
(* If an expression was already computed and stored in variable t,   *)
(* replace the expression by Var t.                                  *)
(* ================================================================ *)

Definition CSEStore := list (Expr * nat).

Fixpoint cse_lookup (s : CSEStore) (e : Expr) : option nat :=
  match s with
  | [] => None
  | (expr, var) :: rest =>
      if expr_eqb expr e then Some var else cse_lookup rest e
  end.

Definition cse_set (s : CSEStore) (e : Expr) (var : nat) : CSEStore :=
  (e, var) :: s.

Definition cse_replace (s : CSEStore) (e : Expr) : Expr :=
  match cse_lookup s e with
  | Some var => Var var
  | None => e
  end.

(* CSE-store compatibility: stored expr evaluates to rho(var) *)
Definition cse_compatible (s : CSEStore) (rho : env) : Prop :=
  forall e x,
    cse_lookup s e = Some x ->
    eval_expr e rho = Some (rho x).

Lemma cse_compatible_empty : forall rho,
  cse_compatible [] rho.
Proof.
  unfold cse_compatible. intros rho e x H. simpl in H. discriminate.
Qed.

Theorem cse_replace_correct : forall s rho e v,
  cse_compatible s rho ->
  eval_expr e rho = Some v ->
  eval_expr (cse_replace s e) rho = Some v.
Proof.
  intros s rho e v Hcomp Heval.
  unfold cse_replace.
  destruct (cse_lookup s e) as [x|] eqn:Hlookup.
  - simpl. f_equal.
    apply Hcomp in Hlookup. rewrite Heval in Hlookup.
    inversion Hlookup. reflexivity.
  - exact Heval.
Qed.

(* ================================================================ *)
(* H. expr_eqb Soundness (needed for CSE)                           *)
(*                                                                    *)
(* Structural equality (expr_eqb) implies Leibniz equality.          *)
(* Uses structural comparison on Q components (Qnum, Qden) so       *)
(* the boolean test coincides with propositional equality.            *)
(* ================================================================ *)

Lemma expr_eqb_sound : forall a b, expr_eqb a b = true -> a = b.
Proof.
  fix IH 1.
  intros a b.
  destruct a, b; simpl; intro H; try discriminate.
  - (* Const, Const *)
    apply (proj1 (Z.eqb_eq _ _)) in H. subst. reflexivity.
  - (* BConst, BConst *)
    apply eqb_prop in H. subst. reflexivity.
  - (* QConst, QConst *)
    apply andb_prop in H. destruct H as [Hz Hp].
    destruct q as [qn qd], q0 as [qn0 qd0]. simpl in Hz, Hp.
    apply (proj1 (Z.eqb_eq _ _)) in Hz.
    apply (proj1 (Pos.eqb_eq _ _)) in Hp.
    subst. reflexivity.
  - (* Var, Var *)
    apply (proj1 (Nat.eqb_eq _ _)) in H. subst. reflexivity.
  - (* BinOp, BinOp *)
    destruct (IROpCode_eq_dec i i0) as [Heq|Hne]; [|discriminate].
    subst. apply andb_prop in H. destruct H as [H1 H2].
    rewrite (IH _ _ H1), (IH _ _ H2). reflexivity.
  - (* UnOp, UnOp *)
    destruct (IROpCode_eq_dec i i0) as [Heq|Hne]; [|discriminate].
    subst. rewrite (IH _ _ H). reflexivity.
  - (* IfExpr, IfExpr *)
    apply andb_prop in H. destruct H as [H12 H3].
    apply andb_prop in H12. destruct H12 as [H1 H2].
    rewrite (IH _ _ H1), (IH _ _ H2), (IH _ _ H3). reflexivity.
  - (* WhileExpr, WhileExpr *)
    apply andb_prop in H. destruct H as [H1 H2].
    rewrite (IH _ _ H1), (IH _ _ H2). reflexivity.
  - (* Block, Block *)
    f_equal. revert l0 H. induction l as [|x xs IHl]; intros [|y ys] H;
    simpl in H; try discriminate.
    + reflexivity.
    + apply andb_prop in H. destruct H as [Hx Hxs].
      f_equal; [exact (IH _ _ Hx) | exact (IHl _ Hxs)].
  - (* MatchExpr, MatchExpr *)
    apply andb_prop in H. destruct H as [Hs Hcs].
    f_equal; [exact (IH _ _ Hs)|].
    revert l0 Hcs. induction l as [|[a1 b1] xs IHl]; intros [|[a2 b2] ys] Hcs;
    simpl in Hcs; try discriminate.
    + reflexivity.
    + apply andb_prop in Hcs. destruct Hcs as [H12 Hxs].
      apply andb_prop in H12. destruct H12 as [Ha Hb].
      f_equal; [f_equal; [exact (IH _ _ Ha) | exact (IH _ _ Hb)] | exact (IHl _ Hxs)].
Qed.

Lemma cse_compatible_set : forall s rho e x,
  cse_compatible s rho ->
  eval_expr e rho = Some (rho x) ->
  cse_compatible (cse_set s e x) rho.
Proof.
  unfold cse_compatible. intros s rho e x Hcomp Heval e' y Hlookup.
  simpl in Hlookup.
  destruct (expr_eqb e e') eqn:Heq.
  - inversion Hlookup. subst.
    apply expr_eqb_sound in Heq. subst. exact Heval.
  - apply Hcomp. exact Hlookup.
Qed.
