(* FILE: proof/optim/CatnipConstFoldProof.v *)
(* CatnipConstFoldProof.v - Correctness of the constant folding pass
 *
 * Source of truth:
 *   catnip_rs/src/semantic/constant_folding.rs
 *
 * Scope: pure algebraic constant folding. Expressions whose operands
 * are all concrete literals are replaced by their computed value.
 * Division by zero is never folded - preserved for runtime error.
 *
 * TrueDiv folds to QConst (rational). Pow folds to Const when
 * exponent >= 0, left unfolded otherwise (negative exponent yields
 * float in Python, not modeled).
 *
 * Not modeled:
 *   - Stateful passes (constant propagation, CSE, DSE)
 *   - CFG/SSA passes
 *
 * Depends on:
 *   CatnipIR.v        (IROpCode, IRPure, ir_op, ir_binop)
 *   CatnipExprModel.v (Expr, QConst, env, eval_expr, eval_bool, expr_to_ir)
 *)

From Coq Require Import List ZArith Bool Lia QArith.
From Catnip Require Import CatnipIR.
From Catnip Require Import CatnipExprModel.
Import ListNotations.


(* ================================================================ *)
(* B. Constant Folding Helpers                                       *)
(*                                                                    *)
(* One-step evaluation of an op on concrete literals.                *)
(* Returns None for unsupported ops or division by zero.             *)
(* ================================================================ *)

(* Binary op on two integer constants. *)
Definition cf_eval_binop (op : IROpCode) (a b : Z) : option Expr :=
  match op with
  | Add      => Some (Const (a + b)%Z)
  | Sub      => Some (Const (a - b)%Z)
  | Mul      => Some (Const (a * b)%Z)
  | TrueDiv  => if Z.eqb b 0 then None
                 else Some (QConst (inject_Z a / inject_Z b)%Q)
  | FloorDiv => if Z.eqb b 0 then None else Some (Const (Z.div a b))
  | Mod      => if Z.eqb b 0 then None else Some (Const (Z.modulo a b))
  | Pow      => if Z.ltb b 0 then None else Some (Const (Z.pow a b))
  | Eq       => Some (BConst (Z.eqb a b))
  | Ne       => Some (BConst (negb (Z.eqb a b)))
  | Lt       => Some (BConst (Z.ltb a b))
  | Le       => Some (BConst (Z.leb a b))
  | Gt       => Some (BConst (Z.ltb b a))
  | Ge       => Some (BConst (Z.leb b a))
  | BAnd     => Some (Const (Z.land a b))
  | BOr      => Some (Const (Z.lor a b))
  | BXor     => Some (Const (Z.lxor a b))
  | LShift   => Some (Const (Z.shiftl a b))
  | RShift   => Some (Const (Z.shiftr a b))
  | _        => None
  end.

(* Binary op on two boolean constants. *)
Definition cf_eval_bool_binop (op : IROpCode) (a b : bool) : option Expr :=
  match op with
  | And => Some (BConst (a && b))
  | Or  => Some (BConst (a || b))
  | _   => None
  end.

(* Unary op on an integer constant. *)
Definition cf_eval_unop_int (op : IROpCode) (a : Z) : option Expr :=
  match op with
  | Neg  => Some (Const (- a)%Z)
  | Pos  => Some (Const a)
  | BNot => Some (Const (Z.lnot a))
  | _    => None
  end.

(* Unary op on a boolean constant. *)
Definition cf_eval_unop_bool (op : IROpCode) (a : bool) : option Expr :=
  match op with
  | Not => Some (BConst (negb a))
  | _   => None
  end.


(* ================================================================ *)
(* C. Constant Folding Pass                                          *)
(*                                                                    *)
(* Bottom-up: fold children first, then fold the root if all         *)
(* children reduced to literals.                                      *)
(* ================================================================ *)

