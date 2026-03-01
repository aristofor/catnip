# Guide Shebang

Comment créer des scripts Catnip exécutables comme commandes système.

## Qu'est-ce qu'un shebang ?

Le **shebang** (`#!`) est une ligne spéciale au début d'un script qui indique au système quel interpréteur utiliser.

```python
#!/usr/bin/env catnip
# Votre code Catnip ici
```

Avec un shebang, vous pouvez exécuter votre script directement :

```bash
./script.cat          # Au lieu de: catnip script.cat
```

______________________________________________________________________

## Configuration de base

### 1. Créer un script avec shebang

```bash
cat > hello.cat << 'EOF'
#!/usr/bin/env catnip
# Hello World script

name = 'World'
print('Hello, ' + name + '!')
EOF
```

### 2. Rendre le script exécutable

```bash
chmod +x hello.cat
```

### 3. Exécuter

```bash
./hello.cat
# Output: Hello, World!
```

______________________________________________________________________

## Installation dans PATH

Pour utiliser vos scripts comme commandes système (sans `./`) :

### Option A : ~/bin (Recommandé)

```bash
# 1. Créer ~/bin si inexistant
mkdir -p ~/bin

# 2. Ajouter à PATH (dans ~/.bashrc ou ~/.zshrc)
echo 'export PATH="$HOME/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc

# 3. Copier votre script
cp script.cat ~/bin/my-command
chmod +x ~/bin/my-command

# 4. Utiliser n'importe où
my-command
```

### Option B : /usr/local/bin (System-wide)

```bash
# Nécessite sudo
sudo cp script.cat /usr/local/bin/my-command
sudo chmod +x /usr/local/bin/my-command

# Disponible pour tous les utilisateurs
my-command
```

### Option C : Symlink (Développement)

```bash
# Créer un symlink au lieu de copier
ln -s /path/to/dev/script.cat ~/bin/my-command

# Avantage : modifications visibles immédiatement
```

______________________________________________________________________

## Patterns de scripts système

### Script avec arguments

Les arguments passés au script sont disponibles dans la variable globale `argv` :

```python
#!/usr/bin/env catnip
# Usage: process-data.cat input.json

# argv = ["/path/to/process-data.cat", "input.json"]
input_file = argv[1]
print('Processing ' + input_file)
```

```bash
./process-data.cat data.json
# ⇒ Processing data.json

# Ou sans shebang :
catnip process-data.cat data.json
# ⇒ Processing data.json
```

`argv[0]` contient le chemin du script, les arguments suivent à partir de `argv[1]`.

Pour forcer le mode script sur un fichier portant le même nom qu'une sous-commande (`format`, `lint`, etc.), utiliser
`--` :

```bash
catnip -- format arg1 arg2
```

### Script avec code de sortie

```python
#!/usr/bin/env catnip
# Validation script

valid = True

if condition_fails {
    print('Erreur: validation échouée')
    valid = False
}

# Retourner 0 (succès) ou 1 (échec)
if valid { 0 } else { 1 }
```

Utilisation dans CI/CD :

```bash
./validate.cat
if [ $? -eq 0 ]; then
    echo "Validation OK"
else
    echo "Validation FAILED"
    exit 1
fi
```

### Script avec configuration

```python
#!/usr/bin/env catnip
# Script utilisant un fichier de config

# Charger config depuis dict hardcodé
config = dict(
    output_dir='/tmp',
    verbose=True
)

if config['verbose'] {
    print('Mode verbose activé')
}

# Pour charger depuis JSON, utilisez Python wrapper
```

______________________________________________________________________

## Exemples de commandes système

### 1. validate-config

```bash
#!/usr/bin/env catnip
# Valider des fichiers de configuration
# Usage: validate-config

config = dict(
    port=8080,
    timeout=30
)

valid = True
if config['port'] < 1024 {
    print('Erreur: Port privilégié')
    valid = False
}

if valid { 0 } else { 1 }
```

Installation :

```bash
chmod +x validate-config.cat
mv validate-config.cat ~/bin/validate-config
validate-config  # Utiliser n'importe où
```

### 2. calc

```bash
#!/usr/bin/env catnip
# Calculatrice rapide
# Usage: calc

fib = (n) => {
    if n <= 1 { n }
    else { fib(n-1) + fib(n-2) }
}

print('Fibonacci(10) = ' + str(fib(10)))
print('Fibonacci(20) = ' + str(fib(20)))
```

### 3. data-stats

