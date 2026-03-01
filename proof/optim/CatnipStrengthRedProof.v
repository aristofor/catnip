(* FILE: proof/optim/CatnipStrengthRedProof.v *)
(* Strength Reduction pass — algebraic identities.
 *
 * Source: catnip_rs/src/semantic/strength_reduction.rs
 *
 * Proves:
 *   - 20 algebraic identity rewrites (sr_mul_one_r, ..., sr_or_true_l)
 *   - Arithmetic semantic correctness (sr_mul_one_r_sem, etc.)
 *   - Boolean semantic correctness (sr_and_true_r_sem, etc.)
 *   - strength_reduce preserves eval_bool (strength_reduce_bool_sound)
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
(* 20 algebraic identities from strength_reduction.rs.                *)
(* Nested matches to avoid Coq pattern overlap issues.                *)
(* ================================================================ *)

Definition strength_reduce (e : Expr) : Expr :=
  match e with
  | BinOp op e1 e2 =>
      match op with
      | Mul =>
          match e2 with
          | Const 1 => e1
          | Const 0 => Const 0
          | _ => match e1 with
                 | Const 1 => e2
                 | Const 0 => Const 0
                 | _ => e
                 end
          end
      | Pow =>
          match e2 with
          | Const 2 => BinOp Mul e1 e1
          | Const 1 => e1
          | Const 0 => Const 1
          | _ => e
          end
      | Add =>
          match e2 with
          | Const 0 => e1
          | _ => match e1 with
                 | Const 0 => e2
                 | _ => e
                 end
          end
      | Sub =>
          match e2 with
          | Const 0 => e1
          | _ => e
          end
      | TrueDiv =>
          match e2 with
          | Const 1 => e1
          | _ => e
          end
      | FloorDiv =>
          match e2 with
          | Const 1 => e1
          | _ => e
          end
      | And =>
          match e2 with
          | BConst true => e1
          | BConst false => BConst false
          | _ => match e1 with
                 | BConst true => e2
                 | BConst false => BConst false
                 | _ => e
                 end
          end
      | Or =>
          match e2 with
          | BConst false => e1
          | BConst true => BConst true
          | _ => match e1 with
                 | BConst false => e2
                 | BConst true => BConst true
                 | _ => e
                 end
          end
      | _ => e
      end
  | _ => e
  end.

(* --- Individual correctness theorems --- *)

Theorem sr_mul_one_r : forall x, strength_reduce (BinOp Mul x (Const 1)) = x.
Proof. reflexivity. Qed.

Theorem sr_mul_one_l : forall x, strength_reduce (BinOp Mul (Const 1) x) = x.
Proof.
  intro x; destruct x; simpl; try reflexivity;
  try (destruct z; try reflexivity; destruct p; reflexivity);
  try (destruct b; reflexivity).
Qed.

Theorem sr_mul_zero_r : forall x, strength_reduce (BinOp Mul x (Const 0)) = Const 0.
Proof. reflexivity. Qed.

Theorem sr_mul_zero_l : forall x, strength_reduce (BinOp Mul (Const 0) x) = Const 0.
Proof.
  intro x; destruct x; simpl; try reflexivity;
  try (destruct z; try reflexivity; destruct p; reflexivity);
  try (destruct b; reflexivity).
Qed.

Theorem sr_pow_two : forall x, strength_reduce (BinOp Pow x (Const 2)) = BinOp Mul x x.
Proof. reflexivity. Qed.

Theorem sr_pow_one : forall x, strength_reduce (BinOp Pow x (Const 1)) = x.
Proof. reflexivity. Qed.

Theorem sr_pow_zero : forall x, strength_reduce (BinOp Pow x (Const 0)) = Const 1.
Proof. reflexivity. Qed.

Theorem sr_add_zero_r : forall x, strength_reduce (BinOp Add x (Const 0)) = x.
Proof. reflexivity. Qed.

Theorem sr_add_zero_l : forall x, strength_reduce (BinOp Add (Const 0) x) = x.
Proof.
  intro x; destruct x; simpl; try reflexivity;
  try (destruct z; try reflexivity; destruct p; reflexivity);
  try (destruct b; reflexivity).
Qed.

Theorem sr_sub_zero : forall x, strength_reduce (BinOp Sub x (Const 0)) = x.
Proof. reflexivity. Qed.

Theorem sr_truediv_one : forall x, strength_reduce (BinOp TrueDiv x (Const 1)) = x.
Proof. reflexivity. Qed.

Theorem sr_floordiv_one : forall x, strength_reduce (BinOp FloorDiv x (Const 1)) = x.
Proof. reflexivity. Qed.

Theorem sr_and_true_r : forall x, strength_reduce (BinOp And x (BConst true)) = x.
Proof. reflexivity. Qed.

Theorem sr_and_true_l : forall x, strength_reduce (BinOp And (BConst true) x) = x.
Proof.
  intro x; destruct x; simpl; try reflexivity;
  try (destruct b; reflexivity).
Qed.

Theorem sr_and_false_r : forall x, strength_reduce (BinOp And x (BConst false)) = BConst false.
Proof. reflexivity. Qed.

Theorem sr_and_false_l : forall x, strength_reduce (BinOp And (BConst false) x) = BConst false.
Proof.
  intro x; destruct x; simpl; try reflexivity;
  try (destruct b; reflexivity).
Qed.

Theorem sr_or_false_r : forall x, strength_reduce (BinOp Or x (BConst false)) = x.
Proof. reflexivity. Qed.

Theorem sr_or_false_l : forall x, strength_reduce (BinOp Or (BConst false) x) = x.
Proof.
  intro x; destruct x; simpl; try reflexivity;
  try (destruct b; reflexivity).
Qed.

Theorem sr_or_true_r : forall x, strength_reduce (BinOp Or x (BConst true)) = BConst true.
Proof. reflexivity. Qed.

Theorem sr_or_true_l : forall x, strength_reduce (BinOp Or (BConst true) x) = BConst true.
Proof.
  intro x; destruct x; simpl; try reflexivity;
  try (destruct b; reflexivity).
Qed.

(* Semantic correctness for arithmetic identities *)
Theorem sr_mul_one_r_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (BinOp Mul x (Const 1)) rho = Some v.
Proof. intros x rho v Hx. simpl. rewrite Hx. f_equal. lia. Qed.

Theorem sr_mul_zero_r_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (BinOp Mul x (Const 0)) rho = Some 0%Z.
Proof. intros x rho v Hx. simpl. rewrite Hx. f_equal. lia. Qed.

Theorem sr_add_zero_r_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (BinOp Add x (Const 0)) rho = Some v.
Proof. intros x rho v Hx. simpl. rewrite Hx. f_equal. lia. Qed.

Theorem sr_sub_zero_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (BinOp Sub x (Const 0)) rho = Some v.
Proof. intros x rho v Hx. simpl. rewrite Hx. f_equal. lia. Qed.

(* Commutative variants *)
Theorem sr_mul_one_l_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (BinOp Mul (Const 1) x) rho = Some v.
Proof. intros x rho v Hx. simpl. rewrite Hx. simpl. destruct v; reflexivity. Qed.

Theorem sr_mul_zero_l_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (BinOp Mul (Const 0) x) rho = Some 0%Z.
Proof. intros x rho v Hx. simpl. rewrite Hx. reflexivity. Qed.

Theorem sr_add_zero_l_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (BinOp Add (Const 0) x) rho = Some v.
Proof. intros x rho v Hx. simpl. rewrite Hx. reflexivity. Qed.

(* Power rules — stated on reduced form since eval_expr does not model Pow *)
Theorem sr_pow_two_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (strength_reduce (BinOp Pow x (Const 2))) rho = Some (v * v)%Z.
Proof. intros x rho v Hx. simpl. rewrite Hx. reflexivity. Qed.

Theorem sr_pow_one_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (strength_reduce (BinOp Pow x (Const 1))) rho = Some v.
Proof. intros x rho v Hx. simpl. exact Hx. Qed.

Theorem sr_pow_zero_sem : forall x rho,
  eval_expr (strength_reduce (BinOp Pow x (Const 0))) rho = Some 1%Z.
Proof. reflexivity. Qed.

(* Division by 1 — stated on reduced form (eval_expr does not model division) *)
Theorem sr_truediv_one_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (strength_reduce (BinOp TrueDiv x (Const 1))) rho = Some v.
Proof. intros x rho v Hx. simpl. exact Hx. Qed.

Theorem sr_floordiv_one_sem : forall x rho v,
  eval_expr x rho = Some v ->
  eval_expr (strength_reduce (BinOp FloorDiv x (Const 1))) rho = Some v.
Proof. intros x rho v Hx. simpl. exact Hx. Qed.

(* Boolean semantic correctness via eval_bool *)
Theorem sr_and_true_r_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp And x (BConst true)) rho = Some b.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  destruct (v =? 0)%Z eqn:Hv; simpl; injection Hx as <-; reflexivity.
Qed.

