#!/usr/bin/env python3
"""
Exemple d'utilisation du cache de fonctions pour un script de build.

Démontre comment utiliser le system de cache au niveau fonction
pour éviter de ré-exécuter des tâches de build coûteuses.
"""

import hashlib
import time
from pathlib import Path

from catnip import Catnip
from catnip.cachesys import DiskCache, Memoization


def setup_build_environment():
    """Setup un contexte Catnip avec memoization persistante pour build."""
    # Utiliser DiskCache pour persistance entre exécutions
    disk_backend = DiskCache(cache_dir=Path('.build_cache'))
    memo = Memoization(backend=disk_backend)

    # Créer le contexte Catnip avec la memoization
    catnip = Catnip()
    catnip.context.memoization = memo

    # Ajouter des helpers Python pour le build
    def read_file(path):
        """Lire un fichier et retourner son contenu."""
        return Path(path).read_text()

    def hash_files(files):
        """Calculer le hash d'une liste de fichiers."""
        hasher = hashlib.sha256()
        for f in files:
            content = Path(f).read_bytes()
            hasher.update(content)
        return hasher.hexdigest()

    def write_file(path, content):
        """Écrire dans un fichier."""
        Path(path).write_text(content)
        return path

    # Injecter dans les globals
    catnip.context.globals['read_file'] = read_file
    catnip.context.globals['hash_files'] = hash_files
    catnip.context.globals['write_file'] = write_file
    catnip.context.globals['sleep'] = time.sleep

    return catnip


def example_simple():
    """Exemple simple : fonction de build cachée."""
    print("⇒ Exemple 1 : Build simple avec cache")

    cat = Catnip()

    code = """
    # Simuler une compilation SASS coûteuse
    build_sass = cached((files) => {
        print("→ Compilation SASS en cours…")
        # Simuler le temps de compilation
        result = "/* CSS compiled */"
        result
    }, "build_sass")

    # Premier appel : compilation réelle
    print("Premier appel:")
    css1 = build_sass(list("style.scss", "theme.scss"))

    # Deuxième appel : récupération du cache
    print("\\nDeuxième appel (même arguments):")
    css2 = build_sass(list("style.scss", "theme.scss"))

    # Appel avec fichiers différents : compilation réelle
    print("\\nTroisième appel (fichiers différents):")
    css3 = build_sass(list("admin.scss"))

    print("\\nStatistiques du cache:")
    stats = _cache.stats()
    print("  Hits:", stats["hits"])
    print("  Misses:", stats["misses"])
    """

    cat.parse(code)
    cat.execute()
    print()


def example_with_custom_key():
    """Exemple avec clé de cache custom."""
    print("⇒ Exemple 2 : Cache avec noms dynamiques")

    cat = Catnip()

    code = """
    # Fonctions avec noms de cache différents
    proc1 = cached((data) => {
        print("  → Traitement v1 pour:", data)
        "result_v1_" + str(data)
    }, "processor_v1")

    proc2 = cached((data) => {
        print("  → Traitement v2 pour:", data)
        "result_v2_" + str(data)
    }, "processor_v2")

    # Tester les caches indépendants
    print("Appels processor v1:")
    r1 = proc1("data_a")
    r2 = proc1("data_a")  # Cache hit

    print("\\nAppels processor v2:")
    r3 = proc2("data_a")
    r4 = proc2("data_a")  # Cache hit

    print("\\nRésultats:", list(r1, r2, r3, r4))
    """

    cat.parse(code)
    cat.execute()
    print()


def example_invalidation():
    """Exemple d'invalidation du cache."""
    print("⇒ Exemple 3 : Invalidation du cache")

    cat = Catnip()

    code = """
    counter = 0

    expensive_task = cached((input) => {
        counter = counter + 1
        print("  Exécution #" + str(counter))
        "result_" + str(input)
    }, "expensive_task")

    # Première série d'appels
    print("Première série:")
    expensive_task("data1")
    expensive_task("data1")  # Cache hit
    expensive_task("data2")
    expensive_task("data2")  # Cache hit

    print("\\nInvalidation du cache…")
    _cache.invalidate("expensive_task")

    # Deuxième série : le cache a été invalidé
    print("\\nDeuxième série (après invalidation):")
    expensive_task("data1")  # Cache miss
    expensive_task("data2")  # Cache miss
    """

    cat.parse(code)
    cat.execute()
    print()


def example_build_pipeline():
    """Exemple complet d'un pipeline de build avec cache."""
    print("⇒ Exemple 4 : Pipeline de build complet")

    cat = Catnip()

    # Créer des fichiers source de test
    Path('source.txt').write_text('source content')

    code = """
    # Pipeline de build avec plusieurs étapes cachées

    # Étape 1 : Parser les sources
    parse_sources = cached((files) => {
        print("  [1/3] Parsing sources…")
        # Retourner le nombre de fichiers parsés
        len(files)
    }, "parse_sources")

    # Étape 2 : Optimiser
    optimize = cached((parsed) => {
        print("  [2/3] Optimisation…")
        # Simuler l'optimisation
        parsed * 2
    }, "optimize")

    # Étape 3 : Générer la sortie
    generate = cached((optimized) => {
        print("  [3/3] Génération du build…")
        # Simuler la génération
        "build_output.js"
    }, "generate")

    # Pipeline complet
    build_all = (sources) => {
        parsed = parse_sources(sources)
        opt = optimize(parsed)
        generate(opt)
    }

    # Premier build complet
    print("Premier build complet:")
    output1 = build_all(list("source.txt"))
    print("  ✓ Build terminé:", output1)

    # Deuxième build : tout est en cache
    print("\\nDeuxième build (tout en cache):")
    output2 = build_all(list("source.txt"))
    print("  ✓ Build terminé:", output2)

    print("\\nStatistiques:")
    stats = _cache.stats()
    print("  Cache hits:", stats["hits"])
    print("  Cache misses:", stats["misses"])
    print("  Hit rate:", stats["hit_rate"])
    """

    cat.parse(code)
    cat.execute()

    # Cleanup
    Path('source.txt').unlink()
    print()


def main():
    print("╔════════════════════════════════════════════════╗")
    print("║  Cache de fonctions pour scripts de build     ║")
    print("╚════════════════════════════════════════════════╝\n")

    example_simple()
    example_with_custom_key()
    example_invalidation()
    example_build_pipeline()

    print("✓ Tous les exemples terminés !")
    print("\nPour utiliser un cache persistant (DiskCache, Redis),")
    print("voir les exemples dans examples/cache_*_example.py")


if __name__ == "__main__":
    main()
