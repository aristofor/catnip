(* FILE: proof/lang/CatnipScopeProof.v *)
(* CatnipScopeProof.v — Formal model of Catnip's scope resolution
 *
 * Source of truth: catnip_rs/src/core/scope.rs
 *
 * Models the scope chain as a stack of environments with:
 *   - O(1) lookup (most recent binding wins)
 *   - push/pop frame semantics
 *   - shadowing with restoration on pop
 *
 * Parametric in value type V (proofs hold for any value domain).
 *)

From Coq Require Import List String Bool Lia.
Import ListNotations.


(* ================================================================ *)
(* A. Environment (single frame)                                      *)
(*                                                                    *)
(* Association list: head = most recent binding.                       *)
(* Matches the HashMap semantics where latest insert wins.            *)
(* ================================================================ *)

Section WithValue.

Variable V : Type.

Definition Env := list (string * V).

Fixpoint env_lookup (e : Env) (name : string) : option V :=
  match e with
  | [] => None
  | (k, v) :: rest =>
      if String.eqb k name then Some v
      else env_lookup rest name
  end.

Definition env_contains (e : Env) (name : string) : bool :=
  match env_lookup e name with
  | Some _ => true
  | None => false
  end.

Definition env_set (e : Env) (name : string) (v : V) : Env :=
  (name, v) :: e.


(* ================================================================ *)
(* B. Scope (stack of frames)                                         *)
(*                                                                    *)
(* Head = innermost frame. Lookup searches top-down.                  *)
(* Matches scope.rs push_frame / pop_frame / resolve.                 *)
(* ================================================================ *)

Definition Scope := list Env.

Fixpoint scope_lookup (sc : Scope) (name : string) : option V :=
  match sc with
  | [] => None
  | frame :: rest =>
      match env_lookup frame name with
      | Some v => Some v
      | None => scope_lookup rest name
      end
  end.

Definition scope_empty : Scope := [[]].

Definition scope_push (sc : Scope) : Scope := [] :: sc.

Definition scope_pop (sc : Scope) : Scope :=
  match sc with
  | [] => []
  | _ :: rest => rest
  end.

Definition scope_set (sc : Scope) (name : string) (v : V) : Scope :=
  match sc with
  | [] => [[(name, v)]]
  | frame :: rest => ((name, v) :: frame) :: rest
  end.

Definition scope_depth (sc : Scope) : nat := List.length sc.


(* ================================================================ *)
(* C. Environment Properties                                          *)
(* ================================================================ *)

Lemma env_lookup_set_same : forall (e : Env) name (v : V),
  env_lookup (env_set e name v) name = Some v.
Proof.
  intros e name v. simpl.
  rewrite String.eqb_refl. reflexivity.
Qed.

Lemma env_lookup_set_other : forall (e : Env) name1 name2 (v : V),
  name1 <> name2 ->
  env_lookup (env_set e name1 v) name2 = env_lookup e name2.
Proof.
  intros e name1 name2 v Hne. simpl.
  destruct (String.eqb name1 name2) eqn:Heq.
  - apply String.eqb_eq in Heq. contradiction.
  - reflexivity.
Qed.

Lemma env_lookup_empty : forall name,
  env_lookup ([] : Env) name = None.
Proof. reflexivity. Qed.

Lemma env_contains_iff : forall (e : Env) name,
  env_contains e name = true <-> exists v, env_lookup e name = Some v.
Proof.
  intros e name. unfold env_contains. split.
  - destruct (env_lookup e name) as [v|]; [eauto | discriminate].
  - intros [v H]. rewrite H. reflexivity.
Qed.


(* ================================================================ *)
(* D. Scope Lookup Properties                                         *)
(* ================================================================ *)

(* Setting a variable and looking it up yields the set value *)
Theorem scope_set_lookup_same : forall (sc : Scope) name (v : V),
  scope_lookup (scope_set sc name v) name = Some v.
Proof.
  intros sc name v.
  destruct sc as [|frame rest]; simpl.
  - rewrite String.eqb_refl. reflexivity.
  - rewrite String.eqb_refl. reflexivity.
Qed.

