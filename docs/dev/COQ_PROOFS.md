# Preuves Coq

Vérification mécanisée de fragments de Catnip : grammaire, précédence, modèle dimensionnel, IR, scopes, pattern
matching, fonctions/TCO, et passes d'optimisation.

## TL;DR

50 fichiers Coq dans `proof/` (~17400 lignes, 0 Admitted) prouvent des invariants structurels et sémantiques couvrant le
parsing, le broadcasting, la résolution de scopes, le pattern matching, le trampoline TCO, les 10/10 passes
d'optimisation IR, l'analyse de liveness, la ND-récursion, le pipeline CFG/SSA (Braun et al. 2013), la dominance, le
NaN-boxing VM, la sécurité de pile VM, les frames/IP/jumps, la linéarisation C3 (MRO), les structs/traits, le desugaring
opérateurs (sémantique, pureté, broadcast), et le cache. Si `make proof` passe, les théorèmes sont validés
mécaniquement. Ces preuves portent sur des modèles formels alignés avec le code Rust, pas sur l'exécution du runtime en
production. L'alignement est maintenu explicitement dans les commentaires en tête de chaque fichier `.v`. Tree-sitter et
Cranelift ne sont pas formellement prouvés dans ce repo.

> Un parseur sans preuve est un parseur qui ne sait pas encore qu'il a tort.

## Pourquoi des preuves formelles

Catnip utilise tree-sitter pour parser, et tree-sitter fait son travail correctement. Mais la grammaire déclarée dans
`grammar.js` encode des invariants implicites : la précédence de `*` sur `+`, l'associativité gauche de `-`, le fait que
`not` lie plus fort que `and`. Ces propriétés ne sont vérifiées par aucun test unitaire classique -- un test vérifie
qu'un cas marche, pas que tous les cas marchent.

Les fichiers dans `proof/` couvrent six axes :

- **Syntaxe** -- invariants de parsing de la grammaire (`grammar.js`) via un parseur à descente récursive formalisé.
- **Sémantique** -- propriétés structurelles du modèle dimensionnel (broadcast, ND-récursion).
- **Runtime** -- IR opcodes, scopes (shadowing, isolation), pattern matching (6 types, déterminisme), fonctions
  (binding, trampoline TCO, tail detection), NaN-boxing VM (7 tags), VM opcodes et stack safety, frames/IP/jumps, C3
  linearization (MRO), structs/traits (field access, method resolution, inheritance), desugaring opérateurs
  (injectivité, totalité, cohérence IR).
- **Optimisations** -- 10/10 passes IR prouvées : strength reduction, blunt code, DCE, block flattening, constant
  folding, constant/copy propagation, CSE, DSE, tail recursion to loop.
- **Analyses** -- liveness analysis (linéaire + CFG), dead store elimination, fixpoint, dominance CFG (idom,
  frontières).
- **Infrastructure** -- CFG/SSA (single assignment, phi-nodes, GVN, LICM, CSE inter-blocs, DSE globale), cache (FIFO,
  LRU+TTL, memoization, atomic writes).

Coq vérifie chaque étape de raisonnement : si `make proof` passe, les propriétés sont vraies.

Ce ne sont pas des preuves du runtime lui-même. L'alignement entre les modèles Coq et le code Rust est maintenu
manuellement -- les commentaires en tête de chaque fichier `.v` citent les définitions correspondantes.

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

| Fichier      | Couverture                                         | Théorèmes clés                                                               |
| ------------ | -------------------------------------------------- | ---------------------------------------------------------------------------- |
| `CatnipIR.v` | IROpCode, IRPure, bijection, classification, arity | `opcode_to_nat_injective`, `opcode_roundtrip`, `control_flow_not_arithmetic` |

### D. Preuves runtime

Prouvent les invariants des composants d'exécution : scopes, pattern matching, fonctions.

| Fichier                 | Couverture                                                                         | Théorèmes clés                                                                                                                    |
| ----------------------- | ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `CatnipScopeProof.v`    | Lookup/set O(1), shadowing, push/pop, frames isolées                               | `scope_set_lookup_same`, `scope_push_pop`, `scope_shadowing`, `scope_pop_restores`                                                |
| `CatnipPatternProof.v`  | 6 types de patterns, guards, dispatch, déterminisme                                | `wildcard_always_matches`, `or_first_match_wins`, `match_pattern_deterministic`                                                   |
| `CatnipFunctionProof.v` | Binding params (positional, defaults), trampoline TCO, scope depth, tail detection | `bind_params_exact_length`, `trampoline_normal_terminates`, `trampoline_preserves_scope_depth`, `tail_position_produces_tailcall` |

