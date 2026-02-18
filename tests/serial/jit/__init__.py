# FILE: tests/serial/jit/__init__.py
"""
Tests JIT pour Catnip.

Cette suite de tests vérifie le fonctionnement du compilateur JIT :
- Détection et compilation des hot loops
- Correction fonctionnelle du code généré

Structure :
- test_compilation.py : Détection, tracing, compilation (7 tests)
- test_execution.py : Correction fonctionnelle (5 tests)

NOTE : Ces tests sont dans tests/serial/ car le JIT a un état global.
       La suite est volontairement minimaliste pour éviter les deadlocks.
       Tests edge cases et performance sont couverts par les tests manuels
       dans /tmp/test_jit_*.py
"""
