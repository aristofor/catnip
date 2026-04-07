#!/usr/bin/env python3
"""
Exemple d'intégration de Catnip pour génération de rapports avec templates.

Montre comment :
1. Créer un système de templates avec données dynamiques
2. Calculer des métriques et agrégations
3. Formater les résultats (texte, markdown, HTML)
4. Composer des sections de rapport

Use case : Génération de rapports personnalisés avec logique métier en Catnip.
"""

from catnip import Catnip, Context, pass_context


class ReportContext(Context):
    """
    Contexte de génération de rapport.

    Stocke les données source, métriques calculées, et sections générées.
    """

    def __init__(self, data: dict, **kwargs):
        super().__init__(**kwargs)
        self._data = data
        self._metrics = {}
        self._sections = []

        # Expose les données dans le contexte
        self.globals['data'] = data
        self.globals['metrics'] = self._metrics

    @property
    def data(self) -> dict:
        return self._data

    @property
    def metrics(self) -> dict:
        return self._metrics

    @property
    def sections(self) -> list:
        return self._sections

    def set_metric(self, name: str, value):
        """Enregistre une métrique calculée."""
        self._metrics[name] = value
        self.globals['metrics'][name] = value

    def add_section(self, title: str, content: str):
        """Ajoute une section au rapport."""
        self._sections.append({'title': title, 'content': content})


class ReportBuilder(Catnip):
    """
    DSL Catnip pour génération de rapports.

    Syntaxe déclarative pour calculer des métriques et composer des rapports.
    """

    @staticmethod
    def _calculate_sum(ctx, field: str, list_name: str = 'items'):
        """Calcule la somme d'un champ."""
        items = ctx.data.get(list_name, [])
        total = sum(item.get(field, 0) for item in items)
        return total

    @staticmethod
    def _calculate_avg(ctx, field: str, list_name: str = 'items'):
        """Calcule la moyenne d'un champ."""
        items = ctx.data.get(list_name, [])
        if not items:
            return 0
        total = sum(item.get(field, 0) for item in items)
        return total / len(items)

    @staticmethod
    def _calculate_count(ctx, list_name: str = 'items'):
        """Compte le nombre d'éléments."""
        items = ctx.data.get(list_name, [])
        return len(items)

    @staticmethod
    def _calculate_max(ctx, field: str, list_name: str = 'items'):
        """Trouve la valeur maximale d'un champ."""
        items = ctx.data.get(list_name, [])
        if not items:
            return None
        return max(item.get(field, 0) for item in items)

    @staticmethod
    def _calculate_min(ctx, field: str, list_name: str = 'items'):
        """Trouve la valeur minimale d'un champ."""
        items = ctx.data.get(list_name, [])
        if not items:
            return None
        return min(item.get(field, 0) for item in items)

    @staticmethod
    def _filter_items(ctx, field: str, operator: str, value, list_name: str = 'items'):
        """Filtre et compte les éléments selon une condition."""
        items = ctx.data.get(list_name, [])

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

        filtered = [item for item in items if op_map[operator](item.get(field), value)]
        return len(filtered)

    @staticmethod
    def _set_metric(ctx, name: str, value):
        """Définit une métrique."""
        ctx.set_metric(name, value)
        return value

    @staticmethod
    def _format_number(ctx, number, decimals: int = 2):
        """Formate un nombre avec nombre de décimales."""
        if isinstance(number, (int, float)):
            return round(number, decimals)
        return number

    @staticmethod
    def _format_currency(ctx, amount, symbol: str = '$'):
        """Formate un montant en devise."""
        return f"{symbol}{amount:,.2f}"

    @staticmethod
    def _add_section(ctx, title: str, content: str):
        """Ajoute une section au rapport."""
        ctx.add_section(title, content)
        return True

    # Fonctions DSL injectées
    REPORT_FUNCTIONS = dict(
        calculate_sum=pass_context(_calculate_sum),
        calculate_avg=pass_context(_calculate_avg),
        calculate_count=pass_context(_calculate_count),
        calculate_max=pass_context(_calculate_max),
        calculate_min=pass_context(_calculate_min),
        filter_items=pass_context(_filter_items),
        set_metric=pass_context(_set_metric),
        format_number=pass_context(_format_number),
        format_currency=pass_context(_format_currency),
        add_section=pass_context(_add_section),
    )

    def __init__(self, data: dict, **kwargs):
        context = ReportContext(data)
        super().__init__(context=context, **kwargs)
        self.context.globals.update(self.REPORT_FUNCTIONS)

    def generate(self, report_script: str) -> dict:
        """
        Génère le rapport.

        Returns:
            dict - Rapport avec métriques et sections
        """
        self.parse(report_script)
        self.execute()

        return {
            'metrics': self.context.metrics,
            'sections': self.context.sections,
            'data': self.context.data,
        }

    def to_text(self) -> str:
        """Exporte le rapport en texte brut."""
        output = []

        # Métriques
        if self.context.metrics:
            output.append("⇒ MÉTRIQUES")
            output.append("")
            for name, value in self.context.metrics.items():
                output.append(f"{name}: {value}")
            output.append("")

        # Sections
        for section in self.context.sections:
            output.append(f"⇒ {section['title']}")
            output.append("")
            output.append(section['content'])
            output.append("")

        return '\n'.join(output)

    def to_markdown(self) -> str:
        """Exporte le rapport en Markdown."""
        output = []

        # Métriques
        if self.context.metrics:
            output.append("## Métriques")
            output.append("")
            for name, value in self.context.metrics.items():
                output.append(f"- **{name}**: {value}")
            output.append("")

        # Sections
        for section in self.context.sections:
            output.append(f"## {section['title']}")
            output.append("")
            output.append(section['content'])
            output.append("")

        return '\n'.join(output)


