"""
Exemple d'intégration de Catnip comme sandbox pour scripts utilisateur dans Flask.

Montre comment :
1. Créer un environnement sandbox sécurisé
2. Exposer APIs limitées aux scripts utilisateur
3. Exécuter du code utilisateur de manière isolée
4. Gérer les erreurs et timeouts

Use case : Permettre aux utilisateurs d'écrire des règles métier/workflows personnalisés.
"""

from catnip import Catnip, Context, pass_context


class SandboxContext(Context):
    """
    Contexte sandbox pour exécution de scripts utilisateur.

    Expose uniquement les fonctions et données autorisées.
    """

    def __init__(self, user_data: dict, **kwargs):
        # Contexte avec builtins limités
        super().__init__(**kwargs)
        self._user_data = user_data
        self._execution_log = []

        # Expose uniquement les données utilisateur
        self.globals['user'] = user_data
        self.globals['log'] = []

    @property
    def user_data(self) -> dict:
        return self._user_data

    @property
    def execution_log(self) -> list:
        return self._execution_log

    def log_action(self, action: str, details: str = ''):
        """Enregistre une action exécutée par le script."""
        entry = {'action': action, 'details': details}
        self._execution_log.append(entry)
        # Aussi disponible dans le script
        self.globals['log'].append(entry)


class FlaskSandbox(Catnip):
    """
    Sandbox Catnip pour exécution de scripts utilisateur dans Flask.

    Fournit des APIs limitées et sécurisées.
    """

    @staticmethod
    def _send_email(ctx, recipient: str, subject: str, body: str):
        """Simule l'envoi d'email (API exposée au script)."""
        # Validation des paramètres
        if not recipient or '@' not in recipient:
            raise ValueError("Adresse email invalide")

        if len(body) > 10000:
            raise ValueError("Message trop long (max 10000 caractères)")

        # Simulation (dans un vrai cas, utiliser SMTP)
        ctx.log_action('send_email', f"To: {recipient}, Subject: {subject}")
        return True

    @staticmethod
    def _send_notification(ctx, message: str, level: str = 'info'):
        """Simule l'envoi de notification (API exposée)."""
        if level not in ['info', 'warning', 'error']:
            raise ValueError(f"Niveau invalide: {level}")

        ctx.log_action('send_notification', f"[{level.upper()}] {message}")
        return True

    @staticmethod
    def _update_status(ctx, status: str):
        """Met à jour le statut utilisateur (API exposée)."""
        allowed_statuses = ['active', 'inactive', 'pending', 'blocked']
        if status not in allowed_statuses:
            raise ValueError(f"Statut invalide: {status}")

        ctx._user_data['status'] = status
        ctx.log_action('update_status', status)
        return True

    @staticmethod
    def _check_permission(ctx, permission: str):
        """Vérifie si l'utilisateur a une permission (API exposée)."""
        permissions = ctx._user_data.get('permissions', [])
        return permission in permissions

    @staticmethod
    def _add_tag(ctx, tag: str):
        """Ajoute un tag à l'utilisateur (API exposée)."""
        if 'tags' not in ctx._user_data:
            ctx._user_data['tags'] = []

        if tag not in ctx._user_data['tags']:
            ctx._user_data['tags'].append(tag)
            ctx.log_action('add_tag', tag)

        return True

    @staticmethod
    def _remove_tag(ctx, tag: str):
        """Supprime un tag de l'utilisateur (API exposée)."""
        if 'tags' in ctx._user_data:
            if tag in ctx._user_data['tags']:
                ctx._user_data['tags'].remove(tag)
                ctx.log_action('remove_tag', tag)
                return True

        return False

    @staticmethod
    def _has_tag(ctx, tag: str):
        """Vérifie si l'utilisateur a un tag (API exposée)."""
        return tag in ctx._user_data.get('tags', [])

    # Fonctions DSL exposées au sandbox
    SANDBOX_FUNCTIONS = dict(
        send_email=pass_context(_send_email),
        send_notification=pass_context(_send_notification),
        update_status=pass_context(_update_status),
        check_permission=pass_context(_check_permission),
        add_tag=pass_context(_add_tag),
        remove_tag=pass_context(_remove_tag),
        has_tag=pass_context(_has_tag),
    )

    def __init__(self, user_data: dict, **kwargs):
        context = SandboxContext(user_data)
        super().__init__(context=context, **kwargs)
        self.context.globals.update(self.SANDBOX_FUNCTIONS)

    def run_user_script(self, script: str) -> dict:
        """
        Exécute un script utilisateur dans le sandbox.

        Returns:
            dict - Résultat avec status, result, log, errors
        """
        try:
            self.parse(script)
            result = self.execute()

            return {
                'status': 'success',
                'result': result,
                'log': self.context.execution_log,
                'user_data': self.context.user_data,
            }

        except SyntaxError as e:
            return {
                'status': 'error',
                'error_type': 'SyntaxError',
                'error_message': str(e),
                'log': self.context.execution_log,
            }

        except Exception as e:
            return {
                'status': 'error',
                'error_type': type(e).__name__,
                'error_message': str(e),
                'log': self.context.execution_log,
            }


