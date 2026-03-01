# FILE: tests/bytecode/__init__.py
"""
Tests de bytecode intermédiaire.

Vérifie que le pipeline génère le bon bytecode:
- Transformer: AST → IR correct
- Semantic: IR → IR optimisé
- Compiler: IR → Bytecode correct
"""
