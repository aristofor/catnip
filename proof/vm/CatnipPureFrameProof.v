(* FILE: proof/vm/CatnipPureFrameProof.v *)
(* CatnipPureFrameProof.v - PureFrame bind_args and pool correctness
 *
 * Source of truth:
 *   catnip_vm/src/vm/frame.rs
 *
 * Extends CatnipVMFrame.v with properties specific to PureFrame:
 *   - bind_args: positional argument binding (no kwargs)
 *   - Default parameter filling
 *   - Pool alloc/free invariants
 *
 * 10 theorems, 0 Admitted.
 *)

From Coq Require Import List Lia PeanoNat Bool.
Import ListNotations.
Open Scope nat_scope.


(* ================================================================ *)
(* A. Model                                                           *)
(*                                                                    *)
(* PureFrame.bind_args(args):                                         *)
(*   1. Copy args[0..min(n, nparams)] to locals[0..min(n, nparams)] *)
(*   2. For i in max(n, default_start)..nparams:                     *)
(*        if locals[i] is nil: locals[i] = defaults[i - default_start] *)
(*   where default_start = nparams - ndefaults                       *)
(*                                                                    *)
(* No kwargs, no varargs in the pure model.                           *)
(* ================================================================ *)

Definition Nil : nat := 0.

(* Function signature *)
Record FunSig := mkSig {
  sig_nparams  : nat;            (* number of parameters *)
  sig_defaults : list nat;       (* default values, right-aligned *)
}.

(* Locals as a list of nats *)
Definition Locals := list nat.

(* Initial locals: all Nil *)
Definition init_locals (n : nat) : Locals := repeat Nil n.

(* Copy args into locals[0..min(n_args, nparams)] *)
Definition copy_args (args : list nat) (nparams : nat) (locs : Locals) : Locals :=
  let n := min (length args) nparams in
  firstn n args ++ skipn n locs.

(* Fill defaults for unbound slots *)
Fixpoint fill_defaults_aux (locs : Locals) (defs : list nat) (start idx : nat) : Locals :=
  match defs with
  | [] => locs
  | d :: ds =>
      let slot := start + idx in
      let locs' :=
        if slot <? length locs then
          if Nat.eqb (nth slot locs Nil) Nil then
            firstn slot locs ++ [d] ++ skipn (S slot) locs
          else locs
        else locs
      in
      fill_defaults_aux locs' ds start (S idx)
  end.

Definition fill_defaults (locs : Locals) (sig : FunSig) (n_args : nat) : Locals :=
  let nparams := sig_nparams sig in
  let ndefaults := length (sig_defaults sig) in
  let default_start := nparams - ndefaults in
  fill_defaults_aux locs (sig_defaults sig) default_start 0.

(* Full bind_args *)
Definition bind_args (args : list nat) (sig : FunSig) (nlocals : nat) : Locals :=
  let locs := init_locals nlocals in
  let locs := copy_args args (sig_nparams sig) locs in
  fill_defaults locs sig (length args).


(* ================================================================ *)
(* B. copy_args properties                                            *)
(* ================================================================ *)

(* copy_args preserves locals length *)
Theorem copy_args_length : forall args nparams locs,
  length locs >= nparams ->
  length (copy_args args nparams locs) = length locs.
Proof.
  intros args nparams locs Hge.
  unfold copy_args.
  rewrite length_app, length_firstn, length_skipn.
  lia.
Qed.

(* Bound args land in the correct slot *)
Theorem copy_args_slot_bound : forall args nparams locs i,
  i < min (length args) nparams ->
  length locs >= nparams ->
  nth i (copy_args args nparams locs) Nil = nth i args Nil.
Proof.
  intros args nparams locs i Hi Hlen.
  unfold copy_args.
  rewrite app_nth1 by (rewrite length_firstn; lia).
  rewrite nth_firstn.
  replace (i <? _) with true by (symmetry; apply Nat.ltb_lt; lia).
  reflexivity.
Qed.

(* Unbound slots remain Nil after copy_args on fresh locals *)
Theorem copy_args_unbound_nil : forall args nparams nlocals i,
  min (length args) nparams <= i ->
  i < nlocals ->
  nlocals >= nparams ->
  nth i (copy_args args nparams (init_locals nlocals)) Nil = Nil.
Proof.
  intros args nparams nlocals i Hge Hi Hnl.
  unfold copy_args, init_locals.
  rewrite app_nth2 by (rewrite length_firstn; lia).
  rewrite length_firstn.
  rewrite nth_skipn.
  apply nth_repeat.
Qed.


