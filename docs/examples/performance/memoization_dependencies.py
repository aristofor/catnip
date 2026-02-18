#!/usr/bin/env python3
"""
Exemple d'utilisation de la memoization avec validation basée sur dépendances.

Similaire au pattern de BuildParserLegacy où les commandes déterminent
si le cache est valide basé sur les fichiers sources.
"""

import hashlib
import time
from pathlib import Path

from catnip import Catnip


def example_file_timestamp_cache():
    """Exemple : invalidation automatique basée sur timestamp de fichiers."""
    print("⇒ Exemple 1 : Cache avec timestamp de fichiers")

    # Créer des fichiers de test
    src1 = Path('src1.txt')
    src2 = Path('src2.txt')
    src1.write_text('content 1')
    src2.write_text('content 2')

    cat = Catnip()

    # Fonction de clé qui inclut les mtimes des fichiers
    def file_timestamp_key(*files):
        """Génère une clé basée sur les timestamps des fichiers."""
        timestamps = []
        for f in files:
            p = Path(f)
            if p.exists():
                timestamps.append(f"{f}:{p.stat().st_mtime}")
            else:
                timestamps.append(f"{f}:missing")
        return '|'.join(sorted(timestamps))

    cat.context.globals['file_timestamp_key'] = file_timestamp_key

    code = """
    builds = 0

    # Build intelligent qui détecte les changements de fichiers
    smart_build = cached((files) => { builds = builds + 1; print("  → Build #" + str(builds) + " pour:", files); "output_" + str(builds) + ".js" }, "smart_build", file_timestamp_key)

    # Premier build
    print("Premier build:")
    out1 = smart_build(list("src1.txt", "src2.txt"))

    # Deuxième build (fichiers inchangés) : cache hit
    print("\\nDeuxième build (fichiers inchangés):")
    out2 = smart_build(list("src1.txt", "src2.txt"))

    list(out1, out2, builds)
    """

    cat.parse(code)
    result = cat.execute()
    print(f"Résultat: {result}\n")

    # Modifier un fichier
    print("Modification de src1.txt…")
    time.sleep(0.01)
    src1.write_text('modified content 1')

    # Nouveau build avec le même cache
    cat2 = Catnip()
    cat2.context.memoization = cat.context.memoization  # Réutiliser le même cache
    cat2.context.globals['file_timestamp_key'] = file_timestamp_key

    code2 = """
    builds = 1

    smart_build = cached(
        (files) => {
            builds = builds + 1
            print("  → Build #" + str(builds) + " pour:", files)
            "output_" + str(builds) + ".js"
        },
        "smart_build",
        file_timestamp_key
    )

    # Build après modification : cache miss car timestamp a changé
    print("\\nTroisième build (src1.txt modifié):")
    out3 = smart_build(list("src1.txt", "src2.txt"))

    list(out3, builds)
    """

    cat2.parse(code2)
    result2 = cat2.execute()
    print(f"Résultat: {result2}\n")

    # Cleanup
    src1.unlink()
    src2.unlink()


def example_content_hash_cache():
    """Exemple : invalidation basée sur le hash du contenu des fichiers."""
    print("⇒ Exemple 2 : Cache avec hash de contenu")

    # Créer des fichiers de test
    files = {
        'style.scss': b'body { color: red; }',
        'theme.scss': b'$primary: #007bff;',
    }

    for name, content in files.items():
        Path(name).write_bytes(content)

    cat = Catnip()

    # Fonction de clé basée sur le hash du contenu
    def content_hash_key(*file_paths):
        """Génère une clé basée sur le hash du contenu des fichiers."""
        hasher = hashlib.sha256()
        for fp in sorted(file_paths):
            p = Path(fp)
            if p.exists():
                hasher.update(p.read_bytes())
            else:
                hasher.update(b'missing')
        return hasher.hexdigest()

    cat.context.globals['content_hash_key'] = content_hash_key

    code = """
    compilations = 0

    # Compilation SASS avec cache basé sur contenu
    compile_sass = cached(
        (sources) => {
            compilations = compilations + 1
            print("  → Compilation #" + str(compilations))
            "output.css"
        },
        "compile_sass",
        content_hash_key
    )

    # Premier build
    print("Première compilation:")
    css1 = compile_sass(list("style.scss", "theme.scss"))

    # Deuxième build : cache hit
    print("\\nDeuxième compilation (contenu identique):")
    css2 = compile_sass(list("style.scss", "theme.scss"))

    list(css1, css2, compilations)
    """

    cat.parse(code)
    result = cat.execute()
    print(f"Résultat: {result}\n")

    # Modifier le contenu
    print("Modification du contenu de style.scss…")
    Path('style.scss').write_bytes(b'body { color: blue; }')

    # Nouveau build
    cat2 = Catnip()
    cat2.context.memoization = cat.context.memoization
    cat2.context.globals['content_hash_key'] = content_hash_key

    code2 = """
    compilations = 1

    compile_sass = cached(
        (sources) => {
            compilations = compilations + 1
            print("  → Compilation #" + str(compilations))
            "output.css"
        },
        "compile_sass",
        content_hash_key
    )

    # Build après modification : cache miss
    print("\\nTroisième compilation (contenu modifié):")
    css3 = compile_sass(list("style.scss", "theme.scss"))

    list(css3, compilations)
    """

    cat2.parse(code2)
    result2 = cat2.execute()
    print(f"Résultat: {result2}\n")

    # Cleanup
    Path('style.scss').unlink()
    Path('theme.scss').unlink()


