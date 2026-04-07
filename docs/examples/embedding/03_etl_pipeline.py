#!/usr/bin/env python3
"""
Exemple d'intégration de Catnip comme DSL pour pipelines ETL.

Montre comment :
1. Créer un DSL déclaratif pour transformation de données
2. Charger des données depuis CSV/dict
3. Appliquer des transformations via règles Catnip
4. Exporter vers JSON/CSV/dict

Use case : Pipelines de transformation de données avec logique métier en Catnip.
"""

import json
from io import StringIO
from catnip import Catnip, Context, pass_context


class ETLContext(Context):
    """
    Contexte de transformation ETL.

    Stocke les données en cours de transformation et l'historique des opérations.
    """

    def __init__(self, data: list[dict], **kwargs):
        super().__init__(**kwargs)
        self._data = data
        self._operations = []
        # Expose les données dans le contexte
        self.globals['data'] = data
        self.globals['len'] = len

    @property
    def data(self) -> list[dict]:
        return self._data

    @property
    def operations(self) -> list:
        return self._operations

    def update_data(self, new_data: list[dict]):
        """Mise à jour des données après transformation."""
        self._data = new_data
        self.globals['data'] = new_data

    def log_operation(self, op_name: str, details: str = ''):
        """Enregistre une opération appliquée."""
        self._operations.append({'operation': op_name, 'details': details})


class ETLDSL(Catnip):
    """
    DSL Catnip pour pipelines ETL.

    Syntaxe déclarative pour filtrer, transformer, agréger des données.
    """

    @staticmethod
    def _filter_rows(ctx, condition_field: str, operator: str, value):
        """Filtre les lignes selon une condition."""
        filtered = []
        op_map = {
            '==': lambda a, b: a == b,
            '!=': lambda a, b: a != b,
            '>': lambda a, b: a > b,
            '>=': lambda a, b: a >= b,
            '<': lambda a, b: a < b,
            '<=': lambda a, b: a <= b,
        }

        if operator not in op_map:
            raise ValueError(f"Opérateur inconnu: {operator}")

        for row in ctx.data:
            if condition_field in row:
                if op_map[operator](row[condition_field], value):
                    filtered.append(row)

        ctx.update_data(filtered)
        ctx.log_operation('filter', f"{condition_field} {operator} {value}")
        return len(filtered)

    @staticmethod
    def _map_field(ctx, field: str, operation: str, value=None):
        """Applique une transformation à un champ."""
        op_map = {
            'multiply': lambda x: x * value,
            'add': lambda x: x + value,
            'upper': lambda x: x.upper() if isinstance(x, str) else x,
            'lower': lambda x: x.lower() if isinstance(x, str) else x,
        }

        if operation not in op_map:
            raise ValueError(f"Opération inconnue: {operation}")

        for row in ctx.data:
            if field in row:
                row[field] = op_map[operation](row[field])

        ctx.log_operation('map', f"Transform field '{field}' ({operation})")
        return len(ctx.data)

    @staticmethod
    def _rename_field(ctx, old_name: str, new_name: str):
        """Renomme un champ."""
        for row in ctx.data:
            if old_name in row:
                row[new_name] = row.pop(old_name)

        ctx.log_operation('rename', f"{old_name} → {new_name}")
        return len(ctx.data)

    @staticmethod
    def _add_field(ctx, field: str, formula: str, field1: str, field2: str = None):
        """Ajoute un nouveau champ calculé."""
        formulas = {
            'multiply': lambda r: r.get(field1, 0) * r.get(field2, 1),
            'add': lambda r: r.get(field1, 0) + r.get(field2, 0),
            'concat': lambda r: str(r.get(field1, '')) + str(r.get(field2, '')),
        }

        if formula not in formulas:
            raise ValueError(f"Formule inconnue: {formula}")

        for row in ctx.data:
            row[field] = formulas[formula](row)

        ctx.log_operation('add_field', f"Added '{field}' = {formula}({field1}, {field2})")
        return len(ctx.data)

    @staticmethod
    def _drop_field(ctx, field: str):
        """Supprime un champ."""
        for row in ctx.data:
            row.pop(field, None)

        ctx.log_operation('drop_field', f"Dropped '{field}'")
        return len(ctx.data)

    @staticmethod
    def _sort_by(ctx, field: str, reverse: bool = False):
        """Trie les données par un champ."""
        ctx._data = sorted(ctx.data, key=lambda r: r.get(field, ''), reverse=reverse)
        ctx.globals['data'] = ctx._data

        ctx.log_operation('sort', f"By '{field}' {'desc' if reverse else 'asc'}")
        return len(ctx.data)

    @staticmethod
    def _limit(ctx, n: int):
        """Limite le nombre de lignes."""
        ctx.update_data(ctx.data[:n])
        ctx.log_operation('limit', f"First {n} rows")
        return len(ctx.data)

    @staticmethod
    def _group_by(ctx, field: str, agg_field: str, agg_func: str):
        """Agrège les données par un champ."""
        groups = {}
        for row in ctx.data:
            key = row.get(field)
            if key not in groups:
                groups[key] = []
            if agg_field in row:
                groups[key].append(row[agg_field])

        agg_map = {
            'sum': sum,
            'count': len,
            'avg': lambda vals: sum(vals) / len(vals) if vals else 0,
            'min': lambda vals: min(vals) if vals else None,
            'max': lambda vals: max(vals) if vals else None,
        }

        if agg_func not in agg_map:
            raise ValueError(f"Fonction d'agrégation inconnue: {agg_func}")

        result = [{field: key, f"{agg_func}_{agg_field}": agg_map[agg_func](vals)} for key, vals in groups.items()]

        ctx.update_data(result)
        ctx.log_operation('group_by', f"{field}, {agg_func}({agg_field})")
        return len(result)

    # Fonctions DSL injectées
    DSL_FUNCTIONS = dict(
        filter_rows=pass_context(_filter_rows),
        map_field=pass_context(_map_field),
        rename_field=pass_context(_rename_field),
        add_field=pass_context(_add_field),
        drop_field=pass_context(_drop_field),
        sort_by=pass_context(_sort_by),
        limit=pass_context(_limit),
        group_by=pass_context(_group_by),
    )

    def __init__(self, data: list[dict], **kwargs):
        context = ETLContext(data)
        super().__init__(context=context, **kwargs)
        self.context.globals.update(self.DSL_FUNCTIONS)

    def transform(self, pipeline_script: str) -> list[dict]:
        """
        Exécute le pipeline de transformation.

        Returns:
            list[dict] - Données transformées
        """
        self.parse(pipeline_script)
        self.execute()
        return self.context.data

    def to_json(self, indent: int = 2) -> str:
        """Exporte les données en JSON."""
        return json.dumps(self.context.data, indent=indent, ensure_ascii=False)

    def to_csv(self) -> str:
        """Exporte les données en CSV."""
        if not self.context.data:
            return ''

        output = StringIO()
        fields = list(self.context.data[0].keys())
        output.write(','.join(fields) + '\n')

        for row in self.context.data:
            values = [str(row.get(f, '')) for f in fields]
            output.write(','.join(values) + '\n')

        return output.getvalue()


