# Exemples d'intégration

Cette section contient des exemples complets d'intégration de Catnip comme DSL dans des applications Python. Chaque
exemple montre un cas d'usage et des patterns réutilisables.

## Vue d'ensemble

Catnip est conçu comme DSL embarqué dans des applications Python, avec un moteur de scripting sécurisé. Les exemples
ci-dessous montrent des cas où Catnip permet aux utilisateurs ou admins de définir des règles, des workflows ou des
transformations sans changer le code.

## Pattern commun

Tous les exemples suivent le même schéma :

```python
from catnip import Catnip, Context, pass_context

# 1. Sous-classer Context pour état spécifique
class MyContext(Context):
    def __init__(self, data: dict, **kwargs):
        super().__init__(**kwargs)
        self._data = data
        self.globals['data'] = data

# 2. Sous-classer Catnip pour DSL
class MyDSL(Catnip):
    @staticmethod
    def _my_function(ctx, arg):
        # Logique métier
        return result

    DSL_FUNCTIONS = dict(
        my_function=pass_context(_my_function),
    )

    def __init__(self, data: dict, **kwargs):
        context = MyContext(data)
        super().__init__(context=context, **kwargs)
        self.context.globals.update(self.DSL_FUNCTIONS)

# 3. Utiliser le DSL
dsl = MyDSL(data)
result = dsl.execute(script)
```

## Exemples disponibles

### 1. DataFrame DSL (`01_dataframe_dsl.py`)

**Cas d'usage** : manipulation de DataFrames pandas avec syntaxe Catnip

Exemple d'embedding simple pour créer un DSL qui manipule des DataFrames pandas. Les transformations sont définies en
Catnip avec une syntaxe chaînée.

**Prérequis** : `pip install pandas`

```bash
python docs/examples/embedding/01_dataframe_dsl.py
```

______________________________________________________________________

### 2. Configuration DSL (`02_config_dsl.py`)

**Cas d'usage** : validation de fichiers de configuration avec règles déclaratives

Montre comment créer un DSL pour valider des configurations utilisateur avec des règles claires (types, ranges,
patterns, etc.). Idéal pour les systèmes configurables.

**Fonctions DSL** : `required()`, `type_check()`, `range_check()`, `length_check()`, `one_of()`

```bash
python docs/examples/embedding/02_config_dsl.py
```

______________________________________________________________________

### 3. ETL Pipeline (`03_etl_pipeline.py`)

**Cas d'usage** : transformation de données (CSV → JSON/autre format)

Démontre comment utiliser Catnip pour créer des pipelines ETL déclaratifs. Les transformations (filtrage, mapping,
agrégation) sont définies en Catnip et peuvent être stockées en base de données.

**Fonctions DSL** : `filter_rows()`, `map_field()`, `rename_field()`, `add_field()`, `drop_field()`, `sort_by()`,
`group_by()`

```bash
python docs/examples/embedding/03_etl_pipeline.py
```

______________________________________________________________________

### 4. Rule Engine (`04_rule_engine.py`)

**Cas d'usage** : moteur de règles métier (pricing, éligibilité, calculs)

Démontre un moteur de règles pour pricing dynamique, éligibilité crédit, calcul de frais. Les règles métier sont
définies en Catnip et modifiables sans recompiler.

**Fonctions DSL** : `set_result()`, `mark_applied()`, `get_input()`, `calculate_percentage()`, `apply_discount()`,
`clamp()`

```bash
python docs/examples/embedding/04_rule_engine.py
```

______________________________________________________________________

### 5. Report Builder (`05_report_builder.py`)

**Cas d'usage** : génération de rapports avec templates et données dynamiques

Montre comment utiliser Catnip pour générer des rapports personnalisés avec calcul de métriques et formatage. Les
templates peuvent être définis par les administrateurs.

**Fonctions DSL** : `calculate_sum()`, `calculate_avg()`, `calculate_count()`, `set_metric()`, `format_currency()`,
`add_section()`

```bash
python docs/examples/embedding/05_report_builder.py
```

______________________________________________________________________

### 6. Workflow DSL (`06_workflow_dsl.py`)

**Cas d'usage** : orchestration de workflows (ETL, onboarding, traitement de commandes)

Illustre comment créer des workflows avec étapes séquentielles, gestion d'état et historique. Les workflows peuvent être
définis en Catnip et stockés en base de données.

**Fonctions DSL** : `step()`, `set_state()`, `get_state()`, `log_event()`, `validate_state()`, `increment()`

```bash
python docs/examples/embedding/06_workflow_dsl.py
```

______________________________________________________________________

### 7. Flask Sandbox (`07_flask_sandbox.py`)

**Cas d'usage** : exécution sécurisée de scripts utilisateur dans Flask

Illustre comment créer un sandbox Catnip pour permettre aux utilisateurs d'écrire des workflows personnalisés dans une
application web. Les scripts sont isolés et n'exposent que les APIs autorisées.

**Fonctions DSL** : `send_email()`, `send_notification()`, `update_status()`, `check_permission()`, `add_tag()`,
`has_tag()`

```bash
python docs/examples/embedding/07_flask_sandbox.py
```

______________________________________________________________________

### 8. Jupyter Integration (`08_jupyter_integration.py`)

**Cas d'usage** : magic commands IPython pour Jupyter notebooks

Montre comment créer des magic commands IPython (`%catnip`, `%%catnip`) pour utiliser Catnip dans des notebooks Jupyter.
Permet l'analyse de données interactive avec partage d'état Python ↔ Catnip.

**Magic commands** : `%catnip`, `%%catnip`, `%catnip_load`, `%catnip_reset`, `%catnip_vars`

**Installation dans Jupyter** :

```python
%load_ext jupyter_integration
```

**Démo** :

```bash
python docs/examples/embedding/08_jupyter_integration.py
```

______________________________________________________________________

### 9. Streamlit App (`09_streamlit_app.py`)

**Cas d'usage** : dashboards interactifs et playground web

Démontre l'intégration de Catnip dans Streamlit pour créer un playground interactif et une calculatrice. Les
utilisateurs peuvent écrire et exécuter du code Catnip dans le navigateur.

**Lancement de l'application** :

```bash
pip install streamlit
streamlit run docs/examples/embedding/09_streamlit_app.py
```

**Démo** :

```bash
python docs/examples/embedding/09_streamlit_app.py
```

______________________________________________________________________

## Avantages de l'embedding

### Sécurité

Les scripts Catnip s'exécutent dans un contexte isolé. Seules les fonctions explicitement exposées sont disponibles.

### Flexibilité

Les règles métier et workflows peuvent être modifiés sans recompiler l'application. Stockez les scripts en base de
données pour permettre aux administrateurs de les éditer.

### Simplicité

La syntaxe Catnip est plus simple que Python pour des utilisateurs non-développeurs. Les scripts sont déclaratifs et
focalisés sur la logique métier.

### Performance

Les scripts sont compilés en bytecode avant exécution.

## Ressources supplémentaires

- [Guide utilisateur](../../user/index.md) - Documentation complète sur l'utilisation de Catnip
- [Référence du langage](../../lang/index.md) - Syntaxe complète de Catnip
- [Architecture interne](../../dev/index.md) - Architecture et contribution au projet
