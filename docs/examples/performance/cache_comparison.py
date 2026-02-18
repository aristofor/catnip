#!/usr/bin/env python3
"""
Comparaison et contrôle fin des différents backends de cache.

Montre comment le même code peut avoir plusieurs entrées de cache
selon les options de compilation (optimize, tco_enabled).
"""
import time
from catnip import Catnip
from catnip.cache import CatnipCache, MemoryCache, CacheKey, CacheType


def benchmark_parsing(name, catnip, code, iterations=100):
    """Benchmark le parsing avec ou sans cache."""
    start = time.perf_counter()
    for _ in range(iterations):
        catnip.parse(code)
    elapsed = time.perf_counter() - start
    avg = elapsed / iterations * 1000  # en ms
    print(f"   {name}: {avg:.3f} ms/parse (total: {elapsed:.2f}s)")
    return avg


def main():
    print("⇒ Contrôle fin et Comparaison des Caches")

    # Code de test complexe
    code = """
    quicksort = (arr) => {
        if arr.length <= 1 {
            arr
        } else {
            pivot = arr[0]
            rest = arr[1:]
            left = rest.filter(x => x < pivot)
            right = rest.filter(x => x >= pivot)
            quicksort(left) + [pivot] + quicksort(right)
        }
    }
    quicksort([5, 2, 8, 1, 9, 3, 7, 4, 6])
    """

    # 1. Démonstration du contrôle fin
    print("1. Contrôle fin : même code, clés différentes\n")

    cache = CatnipCache(backend=MemoryCache())

    # Créer différentes clés pour le même code
    keys = [
        CacheKey(code, CacheType.AST, optimize=True, tco_enabled=True),
        CacheKey(code, CacheType.AST, optimize=True, tco_enabled=False),
        CacheKey(code, CacheType.AST, optimize=False, tco_enabled=True),
        CacheKey(code, CacheType.AST, optimize=False, tco_enabled=False),
    ]

    print("   Clés générées pour le même code:")
    for i, key in enumerate(keys, 1):
        print(f"   {i}. optimize={key.optimize}, tco={key.tco_enabled}")
        print(f"      → {key.to_string()}")
    print()

    # 2. Benchmark avec et sans cache
    print("2. Benchmark : impact du cache\n")

    # Sans cache
    catnip_no_cache = Catnip()
    time_no_cache = benchmark_parsing("Sans cache", catnip_no_cache, code, iterations=50)

    # Avec cache mémoire
    catnip_cached = Catnip(cache=cache)
    # Premier passage pour remplir le cache
    catnip_cached.parse(code)

    time_cached = benchmark_parsing("Avec cache", catnip_cached, code, iterations=50)

    speedup = time_no_cache / time_cached
    print(f"\n   Accélération: {speedup:.1f}x plus rapide avec cache\n")

    # 3. Comparaison des backends
    print("3. Statistiques des différents backends\n")

    backends = {
        "Memory": MemoryCache(max_size=100),
    }

    # Tester DiskCache si disponible
    try:
        from catnip.cache import DiskCache
        import tempfile

        tmpdir = tempfile.mkdtemp(prefix="catnip_bench_")
        backends["Disk"] = DiskCache(directory=tmpdir)
    except ImportError:
        print("   (DiskCache non disponible - pip install diskcache)")

    # Tester Redis si disponible
    try:
        from catnip.cache import RedisCache

        backends["Redis"] = RedisCache(prefix="catnip_bench")
    except (ImportError, Exception):
        print("   (Redis non disponible)")

    print()

    # Benchmark chaque backend
    results = {}
    for name, backend in backends.items():
        cache = CatnipCache(backend=backend, cache_ast=True)
        catnip = Catnip(cache=cache)

        # Remplir le cache
        catnip.parse(code)

        # Mesurer
        avg = benchmark_parsing(f"{name:8s}", catnip, code, iterations=50)
        results[name] = avg

        # Stats
        stats = cache.stats()
        print(f"            Stats: {stats.get('hit_rate', 'N/A')}")

        # Nettoyer
        if hasattr(backend, 'close'):
            backend.close()

    print("\n4. Résumé des performances\n")
    sorted_results = sorted(results.items(), key=lambda x: x[1])
    for name, avg in sorted_results:
        print(f"   {name:8s}: {avg:.3f} ms/parse")

    fastest = sorted_results[0][1]
    print(f"\n   Le plus rapide: {sorted_results[0][0]} ({fastest:.3f} ms)")


if __name__ == "__main__":
    main()
