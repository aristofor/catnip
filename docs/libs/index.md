# Modules stdlib

Modules natifs livrés avec Catnip, chargés via `import('name')`. Implémentés en Rust comme plugins `.so`, sans
dépendance Python runtime.

> À ne pas confondre avec les modules Python chargés via `-m` (CLI) ou `import('numpy')` (selon la policy). Les stdlib
> Catnip sont compilées dans le runtime ; les modules Python passent par le loader externe. Voir
> [Module loading](../user/MODULE_LOADING.md) pour la distinction.

## Modules disponibles

| Module            | Description                                                                |
| ----------------- | -------------------------------------------------------------------------- |
| [`http`](http.md) | Client HTTP et serveur léger : verbes, JSON, streaming, multipart, cookies |
| `io`              | Lecture / écriture fichiers (`open`, `print`, `write`, `read`)             |
| `sys`             | Métadonnées runtime (`platform`, `version`, `cpu_count`, `argv`, `exit`)   |

## Usage

```catnip
import('http')

response = http.get("https://example.com")
print(response.status, response.body)
```

`import('name')` charge le module et le bind dans le scope courant sous le nom `name`. Variantes :

```catnip
# Renomme
h = import('http')
h.get("https://example.com")

# Sélectif (extrait des symboles dans le scope)
import('http', 'get', 'post')
get("https://example.com")
```

## Convention

Chaque module stdlib expose au minimum :

- `PROTOCOL = "rust"` : permet de distinguer les modules natifs des modules Python
- `VERSION` : version semver du module

```catnip
import('http')
print(http.PROTOCOL)  # "rust"
print(http.VERSION)   # "0.2.0"
```

> Le `PROTOCOL` est un signal stable : si un jour Catnip migre certains modules vers du WASM ou du WASI, la valeur
> changera et le code applicatif peut s'y adapter. Pour l'instant elle est constante.

## Étendre

Pour écrire un nouveau module stdlib, voir [`catnip_libs/README.md`](../../catnip_libs/README.md) (guide contributeur).
Le manifeste `spec.toml` reste la source de vérité pour les signatures exportées ; les pages dans `docs/libs/`
documentent l'usage et les patterns.
