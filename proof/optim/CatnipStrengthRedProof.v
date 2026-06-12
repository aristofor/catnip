(* FILE: proof/optim/CatnipStrengthRedProof.v *)
(* Strength Reduction pass - boolean literal simplification.
 *
 * Source: catnip_core/src/semantic/passes/strength_reduction.rs
 *
 * The live pass only simplifies And/Or when BOTH operands are boolean
 * literals. The arithmetic identities of the former pass (x * 1, x * 0,
 * x ** 2 -> x * x, x + 0, x - 0, x / 1, x // 1, x ** 1, x ** 0) and the
 * one-sided boolean rules (x and True -> x) were removed from the code
 * (review 2026-06-10): they hold on this integer model but not on the
 * language's values ("abc" * 0 is "", 7.5 // 1 is 7.0, `**` and `*`
 * dispatch to distinct overloads, x and True changes the return type
 * when x is not a bool). The *_untouched theorems pin that absence.
 *
 * Proves:
 *   - sr_and_bools, sr_or_bools (the two live rewrites)
 *   - *_untouched guards: removed identities never fire
 *   - strength_reduce_sound: preserves eval_expr exactly (stronger than
 *     the former one-sided rules, which only preserved eval_bool)
 *   - strength_reduce_bool_sound (corollary)
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
(* B. Strength Reduction                                             *)
(*                                                                    *)
(* Mirror of reduce_binary in strength_reduction.rs: And/Or with two  *)
(* boolean literals. The Rust code returns left/right clones case by  *)
(* case; on two literals this is extensionally BConst (andb/orb a b). *)
(* ================================================================ *)

Definition strength_reduce (e : Expr) : Expr :=
  match e with
  | BinOp And (BConst a) (BConst b) => BConst (andb a b)
  | BinOp Or (BConst a) (BConst b) => BConst (orb a b)
  | _ => e
  end.

(* --- The two live rewrites --- *)

Theorem sr_and_bools : forall a b,
  strength_reduce (BinOp And (BConst a) (BConst b)) = BConst (andb a b).
Proof. reflexivity. Qed.

Theorem sr_or_bools : forall a b,
  strength_reduce (BinOp Or (BConst a) (BConst b)) = BConst (orb a b).
Proof. reflexivity. Qed.

(* --- Guards: removed identities never fire --- *)

Theorem sr_mul_one_untouched : forall x,
  strength_reduce (BinOp Mul x (Const 1)) = BinOp Mul x (Const 1).
Proof. reflexivity. Qed.

Theorem sr_mul_zero_untouched : forall x,
  strength_reduce (BinOp Mul x (Const 0)) = BinOp Mul x (Const 0).
Proof. reflexivity. Qed.

Theorem sr_pow_two_untouched : forall x,
  strength_reduce (BinOp Pow x (Const 2)) = BinOp Pow x (Const 2).
Proof. reflexivity. Qed.

Theorem sr_pow_one_untouched : forall x,
  strength_reduce (BinOp Pow x (Const 1)) = BinOp Pow x (Const 1).
Proof. reflexivity. Qed.

Theorem sr_pow_zero_untouched : forall x,
  strength_reduce (BinOp Pow x (Const 0)) = BinOp Pow x (Const 0).
Proof. reflexivity. Qed.

Theorem sr_add_zero_untouched : forall x,
  strength_reduce (BinOp Add x (Const 0)) = BinOp Add x (Const 0).
Proof. reflexivity. Qed.

Theorem sr_sub_zero_untouched : forall x,
  strength_reduce (BinOp Sub x (Const 0)) = BinOp Sub x (Const 0).
Proof. reflexivity. Qed.

Theorem sr_truediv_one_untouched : forall x,
  strength_reduce (BinOp TrueDiv x (Const 1)) = BinOp TrueDiv x (Const 1).
Proof. reflexivity. Qed.

Theorem sr_floordiv_one_untouched : forall x,
  strength_reduce (BinOp FloorDiv x (Const 1)) = BinOp FloorDiv x (Const 1).
Proof. reflexivity. Qed.

(* One-sided boolean rules: x and True must NOT rewrite to x when x is
   not itself a boolean literal (the rewrite would change the return
   type: `5 and True` is True in Catnip, not 5). *)
Theorem sr_and_one_sided_untouched : forall x b,
  (forall c, x <> BConst c) ->
  strength_reduce (BinOp And x (BConst b)) = BinOp And x (BConst b).
Proof.
  intros x b Hx; destruct x; simpl; try reflexivity.
  exfalso; apply (Hx b0); reflexivity.
Qed.

Theorem sr_or_one_sided_untouched : forall x b,
  (forall c, x <> BConst c) ->
  strength_reduce (BinOp Or x (BConst b)) = BinOp Or x (BConst b).
Proof.
  intros x b Hx; destruct x; simpl; try reflexivity.
  exfalso; apply (Hx b0); reflexivity.
Qed.

(* --- Semantic soundness --- *)

(* On two boolean literals the rewrite preserves eval_expr exactly:
   the result value (0/1) is identical, not merely the truthiness. *)
Theorem strength_reduce_sound : forall e rho,
  eval_expr (strength_reduce e) rho = eval_expr e rho.
Proof.
  intros e rho.
  destruct e as [| | | |op l r| | | | |]; try reflexivity.
  destruct op; try reflexivity.
  - (* And *)
    destruct l; try reflexivity.
    destruct r; try reflexivity.
    destruct b, b0; reflexivity.
  - (* Or *)
    destruct l; try reflexivity.
    destruct r; try reflexivity.
    destruct b, b0; reflexivity.
Qed.

Theorem strength_reduce_bool_sound : forall e rho bv,
  eval_bool e rho = Some bv ->
  eval_bool (strength_reduce e) rho = Some bv.
Proof.
  intros e rho bv H. unfold eval_bool in *.
  rewrite strength_reduce_sound. exact H.
Qed.