# --- Démonstration ---

if __name__ == '__main__':
    print("⇒ Exemple 1 : Pipeline de nettoyage de données")
    print()

    # Données brutes (simulation CSV)
    raw_data = [
        {'id': 1, 'name': 'Alice', 'age': 28, 'salary': 50000, 'dept': 'Engineering'},
        {'id': 2, 'name': 'Bob', 'age': 35, 'salary': 60000, 'dept': 'Sales'},
        {'id': 3, 'name': 'Charlie', 'age': 22, 'salary': 45000, 'dept': 'Engineering'},
        {'id': 4, 'name': 'Diana', 'age': 29, 'salary': 55000, 'dept': 'Marketing'},
        {'id': 5, 'name': 'Eve', 'age': 31, 'salary': 58000, 'dept': 'Engineering'},
    ]

    pipeline = """
    # Filtrer les ingénieurs
    filter_rows('dept', '==', 'Engineering')

    # Augmenter les salaires de 10%
    map_field('salary', 'multiply', 1.1)

    # Renommer champ
    rename_field('salary', 'annual_comp')

    # Trier par compensation
    sort_by('annual_comp', True)
    """

    etl = ETLDSL(raw_data.copy())
    result = etl.transform(pipeline)

    print(f"Résultat : {len(result)} lignes après transformation")
    print()
    print(etl.to_json())

    print()
    print("Opérations appliquées :")
    for op in etl.context.operations:
        print(f"  - {op['operation']}: {op['details']}")

    print()
    print("⇒ Exemple 2 : Agrégation par département")
    print()

    data = [
        {'name': 'Alice', 'dept': 'Engineering', 'salary': 50000},
        {'name': 'Bob', 'dept': 'Sales', 'salary': 60000},
        {'name': 'Charlie', 'dept': 'Engineering', 'salary': 45000},
        {'name': 'Diana', 'dept': 'Sales', 'salary': 55000},
    ]

    pipeline = """
    group_by('dept', 'salary', 'avg')
    sort_by('avg_salary', True)
    """

    etl = ETLDSL(data)
    result = etl.transform(pipeline)

    print("Salaire moyen par département :")
    print(etl.to_json())

    print()
    print("⇒ Exemple 3 : Pipeline complet avec ajout de champs")
    print()

    data = [
        {'product': 'Laptop', 'price': 1000, 'quantity': 5},
        {'product': 'Mouse', 'price': 20, 'quantity': 50},
        {'product': 'Keyboard', 'price': 80, 'quantity': 30},
    ]

    pipeline = """
    # Ajouter champ total (price * quantity)
    add_field('total', 'multiply', 'price', 'quantity')

    # Filtrer les totaux > 1000
    filter_rows('total', '>', 1000)

    # Supprimer le champ quantity
    drop_field('quantity')

    # Trier par total
    sort_by('total', True)
    """

    etl = ETLDSL(data)
    result = etl.transform(pipeline)

    print(f"Produits avec total > 1000 : {len(result)}")
    print()
    print(etl.to_json())
    print()
    print("Export CSV :")
    print(etl.to_csv())
