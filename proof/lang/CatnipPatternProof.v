(* FILE: proof/lang/CatnipPatternProof.v *)
(* CatnipPatternProof.v - Formal model of Catnip's pattern matching
 *
 * Source of truth:
 *   catnip_rs/src/core/pattern.rs       (6 pattern types, tag dispatch)
 *   catnip_rs/src/core/registry/patterns.rs (match_pattern, op_match)
 *
 * Models pattern matching as a pure function from (Pattern, Value) to
 * optional bindings. Proves determinism, completeness of wildcard/var,
 * and structural properties of OR, tuple, and struct patterns.
 *
 * Parametric in value type V with decidable equality.
 *)

From Coq Require Import List String Bool PeanoNat Lia.
From Catnip Require Import CatnipIR.
Import ListNotations.


(* ================================================================ *)
(* A. Value and Pattern types                                        *)
(*                                                                   *)
(* V = abstract value with decidable equality.                       *)
(* Pattern = 6 variants matching pattern.rs tags 0-5.                *)
(* ================================================================ *)

Section WithValue.

Variable V : Type.
Variable V_eqb : V -> V -> bool.
Hypothesis V_eqb_spec : forall a b, V_eqb a b = true <-> a = b.

(* Bindings: list of (name, value) pairs *)
Definition Bindings := list (string * V).

(* Struct value: type name + named fields *)
Record StructValue := mkStruct {
  sv_type : string;
  sv_fields : list (string * V);
}.

(* Pattern type matching pattern.rs tags *)
Inductive Pattern :=
  | PWildcard                             (* tag 0 *)
  | PLiteral (v : V)                      (* tag 1 *)
  | PVar (name : string)                  (* tag 2 *)
  | POr (pats : list Pattern)             (* tag 3 *)
  | PTuple (pats : list Pattern)          (* tag 4 *)
  | PStruct (name : string)              (* tag 5, fields extracted by name *)
              (fields : list string).


(* ================================================================ *)
(* B. Pattern matching function                                      *)
(*                                                                   *)
(* match_pattern : Pattern -> V -> option Bindings                   *)
(* Models match_pattern() from registry/patterns.rs.                 *)
(*                                                                   *)
(* Struct matching uses a helper that looks up fields in a           *)
(* StructValue. Tuple matching requires a value-to-list function.    *)
(* ================================================================ *)

(* Lookup a field in a struct value *)
Fixpoint field_lookup (fields : list (string * V)) (name : string)
    : option V :=
  match fields with
  | [] => None
  | (k, v) :: rest =>
      if String.eqb k name then Some v
      else field_lookup rest name
  end.

(* Match a single pattern against a value.
   struct_of: extract StructValue from V (partial, returns None if not a struct)
   list_of:   extract list of V from V (partial, returns None if not iterable) *)
Variable struct_of : V -> option StructValue.
Variable list_of : V -> option (list V).

Fixpoint match_pattern (p : Pattern) (v : V) : option Bindings :=
  match p with
  | PWildcard => Some []
  | PLiteral pv =>
      if V_eqb pv v then Some []
      else None
  | PVar name => Some [(name, v)]
  | POr pats =>
      (fix try_or (ps : list Pattern) : option Bindings :=
        match ps with
        | [] => None
        | p1 :: rest =>
            match match_pattern p1 v with
            | Some bs => Some bs
            | None => try_or rest
            end
        end) pats
  | PTuple pats =>
      match list_of v with
      | None => None
      | Some vs =>
          if Nat.eqb (List.length pats) (List.length vs) then
            (fix match_all (ps : list Pattern) (vals : list V)
                : option Bindings :=
              match ps, vals with
              | [], [] => Some []
              | p1 :: ps', v1 :: vs' =>
                  match match_pattern p1 v1 with
                  | Some bs =>
                      match match_all ps' vs' with
                      | Some rest => Some (bs ++ rest)
                      | None => None
                      end
                  | None => None
                  end
              | _, _ => None
              end) pats vs
          else None
      end
  | PStruct sname sfields =>
      match struct_of v with
      | None => None
      | Some sv =>
          if String.eqb (sv_type sv) sname then
            (fix extract_fields (fnames : list string)
                : option Bindings :=
              match fnames with
              | [] => Some []
              | f :: rest =>
                  match field_lookup (sv_fields sv) f with
                  | Some fv =>
                      match extract_fields rest with
                      | Some bs => Some ((f, fv) :: bs)
                      | None => None
                      end
                  | None => None
                  end
              end) sfields
          else None
      end
  end.


