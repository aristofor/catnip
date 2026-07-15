# Menus interactifs avec `prompt_toolkit`

Guide de référence pour utiliser `prompt_toolkit` depuis un script Catnip quand l'objectif est de construire des menus
interactifs en terminal : choix unique, options multiples, confirmations, boutons et petits assistants à étapes.

Si ce qui t'intéresse est la sélection clavier et les listes cochables, commence ici.

> `prompt_toolkit` expose des fonctions factory qui retournent des objets `Application`. Un seul appel `.run()` suffit
> pour capturer le terminal, afficher le dialog, et récupérer la réponse.

______________________________________________________________________

## 1. Installation

```bash
pip install prompt_toolkit
```

`prompt_toolkit` n'est pas une dépendance de Catnip. L'installer dans le même environnement suffit.

______________________________________________________________________

## 2. Le pattern de base

Tous les dialogs `prompt_toolkit` suivent le même cycle :

1. Construire un `Application` via une fonction factory
1. Appeler `.run()` pour bloquer jusqu'à l'interaction utilisateur
1. Récupérer la valeur retournée

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')

app = pt.yes_no_dialog(title="Confirmation", text="Lancer le déploiement ?")
result = app.run()

if (result) {
    print("go.")
} else {
    print("aborted.")
}
```

> Les dialogs sont modaux. Le terminal est capturé pendant `.run()`, puis rendu. Comme un appel système, mais poli.

______________________________________________________________________

## 3. Quel dialog choisir

Pour les menus interactifs, trois helpers couvrent l'essentiel :

- `radiolist_dialog(...)` : choisir une seule option parmi plusieurs
- `checkboxlist_dialog(...)` : cocher plusieurs options puis valider
- `button_dialog(...)` : déclencher une action parmi quelques boutons

Règle pratique :

- tu veux une valeur métier unique -> `radiolist_dialog`
- tu veux une liste d'options actives -> `checkboxlist_dialog`
- tu veux 2 ou 3 actions explicites -> `button_dialog`

______________________________________________________________________

## 4. Oui / Non

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')

confirm = pt.yes_no_dialog(
    title="Suppression",
    text="Supprimer les fichiers temporaires ?",
    yes_text="Oui",
    no_text="Non"
).run()

# confirm : True ou False
```

Retourne `True` (oui) ou `False` (non). Retourne `None` si l'utilisateur fait Escape.

______________________________________________________________________

## 5. Saisie de texte

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')

name = pt.input_dialog(
    title="Identité",
    text="Nom du projet :",
    default="untitled"
).run()

if (name != None) {
    print(f"Projet : {name}")
}
```

`cancel_text` et `ok_text` contrôlent les boutons. Si l'utilisateur annule, la valeur retournée est `None`.

______________________________________________________________________

## 6. Choix unique (radiolist)

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')

env = pt.radiolist_dialog(
    title="Environnement",
    text="Cible de déploiement :",
    values=list(
        tuple("dev", "Development"),
        tuple("staging", "Staging"),
        tuple("prod", "Production")
    )
).run()

match (env) {
    case "dev" => print("deploying to dev")
    case "staging" => print("deploying to staging")
    case "prod" => print("deploying to prod (good luck)")
    case None => print("cancelled")
}
```

Chaque entrée de `values` est un `tuple(valeur_retournée, label_affiché)`. L'utilisateur navigue avec les flèches et
valide avec Entrée.

______________________________________________________________________

## 7. Choix multiples (checkboxlist)

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')

toppings = pt.checkboxlist_dialog(
    title="Pizza",
    text="Garnitures :",
    values=list(
        tuple("cheese", "Fromage"),
        tuple("ham", "Jambon"),
        tuple("mushrooms", "Champignons"),
        tuple("olives", "Olives"),
        tuple("anchovies", "Anchois")
    )
).run()

