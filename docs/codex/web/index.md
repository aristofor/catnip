# Web & HTTP

Requêtes HTTP et interaction avec des APIs.

## Pourquoi cette catégorie

Le web est une source de données universelle. httpx offre un client HTTP moderne (sync/async, HTTP/2) qui s'utilise
directement depuis Catnip.

Ces exemples montrent le pattern fetch → parse → process sans quitter Catnip.

## Exemples

| Fichier                                              | Module            | Description                              |
| ---------------------------------------------------- | ----------------- | ---------------------------------------- |
| [`httpx_requests.cat`](httpx_requests.cat)           | httpx             | GET, POST, headers, JSON, timeout        |
| [`selectolax_scraping.cat`](selectolax_scraping.cat) | selectolax.parser | Parsing HTML, sélecteurs CSS, extraction |

## Exécution

```bash
catnip docs/codex/web/httpx_requests.cat
catnip docs/codex/web/selectolax_scraping.cat
```
