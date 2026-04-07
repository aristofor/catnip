# Catnip language

Version <!-- catnip:version -->0.0.8<!-- /catnip:version -->

Documentation: [https://docs.catnip-lang.io](https://docs.catnip-lang.io)

Catnip est un langage de script embarqué pour applications Python. Il permet de définir des règles métier, des workflows
et des transformations de données sans modifier le code applicatif.

## Installation

```bash
pip install catnip-lang
```

## Utilisation rapide

### Exécution en CLI

```bash
catnip script.cat
catnip -c "2 + 3"
echo "2 + 3" | catnip
```

### Intégration Python (embedded)

```python
from catnip import Catnip

engine = Catnip()
engine.parse("x = 2 + 3")
result = engine.execute()
print(result)
```

## Cas d'usage typiques

- Règles métier configurables sans redéploiement
- Scripts utilisateur sandboxés dans une application web
- Pipelines de transformation de données (ETL)
- Validation de configuration et logique d'éligibilité

## Statut du projet

Catnip est utilisé en production pour des cas d'expressions et de transformation de données. Le projet est encore en
phase `0.x`, avec des évolutions en cours sur certains aspects.

## Documentation

- [Guide utilisateur](docs/user/)
- [Référence du langage](docs/lang/LANGUAGE.md)
- [Exemples d'embedding](docs/examples/embedding/)
- [Documentation développeur](docs/dev/)
- [Introduction](docs/INTRODUCTION.md)

## Code source

- Framagit (principal): https://framagit.org/aristofor/catnip
- GitHub (miroir): https://github.com/aristofor/catnip

## Licence

GPLv3