# toppings : liste des valeurs cochées, ou None si annulé
if (toppings != None) {
    print(f"Commande : {toppings}")
}
```

Retourne une liste des valeurs sélectionnées. Espace pour cocher/décocher, Entrée pour valider.

Pour un menu d'options, c'est le pattern à privilégier : la valeur retournée est déjà directement exploitable dans le
script.

______________________________________________________________________

## 8. Différence entre `radiolist`, `checkboxlist` et `button_dialog`

Les signatures se ressemblent, mais pas totalement :

- `radiolist_dialog(values=...)` attend des tuples `(valeur_retournée, label_affiché)`
- `checkboxlist_dialog(values=...)` attend aussi des tuples `(valeur_retournée, label_affiché)`
- `button_dialog(buttons=...)` attend des tuples `(label_affiché, valeur_retournée)`

Autrement dit, `button_dialog` inverse l'ordre.

______________________________________________________________________

## 9. Raccourcis clavier utiles

Dans les listes :

- `↑` / `↓` : naviguer
- `Espace` : cocher ou décocher dans `checkboxlist_dialog`
- `Entrée` : valider
- `Tab` : passer de la liste aux boutons
- `Escape` : annuler, avec retour `None`

______________________________________________________________________

## 10. Boutons

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')

action = pt.button_dialog(
    title="Action",
    text="Que faire avec ce commit ?",
    buttons=list(
        tuple("Deploy", "deploy"),
        tuple("Rollback", "rollback"),
        tuple("Ignore", "ignore")
    )
).run()

match (action) {
    case "deploy" => print("shipping")
    case "rollback" => print("rewinding")
    case "ignore" => print("nothing happened")
}
```

Chaque bouton est un `tuple(label, valeur_retournée)`. L'ordre est inversé par rapport à `radiolist_dialog` (label en
premier).

______________________________________________________________________

## 11. Message (info / alerte)

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')

pt.message_dialog(
    title="Statut",
    text="Déploiement terminé. 0 erreurs, 3 warnings.",
    ok_text="OK"
).run()
```

Bloque jusqu'à ce que l'utilisateur valide. Retourne `None`.

______________________________________________________________________

## 12. Combiner les dialogs

Les dialogs se chaînent naturellement. Chaque `.run()` bloque, puis le script continue.

<!-- check: no-check -->

```catnip
#!/usr/bin/env catnip
pt = import('prompt_toolkit.shortcuts')

# Étape 1 : saisie
name = pt.input_dialog(title="Setup", text="Nom du projet :").run()
if (name == None) { print("cancelled"); import('sys').exit(0) }

# Étape 2 : choix
lang = pt.radiolist_dialog(
    title="Setup",
    text="Langage principal :",
    values=list(
        tuple("rust", "Rust"),
        tuple("python", "Python"),
        tuple("catnip", "Catnip")
    )
).run()
if (lang == None) { print("cancelled"); import('sys').exit(0) }

# Étape 3 : options
features = pt.checkboxlist_dialog(
    title="Setup",
    text="Features :",
    values=list(
        tuple("ci", "CI/CD"),
        tuple("docker", "Docker"),
        tuple("tests", "Tests"),
        tuple("docs", "Documentation")
    )
).run()
if (features == None) { features = list() }

# Étape 4 : confirmation
ok = pt.yes_no_dialog(
    title="Confirmer",
    text=f"Créer {name} ({lang}) avec {len(features)} features ?"
).run()

if (ok) {
    print(f"Creating {name}...")
    print(f"  lang: {lang}")
    print(f"  features: {features}")
} else {
    print("aborted.")
}
```

> Un wizard en 30 lignes. Le ratio information/boilerplate reste du bon côté de la maintenance.

______________________________________________________________________

## 13. Construire un menu réutilisable

Quand tu as un menu principal, le plus simple est d'encapsuler le dialog dans une fonction puis de dispatcher sur la clé
retournée :

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')
sys = import('sys')

main_menu = () => {
    pt.radiolist_dialog(
        title="Operations",
        text="Choisir une commande",
        values=list(
            tuple("build", "Compiler"),
            tuple("test", "Lancer les tests"),
            tuple("clean", "Nettoyer"),
            tuple("exit", "Quitter")
        )
    ).run()
}

choice = main_menu()

match (choice) {
    case "build" => print("build...")
    case "test" => print("test...")
    case "clean" => print("clean...")
    case "exit" => sys.exit(0)
    case None => print("cancelled")
}
```

