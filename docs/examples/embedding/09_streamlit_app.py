#!/usr/bin/env python3
"""
Exemple d'intégration de Catnip dans Streamlit pour dashboards interactifs.

Montre comment :
1. Créer un playground Catnip dans Streamlit
2. Exécuter du code Catnip avec état partagé
3. Visualiser les résultats en temps réel
4. Créer des dashboards de calcul interactifs

Use case : Dashboards de calcul, playground interactif, notebooks web.

Installation :
    pip install streamlit

Utilisation :
    streamlit run streamlit_app.py
"""

try:
    import streamlit as st

    STREAMLIT_AVAILABLE = True
except ImportError:
    STREAMLIT_AVAILABLE = False

    # Mock pour la démonstration
    class StreamlitMock:
        def title(self, text):
            print(f"# {text}")

        def header(self, text):
            print(f"## {text}")

        def subheader(self, text):
            print(f"### {text}")

        def text_area(self, label, value='', height=None):
            return value

        def button(self, label):
            return False

        def write(self, *args):
            print(*args)

        def code(self, text, language=None):
            print(f"```{language or ''}\n{text}\n```")

        def error(self, text):
            print(f"ERROR: {text}")

        def success(self, text):
            print(f"SUCCESS: {text}")

        def sidebar(self):
            return self

        def selectbox(self, label, options):
            return options[0]

        def checkbox(self, label, value=False):
            return value

        def markdown(self, text):
            print(text)

    st = StreamlitMock()

from catnip import Catnip


def catnip_playground():
    """
    Playground Catnip interactif dans Streamlit.

    Interface pour écrire et exécuter du code Catnip avec visualisation des résultats.
    """
    st.title("🐱 Catnip Playground")
    st.markdown("Playground interactif pour expérimenter avec Catnip")

    # Sidebar avec exemples
    st.sidebar.header("Exemples")
    example = st.sidebar.selectbox(
        "Choisir un exemple",
        [
            "Vide",
            "Fibonacci",
            "Calculs mathématiques",
            "Traitement de liste",
            "Conditions et boucles",
        ],
    )

    # Exemples pré-définis
    examples = {
        "Vide": "",
        "Fibonacci": """# Fonction fibonacci récursive
fib = (n) => {
    if n <= 1 { n }
    else { fib(n-1) + fib(n-2) }
}

# Calculer fib(10)
result = fib(10)
result""",
        "Calculs mathématiques": """# Opérations mathématiques
x = 10
y = 20
z = 30

# Moyenne
avg = (x + y + z) / 3

# Produit
product = x * y * z

# Résultat
result = 'Moyenne: ' + avg + ', Produit: ' + product
result""",
        "Traitement de liste": """# Créer une liste
numbers = list(1, 2, 3, 4, 5)

# Somme
sum = 0
i = 0
while i < 5 {
    sum = sum + numbers[i]
    i = i + 1
}

sum""",
        "Conditions et boucles": """# Compter les pairs
count = 0
n = 1
while n <= 10 {
    if n % 2 == 0 {
        count = count + 1
    }
    n = n + 1
}

count""",
    }

    # Zone de code
    code = st.text_area(
        "Code Catnip",
        value=examples.get(example, ""),
        height=300,
    )

    # Options
    col1, col2 = st.sidebar.checkbox("Afficher variables", value=False), st.sidebar.checkbox("Verbose", value=False)
    show_vars = col1
    verbose = col2

    # Bouton d'exécution
    if st.button("▶ Exécuter", type="primary") or code:
        if code.strip():
            try:
                # Exécuter le code
                catnip = Catnip()
                catnip.parse(code)
                result = catnip.execute()

                # Afficher le résultat
                st.success("✓ Exécution réussie")
                st.subheader("Résultat")
                st.code(str(result), language="python")

                # Variables (optionnel)
                if show_vars:
                    st.subheader("Variables")
                    variables = {
                        k: v for k, v in catnip.context.globals.items() if not k.startswith('_') and not callable(v)
                    }
                    if variables:
                        for name, value in variables.items():
                            st.write(f"- `{name}` = {value}")
                    else:
                        st.write("Aucune variable définie")

                # Mode verbose
                if verbose:
                    st.subheader("Détails")
                    st.write(f"Nombre d'instructions : {len(code.split(chr(10)))}")

            except SyntaxError as e:
                st.error(f"❌ Erreur de syntaxe : {e}")
            except Exception as e:
                st.error(f"❌ Erreur d'exécution : {e}")
        else:
            st.write("Écrivez du code Catnip ci-dessus et cliquez sur Exécuter")


