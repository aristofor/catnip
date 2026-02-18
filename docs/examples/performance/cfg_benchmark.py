#!/usr/bin/env python3
"""
Benchmark CFG - Gains Potentiels

Mesure ce que le CFG apporte en termes d'optimisations structurelles
par rapport aux passes IR actuelles.
"""

import time
import statistics
from catnip import Catnip
import catnip._rs as rs


def analyze_cfg(code, name="test"):
    """Analyse un code avec CFG et retourne les statistiques."""
    cat = Catnip(optimize=0)  # Pas d'optimisations IR
    ast = cat.parse(code, semantic=False)

    # Construire CFG
    cfg = rs.cfg.build_cfg_from_ir(ast, name)

    # Stats avant optimisation
    blocks_before = cfg.num_blocks
    edges_before = cfg.num_edges

    # Calculer dominateurs (requis pour détection loops)
    cfg.compute_dominators()

    # Détecter loops
    loops = cfg.detect_loops()

    # Détecter dead code avant optimisation
    unreachable_before = cfg.get_unreachable_blocks()

    # Appliquer optimisations CFG
    dead, merged, empty, branches = cfg.optimize()

    # Stats après optimisation
    blocks_after = cfg.num_blocks
    edges_after = cfg.num_edges

    return {
        'blocks_before': blocks_before,
        'blocks_after': blocks_after,
        'edges_before': edges_before,
        'edges_after': edges_after,
        'loops': len(loops),
        'unreachable_before': len(unreachable_before),
        'dead_removed': dead,
        'blocks_merged': merged,
        'empty_removed': empty,
        'branches_eliminated': branches,
        'reduction_blocks': (blocks_before - blocks_after) / blocks_before * 100 if blocks_before > 0 else 0,
        'reduction_edges': (edges_before - edges_after) / edges_before * 100 if edges_before > 0 else 0,
    }


