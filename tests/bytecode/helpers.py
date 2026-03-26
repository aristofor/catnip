# FILE: tests/bytecode/helpers.py
"""
Helpers pour tester l'AST et le code transformé.

Facilite l'inspection et la validation de l'AST après transformation et analyse sémantique.
Note: Le bytecode VM n'est pas directement accessible depuis Python.
"""

from catnip import Catnip


class ASTInspector:
    """Helper pour inspecter et valider l'AST transformé (via PyIRNode)."""

    def __init__(self, source: str):
        self.source = source
        self.catnip = Catnip()
        self.ast = None
        self.analyzed_code = None

    def get_ast(self):
        """Parse le code et retourne l'AST après transformation (sans semantic)."""
        self.ast = self.catnip._pipeline.parse_to_ir(self.source, False)
        return self.ast

    def get_analyzed_code(self):
        """Parse et analyse sémantiquement (avec semantic)."""
        self.analyzed_code = self.catnip._pipeline.parse_to_ir(self.source, True)
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

        if isinstance(node, (list, tuple)):
            for item in node:
                refs.extend(self._find_refs_in_node(item, var_name))
        elif hasattr(node, 'kind'):
            if node.kind == 'Ref' and node.name == var_name:
                refs.append(node)
            else:
                for arg in node.args:
                    refs.extend(self._find_refs_in_node(arg, var_name))

        return refs

    def _find_ops_by_opcode(self, node, opcode: int) -> list:
        """Trouve récursivement tous les nodes avec un opcode donné."""
        from catnip.semantic.opcode import OpCode

        ops = []
        # Convert SCREAMING_SNAKE to PascalCase for PyIRNode comparison
        enum_name = OpCode(opcode).name
        target_name = ''.join(w.capitalize() for w in enum_name.split('_'))

        if isinstance(node, (list, tuple)):
            for item in node:
                ops.extend(self._find_ops_by_opcode(item, opcode))
        elif hasattr(node, 'kind'):
            if node.kind == 'Op' and node.opcode == target_name:
                ops.append(node)
            for arg in node.args or []:
                ops.extend(self._find_ops_by_opcode(arg, opcode))

        return ops


def inspect_ast(source: str) -> ASTInspector:
    """Shortcut pour créer un inspector et récupérer l'AST."""
    inspector = ASTInspector(source)
    inspector.get_ast()
    return inspector
