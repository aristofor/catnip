(* FILE: proof/expr/CatnipExprMonoProof.v *)
(*                                                                    *)
(* Fuel monotonicity and comparison chain desugaring for the           *)
(* expression parser.  Split from CatnipExprProof.v to reduce         *)
(* peak memory during compilation.                                     *)

From Coq Require Import List Bool Arith Lia.
Import ListNotations.
From Catnip Require Import CatnipExprProof.


(* ================================================================ *)
(* UNFOLDING LEMMAS                                                   *)
(* ================================================================ *)

Lemma parse_bool_or_unfold : forall f ts,
  parse_bool_or (S f) ts =
  match parse_bool_and f ts with
  | Some (lhs, rest) => parse_bool_or_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_bool_or_tail_unfold : forall f lhs ts,
  parse_bool_or_tail (S f) lhs ts =
  match ts with
  | TOr :: rest =>
      match parse_bool_and f rest with
      | Some (rhs, rest') => parse_bool_or_tail f (EOr lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_bool_and_unfold : forall f ts,
  parse_bool_and (S f) ts =
  match parse_bool_not f ts with
  | Some (lhs, rest) => parse_bool_and_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_bool_and_tail_unfold : forall f lhs ts,
  parse_bool_and_tail (S f) lhs ts =
  match ts with
  | TAnd :: rest =>
      match parse_bool_not f rest with
      | Some (rhs, rest') => parse_bool_and_tail f (EAnd lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_bool_not_unfold : forall f ts,
  parse_bool_not (S f) ts =
  match ts with
  | TNot :: rest =>
      match parse_bool_not f rest with
      | Some (e, rest') => Some (ENot e, rest')
      | None => None
      end
  | _ => parse_comparison f ts
  end.
Proof. reflexivity. Qed.

Lemma parse_comparison_unfold : forall f ts,
  parse_comparison (S f) ts =
  match parse_bit_or f ts with
  | Some (lhs, rest) => parse_comparison_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_comparison_tail_unfold : forall f lhs ts,
  parse_comparison_tail (S f) lhs ts =
  match ts with
  | TLt :: rest =>
      match parse_bit_or f rest with
      | Some (rhs, rest') => parse_comparison_tail f (ELt lhs rhs) rest'
      | None => None
      end
  | TLe :: rest =>
      match parse_bit_or f rest with
      | Some (rhs, rest') => parse_comparison_tail f (ELe lhs rhs) rest'
      | None => None
      end
  | TGt :: rest =>
      match parse_bit_or f rest with
      | Some (rhs, rest') => parse_comparison_tail f (EGt lhs rhs) rest'
      | None => None
      end
  | TGe :: rest =>
      match parse_bit_or f rest with
      | Some (rhs, rest') => parse_comparison_tail f (EGe lhs rhs) rest'
      | None => None
      end
  | TNe :: rest =>
      match parse_bit_or f rest with
      | Some (rhs, rest') => parse_comparison_tail f (ENe lhs rhs) rest'
      | None => None
      end
  | TEq :: rest =>
      match parse_bit_or f rest with
      | Some (rhs, rest') => parse_comparison_tail f (EEq lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_bit_or_unfold : forall f ts,
  parse_bit_or (S f) ts =
  match parse_bit_xor f ts with
  | Some (lhs, rest) => parse_bit_or_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_bit_or_tail_unfold : forall f lhs ts,
  parse_bit_or_tail (S f) lhs ts =
  match ts with
  | TPipe :: rest =>
      match parse_bit_xor f rest with
      | Some (rhs, rest') => parse_bit_or_tail f (EBitOr lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_bit_xor_unfold : forall f ts,
  parse_bit_xor (S f) ts =
  match parse_bit_and f ts with
  | Some (lhs, rest) => parse_bit_xor_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_bit_xor_tail_unfold : forall f lhs ts,
  parse_bit_xor_tail (S f) lhs ts =
  match ts with
  | TCaret :: rest =>
      match parse_bit_and f rest with
      | Some (rhs, rest') => parse_bit_xor_tail f (EBitXor lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_bit_and_unfold : forall f ts,
  parse_bit_and (S f) ts =
  match parse_shift f ts with
  | Some (lhs, rest) => parse_bit_and_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_bit_and_tail_unfold : forall f lhs ts,
  parse_bit_and_tail (S f) lhs ts =
  match ts with
  | TAmpersand :: rest =>
      match parse_shift f rest with
      | Some (rhs, rest') => parse_bit_and_tail f (EBitAnd lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_shift_unfold : forall f ts,
  parse_shift (S f) ts =
  match parse_add f ts with
  | Some (lhs, rest) => parse_shift_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_shift_tail_unfold : forall f lhs ts,
  parse_shift_tail (S f) lhs ts =
  match ts with
  | TLShift :: rest =>
      match parse_add f rest with
      | Some (rhs, rest') => parse_shift_tail f (ELShift lhs rhs) rest'
      | None => None
      end
  | TRShift :: rest =>
      match parse_add f rest with
      | Some (rhs, rest') => parse_shift_tail f (ERShift lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_add_unfold : forall f ts,
  parse_add (S f) ts =
  match parse_mul f ts with
  | Some (lhs, rest) => parse_add_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_add_tail_unfold : forall f lhs ts,
  parse_add_tail (S f) lhs ts =
  match ts with
  | TPlus :: rest =>
      match parse_mul f rest with
      | Some (rhs, rest') => parse_add_tail f (EAdd lhs rhs) rest'
      | None => None
      end
  | TMinus :: rest =>
      match parse_mul f rest with
      | Some (rhs, rest') => parse_add_tail f (ESub lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_mul_unfold : forall f ts,
  parse_mul (S f) ts =
  match parse_exp f ts with
  | Some (lhs, rest) => parse_mul_tail f lhs rest
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_mul_tail_unfold : forall f lhs ts,
  parse_mul_tail (S f) lhs ts =
  match ts with
  | TMul :: rest =>
      match parse_exp f rest with
      | Some (rhs, rest') => parse_mul_tail f (EMul lhs rhs) rest'
      | None => None
      end
  | TDiv :: rest =>
      match parse_exp f rest with
      | Some (rhs, rest') => parse_mul_tail f (EDiv lhs rhs) rest'
      | None => None
      end
  | TFloorDiv :: rest =>
      match parse_exp f rest with
      | Some (rhs, rest') => parse_mul_tail f (EFloorDiv lhs rhs) rest'
      | None => None
      end
  | TMod :: rest =>
      match parse_exp f rest with
      | Some (rhs, rest') => parse_mul_tail f (EMod lhs rhs) rest'
      | None => None
      end
  | _ => Some (lhs, ts)
  end.
Proof. reflexivity. Qed.

Lemma parse_exp_unfold : forall f ts,
  parse_exp (S f) ts =
  match parse_unary f ts with
  | Some (lhs, TStarStar :: rest) =>
      match parse_exp f rest with
      | Some (rhs, rest') => Some (EExp lhs rhs, rest')
      | None => None
      end
  | Some (lhs, rest) => Some (lhs, rest)
  | None => None
  end.
Proof. reflexivity. Qed.

Lemma parse_unary_unfold : forall f ts,
  parse_unary (S f) ts =
  match ts with
  | TMinus :: rest =>
      match parse_unary f rest with
      | Some (e, r) => Some (ENeg e, r)
      | None => None
      end
  | TPlus :: rest =>
      match parse_unary f rest with
      | Some (e, r) => Some (EPos e, r)
      | None => None
      end
  | TTilde :: rest =>
      match parse_unary f rest with
      | Some (e, r) => Some (EBitNot e, r)
      | None => None
      end
  | _ => parse_atom f ts
  end.
Proof. reflexivity. Qed.

Lemma parse_atom_unfold : forall f ts,
  parse_atom (S f) ts =
  match ts with
  | TNum :: rest => Some (ENum, rest)
  | TTrue :: rest => Some (ETrueLit, rest)
  | TFalse :: rest => Some (EFalseLit, rest)
  | TLParen :: rest =>
      match parse_bool_or f rest with
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
     parse_bool_or fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bool_or fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_bool_or_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bool_or_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_bool_and fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bool_and fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_bool_and_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bool_and_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_bool_not fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bool_not fuel' ts = Some (e, r))
  /\
  (forall ts e r,
     parse_comparison fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_comparison fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_comparison_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_comparison_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_bit_or fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bit_or fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_bit_or_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bit_or_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_bit_xor fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bit_xor fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_bit_xor_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bit_xor_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_bit_and fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bit_and fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_bit_and_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_bit_and_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_shift fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_shift fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_shift_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_shift_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_add fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_add fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_add_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_add_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_mul fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_mul fuel' ts = Some (e, r))
  /\
  (forall lhs ts e r,
     parse_mul_tail fuel lhs ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_mul_tail fuel' lhs ts = Some (e, r))
  /\
  (forall ts e r,
     parse_exp fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_exp fuel' ts = Some (e, r))
  /\
  (forall ts e r,
     parse_unary fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_unary fuel' ts = Some (e, r))
  /\
  (forall ts e r,
     parse_atom fuel ts = Some (e, r) ->
     forall fuel', fuel <= fuel' ->
     parse_atom fuel' ts = Some (e, r)).
Proof.
  induction fuel as [| f IH].
  - repeat split; intros; simpl in *; discriminate.
  - destruct IH as [IHbor [IHbort [IHba [IHbat [IHbn [IHcmp [IHcmpt
      [IHbitor [IHbitort [IHbitxor [IHbitxort [IHbitand [IHbitandt
      [IHshift [IHshiftt [IHadd [IHaddt [IHmul [IHmult
      [IHexp [IHunary IHatom]]]]]]]]]]]]]]]]]]]]].
    repeat split; intros.
    + (* parse_bool_or *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bool_or_unfold in H |- *.
      destruct (parse_bool_and f ts) as [[lhs rest]|] eqn:Hba; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHba _ _ _ Hba _ Hle).
      eapply IHbort; eauto.
    + (* parse_bool_or_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bool_or_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H.
      (* TOr *)
      destruct (parse_bool_and f ts') as [[rhs rest']|] eqn:Hba; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHba _ _ _ Hba _ Hle).
      eapply IHbort; eauto.
    + (* parse_bool_and *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bool_and_unfold in H |- *.
      destruct (parse_bool_not f ts) as [[lhs rest]|] eqn:Hbn; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHbn _ _ _ Hbn _ Hle).
      eapply IHbat; eauto.
    + (* parse_bool_and_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bool_and_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H.
      (* TAnd *)
      destruct (parse_bool_not f ts') as [[rhs rest']|] eqn:Hbn; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHbn _ _ _ Hbn _ Hle).
      eapply IHbat; eauto.
    + (* parse_bool_not *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bool_not_unfold in H |- *.
      destruct ts as [|t ts']; [eapply IHcmp; eauto; lia|].
      destruct t; try (eapply IHcmp; eauto; lia).
      (* TNot *)
      destruct (parse_bool_not f ts') as [[e0 rest']|] eqn:Hbn; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHbn _ _ _ Hbn _ Hle). exact H.
    + (* parse_comparison *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_comparison_unfold in H |- *.
      destruct (parse_bit_or f ts) as [[lhs rest]|] eqn:Hbo; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHbitor _ _ _ Hbo _ Hle).
      eapply IHcmpt; eauto.
    + (* parse_comparison_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_comparison_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H;
      (* All 6 comparison operators *)
      (destruct (parse_bit_or f ts') as [[rhs rest']|] eqn:Hbo; [|discriminate];
       assert (Hle : f <= f') by lia;
       rewrite (IHbitor _ _ _ Hbo _ Hle);
       eapply IHcmpt; eauto).
    + (* parse_bit_or *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bit_or_unfold in H |- *.
      destruct (parse_bit_xor f ts) as [[lhs rest]|] eqn:Hbx; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHbitxor _ _ _ Hbx _ Hle).
      eapply IHbitort; eauto.
    + (* parse_bit_or_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bit_or_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H.
      (* TPipe *)
      destruct (parse_bit_xor f ts') as [[rhs rest']|] eqn:Hbx; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHbitxor _ _ _ Hbx _ Hle).
      eapply IHbitort; eauto.
    + (* parse_bit_xor *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bit_xor_unfold in H |- *.
      destruct (parse_bit_and f ts) as [[lhs rest]|] eqn:Hband; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHbitand _ _ _ Hband _ Hle).
      eapply IHbitxort; eauto.
    + (* parse_bit_xor_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bit_xor_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H.
      (* TCaret *)
      destruct (parse_bit_and f ts') as [[rhs rest']|] eqn:Hband; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHbitand _ _ _ Hband _ Hle).
      eapply IHbitxort; eauto.
    + (* parse_bit_and *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bit_and_unfold in H |- *.
      destruct (parse_shift f ts) as [[lhs rest]|] eqn:Hsh; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHshift _ _ _ Hsh _ Hle).
      eapply IHbitandt; eauto.
    + (* parse_bit_and_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_bit_and_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H.
      (* TAmpersand *)
      destruct (parse_shift f ts') as [[rhs rest']|] eqn:Hsh; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHshift _ _ _ Hsh _ Hle).
      eapply IHbitandt; eauto.
    + (* parse_shift *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_shift_unfold in H |- *.
      destruct (parse_add f ts) as [[lhs rest]|] eqn:Hadd; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHadd _ _ _ Hadd _ Hle).
      eapply IHshiftt; eauto.
    + (* parse_shift_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_shift_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H;
      (* TLShift, TRShift *)
      (destruct (parse_add f ts') as [[rhs rest']|] eqn:Hadd; [|discriminate];
       assert (Hle : f <= f') by lia;
       rewrite (IHadd _ _ _ Hadd _ Hle);
       eapply IHshiftt; eauto).
    + (* parse_add *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_add_unfold in H |- *.
      destruct (parse_mul f ts) as [[lhs rest]|] eqn:Hmul; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHmul _ _ _ Hmul _ Hle).
      eapply IHaddt; eauto.
    + (* parse_add_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_add_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H;
      (* TPlus, TMinus *)
      (destruct (parse_mul f ts') as [[rhs rest']|] eqn:Hmul; [|discriminate];
       assert (Hle : f <= f') by lia;
       rewrite (IHmul _ _ _ Hmul _ Hle);
       eapply IHaddt; eauto).
    + (* parse_mul *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_mul_unfold in H |- *.
      destruct (parse_exp f ts) as [[lhs rest]|] eqn:Hexp; [|discriminate].
      assert (Hle : f <= f') by lia.
      rewrite (IHexp _ _ _ Hexp _ Hle).
      eapply IHmult; eauto.
    + (* parse_mul_tail *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_mul_tail_unfold in H |- *.
      destruct ts as [|t ts']; [exact H|].
      destruct t; try exact H;
      (* TMul, TDiv, TFloorDiv, TMod *)
      (destruct (parse_exp f ts') as [[rhs rest']|] eqn:Hexp; [|discriminate];
       assert (Hle : f <= f') by lia;
       rewrite (IHexp _ _ _ Hexp _ Hle);
       eapply IHmult; eauto).
    + (* parse_exp *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_exp_unfold in H |- *.
      destruct (parse_unary f ts) as [[lhs [|t rest]]|] eqn:Hu.
      * (* Some (lhs, []) *)
        assert (Hle : f <= f') by lia.
        rewrite (IHunary _ _ _ Hu _ Hle). exact H.
      * (* Some (lhs, t :: rest) *)
        destruct t; try (assert (Hle : f <= f') by lia;
          rewrite (IHunary _ _ _ Hu _ Hle); exact H).
        (* TStarStar *)
        destruct (parse_exp f rest) as [[rhs rest']|] eqn:Hexp; [|discriminate].
        assert (Hle : f <= f') by lia.
        rewrite (IHunary _ _ _ Hu _ Hle).
        rewrite (IHexp _ _ _ Hexp _ Hle). exact H.
      * (* None *)
        discriminate.
    + (* parse_unary *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_unary_unfold in H |- *.
      destruct ts as [|t ts']; [eapply IHatom; eauto; lia|].
      destruct t; try (eapply IHatom; eauto; lia);
      (* TMinus, TPlus, TTilde *)
      (destruct (parse_unary f ts') as [[e0 rest']|] eqn:Hu; [|discriminate];
       assert (Hle : f <= f') by lia;
       rewrite (IHunary _ _ _ Hu _ Hle); exact H).
    + (* parse_atom *)
      destruct fuel' as [|f']; [lia|].
      rewrite parse_atom_unfold in H |- *.
      destruct ts as [|t ts']; [discriminate|].
      destruct t; try discriminate; try exact H.
      (* TLParen *)
      destruct (parse_bool_or f ts') as [[e' [|t' rest'']]|] eqn:Hbor;
        try discriminate.
      destruct t'; try discriminate.
      assert (Hle : f <= f') by lia.
      rewrite (IHbor _ _ _ Hbor _ Hle). exact H.
Qed.


(* ================================================================ *)
(* COROLLARIES                                                        *)
(* ================================================================ *)

Corollary parse_bool_or_mono : forall fuel fuel' ts e r,
  parse_bool_or fuel ts = Some (e, r) -> fuel <= fuel' ->
  parse_bool_or fuel' ts = Some (e, r).
Proof. intros. eapply (proj1 (fuel_mono fuel)); eauto. Qed.

Lemma parse_full_consumes_all : forall fuel ts e,
  parse_full fuel ts = Some e ->
  parse_bool_or fuel ts = Some (e, []).
Proof.
  unfold parse_full. intros fuel ts e.
  destruct (parse_bool_or fuel ts) as [[e' [|t r']]|].
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
  rewrite (parse_bool_or_mono _ _ _ _ _ Hpf Hle). reflexivity.
Qed.


(* ================================================================ *)
(* COMPARISON CHAIN DESUGARING                                        *)
(*                                                                    *)
(* The parser produces left-nested comparison trees (a < b <= c       *)
(* becomes ELe (ELt a b) c).  Catnip's runtime desugars these into   *)
(* conjunction chains.  We define extraction and prove properties.    *)
(* ================================================================ *)

(* Extract a comparison chain from a left-nested comparison AST.
   Returns the leftmost non-comparison base and the operator chain. *)
Fixpoint extract_chain (e : expr) : option (expr * list (comp_op * expr)) :=
  match e with
  | ELt l r => match extract_chain l with
                | Some (base, ops) => Some (base, ops ++ [(OpLt, r)])
                | None => Some (l, [(OpLt, r)])
                end
  | ELe l r => match extract_chain l with
                | Some (base, ops) => Some (base, ops ++ [(OpLe, r)])
                | None => Some (l, [(OpLe, r)])
                end
  | EGt l r => match extract_chain l with
                | Some (base, ops) => Some (base, ops ++ [(OpGt, r)])
                | None => Some (l, [(OpGt, r)])
                end
  | EGe l r => match extract_chain l with
                | Some (base, ops) => Some (base, ops ++ [(OpGe, r)])
                | None => Some (l, [(OpGe, r)])
                end
  | EEq l r => match extract_chain l with
                | Some (base, ops) => Some (base, ops ++ [(OpEq, r)])
                | None => Some (l, [(OpEq, r)])
                end
  | ENe l r => match extract_chain l with
                | Some (base, ops) => Some (base, ops ++ [(OpNe, r)])
                | None => Some (l, [(OpNe, r)])
                end
  | _ => None
  end.

(* Non-comparison expressions yield None *)
Lemma extract_chain_non_cmp :
  forall e,
    (match e with
     | ELt _ _ | ELe _ _ | EGt _ _ | EGe _ _ | EEq _ _ | ENe _ _ => False
     | _ => True
     end) ->
    extract_chain e = None.
Proof.
  intros e H. destruct e; simpl; try reflexivity; contradiction.
Qed.

(* Extraction always produces a non-empty chain *)
Lemma extract_chain_nonempty :
  forall e base ops,
    extract_chain e = Some (base, ops) ->
    ops <> [].
Proof.
  intros e. induction e; intros base ops H; simpl in H; try discriminate;
  (destruct (extract_chain e1) as [[b' ops']|];
   [injection H as <- <-;
    intro Habs; destruct ops';
    [apply app_eq_nil in Habs; destruct Habs; discriminate
    |discriminate]
   |injection H as <- <-; discriminate]).
Qed.

(* The base of an extracted chain is never itself a comparison *)
Lemma extract_chain_base_non_cmp :
  forall e base ops,
    extract_chain e = Some (base, ops) ->
    extract_chain base = None.
Proof.
  intros e. induction e; intros base ops H; simpl in H; try discriminate;
  (destruct (extract_chain e1) as [[b' ops']|] eqn:He1;
   [injection H as <- <-; eapply IHe1; eauto
   |injection H as <- <-; exact He1]).
Qed.

(* Concrete examples *)
Theorem chain_extract_two :
  extract_chain (ELe (ELt ENum ENum) ENum) =
  Some (ENum, [(OpLt, ENum); (OpLe, ENum)]).
Proof. reflexivity. Qed.

Theorem chain_extract_three :
  extract_chain (EGt (ELe (ELt ENum ETrueLit) EFalseLit) ENum) =
  Some (ENum, [(OpLt, ETrueLit); (OpLe, EFalseLit); (OpGt, ENum)]).
Proof. reflexivity. Qed.

(* Simple evaluator for chain semantics (atoms to nats for demo) *)
Definition eval_nat (e : expr) : nat :=
  match e with
  | ENum => 0
  | ETrueLit => 1
  | EFalseLit => 0
  | _ => 0
  end.

Definition eval_extracted_chain (base : expr) (ops : list (comp_op * expr)) : bool :=
  eval_comp_chain (eval_nat base) (map (fun p => (fst p, eval_nat (snd p))) ops).

(* A single comparison's chain evaluates the same as direct evaluation *)
Theorem chain_desugar_correct_single :
  forall l r op,
    (match l with
     | ELt _ _ | ELe _ _ | EGt _ _ | EGe _ _ | EEq _ _ | ENe _ _ => False
     | _ => True
     end) ->
    eval_extracted_chain l [(op, r)] = eval_comp_op op (eval_nat l) (eval_nat r).
Proof.
  intros l r op Hl.
  unfold eval_extracted_chain. simpl.
  rewrite andb_true_r. reflexivity.
Qed.

(* Two-comparison chain evaluates as conjunction of individual comparisons *)
Theorem chain_desugar_correct_two :
  forall a b c op1 op2,
    eval_extracted_chain a [(op1, b); (op2, c)] =
    andb (eval_comp_op op1 (eval_nat a) (eval_nat b))
         (eval_comp_op op2 (eval_nat b) (eval_nat c)).
Proof.
  intros. unfold eval_extracted_chain. simpl.
  rewrite andb_true_r. reflexivity.
Qed.
