# Intégration hôte

Ce guide montre comment intégrer Catnip dans une application Python et exposer proprement des services de l'hôte (cache,
logging, exceptions, etc.) aux scripts Catnip.

## Philosophie

Catnip est conçu comme **DSL embarqué** dans des applications Python. L'hôte peut exposer ses services (Redis, logging,
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
catnip.parse("2 + 2")
result = catnip.execute()
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

Catnip gère les clés de cache automatiquement (xxHash64 sur le source + options). L'hôte n'a pas besoin de calculer ou
manipuler les signatures.

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

# Le logger est utilisé par Catnip pour print(), warnings, etc.
catnip.parse('print("BORN TO SEGFAULT")')
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

## Gestion des Exceptions

Les exceptions Catnip héritent de `CatnipError` et sont définies dans `catnip.exc` :

```python
from catnip import Catnip
from catnip.exc import CatnipError, CatnipSyntaxError, CatnipRuntimeError

catnip = Catnip()

try:
    catnip.parse("invalid syntax !")
    catnip.execute()
except CatnipSyntaxError as e:
    print(f"Erreur de parsing : {e}")
except CatnipRuntimeError as e:
    print(f"Erreur d'exécution : {e}")
except CatnipError as e:
    print(f"Erreur Catnip : {e}")
```

Hiérarchie : `CatnipError` > `CatnipSyntaxError`, `CatnipSemanticError`, `CatnipRuntimeError`, `CatnipNameError`,
`CatnipTypeError`, `CatnipPatternError`, `CatnipArityError`.

## Exposition de Services Hôte

### Injection via Context.globals

L'hôte peut injecter ses services dans les globals du contexte :

```python
from catnip import Catnip
from catnip.context import Context

class HostServices:
    def __init__(self, redis_client):
        self.redis = redis_client

    def cache_get(self, key: str):
        return self.redis.get(key)

    def cache_set(self, key: str, value, ttl: int = 3600):
        self.redis.setex(key, ttl, value)

# Injection dans les globals
ctx = Context()
ctx.globals['host'] = HostServices(redis_client)
catnip = Catnip(context=ctx)

# Les scripts Catnip ont accès aux services via `host`
catnip.parse("""
data = host.cache_get("users")
host.cache_set("result", data, 3600)
""")
catnip.execute()
```

## Résumé

| Service      | Interface                  | Exemples                    |
| ------------ | -------------------------- | --------------------------- |
| **Cache**    | `CacheBackend`             | Redis, diskcache, memcached |
| **Logging**  | `Context(logger=...)`      | JSON logger, Syslog         |
| **Services** | `context.globals['x'] = …` | DB, API, notifications      |

### Exemple Minimal

```python
from catnip import Catnip
from catnip.cachesys import CatnipCache
import redis

redis_client = redis.Redis()
backend = RedisCache(redis_client, prefix="myapp:")

catnip = Catnip(cache=CatnipCache(backend=backend))

catnip.parse("x = 2 + 2")   # compilé et mis en cache
catnip.execute()

catnip.parse("x = 2 + 2")   # récupéré depuis le cache
catnip.execute()
```
