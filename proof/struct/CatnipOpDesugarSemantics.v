(* FILE: proof/struct/CatnipOpDesugarSemantics.v *)
(* Operator overloading: semantic preservation
 *
 * Source of truth:
 *   catnip_rs/src/vm/core.rs  (binary_add fallback to try_struct_binop/rbinop)
 *
 * Proves:
 *   - VM dispatch equivalence: binary op fallback = method call
 *   - Reverse dispatch: prim OP struct -> struct's method with (struct, prim)
 *   - Roundtrip: symbol -> method name -> opcode -> same symbol
 *   - Concrete examples on Vec2-like structs
 *)

From Coq Require Import List String Bool ZArith.
Import ListNotations.

From Catnip Require Import CatnipStructBase.
From Catnip Require Import CatnipStructOps.
From Catnip Require Import CatnipOpDesugar.
From Catnip Require Import CatnipOpDesugarProps.

Open Scope string_scope.


(* ================================================================ *)
(* A. VM Dispatch Model                                             *)
(*                                                                  *)
(* Models the two-phase dispatch in vm/core.rs:                     *)
(*   1. Try native op (binary_add, binary_sub, ...)                 *)
(*   2. On TypeError, try_struct_binop -> method call               *)
(*                                                                  *)
(* A value is either a primitive (native ops work) or a struct      *)
(* instance (native ops fail, method dispatch attempted).           *)
(* ================================================================ *)

Inductive VMValue :=
  | VPrim (v : Z)                                  (* int, float, ... *)
  | VStruct (type_id : nat) (fields : FieldValues). (* struct instance *)

(* Native binary op: succeeds only on primitives *)
Definition native_binop (op : OperatorSymbol) (a b : VMValue) : option Z :=
  match a, b with
  | VPrim x, VPrim y =>
      Some (match op with
            | SymPlus => x + y
            | SymMinus => x - y
            | SymStar => x * y
            | _ => 0  (* stub for other ops *)
            end)%Z
  | _, _ => None  (* TypeError on struct operands *)
  end.

