# Catnip laguage

Version <!-- catnip:version -->0.0.5<!-- /catnip:version -->

Documentation : [https://docs.catnip-lang.io](https://docs.catnip-lang.io)

**Moteur de scripting embarqué pour applications Python**

Catnip est un langage de script conçu pour vivre **à l'intérieur** d'applications Python.
Il est embarquable pour définir des règles métier, des workflows ou des transformations de données sans modifier le code de l'application.

## Statut

Utilisé en production comme langage d'expressions pour manipuler des dataframes, sans problème.

C'est une **version zéro**, pas encore une release.

Qualité encore expérimentale pour :

- structs et héritage (cas diamond rejetés par design pour l'instant)
- système de modules très basique, namespaces globaux pour les modules, pas de chargement de modules compilés
- tools en chantier

## Démarrage rapide (mode embedded)

DSL personnalisé en 3 étapes :

```python
from catnip import Catnip, Context, pass_context

# 1. Contexte avec état spécifique
class ConfigContext(Context):
    def __init__(self, config: dict):
        super().__init__()
        self.config = config
        self.errors = []
        self.globals['config'] = config

# 2. Fonctions DSL exposées
class ConfigDSL(Catnip):
    @staticmethod
    def _required(ctx, field):
        if field not in ctx.config:
            ctx.errors.append(f"Champ requis '{field}' manquant")
            return False
        return True

    DSL_FUNCTIONS = dict(
        required=pass_context(_required),
    )

    def __init__(self, config: dict):
        context = ConfigContext(config)
        super().__init__(context=context)
        self.context.globals.update(self.DSL_FUNCTIONS)

# 3. Utilisez votre DSL
config = {'username': 'alice', 'email': 'alice@example.com'}

dsl = ConfigDSL(config)
dsl.parse("""
    required('username')
    required('email')
    required('age')  # Manquant, erreur détectée
""")
dsl.execute()

print(dsl.context.errors)
# ["Champ requis 'age' manquant"]
```

**Voir les exemples complets** dans [`docs/examples/embedding/`](docs/examples/embedding/) :

- Validation de configuration
- Pipelines ETL
- Sandbox Flask pour scripts utilisateur
- Moteur de règles métier (pricing, éligibilité)
- Génération de rapports
- Orchestration de workflows
- Magic commands Jupyter
- Dashboard Streamlit

## Modes d'utilisation

### Embedded (mode principal)

Le mode principal de Catnip (DSL).

**Cas d'usage** :

- Règles métier modifiables sans recompilation
- Scripts utilisateur sandboxés dans des applications web
- Pipelines de transformation de données (ETL)
- Workflows configurables (onboarding, traitement commandes)
- Validation de configuration avec règles complexes

**Avantages** :

- Sécurité : contexte isolé, seules les fonctions exposées sont accessibles
- Flexibilité : règles stockées en DB, modifiables par administrateurs
- Simplicité : syntaxe déclarative, plus accessible que Python
- Performance : moteur Rust (PyO3), compilation bytecode

### Standalone (scripts)

Scripts Catnip exécutés directement via CLI.

```bash
catnip script.cat
catnip -c "2 + 3"
echo "2 + 3" | catnip
```

### REPL (outils dev)

REPL interactif pour exploration et debugging.

```bash
catnip
# catnip> x = 10; y = x * 2; y
# 20
```

## Installation

```bash
pip install catnip-lang
```

## Propriétés du langage

Catnip combine une syntaxe lisible avec des primitives orientées DSL :

- ND-recursion : exploration non déterministe des branches, parallélisable sur plusieurs threads/cœurs.
- Exécution haute performance : VM Rust, bytecode, hot paths optimisés.
- Embedding sécurisé : contexte isolé, API DSL explicitement exposée.
- Pattern matching : logique déclarative lisible pour règles métier.
- Fonctions, closures et blocs-expressions : composition naturelle des règles.
- Broadcasting vectoriel : transformations de collections sans boilerplate.
- Structs : modélisation de données métier (héritage en évolution).
- Traits : composition de comportements réutilisables.
- Système de modules : organisation du code (actuellement basique).

Voir la [référence complète du langage](docs/lang/LANGUAGE.md) pour les détails et exemples.

## Architecture

**Pipeline de compilation** :

1. **Parsing** : Tree-sitter (Rust) -> IR
1. **Analyse sémantique** : optimisations (Rust)
1. **Exécution** : VM bytecode (Rust) avec NaN-boxing + JIT

**Performance** :

- Core 100% Rust (PyO3) pour hot paths
- VM stack-based avec dispatch O(1)
- JIT hot loop detection (100-200x speedup)
- Tail-call optimization (TCO) pour récursion terminale

## Documentation

- **[Guide utilisateur](docs/user/)** - Installation, CLI, REPL, chargement de modules
- **[Référence du langage](docs/lang/LANGUAGE.md)** - Syntaxe complète, pattern matching, broadcasting
- **[Exemples embedding](docs/examples/embedding/)** - 9 exemples complets de DSL
- **[Architecture interne](docs/dev/)** - Pipeline, VM, JIT, optimisations
- **[Introduction](docs/INTRODUCTION.md)** - Philosophie et design

## Build from source

```bash
# Prérequis : Rust toolchain, Python 3.11+, uv
make setup     # Setup complet (venv + deps + compile + install)
make test      # Exécuter les tests
```

## Licence

GPLv3