Theorem sr_and_true_l_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp And (BConst true) x) rho = Some b.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  exact Hx.
Qed.

Theorem sr_and_false_r_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp And x (BConst false)) rho = Some false.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  destruct (v =? 0)%Z; reflexivity.
Qed.

Theorem sr_and_false_l_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp And (BConst false) x) rho = Some false.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  reflexivity.
Qed.

Theorem sr_or_false_r_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp Or x (BConst false)) rho = Some b.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  destruct (v =? 0)%Z eqn:Hv; simpl.
  - injection Hx as <-. reflexivity.
  - rewrite Hv. exact Hx.
Qed.

Theorem sr_or_false_l_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp Or (BConst false) x) rho = Some b.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  exact Hx.
Qed.

Theorem sr_or_true_r_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp Or x (BConst true)) rho = Some true.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  destruct (v =? 0)%Z eqn:Hv; simpl.
  - reflexivity.
  - rewrite Hv. reflexivity.
Qed.

Theorem sr_or_true_l_sem : forall x rho b,
  eval_bool x rho = Some b ->
  eval_bool (BinOp Or (BConst true) x) rho = Some true.
Proof.
  intros x rho b0 Hx. unfold eval_bool in *. simpl.
  destruct (eval_expr x rho) as [v|]; [|discriminate].
  reflexivity.