def catnip_calculator():
    """
    Calculatrice interactive Catnip dans Streamlit.
    """
    st.title("🧮 Calculatrice Catnip")
    st.markdown("Calculatrice basée sur Catnip pour calculs complexes")

    # Inputs
    st.subheader("Entrées")
    col1, col2, col3 = (
        st.sidebar.selectbox("a", list(range(0, 101)), index=10),
        st.sidebar.selectbox("b", list(range(0, 101)), index=20),
        st.sidebar.selectbox("c", list(range(0, 101)), index=30),
    )
    a, b, c = col1, col2, col3

    st.write(f"a = {a}, b = {b}, c = {c}")

    # Formules pré-définies
    st.subheader("Formules")
    formula = st.selectbox(
        "Choisir une formule",
        [
            "Somme (a + b + c)",
            "Moyenne (a + b + c) / 3",
            "Produit (a * b * c)",
            "Formule quadratique (a*x² + b*x + c)",
        ],
    )

    # Mapping formules → code Catnip
    formulas_code = {
        "Somme (a + b + c)": "result = a + b + c; result",
        "Moyenne (a + b + c) / 3": "result = (a + b + c) / 3; result",
        "Produit (a * b * c)": "result = a * b * c; result",
        "Formule quadratique (a*x² + b*x + c)": """# Pour x=2
x = 2
result = a * x * x + b * x + c
result""",
    }

    code = formulas_code.get(formula, "")

    if code:
        try:
            # Préparer contexte avec variables
            catnip = Catnip()
            catnip.context.globals.update({'a': a, 'b': b, 'c': c})

            # Exécuter
            catnip.parse(code)
            result = catnip.execute()

            # Afficher
            st.subheader("Résultat")
            st.code(str(result), language="python")

        except Exception as e:
            st.error(f"Erreur : {e}")


def main():
    """
    Application Streamlit principale.
    """
    if not STREAMLIT_AVAILABLE:
        print("=" * 60)
        print("DÉMO : Streamlit Catnip App")
        print("=" * 60)
        print()
        print("Cette démonstration montre l'interface Streamlit pour Catnip.")
        print("Pour lancer l'application réelle :")
        print()
        print("  1. pip install streamlit")
        print("  2. streamlit run streamlit_app.py")
        print()
        print("=" * 60)
        print()

    # Sélection du mode
    st.sidebar.title("Navigation")
    page = st.sidebar.selectbox(
        "Mode",
        ["Playground", "Calculatrice"],
    )

    if page == "Playground":
        catnip_playground()
    elif page == "Calculatrice":
        catnip_calculator()

    # Footer
    st.sidebar.markdown("---")
    st.sidebar.markdown("Built with Catnip 🐱")


if __name__ == '__main__':
    if STREAMLIT_AVAILABLE:
        main()
    else:
        # Démonstration sans Streamlit
        print("=" * 60)
        print("Application Streamlit avec Catnip")
        print("=" * 60)
        print()
        print("Cette application démontre l'intégration de Catnip dans Streamlit.")
        print()
        print("Fonctionnalités :")
        print("  - Playground interactif pour écrire et exécuter du code Catnip")
        print("  - Exemples pré-définis (Fibonacci, calculs, listes, boucles)")
        print("  - Calculatrice avec formules personnalisées")
        print("  - Visualisation des variables et résultats")
        print()
        print("Pour lancer l'application :")
        print("  1. pip install streamlit")
        print("  2. streamlit run streamlit_app.py")
        print()
        print("=" * 60)
        print()
        print("Démonstration du backend Catnip :")
        print()

        # Démonstration backend
        from catnip import Catnip

        # Exemple Fibonacci
        print("Fibonacci(10)")
        code = """
fib = (n) => {
    if n <= 1 { n }
    else { fib(n-1) + fib(n-2) }
}
fib(10)
"""
        catnip = Catnip()
        catnip.parse(code)
        result = catnip.execute()
        print(f"Résultat : {result}")
        print()

        # Exemple calculatrice
        print("Calculatrice (a=10, b=20, c=30)")
        code = "result = (a + b + c) / 3; result"
        catnip = Catnip()
        catnip.context.globals.update({'a': 10, 'b': 20, 'c': 30})
        catnip.parse(code)
        result = catnip.execute()
        print(f"Moyenne : {result}")
        print()

        print("=" * 60)