Fixpoint cf_fold (e : Expr) : Expr :=
  match e with
  | Const _ | BConst _ | QConst _ | Var _ => e
  | BinOp op l r =>
      let l' := cf_fold l in
      let r' := cf_fold r in
      let folded :=
        match l' with
        | Const a =>
            match r' with
            | Const b => cf_eval_binop op a b
            | _       => None
            end
        | BConst a =>
            match r' with
            | BConst b => cf_eval_bool_binop op a b
            | _        => None
            end
        | _ => None
        end
      in
      match folded with
      | Some e' => e'
      | None    => BinOp op l' r'
      end
  | UnOp op x =>
      let x' := cf_fold x in
      let folded :=
        match x' with
        | Const  a => cf_eval_unop_int  op a
        | BConst a => cf_eval_unop_bool op a
        | _        => None
        end
      in
      match folded with
      | Some e' => e'
      | None    => UnOp op x'
      end
  | IfExpr c t f     => IfExpr (cf_fold c) (cf_fold t) (cf_fold f)
  | WhileExpr c body => WhileExpr (cf_fold c) (cf_fold body)
  | Block stmts      => Block (map cf_fold stmts)
  | MatchExpr scrut cases =>
      MatchExpr (cf_fold scrut)
                (map (fun '(p, b) => (cf_fold p, cf_fold b)) cases)
  end.


(* ================================================================ *)
(* D. Syntactic Fold Theorems                                        *)
(*                                                                    *)
(* For each supported op, cf_fold on literal operands returns the    *)
(* computed literal. Proofs are reflexivity (or case split for        *)
(* division-by-zero guard).                                           *)
(* ================================================================ *)

(* --- Literals pass through unchanged --- *)

Theorem cf_const_id : forall n, cf_fold (Const n) = Const n.
Proof. reflexivity. Qed.

Theorem cf_bconst_id : forall b, cf_fold (BConst b) = BConst b.
Proof. reflexivity. Qed.

Theorem cf_qconst_id : forall q, cf_fold (QConst q) = QConst q.
Proof. reflexivity. Qed.

Theorem cf_var_id : forall x, cf_fold (Var x) = Var x.
Proof. reflexivity. Qed.

(* --- Arithmetic --- *)

Theorem cf_add_fold : forall a b,
  cf_fold (BinOp Add (Const a) (Const b)) = Const (a + b)%Z.
Proof. reflexivity. Qed.

Theorem cf_sub_fold : forall a b,
  cf_fold (BinOp Sub (Const a) (Const b)) = Const (a - b)%Z.
Proof. reflexivity. Qed.

Theorem cf_mul_fold : forall a b,
  cf_fold (BinOp Mul (Const a) (Const b)) = Const (a * b)%Z.
Proof. reflexivity. Qed.

Theorem cf_floordiv_fold : forall a b,
  b <> 0%Z ->
  cf_fold (BinOp FloorDiv (Const a) (Const b)) = Const (Z.div a b).
Proof.
  intros a b Hb. simpl.
  destruct (Z.eqb_spec b 0) as [H | H].
  - exfalso. apply Hb. exact H.
  - reflexivity.
Qed.

Theorem cf_mod_fold : forall a b,
  b <> 0%Z ->
  cf_fold (BinOp Mod (Const a) (Const b)) = Const (Z.modulo a b).
Proof.
  intros a b Hb. simpl.
  destruct (Z.eqb_spec b 0) as [H | H].
  - exfalso. apply Hb. exact H.
  - reflexivity.
Qed.

Theorem cf_neg_fold : forall a,
  cf_fold (UnOp Neg (Const a)) = Const (- a)%Z.
Proof. reflexivity. Qed.

Theorem cf_pos_fold : forall a,
  cf_fold (UnOp Pos (Const a)) = Const a.
Proof. reflexivity. Qed.

Theorem cf_truediv_fold : forall a b,
  b <> 0%Z ->
  cf_fold (BinOp TrueDiv (Const a) (Const b)) =
  QConst (inject_Z a / inject_Z b)%Q.
Proof.
  intros a b Hb. simpl.
  destruct (Z.eqb_spec b 0) as [H | H].
  - exfalso. apply Hb. exact H.
  - reflexivity.
Qed.

Theorem cf_pow_fold : forall a b,
  (0 <= b)%Z ->
  cf_fold (BinOp Pow (Const a) (Const b)) = Const (Z.pow a b).
Proof.
  intros a b Hb. simpl.
  destruct (Z.ltb_spec b 0) as [H | H].
  - lia.
  - reflexivity.
Qed.

(* --- Comparisons --- *)

Theorem cf_eq_fold : forall a b,
  cf_fold (BinOp Eq (Const a) (Const b)) = BConst (Z.eqb a b).
Proof. reflexivity. Qed.

Theorem cf_ne_fold : forall a b,
  cf_fold (BinOp Ne (Const a) (Const b)) = BConst (negb (Z.eqb a b)).
Proof. reflexivity. Qed.

Theorem cf_lt_fold : forall a b,
  cf_fold (BinOp Lt (Const a) (Const b)) = BConst (Z.ltb a b).
Proof. reflexivity. Qed.

Theorem cf_le_fold : forall a b,
  cf_fold (BinOp Le (Const a) (Const b)) = BConst (Z.leb a b).
Proof. reflexivity. Qed.

Theorem cf_gt_fold : forall a b,
  cf_fold (BinOp Gt (Const a) (Const b)) = BConst (Z.ltb b a).
Proof. reflexivity. Qed.

Theorem cf_ge_fold : forall a b,
  cf_fold (BinOp Ge (Const a) (Const b)) = BConst (Z.leb b a).
Proof. reflexivity. Qed.

(* --- Logical --- *)

Theorem cf_and_bool_fold : forall a b,
  cf_fold (BinOp And (BConst a) (BConst b)) = BConst (a && b).
Proof. reflexivity. Qed.

Theorem cf_or_bool_fold : forall a b,
  cf_fold (BinOp Or (BConst a) (BConst b)) = BConst (a || b).
Proof. reflexivity. Qed.

Theorem cf_not_fold : forall a,
  cf_fold (UnOp Not (BConst a)) = BConst (negb a).
Proof. reflexivity. Qed.

(* --- Bitwise --- *)

Theorem cf_band_fold : forall a b,
  cf_fold (BinOp BAnd (Const a) (Const b)) = Const (Z.land a b).
Proof. reflexivity. Qed.

Theorem cf_bor_fold : forall a b,
  cf_fold (BinOp BOr (Const a) (Const b)) = Const (Z.lor a b).
Proof. reflexivity. Qed.

Theorem cf_bxor_fold : forall a b,
  cf_fold (BinOp BXor (Const a) (Const b)) = Const (Z.lxor a b).
Proof. reflexivity. Qed.

Theorem cf_bnot_fold : forall a,
  cf_fold (UnOp BNot (Const a)) = Const (Z.lnot a).
Proof. reflexivity. Qed.

Theorem cf_lshift_fold : forall a b,
  cf_fold (BinOp LShift (Const a) (Const b)) = Const (Z.shiftl a b).
Proof. reflexivity. Qed.

Theorem cf_rshift_fold : forall a b,
  cf_fold (BinOp RShift (Const a) (Const b)) = Const (Z.shiftr a b).
Proof. reflexivity. Qed.


(* ================================================================ *)
(* E. Semantic Preservation                                          *)
(*                                                                    *)
(* eval_expr (cf_fold e) rho = eval_expr e rho for ground constant   *)
(* expressions, restricted to ops covered by eval_expr:              *)
(*   Add, Sub, Mul (integer), And, Or (boolean), Not, Neg (unary).  *)
(*                                                                    *)
(* Comparisons (Eq, Ne, Lt, …) and bitwise ops return None in        *)
(* eval_expr regardless of operands; their correctness is the        *)
(* syntactic fold established in Section D.                           *)
(* ================================================================ *)

Theorem cf_add_fold_sem : forall a b rho,
  eval_expr (cf_fold (BinOp Add (Const a) (Const b))) rho =
  eval_expr (BinOp Add (Const a) (Const b)) rho.
Proof. reflexivity. Qed.

Theorem cf_sub_fold_sem : forall a b rho,
  eval_expr (cf_fold (BinOp Sub (Const a) (Const b))) rho =
  eval_expr (BinOp Sub (Const a) (Const b)) rho.
Proof. reflexivity. Qed.

Theorem cf_mul_fold_sem : forall a b rho,
  eval_expr (cf_fold (BinOp Mul (Const a) (Const b))) rho =
  eval_expr (BinOp Mul (Const a) (Const b)) rho.
Proof. reflexivity. Qed.

Theorem cf_neg_fold_sem : forall a rho,
  eval_expr (cf_fold (UnOp Neg (Const a))) rho =
  eval_expr (UnOp Neg (Const a)) rho.
Proof. reflexivity. Qed.

(* not True → False (0), not False → True (1) *)
Theorem cf_not_bool_fold_sem : forall a rho,
  eval_expr (cf_fold (UnOp Not (BConst a))) rho =
  eval_expr (UnOp Not (BConst a)) rho.
Proof. destruct a; reflexivity. Qed.

(* Boolean and/or: Z encoding (0 = false, nonzero = true). *)
Theorem cf_and_bool_fold_sem : forall a b rho,
  eval_expr (cf_fold (BinOp And (BConst a) (BConst b))) rho =
  eval_expr (BinOp And (BConst a) (BConst b)) rho.
Proof. destruct a; destruct b; reflexivity. Qed.

Theorem cf_or_bool_fold_sem : forall a b rho,
  eval_expr (cf_fold (BinOp Or (BConst a) (BConst b))) rho =
  eval_expr (BinOp Or (BConst a) (BConst b)) rho.
Proof. destruct a; destruct b; reflexivity. Qed.


(* ================================================================ *)
(* F. Lowering to IRPure                                             *)
(*                                                                    *)
(* cf_fold on ground expressions produces the matching IRPure literal *)
(* via expr_to_ir.                                                    *)
(* ================================================================ *)

Theorem cf_add_fold_ir : forall a b,
  expr_to_ir (cf_fold (BinOp Add (Const a) (Const b))) = IRInt (a + b)%Z.
Proof. reflexivity. Qed.

Theorem cf_sub_fold_ir : forall a b,
  expr_to_ir (cf_fold (BinOp Sub (Const a) (Const b))) = IRInt (a - b)%Z.
Proof. reflexivity. Qed.

Theorem cf_mul_fold_ir : forall a b,
  expr_to_ir (cf_fold (BinOp Mul (Const a) (Const b))) = IRInt (a * b)%Z.
Proof. reflexivity. Qed.

Theorem cf_neg_fold_ir : forall a,
  expr_to_ir (cf_fold (UnOp Neg (Const a))) = IRInt (- a)%Z.
Proof. reflexivity. Qed.

Theorem cf_eq_fold_ir : forall a b,
  expr_to_ir (cf_fold (BinOp Eq (Const a) (Const b))) = IRBool (Z.eqb a b).
Proof. reflexivity. Qed.

Theorem cf_lt_fold_ir : forall a b,
  expr_to_ir (cf_fold (BinOp Lt (Const a) (Const b))) = IRBool (Z.ltb a b).
Proof. reflexivity. Qed.

Theorem cf_gt_fold_ir : forall a b,
  expr_to_ir (cf_fold (BinOp Gt (Const a) (Const b))) = IRBool (Z.ltb b a).
Proof. reflexivity. Qed.

Theorem cf_not_fold_ir : forall a,
  expr_to_ir (cf_fold (UnOp Not (BConst a))) = IRBool (negb a).
Proof. reflexivity. Qed.

Theorem cf_band_fold_ir : forall a b,
  expr_to_ir (cf_fold (BinOp BAnd (Const a) (Const b))) = IRInt (Z.land a b).
Proof. reflexivity. Qed.

Theorem cf_truediv_fold_ir : forall a b,
  b <> 0%Z ->
  expr_to_ir (cf_fold (BinOp TrueDiv (Const a) (Const b))) =
  IRFloat (Qnum (inject_Z a / inject_Z b)%Q).
Proof.
  intros a b Hb. simpl.
  destruct (Z.eqb_spec b 0) as [H | H].
  - exfalso. apply Hb. exact H.
  - reflexivity.
Qed.

Theorem cf_pow_fold_ir : forall a b,
  (0 <= b)%Z ->
  expr_to_ir (cf_fold (BinOp Pow (Const a) (Const b))) = IRInt (Z.pow a b).
Proof.
  intros a b Hb. simpl.
  destruct (Z.ltb_spec b 0) as [H | H].
  - lia.
  - reflexivity.
Qed.
