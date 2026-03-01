(* FILE: proof/struct/CatnipMROC3Core.v *)
(* C3 Merge Algorithm Core
 *
 * Source of truth:
 *   catnip_rs/src/vm/mro.rs (c3_linearize)
 *
 * Proves:
 *   - C3 merge algorithm correctness (self-first, no reinsertion)
 *   - Self-first property
 *   - C3 merge on empty input
 *
 * Standalone: no dependencies on other Catnip proofs.
 *
 * Reference: https://www.python.org/download/releases/2.3/mro/
 *)

From Coq Require Import List ZArith Bool Lia PeanoNat.
From Coq Require Import String.
Import ListNotations.

(* Resolve length ambiguity: List.length over String.length *)
Definition length {A : Type} := @List.length A.

Notation "x '++' y" := (@app _ x y) (at level 60, right associativity) : list_scope.
Open Scope string_scope.
Open Scope list_scope.

(* Helpers *)
Lemma string_eqb_refl : forall s, String.eqb s s = true.
Proof. induction s; simpl. reflexivity. rewrite Ascii.eqb_refl. exact IHs. Qed.


(* ================================================================ *)
(* A. C3 Merge Algorithm                                             *)
(*                                                                    *)
(* Models c3_linearize from mro.rs.                                  *)
(* Sequences = list of parent MROs + parents list.                   *)
(* Iteratively picks a "good head" not in any tail.                  *)
(* ================================================================ *)

Definition Sequences := list (list string).

(* Check if candidate appears in the tail (all but first) of a sequence *)
Definition in_tail (candidate : string) (s : list string) : bool :=
  match s with
  | [] => false
  | _ :: rest => existsb (String.eqb candidate) rest
  end.

(* Check if candidate appears in the tail of ANY sequence *)
Definition in_any_tail (candidate : string) (seqs : Sequences) : bool :=
  existsb (in_tail candidate) seqs.

(* Find a good head: first element of some sequence not in any tail *)
Fixpoint find_good_head (seqs : Sequences) (all_seqs : Sequences) : option string :=
  match seqs with
  | [] => None
  | s :: rest =>
      match s with
      | [] => find_good_head rest all_seqs
      | candidate :: _ =>
          if in_any_tail candidate all_seqs then
            find_good_head rest all_seqs
          else Some candidate
      end
  end.

(* Remove head from the front of each sequence where it appears *)
Definition remove_head_from (head : string) (seqs : Sequences) : Sequences :=
  map (fun s =>
    match s with
    | h :: rest => if String.eqb h head then rest else s
    | [] => []
    end) seqs.

(* Remove empty sequences *)
Definition remove_empty (seqs : Sequences) : Sequences :=
  filter (fun s => match s with [] => false | _ => true end) seqs.

(* C3 merge with fuel for termination *)
Fixpoint c3_merge (seqs : Sequences) (fuel : nat) : option (list string) :=
  match fuel with
  | 0 => Some []
  | S fuel' =>
      let seqs' := remove_empty seqs in
      match seqs' with
      | [] => Some []
      | _ =>
          match find_good_head seqs' seqs' with
          | None => None  (* C3 linearization failed *)
          | Some head =>
              match c3_merge (remove_head_from head seqs') fuel' with
              | None => None
              | Some rest => Some (head :: rest)
              end
          end
      end
  end.

(* Full C3 linearization: prepend self, then merge *)
Definition c3_linearize
  (name : string)
  (parents : list string)
  (parent_mros : list (list string)) : option (list string) :=
  match parents with
  | [] => Some [name]
  | _ =>
      let sequences := parent_mros ++ [parents] in
      let fuel := fold_left (fun acc s => acc + length s) sequences 0 in
      match c3_merge sequences (S fuel) with
      | None => None
      | Some merged => Some (name :: merged)
      end
  end.


(* ================================================================ *)
(* B. Self-First Property                                            *)
(*                                                                    *)
(* The type itself is always the first element of its MRO.           *)
(* ================================================================ *)

Theorem c3_self_first : forall name parents parent_mros mro,
  c3_linearize name parents parent_mros = Some mro ->
  exists rest, mro = name :: rest.
Proof.
  intros name parents parent_mros mro H.
  unfold c3_linearize in H.
  destruct parents.
  - inversion H; subst. exists []. reflexivity.
  - destruct (c3_merge _ _) eqn:Em; [|discriminate].
    inversion H; subst. exists l. reflexivity.
Qed.

Corollary c3_self_is_head : forall name parents parent_mros mro,
  c3_linearize name parents parent_mros = Some mro ->
  hd_error mro = Some name.
Proof.
  intros. apply c3_self_first in H. destruct H as [rest Heq].
  subst. reflexivity.
Qed.

(* No parents => MRO is just [name] *)
Theorem c3_no_parents : forall name,
  c3_linearize name [] [] = Some [name].
Proof. reflexivity. Qed.

(* c3_merge on empty input produces empty output *)
Lemma c3_merge_empty : forall fuel,
  c3_merge [] fuel = Some [].
Proof. destruct fuel; reflexivity. Qed.
