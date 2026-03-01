# Geometry

Algorithmes géométriques 2D (planaires) pour nuages de points, polylignes et polygones.

## Pourquoi cette catégorie

Les cas "géospatial" réels mélangent souvent deux couches :

- géodésie (sphère/ellipsoïde : haversine, projections)
- géométrie plane (enveloppe convexe, intersections, simplification)

Ici on isole la partie géométrie pure pour des algorithmes 2D robustes et réutilisables.

## Exemples

| Fichier                                                  | Module | Description                                                  |
| -------------------------------------------------------- | ------ | ------------------------------------------------------------ |
| [`convex_hull.cat`](convex_hull.cat)                     | list   | Enveloppe convexe 2D avec scan de Graham (O(n log n))        |
| [`quickhull.cat`](quickhull.cat)                         | list   | Enveloppe convexe 2D par divide & conquer (O(n log n) moyen) |
| [`rdp_simplification.cat`](rdp_simplification.cat)       | list   | Simplification de polyligne (Ramer-Douglas-Peucker)          |
| [`point_in_polygon_bbox.cat`](point_in_polygon_bbox.cat) | list   | Point-in-Polygon avec préfiltre bbox                         |

## Progression suggérée

1. **convex_hull** - Scan de Graham (tri polaire + scan linéaire)
1. **quickhull** - Divide & conquer (partition récursive)
1. **rdp_simplification** - Simplification de polylignes (Ramer-Douglas-Peucker)
1. **point_in_polygon_bbox** - Inclusion point-polygone avec préfiltre bbox

## Exécution

```bash
catnip geometry/convex_hull.cat
```
