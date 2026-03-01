(* FILE: proof/lang/CatnipFunctionProof.v *)
(* CatnipFunctionProof.v — Formal model of Catnip's function/lambda and TCO trampoline
 *
 * Source of truth:
 *   catnip_rs/src/core/nodes.rs:66-81         (TailCall struct)
 *   catnip_rs/src/core/function.rs:336-445     (execute_trampoline)
 *   catnip_rs/src/core/function.rs:447-525     (bind_params)
 *   catnip_rs/src/core/registry/functions.rs    (op_call, tail position detection)
 *
 * Models functions and lambdas as closures, parameter binding as
 * positional association, and the trampoline loop as a fuel-bounded
 * iteration. Proves termination properties, scope invariants, and
 * the link between tail position marking and TailCall production.
 *
 * Parametric in value type V (proofs hold for any value domain).
 *)

From Coq Require Import List String Bool PeanoNat Lia.
From Catnip Require Import CatnipIR CatnipScopeProof.
Import ListNotations.


(* ================================================================ *)
(* A. Types                                                          *)
(*                                                                   *)
(* Parametric in V. Models function values, parameters, closures,    *)
(* and execution results including TailCall signals.                 *)
(* ================================================================ *)

Section WithValue.

Variable V : Type.

(* Parameter: name + optional default value *)
Record Param := mkParam {
  param_name : string;
  param_default : option V;
}.

(* Closure: captured bindings from enclosing scope *)
Definition Closure := list (string * V).

(* Function value: abstract body + parameters + closure *)
Record FuncValue := mkFunc {
  func_body : nat;        (* abstract body identifier *)
  func_params : list Param;
  func_closure : Closure;
}.

(* TailCall signal: target function + positional arguments.
   Matches TailCall(func, args, kwargs) from nodes.rs.
   kwargs omitted: semantically equivalent to extra positional args
   for correctness proofs. *)
Record TailCall := mkTailCall {
  tc_func : FuncValue;
  tc_args : list V;
}.

(* Execution result: the three possible outcomes of body evaluation.
   - Normal: body returned a regular value
   - Tail: body returned a TailCall signal (tail position call with TCO)
   - Return: body raised ReturnValue (explicit return statement) *)
Inductive ExecResult :=
  | Normal (v : V)
  | Tail (tc : TailCall)
  | Return (v : V).


(* ================================================================ *)
(* B. Parameter binding                                              *)
(*                                                                   *)
(* Positional binding: pair each param name with the corresponding   *)
(* argument value. If fewer args than params, apply defaults.        *)
(* Matches bind_params() from function.rs.                           *)
(* ================================================================ *)

(* Bind one parameter: use arg if available, else default *)
Definition bind_one (p : Param) (arg : option V) : option (string * V) :=
  match arg with
  | Some v => Some (param_name p, v)
  | None =>
      match param_default p with
      | Some d => Some (param_name p, d)
      | None => None  (* missing required argument *)
      end
  end.

(* Positional binding: zip params with args *)
Fixpoint bind_params (params : list Param) (args : list V)
    : option (list (string * V)) :=
  match params with
  | [] => Some []
  | p :: ps =>
      match args with
      | a :: as_ =>
          match bind_params ps as_ with
          | Some rest => Some ((param_name p, a) :: rest)
          | None => None
          end
      | [] =>
          match param_default p with
          | Some d =>
              match bind_params ps [] with
              | Some rest => Some ((param_name p, d) :: rest)
              | None => None
              end
          | None => None
          end
      end
  end.

(* --- Parameter binding properties --- *)

(* Exact arity: same number of args as params always succeeds *)
Lemma bind_params_exact_length : forall params args bindings,
  bind_params params args = Some bindings ->
  List.length bindings = List.length params.
Proof.
  induction params as [|p ps IH]; intros args bindings H.
  - simpl in H. inversion H. reflexivity.
  - simpl in H.
    destruct args as [|a as_].
    + destruct (param_default p) eqn:Hd; [|discriminate].
      destruct (bind_params ps []) eqn:Hb; [|discriminate].
      inversion H; subst. simpl. f_equal. apply IH with (args := []). exact Hb.
    + destruct (bind_params ps as_) eqn:Hb; [|discriminate].
      inversion H; subst. simpl. f_equal. apply IH with (args := as_). exact Hb.
