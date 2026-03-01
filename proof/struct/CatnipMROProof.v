(* FILE: proof/struct/CatnipMROProof.v *)
(* Facade: re-exports all MRO proof modules.
 *
 * Source of truth:
 *   catnip_rs/src/vm/mro.rs          (c3_linearize)
 *   catnip_rs/src/vm/core.rs         (MakeStruct: MRO field/method merge)
 *   catnip_rs/src/vm/structs.rs      (StructType.mro, SuperProxy.method_sources)
 *   catnip_rs/src/core/registry/functions.rs (build_super_proxy)
 *
 * Reference: https://www.python.org/download/releases/2.3/mro/
 *)

From Catnip Require Export CatnipMROC3Core.
From Catnip Require Export CatnipMROC3Properties.
From Catnip Require Export CatnipMROFields.
From Catnip Require Export CatnipMROMethods.
From Catnip Require Export CatnipMROSuper.
From Catnip Require Export CatnipMROExamples.
