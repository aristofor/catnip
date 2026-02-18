#!/usr/bin/env python3
"""
Démonstration rapide du système de cache Catnip.
"""
from catnip import Catnip
from catnip.cache import CatnipCache, MemoryCache


def demo_basic_cache():
    """Démo de base : activer le cache."""
    print("⇒ Démo 1 : Cache de base")

    # Activer simplement le cache
    catnip = Catnip(enable_cache=True)

    code = "(x) => { x * x }"

    # Premier parsing (cache miss)
    catnip.parse(code)
    print(f"Parse 1: {catnip.cache.stats()['misses']} miss(es)")

    # Deuxième parsing (cache hit)
    catnip2 = Catnip(cache=catnip.cache)  # Réutiliser le même cache
    catnip2.parse(code)
    print(f"Parse 2: {catnip2.cache.stats()['hits']} hit(s)")
    print(f"Hit rate: {catnip2.cache.stats()['hit_rate']}\n")


def demo_fine_grained():
    """Démo du contrôle fin : options de compilation."""
    print("⇒ Démo 2 : Contrôle fin")

    cache = CatnipCache(backend=MemoryCache())
    code = "fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }"

    # Parser avec optimize=True
    cat1 = Catnip(cache=cache)
    cat1.pragma_context.optimize_level = 1
    cat1.parse(code)
    print(f"Avec optimize: {cache.stats()['size']} entrée(s)")

    # Parser avec optimize=False (nouvelle entrée !)
    cat2 = Catnip(cache=cache)
    cat2.pragma_context.optimize_level = 0
    cat2.parse(code)
    print(f"Sans optimize: {cache.stats()['size']} entrées")
    print("→ Deux clés différentes pour le même code\n")


def demo_custom_backend():
    """Démo avec backend personnalisé."""
    print("⇒ Démo 3 : Backend personnalisé")

    # Cache mémoire avec limite de 2 entrées
    backend = MemoryCache(max_size=2)
    cache = CatnipCache(backend=backend, cache_ast=True, cache_bytecode=False)  # Uniquement AST

    # Remplir le cache
    for i in range(3):
        cat = Catnip(cache=cache)
        cat.parse(f"{i} + {i}")
        print(f"Parse '{i} + {i}': size={cache.stats()['size']}")

    print("→ Taille max respectée (éviction FIFO)\n")


def demo_stats():
    """Démo des statistiques."""
    print("⇒ Démo 4 : Statistiques")

    cache = CatnipCache(backend=MemoryCache())
    catnip = Catnip(cache=cache)

    # Parser plusieurs fois
    for _ in range(5):
        catnip.parse("1 + 1")

    for _ in range(3):
        catnip.parse("2 + 2")

    # Afficher les stats
    stats = cache.stats()
    print(f"Backend:  {stats['backend']}")
    print(f"Entrées:  {stats['size']}")
    print(f"Hits:     {stats['hits']}")
    print(f"Misses:   {stats['misses']}")
    print(f"Hit rate: {stats['hit_rate']}\n")


def main():
    print("╔═══════════════════════════════════════╗")
    print("║  Démonstration du Cache Catnip        ║")
    print("╚═══════════════════════════════════════╝\n")

    demo_basic_cache()
    demo_fine_grained()
    demo_custom_backend()
    demo_stats()

    print("✓ Toutes les démos réussies !")


if __name__ == "__main__":
    main()
