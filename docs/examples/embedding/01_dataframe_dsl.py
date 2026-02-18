"""
Exemple d'intégration de Catnip comme DSL pour manipuler des DataFrames.

Montre comment :
1. Sous-classer Context pour ajouter un état (DataFrame)
2. Sous-classer Catnip pour injecter des fonctions DSL
3. Exécuter des scripts simples qui manipulent la DataFrame

Inspiré de l'intégration Catkin (générateur de sites statiques).
"""

import pandas as pd
from catnip import Catnip, Context, pass_context


class DataFrameContext(Context):
    """
    Contexte d'exécution enrichi avec une DataFrame.

    La DataFrame est accessible via `_` dans les scripts Catnip.
    """

    def __init__(self, df: pd.DataFrame, **kwargs):
        super().__init__(**kwargs)
        self._df = df
        # expose la DataFrame comme variable `_` dans le contexte
        self.globals['_'] = df

    @property
    def df(self) -> pd.DataFrame:
        return self._df

    def set_df(self, value: pd.DataFrame):
        self._df = value
        self.globals['_'] = value


class DataFrameDSL(Catnip):
    """
    DSL Catnip pour manipuler des DataFrames.

    Les fonctions DSL reçoivent le contexte via @pass_context
    et opèrent sur ctx.df.
    """

    # Fonctions DSL - toutes reçoivent ctx en premier argument via @pass_context

    @staticmethod
    def _sort(ctx, col=None, reverse=False):
        """Trie la DataFrame par colonne."""
        if col is None:
            col = ctx.df.columns[0]
        ctx.df.sort_values(by=col, ascending=not reverse, inplace=True)

    @staticmethod
    def _head(ctx, n=5):
        """Garde les n premières lignes."""
        ctx.set_df(ctx.df.head(n))

    @staticmethod
    def _tail(ctx, n=5):
        """Garde les n dernières lignes."""
        ctx.set_df(ctx.df.tail(n))

    @staticmethod
    def _filter(ctx, col, op, value):
        """Filtre la DataFrame selon une condition."""
        ops = {
            '==': lambda a, b: a == b,
            '!=': lambda a, b: a != b,
            '>': lambda a, b: a > b,
            '<': lambda a, b: a < b,
            '>=': lambda a, b: a >= b,
            '<=': lambda a, b: a <= b,
        }
        if op not in ops:
            raise ValueError(f"Opérateur inconnu: {op}")
        mask = ops[op](ctx.df[col], value)
        ctx.set_df(ctx.df[mask])

    @staticmethod
    def _select(ctx, *cols):
        """Sélectionne des colonnes."""
        ctx.set_df(ctx.df[list(cols)])

    @staticmethod
    def _drop(ctx, *cols):
        """Supprime des colonnes."""
        ctx.set_df(ctx.df.drop(columns=list(cols)))

    @staticmethod
    def _rename(ctx, old, new):
        """Renomme une colonne."""
        ctx.df.rename(columns={old: new}, inplace=True)

    @staticmethod
    def _groupby(ctx, col):
        """Retourne un résumé groupé (count par groupe)."""
        result = ctx.df.groupby(col).size().reset_index(name='count')
        ctx.set_df(result)

    @staticmethod
    def _show(ctx):
        """Affiche la DataFrame courante."""
        print(ctx.df.to_string(index=False))

    # Dictionnaire des fonctions DSL injectées dans le contexte Catnip
    DSL_FUNCTIONS = dict(
        sort=pass_context(_sort),
        head=pass_context(_head),
        tail=pass_context(_tail),
        filter=pass_context(_filter),
        select=pass_context(_select),
        drop=pass_context(_drop),
        rename=pass_context(_rename),
        groupby=pass_context(_groupby),
        show=pass_context(_show),
    )

    def __init__(self, df: pd.DataFrame, **kwargs):
        # crée le contexte enrichi avec la DataFrame
        context = DataFrameContext(df)
        super().__init__(context=context, **kwargs)
        # injecte les fonctions DSL
        self.context.globals.update(self.DSL_FUNCTIONS)

    def run(self, script: str) -> pd.DataFrame:
        """Exécute un script DSL et retourne la DataFrame résultante."""
        self.parse(script)
        self.execute()
        return self.context.df


# --- Démonstration ---

if __name__ == '__main__':
    # DataFrame d'exemple
    data = pd.DataFrame(
        {
            'name': ['Alice', 'Bob', 'Charlie', 'Diana', 'Eve'],
            'age': [25, 30, 35, 28, 22],
            'city': ['Paris', 'Lyon', 'Paris', 'Marseille', 'Lyon'],
            'score': [85, 92, 78, 95, 88],
        }
    )

    print("⇒ DataFrame initiale")
    print(data.to_string(index=False))
    print()

    # Script DSL : filtre, trie et affiche
    script1 = """
    filter('age', '>=', 25)
    sort('score', True)
    show()
    """

    print("⇒ Script 1: filter('age', '>=', 25) + sort('score', True)")
    dsl = DataFrameDSL(data.copy())
    result = dsl.run(script1)
    print()

    # Script DSL : groupby
    script2 = """
    groupby('city')
    sort('count', True)
    show()
    """

    print("⇒ Script 2: groupby('city') + sort('count', True)")
    dsl = DataFrameDSL(data.copy())
    result = dsl.run(script2)
    print()

    # Script DSL : pipeline avec head
    script3 = """
    sort('age')
    head(3)
    select('name', 'age')
    show()
    """

    print("⇒ Script 3: sort('age') + head(3) + select('name', 'age')")
    dsl = DataFrameDSL(data.copy())
    result = dsl.run(script3)
