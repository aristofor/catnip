# Guide d'Embedding Catnip

Guide complet pour embarquer Catnip comme moteur de scripting dans vos applications Python.

## Table des matières

- [Introduction](#introduction)
- [Pattern de base](#pattern-de-base)
- [Contexte personnalisé](#contexte-personnalis%C3%A9)
- [Injection de fonctions DSL](#injection-de-fonctions-dsl)
- [Gestion d'état](#gestion-d%C3%A9tat)
- [Sécurité et sandboxing](#s%C3%A9curit%C3%A9-et-sandboxing)
- [Patterns avancés](#patterns-avanc%C3%A9s)
- [Performance](#performance)
- [Exemples complets](#exemples-complets)

## Introduction

Catnip est pensé pour être embarqué dans des applications Python. Le mode embedded sert à :

**Avantages** :

- Scripts utilisateur exécutés dans un contexte isolé
- Règles métier modifiables sans recompilation
- Syntaxe déclarative accessible aux non-développeurs
- Exécution native (JIT)

**Use cases typiques** :

- Validation de configuration
- Pipelines ETL/transformation de données
- Moteur de règles métier (pricing, éligibilité)
- Workflows configurables
- Sandbox pour scripts utilisateur dans applications web

## Pattern de base

### Schéma minimum

```python
from catnip import Catnip

# 1. Créer instance
catnip = Catnip()

# 2. Parser le script
catnip.parse("x = 10; y = x * 2; y")

# 3. Exécuter
result = catnip.execute()  # 20
```

### Accès aux variables

```python
catnip = Catnip()
catnip.parse("x = 42")
catnip.execute()

# Lire une variable Catnip depuis Python
x = catnip.context.globals['x']  # 42

# Injecter une variable Python dans Catnip
catnip.context.globals['y'] = 100
catnip.parse("z = y * 2")
result = catnip.execute()  # 200
```

## Contexte personnalisé

Le contexte stocke l'état d'exécution et les variables. Sous-classez `Context` pour ajouter état et logique métier.

### Exemple : Contexte de validation

```python
from catnip import Context

class ValidationContext(Context):
    def __init__(self, data: dict, **kwargs):
        super().__init__(**kwargs)
        self._data = data
        self._errors = []
        # Expose data dans le script
        self.globals['data'] = data

    @property
    def errors(self) -> list:
        return self._errors

    def add_error(self, field: str, message: str):
        """Enregistre une erreur de validation."""
        self._errors.append({'field': field, 'message': message})

    def is_valid(self) -> bool:
        return len(self._errors) == 0
```

### Utilisation

```python
from catnip import Catnip

data = {'username': 'alice', 'age': 25}
ctx = ValidationContext(data)

catnip = Catnip(context=ctx)
catnip.parse("""
    if data['age'] < 18 {
        # add_error() sera exposée via @pass_context
    }
""")
catnip.execute()

if not ctx.is_valid():
    print(ctx.errors)
```

## Injection de fonctions DSL

Utilisez `@pass_context` pour exposer des fonctions Python dans les scripts Catnip.

### Pattern avec décorateur

```python
from catnip import Catnip, Context, pass_context

class MyDSL(Catnip):
    @staticmethod
    def _check_email(ctx, email: str):
        """Valide un email (fonction Python non exposée)."""
        return '@' in email

    @staticmethod
    def _validate_field(ctx, field: str, value):
        """Fonction exposée dans le script Catnip."""
        if not value:
            ctx.add_error(field, f"Champ '{field}' requis")
            return False
        return True

    # Dict des fonctions exposées
    DSL_FUNCTIONS = dict(
        validate_field=pass_context(_validate_field),
        check_email=pass_context(_check_email),
    )

    def __init__(self, context: Context):
        super().__init__(context=context)
        # Injecter dans le contexte
        self.context.globals.update(self.DSL_FUNCTIONS)
```

### Utilisation dans script

```python
# Script Catnip utilisant les fonctions DSL
script = """
    validate_field('username', data['username'])
    validate_field('email', data['email'])

    if check_email(data['email']) == False {
        # Email invalide
    }
"""

dsl = MyDSL(context=ctx)
dsl.parse(script)
dsl.execute()
```

### Pourquoi @pass_context ?

Le décorateur `@pass_context` injecte automatiquement le contexte comme premier argument :

```python
# Fonction Python
@staticmethod
def _my_func(ctx, arg1, arg2):
    ctx.some_state = arg1 + arg2
    return ctx.some_state

# Exposition dans Catnip
DSL_FUNCTIONS = dict(
    my_func=pass_context(_my_func),
)

# Utilisation dans script (ctx invisible)
script = "result = my_func(10, 20)"  # ctx passé automatiquement
```

## Gestion d'état

Le contexte permet de partager état entre Python et Catnip.

### Pattern : État mutable

```python
class StatefulContext(Context):
    def __init__(self):
        super().__init__()
        self._state = {}
        self.globals['state'] = self._state

    def set_state(self, key: str, value):
        self._state[key] = value
        # Synchroniser dans globals
        self.globals['state'][key] = value

    def get_state(self, key: str, default=None):
        return self._state.get(key, default)
```

### Utilisation

```python
class StatefulDSL(Catnip):
    @staticmethod
    def _set(ctx, key, value):
        ctx.set_state(key, value)
        return value

    @staticmethod
    def _get(ctx, key, default=None):
        return ctx.get_state(key, default)

    DSL_FUNCTIONS = dict(
        set=pass_context(_set),
        get=pass_context(_get),
    )

    def __init__(self):
        context = StatefulContext()
        super().__init__(context=context)
        self.context.globals.update(self.DSL_FUNCTIONS)

# Script Catnip
dsl = StatefulDSL()
dsl.parse("""
    set('counter', 0)
    count = get('counter', 0)
    set('counter', count + 1)
""")
dsl.execute()

# Accès depuis Python
print(dsl.context.get_state('counter'))  # 1
```

## Sécurité et sandboxing

### Isolation du contexte

Par défaut, Catnip expose les builtins Python (`len`, `range`, `str`, etc.). Pour un sandbox strict :

```python
class SecureContext(Context):
    def __init__(self, **kwargs):
        # Créer contexte vide
        super().__init__(**kwargs)
        # Retirer tous les builtins
        self.globals.clear()
        # Exposer uniquement ce qui est nécessaire
        self.globals['len'] = len
        # N'exposez PAS : open, eval, exec, __import__, etc.
```

### Validation des entrées

Toujours valider les données utilisateur avant exécution :

```python
class SafeDSL(Catnip):
    MAX_SCRIPT_SIZE = 10000  # 10KB max

    def parse(self, script: str):
        # Limiter taille du script
        if len(script) > self.MAX_SCRIPT_SIZE:
            raise ValueError(f"Script trop long (max {self.MAX_SCRIPT_SIZE} bytes)")

        # Parser et valider syntaxe
        super().parse(script)

    @staticmethod
    def _safe_operation(ctx, value):
        # Valider type et bornes
        if not isinstance(value, (int, float)):
            raise TypeError("Type invalide")
        if value > 1000000:
            raise ValueError("Valeur trop grande")
        return value
```

### Limiter les opérations

```python
class RateLimitedContext(Context):
    def __init__(self, max_operations=1000):
        super().__init__()
        self._op_count = 0
        self._max_ops = max_operations

    def check_limit(self):
        self._op_count += 1
        if self._op_count > self._max_ops:
            raise RuntimeError("Limite d'opérations atteinte")

# Injecter check dans chaque fonction DSL
@staticmethod
def _operation(ctx, arg):
    ctx.check_limit()
    # ... logique métier
```

### Timeout d'exécution

```python
import signal
from contextlib import contextmanager

@contextmanager
def timeout(seconds):
    def handler(signum, frame):
        raise TimeoutError(f"Exécution dépassée ({seconds}s)")

    signal.signal(signal.SIGALRM, handler)
    signal.alarm(seconds)
    try:
        yield
    finally:
        signal.alarm(0)

# Utilisation
try:
    with timeout(5):
        dsl.execute()
except TimeoutError as e:
    print(f"Script trop long : {e}")
```

## Patterns avancés

### Pattern : DSL chainable

```python
class ChainableContext(Context):
    def __init__(self, data):
        super().__init__()
        self._data = data
        self._result = data
        self.globals['data'] = data

    @property
    def result(self):
        return self._result

    def update_result(self, new_data):
        self._result = new_data
        self.globals['data'] = new_data

class ChainableDSL(Catnip):
    @staticmethod
    def _filter(ctx, condition_fn):
        """Filtre les données."""
        filtered = [x for x in ctx._result if condition_fn(x)]
        ctx.update_result(filtered)
        return filtered

    @staticmethod
    def _map(ctx, transform_fn):
        """Transforme les données."""
        mapped = [transform_fn(x) for x in ctx._result]
        ctx.update_result(mapped)
        return mapped

    DSL_FUNCTIONS = dict(
        filter=pass_context(_filter),
        map=pass_context(_map),
    )

# Utilisation chainable
script = """
    # Pipeline de transformation
    filter((x) => x > 10)
    map((x) => x * 2)
"""
```

### Pattern : Résultats structurés

```python
class ResultContext(Context):
    def __init__(self):
        super().__init__()
        self._results = {}
        self._metadata = {}

    def set_result(self, key: str, value, metadata: dict = None):
        self._results[key] = value
        if metadata:
            self._metadata[key] = metadata

    def get_results(self) -> dict:
        return {
            'results': self._results,
            'metadata': self._metadata,
        }

# Retour structuré
def execute_workflow(self, script: str) -> dict:
    self.parse(script)
    self.execute()
    return self.context.get_results()
```

### Pattern : Plugins et extensions

```python
class PluggableDSL(Catnip):
    def __init__(self, plugins: list = None):
        context = Context()
        super().__init__(context=context)

        # Charger plugins
        for plugin in plugins or []:
            self.load_plugin(plugin)

    def load_plugin(self, plugin):
        """Charge un plugin DSL."""
        if hasattr(plugin, 'DSL_FUNCTIONS'):
            self.context.globals.update(plugin.DSL_FUNCTIONS)

# Plugin externe
class MathPlugin:
    @staticmethod
    def _sqrt(ctx, x):
        import math
        return math.sqrt(x)

    DSL_FUNCTIONS = dict(
        sqrt=pass_context(_sqrt),
    )

# Utilisation
dsl = PluggableDSL(plugins=[MathPlugin()])
```

## Performance

### Réutiliser les instances

Créer un `Catnip()` par requête est acceptable, mais pour haute fréquence :

```python
class DSLPool:
    def __init__(self, size=10):
        self._pool = [MyDSL() for _ in range(size)]
        self._available = list(self._pool)

    def get(self):
        return self._available.pop()

    def release(self, dsl):
        # Réinitialiser le contexte
        dsl.context = MyContext()
        self._available.append(dsl)

# Utilisation
pool = DSLPool()
dsl = pool.get()
try:
    result = dsl.execute(script)
finally:
    pool.release(dsl)
```

### Compilation cache

Les scripts identiques sont automatiquement cachés par Catnip. Pour cache persistant :

```python
import hashlib
import pickle

class CachingDSL(Catnip):
    def __init__(self, cache_dir='/tmp/catnip-cache'):
        super().__init__()
        self._cache_dir = cache_dir

    def parse(self, script: str):
        # Hash du script
        script_hash = hashlib.sha256(script.encode()).hexdigest()
        cache_file = f"{self._cache_dir}/{script_hash}.cache"

        # Charger cache si disponible
        if os.path.exists(cache_file):
            with open(cache_file, 'rb') as f:
                self._ast = pickle.load(f)
        else:
            # Parser et sauvegarder
            super().parse(script)
            with open(cache_file, 'wb') as f:
                pickle.dump(self._ast, f)
```

### JIT et optimisations

Le JIT Catnip est automatique pour loops et fonctions récursives. Pour forcer :

```python
# Pragma dans script
script = """
    pragma("jit", True)

    fib = (n) => {
        if n <= 1 { n }
        else { fib(n-1) + fib(n-2) }
    }
    fib(30)  # Compilé en code natif après 100 appels
"""
```

## Exemples complets

Voir [`docs/examples/embedding/`](../examples/embedding/) pour 9 exemples complets :

1. **config_dsl.py** - Validation de configuration
1. **etl_pipeline.py** - Pipelines ETL
1. **flask_sandbox.py** - Sandbox Flask
1. **report_builder.py** - Génération de rapports
1. **rule_engine.py** - Moteur de règles métier
1. **workflow_dsl.py** - Orchestration de workflows
1. **jupyter_integration.py** - Magic commands IPython
1. **streamlit_app.py** - Dashboard Streamlit
1. **dataframe_dsl.py** - DSL pandas

## Checklist d'intégration

Avant de déployer un DSL Catnip en production :

- [ ] Contexte isolé avec uniquement les fonctions nécessaires exposées
- [ ] Validation des entrées (taille script, types, bornes)
- [ ] Timeout d'exécution configuré
- [ ] Limite d'opérations pour prévenir abus
- [ ] Gestion d'erreur propre (SyntaxError, RuntimeError, etc.)
- [ ] Logs des exécutions pour audit
- [ ] Tests des scripts malveillants potentiels
- [ ] Documentation claire pour les utilisateurs écrivant des scripts
- [ ] Exemples de scripts fournis

## Ressources

- [API Reference](../lang/LANGUAGE.md) - Syntaxe complète de Catnip
- [Context API](../user/EXTENDING_CONTEXT.md) - Détails de l'API Context
- [Performance](../dev/OPTIMIZATIONS.md) - Optimisations et profiling
