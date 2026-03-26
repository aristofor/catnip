# Geospatial

Analyse et manipulation de données spatiales.

## Pourquoi cette catégorie

Les coordonnées GPS sont des paires de nombres. La distance entre deux points est une fonction pure. Appliquer cette
fonction à une liste de points, c'est du broadcasting. Pas besoin de GIS pour faire de la géo utile.

Cas d'usage : calcul de distances, rayon de recherche, matrices de distances, traitement raster, change detection. Les
algorithmes géométriques planaires (enveloppe convexe, simplification, PIP 2D) sont rangés dans `geometry/`.

## Exemples

| Fichier                                                                  | Module                                          | Description                                                        |
| ------------------------------------------------------------------------ | ----------------------------------------------- | ------------------------------------------------------------------ |
| [`haversine_distance.cat`](haversine_distance.cat)                       | math                                            | Distance orthodromique, broadcasting, matrice, trajet              |
| [`rasterio_change_detection.cat`](rasterio_change_detection.cat)         | rasterio, numpy                                 | Raster, NDVI, change detection, surface (synthétique)              |
| [`sentinel2_deforestation.cat`](sentinel2_deforestation.cat)             | rasterio, numpy, pystac-client, shapely, pyproj | Déforestation sur données réelles (STAC, COG)                      |
| [`geopandas_folium_street_trees.cat`](geopandas_folium_street_trees.cat) | geopandas, osmnx, folium, branca, orjson        | Arbres publics + voirie OSM, score de végétalisation, carte folium |

## Exécution

```bash
catnip docs/codex/geospatial/haversine_distance.cat
catnip docs/codex/geospatial/rasterio_change_detection.cat
catnip docs/codex/geospatial/sentinel2_deforestation.cat    # internet requis
catnip docs/codex/geospatial/geopandas_folium_street_trees.cat # internet requis
```
