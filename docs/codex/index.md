# Codex Python

Exemples d'intégration de bibliothèques Python dans Catnip. Python dans Catnip, sans friction.

## Principe

Catnip charge des modules Python via `import('module')` ou l'option CLI `-m module`. Les objets et fonctions du module
deviennent accessibles directement dans le code Catnip.

```catnip
numpy = import('numpy')
arr = numpy.array(list(1, 2, 3))
print(arr.mean())
```

Pas de wrapper, pas de FFI : l'interopérabilité est native.

## Organisation

| Dossier                              | Domaine                        | Exemples                                |
| ------------------------------------ | ------------------------------ | --------------------------------------- |
| [files-formats/](files-formats/)     | Fichiers et formats de données | regex, yaml, xml, jmespath, parquet     |
| [data-analytics/](data-analytics/)   | Analyse de données             | numpy, duckdb, sqlalchemy, polars       |
| [web/](web/)                         | HTTP et APIs                   | selectolax                              |
| [images-media/](images-media/)       | Images et multimédia           | pillow                                  |
| [geospatial/](geospatial/)           | Géospatial                     | haversine, rasterio, sentinel-2, folium |
| [geometry/](geometry/)               | Géométrie                      | enveloppe convexe (Graham, Quickhull)   |
| [visualization/](visualization/)     | Visualisation                  | hvplot                                  |
| [symbolic-graphs/](symbolic-graphs/) | Calcul symbolique & graphes    | networkx                                |
| [utils/](utils/)                     | Utilitaires                    | tqdm                                    |

## Exemples de base

Les exemples minimalistes de chargement de modules sont dans
[`/examples/module-loading/`](../examples/module-loading/index.md).

## Modes de chargement

### Namespace (défaut)

```bash
catnip -m numpy script.cat
```

Le module est accessible via son nom : `numpy.array(...)`.

### Alias dans le code

```catnip
np = import('numpy')
arr = np.array(list(1, 2, 3))
```

Pour un alias, utiliser `import()` dans le code Catnip.

### Fichier local

```bash
catnip -m ./mon_module.py script.cat
```

Charge un fichier Python local.

## Documentation

- [EXTENDING_CONTEXT](../user/EXTENDING_CONTEXT.md) - Guide complet d'extension du contexte
