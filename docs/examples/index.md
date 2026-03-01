# Exemples Catnip

> Les exemples avancés et multi-fichiers sont dans [advanced/](advanced/).

## Bases (`basics/`)

Exemples fondamentaux :

- [`01_born_to_segfault.cat`](basics/01_born_to_segfault.cat) - Premier programme, debug et affichage
- [`02_loops.cat`](basics/02_loops.cat) - Boucles for et while
- [`03_statement_separators.cat`](basics/03_statement_separators.cat) - Newlines, semicolons et séparateurs mixtes
- [`04_repl_multiline_demo.txt`](basics/04_repl_multiline_demo.txt) - Utilisation de la REPL en mode multiline
- [`05_fstrings.cat`](basics/05_fstrings.cat) - F-strings: interpolation, format specs, conversion et debug

## Fonctions (`functions/`)

Fonctions, lambdas et closures :

- [`01_functions.cat`](functions/01_functions.cat) - Définition de fonctions, paramètres, defaults
- [`02_lambdas.cat`](functions/02_lambdas.cat) - Fonctions anonymes, lambdas inline
- [`03_closures_and_higher_order.cat`](functions/03_closures_and_higher_order.cat) - Closures, fonctions de fonctions,
  composition
- [`04_tail_recursion.cat`](functions/04_tail_recursion.cat) - Récursion terminale et transformation automatique en
  boucles

## Pattern Matching (`pattern-matching/`)

Matching et destructuration :

- [`01_pattern_matching.cat`](pattern-matching/01_pattern_matching.cat) - match/case, patterns littéraux, variables,
  guards, OR patterns

## Flux de Contrôle (`control-flow/`)

Structures de contrôle avancées :

- [`01_break_example.cat`](control-flow/01_break_example.cat) - Sortie anticipée de boucle
- [`02_continue_example.cat`](control-flow/02_continue_example.cat) - Passage à l'itération suivante
- [`03_break_continue_combined.cat`](control-flow/03_break_continue_combined.cat) - Combinaison des deux

## Avancé (`advanced/`)

Fonctionnalités avancées :

- [`01_advanced_techniques.cat`](advanced/01_advanced_techniques.cat) - Techniques avancées combinées
- [`02_blocks_and_expressions.cat`](advanced/02_blocks_and_expressions.cat) - Blocs comme expressions
- [`03_data_structures.cat`](advanced/03_data_structures.cat) - Manipulation de structures complexes
- [`04_literals_and_variadic.cat`](advanced/04_literals_and_variadic.cat) - Littéraux et fonctions variadiques
- [`05_pragma_features.cat`](advanced/05_pragma_features.cat) - Pragmas et directives
- [`06_fstrings.cat`](advanced/06_fstrings.cat) - Formatage de chaînes
- [`07_binary_search.cat`](advanced/07_binary_search.cat) - Algorithme de recherche binaire
- [`08_turing_completeness.cat`](advanced/08_turing_completeness.cat) - Démonstration de Turing-complétude
- [`09_assignment_syntax.cat`](advanced/09_assignment_syntax.cat) - Affectation d'attributs et par index
- [`10_nd_recursion.cat`](advanced/10_nd_recursion.cat) - Récursion N-dimensionnelle
- [`11_pickle_example.py`](advanced/11_pickle_example.py) - Sérialisation pickle (AST, Scope, lambdas, closures)
- [`12_structs.cat`](advanced/12_structs.cat) - Structures nommées (déclaration, instanciation, mutation, imbrication)
- [`13_traits.cat`](advanced/13_traits.cat) - Traits, composition et résolution de conflits

## Broadcasting (`broadcast/`)

Opérations vectorielles sur collections :

- [`01_scalar_broadcasting.cat`](broadcast/01_scalar_broadcasting.cat) - Broadcasting sur scalaire
- [`02_basic_broadcasting.cat`](broadcast/02_basic_broadcasting.cat) - Syntaxe `A.[op M]`, bases
- [`03_math_operations.cat`](broadcast/03_math_operations.cat) - Opérations mathématiques vectorielles
- [`04_comparisons.cat`](broadcast/04_comparisons.cat) - Comparaisons et filtrage
- [`05_boolean_logic.cat`](broadcast/05_boolean_logic.cat) - Logique booléenne distribuée
- [`06_nested_structures.cat`](broadcast/06_nested_structures.cat) - Broadcasting sur structures imbriquées
- [`07_cas_illustratifs.cat`](broadcast/07_cas_illustratifs.cat) - Cas d'usage illustratifs
- [`bench_broadcast.py`](broadcast/bench_broadcast.py) - Benchmarks de performance

## Chargement de Modules (`module-loading/`)

Intégration avec Python :

- [`index.md`](module-loading/index.md) - Mécanismes de base
- [`01_demo.cat`](module-loading/01_demo.cat) - Mode namespace par défaut
- [`02_demo_with_as.cat`](module-loading/02_demo_with_as.cat) - Namespace personnalisé
- [`03_host_module_example.py`](module-loading/03_host_module_example.py) - Module Python exemple
- [`04_app_script.cat`](module-loading/04_app_script.cat) - Script utilisant l'API
- [`05_simple_extension.py`](module-loading/05_simple_extension.py) - Extension simple
- [`06_custom_functions.py`](module-loading/06_custom_functions.py) - Fonctions personnalisées
- [`07_app_api.py`](module-loading/07_app_api.py) - API d'application simulée
- [`08_aesgcm.cat`](module-loading/08_aesgcm.cat) - AES-GCM avec propagation d'erreur Python

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

## Standalone (`standalone/`)

Scripts Catnip exécutables en ligne de commande :

- [`01_calculate.cat`](standalone/01_calculate.cat) - Calculatrice et formules
- [`02_filter_data.cat`](standalone/02_filter_data.cat) - Filtrage et transformations
- [`03_transform_csv.cat`](standalone/03_transform_csv.cat) - Transformation de données tabulaires
- [`04_config_validator.cat`](standalone/04_config_validator.cat) - Validation de configuration
- [`05_data_report.cat`](standalone/05_data_report.cat) - Rapport de données

## Performance

Section temporairement retiree de la documentation publique.

Les exemples de performance ont ete deplaces sous `wip/examples/performance/` pour refonte.

## Control Flow Graph (`cfg/`)

Analyse et optimisation du flux de contrôle :

- [`01_cfg_basic.md`](cfg/01_cfg_basic.md) - Construction de CFG, blocs basiques, edges
- [`02_cfg_analysis.md`](cfg/02_cfg_analysis.md) - Dominance, détection de boucles
- [`03_cfg_optimization.md`](cfg/03_cfg_optimization.md) - Optimisations (dead code, fusion, branches)
- [`04_cfg_reconstruction.md`](cfg/04_cfg_reconstruction.md) - Reconstruction de code structuré depuis CFG

## Outils (`tools/`)

Utilitaires de développement :

- [`extract_grammar_demo.py`](tools/extract_grammar_demo.py) - Extraction de la grammaire Tree-sitter
- [`json_ir_analysis.py`](tools/json_ir_analysis.py) - Analyse programmatique de l'IR via sérialisation JSON

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
