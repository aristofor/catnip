# Exemples Standalone

Scripts Catnip exécutables directement via CLI.

## Quand utiliser le mode standalone ?

Le mode standalone est approprié pour :

### ✓ Cas d'usage valides

1. **Scripts de traitement de données one-off**

   - Transformation CSV/JSON simple
   - Calculs rapides sur datasets
   - Génération de rapports basiques

1. **Validation de configuration**

   - Vérifier des fichiers de config
   - Scripts de pre-commit hooks
   - Validation de données avant import

1. **Calculatrice/formules**

   - Calculs mathématiques complexes
   - Formules métier réutilisables
   - Scripts de calcul batch

1. **Prototypage rapide**

   - Tester des transformations
   - Explorer des données
   - POC avant intégration embedded

### ✗ Quand NE PAS utiliser standalone

- **Applications complètes** → Utilisez Python
- **Scripts > 200 lignes** → Utilisez Python avec Catnip embedded
- **Intégration avec libs complexes** → Python + Catnip embedded
- **Workflows multi-utilisateurs** → Embedded dans application web
- **Règles métier modifiables** → Embedded avec scripts en DB

**Règle empirique** : si ton script standalone devient complexe, passe sur Python et garde Catnip pour les parties
configurables.

______________________________________________________________________

## Exemples disponibles

### 1. [01_calculate.cat](01_calculate.cat)

**Cas d'usage** : Calculatrice avec fonctions mathématiques

```bash
catnip 01_calculate.cat
```

**Montre** :

- Fonctions récursives (factorial, fibonacci, power)
- Opérations sur listes (sum, average)
- Formules composées

**Utile pour** : Calculs batch, scripts de formules métier.

______________________________________________________________________

### 2. [02_filter_data.cat](02_filter_data.cat)

**Cas d'usage** : Filtrage et transformation fonctionnelle

```bash
catnip 02_filter_data.cat
```

**Montre** :

- Fonctions de filtrage génériques
- Fonctions de mapping
- Composition de transformations
- Style fonctionnel avec lambdas

**Note** : Pour pipelines complexes, voir `docs/examples/embedding/03_etl_pipeline.py`.

______________________________________________________________________

### 3. [03_transform_csv.cat](03_transform_csv.cat)

**Cas d'usage** : Transformation de données CSV/JSON simple

```bash
catnip 03_transform_csv.cat
# Ou avec shebang:
chmod +x 03_transform_csv.cat
./03_transform_csv.cat
```

**Montre** :

- Filtrage de données (âge > 25)
- Transformation de structure
- Ajout de champs calculés

**Note** : Pour manipulation CSV réelle, utilisez Python + pandas embedded.

______________________________________________________________________

### 4. [04_config_validator.cat](04_config_validator.cat)

**Cas d'usage** : Validation de configuration

```bash
catnip 04_config_validator.cat
```

**Montre** :

- Règles de validation (ranges, types, valeurs autorisées)
- Messages d'erreur clairs
- Code de sortie (0 = succès, 1 = échec)

**Utile pour** : Pre-commit hooks, CI/CD, validation avant déploiement.

______________________________________________________________________

### 5. [05_data_report.cat](05_data_report.cat)

**Cas d'usage** : Génération de rapport de données

```bash
catnip 05_data_report.cat
```

**Montre** :

- Calcul de métriques (total, moyenne)
- Identification du best seller
- Formatage de rapport texte

**Alternative embedded** : Voir `docs/examples/embedding/05_report_builder.py` pour rapports avec templates.

______________________________________________________________________

## Installation système (shebang)

Pour rendre vos scripts Catnip exécutables comme commandes système :

### 1. Ajouter shebang

```bash
#!/usr/bin/env catnip
# Votre code ici
```

### 2. Rendre exécutable

```bash
chmod +x script.cat
```

### 3. Installer dans PATH (optionnel)

```bash
# Copier dans ~/bin (doit être dans PATH)
cp script.cat ~/bin/my-command
chmod +x ~/bin/my-command

# Ou créer un symlink
ln -s /path/to/script.cat ~/bin/my-command
```

### 4. Utiliser

```bash
./script.cat              # Local
my-command                # Si dans PATH
```

______________________________________________________________________

## Bonnes pratiques

### Structure d'un bon script standalone

```python
#!/usr/bin/env catnip
# Description courte du script
# Usage: catnip script.cat [args]

# 1. Définir fonctions utilitaires
helper = (x) => { x * 2 }

# 2. Charger/définir données
data = list(1, 2, 3, 4, 5)

# 3. Traiter données
result = helper(42)

# 4. Afficher résultats
print('Résultat : ' + str(result))

# 5. Retourner valeur de sortie
result
```

### Gestion des erreurs

```python
# Validation des entrées
if len(data) == 0 {
    print('Erreur : Données vides')
    1  # Code erreur
} else {
    # Traitement normal
    process(data)
    0  # Code succès
}
```

### Comments et documentation

```python
# Utilisez des commentaires pour expliquer la logique
# Pas besoin de docstrings (pas de fonction principale)

# Évitez les commentaires évidents
x = 10  # ✗ "Set x to 10"
x = 10  # ✓ "Seuil de validation (config)"
```

______________________________________________________________________

## Comparaison Standalone vs Embedded

| Critère          | Standalone      | Embedded           |
| ---------------- | --------------- | ------------------ |
| **Taille**       | < 200 lignes    | Illimité           |
| **Utilisateurs** | Développeurs    | Admins/Users       |
| **Modification** | Éditer fichier  | Scripts en DB      |
| **Sécurité**     | Accès système   | Sandbox isolé      |
| **Intégration**  | CLI/cron        | Application Python |
| **Best pour**    | Scripts one-off | Règles métier      |

______________________________________________________________________

## Alternatives à considérer

### Quand utiliser Python au lieu de Catnip standalone

Si votre script :

- Fait plus de 200 lignes
- Nécessite imports de libs complexes (requests, sqlalchemy, etc.)
- Manipule des fichiers/réseau de manière intensive
- Est une application complète avec UI

→ **Utilisez Python** avec Catnip embedded pour les parties configurables.

### Exemple de migration standalone → embedded

**Avant (standalone)** :

```bash
# script.cat (150 lignes)
# Règles métier hardcodées
if user['age'] > 25 { ... }
```

**Après (embedded)** :

```python
# app.py
from catnip import Catnip

# Charger règles depuis DB
rules = db.get_rules('user_validation')

dsl = Catnip()
dsl.parse(rules)
result = dsl.execute({'user': user_data})
```

______________________________________________________________________

## Ressources

- [CLI Documentation](../../user/CLI.md) - Options CLI complètes
- [REPL Guide](../../user/REPL.md) - Mode interactif
- [Embedding Guide](../../user/EMBEDDING_GUIDE.md) - Intégrer Catnip dans apps Python
- [Language Reference](../../lang/index.md) - Syntaxe complète
