(* FILE: proof/delta/DeltaProof.v *)
(* DeltaProof.v - facade for the delta dataflow proofs
 *
 * Re-exports:
 *   DeltaCollection.v : neutral compaction of the collection core (step 1)
 *   DeltaStateless.v  : stateless-operator homomorphisms (step 2)
 *)

From Catnip Require Export DeltaCollection.
From Catnip Require Export DeltaStateless.
