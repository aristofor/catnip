# Intégration avec l'Application Hôte

Ce guide montre comment intégrer Catnip dans une application Python et exposer proprement des services de l'hôte
(cache, logging, exceptions, etc.) aux scripts Catnip.

## Philosophie

Catnip est conçu pour être **embarqué** dans des applications Python. L'hôte peut exposer ses services (Redis, logging,
etc.) via le `Context` pour que les scripts Catnip les utilisent de manière contrôlée.

> Expose l'utile, garde le sas fermé.

## Mise en Cache Pluggable

### Interface de Cache

Le système de cache de Catnip est **pluggable** : l'hôte peut fournir son propre backend (Redis, diskcache, memcached,
etc.).

#### Protocole de Cache

Le backend doit implémenter le protocole `CacheBackend` (voir `catnip/cachesys/base.py`) :

```python
from typing import Protocol, Optional
from catnip.cachesys import CacheBackend
from catnip._rs import CacheEntry, CacheKey

class CacheBackend(Protocol):
    def get(self, key: CacheKey) -> Optional[CacheEntry]: ...
    def set(self, key: CacheKey, value, metadata: dict | None = None) -> None: ...
    def delete(self, key: CacheKey) -> bool: ...
    def clear(self) -> None: ...
    def exists(self, key: CacheKey) -> bool: ...
    def stats(self) -> dict: ...
```

### Implémentation avec Redis

```python
import redis
from catnip import Catnip
from catnip.cachesys import CatnipCache
from catnip._rs import CacheEntry, CacheKey

class RedisCache:
    """Backend de cache utilisant Redis."""

    def __init__(self, client: redis.Redis, prefix: str = "catnip:"):
        self.client = client
        self.prefix = prefix

    def _make_key(self, key: CacheKey) -> str:
        return f"{self.prefix}{key.signature}"

    def get(self, key: CacheKey) -> CacheEntry | None:
        value = self.client.get(self._make_key(key))
        return CacheEntry(value=value, metadata={}) if value else None

    def set(self, key: CacheKey, value, metadata: dict | None = None) -> None:
        redis_key = self._make_key(key)
        self.client.set(redis_key, value)

    def delete(self, key: CacheKey) -> bool:
        return bool(self.client.delete(self._make_key(key)))

    def clear(self) -> None:
        keys = self.client.keys(f"{self.prefix}*")
        if keys:
            self.client.delete(*keys)

    def exists(self, key: CacheKey) -> bool:
        return bool(self.client.exists(self._make_key(key)))

    def stats(self) -> dict:
        return {"backend": "redis"}

# Utilisation
redis_client = redis.Redis(host='localhost', port=6379, db=0)
backend = RedisCache(redis_client, prefix="myapp:catnip:")

catnip = Catnip(cache=CatnipCache(backend=backend))

# Le cache est utilisé automatiquement
result = catnip.eval("2 + 2")
```

### Implémentation avec diskcache

```python
import diskcache
from catnip import Catnip
from catnip.cachesys import CatnipCache
from catnip._rs import CacheEntry, CacheKey

class DiskCache:
    """Backend de cache utilisant diskcache."""

    def __init__(self, directory: str):
        self.cache = diskcache.Cache(directory)

    def get(self, key: CacheKey) -> CacheEntry | None:
        value = self.cache.get(key.signature)
        return CacheEntry(value=value, metadata={}) if value else None

    def set(self, key: CacheKey, value, metadata: dict | None = None) -> None:
        self.cache.set(key.signature, value)

    def delete(self, key: CacheKey) -> bool:
        return bool(self.cache.delete(key.signature))

    def clear(self) -> None:
        self.cache.clear()

    def exists(self, key: CacheKey) -> bool:
        return key.signature in self.cache

    def stats(self) -> dict:
        return {"backend": "diskcache"}

# Utilisation
backend = DiskCache("/tmp/catnip_cache")
catnip = Catnip(cache=CatnipCache(backend=backend))
```

### Signature de Cache

Catnip gère les clés de cache automatiquement (xxHash64 sur le source + options).
L'hôte n'a pas besoin de calculer ou manipuler les signatures.

### Cache avec TTL personnalisé

```python
from catnip import Catnip
from catnip.cachesys import CatnipCache, CacheBackend
from catnip._rs import CacheEntry, CacheKey

class CacheWithTTL:
    """Backend wrapper avec TTL par défaut configurable."""

    def __init__(self, backend: CacheBackend, default_ttl: int = 3600):
        self.backend = backend
        self.default_ttl = default_ttl

    def get(self, key: CacheKey) -> CacheEntry | None:
        return self.backend.get(key)

    def set(self, key: CacheKey, value, metadata: dict | None = None) -> None:
        metadata = metadata or {}
        metadata.setdefault("ttl_seconds", self.default_ttl)
        self.backend.set(key, value, metadata)

    def delete(self, key: CacheKey) -> bool:
        return self.backend.delete(key)

    def clear(self) -> None:
        self.backend.clear()

    def exists(self, key: CacheKey) -> bool:
        return self.backend.exists(key)

    def stats(self) -> dict:
        return self.backend.stats()

# Cache avec expiration après 1 heure
backend = CacheWithTTL(RedisCache(redis_client), default_ttl=3600)
catnip = Catnip(cache=CatnipCache(backend=backend))
```