Qed.


(* --- Semantic soundness: strength_reduce preserves eval_bool --- *)
(*
 * Note: strength_reduce does NOT preserve eval_expr for And/Or.
 * Example: And x (BConst true) -> x changes value when x > 1
 * (original: if x=0 then 0 else 1, reduced: x).
 * But it DOES preserve eval_bool (truthiness).
 *)

(* Close strength_reduce goals in two phases:
   A: arithmetic normalization + rewrite expanded eval_expr hypotheses
   B: injection + Z case split for And/Or goals *)
Local Ltac sr_close :=
  try assumption; try congruence;
  (* Phase A: normalize arithmetic, substitute expanded eval hypotheses *)
  try solve [
    rewrite ?Z.mul_1_r, ?Z.mul_0_r, ?Z.mul_1_l, ?Z.mul_0_l,
            ?Z.add_0_r, ?Z.add_0_l, ?Z.sub_0_r in *;
    (* Rewrite non-Some hypotheses (expanded eval_expr) into goal *)
    repeat match goal with H' : ?lhs = Some _ |- _ =>
      match lhs with Some _ => fail 1 | _ => rewrite H' end end;
    simpl;
    reflexivity || assumption || congruence
  ];
  (* Phase B: inject + Z case split for And/Or *)
  try solve [
    repeat match goal with H : Some _ = Some _ |- _ =>
      injection H; clear H; intros; subst end;
    rewrite ?Z.mul_1_r, ?Z.mul_0_r, ?Z.mul_1_l, ?Z.mul_0_l,
            ?Z.add_0_r, ?Z.add_0_l, ?Z.sub_0_r in *;
    repeat match goal with H' : ?lhs = Some _ |- _ =>
      match lhs with Some _ => fail 1 | _ => rewrite H' end end;
    simpl;
    try reflexivity; try congruence;
    match goal with |- context [(?v =? 0)%Z] =>
      let E := fresh in
      destruct (v =? 0)%Z eqn:E; simpl in *;
      try (rewrite E in *; simpl in *); reflexivity || congruence end
  ].

Theorem strength_reduce_bool_sound : forall e rho bv,
  eval_bool e rho = Some bv ->
  eval_bool (strength_reduce e) rho = Some bv.
Proof.
  unfold eval_bool.
  intros [| | | |op l r| | | | |] rho bv H; try exact H.
  destruct op; try exact H; simpl in H; try discriminate.
  all: destruct (eval_expr l rho) eqn:Hl; [|discriminate].
  all: destruct (eval_expr r rho) eqn:Hr; [|discriminate].
  (* Phase 1: case-split r to resolve strength_reduce's outer match *)
  all: destruct r; simpl in Hr; try discriminate;
       try (injection Hr as <-);
       simpl; try sr_close.
  all: try (destruct b; simpl; try sr_close).
  (* Phase 2: case-split l to resolve strength_reduce's inner match *)
  all: destruct l; simpl in Hl; try discriminate;
       try (injection Hl as <-);
       simpl; try sr_close.
  all: try (destruct b; simpl; try sr_close).
  (* Phase 3: resolve remaining Z matches *)
  all: try (match goal with
            | |- context [match ?v with 0%Z => _ | Z.pos _ => _ | Z.neg _ => _ end] =>
                destruct v as [|[?|?|]|[?|?|]]; simpl; try sr_close
            end).
  all: try (match goal with
            | |- context [match ?v with 0%Z => _ | Z.pos _ => _ | Z.neg _ => _ end] =>
                destruct v as [|[?|?|]|[?|?|]]; simpl; try sr_close
            end).
Qed.