```bash
#!/usr/bin/env catnip
# Statistiques sur données
# Usage: data-stats

numbers = list(10, 20, 30, 40, 50)

sum = 0
i = 0
while i < len(numbers) {
    sum = sum + numbers[i]
    i = i + 1
}

avg = sum / len(numbers)

print('Somme: ' + str(sum))
print('Moyenne: ' + str(avg))
```

______________________________________________________________________

## Intégration avec outils système

### Cron jobs

```bash
# Ajouter à crontab
crontab -e

# Exécuter tous les jours à 6h
0 6 * * * /home/user/bin/daily-report.cat

# Avec logs
0 6 * * * /home/user/bin/daily-report.cat >> /var/log/reports.log 2>&1
```

### Git hooks

```bash
# .git/hooks/pre-commit
#!/bin/bash
/path/to/validate-config.cat
exit $?
```

Rendre exécutable :

```bash
chmod +x .git/hooks/pre-commit
```

### Make targets

```makefile
# Makefile
.PHONY: validate
validate:
	@./scripts/validate-config.cat

.PHONY: report
report:
	@./scripts/generate-report.cat > report.txt
```

### Shell aliases

```bash
# ~/.bashrc ou ~/.zshrc
alias calc='~/bin/calc.cat'
alias validate='~/bin/validate-config.cat'
alias stats='~/bin/data-stats.cat'
```

______________________________________________________________________

## Limitations actuelles

### Pas d'accès fichiers direct

Catnip n'a pas de fonction `open()` intégrée.

**Workaround** : Python wrapper pour charger données

```python
# wrapper.py
import json
from catnip import Catnip

with open('data.json') as f:
    data = json.load(f)

catnip = Catnip()
catnip.context.globals['data'] = data
catnip.parse(open('script.cat').read())
result = catnip.execute()
```

### Pas d'imports Python

Catnip standalone ne peut pas faire `import requests`.

**Workaround** : Embedded mode avec `import()`

```python
#!/usr/bin/env catnip
json = import("json")

# Maintenant json est disponible
# data = json.loads(...)
```

Ou Python wrapper :

```python
# wrapper.py
import requests
from catnip import Catnip

response = requests.get('https://api.example.com')
data = response.json()

catnip = Catnip()
catnip.context.globals['api_data'] = data
catnip.parse(open('process.cat').read())
catnip.execute()
```

______________________________________________________________________

## Quand utiliser shebang vs embedded

| Use Case                | Shebang Standalone | Embedded     |
| ----------------------- | ------------------ | ------------ |
| Script personnel        | ✓ Simple           | ▲✓ Overhead  |
| Pre-commit hook         | ✓ Direct           | ▲✓ Complexe  |
| Cron job                | ✓ Facile           | ▲✓ Setup     |
| Calculs rapides         | ✓ Optimal          | ✗ Trop lourd |
| Règles métier app       | ✗ Pas flexible     | ✓ Idéal      |
| Scripts utilisateur web | ✗ Pas sécurisé     | ✓ Sandbox    |
| Workflows complexes     | ✗ Limitations      | ✓ Extensible |

**Règle** : Shebang pour scripts personnels simples, Embedded pour intégration applicative.

______________________________________________________________________

## Dépannage

### "command not found"

```bash
# Vérifier que le script est dans PATH
which my-command

# Vérifier PATH
echo $PATH

# Vérifier permissions
ls -l ~/bin/my-command
```

### "Permission denied"

```bash
# Rendre exécutable
chmod +x script.cat
```

### "bad interpreter"

```bash
# Vérifier que catnip est installé
which catnip

# Vérifier shebang (pas d'espaces)
head -1 script.cat
# Doit être exactement: #!/usr/bin/env catnip
```

### "No such file or directory"

```bash
# Si le shebang est mal formaté (Windows line endings)
dos2unix script.cat

# Ou manuellement
sed -i 's/\r$//' script.cat
```

______________________________________________________________________

## Exemples complets

Voir [`docs/examples/standalone/`](../examples/standalone/) pour 5 exemples de scripts avec shebang :

1. **transform_csv.cat** - Transformation de données
1. **config_validator.cat** - Validation de config
1. **data_report.cat** - Génération de rapports
1. **calculate.cat** - Calculatrice mathématique
1. **filter_data.cat** - Filtrage fonctionnel

Tous ces exemples sont prêts à être installés comme commandes système.

______________________________________________________________________

## Ressources

- [CLI Documentation](CLI.md) - Options CLI complètes
- [Standalone Examples](../examples/standalone/) - Scripts exemples
- [Embedding Guide](EMBEDDING_GUIDE.md) - Alternative pour scripts complexes
