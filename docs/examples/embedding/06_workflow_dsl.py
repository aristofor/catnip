"""
Exemple d'intégration de Catnip comme DSL pour orchestration de workflows.

Montre comment :
1. Définir des workflows avec étapes séquentielles
2. Gérer l'état et la progression du workflow
3. Implémenter des conditions et branchements
4. Capturer les résultats de chaque étape

Use case : Orchestration de tâches métier (ETL, onboarding, traitement de commandes).
"""

from catnip import Catnip, Context, pass_context
from datetime import datetime


class WorkflowContext(Context):
    """
    Contexte d'exécution de workflow.

    Stocke l'état du workflow, les résultats des étapes, et l'historique.
    """

    def __init__(self, initial_state: dict = None, **kwargs):
        super().__init__(**kwargs)
        self._state = initial_state or {}
        self._steps = []
        self._current_step = None

        # Expose l'état dans le contexte
        self.globals['state'] = self._state

    @property
    def state(self) -> dict:
        return self._state

    @property
    def steps(self) -> list:
        return self._steps

    @property
    def current_step(self):
        return self._current_step

    def set_state(self, key: str, value):
        """Met à jour l'état du workflow."""
        self._state[key] = value
        self.globals['state'][key] = value

    def start_step(self, step_name: str):
        """Démarre une nouvelle étape."""
        self._current_step = {
            'name': step_name,
            'start_time': datetime.now().isoformat(),
            'status': 'running',
        }

    def complete_step(self, result=None, error=None):
        """Complète l'étape courante."""
        if self._current_step:
            self._current_step['end_time'] = datetime.now().isoformat()
            self._current_step['status'] = 'error' if error else 'success'
            if result is not None:
                self._current_step['result'] = result
            if error:
                self._current_step['error'] = str(error)
            self._steps.append(self._current_step)
            self._current_step = None


class WorkflowDSL(Catnip):
    """
    DSL Catnip pour orchestration de workflows.

    Syntaxe déclarative pour définir et exécuter des workflows.
    """

    @staticmethod
    def _step(ctx, name: str, action):
        """Exécute une étape du workflow."""
        ctx.start_step(name)
        try:
            result = action() if callable(action) else action
            ctx.complete_step(result=result)
            return result
        except Exception as e:
            ctx.complete_step(error=e)
            raise

    @staticmethod
    def _set_state(ctx, key: str, value):
        """Met à jour l'état du workflow."""
        ctx.set_state(key, value)
        return value

    @staticmethod
    def _get_state(ctx, key: str, default=None):
        """Récupère une valeur de l'état."""
        return ctx.state.get(key, default)

    @staticmethod
    def _log_event(ctx, event: str, details: str = ''):
        """Enregistre un événement dans le workflow."""
        if 'events' not in ctx.state:
            ctx.state['events'] = []
        ctx.state['events'].append({
            'timestamp': datetime.now().isoformat(),
            'event': event,
            'details': details,
        })
        return True

    @staticmethod
    def _validate_state(ctx, key: str, expected_value):
        """Valide qu'une valeur d'état correspond à l'attendu."""
        actual = ctx.state.get(key)
        if actual != expected_value:
            raise ValueError(f"État invalide : {key} = {actual}, attendu {expected_value}")
        return True

    @staticmethod
    def _increment(ctx, key: str, delta: int = 1):
        """Incrémente une valeur numérique dans l'état."""
        current = ctx.state.get(key, 0)
        new_value = current + delta
        ctx.set_state(key, new_value)
        return new_value

    # Fonctions DSL injectées
    WORKFLOW_FUNCTIONS = dict(
        step=pass_context(_step),
        set_state=pass_context(_set_state),
        get_state=pass_context(_get_state),
        log_event=pass_context(_log_event),
        validate_state=pass_context(_validate_state),
        increment=pass_context(_increment),
    )

    def __init__(self, initial_state: dict = None, **kwargs):
        context = WorkflowContext(initial_state)
        super().__init__(context=context, **kwargs)
        self.context.globals.update(self.WORKFLOW_FUNCTIONS)

    def execute_workflow(self, workflow_script: str) -> dict:
        """
        Exécute le workflow.

        Returns:
            dict - État final et historique des étapes
        """
        try:
            self.parse(workflow_script)
            self.execute()
            status = 'completed'
        except Exception as e:
            status = 'failed'
            self.context.set_state('error', str(e))

        return {
            'status': status,
            'state': self.context.state,
            'steps': self.context.steps,
        }


# --- Démonstration ---

