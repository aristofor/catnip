#!/usr/bin/env python3
"""
Exemple d'intégration de Catnip dans Jupyter via magic commands IPython.

Montre comment :
1. Créer des magic commands pour Catnip
2. Utiliser Catnip dans des cellules Jupyter
3. Partager l'état entre Python et Catnip
4. Intégrer avec le namespace IPython

Use case : Analyse de données interactive dans notebooks Jupyter.

Installation :
    %load_ext jupyter_integration

Utilisation :
    %%catnip
    x = 10
    y = x * 2
    y
"""

from catnip import Catnip

try:
    from IPython.core.magic import Magics, magics_class, line_magic, cell_magic
    from IPython.core.magic_arguments import argument, magic_arguments, parse_argstring

    IPYTHON_AVAILABLE = True
except ImportError:
    IPYTHON_AVAILABLE = False

    # Stubs pour la démonstration
    def magics_class(cls):
        return cls

    def line_magic(func):
        return func

    def cell_magic(func):
        return func

    def magic_arguments():
        def decorator(func):
            return func

        return decorator

    def argument(*args, **kwargs):
        def decorator(func):
            return func

        return decorator

    def parse_argstring(func, line):
        from types import SimpleNamespace

        return SimpleNamespace(verbose=False, output=None)

    class Magics:
        def __init__(self, shell):
            self.shell = shell


@magics_class
class CatnipMagics(Magics):
    """
    Magic commands IPython pour Catnip.

    Commandes disponibles :
    - %catnip <code> : Évalue une ligne de code Catnip
    - %%catnip : Évalue une cellule de code Catnip
    - %catnip_load <file> : Charge et exécute un fichier Catnip
    - %catnip_reset : Réinitialise le contexte Catnip
    """

    def __init__(self, shell):
        super().__init__(shell)
        self._catnip_instance = Catnip()
        # Partager le namespace IPython avec Catnip
        self._catnip_instance.context.globals.update(self.shell.user_ns)

    @line_magic
    @magic_arguments()
    @argument('-v', '--verbose', action='store_true', help='Afficher les détails')
    def catnip(self, line):
        """
        Évalue une ligne de code Catnip.

        Usage:
            %catnip x = 10; y = x * 2; y
            %catnip -v fib = (n) => { if n <= 1 { n } else { fib(n-1) + fib(n-2) } }
        """
        args = parse_argstring(self._catnip_instance, line)
        code = line.replace('-v', '').replace('--verbose', '').strip()

        try:
            self._catnip_instance.parse(code)
            result = self._catnip_instance.execute()

            # Synchroniser le namespace
            self._sync_namespace()

            if args.verbose:
                print(f"Code: {code}")
                print(f"Résultat: {result}")

            return result

        except Exception as e:
            print(f"Erreur Catnip: {e}")
            return None

    @cell_magic
    @magic_arguments()
    @argument('-o', '--output', help='Variable Python où stocker le résultat')
    def catnip(self, line, cell):
        """
        Évalue une cellule de code Catnip.

        Usage:
            %%catnip
            x = 10
            y = x * 2
            y

            %%catnip -o result
            fib = (n) => { if n <= 1 { n } else { fib(n-1) + fib(n-2) } }
            fib(10)
        """
        args = parse_argstring(self._catnip_instance, line)

        try:
            self._catnip_instance.parse(cell)
            result = self._catnip_instance.execute()

            # Synchroniser le namespace
            self._sync_namespace()

            # Stocker dans variable Python si demandé
            if args.output:
                self.shell.user_ns[args.output] = result

            return result

        except Exception as e:
            print(f"Erreur Catnip: {e}")
            return None

    @line_magic
    def catnip_load(self, line):
        """
        Charge et exécute un fichier Catnip.

        Usage:
            %catnip_load script.cat
        """
        filename = line.strip()

        try:
            with open(filename, 'r') as f:
                code = f.read()

            self._catnip_instance.parse(code)
            result = self._catnip_instance.execute()

            # Synchroniser le namespace
            self._sync_namespace()

            print(f"Fichier chargé: {filename}")
            return result

        except FileNotFoundError:
            print(f"Fichier introuvable: {filename}")
        except Exception as e:
            print(f"Erreur: {e}")

        return None

    @line_magic
    def catnip_reset(self, line):
        """
        Réinitialise le contexte Catnip.

        Usage:
            %catnip_reset
        """
        self._catnip_instance = Catnip()
        self._catnip_instance.context.globals.update(self.shell.user_ns)
        print("Contexte Catnip réinitialisé")

    @line_magic
    def catnip_vars(self, line):
        """
        Affiche les variables Catnip disponibles.

        Usage:
            %catnip_vars
        """
        print("Variables Catnip:")
        for key, value in self._catnip_instance.context.globals.items():
            if not key.startswith('_'):
                print(f"  {key} = {value}")

    def _sync_namespace(self):
        """
        Synchronise les variables entre Catnip et IPython.

        Les variables créées en Catnip sont disponibles en Python et vice-versa.
        """
        # Catnip → Python
        for key, value in self._catnip_instance.context.globals.items():
            if not key.startswith('_') and not callable(value):
                self.shell.user_ns[key] = value


