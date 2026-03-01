(* FILE: proof/expr/CatnipExprProof.v *)
From Coq Require Import List Bool Arith Lia.
Import ListNotations.

(* Fragment aligned with catnip_grammar/grammar.js:
   _expression -> _bool_or
   _bool_or -> bool_or | _bool_and
   _bool_and -> bool_and | _bool_not
   _bool_not -> bool_not | _comparison
   _comparison -> comparison | _bit_or
   _bit_or -> bit_or | _bit_xor
   _bit_xor -> bit_xor | _bit_and
   _bit_and -> bit_and | _shift
   _shift -> shift | _additive
   _additive -> additive | _multiplicative
   _multiplicative -> multiplicative | _exponent
   _exponent -> exponent | _unary
   _unary -> unary | atom
*)

Inductive token :=
| TNum
| TTrue
| TFalse
| TPlus
| TMinus
| TMul
| TDiv
| TFloorDiv
| TMod
| TStarStar
| TPipe
| TCaret
| TAmpersand
| TLShift
| TRShift
| TTilde
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
| EFloorDiv (l r : expr)
| EMod (l r : expr)
| EExp (l r : expr)
| EBitOr (l r : expr)
| EBitXor (l r : expr)
| EBitAnd (l r : expr)
| ELShift (l r : expr)
| ERShift (l r : expr)
| ENeg (e : expr)
| EPos (e : expr)
| EBitNot (e : expr)
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
with parse_bit_or (fuel : nat) (ts : list token) : option (expr * list token)
with parse_bit_or_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_bit_xor (fuel : nat) (ts : list token) : option (expr * list token)
with parse_bit_xor_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_bit_and (fuel : nat) (ts : list token) : option (expr * list token)
with parse_bit_and_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_shift (fuel : nat) (ts : list token) : option (expr * list token)
with parse_shift_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_add (fuel : nat) (ts : list token) : option (expr * list token)
with parse_add_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_mul (fuel : nat) (ts : list token) : option (expr * list token)
with parse_mul_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_exp (fuel : nat) (ts : list token) : option (expr * list token)
with parse_unary (fuel : nat) (ts : list token) : option (expr * list token)
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
        match parse_bit_or fuel' ts with
        | Some (lhs, rest) => parse_comparison_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TLt :: rest =>
            match parse_bit_or fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (ELt lhs rhs) rest'
            | None => None
            end
        | TLe :: rest =>
            match parse_bit_or fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (ELe lhs rhs) rest'
            | None => None
            end
        | TGt :: rest =>
            match parse_bit_or fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (EGt lhs rhs) rest'
            | None => None
            end
        | TGe :: rest =>
            match parse_bit_or fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (EGe lhs rhs) rest'
            | None => None
            end
        | TNe :: rest =>
            match parse_bit_or fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (ENe lhs rhs) rest'
            | None => None
            end
        | TEq :: rest =>
            match parse_bit_or fuel' rest with
            | Some (rhs, rest') => parse_comparison_tail fuel' (EEq lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_bit_xor fuel' ts with
        | Some (lhs, rest) => parse_bit_or_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TPipe :: rest =>
            match parse_bit_xor fuel' rest with
            | Some (rhs, rest') => parse_bit_or_tail fuel' (EBitOr lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_bit_and fuel' ts with
        | Some (lhs, rest) => parse_bit_xor_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TCaret :: rest =>
            match parse_bit_and fuel' rest with
            | Some (rhs, rest') => parse_bit_xor_tail fuel' (EBitXor lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_shift fuel' ts with
        | Some (lhs, rest) => parse_bit_and_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TAmpersand :: rest =>
            match parse_shift fuel' rest with
            | Some (rhs, rest') => parse_bit_and_tail fuel' (EBitAnd lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_add fuel' ts with
        | Some (lhs, rest) => parse_shift_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TLShift :: rest =>
            match parse_add fuel' rest with
            | Some (rhs, rest') => parse_shift_tail fuel' (ELShift lhs rhs) rest'
            | None => None
            end
        | TRShift :: rest =>
            match parse_add fuel' rest with
            | Some (rhs, rest') => parse_shift_tail fuel' (ERShift lhs rhs) rest'
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
        match parse_exp fuel' ts with
        | Some (lhs, rest) => parse_mul_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TMul :: rest =>
            match parse_exp fuel' rest with
            | Some (rhs, rest') => parse_mul_tail fuel' (EMul lhs rhs) rest'
            | None => None
            end
        | TDiv :: rest =>
            match parse_exp fuel' rest with
            | Some (rhs, rest') => parse_mul_tail fuel' (EDiv lhs rhs) rest'
            | None => None
            end
        | TFloorDiv :: rest =>
            match parse_exp fuel' rest with
            | Some (rhs, rest') => parse_mul_tail fuel' (EFloorDiv lhs rhs) rest'
            | None => None
            end
        | TMod :: rest =>
            match parse_exp fuel' rest with
            | Some (rhs, rest') => parse_mul_tail fuel' (EMod lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_unary fuel' ts with
        | Some (lhs, TStarStar :: rest) =>
            match parse_exp fuel' rest with
            | Some (rhs, rest') => Some (EExp lhs rhs, rest')
            | None => None
            end
        | Some (lhs, rest) => Some (lhs, rest)
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TMinus :: rest =>
            match parse_unary fuel' rest with
            | Some (e, r) => Some (ENeg e, r)
            | None => None
            end
        | TPlus :: rest =>
            match parse_unary fuel' rest with
            | Some (e, r) => Some (EPos e, r)
            | None => None
            end
        | TTilde :: rest =>
            match parse_unary fuel' rest with
            | Some (e, r) => Some (EBitNot e, r)
            | None => None
            end
        | _ => parse_atom fuel' ts
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

Ltac solve_parse_example := vm_compute; reflexivity.

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
  solve_parse_example.
Qed.

Theorem sub_left_assoc :
  parse_full 64 [TNum; TMinus; TNum; TMinus; TNum] =
  Some (ESub (ESub ENum ENum) ENum).
Proof.
  solve_parse_example.
Qed.

Theorem cmp_over_and :
  parse_full 64 [TNum; TLt; TNum; TAnd; TTrue] =
  Some (EAnd (ELt ENum ENum) ETrueLit).
Proof.
  solve_parse_example.
Qed.

Theorem not_and_or_precedence :
  parse_full 64 [TNot; TNum; TLt; TNum; TOr; TFalse] =
  Some (EOr (ENot (ELt ENum ENum)) EFalseLit).
Proof.
  solve_parse_example.
Qed.

Theorem paren_override_bool :
  parse_full 64 [TLParen; TTrue; TOr; TFalse; TRParen; TAnd; TTrue] =
  Some (EAnd (EOr ETrueLit EFalseLit) ETrueLit).
Proof.
  solve_parse_example.
Qed.

Theorem comparison_ops_examples :
  parse_full 64 [TNum; TLe; TNum] = Some (ELe ENum ENum) /\
  parse_full 64 [TNum; TGt; TNum] = Some (EGt ENum ENum) /\
  parse_full 64 [TNum; TGe; TNum] = Some (EGe ENum ENum) /\
  parse_full 64 [TNum; TNe; TNum] = Some (ENe ENum ENum) /\
  parse_full 64 [TNum; TEq; TNum] = Some (EEq ENum ENum).
Proof.
  repeat split; solve_parse_example.
Qed.

(* Syntactic chaining, aligned with repeat1(comp_op, expr) shape. *)
Theorem comparison_chain_example :
  parse_full 64 [TNum; TLt; TNum; TLe; TNum] =
  Some (ELe (ELt ENum ENum) ENum).
Proof.
  solve_parse_example.
Qed.

(* ** binds tighter than * *)
Theorem exp_over_mul :
  parse_full 64 [TNum; TMul; TNum; TStarStar; TNum] =
  Some (EMul ENum (EExp ENum ENum)).
Proof.
  solve_parse_example.
Qed.

(* Unary binds tighter than ** *)
Theorem unary_over_exp :
  parse_full 64 [TMinus; TNum; TStarStar; TNum] =
  Some (EExp (ENeg ENum) ENum).
Proof.
  solve_parse_example.
Qed.

(* ** is right-associative *)
Theorem exp_right_assoc :
  parse_full 64 [TNum; TStarStar; TNum; TStarStar; TNum] =
  Some (EExp ENum (EExp ENum ENum)).
Proof.
  solve_parse_example.
Qed.

(* << binds tighter than & *)
Theorem shift_over_bit_and :
  parse_full 64 [TNum; TAmpersand; TNum; TLShift; TNum] =
  Some (EBitAnd ENum (ELShift ENum ENum)).
Proof.
  solve_parse_example.
Qed.

(* & binds tighter than ^ *)
Theorem bit_and_over_bit_xor :
  parse_full 64 [TNum; TCaret; TNum; TAmpersand; TNum] =
  Some (EBitXor ENum (EBitAnd ENum ENum)).
Proof.
  solve_parse_example.
Qed.

(* ^ binds tighter than | *)
Theorem bit_xor_over_bit_or :
  parse_full 64 [TNum; TPipe; TNum; TCaret; TNum] =
  Some (EBitOr ENum (EBitXor ENum ENum)).
Proof.
  solve_parse_example.
Qed.

(* + binds tighter than << *)
Theorem add_over_shift :
  parse_full 64 [TNum; TLShift; TNum; TPlus; TNum] =
  Some (ELShift ENum (EAdd ENum ENum)).
Proof.
  solve_parse_example.
Qed.

(* | binds tighter than comparison *)
Theorem bit_or_over_comparison :
  parse_full 64 [TNum; TPipe; TNum; TLt; TNum] =
  Some (ELt (EBitOr ENum ENum) ENum).
Proof.
  solve_parse_example.
Qed.

(* Chained unary: --x *)
Theorem unary_chain :
  parse_full 64 [TMinus; TMinus; TNum] =
  Some (ENeg (ENeg ENum)).
Proof.
  solve_parse_example.
Qed.

(* ~x + y: unary binds tighter than addition *)
Theorem bit_not_and_add :
  parse_full 64 [TTilde; TNum; TPlus; TNum] =
  Some (EAdd (EBitNot ENum) ENum).
Proof.
  solve_parse_example.
Qed.

(* // and % at same level as * (left-associative) *)
Theorem floor_div_same_as_mul :
  parse_full 64 [TNum; TMul; TNum; TFloorDiv; TNum] =
  Some (EFloorDiv (EMul ENum ENum) ENum).
Proof.
  solve_parse_example.
Qed.

Theorem mod_over_add :
  parse_full 64 [TNum; TMod; TNum; TPlus; TNum] =
  Some (EAdd (EMod ENum ENum) ENum).
Proof.
  solve_parse_example.
Qed.

(* Parentheses override right-associativity of ** *)
Theorem paren_override_exp :
  parse_full 64 [TLParen; TNum; TStarStar; TNum; TRParen; TStarStar; TNum] =
  Some (EExp (EExp ENum ENum) ENum).
Proof.
  solve_parse_example.
Qed.

(* << is left-associative *)
Theorem shift_left_assoc :
  parse_full 64 [TNum; TLShift; TNum; TRShift; TNum] =
  Some (ERShift (ELShift ENum ENum) ENum).
Proof.
  solve_parse_example.
Qed.

(* Unary + *)
Theorem unary_pos :
  parse_full 64 [TPlus; TNum; TMul; TNum] =
  Some (EMul (EPos ENum) ENum).
Proof.
  solve_parse_example.
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