### E. Preuves d'optimisation

Prouvent la correction des 10/10 passes IR du pipeline.

| Fichier                    | Couverture                                                                     | Théorèmes clés                                                                     |
| -------------------------- | ------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------- |
| `CatnipStrengthRedProof.v` | Strength reduction (20 identités algébriques + correction sémantique)          | `sr_mul_one_r`, `sr_pow_two`, `strength_reduce_bool_sound`                         |
| `CatnipBluntCodeProof.v`   | Blunt code (boolean algebra, inversion cmp, idempotence, complément)           | `blunt_double_neg`, `blunt_and_complement`, `simplify_blunt_bool_sound`            |
| `CatnipDCEFlattenProof.v`  | DCE, block flattening, composition de passes, lowering IR                      | `flatten_stmts_idempotent`, `flatten_block_sound`, `compose_preserves_eval`        |
| `CatnipOptimProof.v`       | Façade (`Require Export` des 3 fichiers ci-dessus)                             | -                                                                                  |
| `CatnipConstFoldProof.v`   | Constant folding (arith+cmp+bool+bitwise), guards div/0 et b\<0                | `cf_add_fold`, `cf_truediv_fold`, `cf_pow_fold`, `cf_band_fold`, `cf_add_fold_sem` |
| `CatnipStorePropProof.v`   | Store model, constant propagation, copy propagation, CSE (structural equality) | `const_prop_correct`, `copy_chain_terminates`, `cse_replace_correct`               |
| `CatnipTailRecLoopProof.v` | Tail recursion → loop, fuel monotonie, two-phase rebinding                     | `tail_rec_loop_equiv`, `rebind_two_phase`, `fuel_monotone`                         |
| `CatnipPurityProof.v`      | Pureté sous surcharge opérateurs : struct ops hors pure_ops, non CSE-eligible  | `overloaded_op_never_eligible`, `desugared_builtins_are_pure`, `call_not_pure`     |

### F. Preuves d'analyse et CFG

Prouvent la correction de l'analyse de liveness, de la dominance, et du pipeline CFG/SSA.

| Fichier                     | Couverture                                                                                                         | Théorèmes clés                                                                                                                     |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `CatnipVarSet.v`            | Bibliothèque VarSet réutilisable : add, union, remove, NoDup, subset                                               | `add_preserves_nodup`, `union_preserves_nodup`, `remove_list_subset`                                                               |
| `CatnipLivenessLinear.v`    | Liveness linéaire : USE/DEF, transfer, backward analysis, DSE linéaire                                             | `live_in_sound`, `dse_linear_correct`, `transfer_monotone`                                                                         |
| `CatnipLivenessCFG.v`       | Liveness CFG : LiveMap, fixpoint itératif, DSE CFG, path soundness                                                 | `dse_cfg_sound_head`, `exec_path_sound`, `iterate_cfg_stable`                                                                      |
| `CatnipLivenessProof.v`     | Façade (`Require Export` des 3 fichiers ci-dessus)                                                                 | -                                                                                                                                  |
| `CatnipDominanceProof.v`    | Dominance CFG : réflexivité, transitivité, antisymétrie, idom unicité, frontières                                  | `dom_refl`, `entry_dom_all`, `dom_trans`, `dom_antisym`, `idom_unique`, `entry_frontier_empty`                                     |
| `CatnipCFGSSABase.v`        | SSA base : modèle SSA, utilitaires, modèles opérationnels (construction SSA, use-count, DSE)                       | `ssaval_eqb_eq`, `unique_def_from_def_block`, `no_dup_phi_from_lookup`, `seal_block_sealed`, `dse_iterate_mono`                    |
| `CatnipCFGSSACorrectness.v` | SSA correctness (49 lemmes/théorèmes, 0 Admitted) : single assignment, phi-nodes, CSE, GVN, LICM, DSE, destruction | `single_assignment`, `def_before_use`, `phi_at_frontier`, `cse_same_key_same_value`, `licm_hoist_sound`, `dse_fixpoint_terminates` |
| `CatnipCFGSSAProof.v`       | Façade de compatibilité (`Require Export` de `CatnipCFGSSABase` + `CatnipCFGSSACorrectness`)                       | -                                                                                                                                  |

