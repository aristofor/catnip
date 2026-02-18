# Geospatial

Analyse et manipulation de données spatiales.

## Pourquoi cette catégorie

Les coordonnées GPS sont des paires de nombres. La distance entre deux points est une fonction pure. Appliquer cette
fonction à une liste de points, c'est du broadcasting. Pas besoin de GIS pour faire de la géo utile.

Cas d'usage : calcul de distances, rayon de recherche, matrices de distances, optimisation de trajets.
Les algorithmes géométriques planaires (enveloppe convexe, simplification, PIP 2D) sont rangés dans `geometry/`.

## Exemples

| Fichier                                            | Module | Description                                           |
| -------------------------------------------------- | ------ | ----------------------------------------------------- |
| [`haversine_distance.cat`](haversine_distance.cat) | math   | Distance orthodromique, broadcasting, matrice, trajet |

## Exécution

```bash
catnip docs/codex/geospatial/haversine_distance.cat
```
