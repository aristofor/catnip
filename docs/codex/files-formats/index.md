# Fichiers & Formats

Manipulation de fichiers et parsing de formats structurés.

## Pourquoi cette catégorie

L'écosystème Python excelle dans le parsing de formats. Catnip en profite via `import(...)` sans réinventer la roue.

Ces exemples montrent que Catnip consomme directement les APIs Python : pas de wrapper, pas de conversion, juste
l'appel.

## Exemples

| Fichier                                      | Module    | Description                                   |
| -------------------------------------------- | --------- | --------------------------------------------- |
| [`regex_patterns.cat`](regex_patterns.cat)   | re        | Expressions régulières, groupes, substitution |
| [`yaml_config.cat`](yaml_config.cat)         | pyyaml    | Parsing et écriture YAML                      |
| [`xml_parsing.cat`](xml_parsing.cat)         | xml.etree | Parsing XML, navigation, modification         |
| [`jmespath_query.cat`](jmespath_query.cat)   | jmespath  | Requêtage JSON déclaratif                     |
| [`parquet_pyarrow.cat`](parquet_pyarrow.cat) | pyarrow   | Format columnar pour analytics                |

## Exécution

```bash
# Exemple avec regex (stdlib)
catnip docs/codex/files-formats/regex_patterns.cat

# Exemple avec yaml (lib externe)
catnip docs/codex/files-formats/yaml_config.cat
```

## Progression suggérée

1. **regex** - Parsing de texte non structuré
1. **yaml** - Format de config
1. **xml** - Format structuré omniprésent
1. **jmespath** - Requêtage avancé sur JSON
1. **parquet** - Big data et analytics