(* Struct method dispatch: look up op_* method in type's methods *)
Definition struct_dispatch
  (method_name : string) (ty : StructType) : option MethodEntry :=
  find_method method_name (st_methods ty).

(* Full VM binary dispatch: native first, then struct fallback *)
Inductive DispatchResult :=
  | DispatchNative (v : Z)
  | DispatchMethod (m : MethodEntry)
  | DispatchError.

Definition vm_dispatch_binop
  (sym : OperatorSymbol) (a b : VMValue) (ty : option StructType)
  : DispatchResult :=
  match native_binop sym a b with
  | Some v => DispatchNative v
  | None =>
      match ty with
      | Some t =>
          match desugar_operator sym Binary with
          | Some name =>
              match struct_dispatch name t with
              | Some m => DispatchMethod m
              | None => DispatchError
              end
          | None => DispatchError
          end
      | None => DispatchError
      end
  end.

(* Direct method call: resolve name in struct, call it *)
Definition vm_call_method
  (name : string) (ty : StructType) : option MethodEntry :=
  struct_dispatch name ty.

(* Extended dispatch with reverse: try a's type, then b's type *)
Definition vm_dispatch_binop_rev
  (sym : OperatorSymbol) (a b : VMValue)
  (ty_a ty_b : option StructType)
  : DispatchResult :=
  match native_binop sym a b with
  | Some v => DispatchNative v
  | None =>
      (* Forward: try left operand's type *)
      match ty_a with
      | Some ta =>
          match desugar_operator sym Binary with
          | Some name =>
              match struct_dispatch name ta with
              | Some m => DispatchMethod m
              | None =>
                  (* Reverse: try right operand's type *)
                  match ty_b with
                  | Some tb =>
                      match struct_dispatch name tb with
                      | Some m => DispatchMethod m
                      | None => DispatchError
                      end
                  | None => DispatchError
                  end
              end
          | None => DispatchError
          end
      | None =>
          (* No left type, try reverse directly *)
          match ty_b with
          | Some tb =>
              match desugar_operator sym Binary with
              | Some name =>
                  match struct_dispatch name tb with
                  | Some m => DispatchMethod m
                  | None => DispatchError
                  end
              | None => DispatchError
              end
          | None => DispatchError
          end
      end
  end.


(* ================================================================ *)
(* B. Semantic Preservation                                         *)
(*                                                                  *)
(* When native op fails on a struct and the struct has the          *)
(* corresponding op_* method, the VM dispatch produces the same    *)
(* method entry as a direct method call.                            *)
(* ================================================================ *)

Theorem operator_dispatch_is_method_call :
  forall sym name a b ty m,
    desugar_operator sym Binary = Some name ->
    native_binop sym a b = None ->
    struct_dispatch name ty = Some m ->
    vm_dispatch_binop sym a b (Some ty) = DispatchMethod m.
Proof.
  intros sym name a b ty m Hdesugar Hnat Hmethod.
  unfold vm_dispatch_binop.
  rewrite Hnat. rewrite Hdesugar. rewrite Hmethod.
  reflexivity.
Qed.

(* Structs always fail native ops *)
Lemma struct_native_fails : forall sym ti fi b,
  native_binop sym (VStruct ti fi) b = None.
Proof. reflexivity. Qed.

(* Corollary: for struct values, dispatch = method call *)
Corollary struct_dispatch_is_method_call :
  forall sym name ti fi b ty m,
    desugar_operator sym Binary = Some name ->
    struct_dispatch name ty = Some m ->
    vm_dispatch_binop sym (VStruct ti fi) b (Some ty) = DispatchMethod m.
Proof.
  intros.
  apply operator_dispatch_is_method_call with (name := name); auto.
Qed.

(* The method found via dispatch is the same as find_method *)
Theorem dispatch_finds_same_method :
  forall sym name ti fi b ty m,
    desugar_operator sym Binary = Some name ->
    find_method name (st_methods ty) = Some m ->
    vm_dispatch_binop sym (VStruct ti fi) b (Some ty) = DispatchMethod m.
Proof.
  intros. apply struct_dispatch_is_method_call with (name := name); auto.
Qed.


(* ================================================================ *)
(* B2. Reverse Dispatch                                             *)
(*                                                                  *)
(* When prim OP struct is evaluated and native op fails,            *)
(* the struct's method is called with (struct, prim) args.          *)
(* ================================================================ *)

(* Prim + Struct: reverse dispatch finds struct's method *)
Theorem reverse_dispatch_finds_method :
  forall sym name v ti fi tb m,
    desugar_operator sym Binary = Some name ->
    native_binop sym (VPrim v) (VStruct ti fi) = None ->
    struct_dispatch name tb = Some m ->
    vm_dispatch_binop_rev sym (VPrim v) (VStruct ti fi) None (Some tb) = DispatchMethod m.
Proof.
  intros sym name v ti fi tb m Hdesugar Hnat Hmethod.
  unfold vm_dispatch_binop_rev.
  rewrite Hnat. rewrite Hdesugar. rewrite Hmethod.
  reflexivity.
Qed.

(* Forward takes priority over reverse *)
Theorem forward_priority_over_reverse :
  forall sym name a b ta tb mf mr,
    desugar_operator sym Binary = Some name ->
    native_binop sym a b = None ->
    struct_dispatch name ta = Some mf ->
    struct_dispatch name tb = Some mr ->
    vm_dispatch_binop_rev sym a b (Some ta) (Some tb) = DispatchMethod mf.
Proof.
  intros sym name a b ta tb mf mr Hdesugar Hnat Hfwd Hrev.
  unfold vm_dispatch_binop_rev.
  rewrite Hnat. rewrite Hdesugar. rewrite Hfwd.
  reflexivity.
Qed.

(* Reverse only fires when forward has no method *)
Theorem reverse_only_without_forward :
  forall sym name a b ta tb m,
    desugar_operator sym Binary = Some name ->
    native_binop sym a b = None ->
    struct_dispatch name ta = None ->
    struct_dispatch name tb = Some m ->
    vm_dispatch_binop_rev sym a b (Some ta) (Some tb) = DispatchMethod m.
Proof.
  intros sym name a b ta tb m Hdesugar Hnat Hno_fwd Hrev.
  unfold vm_dispatch_binop_rev.
  rewrite Hnat. rewrite Hdesugar. rewrite Hno_fwd. rewrite Hrev.
  reflexivity.
Qed.

(* No method on either side -> error *)
Theorem no_method_either_side_errors :
  forall sym name a b ta tb,
    desugar_operator sym Binary = Some name ->
    native_binop sym a b = None ->
    struct_dispatch name ta = None ->
    struct_dispatch name tb = None ->
    vm_dispatch_binop_rev sym a b (Some ta) (Some tb) = DispatchError.
Proof.
  intros sym name a b ta tb Hdesugar Hnat Hno_fwd Hno_rev.
  unfold vm_dispatch_binop_rev.
  rewrite Hnat. rewrite Hdesugar. rewrite Hno_fwd. rewrite Hno_rev.
  reflexivity.
Qed.

(* Backwards compatibility: old dispatch is a special case of new one *)
Theorem rev_dispatch_subsumes_old :
  forall sym a b ty,
    vm_dispatch_binop sym a b ty =
    vm_dispatch_binop_rev sym a b ty None.
Proof.
  intros. unfold vm_dispatch_binop, vm_dispatch_binop_rev.
  destruct (native_binop sym a b); [auto|].
  destruct ty as [ta|]; [|auto].
  destruct (desugar_operator sym Binary) as [name|]; [|auto].
  destruct (struct_dispatch name ta); auto.
Qed.


(* ================================================================ *)
(* C. Opcode Roundtrip                                              *)
(*                                                                  *)
(* symbol -> method name -> opcode -> same opcode as expected.      *)
(* Connects parsing (symbol) to VM execution (opcode) through      *)
(* the desugaring layer.                                            *)
(* ================================================================ *)

Theorem opcode_roundtrip : forall sym ar name opc,
  desugar_operator sym ar = Some name ->
  desugar_to_opcode name = Some opc ->
  expected_opcode sym ar = Some opc.
Proof.
  intros sym ar name opc Hd Ht.
  rewrite <- Ht.
  symmetry. exact (desugar_opcode_consistent sym ar name Hd).
Qed.


(* ================================================================ *)
(* D. Dispatch Determinism                                          *)
(*                                                                  *)
(* The dispatch result is unique: same inputs, same output.         *)
(* ================================================================ *)

Theorem dispatch_deterministic :
  forall sym a b ty r1 r2,
    r1 = vm_dispatch_binop sym a b ty ->
    r2 = vm_dispatch_binop sym a b ty ->
    r1 = r2.
Proof. intros. subst. reflexivity. Qed.

(* No ambiguity: a symbol resolves to exactly one method *)
Theorem no_dispatch_ambiguity :
  forall s1 s2 a1 a2 name ty m1 m2,
    desugar_operator s1 a1 = Some name ->
    desugar_operator s2 a2 = Some name ->
    find_method name (st_methods ty) = Some m1 ->
    find_method name (st_methods ty) = Some m2 ->
    s1 = s2 /\ a1 = a2 /\ m1 = m2.
Proof.
  intros.
  destruct (desugar_injective s1 a1 s2 a2 name H H0) as [Hs Ha].
  rewrite H1 in H2. inversion H2.
  auto.
Qed.


(* ================================================================ *)
(* E. Concrete Examples                                             *)
(* ================================================================ *)

Definition vec2_type := mkStructType
  0 "Vec2"
  [mkField "x" false; mkField "y" false]
  [mkMethod "op_add" MkInstance 1;
   mkMethod "op_sub" MkInstance 2;
   mkMethod "op_neg" MkInstance 3;
   mkMethod "op_eq"  MkInstance 4]
  [] [] [] [] [].

Definition v1 := VStruct 0 [1%Z; 2%Z].
Definition v2 := VStruct 0 [3%Z; 4%Z].

Example vec2_add_dispatches :
  vm_dispatch_binop SymPlus v1 v2 (Some vec2_type) =
  DispatchMethod (mkMethod "op_add" MkInstance 1).
Proof. reflexivity. Qed.

Example vec2_sub_dispatches :
  vm_dispatch_binop SymMinus v1 v2 (Some vec2_type) =
  DispatchMethod (mkMethod "op_sub" MkInstance 2).
Proof. reflexivity. Qed.

Example vec2_eq_dispatches :
  vm_dispatch_binop SymEq v1 v2 (Some vec2_type) =
  DispatchMethod (mkMethod "op_eq" MkInstance 4).
Proof. reflexivity. Qed.

(* Missing method -> error *)
Example vec2_mul_fails :
  vm_dispatch_binop SymStar v1 v2 (Some vec2_type) = DispatchError.
Proof. reflexivity. Qed.

(* Primitives use native path, not method dispatch *)
Example prim_add_native :
  vm_dispatch_binop SymPlus (VPrim 3%Z) (VPrim 4%Z) None = DispatchNative 7%Z.
Proof. reflexivity. Qed.

(* Mixed: struct + prim, native fails, dispatches to method *)
Example struct_plus_prim :
  vm_dispatch_binop SymPlus v1 (VPrim 5%Z) (Some vec2_type) =
  DispatchMethod (mkMethod "op_add" MkInstance 1).
Proof. reflexivity. Qed.

(* Reverse: prim + struct, dispatches to struct's method *)
Example prim_plus_struct_rev :
  vm_dispatch_binop_rev SymPlus (VPrim 5%Z) v1 None (Some vec2_type) =
  DispatchMethod (mkMethod "op_add" MkInstance 1).
Proof. reflexivity. Qed.

(* Forward wins when both sides have methods *)
Example forward_wins_over_reverse :
  vm_dispatch_binop_rev SymPlus v1 v2 (Some vec2_type) (Some vec2_type) =
  DispatchMethod (mkMethod "op_add" MkInstance 1).
Proof. reflexivity. Qed.

(* Reverse: prim - struct *)
Example prim_sub_struct_rev :
  vm_dispatch_binop_rev SymMinus (VPrim 10%Z) v1 None (Some vec2_type) =
  DispatchMethod (mkMethod "op_sub" MkInstance 2).
Proof. reflexivity. Qed.

(* Reverse on missing method -> error *)
Example prim_mul_struct_no_method :
  vm_dispatch_binop_rev SymStar (VPrim 3%Z) v1 None (Some vec2_type) = DispatchError.
Proof. reflexivity. Qed.
