(* FILE: proof/core/CatnipExprModel.v *)
(* CatnipExprModel.v - Shared expression model for optimization proofs
 *
 * Defines the Expr type, environment, evaluation, structural equality,
 * and lowering to IRPure. Used by:
 *   CatnipOptimProof.v      (strength reduction, blunt code, DCE, flattening)
 *   CatnipConstFoldProof.v  (constant folding)
 *   CatnipStorePropProof.v  (store model, const/copy propagation, CSE)
 *
 * Depends on: CatnipIR.v (IROpCode, IRPure, ir_op, ir_binop)
 *)

From Coq Require Import List ZArith Bool QArith.
From Catnip Require Import CatnipIR.
Import ListNotations.


(* ================================================================ *)
(* A. Expression Model                                               *)
(*                                                                    *)
(* Simplified expression tree for optimization reasoning.             *)
(* Z for integers, Q for rationals (TrueDiv results).                *)
(* ================================================================ *)

Inductive Expr :=
  | Const     : Z -> Expr
  | BConst    : bool -> Expr
  | QConst    : Q -> Expr
  | Var       : nat -> Expr
  | BinOp     : IROpCode -> Expr -> Expr -> Expr
  | UnOp      : IROpCode -> Expr -> Expr
  | IfExpr    : Expr -> Expr -> Expr -> Expr
  | WhileExpr : Expr -> Expr -> Expr
  | Block     : list Expr -> Expr
  | MatchExpr : Expr -> list (Expr * Expr) -> Expr.

Definition env := nat -> Z.