def main():
    print("=" * 80)
    print("BENCHMARK CFG - GAINS D'OPTIMISATIONS")
    print("=" * 80)

    # Test 1: Code avec dead code
    print("\n" + "=" * 80)
    print("TEST 1: Dead Code Elimination")
    print("=" * 80)

    dead_code = """
x = 10
if False {
    y = 20
    z = 30
    w = y + z
}
result = x + 5
result
"""

    print("\nCode avec dead code:")
    print(dead_code)

    stats = analyze_cfg(dead_code, "dead_code")

    print(f"\nStatistiques CFG:")
    print(f"  Blocs avant:        {stats['blocks_before']}")
    print(f"  Blocs après:        {stats['blocks_after']}")
    print(f"  Edges avant:        {stats['edges_before']}")
    print(f"  Edges après:        {stats['edges_after']}")
    print(f"\nOptimisations appliquées:")
    print(f"  Dead blocks:        {stats['dead_removed']}")
    print(f"  Blocks merged:      {stats['blocks_merged']}")
    print(f"  Empty removed:      {stats['empty_removed']}")
    print(f"  Branches elim:      {stats['branches_eliminated']}")
    print(f"\nRéduction:")
    print(f"  Blocs: {stats['reduction_blocks']:.1f}%")
    print(f"  Edges: {stats['reduction_edges']:.1f}%")

    # Test 2: Branches vers même cible
    print("\n" + "=" * 80)
    print("TEST 2: Constant Branch Elimination")
    print("=" * 80)

    const_branch = """
x = 5
if x > 0 {
    y = 10
} else {
    y = 10
}
y
"""

    print("\nCode avec branches vers même valeur:")
    print(const_branch)

    stats = analyze_cfg(const_branch, "const_branch")

    print(f"\nStatistiques CFG:")
    print(f"  Blocs avant:        {stats['blocks_before']}")
    print(f"  Blocs après:        {stats['blocks_after']}")
    print(f"  Edges avant:        {stats['edges_before']}")
    print(f"  Edges après:        {stats['edges_after']}")
    print(f"\nOptimisations appliquées:")
    print(f"  Dead blocks:        {stats['dead_removed']}")
    print(f"  Blocks merged:      {stats['blocks_merged']}")
    print(f"  Empty removed:      {stats['empty_removed']}")
    print(f"  Branches elim:      {stats['branches_eliminated']}")
    print(f"\nRéduction:")
    print(f"  Blocs: {stats['reduction_blocks']:.1f}%")
    print(f"  Edges: {stats['reduction_edges']:.1f}%")

    # Test 3: Boucle avec code linéaire
    print("\n" + "=" * 80)
    print("TEST 3: Block Merging (séquences linéaires)")
    print("=" * 80)

    linear_code = """
a = 1
b = 2
c = a + b
d = c * 2
e = d + 5
e
"""

    print("\nCode linéaire (séquence sans branchement):")
    print(linear_code)

    stats = analyze_cfg(linear_code, "linear")

    print(f"\nStatistiques CFG:")
    print(f"  Blocs avant:        {stats['blocks_before']}")
    print(f"  Blocs après:        {stats['blocks_after']}")
    print(f"  Edges avant:        {stats['edges_before']}")
    print(f"  Edges après:        {stats['edges_after']}")
    print(f"\nOptimisations appliquées:")
    print(f"  Dead blocks:        {stats['dead_removed']}")
    print(f"  Blocks merged:      {stats['blocks_merged']}")
    print(f"  Empty removed:      {stats['empty_removed']}")
    print(f"  Branches elim:      {stats['branches_eliminated']}")
    print(f"\nRéduction:")
    print(f"  Blocs: {stats['reduction_blocks']:.1f}%")
    print(f"  Edges: {stats['reduction_edges']:.1f}%")

    # Test 4: Loop detection
    print("\n" + "=" * 80)
    print("TEST 4: Loop Detection")
    print("=" * 80)

    loop_code = """
sum = 0
i = 0
while i < 10 {
    sum = sum + i
    i = i + 1
}
sum
"""

    print("\nCode avec boucle:")
    print(loop_code)

    stats = analyze_cfg(loop_code, "loop")

    print(f"\nStatistiques CFG:")
    print(f"  Blocs avant:        {stats['blocks_before']}")
    print(f"  Blocs après:        {stats['blocks_after']}")
    print(f"  Edges avant:        {stats['edges_before']}")
    print(f"  Edges après:        {stats['edges_after']}")
    print(f"  Loops détectées:    {stats['loops']}")
    print(f"\nOptimisations appliquées:")
    print(f"  Dead blocks:        {stats['dead_removed']}")
    print(f"  Blocks merged:      {stats['blocks_merged']}")
    print(f"  Empty removed:      {stats['empty_removed']}")
    print(f"  Branches elim:      {stats['branches_eliminated']}")
    print(f"\nRéduction:")
    print(f"  Blocs: {stats['reduction_blocks']:.1f}%")
    print(f"  Edges: {stats['reduction_edges']:.1f}%")

    # Test 5: Code complexe (if imbriqués + loop)
    print("\n" + "=" * 80)
    print("TEST 5: Code Complexe")
    print("=" * 80)

    complex_code = """
sum = 0
for i in range(1, 100) {
    if i % 2 == 0 {
        if i % 4 == 0 {
            sum = sum + i
        }
    } else {
        sum = sum + 1
    }
}
sum
"""

    print("\nCode complexe (loop + if imbriqués):")
    print(complex_code)

    stats = analyze_cfg(complex_code, "complex")

    print(f"\nStatistiques CFG:")
    print(f"  Blocs avant:        {stats['blocks_before']}")
    print(f"  Blocs après:        {stats['blocks_after']}")
    print(f"  Edges avant:        {stats['edges_before']}")
    print(f"  Edges après:        {stats['edges_after']}")
    print(f"  Loops détectées:    {stats['loops']}")
    print(f"\nOptimisations appliquées:")
    print(f"  Dead blocks:        {stats['dead_removed']}")
    print(f"  Blocks merged:      {stats['blocks_merged']}")
    print(f"  Empty removed:      {stats['empty_removed']}")
    print(f"  Branches elim:      {stats['branches_eliminated']}")
    print(f"\nRéduction:")
    print(f"  Blocs: {stats['reduction_blocks']:.1f}%")
    print(f"  Edges: {stats['reduction_edges']:.1f}%")

    # Résumé
    print("\n" + "=" * 80)
    print("RÉSUMÉ - CE QUE LE CFG APPORTE")
    print("=" * 80)

    print("\n1. Optimisations structurelles:")
    print("   • Dead code elimination au niveau CFG (pas seulement IR)")
    print("   • Block merging (fusion séquences linéaires)")
    print("   • Empty block removal (simplification du graphe)")
    print("   • Constant branch elimination (if → goto)")

    print("\n2. Analyses avancées:")
    print("   • Détection de loops naturels (via back-edges)")
    print("   • Calcul de dominance (Cooper-Harvey-Kennedy)")
    print("   • Détection de code inaccessible")
    print("   • Analyse de flot de contrôle complet")

    print("\n3. Bénéfices potentiels:")
    print("   • Réduction de la taille du graphe (blocs + edges)")
    print("   • Simplification de la structure de contrôle")
    print("   • Détection de patterns d'optimisation impossibles au niveau IR")

    print("\n4. Intégration actuelle:")
    print("   • CFG intégré au pipeline d'optimisation (optimize=3)")
    print("   • CFG builder gère IR avec raw values et Op nodes")
    print("   • Reconstruction fonctionnelle via region detection")
    print("   • Disponible pour analyse manuelle ET optimisation automatique")

    print("\n5. Pipeline actuel (optimize=3):")
    print("   IR → IR Optimizations → CFG → CFG Optimizations → Reconstruction → Semantic → Op")
    print("\n   Pipeline allégé (optimize=0-2):")
    print("   IR → IR Optimizations → Semantic → Op")

    print("\n" + "=" * 80)
    print("COMPARAISON IR PASSES vs CFG OPTIMIZATIONS")
    print("=" * 80)

    print("\nIR Passes (actuelles - 6 passes actives):")
    print("  ✓ BluntCode - Simplification de patterns")
    print("  ✓ ConstantFolding - Calculs constants")
    print("  ✓ StrengthReduction - Opérations plus rapides")
    print("  ✓ BlockFlattening - Aplatissement de blocs")
    print("  ✓ DeadCodeElimination - Code mort IR")
    print("  ✓ CSE - Sous-expressions communes")
    print("  Niveau: Expressions et statements")
    print("  Overhead: <0.01ms")

    print("\nCFG Optimizations (intégrées via optimize=3):")
    print("  ✓ Dead code elimination - Blocs inaccessibles")
    print("  ✓ Block merging - Fusion séquences")
    print("  ✓ Empty block removal - Simplification graphe")
    print("  ✓ Constant branch elimination - Branches redondantes")
    print("  Niveau: Structure de contrôle")
    print("  Overhead: Inconnu (non benchmarké en production)")

    print("\nComplémentarité:")
    print("  • IR passes: optimisations locales (expressions, statements)")
    print("  • CFG passes: optimisations globales (flot de contrôle)")
    print("  • Les deux sont utiles et complémentaires")
    print("  • CFG détecte des patterns invisibles au niveau IR")


if __name__ == "__main__":
    main()
