# Preuves Coq

Vérification mécanisée de fragments de Catnip : grammaire, précédence, modèle dimensionnel, IR, scopes, pattern
matching, fonctions/TCO, et passes d'optimisation.

## TL;DR

69 fichiers Coq dans `proof/` (~20000 lignes, 0 Admitted) prouvent des invariants structurels et sémantiques couvrant le
parsing, le broadcasting, la résolution de scopes, le pattern matching, le trampoline TCO, les 5 passes d'optimisation
IR vivantes (réécritures prouvées + gardes sur les règles retirées), l'analyse de liveness, la ND-récursion, le pipeline
CFG/SSA (Braun et al. 2013), la destruction SSA (séquencement de copies parallèles swap/lost-copy, résolution de phi
triviaux, invisibilité d'une copie morte pour le gate liveness, raccord solveur↔environnement SSA : le lot d'une arête
réalise la sémantique parallèle des phis du join), la reconstruction de régions (le merge d'un if par marche
forward-only), la dominance, le NaN-boxing VM, la frontière de valeur (classification tag scalar/index/pointer, sûreté
`from_raw_scalar`, frontière plugin ABI v5, propriété refcount cross-VM), la sécurité de pile VM, les frames/IP/jumps,
la linéarisation C3 (MRO), les structs/traits, le desugaring opérateurs (sémantique, pureté, broadcast), et le cache. Si
`make proof` passe, les théorèmes sont validés mécaniquement. Ces preuves portent sur des modèles formels alignés avec
le code Rust, pas sur l'exécution du runtime en production. L'alignement est maintenu explicitement dans les
commentaires en tête de chaque fichier `.v`. Tree-sitter et Cranelift ne sont pas formellement prouvés dans ce repo.

> Un parseur sans preuve est un parseur qui ne sait pas encore qu'il a tort.

## Pourquoi des preuves formelles

Catnip utilise tree-sitter pour parser, et tree-sitter fait son travail correctement. Mais la grammaire déclarée dans
`grammar.js` encode des invariants implicites : la précédence de `*` sur `+`, l'associativité gauche de `-`, le fait que
`not` lie plus fort que `and`. Ces propriétés ne sont vérifiées par aucun test unitaire classique - un test vérifie
qu'un cas marche, pas que tous les cas marchent.

Les fichiers dans `proof/` couvrent sept axes :

- **Syntaxe** - invariants de parsing de la grammaire (`grammar.js`) via un parseur à descente récursive formalisé.
- **Sémantique** - propriétés structurelles du modèle dimensionnel (broadcast, ND-récursion).
- **Runtime** - IR opcodes, scopes (shadowing, isolation), pattern matching (6 types, déterminisme), fonctions (binding,
  trampoline TCO, tail detection), NaN-boxing VM (7 tags), VM opcodes et stack safety, frames/IP/jumps, C3 linearization
  (MRO), structs/traits (field access, method resolution, inheritance), desugaring opérateurs (injectivité, totalité,
  cohérence IR).
- **Optimisations** - les 5 passes IR vivantes prouvées (strength reduction, blunt code, DCE, block flattening, constant
  folding) : réécritures correctes + théorèmes de garde sur les règles retirées ; invariant du TCO (équivalence
  tail-récursion/boucle).
- **Analyses** - liveness analysis (linéaire + CFG), dead store elimination, fixpoint, dominance CFG (idom, frontières).
- **Infrastructure** - CFG/SSA (single assignment, phi-nodes, GVN, LICM, CSE inter-blocs, DSE globale), destruction SSA
  (séquencement de copies parallèles swap/lost-copy, phi triviaux, copie morte invisible), cache (FIFO, LRU+TTL,
  mémoïsation, atomic writes).
- **Delta dataflow** - le noyau collection du moteur différentiel (compaction neutre par sommes par clé, delta vide
  neutre, negate round-trip) et les opérateurs stateless (homomorphismes map/filter sur la représentation delta, concat
  = union de multisets, commit stateless identité).

Coq vérifie chaque étape de raisonnement : si `make proof` passe, les propriétés sont vraies.

Ce ne sont pas des preuves du runtime lui-même. L'alignement entre les modèles Coq et le code Rust est maintenu
manuellement - les commentaires en tête de chaque fichier `.v` citent les définitions correspondantes.

## Vue d'ensemble

### A. Preuves syntaxiques

Prouvent précédence, associativité, non-ambiguïté et chaînage pour le modèle de parsing.

| Fichier                 | Couverture                                                                    | Théorèmes clés                                                                                        |
| ----------------------- | ----------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `GrammarProof.v`        | CFG formelle (S -> AB), arbres, unicité, non-ambiguïté via yield              | `tree_sound`, `grammar_unambiguous_S`, `yield_injective`, `grammar_unambiguous`                       |
| `CatnipAddMulProof.v`   | `+`/`*`, précédence, associativité gauche, monotonie fuel, soundness          | `fuel_mono`, `precedence_general`, `parser_sound`, `parse_full_sound`                                 |
| `CatnipExprProof.v`     | Tour complet (or > and > not > cmp > add > mul), chaînage, desugaring         | `not_and_or_precedence`, `extract_chain`, `chain_desugar_correct_single`, `chain_desugar_correct_two` |
| `CatnipExprMonoProof.v` | Monotonie fuel pour le parseur d'expressions complet (12 fonctions mutuelles) | `fuel_mono`, `parse_bool_or_mono`, `parse_full_mono`                                                  |

### B. Preuves sémantiques

Prouvent cohérence, confluence, terminaison partielle et universalité du modèle dimensionnel.

| Fichier                     | Couverture                                                                              | Théorèmes clés                                                                                             |
| --------------------------- | --------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `CatnipDimensional.v`       | Broadcast, cohérence (lois de foncteur), confluence, filtrage, masque booléen           | `coherence_composition`, `eval_deterministic`, `eval_fusion`, `mask_map_commute`                           |
| `CatnipDimensionalProps.v`  | Universalité, lois structurelles, homomorphisme, algèbre de pipelines                   | `universality`, `broadcast_unique`, `broadcast_concat`, `pipeline_normalization`                           |
| `CatnipNDRecursion.v`       | ND-récursion fuel-bounded, monotonie, déterminisme, terminaison partielle, mémoisation  | `nd_eval_mono`, `nd_eval_deterministic`, `nd_partial_termination`, `memo_coherence`                        |
| `CatnipBroadcastOverload.v` | Broadcast/ND sous surcharge opérateurs : invariants de shape, composition, déterminisme | `overloaded_broadcast_preserves_length`, `overloaded_broadcast_composition`, `nd_overloaded_deterministic` |

### C. Modèle IR

Formalise la structure de l'IR et ses invariants structurels.

| Fichier      | Couverture                                     | Théorèmes clés                                                               |
| ------------ | ---------------------------------------------- | ---------------------------------------------------------------------------- |
| `CatnipIR.v` | IROpCode, IR, bijection, classification, arity | `opcode_to_nat_injective`, `opcode_roundtrip`, `control_flow_not_arithmetic` |

### D. Preuves runtime

Prouvent les invariants des composants d'exécution : scopes, pattern matching, fonctions.

| Fichier                 | Couverture                                                                         | Théorèmes clés                                                                                                                    |
| ----------------------- | ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `CatnipScopeProof.v`    | Lookup/set O(1), shadowing, push/pop, frames isolées                               | `scope_set_lookup_same`, `scope_push_pop`, `scope_shadowing`, `scope_pop_restores`                                                |
| `CatnipPatternProof.v`  | 6 types de patterns, guards, dispatch, déterminisme                                | `wildcard_always_matches`, `or_first_match_wins`, `match_pattern_deterministic`                                                   |
| `CatnipFunctionProof.v` | Binding params (positional, defaults), trampoline TCO, scope depth, tail detection | `bind_params_exact_length`, `trampoline_normal_terminates`, `trampoline_preserves_scope_depth`, `tail_position_produces_tailcall` |