# --- Démonstration (simulation Flask) ---

if __name__ == '__main__':
    print("⇒ Exemple 1 : Workflow automatique après inscription")
    print()

    # Données utilisateur (simulation request Flask)
    user_data = {
        'id': 123,
        'email': 'alice@example.com',
        'name': 'Alice',
        'status': 'pending',
        'permissions': ['read', 'write'],
        'tags': [],
    }

    # Script défini par l'administrateur (stocké en DB)
    welcome_workflow = """
    # Vérifier les permissions
    if check_permission('write') {
        # Activer le compte
        update_status('active')
        add_tag('new_user')

        # Envoyer email de bienvenue
        send_email(
            user['email'],
            'Bienvenue sur la plateforme',
            'Bonjour ' + user['name'] + ', votre compte est activé !'
        )

        # Notification interne
        send_notification('Nouveau compte activé: ' + user['name'], 'info')
    }
    """

    sandbox = FlaskSandbox(user_data)
    result = sandbox.run_user_script(welcome_workflow)

    print(f"Status: {result['status']}")
    print()
    print("Actions exécutées :")
    for action in result['log']:
        print(f"  - {action['action']}: {action['details']}")
    print()
    print(f"Statut utilisateur après exécution: {result['user_data']['status']}")
    print(f"Tags: {result['user_data']['tags']}")

    print()
    print("⇒ Exemple 2 : Règle métier personnalisée avec conditions")
    print()

    user_data = {
        'id': 456,
        'email': 'bob@example.com',
        'name': 'Bob',
        'status': 'active',
        'permissions': ['read'],  # Pas de permission 'write'
        'tags': ['premium'],
    }

    # Script utilisateur (workflow conditionnel)
    conditional_workflow = """
    # Vérifier permission admin
    if check_permission('admin') {
        send_notification('Admin détecté: ' + user['name'], 'warning')
        add_tag('admin')
    } else {
        # Utilisateur normal
        if has_tag('premium') {
            send_notification('Utilisateur premium: ' + user['name'], 'info')
        }
    }
    """

    sandbox = FlaskSandbox(user_data)
    result = sandbox.run_user_script(conditional_workflow)

    print(f"Status: {result['status']}")
    print()
    print("Actions exécutées :")
    for action in result['log']:
        print(f"  - {action['action']}: {action['details']}")

    print()
    print("⇒ Exemple 3 : Gestion d'erreur (email invalide)")
    print()

    user_data = {
        'id': 789,
        'email': 'invalid-email',  # Email invalide
        'name': 'Charlie',
    }

    # Script qui va échouer
    error_workflow = """
    send_email(user['email'], 'Test', 'Message')
    """

    sandbox = FlaskSandbox(user_data)
    result = sandbox.run_user_script(error_workflow)

    print(f"Status: {result['status']}")
    if result['status'] == 'error':
        print(f"Erreur: {result['error_type']} - {result['error_message']}")

    print()
    print("⇒ Exemple 4 : Syntaxe invalide")
    print()

    invalid_script = """
    send_email('test@example.com', 'Subject'  # Parenthèse manquante
    """

    sandbox = FlaskSandbox(dict(id=999, email='test@example.com'))
    result = sandbox.run_user_script(invalid_script)

    print(f"Status: {result['status']}")
    if result['status'] == 'error':
        print(f"Erreur: {result['error_type']}")
        print(f"Message: {result['error_message']}")

    print()
    print("⇒ Use Case Flask : Route avec script utilisateur")
    print()
    print("""
# Exemple d'intégration dans Flask:

from flask import Flask, request, jsonify
from flask_sandbox import FlaskSandbox

app = Flask(__name__)

@app.route('/api/trigger-workflow/<int:user_id>', methods=['POST'])
def trigger_workflow(user_id):
    # Récupérer données utilisateur
    user_data = get_user_from_db(user_id)

    # Récupérer script depuis la DB (défini par l'admin)
    workflow_script = get_workflow_script('onboarding')

    # Exécuter dans le sandbox
    sandbox = FlaskSandbox(user_data)
    result = sandbox.run_user_script(workflow_script)

    # Sauvegarder les changements si succès
    if result['status'] == 'success':
        save_user_to_db(user_id, result['user_data'])

    return jsonify(result)

if __name__ == '__main__':
    app.run(debug=True)
    """)
