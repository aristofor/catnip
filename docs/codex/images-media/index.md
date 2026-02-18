# Images & Media

Manipulation d'images et fichiers multimédia.

## Pourquoi cette catégorie

Pillow (PIL) est la référence pour le traitement d'images en Python. Catnip peut l'utiliser pour des transformations,
filtres, conversions de format.

Cas d'usage : preprocessing pour ML, génération de thumbnails, batch processing.

## Exemples

| Fichier                                          | Module       | Description                                             |
| ------------------------------------------------ | ------------ | ------------------------------------------------------- |
| [`pillow_transforms.cat`](pillow_transforms.cat) | PIL (Pillow) | Transformations, filtres, conversions, batch processing |

## Exécution

```bash
catnip -m PIL.Image -m PIL.ImageFilter -m PIL.ImageDraw -m PIL.ImageStat docs/codex/images-media/pillow_transforms.cat
```

## Opérations Couvertes

**Transformations géométriques** :

- Resize, thumbnail (préserve ratio)
- Rotation, crop
- Flip horizontal/vertical

**Filtres et effets** :

- Blur, sharpen
- Contour, edge enhance

**Conversions** :

- RGB ↔ Grayscale
- Changement de format (JPG, PNG)
- Transparence (RGBA)

**Manipulation avancée** :

- Accès/modification de pixels
- Ajout de texte et formes
- Statistiques d'image (mean, median, stddev)
- Traitement par lot avec broadcasting Catnip