### E. Preuves d'optimisation

Prouvent la correction des 5 passes IR vivantes du pipeline, avec des gardes `*_untouched` sur les règles retirées.

| Fichier                    | Couverture                                                                                                          | Théorèmes clés                                                                     |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `CatnipStrengthRedProof.v` | Strength reduction (And/Or sur 2 littéraux Bool + gardes sur identités retirées)                                    | `sr_and_bools`, `sr_mul_one_untouched`, `strength_reduce_sound`                    |
| `CatnipBluntCodeProof.v`   | Blunt code (inversion Eq↔Ne, And/Or Bool/Bool, complément + gardes)                                                 | `blunt_not_eq`, `blunt_and_complement`, `simplify_blunt_bool_sound`                |
| `CatnipDCEFlattenProof.v`  | DCE, block flattening, composition de passes, lowering IR                                                           | `flatten_stmts_idempotent`, `flatten_block_sound`, `compose_preserves_eval`        |
| `CatnipOptimProof.v`       | Façade (`Require Export` des 3 fichiers ci-dessus)                                                                  | -                                                                                  |
| `CatnipConstFoldProof.v`   | Constant folding (arith+cmp+bool+bitwise), guards div/0 et b\<0                                                     | `cf_add_fold`, `cf_truediv_fold`, `cf_pow_fold`, `cf_band_fold`, `cf_add_fold_sem` |
| `CatnipTailRecLoopProof.v` | Équivalence tail-récursion/boucle (invariant du TCO vivant), tail calls généralisés (mutuelle), two-phase rebinding | `tail_rec_to_loop_correct`, `tramp_eq_eval_rec`, `two_phase_uses_original_env`     |
| `CatnipPurityProof.v`      | Pureté sous surcharge opérateurs : struct ops jamais classés purs (JIT)                                             | `overloaded_op_never_eligible`, `desugared_builtins_are_pure`, `call_not_pure`     |

### F. Preuves d'analyse et CFG

Prouvent la correction de l'analyse de liveness, de la dominance, et du pipeline CFG/SSA.

| Fichier                     | Couverture                                                                                                                                                                                                                                                                                                    | Théorèmes clés                                                                                                                     |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `CatnipVarSet.v`            | Bibliothèque VarSet réutilisable : add, union, remove, NoDup, subset                                                                                                                                                                                                                                          | `add_preserves_nodup`, `union_preserves_nodup`, `remove_list_subset`                                                               |
| `CatnipLivenessLinear.v`    | Liveness linéaire : USE/DEF, transfer, backward analysis, DSE linéaire                                                                                                                                                                                                                                        | `live_in_sound`, `dse_linear_correct`, `transfer_monotone`                                                                         |
| `CatnipLivenessCFG.v`       | Liveness CFG : LiveMap, fixpoint itératif, DSE CFG, path soundness                                                                                                                                                                                                                                            | `dse_cfg_sound_head`, `exec_path_sound`, `iterate_cfg_stable`                                                                      |
| `CatnipLivenessProof.v`     | Façade (`Require Export` des 3 fichiers ci-dessus)                                                                                                                                                                                                                                                            | -                                                                                                                                  |
| `CatnipDominanceProof.v`    | Dominance CFG : réflexivité, transitivité, antisymétrie, idom unicité, frontières                                                                                                                                                                                                                             | `dom_refl`, `entry_dom_all`, `dom_trans`, `dom_antisym`, `idom_unique`, `entry_frontier_empty`                                     |
| `CatnipCFGSSABase.v`        | SSA base : modèle SSA, utilitaires, modèles opérationnels (construction SSA, use-count, DSE)                                                                                                                                                                                                                  | `ssaval_eqb_eq`, `unique_def_from_def_block`, `no_dup_phi_from_lookup`, `seal_block_sealed`, `dse_iterate_mono`                    |
| `CatnipCFGSSACorrectness.v` | SSA correctness (49 lemmes/théorèmes, 0 Admitted) : single assignment, phi-nodes, CSE, GVN, LICM, DSE, destruction                                                                                                                                                                                            | `single_assignment`, `def_before_use`, `phi_at_frontier`, `cse_same_key_same_value`, `licm_hoist_sound`, `dse_fixpoint_terminates` |
| `CatnipCFGSSAProof.v`       | Façade de compatibilité (`Require Export` de `CatnipCFGSSABase` + `CatnipCFGSSACorrectness`)                                                                                                                                                                                                                  | -                                                                                                                                  |
| `CatnipParallelCopyProof.v` | Destruction SSA : séquencement des copies parallèles (swap/lost-copy, Briggs 1998 / Boissinot 2009), cassage de cycle par un scratch ; gate liveness (une copie morte est invisible)                                                                                                                          | `cycle_break_correct`, `cycle_break_outside`, `exec_frame_off`, `dead_copy_invisible`                                              |
| `CatnipTrivialPhiProof.v`   | Destruction SSA : résolution des phi triviaux (`canonical`) — un phi à opérandes tous égaux dénote son opérande ; la résolution préserve la valeur runtime à toute profondeur                                                                                                                                 | `trivial_phi_value`, `resolve_preserves`                                                                                           |
| `CatnipRegionMergeProof.v`  | Reconstruction (`region.rs`) : la recherche du merge d'un if en marche forward-only (back-edges coupées) retourne le merge du builder quel que soit l'ordre de parcours ; le contre-exemple du bug (boucle nichée dans une branche, atteinte depuis la branche sœur via une back-edge) est épinglé en concret | `merge_unique_minimal`, `candidate_behind_merge`, `forward_walk_stops_at_merge`                                                    |
| `CatnipDestructionBridge.v` | Destruction SSA : raccord des modèles ParallelCopy (état abstrait) et Correctness (`Env = SSAVal → nat`) — le lot de copies séquentialisé d'une arête réalise la sémantique parallèle de tous les phis du join                                                                                                | `cycle_break_env_correct`, `cycle_batch_realizes_phi`, `cycle_batch_preserves_others`                                              |

### G. Preuves runtime avancées

Prouvent les invariants des composants runtime avancés.

