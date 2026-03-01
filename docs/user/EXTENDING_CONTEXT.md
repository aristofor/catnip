# Étendre le contexte

## Introduction

Catnip est volontairement **minimal côté I/O** : pas d'accès réseau/fichiers ni d'API haut niveau par défaut. `print` et
`write` servent juste la REPL ou l'exécution en console. L'hôte expose ensuite ce dont l'application a besoin, au bon
niveau de contrôle.

> Kernighan, Ritchie et Thompson auraient probablement approuvé un coeur sans I/O.

Le point d'extension principal est le **context** (`Context`). Il fait le lien entre l'application et le runtime, et
transporte :

- **`globals`** : dictionnaire de fonctions, classes et valeurs accessibles depuis Catnip
- **`locals`** : scope local (géré automatiquement par Catnip)
- **`result`** : dernier résultat (accessible via `ctx.result` côté hôte)

Par défaut, le context expose quelques primitives (ex: `print`, `write`, `len`) et un logger minimal, mais aucun accès
réseau ou fichier.

## Méthode 1 : Ajouter des fonctions simples

### Via le Context directement

```python
from catnip import Catnip, Context

# Créer un context personnalisé
ctx = Context()

# Ajouter vos fonctions
def greet(name):
    return f"Hello, {name}!"

def calculate_discount(price, percent):
    return price * (1 - percent / 100)

ctx.globals['greet'] = greet
ctx.globals['discount'] = calculate_discount

# Utiliser le context avec Catnip
catnip = Catnip(context=ctx)
code = catnip.parse('greet("Alice")')
result = catnip.execute()
print(result)  # "Hello, Alice!"
```

### Via le constructeur de Context

```python
from catnip import Context, Catnip

# Définir vos fonctions
def add_tax(price, rate=0.2):
    return price * (1 + rate)

def format_price(amount):
    return f"{amount:.2f}€"

# Créer le context avec vos fonctions
custom_globals = {
    'add_tax': add_tax,
    'format_price': format_price,
}

ctx = Context(globals=custom_globals)
catnip = Catnip(context=ctx)

# Utiliser depuis Catnip
code = catnip.parse('format_price(add_tax(100))')
result = catnip.execute()
print(result)  # "120.00€"
```

## Méthode 2 : Fonctions avec accès au Context

Pour les fonctions qui ont besoin d'accéder au context (pour lire/modifier des variables, accéder au résultat précédent,
etc.), utilisez le décorateur `@pass_context`.

```python
from catnip import Catnip, Context, pass_context

@pass_context
def store(ctx, key, value):
    """Stocke une valeur dans le context global."""
    ctx.globals[key] = value
    return value

@pass_context
def recall(ctx, key):
    """Récupère une valeur depuis le context global."""
    return ctx.globals.get(key, None)

@pass_context
def get_last_result(ctx):
    """Récupère le dernier résultat calculé."""
    return ctx.result

# Configuration
ctx = Context()
ctx.globals['store'] = store
ctx.globals['recall'] = recall
ctx.globals['last'] = get_last_result

catnip = Catnip(context=ctx)

# Utilisation
catnip.parse('store("user_score", 42)')
catnip.execute()

catnip.parse('recall("user_score") + 8')
result = catnip.execute()
print(result)  # 50
```

## Méthode 3 : Classes et objets

Vous pouvez exposer des classes et des objets complets à Catnip.

```python
from catnip import Catnip, Context

class DataStore:
    def __init__(self):
        self.data = {}

    def set(self, key, value):
        self.data[key] = value
        return value

    def get(self, key, default=None):
        return self.data.get(key, default)

    def all(self):
        return self.data

# Créer une instance
store = DataStore()

# Exposer l'objet
ctx = Context()
ctx.globals['db'] = store

catnip = Catnip(context=ctx)

# Utiliser depuis Catnip
catnip.parse('db.set("name", "Alice")')
catnip.execute()

catnip.parse('db.get("name")')
result = catnip.execute()
print(result)  # "Alice"

catnip.parse('db.all()')
result = catnip.execute()
print(result)  # {'name': 'Alice'}
```

## Méthode 4 : Modules complets

Pour exposer des modules Python entiers (comme `math`, `datetime`, etc.) :