(* ================================================================ *)
(* C. Match dispatch (op_match)                                      *)
(*                                                                   *)
(* Sequential first-match with optional guards.                      *)
(* Models op_match() from registry/patterns.rs.                      *)
(* ================================================================ *)

(* A case: pattern, optional guard, body result *)
Record Case := mkCase {
  case_pattern : Pattern;
  case_guard : option (Bindings -> bool);  (* guard evaluated with bindings *)
  case_body : V;
}.

Fixpoint match_cases (cases : list Case) (v : V) : option (Bindings * V) :=
  match cases with
  | [] => None
  | c :: rest =>
      match match_pattern (case_pattern c) v with
      | Some bs =>
          match case_guard c with
          | None => Some (bs, case_body c)
          | Some guard =>
              if guard bs then Some (bs, case_body c)
              else match_cases rest v
          end
      | None => match_cases rest v
      end
  end.


(* ================================================================ *)
(* D. Wildcard properties                                            *)
(* ================================================================ *)

Theorem wildcard_always_matches : forall v,
  match_pattern PWildcard v = Some [].
Proof. reflexivity. Qed.

Theorem wildcard_no_bindings : forall v bs,
  match_pattern PWildcard v = Some bs -> bs = [].
Proof. intros v bs H. simpl in H. inversion H. reflexivity. Qed.


(* ================================================================ *)
(* E. Variable properties                                            *)
(* ================================================================ *)

Theorem var_always_matches : forall name v,
  match_pattern (PVar name) v = Some [(name, v)].
Proof. reflexivity. Qed.

Theorem var_captures_value : forall name v bs,
  match_pattern (PVar name) v = Some bs ->
  In (name, v) bs.
Proof.
  intros name v bs H. simpl in H. inversion H.
  left. reflexivity.
Qed.

Theorem var_single_binding : forall name v bs,
  match_pattern (PVar name) v = Some bs ->
  List.length bs = 1%nat.
Proof.
  intros name v bs H. simpl in H. inversion H. reflexivity.
Qed.


(* ================================================================ *)
(* F. Literal properties                                             *)
(* ================================================================ *)

Theorem literal_matches_equal : forall v,
  match_pattern (PLiteral v) v = Some [].
Proof.
  intros v. simpl.
  assert (H : V_eqb v v = true) by (apply V_eqb_spec; reflexivity).
  rewrite H. reflexivity.
Qed.

Theorem literal_rejects_different : forall v1 v2,
  v1 <> v2 ->
  match_pattern (PLiteral v1) v2 = None.
Proof.
  intros v1 v2 Hne. simpl.
  destruct (V_eqb v1 v2) eqn:E.
  - apply V_eqb_spec in E. contradiction.
  - reflexivity.
Qed.

Theorem literal_no_bindings : forall pv v bs,
  match_pattern (PLiteral pv) v = Some bs -> bs = [].
Proof.
  intros pv v bs H. simpl in H.
  destruct (V_eqb pv v); inversion H. reflexivity.
Qed.


(* ================================================================ *)
(* G. OR pattern properties                                          *)
(* ================================================================ *)

Theorem or_empty_fails : forall v,
  match_pattern (POr []) v = None.
Proof. reflexivity. Qed.

Theorem or_first_match_wins : forall p1 rest v bs,
  match_pattern p1 v = Some bs ->
  match_pattern (POr (p1 :: rest)) v = Some bs.
Proof.
  intros p1 rest v bs H. simpl. rewrite H. reflexivity.
Qed.

Theorem or_skips_failure : forall p1 rest v,
  match_pattern p1 v = None ->
  match_pattern (POr (p1 :: rest)) v = match_pattern (POr rest) v.
Proof.
  intros p1 rest v H. simpl. rewrite H. reflexivity.
Qed.

Theorem or_singleton : forall p v,
  match_pattern (POr [p]) v = match_pattern p v.
Proof.
  intros p v. simpl.
  destruct (match_pattern p v); reflexivity.
Qed.

(* Wildcard in OR always succeeds *)
Theorem or_with_wildcard : forall pats v,
  In PWildcard pats ->
  exists bs, match_pattern (POr pats) v = Some bs.
Proof.
  intros pats v H.
  induction pats as [|p ps IH].
  - inversion H.
  - destruct H as [Heq | Hin].
    + subst p. simpl. exists []. reflexivity.
    + simpl. destruct (match_pattern p v) as [bs|].
      * exists bs. reflexivity.
      * apply IH. exact Hin.
Qed.


(* ================================================================ *)
(* H. Tuple pattern properties                                       *)
(* ================================================================ *)

Theorem tuple_empty_matches_empty : forall v,
  list_of v = Some [] ->
  match_pattern (PTuple []) v = Some [].
