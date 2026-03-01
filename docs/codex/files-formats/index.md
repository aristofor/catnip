# Fichiers & Formats

Manipulation de fichiers et parsing de formats structurés.

## Pourquoi cette catégorie

L'écosystème Python excelle dans le parsing de formats. Catnip en profite via `import(...)` sans réinventer la roue.

Ces exemples montrent que Catnip consomme directement les APIs Python : pas de wrapper, pas de conversion, juste
l'appel.

## Exemples

| Fichier                                      | Module    | Description                                   |
| -------------------------------------------- | --------- | --------------------------------------------- |
| [`pathlib_files.cat`](pathlib_files.cat)     | pathlib   | Chemins, glob, lecture/écriture               |
| [`regex_patterns.cat`](regex_patterns.cat)   | re        | Expressions régulières, groupes, substitution |
| [`yaml_config.cat`](yaml_config.cat)         | pyyaml    | Parsing et écriture YAML                      |
| [`json_orjson.cat`](json_orjson.cat)         | orjson    | JSON rapide (bytes natifs, 3-10x stdlib)      |
| [`xml_parsing.cat`](xml_parsing.cat)         | xml.etree | Parsing XML, navigation, modification         |
| [`jmespath_query.cat`](jmespath_query.cat)   | jmespath  | Requêtage JSON déclaratif                     |
| [`parquet_pyarrow.cat`](parquet_pyarrow.cat) | pyarrow   | Format columnar pour analytics                |

## Exécution

```bash
# Exemple avec pathlib (stdlib)
catnip -m pathlib -m tempfile docs/codex/files-formats/pathlib_files.cat

# Exemple avec orjson (lib externe)
catnip -m orjson -m tempfile -m pathlib -m shutil -m datetime docs/codex/files-formats/json_orjson.cat
```

## Progression suggérée

1. **pathlib** - Base pour tous les autres (chemins, I/O)
1. **regex** - Parsing de texte non structuré
1. **yaml/json** - Formats de config
1. **xml** - Format structuré omniprésent
1. **jmespath** - Requêtage avancé sur JSON
1. **parquet** - Big data et analytics
