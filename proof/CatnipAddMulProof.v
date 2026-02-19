From Coq Require Import List.
Import ListNotations.

(* Fragment aligned with catnip_grammar/grammar.js:
   _additive       -> additive | _multiplicative
   additive        -> _additive ("+" | "-") _multiplicative   (left)
   _multiplicative -> multiplicative | _exponent
   multiplicative  -> _multiplicative ("*" | "/" | "//" | "%") _exponent (left)

   Here we keep only "+" and "*" plus parenthesized atoms.
   We use an equivalent non-left-recursive parser to mechanize proofs. *)

Inductive token :=
| TNum
| TPlus
| TMul
| TLParen
| TRParen.

Inductive expr :=
| ELit
| EAdd (l r : expr)
| EMul (l r : expr).

Fixpoint parse_expr (fuel : nat) (ts : list token) : option (expr * list token)
with parse_expr_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_term (fuel : nat) (ts : list token) : option (expr * list token)
with parse_term_tail (fuel : nat) (lhs : expr) (ts : list token) : option (expr * list token)
with parse_factor (fuel : nat) (ts : list token) : option (expr * list token).
Proof.
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_term fuel' ts with
        | Some (lhs, rest) => parse_expr_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TPlus :: rest =>
            match parse_term fuel' rest with
            | Some (rhs, rest') => parse_expr_tail fuel' (EAdd lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match parse_factor fuel' ts with
        | Some (lhs, rest) => parse_term_tail fuel' lhs rest
        | None => None
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TMul :: rest =>
            match parse_factor fuel' rest with
            | Some (rhs, rest') => parse_term_tail fuel' (EMul lhs rhs) rest'
            | None => None
            end
        | _ => Some (lhs, ts)
        end
      ).
  - destruct fuel as [|fuel'].
    + exact None.
    + refine (
        match ts with
        | TNum :: rest => Some (ELit, rest)
        | TLParen :: rest =>
            match parse_expr fuel' rest with
            | Some (e, TRParen :: rest') => Some (e, rest')
            | _ => None
            end
        | _ => None
        end
      ).
Defined.

Definition parse_full (fuel : nat) (ts : list token) : option expr :=
  match parse_expr fuel ts with
  | Some (e, []) => Some e
  | _ => None
  end.

Theorem parse_expr_deterministic :
  forall fuel ts e1 r1 e2 r2,
    parse_expr fuel ts = Some (e1, r1) ->
    parse_expr fuel ts = Some (e2, r2) ->
    e1 = e2 /\ r1 = r2.
Proof.
  intros fuel ts e1 r1 e2 r2 H1 H2.
  rewrite H1 in H2.
  inversion H2.
  auto.
Qed.

(* "*" has higher precedence than "+" in this fragment. *)
Theorem precedence_example :
  parse_full 32 [TNum; TPlus; TNum; TMul; TNum] =
  Some (EAdd ELit (EMul ELit ELit)).
Proof.
  reflexivity.
Qed.

(* Additive chains are left-associative. *)
Theorem left_assoc_add_example :
  parse_full 32 [TNum; TPlus; TNum; TPlus; TNum] =
  Some (EAdd (EAdd ELit ELit) ELit).
Proof.
  reflexivity.
Qed.

(* Parentheses override precedence. *)
Theorem paren_example :
  parse_full 32 [TLParen; TNum; TPlus; TNum; TRParen; TMul; TNum] =
  Some (EMul (EAdd ELit ELit) ELit).
Proof.
  reflexivity.
Qed.