Le point important : la valeur utile est la clé interne, pas le libellé affiché.

______________________________________________________________________

## 14. Saisie avec mot de passe

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')

token = pt.input_dialog(
    title="Auth",
    text="API token :",
    password=True
).run()
```

`password=True` masque la saisie. Le terminal affiche des astérisques.

______________________________________________________________________

## 15. Construire `values` dynamiquement

Les listes de choix n'ont pas besoin d'être statiques.

<!-- check: no-check -->

```catnip
pt = import('prompt_toolkit.shortcuts')
os = import('os')

# Lister les fichiers .cat du répertoire courant
files = os.listdir(".")
cat_files = filter((f) => { f.endswith(".cat") }, files)

values = map((f) => { tuple(f, f) }, cat_files)

choice = pt.radiolist_dialog(
    title="Script",
    text="Quel script exécuter ?",
    values=list(values)
).run()

if (choice != None) {
    print(f"Running {choice}...")
}
```

______________________________________________________________________

## 16. Limites

- **Pas de REPL** : les dialogs `prompt_toolkit` ne fonctionnent pas dans la REPL interactive (`catnip` sans argument).
  La REPL ratatui active le raw mode terminal et redirige stdout vers un pipe interne pour réinjecter l'affichage via
  son propre viewport. prompt_toolkit a besoin du contrôle direct du terminal (mode cooked + accès au tty) pour ses
  séquences de positionnement curseur et d'alternate screen. Les deux sont incompatibles. Utiliser les dialogs depuis un
  script (`catnip script.cat`) ou via `catnip -c "..."`.

- **Pas de callback** : les dialogs `prompt_toolkit` n'acceptent pas de callbacks Catnip pour validation avancée.
  `input_dialog` a un paramètre `validator`, mais il attend un objet `Validator` Python avec des méthodes. Construire un
  `Validator` depuis Catnip n'est pas direct.

- **Terminal requis** : les dialogs capturent le terminal (alternate screen). Ils ne fonctionnent pas en mode pipe ou
  dans un contexte sans TTY.

- **Pas de `progress_dialog`** : ce dialog attend une callback Python avec une signature spécifique (deux paramètres
  callable). Les fonctions Catnip ne sont pas compatibles avec cette introspection.

______________________________________________________________________

## 17. Quand `prompt_toolkit` est le bon choix

Utilise `prompt_toolkit` si tu veux :

- un menu de lancement
- un choix unique propre au clavier
- une liste d'options multi-sélectionnables
- une confirmation sensible
- un petit wizard terminal

Reste sur `io.input(...)` si tu veux juste une ou deux questions libres sans dépendance externe.

______________________________________________________________________

## 18. Alternative légère : `input()` natif

Pour une simple confirmation sans dépendance externe :

<!-- check: no-check -->

```catnip
io = import('io')

answer = io.input("Continuer ? [y/N] ")
if (answer == "y" or answer == "Y") {
    print("ok")
} else {
    print("cancelled")
}
```

`prompt_toolkit` n'est justifié que quand tu as besoin de navigation clavier, de listes de choix, ou d'un affichage
structuré.

______________________________________________________________________

## Références

- [prompt_toolkit dialogs](https://python-prompt-toolkit.readthedocs.io/en/master/pages/dialogs.html)
- [MODULE_LOADING](../user/MODULE_LOADING.md) -- module loading Python depuis Catnip
- [CLICK_INTEGRATION](CLICK_INTEGRATION.md) -- construire une CLI avec Click
