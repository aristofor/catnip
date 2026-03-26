# Construire une CLI avec Click

Utiliser `import("click")` dans un script Catnip pour construire une vraie CLI avec arguments, options, help et
validation.

> Click est un framework CLI Python. Catnip peut l'utiliser directement via `import("click")`.

______________________________________________________________________

## 1. Le pattern de base

<!-- check: no-check -->

```catnip
#!/usr/bin/env catnip
click = import("click")
sys = import("sys")

mantra = (name) => {
    click.echo(f"BORN TO SEGFAULT, {name}!")
}

cmd = click.Command("mantra", callback=mantra, params=list(click.Argument(list("name"))))

# sys.argv[0] est le nom du script — Click attend uniquement les arguments
ctx = cmd.make_context("mantra", sys.argv[1:])
p = ctx.params
mantra(p["name"])
```

Trois étapes :

1. Définir la callback (fonction Catnip normale)
1. Construire la commande avec `click.Command` + paramètres
1. Résoudre les arguments via `make_context`, puis appeler la callback

______________________________________________________________________

## 2. Arguments et options

Click distingue les arguments positionnels (`click.Argument`) et les options nommées (`click.Option`).

<!-- check: no-check -->

```catnip
#!/usr/bin/env catnip
click = import("click")
sys = import("sys")
io = import("io", wild=True)

transform = (input, output, scale, verbose) => {
    if (verbose) { print(f"Processing {input} -> {output} (scale={scale})") }
    # ... logique de transformation
    print("done")
}

cmd = click.Command("transform", callback=transform, params=list(
    click.Argument(list("input")),
    click.Argument(list("output")),
    click.Option(list("--scale", "-s"), default=1.0, type=click.FLOAT, help="Scale factor"),
    click.Option(list("--verbose", "-v"), is_flag=True, help="Show details")
))

ctx = cmd.make_context("transform", sys.argv[1:])
p = ctx.params
transform(p["input"], p["output"], p["scale"], p["verbose"])
```

Les types Click (`click.INT`, `click.FLOAT`, `click.STRING`, `click.BOOL`, `click.Choice(...)`) fonctionnent pour la
validation et la conversion.

______________________________________________________________________

## 3. Help automatique

Click génère le help à partir des paramètres déclarés :

<!-- check: no-check -->

```catnip
print(cmd.get_help(cmd.make_context("transform", list("_", "_"))))
```

```
Usage: transform [OPTIONS] INPUT OUTPUT

Options:
  -s, --scale FLOAT  Scale factor
  -v, --verbose      Show details
  --help             Show this message and exit.
```

Les arguments requis nécessitent des valeurs dummy dans `make_context` pour que le help se génère sans erreur.

______________________________________________________________________

## 4. Validation et choix

<!-- check: no-check -->

```catnip
#!/usr/bin/env catnip
click = import("click")
sys = import("sys")
io = import("io", wild=True)

deploy = (env, replicas) => {
    print(f"Deploying to {env} with {replicas} replicas")
}

cmd = click.Command("deploy", callback=deploy, params=list(
    click.Option(list("--env", "-e"), required=True,
        type=click.Choice(list("dev", "staging", "prod")),
        help="Target environment"),
    click.Option(list("--replicas", "-r"), default=1,
        type=click.IntRange(1, 10),
        help="Number of replicas (1-10)")
))

ctx = cmd.make_context("deploy", sys.argv[1:])
p = ctx.params
deploy(p["env"], p["replicas"])
```

Click valide les entrées avant que la callback soit appelée. Un `--env=invalid` produit une erreur claire sans code
supplémentaire.

______________________________________________________________________

## 5. `--help` et `SystemExit`

Click affiche le help et appelle `sys.exit(0)` quand `--help` est dans les arguments. Catnip convertit `SystemExit` en
`RuntimeError: Exit: 0` -- le help s'affiche mais une erreur suit.

Pour générer le help sans `sys.exit`, appeler `get_help` directement :

<!-- check: no-check -->

```catnip
if ("--help" in sys.argv) {
    # make_context avec des dummies pour satisfaire les arguments requis
    dummies = list("_") + sys.argv[1:]
    print(cmd.get_help(cmd.make_context(cmd.name, dummies, resilient_parsing=True)))
} else {
    ctx = cmd.make_context(cmd.name, sys.argv[1:])
    p = ctx.params
    callback(p["arg1"], p["opt1"])
}
```

______________________________________________________________________

## 6. Pourquoi l'API programmatique

Les décorateurs Click (`@click.command()`, `@click.option()`) ne fonctionnent pas sur les fonctions Catnip : ils posent
des attributs (`__click_params__`, `__name__`) sur l'objet fonction, et les fonctions Catnip n'ont pas de `__dict__`.

De même, `cmd.invoke(ctx)` introspecte la signature de la callback via `inspect.signature()`. Les fonctions Catnip
exposent `(*args, **kwargs)`, ce qui empêche Click de dispatcher les paramètres par nom.

L'API programmatique (`click.Command`, `click.Argument`, `click.Option`) contourne ces deux limites : pas de
décorateurs, pas de dispatch par introspection.

______________________________________________________________________

## 7. Quand préférer Python comme point d'entrée

Le pattern pur Catnip convient aux scripts utilitaires autonomes. Si la CLI grossit (sous-commandes, middlewares,
plugins, gestion d'erreurs fine), Python comme point d'entrée donne plus de contrôle :

```python
import click
from catnip import Catnip, Context
from catnip.exc import CatnipError

@click.command()
@click.argument("script", type=click.Path(exists=True))
@click.option("--name", default="world")
def main(script, name):
    ctx = Context(globals={"name": name})
    cat = Catnip(context=ctx)
    try:
        cat.parse(open(script).read())
        result = cat.execute()
    except CatnipError as exc:
        raise click.ClickException(str(exc)) from exc
    if result is not None:
        click.echo(result)
```

L'avantage : les décorateurs Click fonctionnent normalement, les exceptions Catnip sont converties en erreurs CLI
propres, et le contexte est contrôlé explicitement.

Voir [HOST_INTEGRATION](../user/HOST_INTEGRATION.md) et [EXTENDING_CONTEXT](../user/EXTENDING_CONTEXT.md) pour les
patterns d'embedding.

______________________________________________________________________

## Références

- [Click documentation](https://click.palletsprojects.com/)
