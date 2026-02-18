#!/usr/bin/env python3
"""Analyse de l'IR Catnip via sérialisation JSON.

Démontre comment utiliser --format json (serde JSON complet) pour analyser
programmatiquement la structure de l'IR et extraire des métriques.

Note : --format json produit le format serde avec tagged enums ({"Op": {"opcode": ...}}).
Le format par défaut (text) produit du compact JSON ({"op": "Add", "args": [...]}).
"""

import json
import subprocess
from collections import Counter


def get_ir_json(code: str, level: int = 1) -> list:
    """Récupère l'IR en JSON via la CLI."""
    result = subprocess.run(
        ["catnip", "-p", str(level), "--format", "json", "-c", code],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(result.stdout)


def count_opcodes(ir_list: list) -> Counter:
    """Compte les opcodes utilisés dans l'IR."""
    counter = Counter()

    def visit(node):
        if isinstance(node, dict):
            if "Op" in node:
                op = node["Op"]
                counter[op["opcode"]] += 1
                # Visiter args et kwargs
                for arg in op.get("args", []):
                    visit(arg)
                for val in op.get("kwargs", {}).values():
                    visit(val)
            else:
                # Visiter récursivement
                for val in node.values():
                    visit(val)
        elif isinstance(node, list):
            for item in node:
                visit(item)

    for ir_node in ir_list:
        visit(ir_node)
    return counter


def analyze_complexity(ir_list: list) -> dict:
    """Analyse la complexité de l'IR."""
    metrics = {
        "total_nodes": 0,
        "max_depth": 0,
        "tail_calls": 0,
    }

    def visit(node, depth=0):
        metrics["total_nodes"] += 1
        metrics["max_depth"] = max(metrics["max_depth"], depth)

        if isinstance(node, dict):
            if "Op" in node:
                op = node["Op"]
                if op.get("tail"):
                    metrics["tail_calls"] += 1
                for arg in op.get("args", []):
                    visit(arg, depth + 1)
                for val in op.get("kwargs", {}).values():
                    visit(val, depth + 1)
            else:
                for val in node.values():
                    visit(val, depth + 1)
        elif isinstance(node, list):
            for item in node:
                visit(item, depth)

    for ir_node in ir_list:
        visit(ir_node)
    return metrics


def find_pattern(ir_list: list, opcode: str) -> list:
    """Trouve tous les usages d'un opcode spécifique."""
    matches = []

    def visit(node, path=""):
        if isinstance(node, dict):
            if "Op" in node:
                op = node["Op"]
                current_path = f"{path}/{op['opcode']}"
                if op["opcode"] == opcode:
                    matches.append({
                        "path": current_path,
                        "start_byte": op.get("start_byte"),
                        "end_byte": op.get("end_byte"),
                        "args_count": len(op.get("args", [])),
                    })
                for i, arg in enumerate(op.get("args", [])):
                    visit(arg, f"{current_path}/arg[{i}]")
            else:
                for key, val in node.items():
                    visit(val, f"{path}/{key}")
        elif isinstance(node, list):
            for i, item in enumerate(node):
                visit(item, f"{path}[{i}]")

    for ir_node in ir_list:
        visit(ir_node)
    return matches


def main():
    # Exemple 1 : Compter les opcodes
    print("⇒ Exemple 1 : Analyse d'opcodes")
    code1 = "x = 10; y = x + 5; z = y * 2"
    ir1 = get_ir_json(code1)
    opcodes = count_opcodes(ir1)
    print(f"Code: {code1}")
    print("Opcodes utilisés:")
    for op, count in opcodes.most_common():
        print(f"  {op}: {count}")
    print()

    # Exemple 2 : Analyse de complexité
    print("⇒ Exemple 2 : Métriques de complexité")
    code2 = "f = (x) => { if (x > 0) { x + f(x - 1) } else { 0 } }"
    ir2 = get_ir_json(code2, level=2)  # Niveau 2 pour voir optimisations
    metrics = analyze_complexity(ir2)
    print(f"Code: {code2}")
    print(f"Total nodes: {metrics['total_nodes']}")
    print(f"Max depth: {metrics['max_depth']}")
    print(f"Tail calls: {metrics['tail_calls']}")
    print()

    # Exemple 3 : Recherche de pattern
    print("⇒ Exemple 3 : Recherche de patterns")
    code3 = "a = 1 + 2; b = 3 + 4; c = a + b"
    ir3 = get_ir_json(code3)
    additions = find_pattern(ir3, "Add")
    print(f"Code: {code3}")
    print(f"Trouvé {len(additions)} additions:")
    for match in additions:
        print(f"  {match['path']} (bytes {match['start_byte']}-{match['end_byte']})")
    print()

    # Exemple 4 : Comparaison avant/après optimisations
    print("⇒ Exemple 4 : Avant/Après optimisations")
    code4 = "x = 2 + 3; y = x * 1; z = y + 0"
    ir_before = get_ir_json(code4, level=1)
    ir_after = get_ir_json(code4, level=2)
    print(f"Code: {code4}")
    print(f"Nodes avant optimisation: {analyze_complexity(ir_before)['total_nodes']}")
    print(f"Nodes après optimisation: {analyze_complexity(ir_after)['total_nodes']}")


if __name__ == "__main__":
    main()