# --- Démonstration ---

if __name__ == '__main__':
    print("⇒ Exemple 1 : Rapport de ventes mensuel")
    print()

    # Données de ventes
    sales_data = {
        'month': 'Janvier 2026',
        'items': [
            {'product': 'Laptop', 'quantity': 5, 'price': 1000, 'category': 'Electronics'},
            {'product': 'Mouse', 'quantity': 50, 'price': 20, 'category': 'Accessories'},
            {'product': 'Keyboard', 'quantity': 30, 'price': 80, 'category': 'Accessories'},
            {'product': 'Monitor', 'quantity': 10, 'price': 300, 'category': 'Electronics'},
            {'product': 'Cable', 'quantity': 100, 'price': 5, 'category': 'Accessories'},
        ],
    }

    report_script = """
    # Calculer les métriques principales
    total_items = calculate_count('items')
    set_metric('total_products', total_items)

    total_revenue = calculate_sum('price', 'items')
    set_metric('revenue', format_currency(total_revenue, '$'))

    avg_price = calculate_avg('price', 'items')
    set_metric('avg_price', format_currency(avg_price, '$'))

    max_price = calculate_max('price', 'items')
    set_metric('highest_price', format_currency(max_price, '$'))

    # Compter par catégorie
    electronics_count = filter_items('category', '==', 'Electronics', 'items')
    set_metric('electronics', electronics_count)

    accessories_count = filter_items('category', '==', 'Accessories', 'items')
    set_metric('accessories', accessories_count)
    """

    builder = ReportBuilder(sales_data)
    report = builder.generate(report_script)

    print(f"Rapport pour : {sales_data['month']}")
    print()
    print("Métriques calculées :")
    for name, value in report['metrics'].items():
        print(f"  - {name}: {value}")

    print()
    print("⇒ Exemple 2 : Rapport avec sections formatées")
    print()

    data = {
        'title': 'Rapport Q1 2026',
        'sales': [
            {'month': 'Jan', 'revenue': 50000, 'costs': 30000},
            {'month': 'Fev', 'revenue': 55000, 'costs': 32000},
            {'month': 'Mar', 'revenue': 60000, 'costs': 35000},
        ],
    }

    report_script = """
    # Calculer métriques globales
    total_revenue = calculate_sum('revenue', 'sales')
    total_costs = calculate_sum('costs', 'sales')
    profit = total_revenue - total_costs

    set_metric('revenue_total', format_currency(total_revenue, '$'))
    set_metric('costs_total', format_currency(total_costs, '$'))
    set_metric('profit', format_currency(profit, '$'))

    # Calculer marge
    margin_pct = profit * 100 / total_revenue
    set_metric('margin', format_number(margin_pct, 2))
    """

    builder = ReportBuilder(data)
    report = builder.generate(report_script)

    print(builder.to_markdown())

    print()
    print("⇒ Exemple 3 : Rapport complexe avec analyse")
    print()

    inventory_data = {
        'warehouse': 'Entrepôt A',
        'products': [
            {'name': 'Widget A', 'stock': 100, 'min_stock': 50, 'value': 10},
            {'name': 'Widget B', 'stock': 25, 'min_stock': 50, 'value': 15},
            {'name': 'Widget C', 'stock': 200, 'min_stock': 100, 'value': 8},
            {'name': 'Widget D', 'stock': 10, 'min_stock': 50, 'value': 20},
        ],
    }

    report_script = """
    # Métriques globales
    total_products = calculate_count('products')
    set_metric('total_products', total_products)

    total_value = calculate_sum('value', 'products')
    set_metric('inventory_value', format_currency(total_value, '$'))

    # Analyse des stocks faibles
    low_stock = filter_items('stock', '<', 50, 'products')
    set_metric('low_stock_count', low_stock)

    # Stock moyen
    avg_stock = calculate_avg('stock', 'products')
    set_metric('avg_stock', format_number(avg_stock, 0))
    """

    builder = ReportBuilder(inventory_data)
    report = builder.generate(report_script)

    print(f"Entrepôt : {inventory_data['warehouse']}")
    print()
    print("Métriques :")
    for name, value in report['metrics'].items():
        print(f"  {name}: {value}")

    print()
    print("Analyse :")
    low_stock_count = report['metrics'].get('low_stock_count', 0)
    if low_stock_count > 0:
        print(f"  ⚠ {low_stock_count} produit(s) sous le seuil minimum")
        print("  → Réapprovisionnement nécessaire")
    else:
        print("  ✓ Tous les stocks sont au niveau adéquat")
