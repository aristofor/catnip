# Geospatial

Analyse et manipulation de données spatiales.

## Pourquoi cette catégorie

Les coordonnées GPS sont des paires de nombres. La distance entre deux points est une fonction pure. Appliquer cette
fonction à une liste de points, c'est du broadcasting. Pas besoin de GIS pour faire de la géo utile.

Cas d'usage : calcul de distances, rayon de recherche, matrices de distances, traitement raster, change detection. Les
algorithmes géométriques planaires (enveloppe convexe, simplification, PIP 2D) sont rangés dans `geometry/`.

## Exemples

| Fichier                                                          | Module                                          | Description                                           |
| ---------------------------------------------------------------- | ----------------------------------------------- | ----------------------------------------------------- |
| [`haversine_distance.cat`](haversine_distance.cat)               | math                                            | Distance orthodromique, broadcasting, matrice, trajet |
| [`rasterio_change_detection.cat`](rasterio_change_detection.cat) | rasterio, numpy                                 | Raster, NDVI, change detection, surface (synthétique) |
| [`sentinel2_deforestation.cat`](sentinel2_deforestation.cat)     | rasterio, numpy, pystac-client, shapely, pyproj | Déforestation sur données réelles (STAC, COG)         |

## Progression suggérée

1. `haversine_distance` -- coordonnées, distances, fonctions pures
1. `rasterio_change_detection` -- raster, NDVI, change detection, surface (données synthétiques)
1. `sentinel2_deforestation` -- pipeline complet sur données réelles Sentinel-2 (internet requis)

## Exécution

```bash
catnip docs/codex/geospatial/haversine_distance.cat
catnip docs/codex/geospatial/rasterio_change_detection.cat
catnip docs/codex/geospatial/sentinel2_deforestation.cat    # internet requis
```