```python
from catnip import Catnip, Context
import math
import datetime

ctx = Context()

# Exposer un module entier
ctx.globals['math'] = math
ctx.globals['datetime'] = datetime

catnip = Catnip(context=ctx)

# Utiliser les fonctions du module
catnip.parse('math.sqrt(16)')
result = catnip.execute()
print(result)  # 4.0

catnip.parse('math.pi * 2')
result = catnip.execute()
print(result)  # 6.283185307179586
```

## Exemples pratiques

### Exemple 1 : API de configuration

```python
from catnip import Catnip, Context

class Config:
    def __init__(self):
        self.settings = {
            'debug': False,
            'port': 8080,
            'host': 'localhost'
        }

    def get(self, key):
        return self.settings.get(key)

    def set(self, key, value):
        self.settings[key] = value
        return value

    def update(self, **kwargs):
        self.settings.update(kwargs)
        return self.settings

config = Config()
ctx = Context()
ctx.globals['config'] = config

catnip = Catnip(context=ctx)

# L'utilisateur peut configurer via Catnip
catnip.parse('''
    config.set("port", 3000)
    config.set("debug", True)
''')
catnip.execute()

print(config.settings)  # {'debug': True, 'port': 3000, 'host': 'localhost'}
```

### Exemple 2 : Pipeline de traitement de données

```python
from catnip import Catnip, Context

def load_data(source):
    """Simule le chargement de données."""
    return [1, 2, 3, 4, 5]

def transform(data, operation):
    """Applique une transformation."""
    if operation == 'double':
        return [x * 2 for x in data]
    elif operation == 'square':
        return [x ** 2 for x in data]
    return data

def aggregate(data, func_name):
    """Agrège les données."""
    if func_name == 'sum':
        return sum(data)
    elif func_name == 'avg':
        return sum(data) / len(data)
    return data

ctx = Context()
ctx.globals['load'] = load_data
ctx.globals['transform'] = transform
ctx.globals['aggregate'] = aggregate

catnip = Catnip(context=ctx)

# Pipeline défini en Catnip
pipeline = '''
    data = load("source")
    data = transform(data, "double")
    aggregate(data, "sum")
'''

code = catnip.parse(pipeline)
result = catnip.execute()
print(result)  # 30 (sum of [2, 4, 6, 8, 10])
```

### Exemple 3 : Système de règles métier

```python
from catnip import Catnip, Context, pass_context

class RuleEngine:
    def __init__(self):
        self.rules = []

    def add_rule(self, name, condition_code, action_code):
        self.rules.append({
            'name': name,
            'condition': condition_code,
            'action': action_code
        })

    def evaluate(self, catnip, data):
        """Évalue les règles pour un ensemble de données."""
        results = []
        for rule in self.rules:
            # Mettre les données dans le context
            catnip.context.globals['data'] = data

            # Évaluer la condition
            catnip.parse(rule['condition'])
            condition_result = catnip.execute()

            if condition_result:
                # Exécuter l'action
                catnip.parse(rule['action'])
                action_result = catnip.execute()
                results.append({
                    'rule': rule['name'],
                    'result': action_result
                })

        return results

# Configuration
engine = RuleEngine()
ctx = Context()
ctx.globals['engine'] = engine

catnip = Catnip(context=ctx)

# Définir des règles
engine.add_rule(
    'discount_high_value',
    'data.price > 100',
    'data.price * 0.9'  # 10% discount
)

engine.add_rule(
    'discount_quantity',
    'data.quantity >= 10',
    'data.price * 0.85'  # 15% discount
)

# Évaluer
data = {'price': 120, 'quantity': 5}
results = engine.evaluate(catnip, data)
print(results)  # [{'rule': 'discount_high_value', 'result': 108.0}]

data = {'price': 80, 'quantity': 15}
results = engine.evaluate(catnip, data)
print(results)  # [{'rule': 'discount_quantity', 'result': 68.0}]
```

## Bonnes pratiques

### 1. Nommage cohérent

```python
# BON : noms descriptifs
ctx.globals['calculate_tax'] = calculate_tax
ctx.globals['format_date'] = format_date
ctx.globals['validate_email'] = validate_email

# ÉVITER : noms cryptiques
ctx.globals['ct'] = calculate_tax
ctx.globals['fd'] = format_date
```

