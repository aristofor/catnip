#!/usr/bin/env python3
"""
Exemple d'utilisation du cache mémoire avec Catnip.

Idéal pour la REPL : cache rapide en mémoire sans dépendances.
"""
from catnip import Catnip
from catnip.cache import CatnipCache, MemoryCache


def main():
    print("⇒ Cache Mémoire (REPL mode)")

    # Créer un cache mémoire avec taille limite
    memory_backend = MemoryCache(max_size=100)
    cache = CatnipCache(backend=memory_backend, cache_ast=True, cache_bytecode=False)  # Pas besoin de bytecode en REPL

    # Créer l'instance Catnip avec cache
    catnip = Catnip(cache=cache)

    # Premier parsing (cache miss)
    print("1. Premier parsing de '1 + 2 * 3'")
    code = "1 + 2 * 3"
    catnip.parse(code)
    result = catnip.execute()
    print(f"   Résultat: {result}")
    print(f"   Stats: {cache.stats()}\n")

    # Deuxième parsing (cache hit!)
    print("2. Deuxième parsing de '1 + 2 * 3' (depuis cache)")
    catnip2 = Catnip(cache=cache)
    catnip2.parse(code)
    result2 = catnip2.execute()
    print(f"   Résultat: {result2}")
    print(f"   Stats: {cache.stats()}\n")

    # Parser d'autres expressions (indépendantes)
    print("3. Parsing de plusieurs expressions")
    expressions = [
        "2 + 2",
        "10 * 5",
        "100 / 4",
        "2 + 2",  # Dupliquer pour montrer le cache hit
    ]

    for expr in expressions:
        cat = Catnip(cache=cache)
        cat.parse(expr)
        result = cat.execute()
        print(f"   '{expr}' = {result}")

    print(f"\n   Stats finales: {cache.stats()}\n")

    # Tester l'invalidation
    print("4. Invalidation du cache pour une expression")
    cache.invalidate_all("2 + 2")
    print(f"   Stats après invalidation: {cache.stats()}\n")


if __name__ == "__main__":
    main()
