(* FILE: proof/analysis/CatnipTrivialPhiProof.v *)
(* Trivial-phi resolution for versioned SSA destruction.
 *
 * Source: catnip_core/src/cfg/ssa_destruction.rs  (canonical)
 *
 * The SSA builder eliminates a phi whose incoming operands all agree (Braun et
 * al. 2013, tryRemoveTrivialPhi) but leaves the value in `value_defs`, where a
 * stale `instruction_uses` entry may still reference it. Versioned destruction
 * must give that stale use the same name as the surviving definition, else it
 * names a value nothing materializes (the `i__v2` bug the execution oracle
 * caught on nested loops). `canonical` follows the resolution chain; this file
 * proves the chain is value-preserving to any depth -- the universal guarantee
 * the finite oracle (if/else/while/for/nested) samples but cannot cover.
 *
 * Proves:
 *   - trivial_phi_value   : a trivial phi denotes its common operand
 *   - resolve_preserves   : canonical resolution preserves the runtime value
 *   - trivial_phi_all_same, resolve_chain_value : instances
 *
 * Depends on: Coq stdlib only.
 *
 * 0 Admitted.
 *)

From Coq Require Import List Arith Lia.
Import ListNotations.

(* ================================================================ *)
(* A. A phi is its operand list; at runtime it takes one operand.    *)
(* ================================================================ *)

Definition all_eq (w : nat) (ops : list nat) : Prop :=
  forall x, In x ops -> x = w.

(* A trivial phi -- every operand equal to `w` -- denotes `w` whichever
   predecessor edge is taken, so replacing the phi by `w` is value-preserving.
   This is why `canonical` may collapse it. *)
Theorem trivial_phi_value : forall w ops k d,
  all_eq w ops -> k < length ops -> nth k ops d = w.
Proof.
  intros w ops k d Hall Hk.
  apply Hall. apply nth_In. exact Hk.
Qed.

(* ================================================================ *)
(* B. Canonical resolution follows a step map to a fixpoint,         *)
(*    fuel-bounded (mirrors the 64-iteration guard in `canonical`).  *)
(* ================================================================ *)

Fixpoint resolve (step : nat -> option nat) (fuel : nat) (v : nat) : nat :=
  match fuel with
  | 0 => v
  | S f =>
      match step v with
      | Some w => resolve step f w
      | None => v
      end
  end.

(* If each resolution step preserves the runtime value -- a trivial phi holds the
   same value as its replacement, by `trivial_phi_value` -- then resolving to any
   depth preserves it. Proved for every `fuel`, so even a chain cut short by the
   guard still names a value equal to the original: the rename is sound. *)
Theorem resolve_preserves : forall (env : nat -> nat) step fuel v,
  (forall a b, step a = Some b -> env a = env b) ->
  env (resolve step fuel v) = env v.
Proof.
  intros env step fuel. induction fuel as [| f IH]; intros v Hstep.
  - reflexivity.
  - simpl. destruct (step v) as [w |] eqn:E.
    + rewrite (IH w Hstep). symmetry. apply (Hstep v w E).
    + reflexivity.
Qed.

(* ================================================================ *)
(* C. Instances                                                      *)
(* ================================================================ *)

Example trivial_phi_all_same : forall d, nth 1 [7; 7; 7] d = 7.
Proof.
  intro d. apply (trivial_phi_value 7 [7; 7; 7] 1 d).
  - intros x Hin. simpl in Hin.
    destruct Hin as [H | [H | [H | H]]]; try (subst; reflexivity); contradiction.
  - simpl. lia.
Qed.

(* A 3-link chain 2 -> 1 -> 0 of trivial phis resolves to 0 with the value
   preserved, given each link agrees under `env`. *)
Definition chain_step (v : nat) : option nat :=
  match v with
  | 2 => Some 1
  | 1 => Some 0
  | _ => None
  end.

Example resolve_chain_value : forall env : nat -> nat,
  env 2 = env 1 -> env 1 = env 0 -> env (resolve chain_step 64 2) = env 2.
Proof.
  intros env H21 H10.
  apply resolve_preserves.
  intros a b Hs.
  destruct a as [| [| [| a']]]; simpl in Hs; try discriminate.
  - injection Hs as Hb. subst b. exact H10.
  - injection Hs as Hb. subst b. exact H21.
Qed.
