#!/usr/bin/env python3
"""
Exemple d'utilisation de la memoization avec hooks Python personnalisés.

Montre comment intégrer un backend de memoization custom (Redis, Memcached, etc.)
pour contrôler finement la mémorisation des résultats d'exécution.
"""

from typing import Any, Optional

from catnip import Catnip
from catnip.cachesys import CacheBackend, CacheEntry, CacheKey, Memoization


# EXEMPLE 1 : Backend personnalisé simple (logging)


class LoggingMemoizationBackend(CacheBackend):
    """Backend de memoization qui log toutes les opérations."""

    def __init__(self):
        self.store = {}
        self.hits = 0
        self.misses = 0

    def get(self, key: CacheKey) -> Optional[CacheEntry]:
        key_str = key.to_string()
        print(f"  [Cache] GET {key_str}")

        if key_str in self.store:
            self.hits += 1
            print(f"  [Cache] ✓ HIT")
            return self.store[key_str]
        else:
            self.misses += 1
            print(f"  [Cache] ✗ MISS")
            return None

    def set(self, key: CacheKey, value: Any, metadata: dict = None) -> None:
        key_str = key.to_string()
        print(f"  [Cache] SET {key_str} = {value}")
        entry = CacheEntry(key=key_str, value=value, cache_type=key.cache_type, metadata=metadata)
        self.store[key_str] = entry

    def delete(self, key: CacheKey) -> bool:
        key_str = key.to_string()
        existed = key_str in self.store
        if existed:
            del self.store[key_str]
            print(f"  [Cache] DELETE {key_str}")
        return existed

    def clear(self) -> None:
        print(f"  [Cache] CLEAR ALL ({len(self.store)} entries)")
        self.store.clear()

    def exists(self, key: CacheKey) -> bool:
        return key.to_string() in self.store

    def stats(self) -> dict:
        return {
            'backend': 'LoggingCache',
            'size': len(self.store),
            'hits': self.hits,
            'misses': self.misses,
            'hit_rate': (
                f"{self.hits / (self.hits + self.misses) * 100:.1f}%" if (self.hits + self.misses) > 0 else "N/A"
            ),
        }


def example_logging_backend():
    """Exemple avec backend qui log toutes les opérations."""
    print("⇒ Exemple 1 : Backend avec logging")

    # Créer un cache avec le backend custom
    backend = LoggingMemoizationBackend()
    func_cache = Memoization(backend=backend)

    # Créer le contexte Catnip
    cat = Catnip()
    cat.context.memoization = func_cache

    code = """
    square = cached((x) => { x * x }, "square")

    print("Appel 1: square(5)")
    r1 = square(5)

    print("\\nAppel 2: square(5) [devrait être en cache]")
    r2 = square(5)

    print("\\nAppel 3: square(10) [nouveau calcul]")
    r3 = square(10)

    list(r1, r2, r3)
    """

    cat.parse(code)
    result = cat.execute()
    print(f"\nRésultat: {result}")
    print(f"Stats: {backend.stats()}")
    print()


# EXEMPLE 2 : Backend avec TTL (Time-To-Live)


import time


class TTLCacheBackend(CacheBackend):
    """Backend de cache avec expiration automatique (TTL)."""

    def __init__(self, ttl_seconds: float = 2.0):
        self.store = {}  # key -> (entry, timestamp)
        self.ttl = ttl_seconds
        self.hits = 0
        self.misses = 0

    def get(self, key: CacheKey) -> Optional[CacheEntry]:
        key_str = key.to_string()

        if key_str in self.store:
            entry, timestamp = self.store[key_str]
            age = time.time() - timestamp

            if age < self.ttl:
                self.hits += 1
                print(f"  [TTL Cache] HIT (age: {age:.1f}s)")
                return entry
            else:
                # Expiré
                del self.store[key_str]
                self.misses += 1
                print(f"  [TTL Cache] EXPIRED (age: {age:.1f}s)")
                return None
        else:
            self.misses += 1
            print(f"  [TTL Cache] MISS")
            return None

    def set(self, key: CacheKey, value: Any, metadata: dict = None) -> None:
        key_str = key.to_string()
        entry = CacheEntry(key=key_str, value=value, cache_type=key.cache_type, metadata=metadata)
        self.store[key_str] = (entry, time.time())
        print(f"  [TTL Cache] SET {key_str} (TTL: {self.ttl}s)")

    def delete(self, key: CacheKey) -> bool:
        key_str = key.to_string()
        existed = key_str in self.store
        if existed:
            del self.store[key_str]
        return existed

    def clear(self) -> None:
        self.store.clear()

    def exists(self, key: CacheKey) -> bool:
        key_str = key.to_string()
        if key_str in self.store:
            _, timestamp = self.store[key_str]
            return (time.time() - timestamp) < self.ttl
        return False

    def stats(self) -> dict:
        return {
            'backend': 'TTLCache',
            'size': len(self.store),
            'ttl': f"{self.ttl}s",
            'hits': self.hits,
            'misses': self.misses,
            'hit_rate': (
                f"{self.hits / (self.hits + self.misses) * 100:.1f}%" if (self.hits + self.misses) > 0 else "N/A"
            ),
        }