### G. Preuves runtime avancées

Prouvent les invariants des composants runtime avancés.

| Fichier                      | Couverture                                                                                              | Théorèmes clés                                                                                                                                                                           |
| ---------------------------- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CatnipNanBoxProof.v`        | NaN-boxing VM : 7 tags (SmallInt/Bool/Nil/Symbol/PyObj/Struct/BigInt), encoding 48-bit                  | `tag_injective`, `encode_decode_roundtrip`, `smallint_range`, `promote_demote_id`                                                                                                        |
| `CatnipVMBase.v`             | VM opcodes (74), stack effects, stack safety, instruction sequences, compiler invariants                | `vm_opcode_roundtrip`, `stack_safety_fixed`, `exec_seq_app`, `arg_dependent_opcodes`                                                                                                     |
| `CatnipVMFrame.v`            | VM frames (locals, IP, jumps, block stack, ForRange encoding roundtrips)                                | `get_set_same`, `ip_advance_in_bounds`, `jump_preserves_bounds`, `push_pop_restores`, `for_range_full_roundtrip`                                                                         |
| `CatnipVMProof.v`            | Façade de compatibilité (`Require Export` de `CatnipVMBase` + `CatnipVMFrame`)                          | -                                                                                                                                                                                        |
| `CatnipMROC3Core.v`          | C3 merge algorithm, self-first property                                                                 | `c3_self_first`, `c3_self_is_head`, `c3_no_parents`                                                                                                                                      |
| `CatnipMROC3Properties.v`    | C3 local precedence and monotonicity                                                                    | `c3_preserves_local_precedence`, `c3_monotonicity`, `c3_merge_preserves_order`                                                                                                           |
| `CatnipMROFields.v`          | MRO field merge, diamond dedup, redefinition detection                                                  | `dedup_at_most_once`, `no_redefinition_correct`, `redefinition_detected`                                                                                                                 |
| `CatnipMROMethods.v`         | MRO method resolution, left-priority                                                                    | `left_priority`, `merge_methods_subset`                                                                                                                                                  |
| `CatnipMROSuper.v`           | Super resolution, cooperative termination                                                               | `super_at_self`, `super_at_end`, `super_max_steps`, `super_from_last_is_empty`                                                                                                           |
| `CatnipMROExamples.v`        | Exemples concrets (diamond, linear, inconsistent, init chain)                                           | `diamond_c3`, `inconsistent_c3`, `diamond_method_resolution`, `super_from_B_in_diamond`                                                                                                  |
| `CatnipMROProof.v`           | Facade (`Require Export` des 6 modules MRO ci-dessus)                                                   | -                                                                                                                                                                                        |
| `CatnipOpDesugar.v`          | Desugaring opérateurs : symbol x arity -> method name, injectivité, totalité                            | `desugar_injective`, `desugar_total`                                                                                                                                                     |
| `CatnipOpDesugarProps.v`     | Disambiguation +/-, distinctness, résolvabilité méthode, cohérence opcode, préfixe op\_                 | `arity_disambiguation_minus`, `desugar_names_distinct`, `desugar_method_resolvable`, `desugar_opcode_consistent`                                                                         |
| `CatnipOpDesugarExamples.v`  | Exemples concrets (Vec2, disambiguation unaire/binaire, cas négatifs)                                   | `vec2_find_add`, `minus_as_binary`, `minus_as_unary`, `eq_not_unary`                                                                                                                     |
| `CatnipOpDesugarSemantics.v` | Préservation sémantique : dispatch VM = appel méthode, reverse dispatch, roundtrip opcode, déterminisme | `operator_dispatch_is_method_call`, `dispatch_finds_same_method`, `reverse_dispatch_finds_method`, `forward_priority_over_reverse`, `rev_dispatch_subsumes_old`, `no_dispatch_ambiguity` |
| `CatnipStructProof.v`        | Structs/traits : field access O(1), method resolution, inheritance, super chain                         | `field_access_sound`, `method_resolution_order`, `super_chain_terminates`, `trait_linearization`                                                                                         |
| `CatnipCacheKey.v`           | Cache keys : CacheType, encoding Z, injectivité, disjointness                                           | `cache_key_injective`, `cache_type_disjoint`, `cache_key_bijection`                                                                                                                      |
| `CatnipCacheMemory.v`        | MemoryCache FIFO : find/remove/set, key uniqueness, round-trip, eviction                                | `mc_set_get_same`, `mc_fifo_evicts_oldest`, `mc_set_size_bounded`                                                                                                                        |
| `CatnipCacheDisk.v`          | DiskCache LRU+TTL : expiration, prune, eviction, atomic writes                                          | `dc_get_ttl_enforcement`, `dc_lru_evict_size`, `atomic_write_no_partial`                                                                                                                 |
| `CatnipCacheAdapter.v`       | CatnipCache adapter, Memoization, invalidation (16 keys)                                                | `memo_set_get_same`, `invalidation_covers_all`, `all_invalidation_keys_nodup`                                                                                                            |
| `CatnipCacheProof.v`         | Façade (`Require Export` des 4 fichiers ci-dessus)                                                      | -                                                                                                                                                                                        |

## A. Preuves syntaxiques

### GrammarProof.v

Modèle minimal d'une CFG (S -> A B, A -> "a", B -> "b") avec trois résultats :

**Soundness des arbres** : tout arbre de dérivation produit une séquence de terminaux dérivable depuis le non-terminal
racine (`tree_sound`). La preuve construit la chaîne de réécritures explicitement.

**Génération** : la grammaire engendre bien `[a; b]` (`generates_example_ab`), par application successive des trois
productions.

**Non-ambiguïté structurelle** : pour chaque non-terminal, il n'existe qu'une seule forme d'arbre
(`grammar_unambiguous_S`). La preuve procède par destruction dépendante -- Coq élimine structurellement toute
alternative.

**Non-ambiguïté via yield** : formulation standard en théorie des langages -- si deux arbres pour le même non-terminal
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
- **Parenthèses** (`paren_example`) : `(n + n) * n` parse comme `(n + n) * n` -- les parenthèses forcent la structure

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

**Opérateurs de comparaison** : les 6 opérateurs sont testés individuellement (`comparison_ops_examples`) -- chacun
produit le bon noeud AST.

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
une collection est nécessairement `Coll (map op xs)` -- c'est-à-dire `broadcast_map op` (`universality`). Il n'existe
pas d'autre façon de lifter une opération aux collections avec ces propriétés (`broadcast_unique`). Pour les collections
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
- *Homomorphisme de liste* (`broadcast_concat`) : le broadcast distribue sur la concaténation -- fondation algébrique de
  la parallélisation.
- *Sémantique un-niveau* (`broadcast_shallow`, `broadcast_two_levels`) : le broadcast opère à exactement un niveau de
  profondeur ; le broadcast récursif nécessite un emboîtement explicite.
- *Homomorphisme de monoide* (`fold_broadcast_exchange`) : quand `f` distribue sur `op` et fixe `z`, `f` commute avec le
  fold. Corollaire : fold sur un broadcast = fold avec accumulateur composé (`fold_broadcast_map`).

**Algèbre de pipelines** : formalise les pipelines comme séquences d'opérations `PMap` / `PFilt` et prouve trois
résultats :

- *Fusion complète* (`map_chain_fusion`) : toute chaîne de n broadcasts maps se réduit à un seul broadcast avec la
  fonction composée. Sûre (préserve la sémantique) et complète (n quelconque).
- *Equivalence transformationnelle* : trois règles de réécriture prouvées correctes -- map;map fusionne
  (`equiv_map_map`), map;filter se réordonne en filter;map (`equiv_filter_map_swap`), filter;filter s'absorbe
  (`equiv_filter_filter`).
- *Normalisation* (`pipeline_normalization`) : tout pipeline mixte de maps et filters sur une collection se réduit à une
  forme canonique `filter;map`. Deux pipelines qui ont la même forme normale sont équivalents (`normalization_sound`).

### CatnipNDRecursion.v

Modèle parametrique de la ND-récursion (`~~(seed, lambda)`), split de `CatnipDimensional.v`. Indépendant du domaine de
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

**IRPure** : inductif représentant les noeuds IR (IRInt, IRFloat, IRBool, IRStr, IRNone, IRDecimal, IRImaginary, IROp),
avec `ir_size` (taille structurelle), `ir_op` (constructeur par opcode + args), `ir_binop` (spécialisation binaire).

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

**Déterminisme** : `match_pattern_deterministic` et `match_cases_deterministic` -- le résultat du match est unique.

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

**Scope** : `trampoline_preserves_scope_depth` -- le trampoline ne modifie pas la profondeur du scope.

**Tail detection** : `tail_position_produces_tailcall` (si le TCO est actif et la position est tail, un appel récursif
produit un TailCall), `non_tail_produces_normal` (en position non-tail, l'appel est normal),
`tco_disabled_produces_normal` (si TCO désactivé, pas de TailCall).

## E. Preuves d'optimisation

### CatnipOptimProof.v (facade)

Façade qui re-exporte les 3 modules suivants. Source de vérité : `catnip_rs/src/semantic/strength_reduction.rs`,
`blunt_code.rs`, `dead_code_elimination.rs`, `block_flattening.rs`.

Le modèle d'expressions `Expr` (Const, BConst, Var, BinOp, UnOp, IfExpr, WhileExpr, Block, MatchExpr) avec évaluateur
partiel `eval_expr` est défini dans `CatnipExprModel.v`.

### CatnipStrengthRedProof.v

**Strength reduction** (`strength_reduce : Expr -> Expr`) : 20 théorèmes individuels couvrant les identités
multiplicatives (`x * 1 = x`, `x * 0 = 0`), exponentielles (`x^2 -> x*x`, `x^1 = x`, `x^0 = 1`), additives (`x + 0 = x`,
`x - 0 = x`), division (`x / 1 = x`), et booléennes (`x && True = x`, `x || False = x`, `x && False = False`,
`x || True = True`). Correction sémantique prouvée pour les cas arithmétiques. Théorème principal :
`strength_reduce_bool_sound` (la passe préserve `eval_bool`).

### CatnipBluntCodeProof.v

**Blunt code** (`simplify_blunt : Expr -> Expr`) : double négation (`not not x = x`), inversion de comparaisons
(`not (a < b) = a >= b`) avec preuve d'involution de `invert_cmp`, simplification booléenne (`x == True = x`),
idempotence (`x && x = x`, `x || x = x`), complément (`x && not x = False`, `x || not x = True`). Les preuves de
complément utilisent des lemmes de taille structurelle (`expr_eqb_not_self` : x n'est jamais structurellement égal à
`UnOp Not x`). Inclut `expr_eqb_eq` (réflexion de l'égalité structurelle). Théorème principal :
`simplify_blunt_bool_sound`.

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

> Les 10 passes du pipeline IR sont toutes prouvées. Les 6 passes store-based (constant folding, constant/copy
> propagation, CSE, DSE, tail rec to loop) ont été ajoutées dans `CatnipConstFoldProof.v`, `CatnipStorePropProof.v`,
> `CatnipLivenessProof.v` et `CatnipTailRecLoopProof.v`.

### CatnipVMBase.v

Modélise les opcodes VM de `catnip_rs/src/vm/opcode.rs` et prouve la sécurité de pile.

**Opcode numbering** : bijection `VMOpCode <-> nat` avec `vm_opcode_to_nat_injective` et `vm_opcode_roundtrip`. Même
technique que `CatnipIR.v` pour les IR opcodes.

**Stack effects** : chaque opcode a un effet de pile `(pops, pushes)`. La majorité ont un effet fixe (connu
statiquement), le reste est arg-dépendant (`Call`, `BuildList`, `BuildDict`, etc.). La classification est exhaustive
(`effect_total`, `arg_dependent_opcodes`).

**Stack safety** : théorème central `stack_safety_fixed` -- pour tout opcode à effet fixe, si la pile a au moins `pops`
éléments, l'exécution produit une pile de profondeur `depth - pops + pushes`, sans underflow. Pour les opcodes
arg-dépendants, safety prouvée paramétriquement (`call_stack_safety`, `build_seq_stack_safety`,
`build_dict_stack_safety`, `unpack_seq_stack_safety`).

**Instruction sequences** : `exec_seq` exécute une liste d'instructions, `exec_seq_app` prouve la composition.
Invariants du compilateur prouvés par exemples : expression = net +1 (`binop_pattern_depth`), assignment = net 0
(`assignment_pattern_depth`), discard = net 0 (`discard_pattern_depth`).

**Propriétés par catégorie** : net effects uniformes pour les classes d'opcodes -- arithmétique = -1, comparaison = -1,
unaire = 0, load = +1, store = -1, noop = 0.

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

### CatnipVMProof.v

Façade de compatibilité (`Require Export` de `CatnipVMBase` + `CatnipVMFrame`).

### CatnipMRO\*.v (6 modules)

Modélisent la linéarisation C3 et la résolution MRO de `catnip_rs/src/vm/mro.rs`. Standalone, sans dépendances sur les
autres preuves Catnip. `CatnipMROProof.v` est une facade (`Require Export` des 6 modules).

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

Modélisent le desugaring des opérateurs surchargés (`op <symbol>`) de `catnip_rs/src/parser/pure_transforms.rs`. Le
mapping `(symbol, arity) → method_name` est prouvé injectif et total.

**CatnipOpDesugar.v** : Modèle du mapping (19 symboles, 21 combinaisons valides sur 38). `desugar_injective` (deux
couples distincts ne produisent jamais le même nom), `desugar_total` (toute combinaison valide produit un nom).

**CatnipOpDesugarProps.v** : Propriétés dérivées. `arity_disambiguation_minus/plus` (`-` et `+` unaire vs binaire
donnent des noms différents), `desugar_names_distinct` (corollaire d'injectivité), `invalid_combinations_fail` (17 cas
invalides), `desugar_method_resolvable` (connexion avec `find_method` de CatnipStructProof), `desugar_opcode_consistent`
(cohérence avec les IROpCode), `op_prefix_preserved` (tous les noms commencent par `op_`).

**CatnipOpDesugarExamples.v** : Exemples concrets. Vec2 (struct avec méthodes opérateur), disambiguation `-` unaire vs
binaire sur un même struct, cas négatifs (`SymEq Unary = None`, etc.).

**CatnipOpDesugarSemantics.v** : Préservation sémantique du dispatch VM. Modélise le dispatch two-phase de `vm/core.rs`
(native op → struct method) et le dispatch reverse (prim OP struct → méthode du struct).
`operator_dispatch_is_method_call` (pour les structs, le dispatch VM = appel méthode direct),
`reverse_dispatch_finds_method` (prim OP struct dispatche vers la méthode du struct), `forward_priority_over_reverse`
(le dispatch forward a priorité), `rev_dispatch_subsumes_old` (rétro-compatibilité : l'ancien dispatch est un cas
particulier du nouveau), `opcode_roundtrip` (symbol → method name → opcode → même opcode), `no_dispatch_ambiguity` (un
symbole résout vers exactement une méthode).

## F. Preuves d'analyse

### CatnipLivenessProof.v (facade)

Façade qui re-exporte les 3 modules suivants. Modélise l'analyse de liveness et la dead store elimination (DSE), d'abord
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

## Technique : récursion à carburant

Coq exige que toute récursion termine. Un RDP ne termine pas structurellement sur sa liste de tokens -- la tail function
consomme des tokens mais Coq ne le voit pas. Les parseurs dans `CatnipAddMulProof.v` et `CatnipExprProof.v` utilisent le
pattern **fuel-based recursion** :

```coq
Fixpoint parse_expr (fuel : nat) (ts : list token) : option (expr * list token)
```

Le paramètre `fuel` décroît strictement à chaque appel récursif (`destruct fuel as [|fuel']`, appel sur `fuel'`). Quand
le fuel est épuisé, le parseur retourne `None`. Coq accepte la définition parce que `fuel` est un `nat` qui décroît
structurellement.

En pratique, les théorèmes par réflexivité (`Proof. vm_compute; reflexivity. Qed.`) calculent le résultat exact du
parseur sur une entrée concrète -- Coq exécute le parseur et vérifie que le résultat correspond.

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

- [The Coq Proof Assistant](https://coq.inria.fr/) -- assistant de preuve utilisé
- [Braun et al. 2013](https://pp.ipd.kit.edu/uploads/publikationen/braun13cc.pdf) -- Simple and Efficient Construction
  of Static Single Assignment Form (utilisé dans le pipeline SSA de Catnip, [ARCHITECTURE](ARCHITECTURE.md))
- [`grammar.js`](../../catnip_grammar/grammar.js) -- grammaire tree-sitter source de vérité, citée en tête de chaque
  fichier `.v`
