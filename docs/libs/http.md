# Module `http`

Client HTTP et serveur léger.

- **Client** : `ureq 3` (sync, blocking, TLS via rustls)
- **Serveur** : `tiny_http` (sync, single-request ou async via thread + channel)

<!-- check: no-check -->
```catnip
import('http')

response = http.get("https://example.com")
print(response.status)  # 200
```

> Le client et le serveur sont indépendants. Tu peux les utiliser séparément ou ensemble (par exemple un proxy qui
> reçoit côté serveur puis re-envoie côté client).

## Constants

| Attribut        | Valeur            | Description              |
| --------------- | ----------------- | ------------------------ |
| `http.PROTOCOL` | `"rust"`          | Marqueur module natif    |
| `http.VERSION`  | `"0.2.0"`         | Version semver du module |
| `http.Request`  | `"http.Request"`  | Type marker (réservé)    |
| `http.Response` | `"http.Response"` | Type marker (réservé)    |

## Client HTTP

### Verbes basiques

<!-- check: no-check -->
```catnip
http.get(url)             # → Response
http.post(url, body)      # → Response ; body est optionnel
http.put(url, body)       # → Response ; body est optionnel
http.delete(url)          # → Response
```

Exemples :

<!-- check: no-check -->
```catnip
# ⇒ GET simple
r = http.get("https://httpbin.org/get")
print(r.status)           # 200
print(r.body)             # "{ ... }"

# ⇒ POST avec body
r = http.post("https://httpbin.org/post", "name=cat")
print(r.status)           # 200
```

Les statuses 4xx et 5xx remontent comme `Response` (pas comme exception). Les erreurs réseau, URL invalides ou timeouts
lèvent une exception Catnip.

### `request()` avec options

<!-- check: no-check -->
```catnip
http.request(method, url, opts)
```

`method` et `url` sont des strings, `opts` est un dict ou `nil`.

| Clé        | Type             | Défaut     | Description                                          |
| ---------- | ---------------- | ---------- | ---------------------------------------------------- |
| `headers`  | `dict[str, str]` | `{}`       | Headers à envoyer                                    |
| `body`     | `str`            | `""`       | Body de la requête (string)                          |
| `timeout`  | `float`          | aucun      | Timeout global en secondes                           |
| `max_body` | `int`            | `33554432` | Limite de lecture du body de réponse (bytes ; 32 MB) |

<!-- check: no-check -->
```catnip
r = http.request("POST", "https://api.example.com/items", dict(
    headers=dict(("Content-Type", "application/json")),
    body='{"name": "cat"}',
    timeout=5.0,
))
```

> Un body de réponse > `max_body` produit une erreur de lecture explicite, pas un body tronqué silencieux. Avant la
> 0.0.9 c'était l'inverse, et c'était dangereux.

### Object `Response`

| Attribut  | Type             | Description                         |
| --------- | ---------------- | ----------------------------------- |
| `status`  | `int`            | Code HTTP (200, 404, etc.)          |
| `headers` | `dict[str, str]` | Headers reçus (noms lowercase)      |
| `body`    | `str`            | Body lu intégralement (UTF-8 lossy) |

| Méthode   | Retour | Description                                           |
| --------- | ------ | ----------------------------------------------------- |
| `.json()` | `any`  | Parse `body` comme JSON. Lève en cas de JSON invalide |

<!-- check: no-check -->
```catnip
r = http.get("https://api.github.com/repos/anthropics/claude-code")
data = r.json()
print(data['stargazers_count'])
```

Le parser JSON préserve la précision : entiers > 2^46 deviennent BigInt, `u64::MAX` reste exact. Les floats restent
floats, `null` devient `nil`.

## Serveur HTTP

### Mode synchrone (single-thread)

<!-- check: no-check -->
```catnip
server = http.Server("127.0.0.1:8080")

while (true) {
    req = server.recv()        # bloquant
    if (req == nil) { break }  # close()
    req.respond("Hello", 200, "text/plain")
}
```