if __name__ == '__main__':
    print("⇒ Exemple 1 : Workflow de traitement de commande")
    print()

    # État initial
    initial_state = {
        'order_id': 'ORD-12345',
        'customer_id': 'CUST-789',
        'amount': 150.00,
        'items': ['item1', 'item2', 'item3'],
    }

    # Définition du workflow
    order_workflow = """
    # Étape 1 : Validation
    log_event('workflow_start', 'Traitement commande ' + state['order_id'])
    set_state('status', 'validating')

    # Vérifier montant
    if state['amount'] > 0 {
        set_state('validation_passed', True)
        log_event('validation', 'Montant valide')
    }

    # Étape 2 : Vérification stock
    set_state('status', 'checking_stock')
    items_count = 3  # Simulé
    set_state('items_available', items_count)
    log_event('stock_check', 'Stock disponible pour tous les articles')

    # Étape 3 : Réservation
    if get_state('items_available', 0) > 0 {
        set_state('status', 'reserved')
        log_event('reservation', 'Articles réservés')
    }

    # Étape 4 : Paiement
    set_state('status', 'processing_payment')
    set_state('payment_status', 'completed')
    log_event('payment', 'Paiement traité avec succès')

    # Étape 5 : Finalisation
    set_state('status', 'confirmed')
    log_event('workflow_end', 'Commande confirmée')
    """

    workflow = WorkflowDSL(initial_state)
    result = workflow.execute_workflow(order_workflow)

    print(f"Workflow : {result['status']}")
    print(f"Commande : {result['state']['order_id']}")
    print(f"Statut final : {result['state']['status']}")
    print()
    print("Événements :")
    if 'events' in result['state']:
        for event in result['state']['events']:
            print(f"  [{event['timestamp'][-12:-4]}] {event['event']}: {event['details']}")

    print()
    print("⇒ Exemple 2 : Workflow conditionnel avec branchements")
    print()

    initial_state = {
        'user_id': 'USER-456',
        'action': 'withdraw',
        'amount': 500,
        'balance': 1000,
    }

    banking_workflow = """
    # Initialisation
    log_event('start', 'Transaction initiée')
    set_state('transaction_id', 'TXN-' + state['user_id'])

    # Vérification du type d'action
    if state['action'] == 'withdraw' {
        # Vérifier solde suffisant
        if state['balance'] >= state['amount'] {
            new_balance = state['balance'] - state['amount']
            set_state('balance', new_balance)
            set_state('status', 'success')
            log_event('withdraw', 'Retrait effectué')
        } else {
            set_state('status', 'insufficient_funds')
            log_event('error', 'Solde insuffisant')
        }
    }

    if state['action'] == 'deposit' {
        new_balance = state['balance'] + state['amount']
        set_state('balance', new_balance)
        set_state('status', 'success')
        log_event('deposit', 'Dépôt effectué')
    }

    # Finalisation
    log_event('end', 'Transaction terminée')
    """

    workflow = WorkflowDSL(initial_state)
    result = workflow.execute_workflow(banking_workflow)

    print(f"Transaction : {result['state']['transaction_id']}")
    print(f"Action : {result['state']['action']}")
    print(f"Montant : ${result['state']['amount']}")
    print(f"Solde initial : ${initial_state['balance']}")
    print(f"Solde final : ${result['state']['balance']}")
    print(f"Statut : {result['state']['status']}")

    print()
    print("⇒ Exemple 3 : Workflow avec compteurs et boucles")
    print()

    initial_state = {
        'batch_size': 5,
        'processed': 0,
        'errors': 0,
    }

    batch_workflow = """
    # Initialisation
    set_state('status', 'processing')
    log_event('start', 'Traitement batch commencé')

    # Simuler traitement d'items
    batch_size = state['batch_size']
    success_count = 4  # Simulé : 4 succès, 1 échec

    set_state('processed', success_count)
    set_state('errors', batch_size - success_count)

    # Incrémenter compteur total
    increment('total_processed', success_count)

    # Vérifier taux de succès
    success_rate = success_count * 100 / batch_size
    set_state('success_rate', success_rate)

    if success_rate >= 80 {
        set_state('status', 'completed')
        log_event('success', 'Batch traité avec succès')
    } else {
        set_state('status', 'partial_failure')
        log_event('warning', 'Certains items ont échoué')
    }
    """

    workflow = WorkflowDSL(initial_state)
    result = workflow.execute_workflow(batch_workflow)

    print(f"Batch : {result['state']['batch_size']} items")
    print(f"Traités : {result['state']['processed']}")
    print(f"Erreurs : {result['state']['errors']}")
    print(f"Taux de succès : {result['state']['success_rate']}%")
    print(f"Statut : {result['state']['status']}")
    print()
    print("Événements :")
    if 'events' in result['state']:
        for event in result['state']['events']:
            print(f"  - {event['event']}: {event['details']}")

    print()
    print("⇒ Use Case : Intégration dans application")
    print()
    print("""
# Exemple d'utilisation dans une application:

from workflow_dsl import WorkflowDSL

def process_order(order_data):
    # Charger définition du workflow depuis DB/config
    workflow_definition = load_workflow('order_processing')

    # Exécuter le workflow
    workflow = WorkflowDSL(initial_state=order_data)
    result = workflow.execute_workflow(workflow_definition)

    # Sauvegarder résultat et historique
    save_workflow_result(order_data['order_id'], result)

    # Notifier selon le statut
    if result['status'] == 'completed':
        notify_customer(order_data['customer_id'], 'order_confirmed')
    else:
        notify_admin('workflow_failed', result['state'].get('error'))

    return result
    """)
