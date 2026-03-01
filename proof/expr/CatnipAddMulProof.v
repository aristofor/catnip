(* FILE: proof/expr/CatnipAddMulProof.v *)
From Coq Require Import List Lia.
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

Ltac solve_parse_example := vm_compute; reflexivity.

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
  solve_parse_example.
Qed.

(* Additive chains are left-associative. *)
Theorem left_assoc_add_example :
  parse_full 32 [TNum; TPlus; TNum; TPlus; TNum] =
  Some (EAdd (EAdd ELit ELit) ELit).
Proof.
  solve_parse_example.
Qed.

(* Parentheses override precedence. *)
Theorem paren_example :
  parse_full 32 [TLParen; TNum; TPlus; TNum; TRParen; TMul; TNum] =
  Some (EMul (EAdd ELit ELit) ELit).
Proof.
  solve_parse_example.
Qed.


(* ================================================================ *)
(* UNFOLDING LEMMAS                                                   *)
(* ================================================================ *)

Lemma parse_expr_unfold : forall f ts,
  parse_expr (S f) ts =
  match parse_term f ts with
  | Some (lhs, rest) => parse_expr_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_expr_tail_unfold : forall f lhs ts,
  parse_expr_tail (S f) lhs ts =
  match ts with
  | TPlus :: rest =>
      match parse_term f rest with
      | Some (rhs, rest') => parse_expr_tail f (EAdd lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_term_unfold : forall f ts,
  parse_term (S f) ts =
  match parse_factor f ts with
  | Some (lhs, rest) => parse_term_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_term_tail_unfold : forall f lhs ts,
  parse_term_tail (S f) lhs ts =
  match ts with
  | TMul :: rest =>
      match parse_factor f rest with
      | Some (rhs, rest') => parse_term_tail f (EMul lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_factor_unfold : forall f ts,
  parse_factor (S f) ts =
  match ts with
  | TNum :: rest => Some (ELit, rest)
  | TLParen :: rest =>
      match parse_expr f rest with
      | Some (e, TRParen :: rest') => Some (e, rest')
      | _ => None
      end
  | _ => None
  end.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* FUEL MONOTONICITY                                                  *)
(*                                                                    *)
(* If the parser succeeds with fuel [f], it succeeds with the same    *)
(* result for any [f' >= f].  This eliminates the dependency on       *)
(* specific fuel constants (32, 64) in all subsequent results.        *)
(* ================================================================ *)

Theorem fuel_mono : forall fuel,
  (forall ts e r,
     parse_expr fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_expr fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_expr_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_expr_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_term fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_term fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_term_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_term_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_factor fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_factor fuel' ts = Some (e, r)).
Proof.
  induction fuel as [| f IH].
  - repeat split; intros; simpl in *; discriminate.
  - destruct IH as [IHe [IHet [IHt [IHtt IHf]]]].
    repeat split; intros.
    + (* parse_expr *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_expr_unfold in H |- *.
      destruct (parse_term f ts) as [[lhs rest]|] eqn:Ht; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHt _ _ _ Ht _ Hle).
      eapply IHet; eauto.
    + (* parse_expr_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_expr_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H.
      (* TPlus *)
      destruct (parse_term f ts') as [[rhs rest']|] eqn:Ht; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHt _ _ _ Ht _ Hle).
      eapply IHet; eauto.
    + (* parse_term *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_term_unfold in H |- *.
      destruct (parse_factor f ts) as [[lhs rest]|] eqn:Hfa; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHf _ _ _ Hfa _ Hle).
      eapply IHtt; eauto.
    + (* parse_term_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_term_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H.
      (* TMul *)
      destruct (parse_factor f ts') as [[rhs rest']|] eqn:Hfa; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHf _ _ _ Hfa _ Hle).
      eapply IHtt; eauto.
    + (* parse_factor *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_factor_unfold in H |- *.
      destruct ts as [|t ts']; [discriminate|].
      destruct t; try discriminate; try exact H.
      (* TLParen *)
      destruct (parse_expr f ts') as [[e' [|t' rest'']] |] eqn:He;
        try discriminate.
      destruct t'; try discriminate.
      assert (Hle : f <= f') by lia.
      rewrite (IHe _ _ _ He _ Hle). exact H.
Qed.

(* Convenient projections. *)

Corollary parse_expr_mono : forall fuel fuel' ts e r,
  parse_expr fuel ts = Some (e, r) -> fuel <= fuel' ->
  parse_expr fuel' ts = Some (e, r).
Proof. intros. eapply (proj1 (fuel_mono fuel)); eauto. Qed.

Lemma parse_full_consumes_all : forall fuel ts e,
  parse_full fuel ts = Some e ->
  parse_expr fuel ts = Some (e, []).
Proof.
  unfold parse_full. intros fuel ts e.
  destruct (parse_expr fuel ts) as [[e' [|t r']]|].
  - intro H. injection H as <-. reflexivity.
  - discriminate.
  - discriminate.
Qed.

Corollary parse_full_mono : forall fuel fuel' ts e,
  parse_full fuel ts = Some e -> fuel <= fuel' ->
  parse_full fuel' ts = Some e.
Proof.
  intros fuel fuel' ts e Hpf Hle.
  unfold parse_full.
  apply parse_full_consumes_all in Hpf.
  rewrite (parse_expr_mono _ _ _ _ _ Hpf Hle). reflexivity.
Qed.


(* ================================================================ *)
(* FUEL-INDEPENDENT RESULTS                                           *)
(*                                                                    *)
(* With monotonicity we can state properties for all sufficient fuel. *)
(* ================================================================ *)

(* Helper: prove fuel-independent result by providing a witness. *)
Ltac solve_fuel_general n :=
  intros fuel Hge;
  apply (parse_full_mono n); [solve_parse_example | lia].

Theorem precedence_general : forall fuel,
  fuel >= 5 ->
  parse_full fuel [TNum; TPlus; TNum; TMul; TNum] =
  Some (EAdd ELit (EMul ELit ELit)).
Proof. solve_fuel_general 5. Qed.

Theorem left_assoc_add_general : forall fuel,
  fuel >= 5 ->
  parse_full fuel [TNum; TPlus; TNum; TPlus; TNum] =
  Some (EAdd (EAdd ELit ELit) ELit).
Proof. solve_fuel_general 5. Qed.

Theorem paren_override_general : forall fuel,
  fuel >= 7 ->
  parse_full fuel [TLParen; TNum; TPlus; TNum; TRParen; TMul; TNum] =
  Some (EMul (EAdd ELit ELit) ELit).
Proof. solve_fuel_general 7. Qed.

Theorem left_assoc_mul_general : forall fuel,
  fuel >= 5 ->
  parse_full fuel [TNum; TMul; TNum; TMul; TNum] =
  Some (EMul (EMul ELit ELit) ELit).
Proof. solve_fuel_general 5. Qed.


(* ================================================================ *)
(* SOUNDNESS                                                          *)
(*                                                                    *)
(* Inductive parsing relation that mirrors the grammar structure.     *)
(* We prove the parser is sound: if it returns Some, the result       *)
(* satisfies the relation.                                            *)
(* ================================================================ *)

Inductive parses_expr : list token -> expr -> list token -> Prop :=
| PE_intro : forall ts lhs rest e rest',
    parses_term ts lhs rest ->
    parses_expr_tail lhs rest e rest' ->
    parses_expr ts e rest'
with parses_expr_tail : expr -> list token -> expr -> list token -> Prop :=
| PET_plus : forall lhs rest rhs rest' e rest'',
    parses_term rest rhs rest' ->
    parses_expr_tail (EAdd lhs rhs) rest' e rest'' ->
    parses_expr_tail lhs (TPlus :: rest) e rest''
| PET_done : forall lhs ts,
    parses_expr_tail lhs ts lhs ts
with parses_term : list token -> expr -> list token -> Prop :=
| PT_intro : forall ts lhs rest e rest',
    parses_factor ts lhs rest ->
    parses_term_tail lhs rest e rest' ->
    parses_term ts e rest'
with parses_term_tail : expr -> list token -> expr -> list token -> Prop :=
| PTT_mul : forall lhs rest rhs rest' e rest'',
    parses_factor rest rhs rest' ->
    parses_term_tail (EMul lhs rhs) rest' e rest'' ->
    parses_term_tail lhs (TMul :: rest) e rest''
| PTT_done : forall lhs ts,
    parses_term_tail lhs ts lhs ts
with parses_factor : list token -> expr -> list token -> Prop :=
| PF_num : forall rest,
    parses_factor (TNum :: rest) ELit rest
| PF_paren : forall ts e rest,
    parses_expr ts e (TRParen :: rest) ->
    parses_factor (TLParen :: ts) e rest.

Theorem parser_sound : forall fuel,
  (forall ts e r,
     parse_expr fuel ts = Some (e, r) ->
     parses_expr ts e r)
  /\
  (forall lhs ts e r,
     parse_expr_tail fuel lhs ts = Some (e, r) ->
     parses_expr_tail lhs ts e r)
  /\
  (forall ts e r,
     parse_term fuel ts = Some (e, r) ->
     parses_term ts e r)
  /\
  (forall lhs ts e r,
     parse_term_tail fuel lhs ts = Some (e, r) ->
     parses_term_tail lhs ts e r)
  /\
  (forall ts e r,
     parse_factor fuel ts = Some (e, r) ->
     parses_factor ts e r).
Proof.
  induction fuel as [| f IH].
  - repeat split; intros; simpl in *; discriminate.
  - destruct IH as [IHe [IHet [IHt [IHtt IHf]]]].
    repeat split; intros.
    + (* parse_expr *)
      rewrite parse_expr_unfold in H.
      destruct (parse_term f ts) as [[lhs rest]|] eqn:Ht; [|discriminate].
      eapply PE_intro; eauto.
    + (* parse_expr_tail *)
      rewrite parse_expr_tail_unfold in H.
      destruct ts as [|t ts']; [injection H as <- <-; constructor|].
      destruct t; try (injection H as <- <-; constructor).
      (* TPlus *)
      destruct (parse_term f ts') as [[rhs rest']|] eqn:Ht; [|discriminate].
      eapply PET_plus; eauto.
    + (* parse_term *)
      rewrite parse_term_unfold in H.
      destruct (parse_factor f ts) as [[lhs rest]|] eqn:Hfa; [|discriminate].
      eapply PT_intro; eauto.
    + (* parse_term_tail *)
      rewrite parse_term_tail_unfold in H.
      destruct ts as [|t ts']; [injection H as <- <-; constructor|].
      destruct t; try (injection H as <- <-; constructor).
      (* TMul *)
      destruct (parse_factor f ts') as [[rhs rest']|] eqn:Hfa; [|discriminate].
      eapply PTT_mul; eauto.
    + (* parse_factor *)
      rewrite parse_factor_unfold in H.
      destruct ts as [|t ts']; [discriminate|].
      destruct t; try discriminate.
      * (* TNum *)
        injection H as <- <-. constructor.
      * (* TLParen *)
        destruct (parse_expr f ts') as [[e' [|t' rest'']] |] eqn:He;
          try discriminate.
        destruct t'; try discriminate.
        injection H as <- <-.
        apply PF_paren. eauto.
Qed.

(* Soundness for parse_full: the result satisfies the grammar
   and the entire input was consumed. *)

Corollary parse_full_sound : forall fuel ts e,
  parse_full fuel ts = Some e ->
  parses_expr ts e [].
Proof.
  intros fuel ts e H.
  apply parse_full_consumes_all in H.
  eapply (proj1 (parser_sound fuel)); eauto.
Qed.
