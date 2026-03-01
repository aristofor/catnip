"""
Exemple d'intégration de Catnip comme moteur de règles métier.

Montre comment :
1. Définir des règles métier en Catnip (pricing, éligibilité, validation)
2. Évaluer des règles sur des données en entrée
3. Calculer des résultats conditionnels
4. Composer des règles complexes

Use case : Moteur de règles pour pricing dynamique, éligibilité client, calcul de remises.
"""

from catnip import Catnip, Context, pass_context


class RuleContext(Context):
    """
    Contexte d'évaluation de règles métier.

    Stocke les données d'entrée et les résultats de l'évaluation.
    """

    def __init__(self, input_data: dict, **kwargs):
        super().__init__(**kwargs)
        self._input = input_data
        self._results = {}
        self._applied_rules = []

        # Expose les données d'entrée
        self.globals['input'] = input_data
        self.globals['results'] = self._results

    @property
    def input_data(self) -> dict:
        return self._input

    @property
    def results(self) -> dict:
        return self._results

    @property
    def applied_rules(self) -> list:
        return self._applied_rules

    def set_result(self, key: str, value):
        """Enregistre un résultat de règle."""
        self._results[key] = value
        self.globals['results'][key] = value

    def mark_rule_applied(self, rule_name: str, details: str = ''):
        """Marque une règle comme appliquée."""
        self._applied_rules.append({'rule': rule_name, 'details': details})


class RuleEngine(Catnip):
    """
    Moteur de règles Catnip.

    Évalue des règles métier sur des données d'entrée.
    """

    @staticmethod
    def _set_result(ctx, key: str, value):
        """Définit un résultat."""
        ctx.set_result(key, value)
        return value

    @staticmethod
    def _mark_applied(ctx, rule_name: str, details: str = ''):
        """Marque une règle comme appliquée."""
        ctx.mark_rule_applied(rule_name, details)
        return True

    @staticmethod
    def _get_input(ctx, key: str, default=None):
        """Récupère une valeur d'entrée."""
        return ctx.input_data.get(key, default)

    @staticmethod
    def _has_field(ctx, key: str):
        """Vérifie si un champ existe dans l'entrée."""
        return key in ctx.input_data

    @staticmethod
    def _calculate_percentage(ctx, base, percent):
        """Calcule un pourcentage."""
        return base * percent / 100

    @staticmethod
    def _apply_discount(ctx, price, discount_percent):
        """Applique une remise en pourcentage."""
        return price * (1 - discount_percent / 100)

    @staticmethod
    def _clamp(ctx, value, min_val, max_val):
        """Limite une valeur entre min et max."""
        return max(min_val, min(max_val, value))

    # Fonctions DSL injectées
    RULE_FUNCTIONS = dict(
        set_result=pass_context(_set_result),
        mark_applied=pass_context(_mark_applied),
        get_input=pass_context(_get_input),
        has_field=pass_context(_has_field),
        calculate_percentage=pass_context(_calculate_percentage),
        apply_discount=pass_context(_apply_discount),
        clamp=pass_context(_clamp),
    )

    def __init__(self, input_data: dict, **kwargs):
        context = RuleContext(input_data)
        super().__init__(context=context, **kwargs)
        self.context.globals.update(self.RULE_FUNCTIONS)

    def evaluate(self, rules_script: str) -> dict:
        """
        Évalue les règles sur les données d'entrée.

        Returns:
            dict - Résultats avec règles appliquées
        """
        self.parse(rules_script)
        self.execute()

        return {
            'input': self.context.input_data,
            'results': self.context.results,
            'applied_rules': self.context.applied_rules,
        }


# --- Démonstration ---