def example_ttl_backend():
    """Exemple avec cache TTL."""
    print("⇒ Exemple 2 : Backend avec TTL (expiration)")

    # Cache avec TTL de 2 secondes
    backend = TTLCacheBackend(ttl_seconds=2.0)
    func_cache = Memoization(backend=backend)

    cat = Catnip()
    cat.context.memoization = func_cache
    cat.context.globals['sleep'] = time.sleep

    code = """
    expensive = cached((x) => {
        print("    → Calcul coûteux…")
        x * 10
    }, "expensive")

    print("Appel 1:")
    r1 = expensive(5)

    print("\\nAppel 2 (immédiat, en cache):")
    r2 = expensive(5)

    print("\\nAttente 2.5s…")
    sleep(2.5)

    print("\\nAppel 3 (après expiration):")
    r3 = expensive(5)

    list(r1, r2, r3)
    """

    cat.parse(code)
    result = cat.execute()
    print(f"\nRésultat: {result}")
    print(f"Stats: {backend.stats()}")
    print()


# EXEMPLE 3 : Backend avec validation conditionnelle


class ConditionalCacheBackend(CacheBackend):
    """Backend qui invalide le cache selon des conditions custom."""

    def __init__(self, validator_func=None):
        self.store = {}
        self.validator = validator_func or (lambda key, value, metadata: True)
        self.hits = 0
        self.misses = 0
        self.invalidations = 0

    def get(self, key: CacheKey) -> Optional[CacheEntry]:
        key_str = key.to_string()

        if key_str in self.store:
            entry = self.store[key_str]

            # Valider avant de retourner
            if self.validator(key_str, entry.value, entry.metadata):
                self.hits += 1
                return entry
            else:
                # Invalider
                del self.store[key_str]
                self.invalidations += 1
                print(f"  [Conditional Cache] INVALIDATED by validator")
                self.misses += 1
                return None
        else:
            self.misses += 1
            return None

    def set(self, key: CacheKey, value: Any, metadata: dict = None) -> None:
        key_str = key.to_string()
        entry = CacheEntry(key=key_str, value=value, cache_type=key.cache_type, metadata=metadata)
        self.store[key_str] = entry

    def delete(self, key: CacheKey) -> bool:
        key_str = key.to_string()
        existed = key_str in self.store
        if existed:
            del self.store[key_str]
        return existed

    def clear(self) -> None:
        self.store.clear()

    def exists(self, key: CacheKey) -> bool:
        return key.to_string() in self.store

    def stats(self) -> dict:
        return {
            'backend': 'ConditionalCache',
            'size': len(self.store),
            'hits': self.hits,
            'misses': self.misses,
            'invalidations': self.invalidations,
        }


def example_conditional_backend():
    """Exemple avec validation conditionnelle."""
    print("⇒ Exemple 3 : Backend avec validation conditionnelle")

    # Variable externe pour contrôler la validation
    cache_valid = {'value': True}

    # Validator qui vérifie la variable externe
    def validator(key, value, metadata):
        return cache_valid['value']

    backend = ConditionalCacheBackend(validator_func=validator)
    func_cache = Memoization(backend=backend)

    cat = Catnip()
    cat.context.memoization = func_cache

    # Exposer la variable de contrôle
    cat.context.globals['invalidate_cache'] = lambda: cache_valid.update({'value': False})
    cat.context.globals['validate_cache'] = lambda: cache_valid.update({'value': True})

    code = """
    compute = cached((x) => {
        print("    → Calcul en cours…")
        x + 100
    }, "compute")

    print("Appel 1:")
    r1 = compute(5)

    print("\\nAppel 2 (en cache):")
    r2 = compute(5)

    print("\\nInvalidation externe…")
    invalidate_cache()

    print("\\nAppel 3 (après invalidation):")
    r3 = compute(5)

    print("\\nRé-validation…")
    validate_cache()

    print("\\nAppel 4 (nouveau cache):")
    r4 = compute(5)

    list(r1, r2, r3, r4)
    """

    cat.parse(code)
    result = cat.execute()
    print(f"\nRésultat: {result}")
    print(f"Stats: {backend.stats()}")
    print()


# MAIN


def main():
    print("╔════════════════════════════════════════════════════╗")
    print("║  Cache de fonctions avec hooks Python custom      ║")
    print("╚════════════════════════════════════════════════════╝\n")

    example_logging_backend()
    example_ttl_backend()
    example_conditional_backend()

    print("✓ Tous les exemples terminés !")
    print("\nCes backends peuvent être adaptés pour utiliser :")
    print("  - Redis (pour cache distribué)")
    print("  - Memcached (pour cache en mémoire partagé)")
    print("  - DiskCache (pour persistance)")
    print("  - Ou tout autre système de stockage")


if __name__ == "__main__":
    main()