(* Setting one variable doesn't affect lookup of another *)
Theorem scope_set_lookup_other : forall (sc : Scope) name1 name2 (v : V),
  name1 <> name2 ->
  scope_lookup (scope_set sc name1 v) name2 = scope_lookup sc name2.
Proof.
  intros sc name1 name2 v Hne.
  destruct sc as [|frame rest]; simpl.
  - destruct (String.eqb name1 name2) eqn:Heq.
    + apply String.eqb_eq in Heq. contradiction.
    + reflexivity.
  - destruct (String.eqb name1 name2) eqn:Heq.
    + apply String.eqb_eq in Heq. contradiction.
    + reflexivity.
Qed.


(* ================================================================ *)
(* E. Push / Pop Properties                                           *)
(* ================================================================ *)

(* Push then pop is identity *)
Theorem scope_push_pop : forall (sc : Scope),
  scope_pop (scope_push sc) = sc.
Proof. reflexivity. Qed.

(* Push preserves all existing lookups *)
Theorem scope_push_preserves_lookup : forall (sc : Scope) name,
  scope_lookup (scope_push sc) name = scope_lookup sc name.
Proof. reflexivity. Qed.

(* Pop removes inner frame bindings *)
Theorem scope_pop_removes_inner : forall (sc : Scope) name (v : V),
  scope_lookup sc name = None ->
  scope_lookup (scope_pop (scope_set (scope_push sc) name v)) name = None.
Proof.
  intros sc name v H.
  simpl. exact H.
Qed.

(* Push increments depth *)
Theorem scope_push_depth : forall (sc : Scope),
  scope_depth (scope_push sc) = S (scope_depth sc).
Proof. reflexivity. Qed.

(* Pop decrements depth (non-empty) *)
Theorem scope_pop_depth : forall frame (rest : Scope),
  scope_depth (scope_pop (frame :: rest)) = scope_depth rest.
Proof. reflexivity. Qed.

(* Empty scope has depth 1 *)
Theorem scope_empty_depth : scope_depth scope_empty = 1%nat.
Proof. reflexivity. Qed.


(* ================================================================ *)
(* F. Shadowing                                                       *)
(*                                                                    *)
(* Inner frame binding shadows outer frame binding.                   *)
(* After pop, outer binding is restored.                              *)
(* ================================================================ *)

(* Setting in inner frame shadows outer value *)
Theorem scope_shadowing : forall (sc : Scope) name (v_outer v_inner : V),
  scope_lookup (scope_set (scope_push (scope_set sc name v_outer)) name v_inner) name
  = Some v_inner.
Proof.
  intros. apply scope_set_lookup_same.
Qed.

(* Pop restores the outer value *)
Theorem scope_pop_restores : forall (sc : Scope) name (v_outer v_inner : V),
  scope_lookup
    (scope_pop (scope_set (scope_push (scope_set sc name v_outer)) name v_inner))
    name
  = scope_lookup (scope_set sc name v_outer) name.
Proof. reflexivity. Qed.

(* The restored value is the original *)
Corollary scope_shadow_restore : forall (sc : Scope) name (v_outer v_inner : V),
  scope_lookup
    (scope_pop (scope_set (scope_push (scope_set sc name v_outer)) name v_inner))
    name
  = Some v_outer.
Proof.
  intros. rewrite scope_pop_restores. apply scope_set_lookup_same.
Qed.

(* Shadowing doesn't affect other variables *)
Theorem scope_shadow_other : forall (sc : Scope) name1 name2 (v1 v2 : V),
  name1 <> name2 ->
  scope_lookup (scope_set (scope_push (scope_set sc name1 v1)) name2 v2) name1
  = Some v1.
Proof.
  intros sc name1 name2 v1 v2 Hne.
  assert (Hne' : name2 <> name1) by (intro H; apply Hne; symmetry; exact H).
  rewrite (scope_set_lookup_other _ _ _ _ Hne').
  rewrite scope_push_preserves_lookup.
  apply scope_set_lookup_same.
Qed.


(* ================================================================ *)
(* G. Multi-level Scoping                                             *)
(*                                                                    *)
(* Properties that hold across multiple frame levels.                 *)
(* ================================================================ *)

(* Variable set at level 0 visible through n pushes *)
Lemma scope_visible_through_pushes : forall n (sc : Scope) name (v : V),
  scope_lookup (scope_set sc name v) name = Some v ->
  scope_lookup (Nat.iter n scope_push (scope_set sc name v)) name = Some v.
Proof.
  induction n as [|n IH]; intros sc name v H.
  - exact H.
  - simpl. apply IH. exact H.
Qed.

(* Depth after n pushes *)
Lemma scope_depth_push_n : forall n (sc : Scope),
  scope_depth (Nat.iter n scope_push sc) = (n + scope_depth sc)%nat.
Proof.
  induction n as [|n IH]; intros sc.
  - reflexivity.
  - simpl. rewrite IH. reflexivity.
Qed.


(* ================================================================ *)
(* H. Frame Isolation (function scopes)                               *)
(*                                                                    *)
(* Model the isolated frame semantics from scope.rs:                  *)
(* - Non-isolated: _set updates existing variable in place            *)
(* - Isolated: _set shadows parent variable                           *)
(* ================================================================ *)

(* A frame can be transparent (block/loop) or isolated (function) *)
Inductive FrameKind := Transparent | Isolated.

(* Extended scope with frame metadata *)
Definition ScopeEx := list (FrameKind * Env).

Fixpoint scopeex_lookup (sc : ScopeEx) (name : string) : option V :=
  match sc with
  | [] => None
  | (_, frame) :: rest =>
      match env_lookup frame name with
      | Some v => Some v
      | None => scopeex_lookup rest name
      end
  end.

Definition scopeex_push (sc : ScopeEx) (kind : FrameKind) : ScopeEx :=
  (kind, []) :: sc.

Definition scopeex_pop (sc : ScopeEx) : ScopeEx :=
  match sc with
  | [] => []
  | _ :: rest => rest
  end.

Definition scopeex_set (sc : ScopeEx) (name : string) (v : V) : ScopeEx :=
  match sc with
  | [] => [(Transparent, [(name, v)])]
  | (kind, frame) :: rest => (kind, (name, v) :: frame) :: rest
  end.

(* Push/pop identity holds for extended scopes *)
Theorem scopeex_push_pop : forall (sc : ScopeEx) kind,
  scopeex_pop (scopeex_push sc kind) = sc.
Proof. reflexivity. Qed.

(* Push preserves lookup for extended scopes *)
Theorem scopeex_push_preserves : forall (sc : ScopeEx) name kind,
  scopeex_lookup (scopeex_push sc kind) name = scopeex_lookup sc name.
Proof. reflexivity. Qed.

(* Isolated frame shadows parent on set *)
Theorem scopeex_isolated_shadow : forall (sc : ScopeEx) name (v_out v_in : V),
  scopeex_lookup
    (scopeex_set (scopeex_push (scopeex_set sc name v_out) Isolated) name v_in)
    name
  = Some v_in.
Proof.
  intros sc name v_out v_in.
  destruct sc as [|[k f] rest]; simpl; rewrite String.eqb_refl; reflexivity.
Qed.

(* Pop isolated frame restores parent value *)
Theorem scopeex_isolated_restore : forall (sc : ScopeEx) name (v_out v_in : V),
  scopeex_lookup
    (scopeex_pop (scopeex_set (scopeex_push (scopeex_set sc name v_out) Isolated) name v_in))
    name
  = Some v_out.
Proof.
  intros sc name v_out v_in.
  destruct sc as [|[k f] rest]; simpl; rewrite String.eqb_refl; reflexivity.
Qed.

End WithValue.

Arguments env_lookup {V}.
Arguments env_set {V}.
Arguments env_contains {V}.
Arguments scope_lookup {V}.
Arguments scope_empty {V}.
Arguments scope_push {V}.
Arguments scope_pop {V}.
Arguments scope_set {V}.
Arguments scope_depth {V}.
Arguments scopeex_lookup {V}.
Arguments scopeex_push {V}.
Arguments scopeex_pop {V}.
Arguments scopeex_set {V}.


(* ================================================================ *)
(* I. Concrete Scope Examples                                         *)
(*                                                                    *)
(* Using nat as value type for executable tests.                      *)
(* ================================================================ *)

(* Push, set two vars, check both visible *)
Example ex_basic_scope :
  let sc := scope_set (scope_set (scope_empty) "x" 10) "y" 20 in
  scope_lookup sc "x" = Some 10 /\ scope_lookup sc "y" = Some 20.
Proof. split; reflexivity. Qed.

(* Shadow and restore *)
Example ex_shadow_restore :
  let sc0 := scope_set scope_empty "a" 1 in
  let sc1 := scope_set (scope_push sc0) "a" 2 in
  scope_lookup sc1 "a" = Some 2 /\
  scope_lookup (scope_pop sc1) "a" = Some 1.
Proof. split; reflexivity. Qed.

(* Three levels with different variables *)
Example ex_three_levels :
  let sc0 := scope_set scope_empty "a" 1 in
  let sc1 := scope_set (scope_push sc0) "b" 2 in
  let sc2 := scope_set (scope_push sc1) "c" 3 in
  scope_lookup sc2 "a" = Some 1 /\
  scope_lookup sc2 "b" = Some 2 /\
  scope_lookup sc2 "c" = Some 3 /\
  scope_lookup (scope_pop sc2) "c" = None /\
  scope_lookup (scope_pop (scope_pop sc2)) "b" = None.
Proof. repeat split; reflexivity. Qed.

(* Missing variable *)
Example ex_missing :
  scope_lookup (scope_empty (V:=nat)) "ghost" = None.
Proof. reflexivity. Qed.

(* Isolated frame example *)
Example ex_isolated_frame :
  let sc0 := scopeex_set ([(Transparent, [])] : ScopeEx nat) "x" 10 in
  let sc1 := scopeex_set (scopeex_push sc0 Isolated) "x" 99 in
  scopeex_lookup sc1 "x" = Some 99 /\
  scopeex_lookup (scopeex_pop sc1) "x" = Some 10.
Proof. split; reflexivity. Qed.