| Fichier                       | Couverture                                                                                                                                           | Théorèmes clés                                                                                                                                                                           |
| ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CatnipNanBoxProof.v`         | NaN-boxing VM : 8 tags (SmallInt/Bool/Nil/Symbol/PyObj/Struct/BigInt/VMFunc), encoding 47-bit, promotion Phase 5 (arith int générique)               | `tag_injective`, `encode_decode_roundtrip`, `smallint_range`, `promote_demote_id`, `to_bigint_small_exact`, `int_paths_agree`, `i64_overflow_never_demotes`                              |
| `CatnipValueClassProof.v`     | Classification canonique des tags (Scalar/Index/Pointer), exhaustivité, disjointness                                                                 | `class_exhaustive`, `scalar_pointer_disjoint`, `classify_scalar`                                                                                                                         |
| `CatnipBoundaryProof.v`       | Sûreté de frontière : `from_raw_scalar` n'émet jamais un tag Pointer/Index depuis des bits bruts                                                     | `from_raw_scalar_class_scalar`, `from_raw_scalar_rejects_pointer`, `validate_index_sound`                                                                                                |
| `CatnipPluginBoundaryProof.v` | Frontière FFI plugin (ABI v5) : canal non-flaggé scalaire-only, OBJECT/HOSTVALUE sanctionnés                                                         | `unflagged_pointer_rejected`, `unflagged_admits_only_scalar`, `hostvalue_admitted`                                                                                                       |
| `CatnipOwnershipProof.v`      | Refcount des slots struct cross-VM : copy/transfer-transplant + proxy échappé ré-ancré au transplant (pass-through non ré-ancré : garde over-decref) | `copy_leaks`, `transfer_no_leak`, `reanchor_no_leak`, `passthrough_reanchor_underflows`, `transfer_preserves_invariant`                                                                  |
| `CatnipVMOpCode.v`            | VM opcodes (83), bijection `VMOpCode <-> nat`, injectivité, range [1..83]                                                                            | `vm_opcode_to_nat_injective`, `vm_opcode_roundtrip`, `nat_to_vm_opcode_roundtrip`                                                                                                        |
| `CatnipVMState.v`             | Stack effect model (`Fixed`/`ArgDependent`), `VMState` record, classification                                                                        | `fixed_effect_count`                                                                                                                                                                     |
| `CatnipVMStackSafety.v`       | Stack safety, net effects par catégorie, arg-dependent effects, instruction sequences                                                                | `stack_safety_fixed`, `exec_seq_app`, `call_stack_safety`, `exit_stack_safety`, `membership_net_effect`                                                                                  |
| `CatnipVMInvariants.v`        | Compilation invariants : expression net +1, statement net 0, DupTop/RotTwo                                                                           | `load_net_plus_one`, `binop_pattern_depth`, `assignment_pattern_depth`                                                                                                                   |
| `CatnipVMExamples.v`          | 10 exemples concrets, classification completeness (effect_total, arg_dependent_opcodes)                                                              | `effect_total`, `arg_dependent_opcodes`                                                                                                                                                  |
| `CatnipVMFrame.v`             | VM frames (locals, IP, jumps, block stack, ForRange encoding roundtrips)                                                                             | `get_set_same`, `ip_advance_in_bounds`, `jump_preserves_bounds`, `push_pop_restores`, `for_range_full_roundtrip`                                                                         |
| `CatnipArithProof.v`          | Floor div/mod (Python semantics), equality, overflow promotion                                                                                       | `floor_div_mod_identity`, `floor_mod_sign`, `exact_div_mod_zero`, `neg_overflow_only_min`                                                                                                |
| `CatnipPureFrameProof.v`      | PureFrame bind_args, copy_args, fill_defaults, pool alloc/free                                                                                       | `copy_args_slot_bound`, `bind_args_length`, `bind_args_no_defaults`, `pool_round_trip`                                                                                                   |
| `CatnipVMProof.v`             | Façade (`Require Export` des 5 modules VM + `CatnipVMFrame`)                                                                                         | -                                                                                                                                                                                        |
| `CatnipMROC3Core.v`           | C3 merge algorithm, self-first property                                                                                                              | `c3_self_first`, `c3_self_is_head`, `c3_no_parents`                                                                                                                                      |
| `CatnipMROC3Properties.v`     | C3 local precedence and monotonicity                                                                                                                 | `c3_preserves_local_precedence`, `c3_monotonicity`, `c3_merge_preserves_order`                                                                                                           |
| `CatnipMROFields.v`           | MRO field merge, diamond dedup, redefinition detection                                                                                               | `dedup_at_most_once`, `no_redefinition_correct`, `redefinition_detected`                                                                                                                 |
| `CatnipMROMethods.v`          | MRO method resolution, left-priority                                                                                                                 | `left_priority`, `merge_methods_subset`                                                                                                                                                  |
| `CatnipMROSuper.v`            | Super resolution, cooperative termination                                                                                                            | `super_at_self`, `super_at_end`, `super_max_steps`, `super_from_last_is_empty`                                                                                                           |
| `CatnipMROExamples.v`         | Exemples concrets (diamond, linear, inconsistent, init chain)                                                                                        | `diamond_c3`, `inconsistent_c3`, `diamond_method_resolution`, `super_from_B_in_diamond`                                                                                                  |
| `CatnipMROProof.v`            | Façade (`Require Export` des 6 modules MRO ci-dessus)                                                                                                | -                                                                                                                                                                                        |
| `CatnipOpDesugar.v`           | Desugaring opérateurs : symbol x arity -> method name, injectivité, totalité                                                                         | `desugar_injective`, `desugar_total`                                                                                                                                                     |
| `CatnipOpDesugarProps.v`      | Disambiguation +/-, distinctness, résolvabilité méthode, cohérence opcode, préfixe op\_                                                              | `arity_disambiguation_minus`, `desugar_names_distinct`, `desugar_method_resolvable`, `desugar_opcode_consistent`                                                                         |
| `CatnipOpDesugarExamples.v`   | Exemples concrets (Vec2, disambiguation unaire/binaire, cas négatifs)                                                                                | `vec2_find_add`, `minus_as_binary`, `minus_as_unary`, `eq_not_unary`                                                                                                                     |
| `CatnipOpDesugarSemantics.v`  | Préservation sémantique : dispatch VM = appel méthode, reverse dispatch, roundtrip opcode, déterminisme                                              | `operator_dispatch_is_method_call`, `dispatch_finds_same_method`, `reverse_dispatch_finds_method`, `forward_priority_over_reverse`, `rev_dispatch_subsumes_old`, `no_dispatch_ambiguity` |
| `CatnipStructProof.v`         | Structs/traits : field access O(1), method resolution, inheritance, super chain                                                                      | `field_access_sound`, `method_resolution_order`, `super_chain_terminates`, `trait_linearization`                                                                                         |
| `CatnipCacheKey.v`            | Cache keys : CacheType, encoding Z, injectivité, disjointness                                                                                        | `cache_key_injective`, `cache_type_disjoint`, `cache_key_bijection`                                                                                                                      |
| `CatnipCacheMemory.v`         | MemoryCache FIFO : find/remove/set, key uniqueness, round-trip, eviction                                                                             | `mc_set_get_same`, `mc_fifo_evicts_oldest`, `mc_set_size_bounded`                                                                                                                        |
| `CatnipCacheDisk.v`           | DiskCache LRU+TTL : expiration, prune, eviction, atomic writes                                                                                       | `dc_get_ttl_enforcement`, `dc_lru_evict_size`, `atomic_write_no_partial`                                                                                                                 |
| `CatnipCacheAdapter.v`        | CatnipCache adapter, Memoization, invalidation (16 keys)                                                                                             | `memo_set_get_same`, `invalidation_covers_all`, `all_invalidation_keys_nodup`                                                                                                            |
| `CatnipCacheProof.v`          | Façade (`Require Export` des 4 fichiers ci-dessus)                                                                                                   | -                                                                                                                                                                                        |
| `DeltaCollection.v`           | Noyau delta : la compaction (somme par clé + zéros retirés, le repli IndexMap du Rust) est inobservable sur les sommes                               | `compaction_preserves_sum`, `apply_compact_eq`, `empty_delta_neutral`, `negate_roundtrip`                                                                                                |
| `DeltaStateless.v`            | Opérateurs stateless : map/filter respectent la compaction et distribuent sur la concaténation, concat = union                                       | `map_respects_compaction`, `map_additive`, `sum_filter_delta`, `concat_union`, `from_deltas_observable`                                                                                  |
| `DeltaProof.v`                | Façade (`Require Export` des 2 fichiers ci-dessus)                                                                                                   | -                                                                                                                                                                                        |

## A. Preuves syntaxiques

### GrammarProof.v

Modèle minimal d'une CFG (S -> A B, A -> "a", B -> "b") avec trois résultats :

**Soundness des arbres** : tout arbre de dérivation produit une séquence de terminaux dérivable depuis le non-terminal
racine (`tree_sound`). La preuve construit la chaîne de réécritures explicitement.

**Génération** : la grammaire engendre bien `[a; b]` (`generates_example_ab`), par application successive des trois
productions.

**Non-ambiguïté structurelle** : pour chaque non-terminal, il n'existe qu'une seule forme d'arbre
(`grammar_unambiguous_S`). La preuve procède par destruction dépendante - Coq élimine structurellement toute
alternative.

**Non-ambiguïté via yield** : formulation standard en théorie des langages - si deux arbres pour le même non-terminal
produisent la même chaîne de terminaux, les arbres sont identiques (`grammar_unambiguous`). La preuve passe par
`yield_injective` (injectivité de la fonction yield), puis `congruence`. La complétude est aussi prouvée : tout arbre
pour S engendre `[ta; tb]` et dérive via la relation (`yield_S_unique`, `tree_complete_S`).

Ce fichier ne modélise pas directement le parseur Catnip. Il pose le vocabulaire (dérivation, arbre, ambiguïté) utilisé
implicitement par les deux fichiers suivants.

### CatnipAddMulProof.v

Formalise le fragment arithmétique de `grammar.js` :

```
_additive       -> additive | _multiplicative
additive        -> _additive ("+" | "-") _multiplicative   (left)
_multiplicative -> multiplicative | _exponent
multiplicative  -> _multiplicative ("*" | "/" | "//" | "%") _exponent (left)
```

Le modèle Coq garde `+` et `*` avec atomes parenthésés. Cinq fonctions mutuellement récursives (`parse_expr`,
`parse_expr_tail`, `parse_term`, `parse_term_tail`, `parse_factor`) implémentent un RDP classique.

**Précédence et associativité** (exemples concrets) :

- **Précédence** (`precedence_example`) : `n + n * n` parse comme `n + (n * n)`, pas `(n + n) * n`
- **Associativité gauche** (`left_assoc_add_example`) : `n + n + n` parse comme `(n + n) + n`
- **Parenthèses** (`paren_example`) : `(n + n) * n` parse comme `(n + n) * n` - les parenthèses forcent la structure

Le déterminisme (`parse_expr_deterministic`) est trivial : deux appels sur la même entrée produisent le même résultat,
par réécriture et inversion.

**Monotonie du fuel** (`fuel_mono`) : si le parseur réussit avec un fuel `f`, il réussit avec le même résultat pour tout
`f' >= f`. Le théorème est un 5-conjonction prouvé par induction sur le fuel, avec lemmes de dépliage
(`parse_expr_unfold`, etc.) pour exposer la structure récursive. Les corollaires `parse_expr_mono` et `parse_full_mono`
fournissent les projections utiles.

