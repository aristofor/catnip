From Coq Require Import List Bool Arith.
Import ListNotations.

(* Fragment aligned with catnip_grammar/grammar.js:
   _bool_or -> bool_or | _bool_and
   _bool_and -> bool_and | _bool_not
   _bool_not -> bool_not | _comparison
   _comparison -> comparison | _additive
   _additive -> additive | _multiplicative
   _multiplicative -> multiplicative | atom
*)

Inductive token :=
| TNum
| TTrue
| TFalse
| TPlus
| TMinus
| TMul
| TDiv
| TLt
| TLe
| TGt
| TGe
| TNe
| TEq
| TAnd
| TOr
| TNot
| TLParen
| TRParen.

Inductive expr :=
| ENum
| ETrueLit
| EFalseLit
| EAdd (l r : expr)
| ESub (l r : expr)
| EMul (l r : expr)
| EDiv (l r : expr)
| ELt (l r : expr)
| ELe (l r : expr)
| EGt (l r : expr)
| EGe (l r : expr)
| ENe (l r : expr)
| EEq (l r : expr)
| EAnd (l r : expr)
| EOr (l r : expr)
| ENot (e : expr).

Fixpoint parse_bool_or (fuel : nat) (ts : list token) : option (expr * list token)
with parse_bool_or_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_bool_and (fuel : nat) (ts : list token) : option (expr * list token)
with parse_bool_and_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_bool_not (fuel : nat) (ts : list token) : option (expr * list token)
with parse_comparison (fuel : nat) (ts : list token) : option (expr * list token)
with parse_comparison_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_add (fuel : nat) (ts : list token) : option (expr * list token)
with parse_add_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_mul (fuel : nat) (ts : list token) : option (expr * list token)
with parse_mul_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_atom (fuel : nat) (ts : list token) : option (expr * list token).
Proof.
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_bool_and fuel' ts with
        | Some (lhs, rest) => parse_bool_or_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TOr :: rest =>
            match parse_bool_and fuel' rest with
            | Some (rhs, rest') => parse_bool_or_tail fuel' (EOr lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_bool_not fuel' ts with
        | Some (lhs, rest) => parse_bool_and_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TAnd :: rest =>
            match parse_bool_not fuel' rest with
            | Some (rhs, rest') => parse_bool_and_tail fuel' (EAnd lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TNot :: rest =>
            match parse_bool_not fuel' rest with
            | Some (e, rest') => Some (ENot e, rest')
            | None => None
            end
        | _ => parse_comparison fuel' ts
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_add fuel' ts with
        | Some (lhs, rest) => parse_comparison_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TLt :: rest =>
            match parse_add fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (ELt lhs rhs) rest'
            | None => None
            end
        | TLe :: rest =>
            match parse_add fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (ELe lhs rhs) rest'
            | None => None
            end
        | TGt :: rest =>
            match parse_add fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (EGt lhs rhs) rest'
            | None => None
            end
        | TGe :: rest =>
            match parse_add fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (EGe lhs rhs) rest'
            | None => None
            end
        | TNe :: rest =>
            match parse_add fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (ENe lhs rhs) rest'
            | None => None
            end
        | TEq :: rest =>
            match parse_add fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (EEq lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_mul fuel' ts with
        | Some (lhs, rest) => parse_add_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TPlus :: rest =>
            match parse_mul fuel' rest with
            | Some (rhs, rest') => parse_add_tail fuel' (EAdd lhs rhs) rest'
            | None => None
            end
        | TMinus :: rest =>
            match parse_mul fuel' rest with
            | Some (rhs, rest') => parse_add_tail fuel' (ESub lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_atom fuel' ts with
        | Some (lhs, rest) => parse_mul_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TMul :: rest =>
            match parse_atom fuel' rest with
            | Some (rhs, rest') => parse_mul_tail fuel' (EMul lhs rhs) rest'
            | None => None
            end
        | TDiv :: rest =>
            match parse_atom fuel' rest with
            | Some (rhs, rest') => parse_mul_tail fuel' (EDiv lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TNum :: rest => Some (ENum, rest)
        | TTrue :: rest => Some (ETrueLit, rest)
        | TFalse :: rest => Some (EFalseLit, rest)
        | TLParen :: rest =>
            match parse_bool_or fuel' rest with
            | Some (e, TRParen :: rest') => Some (e, rest')
            | _ => None
            end
        | _ => None
        end
      ).
Defined.

Definition parse_full (fuel : nat) (ts : list token) : option expr :=
  match parse_bool_or fuel ts with
  | Some (e, []) => Some e
  | _ => None
  end.

Theorem parse_bool_or_deterministic :
  forall fuel ts e1 r1 e2 r2,
    parse_bool_or fuel ts = Some (e1, r1) ->
    parse_bool_or fuel ts = Some (e2, r2) ->
    e1 = e2 /\ r1 = r2.
Proof.
  intros fuel ts e1 r1 e2 r2 H1 H2.
  rewrite H1 in H2.
  inversion H2.
  auto.
Qed.

Theorem mul_over_add :
  parse_full 64 [TNum; TPlus; TNum; TMul; TNum] =
  Some (EAdd ENum (EMul ENum ENum)).
Proof.
  reflexivity.
Qed.

Theorem sub_left_assoc :
  parse_full 64 [TNum; TMinus; TNum; TMinus; TNum] =
  Some (ESub (ESub ENum ENum) ENum).
Proof.
  reflexivity.
Qed.

Theorem cmp_over_and :
  parse_full 64 [TNum; TLt; TNum; TAnd; TTrue] =
  Some (EAnd (ELt ENum ENum) ETrueLit).
Proof.
  reflexivity.
Qed.

Theorem not_and_or_precedence :
  parse_full 64 [TNot; TNum; TLt; TNum; TOr; TFalse] =
  Some (EOr (ENot (ELt ENum ENum)) EFalseLit).
Proof.
  reflexivity.
Qed.

Theorem paren_override_bool :
  parse_full 64 [TLParen; TTrue; TOr; TFalse; TRParen; TAnd; TTrue] =
  Some (EAnd (EOr ETrueLit EFalseLit) ETrueLit).
Proof.
  reflexivity.
Qed.

Theorem comparison_ops_examples :
  parse_full 64 [TNum; TLe; TNum] = Some (ELe ENum ENum) /\
  parse_full 64 [TNum; TGt; TNum] = Some (EGt ENum ENum) /\
  parse_full 64 [TNum; TGe; TNum] = Some (EGe ENum ENum) /\
  parse_full 64 [TNum; TNe; TNum] = Some (ENe ENum ENum) /\
  parse_full 64 [TNum; TEq; TNum] = Some (EEq ENum ENum).
Proof.
  repeat split; reflexivity.
Qed.

(* Syntactic chaining, aligned with repeat1(comp_op, expr) shape. *)
Theorem comparison_chain_example :
  parse_full 64 [TNum; TLt; TNum; TLe; TNum] =
  Some (ELe (ELt ENum ENum) ENum).
Proof.
  reflexivity.
Qed.

(* Runtime semantics model for chained comparisons over numeric values. *)
Inductive comp_op :=
| OpLt
| OpLe
| OpGt
| OpGe
| OpEq
| OpNe.

Definition eval_comp_op (op : comp_op) (x y : nat) : bool :=
  match op with
  | OpLt => Nat.ltb x y
  | OpLe => Nat.leb x y
  | OpGt => Nat.ltb y x
  | OpGe => Nat.leb y x
  | OpEq => Nat.eqb x y
  | OpNe => negb (Nat.eqb x y)
  end.

Fixpoint eval_comp_chain (head : nat) (rest : list (comp_op * nat)) : bool :=
  match rest with
  | [] => true
  | (op, y) :: tl => andb (eval_comp_op op head y) (eval_comp_chain y tl)
  end.

Theorem chain_two_ops_desugars_to_and :
  forall a b c op1 op2,
    eval_comp_chain a [(op1, b); (op2, c)] =
    andb (eval_comp_op op1 a b) (eval_comp_op op2 b c).
Proof.
  intros a b c op1 op2.
  simpl.
  rewrite andb_true_r.
  reflexivity.
Qed.

Theorem chain_lt_le_matches_expected :
  forall a b c,
    eval_comp_chain a [(OpLt, b); (OpLe, c)] =
    andb (Nat.ltb a b) (Nat.leb b c).
Proof.
  intros a b c.
  apply chain_two_ops_desugars_to_and.
Qed.
