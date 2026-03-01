# Data & Analytics

Analyse de données avec les bibliothèques Python de référence.

## Pourquoi cette catégorie

NumPy, DuckDB, pandas : l'écosystème data science Python est incontournable. Catnip s'intègre sans friction : les arrays
NumPy, les DataFrames, les connexions SQL fonctionnent directement.

Le broadcasting Catnip (`.[op]`) complète NumPy : même idiome pour les listes Python et les arrays NumPy.

## Exemples

| Fichier                                                    | Module              | Description                                           |
| ---------------------------------------------------------- | ------------------- | ----------------------------------------------------- |
| [`numpy_climate.cat`](numpy_climate.cat)                   | numpy               | Analyse climatique, statistiques, vectorisation       |
| [`duckdb_analytics.cat`](duckdb_analytics.cat)             | duckdb              | SQL analytique in-process, agrégations                |
| [`sqlalchemy_duckdb.cat`](sqlalchemy_duckdb.cat)           | sqlalchemy + duckdb | ORM complet, requêtes analytiques                     |
| [`polars_parallel_ingest.cat`](polars_parallel_ingest.cat) | polars              | Ingestion parallèle de CSV avec ND-map (eager + lazy) |

## Données

- `nasa_temperature.csv` - Anomalies de température globale (source NASA GISS)

## Exécution

```bash
# NumPy
catnip -m numpy data-analytics/numpy_climate.cat

# DuckDB
catnip -m duckdb data-analytics/duckdb_analytics.cat

# SQLAlchemy + DuckDB
catnip -m sqlalchemy -m duckdb data-analytics/sqlalchemy_duckdb.cat

# Polars (ND-map parallel)
catnip -m polars -m tempfile -m pathlib data-analytics/polars_parallel_ingest.cat
```
