# Exemples

> No gods, no masters, juste du code: si ça compile c'est un exemple, sinon c'est un bug avec de l'attitude.

## Bases (`basics/`)

Exemples fondamentaux :

- [`01_born_to_segfault.cat`](basics/01_born_to_segfault.cat) - Premier programme, debug et affichage
- [`02_loops.cat`](basics/02_loops.cat) - Boucles for et while
- [`03_statement_separators.cat`](basics/03_statement_separators.cat) - Newlines, semicolons et séparateurs mixtes
- [`04_blocks_and_expressions.cat`](basics/04_blocks_and_expressions.cat) - Blocs comme expressions, court-circuit
  logique
- [`05_fstrings.cat`](basics/05_fstrings.cat) - F-strings: interpolation, format specs, conversion et debug
- [`06_assignment_syntax.cat`](basics/06_assignment_syntax.cat) - Affectation simple, chaînée, unpacking, attributs,
  index
- [`07_literals_and_variadic.cat`](basics/07_literals_and_variadic.cat) - Littéraux list/dict, fonctions variadiques
- [`08_data_structures.cat`](basics/08_data_structures.cat) - Range, accumulation, filtrage, statistiques
- [`09_factorielle.cat`](basics/09_factorielle.cat) - Calcul de factorielle avec match
- [`10_fibonacci.cat`](basics/10_fibonacci.cat) - Nombres de Fibonacci
- [`11_tri_bulles.cat`](basics/11_tri_bulles.cat) - Tri à bulles
- [`12_calculatrice.cat`](basics/12_calculatrice.cat) - Calculatrice simple avec match
- [`13_fizzbuzz.cat`](basics/13_fizzbuzz.cat) - FizzBuzz
- [`14_decimals.cat`](basics/14_decimals.cat) - Type Decimal exact (suffixe d/D, arithmétique base-10)
- [`15_complex.cat`](basics/15_complex.cat) - Nombres complexes (suffixe j/J)
- [`16_membership.cat`](basics/16_membership.cat) - Opérateurs `in` / `not in`
- [`17_identity.cat`](basics/17_identity.cat) - Opérateurs `is` / `is not`
- [`18_null_coalesce.cat`](basics/18_null_coalesce.cat) - Opérateur nil-coalescing (`??`)
- [`19_type_introspection.cat`](basics/19_type_introspection.cat) - Introspection de type avec `typeof()`

## Fonctions (`functions/`)

Fonctions, lambdas et closures :

- [`01_functions.cat`](functions/01_functions.cat) - Définition de fonctions, paramètres, defaults
- [`02_lambdas.cat`](functions/02_lambdas.cat) - Fonctions anonymes, lambdas inline
- [`03_closures_and_higher_order.cat`](functions/03_closures_and_higher_order.cat) - Closures, fonctions de fonctions,
  composition
- [`04_tail_recursion.cat`](functions/04_tail_recursion.cat) - Récursion terminale et transformation automatique en
  boucles
- [`05_fold_and_reduce.cat`](functions/05_fold_and_reduce.cat) - Fold et reduce : agrégation avec valeur initiale

## Pattern matching (`pattern-matching/`)

Matching et destructuration :

- [`01_pattern_matching.cat`](pattern-matching/01_pattern_matching.cat) - match/case, patterns littéraux, variables,
  guards, patterns OR

## Flux de Contrôle (`control-flow/`)

Structures de contrôle avancées :

- [`01_break_example.cat`](control-flow/01_break_example.cat) - Sortie anticipée de boucle
- [`02_continue_example.cat`](control-flow/02_continue_example.cat) - Passage à l'itération suivante
- [`03_break_continue_combined.cat`](control-flow/03_break_continue_combined.cat) - Combinaison des deux

## Avancé (`advanced/`)

Fonctionnalités avancées :

- [`01_pragma_features.cat`](advanced/01_pragma_features.cat) - Pragmas et directives
- [`02_structs.cat`](advanced/02_structs.cat) - Structures nommées (déclaration, instanciation, mutation, imbrication)
- [`03_traits.cat`](advanced/03_traits.cat) - Traits, composition et résolution de conflits
- [`04_operator_overloading.cat`](advanced/04_operator_overloading.cat) - Surcharge d'opérateurs
- [`05_nd_recursion.cat`](advanced/05_nd_recursion.cat) - Récursion N-dimensionnelle
- [`06_binary_search.cat`](advanced/06_binary_search.cat) - Algorithme de recherche binaire
- [`07_knapsack.cat`](advanced/07_knapsack.cat) - Problème du sac à dos (brute force + ND-mémoïsé)
- [`08_advanced_techniques.cat`](advanced/08_advanced_techniques.cat) - Techniques avancées combinées
- [`09_turing_completeness.cat`](advanced/09_turing_completeness.cat) - Démonstration de Turing-complétude
- [`10_enums.cat`](advanced/10_enums.cat) - Types énumérés, pattern matching et machine à états

## Broadcasting (`broadcast/`)

Opérations vectorielles sur collections :

- [`01_scalar_broadcasting.cat`](broadcast/01_scalar_broadcasting.cat) - Broadcasting sur scalaire
- [`02_basic_broadcasting.cat`](broadcast/02_basic_broadcasting.cat) - Syntaxe `A.[op M]`, bases
- [`03_math_operations.cat`](broadcast/03_math_operations.cat) - Opérations mathématiques vectorielles
- [`04_comparisons.cat`](broadcast/04_comparisons.cat) - Comparaisons et filtrage
- [`05_boolean_logic.cat`](broadcast/05_boolean_logic.cat) - Logique booléenne distribuée
- [`06_nested_structures.cat`](broadcast/06_nested_structures.cat) - Broadcasting sur structures imbriquées
- [`07_cas_illustratifs.cat`](broadcast/07_cas_illustratifs.cat) - Cas d'usage illustratifs
- [`bench_broadcast.py`](broadcast/bench_broadcast.py) - Mesures de performance