def example_validator_with_external_state():
    """Exemple : validation basée sur état externe."""
    print("⇒ Exemple 3 : Validation avec état externe")

    # État externe simulant une configuration de build
    build_config = {'production': False, 'version': '1.0.0'}

    cat = Catnip()

    # Validator qui vérifie si la config n'a pas changé
    def validate_config(cached_result, *args, **kwargs):
        """Valide que le cache est compatible avec la config actuelle."""
        # En production, invalider tous les caches de dev
        if build_config['production']:
            return False
        return True

    cat.context.globals['validate_config'] = validate_config

    code = """
    builds = 0

    # Build avec validation selon la config
    build_app = cached(
        (entry) => {
            builds = builds + 1
            print("  → Build #" + str(builds))
            "app_" + str(builds) + ".js"
        },
        "build_app",
        None,
        validate_config
    )

    # Premier build en mode dev
    print("Build en mode développement:")
    app1 = build_app("main.js")

    # Deuxième build : cache hit
    print("\\nDeuxième build (mode dev, cache hit):")
    app2 = build_app("main.js")

    list(app1, app2, builds)
    """

    cat.parse(code)
    result = cat.execute()
    print(f"Résultat: {result}\n")

    # Passer en mode production
    print("Passage en mode production…")
    build_config['production'] = True

    # Nouveau build
    cat2 = Catnip()
    cat2.context.memoization = cat.context.memoization
    cat2.context.globals['validate_config'] = validate_config

    code2 = """
    builds = 1

    build_app = cached(
        (entry) => {
            builds = builds + 1
            print("  → Build #" + str(builds))
            "app_" + str(builds) + ".js"
        },
        "build_app",
        None,
        validate_config
    )

    # Build en production : validator invalide le cache
    print("\\nTroisième build (mode production, cache invalidé):")
    app3 = build_app("main.js")

    list(app3, builds)
    """

    cat2.parse(code2)
    result2 = cat2.execute()
    print(f"Résultat: {result2}\n")


def example_buildparser_pattern():
    """Exemple complet imitant le pattern BuildParserLegacy."""
    print("⇒ Exemple 4 : Pattern complet (comme BuildParserLegacy)")

    # Simuler des fichiers sources
    sources = {
        'index.html': b'<html>…</html>',
        'style.css': b'body { … }',
        'script.js': b'console.log("hello");',
    }

    for name, content in sources.items():
        Path(name).write_bytes(content)

    cat = Catnip()

    # Fonction pour calculer les dépendances (comme get_cache_dependencies)
    def get_dependencies(*files):
        """Retourne les dépendances pour la clé de cache."""
        deps = []
        for f in files:
            p = Path(f)
            if p.exists():
                # Inclure taille et mtime
                stat = p.stat()
                deps.append(f"{f}:{stat.st_size}:{stat.st_mtime}")
        return deps

    # Fonction de clé qui inclut les dépendances
    def build_cache_key(*files):
        """Génère une clé de cache incluant les dépendances."""
        deps = get_dependencies(*files)
        key_str = '|'.join(sorted(deps))
        return hashlib.sha256(key_str.encode()).hexdigest()[:16]

    # Validator qui vérifie que les fichiers existent toujours
    def validate_build(cached_result, *files):
        """Valide que tous les fichiers sources existent encore."""
        for f in files:
            if not Path(f).exists():
                print(f"  → Cache invalide : {f} n'existe plus")
                return False
        return True

    cat.context.globals['build_cache_key'] = build_cache_key
    cat.context.globals['validate_build'] = validate_build

    code = """
    builds = 0

    # Commande de build avec cache intelligent
    bundle = cached(
        (sources) => {
            builds = builds + 1
            print("  → Bundling sources (build #" + str(builds) + ")…")
            "dist/bundle_" + str(builds) + ".js"
        },
        "bundle",
        build_cache_key,
        validate_build
    )

    # Premier build
    print("Premier build:")
    out1 = bundle(list("index.html", "style.css", "script.js"))

    # Deuxième build : cache hit
    print("\\nDeuxième build (cache hit):")
    out2 = bundle(list("index.html", "style.css", "script.js"))

    list(out1, out2, builds)
    """

    cat.parse(code)
    result = cat.execute()
    print(f"Résultat: {result}\n")

    # Modifier un fichier
    print("Modification de script.js…")
    time.sleep(0.01)
    Path('script.js').write_bytes(b'console.log("modified");')

    # Nouveau build
    cat2 = Catnip()
    cat2.context.memoization = cat.context.memoization
    cat2.context.globals['build_cache_key'] = build_cache_key
    cat2.context.globals['validate_build'] = validate_build

    code2 = """
    builds = 1

    bundle = cached(
        (sources) => {
            builds = builds + 1
            print("  → Bundling sources (build #" + str(builds) + ")…")
            "dist/bundle_" + str(builds) + ".js"
        },
        "bundle",
        build_cache_key,
        validate_build
    )

    # Build après modification : cache miss
    print("\\nTroisième build (fichier modifié):")
    out3 = bundle(list("index.html", "style.css", "script.js"))

    list(out3, builds)
    """

    cat2.parse(code2)
    result2 = cat2.execute()
    print(f"Résultat: {result2}\n")

    # Cleanup
    Path('index.html').unlink()
    Path('style.css').unlink()
    Path('script.js').unlink()


def main():
    print("╔══════════════════════════════════════════════════════╗")
    print("║  Cache avec dépendances (pattern BuildParserLegacy) ║")
    print("╚══════════════════════════════════════════════════════╝\n")

    example_file_timestamp_cache()
    example_content_hash_cache()
    example_validator_with_external_state()
    example_buildparser_pattern()

    print("✓ Tous les exemples terminés !")
    print("\nCes patterns permettent de créer des commandes de build")
    print("intelligentes qui détectent automatiquement quand recalculer.")


if __name__ == "__main__":
    main()