(* ================================================================ *)
(* C. bind_args end-to-end                                            *)
(* ================================================================ *)

(* Splice preserves length when index is in bounds *)
Lemma splice_length : forall (locs : list nat) slot v,
  slot < length locs ->
  length (firstn slot locs ++ [v] ++ skipn (S slot) locs) = length locs.
Proof.
  intros locs slot v Hlt.
  rewrite !length_app, firstn_length_le by lia.
  rewrite length_skipn. simpl. lia.
Qed.

(* fill_defaults_aux preserves length *)
Lemma fill_defaults_aux_length : forall defs locs start idx,
  length (fill_defaults_aux locs defs start idx) = length locs.
Proof.
  induction defs as [|d ds IHds]; intros locs start idx.
  - reflexivity.
  - simpl.
    destruct (start + idx <? length locs) eqn:Elt.
    + destruct (Nat.eqb (nth (start + idx) locs Nil) Nil) eqn:Eeq.
      * rewrite IHds. apply splice_length. apply Nat.ltb_lt. exact Elt.
      * apply IHds.
    + apply IHds.
Qed.

(* bind_args preserves nlocals length *)
Theorem bind_args_length : forall args sig nlocals,
  nlocals >= sig_nparams sig ->
  length (bind_args args sig nlocals) = nlocals.
Proof.
  intros args sig nlocals Hge.
  unfold bind_args, fill_defaults.
  rewrite fill_defaults_aux_length.
  rewrite copy_args_length.
  - unfold init_locals. rewrite repeat_length. reflexivity.
  - unfold init_locals. rewrite repeat_length. lia.
Qed.

(* With no defaults, bind_args = copy_args on fresh locals *)
Theorem bind_args_no_defaults : forall args nparams nlocals i,
  nlocals >= nparams ->
  i < min (length args) nparams ->
  nth i (bind_args args (mkSig nparams []) nlocals) Nil = nth i args Nil.
Proof.
  intros args nparams nlocals i Hnl Hi.
  unfold bind_args, fill_defaults. simpl.
  apply copy_args_slot_bound.
  - exact Hi.
  - unfold init_locals. rewrite repeat_length. lia.
Qed.


(* ================================================================ *)
(* D. Pool invariants                                                 *)
(* ================================================================ *)

(* Model: pool is a list of reusable frames *)
Definition Pool := list Locals.

Definition pool_alloc (pool : Pool) (nlocals : nat) : Locals * Pool :=
  match pool with
  | [] => (init_locals nlocals, [])
  | _ :: rest => (init_locals nlocals, rest)  (* reset reused frame *)
  end.

Definition pool_free (locs : Locals) (pool : Pool) (max_size : nat) : Pool :=
  if length pool <? max_size then
    init_locals (length locs) :: pool  (* reset before pooling *)
  else pool.

(* Pool alloc always produces fresh locals *)
Theorem pool_alloc_fresh : forall pool nlocals locs pool',
  pool_alloc pool nlocals = (locs, pool') ->
  locs = init_locals nlocals.
Proof.
  intros pool nlocals locs pool' H.
  destruct pool; simpl in H; inversion H; reflexivity.
Qed.

(* Pool alloc fresh locals are all Nil *)
Theorem pool_alloc_all_nil : forall pool nlocals locs pool' i,
  pool_alloc pool nlocals = (locs, pool') ->
  i < nlocals ->
  nth i locs Nil = Nil.
Proof.
  intros pool nlocals locs pool' i Halloc Hi.
  rewrite (pool_alloc_fresh _ _ _ _ Halloc).
  unfold init_locals. apply nth_repeat.
Qed.

(* Pool size is bounded by max_size *)
Theorem pool_free_bounded : forall locs pool max_size,
  length pool <= max_size ->
  length (pool_free locs pool max_size) <= max_size.
Proof.
  intros locs pool max_size Hle.
  unfold pool_free.
  destruct (length pool <? max_size) eqn:E.
  - simpl. apply Nat.ltb_lt in E. lia.
  - exact Hle.
Qed.

(* Pool free then alloc produces a clean frame *)
Theorem pool_round_trip : forall locs pool max_size nlocals locs' pool',
  length pool < max_size ->
  pool_alloc (pool_free locs pool max_size) nlocals = (locs', pool') ->
  locs' = init_locals nlocals.
Proof.
  intros locs pool max_size nlocals locs' pool' Hlt Halloc.
  unfold pool_free in Halloc.
  replace (length pool <? max_size) with true in Halloc
    by (symmetry; apply Nat.ltb_lt; exact Hlt).
  simpl in Halloc. inversion Halloc. reflexivity.
Qed.