**Résultats indépendants du fuel** : avec la monotonie, les propriétés de précédence et d'associativité sont
généralisées à tout fuel suffisant (`precedence_general : forall fuel, fuel >= 5 -> ...`). La tactique
`solve_fuel_general` automatise la preuve : fournir un témoin concret, puis `lia`.

**Soundness** : une relation de parsing inductive (`parses_expr`, `parses_expr_tail`, etc.) avec 8 constructeurs miroir
la structure du parseur. Le théorème `parser_sound` (5-conjonction par induction sur le fuel) prouve que si le parseur
retourne `Some`, le résultat satisfait la relation. Le corollaire `parse_full_sound` combine soundness et consommation
complète de l'entrée.

### CatnipExprProof.v

Étend le modèle au tour de précédence complet de Catnip :

```
bool_or > bool_and > bool_not > comparison > additive > multiplicative > atom
```

Douze fonctions mutuellement récursives (6 niveaux de précédence, chacun avec sa tail function, plus `parse_bool_not` et
`parse_atom`). Les tokens couvrent les 4 opérateurs arithmétiques (`+`, `-`, `*`, `/`), les 6 opérateurs de comparaison
(`<`, `<=`, `>`, `>=`, `==`, `!=`), les 3 opérateurs booléens (`and`, `or`, `not`), les parenthèses et les littéraux.

> Douze fonctions mutuellement récursives. Coq n'a pas bronché. Le reviewer, si.

**Précédence arithmétique** : `*` lie plus fort que `+` (`mul_over_add`), `-` est associatif gauche (`sub_left_assoc`).

**Précédence booléenne** : `not > and > or` vérifié sur `not n < n or false` qui parse comme `(not (n < n)) or false`
(`not_and_or_precedence`). Les comparaisons lient plus fort que `and` (`cmp_over_and`).

**Parenthèses** : forcent la structure à travers tous les niveaux (`paren_override_bool`).

**Opérateurs de comparaison** : les 6 opérateurs sont testés individuellement (`comparison_ops_examples`) - chacun
produit le bon nœud AST.

**Chaînage** : `n < n <= n` parse comme `(n < n) <= n` syntaxiquement (`comparison_chain_example`). Le fichier inclut
aussi un modèle sémantique : `eval_comp_chain` évalue une chaîne de comparaisons comme une conjonction. Le théorème
`chain_two_ops_desugars_to_and` prouve que `a op1 b op2 c` est équivalent à `(a op1 b) and (b op2 c)`.

**Desugaring des chaînes** : `extract_chain` extrait récursivement la base et les opérateurs d'un AST de comparaisons
imbriquées à gauche. Trois propriétés structurelles prouvées : la chaîne n'est jamais vide (`extract_chain_nonempty`),
la base n'est jamais elle-même une comparaison (`extract_chain_base_non_cmp`), et les non-comparaisons donnent `None`
(`extract_chain_non_cmp`). La correction sémantique est prouvée pour les chaînes à 1 et 2 opérateurs
(`chain_desugar_correct_single`, `chain_desugar_correct_two`).

### CatnipExprMonoProof.v

Monotonie du fuel pour le parseur d'expressions complet, split de `CatnipExprProof.v` pour réduire la mémoire de
compilation.

**Monotonie du fuel** (`fuel_mono`) : même structure que `CatnipAddMulProof.v`, mais avec 12 conjonctions (une par
fonction mutuellement récursive). Les 6 opérateurs de comparaison sont traités uniformément dans `parse_comparison_tail`
par semi-colon tactique. Corollaires : `parse_bool_or_mono`, `parse_full_mono`.

Les lemmes de dépliage (`parse_*_unfold`) exposent la structure récursive des 12 fonctions pour que les hypothèses
d'induction soient applicables.

## B. Preuves sémantiques

### CatnipDimensional.v

Formalise le modèle dimensionnel de Catnip : le domaine de valeurs (`Scalar nat | Coll (list Val)`), le broadcast
(`.[op]`), et le filtrage (`.[if p]`).

Trois groupes de résultats :

**Cohérence (lois de foncteur)** : `broadcast_map` satisfait identité (`v.[id] = v`) et composition
(`xs.[f].[g] = xs.[g . f]`). Le broadcast préserve la longueur des collections et fixe le topos vide.

**Confluence** : l'évaluation des expressions broadcast est totale et déterministe (`eval_deterministic`). Si une
expression évalue vers `v1` et `v2`, alors `v1 = v2`. La fusion de broadcasts chaînés est prouvée (`eval_fusion`).

**Propriétés de base** : masque booléen (`mask_select`), sémantique un-niveau (`broadcast_shallow`), absorption de
filtres (`filter_filter`).

### CatnipDimensionalProps.v

Propriétés structurelles et algébriques avancées, split de `CatnipDimensional.v` pour réduire la mémoire de compilation.