## Chargement de Modules (`module-loading/`)

Intégration avec Python :

- [`index.md`](module-loading/index.md) - Mécanismes de base
- [`01_demo.cat`](module-loading/01_demo.cat) - Mode namespace par défaut
- [`02_demo_with_as.cat`](module-loading/02_demo_with_as.cat) - Namespace personnalisé
- [`host_module_example.py`](module-loading/host_module_example.py) - Module Python exemple
- [`03_app_script.cat`](module-loading/03_app_script.cat) - Script utilisant l'API
- [`05_simple_extension.py`](module-loading/05_simple_extension.py) - Extension simple
- [`06_custom_functions.py`](module-loading/06_custom_functions.py) - Fonctions personnalisées
- [`07_app_api.py`](module-loading/07_app_api.py) - API d'application simulée
- [`08_aesgcm.cat`](module-loading/08_aesgcm.cat) - AES-GCM avec propagation d'erreur Python
- [`09_relative_import.cat`](module-loading/09_relative_import.cat) - Import relatif via `caller_dir`
- [`10_wild_import.cat`](module-loading/10_wild_import.cat) - Import global (injection dans globals)
- [`11_selective_import.cat`](module-loading/11_selective_import.cat) - Import sélectif de noms spécifiques
- [`12_io_module.cat`](module-loading/12_io_module.cat) - Module IO builtin
- [`13_relative_import_dots.cat`](module-loading/13_relative_import_dots.cat) - Import relatif avec dots (style Python)

## Intégration DSL (`embedding/`)

Catnip comme moteur DSL embarqué dans une application Python :

- [`01_dataframe_dsl.py`](embedding/01_dataframe_dsl.py) - DSL pour DataFrames pandas
- [`02_config_dsl.py`](embedding/02_config_dsl.py) - Validation de configuration
- [`03_etl_pipeline.py`](embedding/03_etl_pipeline.py) - Pipeline ETL déclaratif
- [`04_rule_engine.py`](embedding/04_rule_engine.py) - Moteur de règles métier
- [`05_report_builder.py`](embedding/05_report_builder.py) - Génération de rapports
- [`06_workflow_dsl.py`](embedding/06_workflow_dsl.py) - Orchestration de workflows
- [`07_flask_sandbox.py`](embedding/07_flask_sandbox.py) - Sandbox Flask sécurisé
- [`08_jupyter_integration.py`](embedding/08_jupyter_integration.py) - Magic commands Jupyter
- [`09_streamlit_app.py`](embedding/09_streamlit_app.py) - Dashboard Streamlit interactif
- [`10_pickle_serialization.py`](embedding/10_pickle_serialization.py) - Sérialisation pickle (AST, Scope, lambdas,
  closures)

## Standalone (`run/`)

Scripts Catnip exécutables en ligne de commande :

- [`01_calculate.cat`](run/01_calculate.cat) - Calculatrice et formules
- [`02_filter_data.cat`](run/02_filter_data.cat) - Filtrage et transformations
- [`03_transform_csv.cat`](run/03_transform_csv.cat) - Transformation de données tabulaires
- [`04_config_validator.cat`](run/04_config_validator.cat) - Validation de configuration
- [`05_data_report.cat`](run/05_data_report.cat) - Rapport de données

## Graphe de contrôle (`cfg/`)

Analyse et optimisation du flux de contrôle :

- [`01_cfg_basic.md`](cfg/01_cfg_basic.md) - Construction de CFG, blocs basiques, arêtes
- [`02_cfg_analysis.md`](cfg/02_cfg_analysis.md) - Dominance, détection de boucles
- [`03_cfg_optimization.md`](cfg/03_cfg_optimization.md) - Optimisations (dead code, fusion, branches)
- [`04_cfg_reconstruction.md`](cfg/04_cfg_reconstruction.md) - Reconstruction de code structuré depuis CFG

## Outils (`tools/`)

Utilitaires de développement :

- [`extract_grammar_demo.py`](tools/extract_grammar_demo.py) - Extraction de la grammaire Tree-sitter
- [`json_ir_analysis.py`](tools/json_ir_analysis.py) - Analyse programmatique de l'IR via sérialisation JSON

## Performance (`performance/`)

Benchmarks et profiling :

- [`vm_optimizations_benchmark.py`](performance/vm_optimizations_benchmark.py) - Hot paths VM vs Python natif (range,
  tail recursion, BigInt)
- [`pipeline_comparison_benchmark.py`](performance/pipeline_comparison_benchmark.py) - Comparaison des niveaux
  d'optimisation (0-3)
- [`tco_vs_iteration_benchmark.py`](performance/tco_vs_iteration_benchmark.py) - Boucle impérative vs récursion
  terminale (TCO on/off)
- [`profiling_example.py`](performance/profiling_example.py) - Profiling cProfile d'un workload représentatif

## Exécuter les Exemples

```bash
# Exemple simple
catnip docs/examples/basics/01_born_to_segfault.cat

# Avec module Python (utiliser import() dans le script pour les fichiers locaux)
catnip docs/examples/module-loading/02_demo_with_as.cat

# Mode verbeux
catnip -v docs/examples/functions/01_functions.cat

# REPL pour expérimenter
catnip
```
