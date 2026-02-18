# FILE: tests/bytecode/helpers.py
"""
Helpers pour tester l'AST et le code transformé.

Facilite l'inspection et la validation de l'AST après transformation et analyse sémantique.
Note: Le bytecode VM n'est pas directement accessible depuis Python.
"""

from typing import Any

from catnip import Catnip
from catnip.nodes import Op, Ref


class ASTInspector:
    """Helper pour inspecter et valider l'AST transformé."""

    def __init__(self, source: str):
        self.source = source
        self.catnip = Catnip()
        self.ast = None
        self.analyzed_code = None

    def get_ast(self):
        """Parse le code et retourne l'AST après transformation (sans semantic)."""
        self.ast = self.catnip.parse(self.source, semantic=False)
        return self.ast

    def get_analyzed_code(self):
        """Parse et analyse sémantiquement (avec semantic)."""
        self.analyzed_code = self.catnip.parse(self.source, semantic=True)
        return self.analyzed_code

    def execute(self):
        """Parse et exécute le code."""
        self.catnip.parse(self.source, semantic=True)
        return self.catnip.execute()

    def assert_contains_ref(self, var_name: str):
        """Vérifie que l'AST contient une référence à la variable."""
        if self.ast is None:
            self.get_ast()

        refs = self._find_refs_in_node(self.ast, var_name)
        assert refs, f"Variable '{var_name}' not found as Ref in AST"
        return refs

    def assert_contains_op(self, opcode: int, min_count: int = 1):
        """Vérifie que l'AST contient au moins min_count occurrences d'un opcode."""
        if self.ast is None:
            self.get_ast()

        ops = self._find_ops_by_opcode(self.ast, opcode)
        assert (
            len(ops) >= min_count
        ), f"Expected at least {min_count} occurrence(s) of opcode {opcode}, found {len(ops)}"
        return ops

    def get_summary(self) -> str:
        """Retourne un résumé lisible de l'AST pour debugging."""
        if self.ast is None:
            self.get_ast()

        lines = []
        lines.append(f"=== AST for: {self.source[:50]}... ===")
        lines.append(str(self.ast))
        return "\n".join(lines)

    def _find_refs_in_node(self, node, var_name: str) -> list:
        """Trouve récursivement toutes les références à une variable."""
        refs = []

        if isinstance(node, Ref):
            if node.ident == var_name:
                refs.append(node)
        elif isinstance(node, Op):
            # Parcourir les args
            if hasattr(node, 'args') and node.args:
                if isinstance(node.args, (list, tuple)):
                    for arg in node.args:
                        refs.extend(self._find_refs_in_node(arg, var_name))
                else:
                    refs.extend(self._find_refs_in_node(node.args, var_name))
        elif isinstance(node, (list, tuple)):
            for item in node:
                refs.extend(self._find_refs_in_node(item, var_name))

        return refs

    def _find_ops_by_opcode(self, node, opcode: int) -> list:
        """Trouve récursivement tous les Op/IR avec un opcode donné."""
        from catnip.transformer import IR

        ops = []

        if isinstance(node, IR):
            # IR nodes have 'ident' as the opcode
            if node.ident == opcode:
                ops.append(node)
            # Parcourir les args
            if hasattr(node, 'args') and node.args:
                if isinstance(node.args, (list, tuple)):
                    for arg in node.args:
                        ops.extend(self._find_ops_by_opcode(arg, opcode))
                else:
                    ops.extend(self._find_ops_by_opcode(node.args, opcode))
        elif isinstance(node, Op):
            # Op nodes have 'opcode'
            if node.opcode == opcode:
                ops.append(node)
            # Parcourir les args
            if hasattr(node, 'args') and node.args:
                if isinstance(node.args, (list, tuple)):
                    for arg in node.args:
                        ops.extend(self._find_ops_by_opcode(arg, opcode))
                else:
                    ops.extend(self._find_ops_by_opcode(node.args, opcode))
        elif isinstance(node, (list, tuple)):
            for item in node:
                ops.extend(self._find_ops_by_opcode(item, opcode))

        return ops


def inspect_ast(source: str) -> ASTInspector:
    """Shortcut pour créer un inspector et récupérer l'AST."""
    inspector = ASTInspector(source)
    inspector.get_ast()
    return inspector


def execute_and_inspect(source: str) -> tuple[Any, ASTInspector]:
    """Parse, exécute et retourne (résultat, inspector) pour validation."""
    inspector = ASTInspector(source)
    result = inspector.execute()
    return result, inspector