### 2. Documentation des fonctions

```python
def process_payment(amount, method):
    """
    Traite un paiement.

    Args:
        amount (float): Montant du paiement
        method (str): Méthode de paiement ('card', 'cash', 'paypal')

    Returns:
        dict: Résultat du paiement avec status et transaction_id
    """
    return {'status': 'success', 'transaction_id': '12345'}
```

### 3. Validation des entrées

```python
def divide(a, b):
    """Division sécurisée."""
    if b == 0:
        raise ValueError("Division par zéro interdite")
    return a / b

ctx.globals['divide'] = divide
```

### 4. Isolation des contextes

Pour des environnements d'exécution multiples (multi-tenant, sandbox, etc.) :

```python
def create_sandbox_context():
    """Crée un context isolé avec des fonctions limitées."""
    ctx = Context(globals={
        # Seulement les fonctions sûres
        'len': len,
        'str': str,
        'int': int,
        'range': range,
    })
    return ctx

# Chaque utilisateur a son propre context
user1_ctx = create_sandbox_context()
user2_ctx = create_sandbox_context()

user1_catnip = Catnip(context=user1_ctx)
user2_catnip = Catnip(context=user2_ctx)
```

### 5. Gestion des erreurs

```python
from catnip import Catnip, Context

def safe_execute(catnip, code):
    """Exécute du code Catnip avec gestion d'erreurs."""
    try:
        catnip.parse(code)
        return {'success': True, 'result': catnip.execute()}
    except Exception as e:
        return {'success': False, 'error': str(e)}

ctx = Context()
catnip = Catnip(context=ctx)

result = safe_execute(catnip, '1 + 1')
print(result)  # {'success': True, 'result': 2}

result = safe_execute(catnip, 'unknown_function()')
print(result)  # {'success': False, 'error': '…'}
```

## Context avancé : locals vs globals

- **`globals`** : Variables et fonctions disponibles partout
- **`locals`** : Variables de scope local (fonction, bloc)

```python
from catnip import Catnip, Context

ctx = Context()

# Variables globales
ctx.globals['PI'] = 3.14159
ctx.globals['TAX_RATE'] = 0.2

catnip = Catnip(context=ctx)

code = '''
    calculate_total = (price) => {
        tax = price * TAX_RATE
        price + tax
    }
    calculate_total(100)
'''

catnip.parse(code)
result = catnip.execute()
print(result)  # 120.0
```

## Logger personnalisé

Catnip expose un objet `logger` et une fonction `debug()` dans tous les contextes. Par défaut, un `MinimalLogger`
affiche les messages avec un préfixe `[DEBUG]`, `[INFO]`, etc.

Pour un contrôle total du logging, fournir un logger personnalisé au Context :

```python
from catnip import Catnip
from catnip.context import Context

class CustomLogger:
    """Logger personnalisé pour Catnip."""

    def print(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        # Intégration avec votre système de logging
        my_app_logger.debug(msg)

    def info(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        my_app_logger.info(msg)

    def warning(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        my_app_logger.warning(msg)

    def error(self, *args, sep=' '):
        msg = sep.join(str(arg) for arg in args)
        my_app_logger.error(msg)

# Créer le context avec le logger custom
ctx = Context(logger=CustomLogger())
catnip = Catnip(context=ctx)

# Utiliser depuis Catnip
code = '''
    print("Démarrage du traitement")
    logger.info("Configuration chargée")
    resultat = 42
    logger.debug("Résultat:", resultat)
'''

catnip.parse(code)
catnip.execute()
```

**Note** : Le logger est toujours exposé via `logger` et `debug()` dans les globals, même si tu fournis un dictionnaire
`globals` personnalisé au Context.

## Résumé

Catnip offre plusieurs façons d'étendre ses capacités :

1. **Fonctions simples** : `ctx.globals['func'] = my_function`
1. **Fonctions avec context** : `@pass_context` decorator
1. **Objets et classes** : `ctx.globals['obj'] = my_object`
1. **Modules complets** : `ctx.globals['math'] = math`
1. **Logger personnalisé** : `Context(logger=custom_logger)`

Le design minimaliste de Catnip permet de créer des DSL (Domain-Specific Languages) sur mesure pour votre application,
en exposant uniquement les fonctionnalités nécessaires.
