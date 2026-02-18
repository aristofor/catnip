#!/usr/bin/env python3
"""
Exemple d'utilisation du cache disque avec Catnip.

Cache persistant sur disque avec TTL et limite de taille.
Implémentation native Rust.
"""
from catnip import Catnip
from catnip.cachesys import CatnipCache, DiskCache
from catnip.config import get_cache_dir


def main():
    print("⇒ Cache Disque (Persistant)")

    # Utilise XDG_CACHE_HOME/catnip/demo par défaut
    cache_dir = get_cache_dir() / 'demo'

    # Créer un backend disque avec limite de 100MB et TTL de 1h
    disk_backend = DiskCache(
        directory=str(cache_dir),
        max_size_bytes=100 * 1024 * 1024,  # 100MB
        ttl_seconds=3600,  # 1 heure
    )

    # Créer le cache
    cache = CatnipCache(backend=disk_backend, cache_ast=True, cache_bytecode=True)

    print(f"1. Cache créé dans: {cache_dir}")
    print(f"   Stats initiales: {cache.stats()}\n")

    # Script de test
    code = """
    fact = (n, acc=1) => {
        if n <= 1 {
            acc
        } else {
            fact(n - 1, n * acc)
        }
    }
    fact(20)
    """

    # Premier parsing (cache miss)
    print("2. Premier parsing (écriture sur disque)")
    catnip = Catnip(cache=cache)
    catnip.parse(code)
    result = catnip.execute()
    print(f"   fact(20) = {result}")
    stats = cache.stats()
    print(f"   Volume: {stats['volume_mb']} MB")
    print(f"   Entrées: {stats['size']}\n")

    # Deuxième parsing (cache hit depuis disque)
    print("3. Deuxième parsing (lecture depuis disque)")
    catnip2 = Catnip(cache=cache)
    catnip2.parse(code)
    result2 = catnip2.execute()
    print(f"   fact(20) = {result2}")
    print(f"   Stats: {cache.stats()}\n")

    # Parser plusieurs scripts
    print("4. Mise en cache de plusieurs scripts")
    scripts = [
        "10 + 5",
        "20 * 3",
        "100 - 25",
    ]

    for i, script in enumerate(scripts, 1):
        cat = Catnip(cache=cache)
        cat.parse(script)
        result = cat.execute()
        print(f"   Script {i}: '{script}' = {result}")

    print(f"\n   Stats finales: {cache.stats()}\n")

    # Démonstration de la persistance
    print("5. Le cache persiste après redémarrage du programme")
    print(f"   Répertoire: {cache_dir}")
    print(f"   Le cache sera disponible au prochain lancement")

    # Prune manuel
    print("\n6. Nettoyage manuel (prune)")
    removed = disk_backend.prune()
    print(f"   Entrées expirées supprimées: {removed}")
    print(f"   Stats après prune: {cache.stats()}")

    # Commandes CLI disponibles
    print("\n7. Gestion du cache via CLI")
    print("   Pour gérer le cache global de Catnip:")
    print("   - catnip cache stats           # Afficher statistiques")
    print("   - catnip cache prune           # Nettoyer entrées expirées")
    print("   - catnip cache clear --force   # Supprimer tout le cache")
    print("\n   Configuration du cache:")
    print("   - catnip config set cache_max_size_mb 50")
    print("   - catnip config set cache_ttl_seconds 7200")
    print("\n   Voir docs/user/CLI.md pour plus de détails")


if __name__ == "__main__":
    main()