Proof.
  intros v H. simpl. rewrite H. reflexivity.
Qed.

Theorem tuple_length_mismatch : forall pats v vs,
  list_of v = Some vs ->
  List.length pats <> List.length vs ->
  match_pattern (PTuple pats) v = None.
Proof.
  intros pats v vs Hlist Hlen. simpl. rewrite Hlist.
  destruct (Nat.eqb (List.length pats) (List.length vs)) eqn:E.
  - apply Nat.eqb_eq in E. contradiction.
  - reflexivity.
Qed.

Theorem tuple_not_iterable : forall pats v,
  list_of v = None ->
  match_pattern (PTuple pats) v = None.
Proof.
  intros pats v H. simpl. rewrite H. reflexivity.
Qed.


(* ================================================================ *)
(* I. Struct pattern properties                                      *)
(* ================================================================ *)

Theorem struct_not_a_struct : forall sname fields v,
  struct_of v = None ->
  match_pattern (PStruct sname fields) v = None.
Proof.
  intros sname fields v H. simpl. rewrite H. reflexivity.
Qed.

Theorem struct_type_mismatch : forall sname fields v sv,
  struct_of v = Some sv ->
  sv_type sv <> sname ->
  match_pattern (PStruct sname fields) v = None.
Proof.
  intros sname fields v sv Hs Hne. simpl. rewrite Hs.
  destruct (String.eqb (sv_type sv) sname) eqn:E.
  - apply String.eqb_eq in E. contradiction.
  - reflexivity.
Qed.

Theorem struct_no_fields : forall sname v sv,
  struct_of v = Some sv ->
  sv_type sv = sname ->
  match_pattern (PStruct sname []) v = Some [].
Proof.
  intros sname v sv Hs Heq. simpl. rewrite Hs.
  rewrite <- Heq. rewrite String.eqb_refl. reflexivity.
Qed.


(* ================================================================ *)
(* J. Match dispatch properties                                      *)
(* ================================================================ *)

Theorem match_cases_empty : forall v,
  match_cases [] v = None.
Proof. reflexivity. Qed.

(* First matching case with passing guard wins *)
Theorem match_cases_first_wins : forall c rest v bs,
  match_pattern (case_pattern c) v = Some bs ->
  case_guard c = None ->
  match_cases (c :: rest) v = Some (bs, case_body c).
Proof.
  intros c rest v bs Hm Hg. simpl. rewrite Hm. rewrite Hg.
  reflexivity.
Qed.

(* Guard failure skips to next case *)
Theorem match_cases_guard_fail : forall c rest v bs,
  match_pattern (case_pattern c) v = Some bs ->
  case_guard c = Some (fun b => false) ->
  match_cases (c :: rest) v = match_cases rest v.
Proof.
  intros c rest v bs Hm Hg. simpl. rewrite Hm. rewrite Hg.
  reflexivity.
Qed.

(* Pattern failure skips to next case *)
Theorem match_cases_pattern_fail : forall c rest v,
  match_pattern (case_pattern c) v = None ->
  match_cases (c :: rest) v = match_cases rest v.
Proof.
  intros c rest v Hm. simpl. rewrite Hm. reflexivity.
Qed.

(* Wildcard case catches all *)
Theorem match_cases_wildcard_catches_all : forall body rest v,
  exists bs result,
    match_cases (mkCase PWildcard None body :: rest) v = Some (bs, result).
Proof.
  intros body rest v. simpl.
  exists []. exists body. reflexivity.
Qed.


(* ================================================================ *)
(* K. Determinism                                                    *)
(*                                                                   *)
(* match_pattern is a function, so deterministic by construction.    *)
(* match_cases returns the FIRST matching case (sequential).         *)
(* ================================================================ *)

(* Pattern matching is deterministic *)
Theorem match_pattern_deterministic : forall p v r1 r2,
  match_pattern p v = r1 ->
  match_pattern p v = r2 ->
  r1 = r2.
Proof. intros. subst. reflexivity. Qed.

(* Match dispatch is deterministic *)
Theorem match_cases_deterministic : forall cases v r1 r2,
  match_cases cases v = r1 ->
  match_cases cases v = r2 ->
  r1 = r2.
Proof. intros. subst. reflexivity. Qed.


(* ================================================================ *)
(* L. Pattern classification (link to CatnipIR.v)                   *)
(*                                                                   *)
(* IRPure pattern constructors correspond 1:1 to Pattern type.       *)
(* ================================================================ *)