**Universalité** : toute opération *elementwise* (qui préserve la longueur et commute avec l'extraction par index) sur
une collection est nécessairement `Coll (map op xs)` - c'est-à-dire `broadcast_map op` (`universality`). Il n'existe pas
d'autre façon de lifter une opération aux collections avec ces propriétés (`broadcast_unique`). Pour les collections
plates, le broadcast est entièrement déterminé par son noyau scalaire (`broadcast_minimal_flat`). Le broadcast Catnip
est caractérisé comme le foncteur libre sur les collections plates sous les axiomes d'elementwise-ness.

<!-- check: no-check -->

```catnip
# Le broadcast est un foncteur libre (loi de composition)
# Pour des fonctions pures f et g :

# F(id) = id
data.[x => x] == data

# F(f . g) = F(f) . F(g)
# viva la composition
data.[g].[f] == data.[x => f(g(x))]
```

**Propriétés non triviales** : lois structurelles prouvées par induction :

- *Filter-map pullback* (`filter_map_pullback`) : filtrer après mapper = mapper après filtrer avec le prédicat tiré en
  arrière `p . f`. Généralise `filter_map_commute` sans hypothèse d'invariance.
- *Absorption de filtres* (`filter_filter`) : deux filtres successifs = un filtre avec conjonction des prédicats.
- *Pullback broadcast-filter-map* (`broadcast_filter_map`) : corollaire au niveau `Val`.
- *Masque booléen* (`mask_select`) : all-true = identité (`mask_all_true`), all-false = vide (`mask_all_false`),
  commutation avec map (`mask_map_commute`), borne de longueur (`mask_length_le`).
- *Homomorphisme de liste* (`broadcast_concat`) : le broadcast distribue sur la concaténation - fondation algébrique de
  la parallélisation.
- *Sémantique un-niveau* (`broadcast_shallow`, `broadcast_two_levels`) : le broadcast opère à exactement un niveau de
  profondeur ; le broadcast récursif nécessite un emboîtement explicite.
- *Homomorphisme de monoide* (`fold_broadcast_exchange`) : quand `f` distribue sur `op` et fixe `z`, `f` commute avec le
  fold. Corollaire : fold sur un broadcast = fold avec accumulateur composé (`fold_broadcast_map`).

**Algèbre de pipelines** : formalise les pipelines comme séquences d'opérations `PMap` / `PFilt` et prouve trois
résultats :

- *Fusion complète* (`map_chain_fusion`) : toute chaîne de n broadcasts maps se réduit à un seul broadcast avec la
  fonction composée. Sûre (préserve la sémantique) et complète (n quelconque).
- *Équivalence transformationnelle* : trois règles de réécriture prouvées correctes - map; map fusionne
  (`equiv_map_map`), map;filter se réordonne en filter;map (`equiv_filter_map_swap`), filter;filter s'absorbe
  (`equiv_filter_filter`).
- *Normalisation* (`pipeline_normalization`) : tout pipeline mixte de maps et filters sur une collection se réduit à une
  forme canonique `filter;map`. Deux pipelines qui ont la même forme normale sont équivalents (`normalization_sound`).

### CatnipNDRecursion.v

Modèle paramétrique de la ND-récursion (`~~(seed, lambda)`), split de `CatnipDimensional.v`. Indépendant du domaine de
valeurs.

**Monotonie** (`nd_eval_mono`) : si l'évaluation réussit avec un fuel `f`, elle réussit avec le même résultat pour tout
`f' >= f`.

**Déterminisme** (`nd_eval_deterministic`) : deux évaluations avec un fuel suffisant produisent le même résultat.

**Terminaison partielle** (`nd_partial_termination`) : quand `step_seed` garantit une mesure strictement décroissante,
la récursion termine pour un fuel suffisant.

**Mémoisation** (`memo_coherence`) : si une valeur est dans le cache pour un seed, elle correspond au résultat de
l'évaluation.

## C. Modèle IR

### CatnipIR.v

Formalise les opcodes de `catnip_rs/src/ir/opcode.rs` comme un inductif Coq `IROpCode`. Chaque opcode reçoit une
numérotation via `opcode_to_nat : IROpCode -> nat` (bijection) et `nat_to_opcode` inverse.

**Injectivité et roundtrip** : `opcode_to_nat_injective` (numérotation injective), `opcode_roundtrip` (aller-retour nat
-> opcode -> nat = identité). Prouvés par énumération exhaustive.

**Classification** : prédicats `is_literal`, `is_op`, `is_pattern`, `is_collection`, `is_comparison_op`,
`is_control_flow_op`, `is_arithmetic_op` avec propriétés de disjointness (`literal_not_op`,
`control_flow_not_arithmetic`).

**IR** : inductif représentant les nœuds IR (IRInt, IRFloat, IRBool, IRStr, IRNone, IRDecimal, IRImaginary, IROp), avec
`ir_size` (taille structurelle), `ir_op` (constructeur par opcode + args), `ir_binop` (spécialisation binaire).

Ce fichier sert de fondation importée par `CatnipStrengthRedProof.v`, `CatnipBluntCodeProof.v` et
`CatnipDCEFlattenProof.v`.

## D. Preuves runtime

### CatnipScopeProof.v

Modélise le système de scopes de `catnip_rs/src/core/scope.rs`. Un environnement (`env`) est un `string -> option Z`, un
scope est une pile de frames avec opérations `push_scope` / `pop_scope`.

**Lookup/set** : `env_lookup_set_same` (lire ce qu'on vient d'écrire), `env_lookup_set_other` (écrire ne touche pas les
autres variables). Même résultats liftés au scope : `scope_set_lookup_same`, `scope_set_lookup_other`.

**Push/pop** : `scope_push_pop` (push puis pop = identité pour les variables pré-existantes),
`scope_push_preserves_lookup` (push ne touche pas les variables existantes).

**Shadowing** : `scope_shadowing` (une variable dans un frame enfant masque le parent), `scope_pop_restores` (pop
restaure la valeur d'avant le push), `scope_shadow_restore` (composition des deux).

**Frames isolées** : modèle étendu (`scope_ex`) avec flag d'isolation par frame. `scopeex_isolated_shadow` (les frames
isolées ne voient pas les variables du parent), `scopeex_isolated_restore` (le pop restaure correctement même avec
isolation).

### CatnipPatternProof.v

Modélise les 6 types de patterns de `catnip_rs/src/core/pattern.rs`. Chaque pattern est un inductif (`PatWild`,
`PatLit`, `PatVar`, `PatOr`, `PatTuple`, `PatStruct`) avec `match_pattern : Pattern -> Value -> option Bindings`.

**Wildcard** : matche toujours, ne capture rien (`wildcard_always_matches`, `wildcard_no_bindings`).

**Variable** : matche toujours, capture la valeur (`var_always_matches`, `var_captures_value`, `var_single_binding`).

**Literal** : matche si et seulement si les valeurs sont égales (`literal_matches_equal`, `literal_rejects_different`).

**OR** : premier match gagne (`or_first_match_wins`), singleton = pattern nu (`or_singleton`), wildcard dans un OR
attrape tout (`or_with_wildcard`).

**Tuple/Struct** : mismatch de longueur = échec (`tuple_length_mismatch`), type mismatch = échec
(`struct_type_mismatch`).

**Dispatch** : `match_cases_first_wins` (premier case qui matche est sélectionné), `match_cases_guard_fail` (si le guard
échoue, on passe au case suivant), `match_cases_wildcard_catches_all` (un wildcard en dernier case attrape tout).

**Déterminisme** : `match_pattern_deterministic` et `match_cases_deterministic` - le résultat du match est unique.

### CatnipFunctionProof.v

Modélise les fonctions et le trampoline TCO de `catnip_rs/src/core/registry/functions.rs` et
`catnip_rs/src/core/nodes.rs`.

**Binding de paramètres** : `bind_params` prend une liste de specs `(name, default)` et une liste d'arguments, produit
un environnement. `bind_params_exact_length` (si autant d'args que de params, chaque param reçoit l'arg correspondant),
`bind_params_all_defaults` (0 args = tous les defaults), `bind_params_missing_required` (param sans default et sans arg
= erreur).

**Trampoline TCO** : modèle fuel-bounded `trampoline fuel scope body`. Le body produit soit un `Normal v` (terminaison),
soit un `Tail args` (tail call = rebind + continuer). `trampoline_normal_terminates` (un Normal termine immédiatement),
`trampoline_tail_continues` (un Tail rebind les params et relance), `trampoline_fuel_monotone`, `trampoline_two_steps`,
`trampoline_three_steps` (exemples multi-itérations).

**Scope** : `trampoline_preserves_scope_depth` - le trampoline ne modifie pas la profondeur du scope.

**Tail detection** : `tail_position_produces_tailcall` (si le TCO est actif et la position est tail, un appel récursif
produit un TailCall), `non_tail_produces_normal` (en position non-tail, l'appel est normal),
`tco_disabled_produces_normal` (si TCO désactivé, pas de TailCall).

## E. Preuves d'optimisation

### CatnipOptimProof.v (façade)

Façade qui ré-exporte les 3 modules suivants. Code modélisé : les passes de `catnip_core/src/semantic/passes/`
(`strength_reduction.rs`, `blunt_code.rs`, `dead_code_elimination.rs`, `block_flattening.rs`).

Le modèle d'expressions `Expr` (Const, BConst, Var, BinOp, UnOp, IfExpr, WhileExpr, Block, MatchExpr) avec évaluateur
partiel `eval_expr` est défini dans `CatnipExprModel.v`.

**Réalignement (2026-06-11)** : les modèles `strength_reduce` et `simplify_blunt` reflètent les passes vivantes
post-revue. Les identités retirées du code (`x * 0 → 0`, `x ** 2 → x * x`, `not not x → x`, `not (a < b) → a >= b`,
`x == True → x`, idempotence, règles booléennes unilatérales) ne sont plus prouvées comme réécritures : elles sont
épinglées par des théorèmes de garde `*_untouched` qui certifient que la passe **ne** réécrit **pas** ces formes. Un
changement de comportement du modèle casse le théorème de garde correspondant.

### CatnipStrengthRedProof.v

**Strength reduction** (`strength_reduce : Expr -> Expr`) : les deux réécritures vivantes -- `And`/`Or` quand les
**deux** opérandes sont des littéraux booléens (`sr_and_bools`, `sr_or_bools` : pliage en `BConst (andb/orb a b)`). 11
gardes `*_untouched` épinglent l'absence des identités arithmétiques et des règles unilatérales. Théorème principal :
`strength_reduce_sound` -- la passe préserve `eval_expr` **exactement** (plus fort que l'ancien
`strength_reduce_bool_sound`, qui ne préservait que la truthiness : conséquence directe d'exiger deux littéraux).

### CatnipBluntCodeProof.v

**Blunt code** (`simplify_blunt : Expr -> Expr`) : inversion `not (a == b) → a != b` restreinte à Eq↔Ne
(`invert_cmp_involution`, `invert_cmp_negates` ; `invert_cmp_order_none` épingle l'exclusion des comparaisons d'ordre,
invalides sous NaN), `And`/`Or` sur deux littéraux booléens, complément (`x && not x = False`, `x || not x = True`,
prouvé sur expressions pures -- le modèle n'a pas d'effets), élagage `if` à condition constante. 9 gardes `*_untouched`
(double négation, `x == True`, idempotence, inversions d'ordre). Les preuves de complément utilisent des lemmes de
taille structurelle (`expr_eqb_not_self`). Inclut `expr_eqb_eq` (réflexion de l'égalité structurelle). Théorème
principal : `simplify_blunt_bool_sound`.

### CatnipDCEFlattenProof.v

**Dead code elimination** (`eliminate_dead : Expr -> option Expr`) : `if True { t } else { f } -> t`, `if False -> f`,
`while False { body } -> eliminated`, `Block [] -> eliminated`, `Block [e] -> e`. Correction sémantique via `eval_expr`.

**Block flattening** (`flatten_block : Expr -> Expr`) : aplatit les blocs imbriqués
(`Block [s1, Block [s2, s3], s4] -> Block [s1, s2, s3, s4]`). Distributivité sur append (`flatten_stmts_app`).
Idempotence (`flatten_stmts_idempotent`, `flatten_block_idempotent`) prouvée via un lemme intermédiaire : la sortie de
`flatten_stmts` ne contient jamais de `Block` au top-level (`flatten_stmts_output_no_blocks`).

**Composition** : `compose_passes` (fold_left), `preserves_eval` (une passe préserve la sémantique),
`compose_preserves_eval` (la composition de passes qui préservent la sémantique préserve la sémantique),
`compose_two_idempotent` (conditions d'idempotence pour la composition de deux passes).

**Périmètre du modèle** : `Expr` n'a ni assignations ni `match` -- deux comportements vivants restent hors modèle (la
préservation du scrutinee à l'élimination de `match`, et la non-fusion des blocs déclarant des locals).
`CatnipStorePropProof.v` (constant/copy propagation, CSE -- passes retirées du pipeline) a été supprimé avec elles ;
`CatnipTailRecLoopProof.v` prouve l'invariant du mécanisme TCO vivant (trampoline/TailCall) -- sa section
`GeneralTailCalls` modélise le trampoline à cible arbitraire (un signal `Tail s'` peut changer de fonction, donc la
récursion mutuelle est une instance) et le réduit au modèle de base par `tramp_eq_eval_rec`, transportant équivalence
boucle, monotonie du fuel et déterminisme. La liveness de `CatnipLivenessProof.v` est le moteur du linter W312.

> Chaque théorème a du code en face, et chaque règle retirée a un théorème qui vérifie son absence. Le système de preuve
> surveille aussi ce que le compilateur s'abstient de faire.

### CatnipVMOpCode.v

Modélise les 83 opcodes VM de `catnip_core/src/vm/opcode.rs`. Bijection `VMOpCode <-> nat` avec
`vm_opcode_to_nat_injective` et `vm_opcode_roundtrip`. 83 constructeurs (repr(u8) 1..83). Même technique que
`CatnipIR.v` pour les IR opcodes.

### CatnipVMState.v

Définit le modèle d'état VM et la classification des effets de pile. Chaque opcode a un effet `(pops, pushes)`. 68
opcodes ont un effet fixe (connu statiquement), 15 sont arg-dépendants (`Call`, `BuildList`, `BuildDict`, `Exit`, etc.).
Prédicats `is_fixed_effect`, `get_pops`, `get_pushes`.

### CatnipVMStackSafety.v

Prouve la sécurité de pile et les propriétés d'exécution.

**Stack safety** : théorème central `stack_safety_fixed` - pour tout opcode à effet fixe, si la pile a au moins `pops`
éléments, l'exécution produit une pile de profondeur `depth - pops + pushes`, sans underflow. Pour les opcodes
arg-dépendants, safety prouvée paramétriquement (`call_stack_safety`, `build_seq_stack_safety`,
`build_dict_stack_safety`, `unpack_seq_stack_safety`, `exit_stack_safety`).

**Propriétés par catégorie** : net effects uniformes - arithmétique = -1, comparaison = -1, membership/identity = -1,
unaire = 0 (inclut `ToBool`), load = +1, store = -1, noop = 0 (inclut `MatchFail`), conditional jumps = Fixed 1 0
(inclut `JumpIfNotNoneOrPop`), match transforms = Fixed 1 1 (inclut `MatchAssignPatternVM`).

**Exit** : arg-dépendant (`arg=0` : pops 0, `arg=1` : pops 1). `exit_zero_noop`, `exit_one_requires_one`.

**Instruction sequences** : `exec_seq` exécute une liste d'instructions, `exec_seq_app` prouve la composition.

### CatnipVMInvariants.v

Invariants du compilateur. Expression = net +1 (`binop_pattern_depth`, `membership_pattern_depth`), statement = net 0
(`assignment_pattern_depth`, `discard_pattern_depth`). Propriétés DupTop (net +1) et RotTwo (net 0).

### CatnipVMExamples.v

10 exemples concrets d'exécution (addition, assignation, négation, expressions imbriquées, membership, etc.) prouvés par
réflexivité. Classification exhaustive : `effect_total` (tout opcode est Fixed ou ArgDependent), `arg_dependent_opcodes`
(énumération des 15).

### CatnipVMFrame.v

Modélise les frames VM de `catnip_rs/src/vm/frame.rs` et prouve les invariants de gestion mémoire et de contrôle.

**Locals** : modèle de vecteur à taille fixe initialisé à nil. `get_set_same` (roundtrip lecture/écriture),
`get_set_other` (écriture ne touche pas les autres slots), `set_local_preserves_wf` (la taille du vecteur est
préservée).

**IP safety** : `ip_initial_valid` (IP initial en bounds), `ip_advance_in_bounds` (avancer reste en bounds si pas au
dernier), `ip_fetch_some` (fetch réussit si en bounds), `ip_exit` (IP = len(code) signifie terminaison).

**Jump safety** : `is_jump_op_enumerated` (classification exhaustive des opcodes de saut), `jump_preserves_bounds` (un
saut vers une cible valide reste en bounds), `non_jump_advances` (un opcode non-saut avance l'IP de 1),
`jump_ops_fixed_effect` (les jumps ont un effet de pile fixe de 0).

**Block stack** : `push_pop_restores` (push puis pop = identité), `push_block_depth` / `pop_block_depth` (profondeur
incrémente/décrémente), `push_pop_saved_region` (la région sauvegardée est restaurée).

**ForRange encoding** : roundtrips pour le bitpacking `ForRangeInt` (slot_i, slot_stop, step_sign, offset) et
`ForRangeStep` (slot_i, step, target). 7 théorèmes de roundtrip individuels couvrant chaque champ.

### CatnipArithProof.v

Prouve les propriétés des opérations arithmétiques pures de `catnip_core/src/arith.rs` (corps génériques Phase 5,
monomorphisés par `catnip_vm` et `catnip_rs`). Couvre floor division et floor modulo (sémantique Python, distincte de la
division tronquée C), propriétés d'égalité native, et correction de la promotion overflow SmallInt vers BigInt. Les
lemmes de promotion Phase 5 (totalité de `to_bigint`, cohérence fast-path i64 / voie bigint) vivent avec le modèle
d'encodage dans `CatnipNanBoxProof.v`.

Théorèmes clés : `floor_div_mod_identity` (a = q\*b + r pour tout b != 0), `floor_mod_sign` (le reste a le signe du
diviseur), `floor_mod_bound_pos/neg` (bornes du reste), `exact_div_mod_zero` (division exacte implique reste nul),
`neg_overflow_only_min` (seul -SMALLINT_MIN déborde en négation). 10 exemples concrets validés par réflexivité contre
les résultats Python.

### CatnipPureFrameProof.v

Prouve les propriétés spécifiques au PureFrame de `catnip_vm/src/vm/frame.rs` : liaison d'arguments positionnels
(`bind_args`), copie dans les slots locaux, remplissage des valeurs par défaut, et invariants du pool de frames.

Théorèmes clés : `copy_args_slot_bound` (les arguments atterrissent aux bons slots), `copy_args_unbound_nil` (slots non
liés restent Nil), `bind_args_length` (préserve la taille des locals), `bind_args_no_defaults` (sans défauts, les slots
correspondent aux args), `pool_alloc_fresh` / `pool_alloc_all_nil` (alloc produit des locals propres),
`pool_free_bounded` (taille du pool bornée), `pool_round_trip` (free puis alloc = frame propre).

### CatnipVMProof.v

Façade (`Require Export` des 5 modules VM + `CatnipVMFrame`).

### CatnipMRO\*.v (6 modules)

Modélisent la linéarisation C3 et la résolution MRO de `catnip_rs/src/vm/mro.rs`. Standalone, sans dépendances sur les
autres preuves Catnip. `CatnipMROProof.v` est une façade (`Require Export` des 6 modules).

**CatnipMROC3Core.v** : Algorithme C3 merge. `c3_self_first` (la classe courante est toujours en tête du MRO),
`c3_no_parents` (classe sans parents = MRO singleton).

**CatnipMROC3Properties.v** : Propriétés C3 (dépend de C3Core). `c3_preserves_local_precedence` (l'ordre de déclaration
des parents est respecté dans le MRO), `c3_monotonicity` (le MRO d'un parent est un sous-ordre du MRO de l'enfant),
`c3_merge_preserves_order` (l'ordre relatif des séquences d'entrée est préservé dans le résultat).

**CatnipMROFields.v** : Field merge (indépendant du C3). `dedup_at_most_once` (chaque champ apparaît au plus une fois
après fusion), `no_redefinition_correct` / `redefinition_detected` (détection de redéfinition de champs entre classe
enfant et héritage).

**CatnipMROMethods.v** : Method resolution (indépendant du C3). `left_priority` (premier parent dans le MRO gagne pour
la résolution de méthode), `merge_methods_deterministic` (la fusion est déterministe).

**CatnipMROSuper.v** : Super resolution (dépend de Methods). `super_at_self` (super depuis la classe courante commence
au parent suivant dans le MRO), `super_tail_bounded` / `super_max_steps` (la chaîne super termine en au plus `|MRO|`
pas), `super_from_last_is_empty` (super depuis la dernière classe du MRO = vide).

**CatnipMROExamples.v** : Exemples concrets (dépend de tous les modules). Diamond, linear chain, inconsistency, init
chaining, field dedup.

### CatnipOpDesugar\*.v (4 modules)

Modélisent le desugaring des opérateurs surchargés (`op <symbol>`) de
`catnip_core/src/parser/pure_transforms/definitions.rs`. Le mapping `(symbol, arity) → method_name` est prouvé injectif
et total.

**CatnipOpDesugar.v** : Modèle du mapping (19 symboles, 21 combinaisons valides sur 38). `desugar_injective` (deux
couples distincts ne produisent jamais le même nom), `desugar_total` (toute combinaison valide produit un nom).

**CatnipOpDesugarProps.v** : Propriétés dérivées. `arity_disambiguation_minus/plus` (`-` et `+` unaire vs binaire
donnent des noms différents), `desugar_names_distinct` (corollaire d'injectivité), `invalid_combinations_fail` (17 cas
invalides), `desugar_method_resolvable` (connexion avec `find_method` de CatnipStructProof), `desugar_opcode_consistent`
(cohérence avec les IROpCode), `op_prefix_preserved` (tous les noms commencent par `op_`).

**CatnipOpDesugarExamples.v** : Exemples concrets. Vec2 (struct avec méthodes opérateur), disambiguation `-` unaire vs
binaire sur un même struct, cas négatifs (`SymEq Unary = None`, etc.).

**CatnipOpDesugarSemantics.v** : Préservation sémantique du dispatch VM. Modélise le dispatch two-phase de
`vm/core/mod.rs` (native op → struct method) et le dispatch reverse (prim OP struct → méthode du struct).
`operator_dispatch_is_method_call` (pour les structs, le dispatch VM = appel méthode direct),
`reverse_dispatch_finds_method` (prim OP struct dispatche vers la méthode du struct), `forward_priority_over_reverse`
(le dispatch forward a priorité), `rev_dispatch_subsumes_old` (rétro-compatibilité : l'ancien dispatch est un cas
particulier du nouveau), `opcode_roundtrip` (symbol → method name → opcode → même opcode), `no_dispatch_ambiguity` (un
symbole résout vers exactement une méthode).

## F. Preuves d'analyse

### CatnipLivenessProof.v (façade)

Façade qui ré-exporte les 3 modules suivants. Modélise l'analyse de liveness et la dead store elimination (DSE), d'abord
sur des blocs linéaires puis sur un CFG. 48 lemmes/théorèmes.

### CatnipVarSet.v

Bibliothèque réutilisable pour les ensembles de variables (`VarSet = list nat`). Opérations : `vs_add`, `vs_union`,
`vs_remove_var`, `vs_remove_list`. Propriétés : préservation de `NoDup`, inclusion (`vs_subset`), membership. Utilisée
par les deux fichiers suivants.

### CatnipLivenessLinear.v

**Modèle** : variables `nat`, instructions avec ensembles USE/DEF, états `Var -> nat`.

**Liveness linéaire** : la fonction de transfert `transfer` calcule `live_in = (live_out \ defs) ∪ uses`. Monotonie de
la fonction de transfert (`transfer_monotone`). `live_in_sound` : si deux états coïncident sur les variables vivantes en
entrée, ils coïncident sur les variables vivantes en sortie après exécution du bloc.

**DSE linéaire** (`dse_linear_correct`) : les instructions mortes (assignation à une variable non vivante en sortie)
peuvent être supprimées sans changer l'état observable (variables vivantes en sortie).

### CatnipLivenessCFG.v

**CFG** : extension au cas multi-blocs avec table de successeurs. La liveness inter-blocs utilise un fixpoint itératif
(`iterate_cfg`) avec monotonie (`step_in_monotone`) et convergence (`iterate_cfg_stable`).

**DSE CFG** (`exec_path_sound`) : pour tout chemin d'exécution dans le CFG, la DSE guidée par la liveness préserve les
variables vivantes. La preuve procède par induction sur le chemin.

### CatnipParallelCopyProof.v

Destruction SSA : sortir de la forme SSA matérialise les phis en copies aux arêtes prédécesseur. Un lot de copies est
**parallèle** (tous les `dst` lus depuis les valeurs initiales des `src`), et le sérialiser naïvement corrompt une
valeur quand les copies forment un cycle — le problème swap / lost-copy (Briggs et al. 1998 ; Boissinot et al. 2009). Le
modèle formalise l'état (`loc -> nat`), la copie (`do_copy`) et l'exécution séquentielle (`exec`).

**Cassage de cycle** (`cycle_break_correct`) : pour un cycle de longueur quelconque et un scratch frais, la séquence
`(t, x0) :: chain xs t` réalise la rotation parallèle sur toute location ; `cycle_break_outside` : les locations hors
cycle restent intactes. `naive_swap_corrupts` épingle la nécessité du scratch (le swap à deux copies perd une valeur).

**Gate liveness** (`dead_copy_invisible`, via `exec_frame_off`) : omettre une copie dont la destination n'est plus relue
ne change aucune autre location — ce qui justifie que la matérialisation ignore les phis morts (variable non live-in).

### CatnipTrivialPhiProof.v

Destruction SSA : le builder élimine un phi dont tous les opérandes coïncident (Braun et al. 2013) mais laisse la valeur
dans `value_defs`, où un usage périmé peut encore la référencer. La résolution `canonical` la fait pointer vers la
définition survivante.

**Correction** : `trivial_phi_value` — un phi à opérandes tous égaux à `w` dénote `w` quel que soit le prédécesseur
pris. `resolve_preserves` — si chaque maillon de résolution préserve la valeur runtime, la suivre à toute profondeur la
préserve (prouvé pour tout carburant, donc même une chaîne coupée par la garde nomme une valeur égale à l'originale).

### CatnipRegionMergeProof.v

Reconstruction CFG → IR : `find_merge_point` (`region.rs`) prend le premier bloc atteignable depuis les deux branches
d'un if et dominé par son header. Le harnais par propriétés de la Phase 4 a montré que la marche naïve (toutes les
arêtes) est fausse : depuis un `else`, elle remonte la boucle englobante par sa back-edge et atteint les blocs du `then`
frère — le header d'un `while` niché dans le `then` passait pour le merge. Le fix coupe les back-edges (une arête dont
la cible domine la source) de la marche.

**Correction** (`merge_unique_minimal`) : sous l'invariant structurel du builder (tout chemin forward d'une branche vers
un candidat passe par le merge), le merge est le seul candidat qui intercepte tous les candidats — toute recherche
premier-candidat en marche forward le retourne, indépendamment de l'ordre de parcours. Preuve par descente infinie sur
les longueurs de chemins, le squelette de l'antisymétrie de dominance.

**Le bug en concret** : sur le CFG du contre-exemple minimisé, `naive_walk_reaches_inner_header` (la marche naïve
atteint le header interne depuis le else) contre `forward_walk_stops_at_merge` (forward-only, le else n'atteint que
lui-même et le merge).

### CatnipDestructionBridge.v

`CatnipParallelCopyProof` prouve le solveur sur un état abstrait (`loc = nat`) ; la section destruction de
`CatnipCFGSSACorrectness` prouve les faits par-copie sur `Env = SSAVal → nat`. Les deux modèles n'étaient pas reliés :
l'énoncé compositionnel — le lot séquentialisé d'une arête réalise **tous** les phis du join simultanément — ne vivait
dans aucun des deux. Constat clé du raccord : `apply_copies` de `CatnipCFGSSABase` est exactement l'exécution
séquentielle qu'émet le solveur.

**Le pont** (`cycle_batch_realizes_phi`) : après exécution du cycle cassé par scratch, chaque cible de phi tient
`eval_phi` de l'environnement *initial* — la sémantique parallèle du join, réalisée par la liste séquentielle.
`cycle_batch_preserves_others` donne le frame (rien d'autre ne bouge) ; `swap_join_demo` instancie le join à deux phis
en swap, le cas lost-copy canonique.

## Technique : récursion à carburant

Coq exige que toute récursion termine. Un RDP ne termine pas structurellement sur sa liste de tokens - la tail function
consomme des tokens mais Coq ne le voit pas. Les parseurs dans `CatnipAddMulProof.v` et `CatnipExprProof.v` utilisent le
pattern **fuel-based recursion** :

```coq
Fixpoint parse_expr (fuel : nat) (ts : list token) : option (expr * list token)
```

Le paramètre `fuel` décroît strictement à chaque appel récursif (`destruct fuel as [|fuel']`, appel sur `fuel'`). Quand
le fuel est épuisé, le parseur retourne `None`. Coq accepte la définition parce que `fuel` est un `nat` qui décroît
structurellement.

En pratique, les théorèmes par réflexivité (`Proof. vm_compute; reflexivity. Qed.`) calculent le résultat exact du
parseur sur une entrée concrète - Coq exécute le parseur et vérifie que le résultat correspond.

**Monotonie** : le théorème `fuel_mono` (prouvé dans les deux fichiers de parsing) établit que si le parseur réussit
avec un fuel `f`, il réussit avec le même résultat pour tout `f' >= f`. Cela élimine la dépendance aux constantes de
fuel spécifiques (32, 64) : les résultats quantifiés universellement (`forall fuel, fuel >= k -> ...`) sont dérivés en
fournissant un témoin concret puis en appliquant la monotonie via `lia`. La technique utilise des lemmes de dépliage
(`parse_*_unfold : parse_*(S f) ts = ...`) prouvés par `reflexivity`, qui exposent la structure récursive pour que les
hypothèses d'induction soient applicables.

Le parseur réel (tree-sitter) ne fonctionne pas par fuel mais par la structure de la grammaire PEG. Le fuel est un
artefact de la preuve, pas du runtime.

## Build

```bash
make proof          # Compile les fichiers .v, vérifie toutes les preuves
make proof-clean    # Supprime les artefacts Coq (.vo, .glob, Makefile.coq)
make proof-scan     # Vérifie l'absence d'Admitted, Abort, axiomes, imports classiques
```

Prérequis : Coq installé (`coqc`, `coq_makefile`). Le fichier `proof/_CoqProject` liste les sources.

## Références

- [The Coq Proof Assistant](https://coq.inria.fr/) - assistant de preuve utilisé
- [Braun et al. 2013](https://pp.ipd.kit.edu/uploads/publikationen/braun13cc.pdf) - Simple and Efficient Construction of
  Static Single Assignment Form (utilisé dans le pipeline SSA de Catnip, [ARCHITECTURE](ARCHITECTURE.md))
- [`grammar.js`](../../catnip_grammar/grammar.js) - grammaire tree-sitter source de vérité, citée en tête de chaque
  fichier `.v`
