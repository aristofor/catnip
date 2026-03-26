#!/usr/bin/env python3
"""
Exemples pratiques d'extension de Catnip avec des fonctions personnalisées.
"""

from catnip import Catnip, Context, pass_context

# Exemple 1 : Calculatrice avec historique


def example_calculator():
    """Calculatrice qui garde l'historique des opérations."""
    print()
    print("⇒ Exemple 1 : Calculatrice avec historique\n")

    class Calculator:
        def __init__(self):
            self.history = []

        def add_to_history(self, operation, result):
            self.history.append({'operation': operation, 'result': result})
            return result

        def show_history(self):
            return self.history

        def clear_history(self):
            self.history = []
            return "History cleared"

    calc = Calculator()

    @pass_context
    def calculate(ctx, expr):
        """Évalue une expression et l'ajoute à l'historique."""
        result = ctx.result
        calc.add_to_history(expr, result)
        return result

    ctx = Context()
    ctx.globals['calc'] = calc
    ctx.globals['record'] = calculate

    catnip = Catnip(context=ctx)

    # Utilisation - opérations simples
    operations = [
        'x = 10 + 5',
        'y = x * 2',
        'z = y - 3',
        'z',
    ]

    for op in operations:
        catnip.parse(op)
        result = catnip.execute()
        print(f"{op:20} = {result}")

    print(f"\nNote: Les variables sont stockées dans le context local")


# Exemple 2 : Validation de données


def example_validation():
    """Système de validation de données avec règles personnalisées."""
    print()
    print("⇒ Exemple 2 : Validation de données\n")

    class Validator:
        def __init__(self):
            self.errors = []

        def is_email(self, value):
            """Vérifie si c'est un email valide (simplifié)."""
            return '@' in value and '.' in value.split('@')[1]

        def is_between(self, value, min_val, max_val):
            """Vérifie si la valeur est dans la plage."""
            return min_val <= value <= max_val

        def is_not_empty(self, value):
            """Vérifie que la valeur n'est pas vide."""
            return bool(value and str(value).strip())

        def add_error(self, field, message):
            """Ajoute une erreur."""
            self.errors.append({'field': field, 'message': message})

        def is_valid(self):
            """Retourne True si aucune erreur."""
            return len(self.errors) == 0

        def get_errors(self):
            """Retourne la liste des erreurs."""
            return self.errors

        def reset(self):
            """Réinitialise les erreurs."""
            self.errors = []

    validator = Validator()
    ctx = Context()
    ctx.globals['v'] = validator

    catnip = Catnip(context=ctx)

    # Données à valider (SimpleNamespace pour accès par attribut en Catnip)
    from types import SimpleNamespace

    user_data = SimpleNamespace(email='test@example.com', age=25, name='Alice')

    ctx.globals['user'] = user_data

    # Règles de validation en Catnip
    validation_rules = '''
        v.is_email(user.email) == False and v.add_error("email", "Email invalide")
        v.is_between(user.age, 18, 120) == False and v.add_error("age", "Age invalide")
        v.is_not_empty(user.name) == False and v.add_error("name", "Nom requis")
    '''

    catnip.parse(validation_rules)
    catnip.execute()

    if validator.is_valid():
        print("✓ Validation réussie !")
        print(f"  Données : {user_data}")
    else:
        print("✗ Erreurs de validation :")
        for error in validator.get_errors():
            print(f"  - {error['field']}: {error['message']}")

    # Test avec des données invalides
    print("\nTest avec données invalides :")
    validator.reset()
    user_data_invalid = SimpleNamespace(email='invalid-email', age=15, name='')
    ctx.globals['user'] = user_data_invalid

    catnip.parse(validation_rules)
    catnip.execute()

    if validator.is_valid():
        print("✓ Validation réussie !")
    else:
        print("✗ Erreurs de validation :")
        for error in validator.get_errors():
            print(f"  - {error['field']}: {error['message']}")


# Exemple 3 : Système de templates


def example_templates():
    """Système simple de templates avec variables."""
    print()
    print("⇒ Exemple 3 : Système de templates\n")

    def render(template, **variables):
        """Rend un template avec les variables fournies."""
        result = template
        for key, value in variables.items():
            result = result.replace(f'{{{key}}}', str(value))
        return result

    def upper(text):
        """Convertit en majuscules."""
        return str(text).upper()

    def lower(text):
        """Convertit en minuscules."""
        return str(text).lower()

    def capitalize(text):
        """Met la première lettre en majuscule."""
        return str(text).capitalize()

    ctx = Context()
    ctx.globals['render'] = render
    ctx.globals['upper'] = upper
    ctx.globals['lower'] = lower
    ctx.globals['capitalize'] = capitalize

    catnip = Catnip(context=ctx)

    # Template
    template = "Hello {name}, you have {count} new messages!"

    # Rendu avec variables
    code = '''
        name = "Alice"
        count = 5
        render(template, name=upper(name), count=count)
    '''

    ctx.globals['template'] = template
    catnip.parse(code)
    result = catnip.execute()

    print(f"Template: {template}")
    print(f"Rendu:    {result}")