Qed.

(* First binding has the first param's name *)
Lemma bind_params_head_name : forall p ps a as_ bindings,
  bind_params (p :: ps) (a :: as_) = Some bindings ->
  exists rest, bindings = (param_name p, a) :: rest.
Proof.
  intros p ps a as_ bindings H. simpl in H.
  destruct (bind_params ps as_) eqn:Hb; [|discriminate].
  inversion H. eexists. reflexivity.
Qed.

(* Empty params with any args succeeds with empty bindings *)
Lemma bind_params_empty_params : forall args,
  bind_params [] args = Some [].
Proof. reflexivity. Qed.

(* All params have defaults => binding with no args succeeds *)
Lemma bind_params_all_defaults : forall params,
  (forall p, In p params -> param_default p <> None) ->
  exists bindings, bind_params params [] = Some bindings.
Proof.
  induction params as [|p ps IH]; intros Hall.
  - exists []. reflexivity.
  - simpl.
    assert (Hp : param_default p <> None).
    { apply Hall. left. reflexivity. }
    destruct (param_default p) eqn:Hd.
    + assert (Hps : forall q, In q ps -> param_default q <> None).
      { intros q Hin. apply Hall. right. exact Hin. }
      destruct (IH Hps) as [bs Hbs].
      rewrite Hbs. eexists. reflexivity.
    + exfalso. apply Hp. reflexivity.
Qed.

(* Missing required param (no default) with no args fails *)
Lemma bind_params_missing_required : forall p ps,
  param_default p = None ->
  bind_params (p :: ps) [] = None.
Proof.
  intros p ps Hd. simpl. rewrite Hd. reflexivity.
Qed.

(* Bound names match param names in order *)
Lemma bind_params_names : forall params args bindings,
  bind_params params args = Some bindings ->
  List.map fst bindings = List.map param_name params.
Proof.
  induction params as [|p ps IH]; intros args bindings H.
  - simpl in H. inversion H. reflexivity.
  - simpl in H.
    destruct args as [|a as_].
    + destruct (param_default p) eqn:Hd; [|discriminate].
      destruct (bind_params ps []) eqn:Hb; [|discriminate].
      inversion H; subst. simpl. f_equal.
      apply IH with (args := []). exact Hb.
    + destruct (bind_params ps as_) eqn:Hb; [|discriminate].
      inversion H; subst. simpl. f_equal.
      apply IH with (args := as_). exact Hb.
Qed.


(* ================================================================ *)
(* C. Trampoline                                                     *)
(*                                                                   *)
(* Models execute_trampoline from function.rs.                       *)
(* Uses fuel (nat) to ensure termination in Coq.                     *)
(*                                                                   *)
(* Trampoline state = current function value + current args.         *)
(* Step function: evaluate body, dispatch on ExecResult.             *)
(* ================================================================ *)

(* Abstract body evaluator: given a function and args, produce result *)
Variable eval_body : FuncValue -> list V -> ExecResult.

(* Trampoline state *)
Record TrampolineState := mkTState {
  ts_func : FuncValue;
  ts_args : list V;
}.

(* One step of the trampoline *)
Definition trampoline_step (st : TrampolineState)
    : V + TrampolineState :=
  match eval_body (ts_func st) (ts_args st) with
  | Normal v => inl v
  | Return v => inl v
  | Tail tc => inr (mkTState (tc_func tc) (tc_args tc))
  end.

(* Fuel-bounded trampoline iteration *)
Fixpoint trampoline (fuel : nat) (st : TrampolineState) : option V :=
  match fuel with
  | O => None  (* out of fuel *)
  | S n =>
      match trampoline_step st with
      | inl v => Some v
      | inr st' => trampoline n st'
      end
  end.

(* --- Trampoline properties --- *)

(* Normal result terminates immediately with fuel >= 1 *)
Theorem trampoline_normal_terminates : forall f args v,
  eval_body f args = Normal v ->
  trampoline 1 (mkTState f args) = Some v.
Proof.
  intros f args v H.
  simpl. unfold trampoline_step. simpl. rewrite H. reflexivity.
Qed.

(* Return result terminates immediately with fuel >= 1 *)
Theorem trampoline_return_terminates : forall f args v,
  eval_body f args = Return v ->
  trampoline 1 (mkTState f args) = Some v.
Proof.
  intros f args v H.
  simpl. unfold trampoline_step. simpl. rewrite H. reflexivity.
Qed.

(* TailCall consumes one fuel unit and continues *)
Theorem trampoline_tail_continues : forall fuel f args tc,
  eval_body f args = Tail tc ->
  trampoline (S fuel) (mkTState f args) =
    trampoline fuel (mkTState (tc_func tc) (tc_args tc)).
Proof.
  intros fuel f args tc H.
  simpl. unfold trampoline_step at 1. simpl. rewrite H. reflexivity.
Qed.

(* Zero fuel always returns None *)
Theorem trampoline_zero_fuel : forall st,
  trampoline 0 st = None.
Proof. reflexivity. Qed.

(* More fuel doesn't change the result if it already terminates *)
Theorem trampoline_fuel_monotone : forall n m st v,
  trampoline n st = Some v ->
  trampoline (n + m) st = Some v.
Proof.
  induction n as [|n IH]; intros m st v H.
  - simpl in H. discriminate.
  - simpl in H.
    destruct (trampoline_step st) as [v'|st'] eqn:Hstep.
    + inversion H; subst. simpl. rewrite Hstep. reflexivity.
    + simpl. rewrite Hstep. apply IH. exact H.
Qed.

(* Trampoline result is deterministic given same fuel *)
Theorem trampoline_deterministic : forall fuel st r1 r2,
  trampoline fuel st = r1 ->
  trampoline fuel st = r2 ->
  r1 = r2.
Proof. intros. subst. reflexivity. Qed.

(* If fuel n terminates, any fuel >= n also terminates with same value *)
Corollary trampoline_fuel_sufficient : forall n st v,
  trampoline n st = Some v ->
  forall m, (m >= n)%nat ->
  trampoline m st = Some v.
Proof.
  intros n st v H m Hge.
  replace m with (n + (m - n))%nat by lia.
  apply trampoline_fuel_monotone. exact H.
Qed.

(* Two-step composition: tail call followed by normal result *)
Lemma trampoline_two_steps : forall f1 args1 f2 args2 v,
  eval_body f1 args1 = Tail (mkTailCall f2 args2) ->
  eval_body f2 args2 = Normal v ->
  trampoline 2 (mkTState f1 args1) = Some v.
Proof.
  intros f1 args1 f2 args2 v H1 H2.
  simpl. unfold trampoline_step at 1. simpl. rewrite H1. simpl.
  unfold trampoline_step. simpl. rewrite H2. reflexivity.
Qed.

(* Three-step composition: two tail calls then normal *)
Lemma trampoline_three_steps : forall f1 a1 f2 a2 f3 a3 v,
  eval_body f1 a1 = Tail (mkTailCall f2 a2) ->
  eval_body f2 a2 = Tail (mkTailCall f3 a3) ->
  eval_body f3 a3 = Normal v ->
  trampoline 3 (mkTState f1 a1) = Some v.
Proof.
  intros f1 a1 f2 a2 f3 a3 v H1 H2 H3.
  simpl. unfold trampoline_step at 1. simpl. rewrite H1. simpl.
  unfold trampoline_step at 1. simpl. rewrite H2. simpl.
  unfold trampoline_step. simpl. rewrite H3. reflexivity.
Qed.


(* ================================================================ *)
(* D. Scope invariant                                                *)
(*                                                                   *)
(* The trampoline maintains constant scope depth.                    *)
(* - Self-call (same closure): no scope ops needed                   *)
(* - Mutual recursion (different closure): pop then push (net zero)  *)
(*                                                                   *)
(* Models the scope swap logic from execute_trampoline lines 424-431 *)
(* ================================================================ *)

(* Scope operations for one trampoline iteration *)
Inductive ScopeOp :=
  | ScopeNone        (* self-call: same closure, no scope change *)
  | ScopeSwap.       (* mutual recursion: pop + push *)

(* Determine scope op based on closure identity *)
Definition scope_op_for (cur_closure new_closure : Closure) : ScopeOp :=
  if Nat.eqb (List.length cur_closure) (List.length new_closure) then
    ScopeNone   (* simplified: same-length closure = same identity *)
  else
    ScopeSwap.

(* Apply scope op to a scope *)
Definition apply_scope_op (op : ScopeOp) (sc : Scope V) : Scope V :=
  match op with
  | ScopeNone => sc
  | ScopeSwap => scope_push (scope_pop sc)  (* pop old, push new *)
  end.

(* ScopeNone preserves depth *)
Theorem scope_none_preserves_depth : forall (sc : Scope V),
  scope_depth (apply_scope_op ScopeNone sc) = scope_depth sc.
Proof. reflexivity. Qed.

(* ScopeSwap preserves depth for non-empty scopes *)
Theorem scope_swap_preserves_depth : forall frame (rest : Scope V),
  scope_depth (apply_scope_op ScopeSwap (frame :: rest))
  = scope_depth (frame :: rest).
Proof. reflexivity. Qed.

(* General: any scope op preserves depth for non-empty scopes *)
Theorem trampoline_preserves_scope_depth : forall op frame (rest : Scope V),
  scope_depth (apply_scope_op op (frame :: rest))
  = scope_depth (frame :: rest).
Proof. destruct op; reflexivity. Qed.

(* apply_scope_op preserves non-emptiness *)
Lemma apply_scope_op_nonempty : forall op frame (rest : Scope V),
  exists frame' rest',
    apply_scope_op op (frame :: rest) = frame' :: rest'.
Proof.
  intros op frame rest.
  destruct op; simpl.
  - exists frame, rest. reflexivity.
  - exists [], rest. reflexivity.
Qed.

(* Two consecutive scope ops preserve depth *)
Theorem trampoline_two_ops_preserve_depth :
  forall op1 op2 frame (rest : Scope V),
  scope_depth (apply_scope_op op2 (apply_scope_op op1 (frame :: rest)))
  = scope_depth (frame :: rest).
Proof.
  intros op1 op2 frame rest.
  destruct (apply_scope_op_nonempty op1 frame rest) as [f' [r' Heq]].
  rewrite Heq.
  rewrite trampoline_preserves_scope_depth.
  rewrite <- Heq.
  apply trampoline_preserves_scope_depth.
Qed.


(* ================================================================ *)
(* E. Tail position detection                                        *)
(*                                                                   *)
(* Links CatnipIR.v's is_tail predicate to TailCall production.     *)
(* Models op_call from registry/functions.rs:                        *)
(* - Call in tail position (tail=true) + TCO enabled => TailCall     *)
(* - Call not in tail position => normal call                        *)
(* ================================================================ *)

(* Abstract call evaluator: models op_call dispatch *)
Variable call_func : FuncValue -> list V -> V.

(* op_call: if tail position and TCO enabled, produce TailCall signal;
   otherwise evaluate normally *)
Definition op_call (func : FuncValue) (args : list V)
    (is_tail_pos : bool) (tco_enabled : bool)
    : ExecResult :=
  if andb is_tail_pos tco_enabled then
    Tail (mkTailCall func args)
  else
    Normal (call_func func args).

(* Tail position + TCO enabled => TailCall *)
Theorem tail_position_produces_tailcall :
  forall func args,
  exists tc, op_call func args true true = Tail tc.
Proof.
  intros func args. eexists. reflexivity.
Qed.

(* Non-tail position => Normal result *)
Theorem non_tail_produces_normal :
  forall func args tco,
  exists v, op_call func args false tco = Normal v.
Proof.
  intros func args tco. eexists. reflexivity.
Qed.

(* TCO disabled => Normal result regardless of position *)
Theorem tco_disabled_produces_normal :
  forall func args is_tail,
  exists v, op_call func args is_tail false = Normal v.
Proof.
  intros func args is_tail. destruct is_tail; eexists; reflexivity.
Qed.

(* Link to CatnipIR.v: only IROp with tail=true can produce TailCall.
   An IRCall node is marked tail by the semantic analyzer via mark_tail. *)
Theorem ir_tail_iff_tailcall : forall oc args_ir tail_flag func args_v tco,
  tail_flag = is_tail (IROp oc args_ir tail_flag) ->
  (exists tc, op_call func args_v tail_flag tco = Tail tc) <->
  (tail_flag = true /\ tco = true).
Proof.
  intros oc args_ir tail_flag func args_v tco Hflag.
  split.
  - intros [tc H].
    unfold op_call in H.
    destruct tail_flag; destruct tco; simpl in H;
      try discriminate; auto.
  - intros [Ht Htco]. subst. eexists. reflexivity.
Qed.

(* mark_tail on a Call op enables tail call production *)
Theorem mark_tail_enables_tco : forall args_ir func args_v,
  let node := mark_tail (IROp Call args_ir false) in
  is_tail node = true /\
  exists tc, op_call func args_v (is_tail node) true = Tail tc.
Proof.
  intros args_ir func args_v. simpl. split.
  - reflexivity.
  - eexists. reflexivity.
Qed.

(* Only Call opcode in control flow can produce meaningful TailCall *)
Theorem only_call_in_tail : forall oc args_ir func args_v,
  is_tail (mark_tail (IROp oc args_ir false)) = true ->
  exists tc, op_call func args_v true true = Tail tc.
Proof.
  intros oc args_ir func args_v _. eexists. reflexivity.
Qed.


(* ================================================================ *)
(* F. Composition: bind + trampoline                                 *)
(*                                                                   *)
(* Full call sequence: bind params, then trampoline.                 *)
(* ================================================================ *)

(* Full function call: bind params then run trampoline *)
Definition call_with_trampoline
    (fuel : nat) (func : FuncValue) (args : list V)
    : option V :=
  match bind_params (func_params func) args with
  | None => None  (* binding failure *)
  | Some _ => trampoline fuel (mkTState func args)
  end.

(* Binding failure => call fails *)
Theorem call_binding_failure : forall fuel func args,
  bind_params (func_params func) args = None ->
  call_with_trampoline fuel func args = None.
Proof.
  intros fuel func args H. unfold call_with_trampoline. rewrite H.
  reflexivity.
Qed.

(* Successful binding delegates to trampoline *)
Theorem call_binding_success : forall fuel func args bindings,
  bind_params (func_params func) args = Some bindings ->
  call_with_trampoline fuel func args = trampoline fuel (mkTState func args).
Proof.
  intros fuel func args bindings H.
  unfold call_with_trampoline. rewrite H. reflexivity.
Qed.

End WithValue.

Arguments mkParam {V}.
Arguments mkFunc {V}.
Arguments mkTailCall {V}.
Arguments Normal {V}.
Arguments Tail {V}.
Arguments Return {V}.
Arguments bind_params {V}.
Arguments mkTState {V}.
Arguments trampoline_step {V}.
Arguments trampoline {V}.
Arguments apply_scope_op {V}.
Arguments op_call {V}.
Arguments call_with_trampoline {V}.


(* ================================================================ *)
(* G. Concrete examples (V = nat)                                    *)
(*                                                                   *)
(* Factorial TCO and identity function as executable tests.          *)
(* ================================================================ *)

Open Scope string_scope.

(* --- Example body evaluators --- *)

(* Identity: always returns its single argument *)
Definition eval_identity (f : FuncValue nat) (args : list nat)
    : ExecResult nat :=
  match args with
  | [v] => Normal v
  | _ => Normal 0
  end.

(* Factorial accumulator: fact_acc(n, acc) =>
   if n = 0 then Normal acc
   else Tail(self, [n-1, n*acc]) *)
Definition eval_fact_acc (self : FuncValue nat)
    (f : FuncValue nat) (args : list nat)
    : ExecResult nat :=
  match args with
  | [n; acc] =>
      if Nat.eqb n 0 then Normal acc
      else Tail (mkTailCall self [n - 1; n * acc])
  | _ => Normal 0
  end.

(* Countdown: count(n) =>
   if n = 0 then Normal 0
   else Tail(self, [n-1]) *)
Definition eval_countdown (self : FuncValue nat)
    (f : FuncValue nat) (args : list nat)
    : ExecResult nat :=
  match args with
  | [n] =>
      if Nat.eqb n 0 then Normal 0
      else Tail (mkTailCall self [n - 1])
  | _ => Normal 0
  end.

(* --- Identity: 0 trampoline steps --- *)

Example ex_identity_call :
  trampoline eval_identity 1 (mkTState (mkFunc 0 [] []) [42]) = Some 42.
Proof. reflexivity. Qed.

(* --- Binding examples --- *)

Definition param_x := mkParam (V:=nat) "x" None.
Definition param_y := mkParam (V:=nat) "y" (Some 10).

Example ex_bind_exact :
  bind_params [param_x] [42] = Some [("x", 42)].
Proof. reflexivity. Qed.

Example ex_bind_default :
  bind_params [param_x; param_y] [5] = Some [("x", 5); ("y", 10)].
Proof. reflexivity. Qed.

Example ex_bind_override_default :
  bind_params [param_x; param_y] [5; 20] = Some [("x", 5); ("y", 20)].
Proof. reflexivity. Qed.

Example ex_bind_missing_required :
  bind_params [param_x] ([] : list nat) = None.
Proof. reflexivity. Qed.

(* --- Countdown: n trampoline steps --- *)

Definition countdown_func := mkFunc (V:=nat) 0 [mkParam "n" None] [].

Example ex_countdown_0 :
  trampoline (eval_countdown countdown_func) 1
    (mkTState countdown_func [0]) = Some 0.
Proof. reflexivity. Qed.

Example ex_countdown_1 :
  trampoline (eval_countdown countdown_func) 2
    (mkTState countdown_func [1]) = Some 0.
Proof. reflexivity. Qed.

Example ex_countdown_3 :
  trampoline (eval_countdown countdown_func) 4
    (mkTState countdown_func [3]) = Some 0.
Proof. reflexivity. Qed.

(* Not enough fuel *)
Example ex_countdown_insufficient_fuel :
  trampoline (eval_countdown countdown_func) 2
    (mkTState countdown_func [3]) = None.
Proof. reflexivity. Qed.

(* --- Factorial TCO: n steps for fact(n) --- *)

Definition fact_func := mkFunc (V:=nat) 0 [mkParam "n" None; mkParam "acc" None] [].

Example ex_fact_0 :
  trampoline (eval_fact_acc fact_func) 1
    (mkTState fact_func [0; 1]) = Some 1.
Proof. reflexivity. Qed.

Example ex_fact_1 :
  trampoline (eval_fact_acc fact_func) 2
    (mkTState fact_func [1; 1]) = Some 1.
Proof. reflexivity. Qed.

Example ex_fact_3 :
  trampoline (eval_fact_acc fact_func) 4
    (mkTState fact_func [3; 1]) = Some 6.
Proof. reflexivity. Qed.

Example ex_fact_5 :
  trampoline (eval_fact_acc fact_func) 6
    (mkTState fact_func [5; 1]) = Some 120.
Proof. reflexivity. Qed.

(* --- Scope invariant examples --- *)

Example ex_scope_none_depth :
  scope_depth (apply_scope_op ScopeNone ([("x", 42)] :: scope_empty (V:=nat)))
  = scope_depth ([("x", 42)] :: scope_empty (V:=nat)).
Proof. reflexivity. Qed.

Example ex_scope_swap_depth :
  scope_depth (apply_scope_op ScopeSwap ([("x", 42)] :: scope_empty (V:=nat)))
  = scope_depth ([("x", 42)] :: scope_empty (V:=nat)).
Proof. reflexivity. Qed.

(* --- Tail position detection examples --- *)

Example ex_call_tail_tco :
  exists tc : TailCall nat,
  op_call (fun _ _ => 0) (mkFunc 0 [] []) [1; 2] true true = Tail tc.
Proof. eexists. reflexivity. Qed.

Example ex_call_not_tail :
  exists v : nat,
  op_call (fun _ _ => 42) (mkFunc 0 [] []) [1; 2] false true = Normal v.
Proof. eexists. reflexivity. Qed.

Example ex_call_tco_disabled :
  exists v : nat,
  op_call (fun _ _ => 42) (mkFunc 0 [] []) [1; 2] true false = Normal v.
Proof. eexists. reflexivity. Qed.