if __name__ == '__main__':
    print("⇒ Exemple 1 : Pricing dynamique avec remises")
    print()

    # Données client et commande
    order_data = {
        'customer_type': 'premium',
        'order_amount': 1000,
        'items_count': 5,
        'first_order': False,
    }

    # Règles de pricing (définies par business)
    pricing_rules = """
    # Prix de base
    base_price = input['order_amount']
    set_result('base_price', base_price)

    # Remise premium (10%)
    if input['customer_type'] == 'premium' {
        discount = calculate_percentage(base_price, 10)
        set_result('premium_discount', discount)
        base_price = base_price - discount
        mark_applied('premium_discount', '10% pour client premium')
    }

    # Remise volume (5% si > 3 items)
    if input['items_count'] > 3 {
        volume_discount = calculate_percentage(base_price, 5)
        set_result('volume_discount', volume_discount)
        base_price = base_price - volume_discount
        mark_applied('volume_discount', '5% pour commande volumineuse')
    }

    # Prix final
    set_result('final_price', base_price)
    """

    engine = RuleEngine(order_data)
    result = engine.evaluate(pricing_rules)

    print("Données d'entrée :")
    print(f"  Type client : {result['input']['customer_type']}")
    print(f"  Montant : ${result['input']['order_amount']}")
    print(f"  Articles : {result['input']['items_count']}")
    print()
    print("Résultats :")
    print(f"  Prix de base : ${result['results']['base_price']}")
    if 'premium_discount' in result['results']:
        print(f"  Remise premium : -${result['results']['premium_discount']}")
    if 'volume_discount' in result['results']:
        print(f"  Remise volume : -${result['results']['volume_discount']}")
    print(f"  Prix final : ${result['results']['final_price']}")
    print()
    print("Règles appliquées :")
    for rule in result['applied_rules']:
        print(f"  - {rule['rule']}: {rule['details']}")

    print()
    print("⇒ Exemple 2 : Éligibilité crédit")
    print()

    applicant_data = {
        'age': 28,
        'income': 45000,
        'credit_score': 720,
        'employment_years': 3,
        'existing_loans': 1,
    }

    eligibility_rules = """
    # Critères d'éligibilité
    eligible = True

    # Âge minimum
    if input['age'] < 21 {
        eligible = False
        set_result('rejection_reason', 'Âge minimum 21 ans requis')
    }

    # Revenu minimum
    if input['income'] < 30000 {
        eligible = False
        set_result('rejection_reason', 'Revenu minimum 30000 requis')
    }

    # Score de crédit
    if input['credit_score'] < 650 {
        eligible = False
        set_result('rejection_reason', 'Score de crédit insuffisant')
    }

    # Emploi stable (2+ ans)
    if input['employment_years'] < 2 {
        eligible = False
        set_result('rejection_reason', 'Emploi stable 2+ ans requis')
    }

    set_result('eligible', eligible)

    # Calculer limite de crédit si éligible
    if eligible {
        # Formule : revenu * 0.3, limité entre 5000 et 50000
        base_limit = input['income'] * 0.3
        credit_limit = clamp(base_limit, 5000, 50000)

        # Bonus pour bon score
        if input['credit_score'] > 750 {
            credit_limit = credit_limit * 1.2
            mark_applied('high_credit_score', 'Bonus 20% pour score > 750')
        }

        # Pénalité si prêts existants
        if input['existing_loans'] > 0 {
            penalty = input['existing_loans'] * 1000
            credit_limit = credit_limit - penalty
            mark_applied('existing_loans_penalty', 'Pénalité 1000 par prêt existant')
        }

        set_result('credit_limit', credit_limit)
    }
    """

    engine = RuleEngine(applicant_data)
    result = engine.evaluate(eligibility_rules)

    print("Demandeur :")
    print(f"  Âge : {result['input']['age']} ans")
    print(f"  Revenu : ${result['input']['income']}")
    print(f"  Score crédit : {result['input']['credit_score']}")
    print(f"  Années d'emploi : {result['input']['employment_years']}")
    print(f"  Prêts existants : {result['input']['existing_loans']}")
    print()

    if result['results']['eligible']:
        print("✓ Éligible au crédit")
        print(f"  Limite accordée : ${result['results']['credit_limit']:.2f}")
        if result['applied_rules']:
            print()
            print("  Ajustements :")
            for rule in result['applied_rules']:
                print(f"    - {rule['rule']}: {rule['details']}")
    else:
        print("✗ Non éligible")
        print(f"  Raison : {result['results']['rejection_reason']}")

    print()
    print("⇒ Exemple 3 : Calcul de frais de livraison")
    print()

    shipping_data = {
        'destination': 'international',
        'weight_kg': 5.5,
        'express': True,
        'value': 200,
    }

    shipping_rules = """
    # Frais de base selon destination
    base_fee = 0
    if input['destination'] == 'local' {
        base_fee = 5
        mark_applied('local_shipping', 'Frais de base local')
    }
    if input['destination'] == 'national' {
        base_fee = 15
        mark_applied('national_shipping', 'Frais de base national')
    }
    if input['destination'] == 'international' {
        base_fee = 50
        mark_applied('international_shipping', 'Frais de base international')
    }

    set_result('base_shipping', base_fee)

    # Frais selon poids (2$/kg)
    weight_fee = input['weight_kg'] * 2
    set_result('weight_fee', weight_fee)
    mark_applied('weight_fee', '2$/kg')

    # Supplément express (50%)
    total = base_fee + weight_fee
    if input['express'] {
        express_fee = calculate_percentage(total, 50)
        set_result('express_fee', express_fee)
        total = total + express_fee
        mark_applied('express_shipping', '+50% pour livraison express')
    }

    # Assurance (1% de la valeur si > 100$)
    if input['value'] > 100 {
        insurance = calculate_percentage(input['value'], 1)
        set_result('insurance', insurance)
        total = total + insurance
        mark_applied('insurance', '1% assurance pour valeur > 100$')
    }

    set_result('total_shipping', total)
    """

    engine = RuleEngine(shipping_data)
    result = engine.evaluate(shipping_rules)

    print("Commande :")
    print(f"  Destination : {result['input']['destination']}")
    print(f"  Poids : {result['input']['weight_kg']} kg")
    print(f"  Express : {'Oui' if result['input']['express'] else 'Non'}")
    print(f"  Valeur : ${result['input']['value']}")
    print()
    print("Frais de livraison :")
    print(f"  Base : ${result['results']['base_shipping']}")
    print(f"  Poids : ${result['results']['weight_fee']}")
    if 'express_fee' in result['results']:
        print(f"  Express : ${result['results']['express_fee']}")
    if 'insurance' in result['results']:
        print(f"  Assurance : ${result['results']['insurance']}")
    print(f"  Total : ${result['results']['total_shipping']:.2f}")
    print()
    print("Règles appliquées :")
    for rule in result['applied_rules']:
        print(f"  - {rule['rule']}: {rule['details']}")
