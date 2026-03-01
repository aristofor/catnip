# Système de cache

Système de cache multi-niveaux, écrit en Rust, avec un protocole simple pour brancher d'autres backends.

## Architecture

### Composants Rust

**`catnip_rs/src/cache/mod.rs`** - Types de base :

- `CacheType` : enum des types de contenu (SOURCE, AST, BYTECODE, RESULT)
- `CacheKey` : clé avec hash xxhash (content + options + version)
- `CacheEntry` : entrée avec métadonnées
- `MemoryCache` : cache mémoire IndexMap FIFO + stats

**`catnip_rs/src/cache/backend.rs`** - Adapter compilation :

- `CatnipCache` : adapter haut niveau pour compilation (AST, bytecode)
- Duck typing via PyO3 : appelle `.get()`, `.set()` dynamiquement sur le backend

**`catnip_rs/src/cache/disk.rs`** - Cache persistant :

- `DiskCache` : stockage XDG avec TTL, max_size, LRU eviction

**`catnip_rs/src/cache/memoization.rs`** - Mémoïsation :

- `Memoization` : cache résultats de fonctions avec index HashMap

### Composants Python

**`catnip/cachesys/base.py`** - Protocole Python :

- `class CacheBackend(Protocol)` : interface pour backends custom Python
- Méthodes requises : `get`, `set`, `delete`, `clear`, `exists`, `stats`

**`catnip/cachesys/memoization.py`** - Wrapper legacy :

- `CachedWrapper` : utilisé par `context.py` pour le décorateur `@cached`

**`catnip/cachesys/__init__.py`** - Réexports :

- Wrapper itérable `CacheType` (metaclass pour `for cache_type in CacheType`)
- Réexports depuis Rust

## Backends intégrés

### MemoryCache

Cache en mémoire rapide avec éviction FIFO.

```python
from catnip._rs import MemoryCache

cache = MemoryCache(max_size=1000)
cache.set(key, value)
entry = cache.get(key)
stats = cache.stats()
```

**Features** :

- IndexMap FIFO (ordre préservé)
- Statistiques hit/miss
- Max size configurable
- O(1) get/set

### DiskCache

Cache persistant sur disque avec TTL.

```python
from catnip._rs import DiskCache

cache = DiskCache(
    cache_dir="/path/to/cache",
    ttl_seconds=3600,
    max_size_mb=100
)
```

**Features** :

- XDG Base Directory (par défaut)
- TTL (time-to-live)
- LRU eviction
- Sérialisation pickle
- Atomic writes (temp + rename) pour sécurité multiprocessus

### CatnipCache

Adapter haut niveau pour compilation.

```python
from catnip._rs import CatnipCache, MemoryCache

cache = CatnipCache(
    backend=MemoryCache(),
    cache_ast=True,
    cache_bytecode=True
)

# Automatique dans Catnip class
ast = cache.get_parsed(source, optimize=True, tco_enabled=True)
cache.set_bytecode(source, bytecode, optimize=True)
```

## Protocole pour backends personnalisés

Les applications hôtes peuvent implémenter leurs propres backends.

### Interface

```python
from typing import Any, Optional
from catnip._rs import CacheEntry, CacheKey

class MyCustomCache:
    def get(self, key: CacheKey) -> Optional[CacheEntry]:
        """Retrieve entry from cache."""
        ...

    def set(self, key: CacheKey, value: Any, metadata: dict = None) -> None:
        """Store entry in cache."""
        ...

    def delete(self, key: CacheKey) -> bool:
        """Delete entry. Returns True if existed."""
        ...

    def clear(self) -> None:
        """Clear entire cache."""
        ...

    def exists(self, key: CacheKey) -> bool:
        """Check if key exists."""
        ...

    def stats(self) -> dict:
        """Return statistics."""
        return {
            'backend': 'custom',
            'size': 0,
            'hits': 0,
            'misses': 0,
        }
```

### Exemple : Redis backend

```python
import redis
import pickle
from catnip._rs import CacheEntry, CacheKey, CatnipCache

class RedisCache:
    def __init__(self, host='localhost', port=6379, db=0, ttl=3600):
        self.redis = redis.Redis(host=host, port=port, db=db)
        self.ttl = ttl
        self.hits = 0
        self.misses = 0

    def get(self, key: CacheKey) -> Optional[CacheEntry]:
        key_str = key.to_string()
        data = self.redis.get(key_str)
        if data:
            self.hits += 1
            return pickle.loads(data)
        self.misses += 1
        return None

    def set(self, key: CacheKey, value: Any, metadata: dict = None) -> None:
        key_str = key.to_string()
        entry = CacheEntry(key_str, value, key.cache_type, metadata or {})
        self.redis.setex(key_str, self.ttl, pickle.dumps(entry))

    def delete(self, key: CacheKey) -> bool:
        key_str = key.to_string()
        return bool(self.redis.delete(key_str))

    def clear(self) -> None:
        self.redis.flushdb()
        self.hits = 0
        self.misses = 0

    def exists(self, key: CacheKey) -> bool:
        key_str = key.to_string()
        return bool(self.redis.exists(key_str))

    def stats(self) -> dict:
        total = self.hits + self.misses
        hit_rate = (self.hits / total * 100) if total > 0 else 0
        return {
            'backend': 'redis',
            'size': self.redis.dbsize(),
            'hits': self.hits,
            'misses': self.misses,
            'hit_rate': f'{hit_rate:.1f}%',
        }

# Utilisation
redis_cache = RedisCache(ttl=7200)
cache = CatnipCache(backend=redis_cache)
```

## JIT Cache

Cache unifié pour les traces JIT et les stencils Cranelift, colocalisé avec le cache de compilation.

**Module** : `catnip_rs/src/jit/trace_cache.rs`

**Emplacement** : `~/.cache/catnip/` (fichiers plats, même répertoire que le DiskCache)

**Fichiers** :

- `jit_v{V}_{HASH}_{OFFSET}` -- traces (bincode), clé = FNV-1a du bytecode + offset de boucle
- `jit_native_{SHA256}` -- stencils Cranelift (postcard), clé = hash du IR + triple ISA + flags CPU

Les traces persistent les enregistrements JIT pour éliminer le warm-up (100+ itérations). Les stencils persistent le
code machine non relocaté pour éliminer la recompilation Cranelift. L'invalidation se fait par version Catnip (traces)
et par `VersionMarker` Cranelift (stencils).

Les écritures sont atomiques (temp + rename), ce qui permet l'utilisation concurrente par les workers ND sans lock.

Le DiskCache (`catnip_*`) ne prune que ses propres fichiers ; les fichiers JIT ne sont pas soumis au TTL du DiskCache.

Voir `docs/dev/JIT.md` pour les détails du mécanisme.

## CacheKey et invalidation

Les clés incluent automatiquement :

- Version du langage (`__lang_id__`)
- Version Catnip (`__version__`)
- Build date
- Contenu (hash xxhash)
- Options de compilation (optimize, tco_enabled)

Le cache s'invalide automatiquement quand :

- La version de Catnip change
- La date de build change
- Les options de compilation changent

## CLI

```bash
# Statistiques
catnip cache stats

# Nettoyage (remove expired)
catnip cache prune

# Clear complet
catnip cache clear

# Configuration
catnip config set cache_max_size_mb 500
catnip config set cache_ttl_seconds 7200
catnip config show --debug
```

## Tests

```bash
pytest tests/ -k cache -v
```

Couverture : CacheKey, MemoryCache FIFO, DiskCache TTL/max_size, CatnipCache adapter, memoization, itération CacheType.