Fixpoint eval_expr (e : Expr) (rho : env) : option Z :=
  match e with
  | Const n => Some n
  | BConst true => Some 1%Z
  | BConst false => Some 0%Z
  | Var x => Some (rho x)
  | BinOp Add l r =>
      match eval_expr l rho, eval_expr r rho with
      | Some a, Some b => Some (a + b)%Z
      | _, _ => None
      end
  | BinOp Sub l r =>
      match eval_expr l rho, eval_expr r rho with
      | Some a, Some b => Some (a - b)%Z
      | _, _ => None
      end
  | BinOp Mul l r =>
      match eval_expr l rho, eval_expr r rho with
      | Some a, Some b => Some (a * b)%Z
      | _, _ => None
      end
  | BinOp And l r =>
      match eval_expr l rho, eval_expr r rho with
      | Some a, Some b => Some (if Z.eqb a 0 then 0 else b)%Z
      | _, _ => None
      end
  | BinOp Or l r =>
      match eval_expr l rho, eval_expr r rho with
      | Some a, Some b => Some (if Z.eqb a 0 then b else a)%Z
      | _, _ => None
      end
  | UnOp Not e' =>
      match eval_expr e' rho with
      | Some v => Some (if Z.eqb v 0 then 1 else 0)%Z
      | None => None
      end
  | UnOp Neg e' =>
      match eval_expr e' rho with
      | Some v => Some (- v)%Z
      | None => None
      end
  | IfExpr c t f =>
      match eval_expr c rho with
      | Some v => if Z.eqb v 0 then eval_expr f rho
                  else eval_expr t rho
      | None => None
      end
  | Block [] => Some 0%Z
  | Block [e'] => eval_expr e' rho
  | _ => None
  end.

Definition eval_bool (e : Expr) (rho : env) : option bool :=
  match eval_expr e rho with
  | Some v => Some (negb (Z.eqb v 0))
  | None => None
  end.


(* ================================================================ *)
(* B. Structural Equality                                            *)
(*                                                                    *)
(* Decidable structural equality on Expr. Used by blunt code         *)
(* (idempotence, complement) and CSE (expression matching).          *)
(* ================================================================ *)

Fixpoint expr_eqb (a b : Expr) : bool :=
  let fix list_eqb (l1 l2 : list Expr) : bool :=
    match l1, l2 with
    | [], [] => true
    | x :: xs, y :: ys => expr_eqb x y && list_eqb xs ys
    | _, _ => false
    end
  in
  let fix pair_list_eqb (l1 l2 : list (Expr * Expr)) : bool :=
    match l1, l2 with
    | [], [] => true
    | (a1, b1) :: xs, (a2, b2) :: ys =>
        expr_eqb a1 a2 && expr_eqb b1 b2 && pair_list_eqb xs ys
    | _, _ => false
    end
  in
  match a, b with
  | Const x, Const y => Z.eqb x y
  | BConst x, BConst y => Bool.eqb x y
  | QConst x, QConst y => Z.eqb (Qnum x) (Qnum y) && Pos.eqb (Qden x) (Qden y)
  | Var x, Var y => Nat.eqb x y
  | BinOp o1 l1 r1, BinOp o2 l2 r2 =>
      if IROpCode_eq_dec o1 o2 then expr_eqb l1 l2 && expr_eqb r1 r2
      else false
  | UnOp o1 e1, UnOp o2 e2 =>
      if IROpCode_eq_dec o1 o2 then expr_eqb e1 e2
      else false
  | IfExpr c1 t1 f1, IfExpr c2 t2 f2 =>
      expr_eqb c1 c2 && expr_eqb t1 t2 && expr_eqb f1 f2
  | WhileExpr c1 b1, WhileExpr c2 b2 =>
      expr_eqb c1 c2 && expr_eqb b1 b2
  | Block l1, Block l2 => list_eqb l1 l2
  | MatchExpr s1 cs1, MatchExpr s2 cs2 =>
      expr_eqb s1 s2 && pair_list_eqb cs1 cs2
  | _, _ => false
  end.

Lemma expr_eqb_refl : forall e, expr_eqb e e = true.
Proof.
  fix IH 1. intro e.
  destruct e; simpl; try reflexivity.
  - apply Z.eqb_refl.
  - destruct b; reflexivity.
  - rewrite Z.eqb_refl, Pos.eqb_refl. reflexivity.
  - apply Nat.eqb_refl.
  - destruct (IROpCode_eq_dec i i) as [_|F];
    [rewrite (IH e1), (IH e2); reflexivity | exfalso; apply F; reflexivity].
  - destruct (IROpCode_eq_dec i i) as [_|F];
    [apply IH | exfalso; apply F; reflexivity].
  - rewrite (IH e1), (IH e2), (IH e3). reflexivity.
  - rewrite (IH e1), (IH e2). reflexivity.
  - induction l as [|x xs IHl]; simpl; [reflexivity|].
    rewrite (IH x). simpl. exact IHl.
  - rewrite (IH e). simpl.
    induction l as [|[a b] xs IHl]; simpl; [reflexivity|].
    rewrite (IH a), (IH b). simpl. exact IHl.
Qed.

(* x is never structurally equal to UnOp Not x *)
Lemma expr_eqb_not_self : forall x, expr_eqb x (UnOp Not x) = false.
Proof.
  induction x; simpl; try reflexivity.
  destruct (IROpCode_eq_dec i Not) as [->|Hne].
  - exact IHx.
  - reflexivity.
Qed.

(* UnOp op x is never structurally equal to x (strictly larger) *)
Lemma expr_eqb_unop_self : forall op x, expr_eqb (UnOp op x) x = false.
Proof.
  intros op x; revert op; induction x; intro op; simpl; try reflexivity.
  destruct (IROpCode_eq_dec op i) as [->|]; [apply IHx | reflexivity].
Qed.

Definition is_not (e : Expr) : option Expr :=
  match e with
  | UnOp Not inner => Some inner
  | _ => None
  end.


(* ================================================================ *)
(* C. Lowering to IRPure                                             *)
(*                                                                    *)
(* Connection between Expr model and CatnipIR.IRPure.                 *)
(* ================================================================ *)

Fixpoint expr_to_ir (e : Expr) : IRPure :=
  match e with
  | Const n => IRInt n
  | BConst b => IRBool b
  | QConst q => IRFloat (Qnum q)
  | Var _ => IRNone
  | BinOp op l r => ir_binop op (expr_to_ir l) (expr_to_ir r)
  | UnOp op x => ir_op op [expr_to_ir x]
  | IfExpr c t f => ir_op OpIf [expr_to_ir c; expr_to_ir t; expr_to_ir f]
  | WhileExpr c body => ir_op OpWhile [expr_to_ir c; expr_to_ir body]
  | Block stmts => ir_op OpBlock (map expr_to_ir stmts)
  | MatchExpr scrut cases =>
      ir_op OpMatch (expr_to_ir scrut ::
        map (fun '(p, b) => ir_op Nop [expr_to_ir p; expr_to_ir b]) cases)
  end.