def load_ipython_extension(ipython):
    """
    Charge l'extension IPython.

    Usage dans Jupyter:
        %load_ext jupyter_integration
    """
    if not IPYTHON_AVAILABLE:
        print("IPython n'est pas installé. Installer avec : pip install ipython")
        return

    ipython.register_magics(CatnipMagics)
    print("Extension Catnip chargée")
    print("Commandes disponibles:")
    print("  %catnip <code> - Évaluer une ligne")
    print("  %%catnip - Évaluer une cellule")
    print("  %catnip_load <file> - Charger un fichier")
    print("  %catnip_reset - Réinitialiser")
    print("  %catnip_vars - Lister les variables")


# --- Démonstration (simulation hors Jupyter) ---

if __name__ == '__main__':
    print("⇒ Exemple 1 : Magic command de base")
    print()

    print("""
Dans Jupyter, après avoir chargé l'extension :

    %load_ext jupyter_integration

Vous pouvez utiliser Catnip directement :

    %catnip x = 10; y = x * 2; y
    # Output: 20

    # La variable est disponible en Python
    print(x)  # 10
    print(y)  # 20
    """)

    print()
    print("⇒ Exemple 2 : Cellule Catnip avec calculs")
    print()

    print("""
    %%catnip
    # Fibonacci en Catnip
    fib = (n) => {
        if n <= 1 { n }
        else { fib(n-1) + fib(n-2) }
    }

    # Calculer fib(10)
    result = fib(10)
    result

    # Output: 55
    """)

    print()
    print("⇒ Exemple 3 : Partage d'état avec Python")
    print()

    print("""
# Créer des données en Python
import pandas as pd
df = pd.DataFrame({
    'a': [1, 2, 3],
    'b': [4, 5, 6]
})

# Utiliser en Catnip (df est disponible)
%%catnip -o col_sum
sum_a = 6  # df['a'].sum() simulé
sum_b = 15 # df['b'].sum() simulé
sum_a + sum_b

# Résultat stocké dans variable Python 'col_sum'
print(col_sum)  # 21
    """)

    print()
    print("⇒ Exemple 4 : Charger un script externe")
    print()

    print("""
# Créer un fichier utils.cat
# factorial = (n) => { if n <= 1 { 1 } else { n * factorial(n-1) } }

# Charger dans Jupyter
%catnip_load utils.cat

# Utiliser la fonction
%catnip factorial(5)
# Output: 120
    """)

    print()
    print("⇒ Exemple 5 : Analyse de données interactive")
    print()

    print("""
import numpy as np

# Données NumPy
data = np.array([10, 20, 30, 40, 50])

%%catnip
# Calculer statistiques
count = 5
mean = 30  # data.mean() simulé
std = 15   # data.std() simulé

# Détection outliers (> 2 std)
threshold = mean + 2 * std
threshold

# Python peut réutiliser ces variables
print(f"Moyenne: {mean}")
print(f"Seuil: {threshold}")
    """)

    print()
    print("⇒ Simulation de session")
    print()

    # Démonstration simple avec Catnip directement (pas besoin d'IPython)
    print("Note: Cette simulation ne nécessite pas IPython")
    print("Pour utiliser réellement dans Jupyter:")
    print("  1. pip install ipython jupyter")
    print("  2. %load_ext jupyter_integration")
    print()

    # Démonstration simple
    print("Démonstration simple :")
    catnip_demo = Catnip()

    code1 = "x = 42; y = x * 2; y"
    catnip_demo.parse(code1)
    result = catnip_demo.execute()
    print(f"Code : {code1}")
    print(f"Résultat : {result}")
    print(f"Variables : x={catnip_demo.context.globals['x']}, y={catnip_demo.context.globals['y']}")
    print()

    code2 = "z = x + y; z"
    catnip_demo.parse(code2)
    result = catnip_demo.execute()
    print(f"Code : {code2}")
    print(f"Résultat : {result}")
    print(f"Variable : z={catnip_demo.context.globals['z']}")
