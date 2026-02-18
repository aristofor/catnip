#!/usr/bin/env python3
"""
Démonstration de l'utilisation de catnip.tools.extract_grammar.

Ce module montre comment utiliser l'extracteur de grammaire pour :
- Extraire les keywords, operators, terminaux et règles
- Exporter en JSON
- Générer un lexer Pygments à jour
"""

from pathlib import Path
from catnip.tools.extract_grammar import GrammarExtractor


def main():
    print("=" * 80)
    print("DÉMONSTRATION: catnip.tools.extract_grammar")
    print("=" * 80)
    print()

    # Initialisation de l'extracteur
    extractor = GrammarExtractor()
    print(f"Grammaire chargée depuis: {extractor.grammar_path}")
    print()

    # Extraction des keywords
    print("1. EXTRACTION DES KEYWORDS")
    print("-" * 80)
    keywords = extractor.extract_keywords()
    print(f"Control flow: {', '.join(keywords['control_flow'])}")
    print(f"Constants: {', '.join(keywords['constants'])}")
    print(f"Types: {', '.join(keywords['types'])}")
    print()

    # Extraction des operators
    print("2. EXTRACTION DES OPERATORS")
    print("-" * 80)
    operators = extractor.extract_operators()
    print(f"Arithmetic: {', '.join(operators['arithmetic'])}")
    print(f"Comparison: {', '.join(operators['comparison'])}")
    print(f"Bitwise: {', '.join(operators['bitwise'])}")
    print(f"Logical: {', '.join(operators['logical'])}")
    print(f"Special: {', '.join(operators['special'])}")
    print()

    # Extraction des terminaux
    print("3. EXTRACTION DES TERMINAUX")
    print("-" * 80)
    terminals = extractor.extract_terminals()
    print(f"Total terminaux: {len(terminals)}")
    print("Exemples:")
    for term in terminals[:5]:
        pattern = term['pattern'][:50] + "…" if len(term['pattern']) > 50 else term['pattern']
        print(f"  {term['name']:20} {pattern}")
    print()

    # Extraction des règles
    print("4. EXTRACTION DES RÈGLES")
    print("-" * 80)
    rules = extractor.extract_rules()
    print(f"Total règles: {len(rules)}")
    print("Exemples:")
    for rule in rules[:5]:
        expansion = ' '.join(rule['expansion'][:5])
        if len(rule['expansion']) > 5:
            expansion += " …"
        print(f"  {rule['name']:20} → {expansion}")
    print()

    # Export JSON
    print("5. EXPORT JSON")
    print("-" * 80)
    output_json = Path("/tmp/catnip_grammar_demo.json")
    extractor.to_json(output_json)
    print(f"JSON exporté vers: {output_json}")
    print(f"Taille: {output_json.stat().st_size} bytes")
    print()

    # Génération lexer Pygments
    print("6. GÉNÉRATION LEXER PYGMENTS")
    print("-" * 80)
    output_lexer = Path("/tmp/catnip_lexer_demo.py")
    extractor.generate_pygments_lexer(output_lexer)
    print(f"Lexer Pygments généré: {output_lexer}")
    print(f"Taille: {output_lexer.stat().st_size} bytes")
    print()

    # Extraction complète
    print("7. EXTRACTION COMPLÈTE")
    print("-" * 80)
    all_data = extractor.extract_all()
    print("Structure complète:")
    print(f"  - keywords: {len(all_data['keywords']['all'])} entrées")
    print(f"  - operators: {len(all_data['operators']['all'])} entrées")
    print(f"  - terminals: {len(all_data['terminals'])} entrées")
    print(f"  - rules: {len(all_data['rules'])} entrées")
    print(f"  - metadata: {all_data['metadata']}")
    print()

    print("=" * 80)
    print("Démonstration terminée!")
    print("=" * 80)


if __name__ == "__main__":
    main()
