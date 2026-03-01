#!/usr/bin/env python3
"""
Exemple simple et fonctionnel d'extension de Catnip.
"""

from catnip import Catnip, Context, pass_context


# Exemple 1 : Fonctions simples


def example_simple_functions():
    """Ajouter des fonctions simples au context."""
    print("\n⇒ Exemple 1 : Fonctions simples\n")

    def greet(name):
        return f"Hello, {name}!"

    def add_tax(price, rate=0.2):
        return price * (1 + rate)

    # Créer un context et ajouter les fonctions
    ctx = Context()
    ctx.globals['greet'] = greet
    ctx.globals['add_tax'] = add_tax

    # Créer une instance Catnip
    catnip = Catnip(context=ctx)

    # Test 1
    catnip.parse('greet("Alice")')
    result = catnip.execute()
    print(f'greet("Alice") = {result}')

    # Test 2
    catnip.parse('add_tax(100)')
    result = catnip.execute()
    print(f'add_tax(100) = {result}')

    # Test 3
    catnip.parse('add_tax(100, 0.1)')
    result = catnip.execute()
    print(f'add_tax(100, 0.1) = {result}')


# Exemple 2 : Objet avec méthodes


def example_object_methods():
    """Exposer un objet avec des méthodes."""
    print("\n⇒ Exemple 2 : Objet avec méthodes\n")

    class Calculator:
        def __init__(self):
            self.memory = 0

        def add(self, a, b):
            result = a + b
            self.memory = result
            return result

        def multiply(self, a, b):
            result = a * b
            self.memory = result
            return result

        def get_memory(self):
            return self.memory

    calc = Calculator()

    ctx = Context()
    ctx.globals['calc'] = calc

    catnip = Catnip(context=ctx)

    # Tests
    catnip.parse('calc.add(5, 3)')
    result = catnip.execute()
    print(f'calc.add(5, 3) = {result}')

    catnip.parse('calc.multiply(4, 7)')
    result = catnip.execute()
    print(f'calc.multiply(4, 7) = {result}')

    catnip.parse('calc.get_memory()')
    result = catnip.execute()
    print(f'calc.get_memory() = {result}')


# Exemple 3 : Fonction avec accès au context


def example_context_access():
    """Fonction qui accède au context avec @pass_context."""
    print("\n⇒ Exemple 3 : Fonction avec accès au context\n")

    @pass_context
    def store(ctx, key, value):
        """Stocke une valeur dans les globals."""
        ctx.globals[key] = value
        return f"Stored '{key}' = {value}"

    @pass_context
    def recall(ctx, key):
        """Récupère une valeur des globals."""
        return ctx.globals.get(key, "Not found")

    ctx = Context()
    ctx.globals['store'] = store
    ctx.globals['recall'] = recall

    catnip = Catnip(context=ctx)

    # Stocker une valeur
    catnip.parse('store("username", "Alice")')
    result = catnip.execute()
    print(result)

    # Récupérer la valeur
    catnip.parse('recall("username")')
    result = catnip.execute()
    print(f'recall("username") = {result}')


# Exemple 4 : Module Python


def example_python_module():
    """Exposer un module Python complet."""
    print("\n⇒ Exemple 4 : Module Python\n")

    import math

    ctx = Context()
    ctx.globals['math'] = math

    catnip = Catnip(context=ctx)

    # Tests
    tests = [
        'math.sqrt(16)',
        'math.pow(2, 8)',
        'math.pi',
    ]

    for test in tests:
        catnip.parse(test)
        result = catnip.execute()
        print(f'{test:20} = {result}')


# Exemple 5 : État partagé


def example_shared_state():
    """État partagé entre plusieurs exécutions."""
    print("\n⇒ Exemple 5 : État partagé\n")

    class DataStore:
        def __init__(self):
            self.data = {}

        def set(self, key, value):
            self.data[key] = value
            return value

        def get(self, key):
            return self.data.get(key)

        def all(self):
            return dict(self.data)

    store = DataStore()
    ctx = Context()
    ctx.globals['store'] = store

    catnip = Catnip(context=ctx)

    # Séquence d'opérations
    operations = [
        ('store.set("name", "Bob")', "Définit le nom"),
        ('store.set("age", 30)', "Définit l'âge"),
        ('store.get("name")', "Récupère le nom"),
        ('store.all()', "Récupère tout"),
    ]

    for code, description in operations:
        catnip.parse(code)
        result = catnip.execute()
        print(f'{code:30} # {description}')
        print(f'{"":30} → {result}')


# Exemple 6 : Variables et calculs


def example_variables():
    """Utilisation de variables dans Catnip."""
    print("\n⇒ Exemple 6 : Variables et calculs\n")

    ctx = Context()
    catnip = Catnip(context=ctx)

    # Code Catnip avec variables
    code = '''
        price = 100
        tax_rate = 0.2
        tax = price * tax_rate
        total = price + tax
        total
    '''

    catnip.parse(code)
    result = catnip.execute()
    print(f'Code Catnip :')
    print(code)
    print(f'\nRésultat: {result}')


# Exemple 7 : API personnalisée


def example_custom_api():
    """Créer une API personnalisée pour votre application."""
    print("\n⇒ Exemple 7 : API personnalisée\n")

    class UserAPI:
        def __init__(self):
            self.users = {}

        def create_user(self, username, email):
            user_id = len(self.users) + 1
            self.users[user_id] = {'id': user_id, 'username': username, 'email': email}
            return user_id

        def get_user(self, user_id):
            return self.users.get(user_id)

        def list_users(self):
            return list(self.users.values())

    api = UserAPI()
    ctx = Context()
    ctx.globals['users'] = api

    catnip = Catnip(context=ctx)

    # Créer des utilisateurs
    print("Création d'utilisateurs:")
    catnip.parse('users.create_user("alice", "alice@example.com")')
    id1 = catnip.execute()
    print(f'  User créé avec ID: {id1}')

    catnip.parse('users.create_user("bob", "bob@example.com")')
    id2 = catnip.execute()
    print(f'  User créé avec ID: {id2}')

    # Lister les utilisateurs
    print("\nListe des utilisateurs:")
    catnip.parse('users.list_users()')
    result = catnip.execute()
    for user in result:
        print(f'  - {user["username"]} ({user["email"]})')


# Main

if __name__ == '__main__':
    print("=" * 70)
    print("EXEMPLES SIMPLES D'EXTENSION DE CATNIP")
    print("=" * 70)

    example_simple_functions()
    example_object_methods()
    example_context_access()
    example_python_module()
    example_shared_state()
    example_variables()
    example_custom_api()

    print("\n" + "=" * 70)
    print("✓ Tous les exemples fonctionnent correctement !")
    print("=" * 70)