## Logging Personnalisé

### Interface de Logging

L'hôte peut fournir un logger personnalisé pour intercepter les logs de Catnip :

```python
import logging
from catnip import Catnip

# Logger personnalisé avec format JSON
class JSONLogger:
    def __init__(self, name: str):
        self.logger = logging.getLogger(name)
        handler = logging.StreamHandler()
        handler.setFormatter(logging.Formatter('{"level": "%(levelname)s", "msg": "%(message)s"}'))
        self.logger.addHandler(handler)
        self.logger.setLevel(logging.INFO)

    def info(self, msg: str) -> None:
        self.logger.info(msg)

    def warning(self, msg: str) -> None:
        self.logger.warning(msg)

    def error(self, msg: str) -> None:
        self.logger.error(msg)

    def print(self, *args, sep=' ') -> None:
        msg = sep.join(str(arg) for arg in args)
        self.logger.debug(msg)

# Utilisation
from catnip.context import Context

logger = JSONLogger("myapp.catnip")
ctx = Context(logger=logger)
catnip = Catnip(context=ctx)

# Le logger est accessible depuis Catnip
catnip.parse('logger.debug("x =", 42)')
catnip.execute()
```

### Logging avec Contexte

```python
class ContextualLogger:
    """Logger qui ajoute automatiquement du contexte (user_id, request_id, etc.)."""

    def __init__(self, base_logger, context: dict):
        self.base = base_logger
        self.context = context

    def _format(self, msg: str) -> str:
        ctx = " ".join(f"{k}={v}" for k, v in self.context.items())
        return f"[{ctx}] {msg}"

    def print(self, *args, sep=' ') -> None:
        msg = sep.join(str(arg) for arg in args)
        self.base.debug(self._format(msg))

    def info(self, *args, sep=' ') -> None:
        msg = sep.join(str(arg) for arg in args)
        self.base.info(self._format(msg))

    def error(self, *args, sep=' ') -> None:
        msg = sep.join(str(arg) for arg in args)
        self.base.error(self._format(msg))

    # … autres méthodes (warning, etc.)

# Utilisation
from catnip.context import Context

base_logger = logging.getLogger("catnip")
logger = ContextualLogger(base_logger, {"user_id": 123, "request_id": "abc"})
ctx = Context(logger=logger)
catnip = Catnip(context=ctx)

# Logs: "[user_id=123 request_id=abc] Parsing script.cat"
```

## Remontée d'Exceptions

### Gestionnaire d'Exceptions Personnalisé

L'hôte peut intercepter les exceptions Catnip pour logging, monitoring, ou traitement personnalisé :

```python
from catnip import Catnip
from catnip.exceptions import CatnipError, ParseError, RuntimeError

class ExceptionHandler:
    """Gestionnaire d'exceptions avec reporting vers un service externe."""

    def __init__(self, sentry_client=None):
        self.sentry = sentry_client

    def handle(self, exc: Exception, context: dict) -> None:
        """Gère une exception levée par Catnip.

        Args:
            exc: L'exception levée
            context: Contexte d'exécution (script, ligne, etc.)
        """
        if isinstance(exc, ParseError):
            self._handle_parse_error(exc, context)
        elif isinstance(exc, RuntimeError):
            self._handle_runtime_error(exc, context)
        else:
            self._handle_generic_error(exc, context)

    def _handle_parse_error(self, exc: ParseError, context: dict) -> None:
        # Log l'erreur de parsing
        print(f"Parse error in {context.get('script')}: {exc}")

        # Envoie à Sentry si configuré
        if self.sentry:
            self.sentry.capture_exception(exc)

    def _handle_runtime_error(self, exc: RuntimeError, context: dict) -> None:
        # Erreur d'exécution - monitoring critique
        print(f"Runtime error: {exc}")

        # Alerte l'équipe
        if self.sentry:
            self.sentry.capture_exception(exc, level="error")

    def _handle_generic_error(self, exc: Exception, context: dict) -> None:
        # Erreur inattendue
        print(f"Unexpected error: {exc}")
        if self.sentry:
            self.sentry.capture_exception(exc, level="critical")

# Utilisation
handler = ExceptionHandler(sentry_client=my_sentry_client)
catnip = Catnip()
catnip.set_exception_handler(handler)

try:
    catnip.eval("invalid syntax !")
except Exception as e:
    # Le handler a déjà été appelé automatiquement
    pass
```

### Exceptions avec Recovery

