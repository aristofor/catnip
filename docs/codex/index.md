# Chargement de Modules Python

Exemples d'intégration de bibliothèques Python dans Catnip.

## Principe

Catnip charge des modules Python via `import("module")` ou l'option CLI `-m module`. Les objets et fonctions du module
deviennent accessibles directement dans le code Catnip.

```catnip
numpy = import("numpy")
arr = numpy.array(list(1, 2, 3))
print(arr.mean())
```

Pas de wrapper, pas de FFI : l'interopérabilité est native.

## Organisation

| Dossier                            | Domaine                        | Exemples                                           |
| ---------------------------------- | ------------------------------ | -------------------------------------------------- |
| [files-formats/](files-formats/)   | Fichiers et formats de données | pathlib, regex, yaml, json, xml, jmespath, parquet |
| [data-analytics/](data-analytics/) | Analyse de données             | numpy, duckdb, sqlalchemy, polars                  |
| [web/](web/)                       | HTTP et APIs                   | httpx, selectolax                                  |
| [images-media/](images-media/)     | Images et multimédia           | pillow                                             |
| [geospatial/](geospatial/)         | Géospatial                     | haversine, rasterio, sentinel-2 (STAC)             |
| [geometry/](geometry/)             | Géométrie                      | enveloppe convexe (Graham, Quickhull)              |

## Exemples de base

Les exemples minimalistes de chargement de modules sont dans
[`/examples/module-loading/`](../examples/module-loading/index.md).

## Modes de chargement

### Namespace (défaut)

```bash
catnip -m numpy script.cat
```

Le module est accessible via son nom : `numpy.array(...)`.

### Injection directe

```bash
catnip -m numpy script.cat
```

Le module est accessible via son nom : `numpy.array(...)` (même syntaxe que le namespace).

### Fichier local

```bash
catnip -m ./mon_module.py script.cat
```

Charge un fichier Python local.

## Documentation

- [EXTENDING_CONTEXT](../user/EXTENDING_CONTEXT.md) - Guide complet d'extension du contexte
