#!/usr/bin/env python3
"""
Exemple d'utilisation du cache Redis avec Catnip.

Permet de partager le cache entre plusieurs processus/machines.
Nécessite Redis et le package 'redis' Python.
"""
from catnip import Catnip
from catnip.cache import CatnipCache, RedisCache


def main():
    print("⇒ Cache Redis (Multi-processus)")

    try:
        # Créer un backend Redis avec TTL de 1 heure
        redis_backend = RedisCache(
            redis_client=None, ttl=3600, prefix="catnip_demo"  # Connexion par défaut localhost:6379  # 1 heure
        )

        # Créer le cache avec contrôle fin
        cache = CatnipCache(backend=redis_backend, cache_ast=True, cache_bytecode=True)  # Cache aussi le bytecode

        print("1. Connexion à Redis réussie")
        print(f"   Config: {redis_backend.stats()['redis_info']}\n")

        # Nettoyer le cache précédent
        cache.clear()

        # Premier parsing
        print("2. Premier parsing avec mise en cache")
        code = """
        fib = (n) => {
            if n <= 1 {
                n
            } else {
                fib(n - 1) + fib(n - 2)
            }
        }
        fib(10)
        """

        catnip = Catnip(cache=cache)
        catnip.parse(code)
        result = catnip.execute()
        print(f"   fib(10) = {result}")
        print(f"   Stats: {cache.stats()}\n")

        # Deuxième parsing depuis un autre "processus" simulé
        print("3. Parsing depuis cache (autre instance)")
        catnip2 = Catnip(cache=cache)
        catnip2.parse(code)
        result2 = catnip2.execute()
        print(f"   fib(10) = {result2}")
        print(f"   Stats: {cache.stats()}\n")

        # Tester le contrôle fin : même code, options différentes
        print("4. Même code avec optimize=False (clé différente)")
        catnip3 = Catnip(cache=cache)
        catnip3.pragma_context.optimize_level = 0
        catnip3.parse(code)
        print(f"   Stats: {cache.stats()}\n")

        # Afficher les statistiques finales
        print("5. Statistiques finales Redis")
        stats = cache.stats()
        for key, value in stats.items():
            print(f"   {key}: {value}")

    except ImportError:
        print("❌ Le package 'redis' n'est pas installé")
        print("   Installe-le avec: pip install redis")
    except Exception as e:
        print(f"❌ Erreur de connexion Redis: {e}")
        print("   Assure-toi que Redis tourne sur localhost:6379")


if __name__ == "__main__":
    main()