```python
class RecoveryHandler:
    """Gestionnaire avec tentatives de recovery automatique."""

    def __init__(self, max_retries: int = 3):
        self.max_retries = max_retries

    def handle(self, exc: Exception, context: dict) -> bool:
        """Retourne True si le recovery a réussi, False sinon."""

        if isinstance(exc, TimeoutError) and context.get("retry_count", 0) < self.max_retries:
            # Retry automatique pour les timeouts
            print(f"Timeout, retry {context['retry_count'] + 1}/{self.max_retries}")
            return True  # Signal pour retry

        # Pas de recovery possible
        return False

# Utilisation
handler = RecoveryHandler(max_retries=3)
catnip = Catnip()
catnip.set_exception_handler(handler)
```

## Exposition de Services Hôte

### Injection de Services dans le Context

L'hôte peut injecter ses services directement dans le `Context` Catnip :

```python
from catnip import Catnip

# Services de l'hôte
class HostServices:
    def __init__(self, redis_client, db_pool, logger):
        self.redis = redis_client
        self.db = db_pool
        self.logger = logger

    def cache_get(self, key: str):
        """Service de cache exposé aux scripts."""
        return self.redis.get(key)

    def cache_set(self, key: str, value, ttl: int = 3600):
        """Stockage dans le cache."""
        self.redis.setex(key, ttl, value)

    def db_query(self, sql: str):
        """Exécute une requête SQL."""
        with self.db.get_connection() as conn:
            return conn.execute(sql).fetchall()

    def log(self, msg: str):
        """Log un message."""
        self.logger.info(f"[Catnip Script] {msg}")

# Configuration
services = HostServices(redis_client, db_pool, logger)

catnip = Catnip()
catnip.context.set("host", services)

# Les scripts Catnip ont accès aux services via `host`
result = catnip.eval("""
host.log("Script started")
data = host.db_query("SELECT * FROM users")
host.cache_set("users", data, 3600)
host.log("Script finished")
""")
```

### Exemple Complet : Application Web

```python
from flask import Flask, request
from catnip import Catnip
import redis
import logging

app = Flask(__name__)

# Configuration des services
redis_client = redis.Redis(host='localhost', port=6379, db=0)
logger = logging.getLogger("myapp.catnip")

class WebAppServices:
    """Services exposés aux scripts Catnip."""

    def __init__(self, redis_client, logger):
        self.redis = redis_client
        self.logger = logger

    def get_user(self, user_id: int):
        """Récupère un utilisateur depuis Redis."""
        data = self.redis.get(f"user:{user_id}")
        return data.decode() if data else None

    def log_event(self, event: str, data: dict):
        """Log un événement métier."""
        self.logger.info(f"Event: {event}", extra=data)

    def send_notification(self, user_id: int, message: str):
        """Envoie une notification."""
        # Implémentation réelle…
        self.logger.info(f"Notification sent to {user_id}: {message}")

# Configuration Catnip
services = WebAppServices(redis_client, logger)
backend = RedisCache(redis_client, prefix="catnip:scripts:")
catnip = Catnip(cache=CatnipCache(backend=backend))
catnip.context.set("app", services)

@app.route("/run-script", methods=["POST"])
def run_script():
    """Endpoint pour exécuter un script Catnip."""
    script = request.json.get("script")

    try:
        # Exécution du script avec accès aux services
        result = catnip.eval(script)
        return {"status": "success", "result": result}

    except Exception as e:
        logger.error(f"Script execution failed: {e}")
        return {"status": "error", "message": str(e)}, 500

# Exemple de script client:
"""
user = app.get_user(123)
app.log_event("user_accessed", {"user_id": 123})
app.send_notification(123, "Welcome back!")
"""
```

## Résumé

### Services Pluggables

| Service        | Interface                | Exemples d'implémentation          |
| -------------- | ------------------------ | ---------------------------------- |
| **Cache**      | `CacheBackend`           | Redis, diskcache, memcached        |
| **Logging**    | `Logger`                 | JSON logger, Syslog, CloudWatch    |
| **Exceptions** | `ExceptionHandler`       | Sentry, Rollbar, monitoring custom |
| **Services**   | Injection dans `Context` | DB, API, notifications, etc.       |

### Checklist d'Intégration

- [ ] Définir un gestionnaire d'exceptions pour monitoring
- [ ] Exposer les services hôte via `context.set("host", services)`
- [ ] Configurer les TTL de cache selon les besoins
- [ ] Tester l'intégration avec des scripts de validation
- [ ] Monitorer les performances du cache (hit rate, latence)

### Exemple Minimal - certifié sans branchements

```python
from catnip import Catnip
import redis

# Setup cache
redis_client = redis.Redis()
backend = RedisCache(redis_client, prefix="myapp:")

# Setup Catnip
catnip = Catnip(cache=CatnipCache(backend=backend))

# Exécution avec cache automatique
result = catnip.eval("x = 2 + 2")  # Compilé et mis en cache
result = catnip.eval("x = 2 + 2")  # Récupéré depuis le cache
```