Méthodes du `Server` :

| Méthode                  | Retour    | Description                                        |
| ------------------------ | --------- | -------------------------------------------------- |
| `.recv()`                | \`Request | nil\`                                              |
| `.try_recv()`            | \`Request | nil\`                                              |
| `.recv_timeout(seconds)` | \`Request | nil\`                                              |
| `.close()`               | `nil`     | Arrête `recv()`. Joint le thread async si démarré  |
| `.addr`                  | `str`     | Adresse réelle (`"127.0.0.1:42587"` pour port `0`) |

> Le mode `try_recv` ne lance pas de thread accept en arrière-plan : il interroge la queue interne de `tiny_http`. Si tu
> veux un vrai event loop sans bloquer, prends le mode async ci-dessous.

### Mode asynchrone (channel-based)

`start()` lance un thread accept qui drain les requêtes dans un channel mpsc. `recv_async()` pop sans bloquer.

<!-- check: no-check -->
```catnip
server = http.Server("127.0.0.1:0")  # port 0 = OS choisit
server.start()

# Event loop principal
while (running) {
    req = server.recv_async()
    if (req != nil) {
        handle(req)
    }
    do_other_work()
}

server.close()  # join le thread proprement
```

| Méthode         | Retour    | Description                          |
| --------------- | --------- | ------------------------------------ |
| `.start()`      | `nil`     | Démarre le thread accept. Idempotent |
| `.recv_async()` | \`Request | nil\`                                |

Si tu laisses le `Server` sortir de scope sans appeler `close()`, le thread est unblock et joint automatiquement. Le
port est libéré aussi.

> En pratique, appelle `close()` explicitement quand tu sais que tu as fini : ça rend les fins de programme plus
> prévisibles que de compter sur le GC.

### Object `Request`

Attributs :

| Attribut  | Type             | Description                               |
| --------- | ---------------- | ----------------------------------------- |
| `url`     | `str`            | URL path (`"/foo?bar=1"`)                 |
| `method`  | `str`            | `"GET"`, `"POST"`, etc.                   |
| `headers` | `dict[str, str]` | Headers entrants (noms lowercase)         |
| `cookies` | `dict[str, str]` | Cookies parsés depuis le header `Cookie:` |

Méthodes :

| Méthode                      | Retour       | Description                                  |
| ---------------------------- | ------------ | -------------------------------------------- |
| `.body()`                    | `str`        | Lit le body brut comme string                |
| `.multipart()`               | `list[dict]` | Parse `multipart/form-data`. Voir ci-dessous |
| `.respond(body, status, ct)` | `nil`        | Envoie une réponse simple                    |
| `.start_chunked(status, ct)` | `Chunked`    | Démarre une réponse chunked                  |
| `.start_sse()`               | `Chunked`    | Démarre une réponse SSE (text/event-stream)  |

`respond()`, `start_chunked()` et `start_sse()` consomment la requête (utilisable une seule fois).

### Multipart

Pour parser un upload `multipart/form-data` côté serveur :

<!-- check: no-check -->
```catnip
req = server.recv()
parts = req.multipart()