Definition pattern_to_ir (p : Pattern) : IRPure :=
  match p with
  | PWildcard => IRPatternWildcard
  | PLiteral _ => IRPatternLiteral (IRNone) (* value abstracted *)
  | PVar name => IRPatternVar name
  | POr _ => IRPatternOr []               (* subpatterns abstracted *)
  | PTuple _ => IRPatternTuple []          (* subpatterns abstracted *)
  | PStruct name fields => IRPatternStruct name fields
  end.

Theorem pattern_to_ir_is_pattern : forall p,
  is_pattern (pattern_to_ir p) = true.
Proof. destruct p; reflexivity. Qed.


(* ================================================================ *)
(* M. Tag numbering (matches pattern.rs constants)                   *)
(* ================================================================ *)

Definition pattern_tag (p : Pattern) : nat :=
  match p with
  | PWildcard => 0
  | PLiteral _ => 1
  | PVar _ => 2
  | POr _ => 3
  | PTuple _ => 4
  | PStruct _ _ => 5
  end.

Theorem pattern_tag_range : forall p,
  (pattern_tag p <= 5)%nat.
Proof. destruct p; simpl; lia. Qed.

Theorem pattern_tag_injective_kind : forall p1 p2,
  pattern_tag p1 = pattern_tag p2 ->
  match p1, p2 with
  | PWildcard, PWildcard => True
  | PLiteral _, PLiteral _ => True
  | PVar _, PVar _ => True
  | POr _, POr _ => True
  | PTuple _, PTuple _ => True
  | PStruct _ _, PStruct _ _ => True
  | _, _ => False
  end.
Proof.
  intros p1 p2 H.
  destruct p1; destruct p2; simpl in H; try discriminate; exact I.
Qed.

End WithValue.

Arguments PWildcard {V}.
Arguments PLiteral {V}.
Arguments PVar {V}.
Arguments POr {V}.
Arguments PTuple {V}.
Arguments PStruct {V}.
Arguments mkStruct {V}.
Arguments mkCase {V}.
Arguments match_pattern {V}.
Arguments match_cases {V}.
Arguments pattern_to_ir {V}.
Arguments pattern_tag {V}.
Arguments field_lookup {V}.


Open Scope string_scope.

(* ================================================================ *)
(* N. Concrete examples (V = nat)                                    *)
(* ================================================================ *)

Definition nat_eqb := Nat.eqb.

Lemma nat_eqb_spec : forall a b : nat, Nat.eqb a b = true <-> a = b.
Proof. intros. split; [apply Nat.eqb_eq | apply Nat.eqb_eq]. Qed.

(* No structs or lists for simple examples *)
Definition no_struct (_ : nat) : option (StructValue nat) := None.
Definition no_list (_ : nat) : option (list nat) := None.

(* Simple list extractor for tuple tests *)
Definition nat_list_of (v : nat) : option (list nat) := None.

Example ex_wildcard :
  match_pattern Nat.eqb no_struct no_list PWildcard 42 = Some [].
Proof. reflexivity. Qed.

Example ex_var :
  match_pattern Nat.eqb no_struct no_list (PVar "x") 42 = Some [("x", 42)].
Proof. reflexivity. Qed.

Example ex_literal_match :
  match_pattern Nat.eqb no_struct no_list (PLiteral 42) 42 = Some [].
Proof. reflexivity. Qed.

Example ex_literal_fail :
  match_pattern Nat.eqb no_struct no_list (PLiteral 1) 2 = None.
Proof. reflexivity. Qed.

Example ex_or_first :
  match_pattern Nat.eqb no_struct no_list
    (POr [PLiteral 1; PLiteral 2; PLiteral 3]) 1 = Some [].
Proof. reflexivity. Qed.

Example ex_or_second :
  match_pattern Nat.eqb no_struct no_list
    (POr [PLiteral 1; PLiteral 2; PLiteral 3]) 2 = Some [].
Proof. reflexivity. Qed.

Example ex_or_none :
  match_pattern Nat.eqb no_struct no_list
    (POr [PLiteral 1; PLiteral 2]) 99 = None.
Proof. reflexivity. Qed.

(* Match dispatch example *)
Example ex_match_dispatch :
  let cases := [
    mkCase (PLiteral 1) None 100;
    mkCase (PLiteral 2) None 200;
    mkCase (PVar "x") None 300
  ] in
  match_cases Nat.eqb no_struct no_list cases 2 = Some ([], 200).
Proof. reflexivity. Qed.

(* Wildcard as default case *)
Example ex_match_default :
  let cases := [
    mkCase (PLiteral 1) None 100;
    mkCase PWildcard None 999
  ] in
  match_cases Nat.eqb no_struct no_list cases 42 = Some ([], 999).
Proof. reflexivity. Qed.