# Exemple 4 : État partagé entre exécutions


def example_shared_state():
    """Démontre comment partager un état entre plusieurs exécutions."""
    print()
    print("⇒ Exemple 4 : État partagé\n")

    class Counter:
        def __init__(self):
            self.value = 0

        def increment(self, step=1):
            self.value += step
            return self.value

        def decrement(self, step=1):
            self.value -= step
            return self.value

        def reset(self):
            self.value = 0
            return self.value

        def get(self):
            return self.value

    counter = Counter()
    ctx = Context()
    ctx.globals['counter'] = counter

    catnip = Catnip(context=ctx)

    # Suite d'opérations
    operations = [
        'counter.increment(5)',
        'counter.increment(3)',
        'counter.decrement(2)',
        'counter.get()',
    ]

    print("Opérations sur le compteur :")
    for op in operations:
        catnip.parse(op)
        result = catnip.execute()
        print(f"  {op:30} → {result}")


# Exemple 5 : API fluente


def example_fluent_api():
    """Démontre une API fluente (method chaining)."""
    print()
    print("⇒ Exemple 5 : API fluente\n")

    class Query:
        def __init__(self):
            self.filters = []
            self.sort_field = None
            self.limit_value = None

        def where(self, field, operator, value):
            """Ajoute un filtre."""
            self.filters.append({'field': field, 'op': operator, 'value': value})
            return self

        def sort_by(self, field):
            """Définit le tri."""
            self.sort_field = field
            return self

        def limit(self, count):
            """Limite les résultats."""
            self.limit_value = count
            return self

        def execute(self):
            """Exécute la requête."""
            return {'filters': self.filters, 'sort': self.sort_field, 'limit': self.limit_value}

    def query():
        """Crée une nouvelle requête."""
        return Query()

    ctx = Context()
    ctx.globals['query'] = query

    catnip = Catnip(context=ctx)

    # Construire une requête fluente
    code = 'query().where("age", ">", 18).where("status", "==", "active").sort_by("name").limit(10).execute()'

    catnip.parse(code)
    result = catnip.execute()

    print("Requête construite :")
    print(f"  Filtres : {result['filters']}")
    print(f"  Tri     : {result['sort']}")
    print(f"  Limite  : {result['limit']}")


# Exemple 6 : Expressions mathématiques avancées


def example_math_extensions():
    """Ajoute des fonctions mathématiques avancées."""
    print()
    print("⇒ Exemple 6 : Extensions mathématiques\n")

    import math

    def factorial(n):
        """Calcule la factorielle."""
        if n <= 1:
            return 1
        return n * factorial(n - 1)

    def fibonacci(n):
        """Calcule le nième nombre de Fibonacci."""
        if n <= 1:
            return n
        return fibonacci(n - 1) + fibonacci(n - 2)

    def is_prime(n):
        """Vérifie si un nombre est premier."""
        if n < 2:
            return False
        for i in range(2, int(math.sqrt(n)) + 1):
            if n % i == 0:
                return False
        return True

    ctx = Context()
    ctx.globals['factorial'] = factorial
    ctx.globals['fib'] = fibonacci
    ctx.globals['is_prime'] = is_prime
    ctx.globals['sqrt'] = math.sqrt
    ctx.globals['pow'] = math.pow

    catnip = Catnip(context=ctx)

    # Tests
    tests = [
        ('factorial(5)', 120),
        ('fib(10)', 55),
        ('is_prime(17)', True),
        ('sqrt(16)', 4.0),
        ('pow(2, 8)', 256.0),
    ]

    print("Tests de fonctions mathématiques :")
    for expr, expected in tests:
        catnip.parse(expr)
        result = catnip.execute()
        status = "✓" if result == expected else "✗"
        print(f"  {status} {expr:20} = {result:10} (attendu: {expected})")


# Main


if __name__ == '__main__':
    print("=" * 70)
    print("EXEMPLES D'EXTENSION DE CATNIP")
    print("=" * 70)

    example_calculator()
    example_validation()
    example_templates()
    example_shared_state()
    example_fluent_api()
    example_math_extensions()

    print("\n" + "=" * 70)
    print("Tous les exemples sont terminés !")
    print("=" * 70)
