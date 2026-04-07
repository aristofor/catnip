#!/usr/bin/env python3
"""
Exemple d'intégration de Catnip comme DSL pour valider des configurations.

Montre comment :
1. Créer un DSL déclaratif pour validation de config
2. Injecter des règles de validation dans le contexte
3. Capturer les erreurs de validation avec messages clairs
4. Utiliser Catnip pour un mini-langage de règles métier

Use case : Valider des fichiers de config utilisateur avec règles complexes.
"""

from catnip import Catnip, Context, pass_context


class ConfigContext(Context):
    """
    Contexte de validation de configuration.

    Stocke la config à valider et les erreurs rencontrées.
    """

    def __init__(self, config: dict, **kwargs):
        super().__init__(**kwargs)
        self._config = config
        self._errors = []
        # Expose la config dans le contexte
        self.globals['config'] = config

    @property
    def config(self) -> dict:
        return self._config

    @property
    def errors(self) -> list:
        return self._errors

    def add_error(self, field: str, message: str):
        self._errors.append({'field': field, 'message': message})

    def is_valid(self) -> bool:
        return len(self._errors) == 0


class ConfigDSL(Catnip):
    """
    DSL Catnip pour validation de configuration.

    Syntaxe déclarative pour définir des règles de validation.
    """

    @staticmethod
    def _required(ctx, field):
        """Vérifie qu'un champ est présent."""
        if field not in ctx.config:
            ctx.add_error(field, f"Champ requis '{field}' manquant")
            return False
        if ctx.config[field] is None or ctx.config[field] == '':
            ctx.add_error(field, f"Champ '{field}' ne peut pas être vide")
            return False
        return True

    @staticmethod
    def _type_check(ctx, field, expected_type):
        """Vérifie le type d'un champ."""
        if field not in ctx.config:
            return False  # Déjà géré par required()

        value = ctx.config[field]
        type_map = {
            'int': int,
            'float': float,
            'str': str,
            'bool': bool,
            'list': list,
            'dict': dict,
        }

        if expected_type not in type_map:
            raise ValueError(f"Type inconnu: {expected_type}")

        if not isinstance(value, type_map[expected_type]):
            ctx.add_error(field, f"Champ '{field}' doit être de type {expected_type}, reçu {type(value).__name__}")
            return False
        return True

    @staticmethod
    def _range_check(ctx, field, min_val=None, max_val=None):
        """Vérifie qu'une valeur est dans un intervalle."""
        if field not in ctx.config:
            return False

        value = ctx.config[field]
        if min_val is not None and value < min_val:
            ctx.add_error(field, f"Champ '{field}' doit être >= {min_val}, reçu {value}")
            return False
        if max_val is not None and value > max_val:
            ctx.add_error(field, f"Champ '{field}' doit être <= {max_val}, reçu {value}")
            return False
        return True

    @staticmethod
    def _length_check(ctx, field, min_len=None, max_len=None):
        """Vérifie la longueur d'une chaîne ou liste."""
        if field not in ctx.config:
            return False

        value = ctx.config[field]
        length = len(value)

        if min_len is not None and length < min_len:
            ctx.add_error(field, f"Champ '{field}' doit avoir >= {min_len} caractères, reçu {length}")
            return False
        if max_len is not None and length > max_len:
            ctx.add_error(field, f"Champ '{field}' doit avoir <= {max_len} caractères, reçu {length}")
            return False
        return True

    @staticmethod
    def _pattern_check(ctx, field, pattern):
        """Vérifie qu'une chaîne match un pattern regex."""
        import re

        if field not in ctx.config:
            return False

        value = ctx.config[field]
        if not isinstance(value, str):
            return False

        if not re.match(pattern, value):
            ctx.add_error(field, f"Champ '{field}' ne match pas le pattern {pattern}")
            return False
        return True

    @staticmethod
    def _one_of(ctx, field, *allowed_values):
        """Vérifie qu'une valeur est dans une liste autorisée."""
        if field not in ctx.config:
            return False

        value = ctx.config[field]
        if value not in allowed_values:
            ctx.add_error(field, f"Champ '{field}' doit être dans {allowed_values}, reçu {value}")
            return False
        return True

    # Fonctions DSL injectées
    DSL_FUNCTIONS = dict(
        required=pass_context(_required),
        type_check=pass_context(_type_check),
        range_check=pass_context(_range_check),
        length_check=pass_context(_length_check),
        pattern_check=pass_context(_pattern_check),
        one_of=pass_context(_one_of),
    )

    def __init__(self, config: dict, **kwargs):
        context = ConfigContext(config)
        super().__init__(context=context, **kwargs)
        self.context.globals.update(self.DSL_FUNCTIONS)

    def validate(self, rules_script: str) -> tuple[bool, list]:
        """
        Exécute les règles de validation.

        Returns:
            (is_valid, errors) - tuple avec status et liste d'erreurs
        """
        self.parse(rules_script)
        self.execute()
        return (self.context.is_valid(), self.context.errors)


# --- Démonstration ---

if __name__ == '__main__':
    print("⇒ Exemple 1 : Configuration valide")
    print()

    config_valid = {
        'username': 'alice',
        'email': 'alice@example.com',
        'age': 28,
        'role': 'admin',
        'api_key': 'sk_test_1234567890',
    }

    rules = """
    required('username')
    required('email')
    required('age')

    type_check('username', 'str')
    type_check('age', 'int')

    length_check('username', 3, 20)
    range_check('age', 18, 120)

    one_of('role', 'user', 'admin', 'guest')
    """

    dsl = ConfigDSL(config_valid)
    is_valid, errors = dsl.validate(rules)

    if is_valid:
        print("✅ Configuration valide !")
    else:
        print(f"❌ {len(errors)} erreur(s) détectée(s) :")
        for err in errors:
            print(f"  - {err['field']}: {err['message']}")

    print()
    print("⇒ Exemple 2 : Configuration invalide")
    print()

    config_invalid = {
        'username': 'ab',  # Trop court
        'email': 'invalid-email',  # Pattern incorrect
        'age': 15,  # Hors intervalle
        'role': 'superadmin',  # Pas dans la liste
        # api_key manquant
    }

    dsl = ConfigDSL(config_invalid)
    is_valid, errors = dsl.validate(rules)

    if is_valid:
        print("✅ Configuration valide !")
    else:
        print(f"❌ {len(errors)} erreur(s) détectée(s) :")
        for err in errors:
            print(f"  - {err['field']}: {err['message']}")

    print()
    print("⇒ Exemple 3 : Validation partielle (sans required)")
    print()

    rules_partial = """
    type_check('age', 'int')
    range_check('age', 0, 150)
    """

    config_partial = {'age': 42}
    dsl = ConfigDSL(config_partial)
    is_valid, errors = dsl.validate(rules_partial)

    print(f"Config partielle : {config_partial}")
    print(f"Résultat : {'✅ Valide' if is_valid else '❌ Invalide'}")