for part in parts {
    print(part['name'], part['filename'], part['content_type'])
    # part['data'] est bytes (préserve le binaire)
}
```

Chaque part contient :

| Clé            | Type    | Description                          |
| -------------- | ------- | ------------------------------------ |
| `name`         | `str`   | Nom du champ (`Content-Disposition`) |
| `filename`     | \`str   | nil\`                                |
| `content_type` | \`str   | nil\`                                |
| `data`         | `bytes` | Contenu brut du part                 |

Le parser respecte RFC 7578 : boundary ancré sur les delimiter lines (pas de split sur des bytes intérieurs au payload),
noms d'headers et paramètres case-insensitive.

> Pas de version client (envoyer du multipart) pour l'instant. C'est faisable manuellement en construisant le body
>
> - le `Content-Type: multipart/form-data; boundary=...` à la main, mais une API dédiée viendra si besoin.

### Cookies

`req.cookies` est un dict `{ name: value }` parsé depuis le header `Cookie:`. Plusieurs headers `Cookie:` sont
fusionnés.

<!-- check: no-check -->
```catnip
req = server.recv()
session_id = req.cookies['session']
```

Pas de gestion des attributs (path, domain, expires) côté lecture : c'est juste le format envoyé par le client. Pour
**envoyer** un cookie en réponse, ajoute manuellement le header `Set-Cookie` via les helpers de `respond()` ou
`start_chunked()`.

## Streaming (`Chunked`)

`start_chunked()` et `start_sse()` retournent un `Chunked` writer pour les réponses streamées (chunked transfer encoding
HTTP/1.1).

<!-- check: no-check -->
```catnip
req = server.recv()
stream = req.start_chunked(200, "text/plain")
stream.send_chunk("Hello ")
stream.send_chunk("World")
stream.end()
```

| Méthode                          | Retour | Description                                                |
| -------------------------------- | ------ | ---------------------------------------------------------- |
| `.send_chunk(data)`              | `nil`  | Envoie un chunk. Les chunks vides sont ignorés             |
| `.send_event(data, event_type?)` | `nil`  | Envoie un event SSE. Multi-lignes split en `data:` séparés |
| `.end()`                         | `nil`  | Envoie le terminator. Auto-appelé sur drop                 |

### Server-Sent Events

<!-- check: no-check -->
```catnip
req = server.recv()
stream = req.start_sse()

stream.send_event("hello")
# Wire: "data: hello\n\n"

stream.send_event('{"x": 1}', "update")
# Wire: "event: update\ndata: {\"x\": 1}\n\n"

stream.send_event("line1\nline2")
# Wire: "data: line1\ndata: line2\n\n"

stream.end()
```

### Refus du streaming

`start_chunked()` et `start_sse()` lèvent une erreur si la combinaison requête/status est protocole-invalide :

- **HEAD** : ne doit pas avoir de body — utilise `respond()` avec body vide
- **HTTP/1.0** : pas de chunked encoding — utilise `respond()`
- **Status 1xx, 204, 304** : interdit d'avoir un body (RFC 7230 §3.3)

Le code applicatif peut alors retomber sur `respond()` pour ces cas particuliers.

## Auth helpers

<!-- check: no-check -->
```catnip
http.basic_auth(user, password)   # → "Basic <base64(user:password)>"
http.bearer(token)                # → "Bearer <token>"
```

À utiliser dans `opts.headers.Authorization` :

<!-- check: no-check -->
```catnip
r = http.request("GET", "https://api.example.com/me", dict(
    headers=dict(("Authorization", http.bearer("abc123"))),
))
```

## `serve()` : helper one-shot

Sert un contenu statique, ouvre le navigateur, attend une requête, répond, retourne.

<!-- check: no-check -->
```catnip
http.serve("<h1>Hello</h1>", 0, nil, true)
# port 0 → OS choisit
# content_type nil → auto-détecté (text/html, image/svg+xml, text/plain)
# open_browser true → ouvre le navigateur sur l'URL
```

Pratique pour debug, preview, ou afficher un graphe SVG. Pas adapté à un vrai serveur (single-request).

## Limitations

- Pas de HTTP/2 (ureq sync supporte uniquement HTTP/1.1)
- Pas de WebSocket
- Pas de multipart côté client
- Body de réponse buffered en mémoire (max 32 MB par défaut, override via `max_body`)
- Le serveur traite une requête à la fois ; pour le concurrent, lancer plusieurs `Server` ou utiliser le mode async +
  thread pool côté application

> Le périmètre du module reste "pratique pour scripts et exemples". Pour un serveur HTTP production il vaut mieux sortir
> du runtime Catnip et utiliser un crate Rust dédié (axum, actix) -- ou intégrer Catnip comme handler à l'intérieur.
