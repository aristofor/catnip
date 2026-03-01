# FILE: catnip/tools/extract_grammar.py
"""
Programmatic extraction of Catnip grammar.

Extracts elements from Tree-sitter grammar (keywords, operators, terminals, rules)
to generate JSON exports and syntax highlighting configs.
"""

from pathlib import Path
from typing import Dict, List, Set, Any, Optional
import json
import re
import subprocess

import click


class GrammarExtractor:
    """Extracts and structures Catnip grammar elements."""

    def __init__(self, grammar_path: Optional[Path] = None):
        """Initialize the extractor with the grammar path.

        Args:
            grammar_path: Path to Tree-sitter grammar.json. If None, uses default path (Rust grammar).
        """
        if grammar_path is None:
            # Use Rust-generated grammar (single source of truth)
            project_root = Path(__file__).parent.parent.parent
            grammar_path = project_root / "catnip_grammar" / "src" / "grammar.json"

        self.grammar_path = grammar_path
        self.grammar_json = json.loads(grammar_path.read_text())

        # Load node-types.json for additional metadata
        self.node_types_path = grammar_path.parent / "node-types.json"
        if self.node_types_path.exists():
            self.node_types = json.loads(self.node_types_path.read_text())
        else:
            self.node_types = []

    def extract_all(self) -> Dict[str, Any]:
        """Extracts all grammar elements into a unified structure.

        Returns:
            Dictionary containing keywords, operators, terminals, rules, etc.
        """
        return {
            "keywords": self.extract_keywords(),
            "operators": self.extract_operators(),
            "terminals": self.extract_terminals(),
            "rules": self.extract_rules(),
            "metadata": {
                "source": str(self.grammar_path),
                "parser": "tree-sitter",
            },
        }

    # Classification sets: stable categories, rarely change
    _CONSTANTS = frozenset({"True", "False", "None"})
    _TYPES = frozenset({"list", "tuple", "dict", "set"})
    _PRAGMAS = frozenset({"pragma"})

    def extract_keywords(self) -> Dict[str, List[str]]:
        """Extracts grammar keywords, classified by category.

        Auto-extracts all alphabetic strings from grammar.json,
        then classifies them. Adding a keyword to grammar.js is enough.

        Returns:
            Dictionary with keyword categories: control_flow, constants, types, pragmas.
        """
        all_kw: Set[str] = set()
        self._extract_keyword_strings(self.grammar_json.get("rules", {}), all_kw)

        constants = all_kw & self._CONSTANTS
        types = all_kw & self._TYPES
        pragmas = all_kw & self._PRAGMAS
        control_flow = all_kw - constants - types - pragmas

        return {
            "control_flow": sorted(control_flow),
            "constants": sorted(constants),
            "types": sorted(types),
            "pragmas": sorted(pragmas),
            "all": sorted(all_kw),
        }

    def _extract_keyword_strings(self, obj: Any, keywords: Set[str]) -> None:
        """Recursively extract alphabetic keyword strings from grammar JSON."""
        if isinstance(obj, dict):
            if obj.get("type") == "STRING":
                value = obj.get("value", "")
                if value.isalpha() and len(value) > 1:
                    keywords.add(value)
            else:
                for v in obj.values():
                    self._extract_keyword_strings(v, keywords)
        elif isinstance(obj, list):
            for item in obj:
                self._extract_keyword_strings(item, keywords)

    def extract_operators(self) -> Dict[str, List[str]]:
        """Extracts grammar operators, classified by type.

        Returns:
            Dictionary with operator categories: arithmetic, comparison, bitwise, etc.
        """
        # Define operators (extracted from Tree-sitter grammar)
        arithmetic = {"+", "-", "*", "/", "//", "%", "**"}
        comparison = {"<", "<=", ">", ">=", "==", "!="}
        bitwise = {"&", "|", "^", "~", "<<", ">>"}
        logical = {"and", "or", "not"}
        special = {"=>", ".[", "="}  # arrow, broadcast, assignment

        # Extract all STRING values from grammar that look like operators
        found_operators = set()
        self._extract_operator_strings(self.grammar_json.get("rules", {}), found_operators)

        # Merge with known operators
        all_ops = arithmetic | comparison | bitwise | special | found_operators

        return {
            "arithmetic": sorted(arithmetic, key=lambda x: (len(x), x), reverse=True),
            "comparison": sorted(comparison, key=lambda x: (len(x), x), reverse=True),
            "bitwise": sorted(bitwise, key=lambda x: (len(x), x), reverse=True),
            "logical": sorted(logical),
            "special": sorted(special, key=lambda x: (len(x), x), reverse=True),
            "all": sorted(all_ops, key=lambda x: (len(x), x), reverse=True),
        }

    def _extract_operator_strings(self, obj: Any, operators: Set[str]) -> None:
        """Recursively extract operator STRING values from grammar JSON."""
        # Symbols that are delimiters or string/comment syntax, not operators
        excluded = {
            "{",
            "}",
            "(",
            ")",
            "[",
            "]",
            ",",
            ";",
            ":",
            ".",
            "_",
            "'",
            '"',
            "'''",
            '"""',
            "#",  # string/comment delimiters
        }
        if isinstance(obj, dict):
            if obj.get("type") == "STRING":
                value = obj.get("value", "")
                # Check if it's an operator (non-alphabetic, not delimiter)
                if value and not value.isalpha() and value not in excluded:
                    if len(value) <= 3 and not any(c in value for c in ["\\", "?"]):
                        operators.add(value)
            else:
                for v in obj.values():
                    self._extract_operator_strings(v, operators)
        elif isinstance(obj, list):
            for item in obj:
                self._extract_operator_strings(item, operators)

    def extract_terminals(self) -> List[Dict[str, Any]]:
        """Extracts terminals with their metadata from node-types.json.

        Returns:
            List of dictionaries containing name, named status for each terminal.
        """
        terminals = []

        # Terminal nodes are those without children in node-types.json
        for node in self.node_types:
            if node.get("named") and "children" not in node:
                terminals.append(
                    {
                        "name": node["type"],
                        "named": node.get("named", False),
                    }
                )

        return sorted(terminals, key=lambda x: x["name"])

    def extract_rules(self) -> List[Dict[str, Any]]:
        """Extracts grammar production rules from grammar.json.

        Returns:
            List of dictionaries containing name and type for each rule.
        """
        rules = []

        for name, rule in self.grammar_json.get("rules", {}).items():
            if not name.startswith("_"):  # Skip internal rules
                rules.append(
                    {
                        "name": name,
                        "type": rule.get("type", "UNKNOWN"),
                    }
                )

        return sorted(rules, key=lambda x: x["name"])

    def to_json(self, output_path: Optional[Path] = None, indent: int = 2) -> str:
        """Exports complete grammar to JSON.

        Args:
            output_path: Optional output path. If provided, writes file.
            indent: JSON indentation (default: 2).

        Returns:
            Formatted JSON string.
        """
        data = self.extract_all()
        json_str = json.dumps(data, indent=indent, ensure_ascii=False)

        if output_path:
            output_path = Path(output_path)
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_text(json_str)

        return json_str

    def generate_pygments_lexer(self, output_path: Optional[Path] = None) -> str:
        """Generates an up-to-date Pygments lexer from the grammar.

        Args:
            output_path: Optional output path. If provided, writes file.

        Returns:
            Python code for the generated lexer.
        """
        keywords = self.extract_keywords()
        operators = self.extract_operators()

        # Generate by assembling lines to avoid escaping issues
        lines = [
            "# FILE: catnip/tools/pygments.py",
            '"""Pygments lexer for the Catnip programming language.',
            "",
            "Auto-generated from Tree-sitter grammar - DO NOT EDIT MANUALLY.",
            "Run `python -m catnip.tools.extract_grammar --update-lexer` to regenerate.",
            '"""',
            "",
            "from pygments.lexer import RegexLexer, bygroups, words",
            "from pygments.token import (",
            "    Comment,",
            "    Keyword,",
            "    Name,",
            "    Number,",
            "    Operator,",
            "    Punctuation,",
            "    String,",
            "    Text,",
            "    Whitespace,",
            ")",
            "",
            "__all__ = ['CatnipLexer']",
            "",
            "",
            "class CatnipLexer(RegexLexer):",
            '    """Lexer for the Catnip programming language.',
            "",
            "    Catnip is a Python-inspired functional language with support for:",
            "    - Lambda expressions: (params) => { body }",
            "    - Broadcasting operations: .[op]",
            "    - Match expressions: match expr { cases }",
            "    - F-strings and built-in collection types",
            '    """',
            "",
            "    name = 'Catnip'",
            "    aliases = ['catnip']",
            "    filenames = ['*.cat', '*.catnip']",
            "    mimetypes = ['text/x-catnip']",
            "",
            "    tokens = {",
            "        'root': [",
            "            # Comments",
            "            (r'#.*?$', Comment.Single),",
            "            # Whitespace",
            "            (r'\\s+', Whitespace),",
            "            # Keywords",
            "            (",
            "                words(",
            "                    (",
        ]

        # Add keywords
        all_keywords = keywords["control_flow"] + keywords["pragmas"]
        for i, kw in enumerate(all_keywords):
            comma = "," if i < len(all_keywords) - 1 else ""
            lines.append(f"                        {repr(kw)}{comma}")

        lines.extend(
            [
                "                    ),",
                "                    suffix=r'\\b',",
                "                ),",
                "                Keyword,",
                "            ),",
                "            # Constants",
            ]
        )

        # Constants tuple
        constants_str = "(" + ", ".join(repr(c) for c in keywords["constants"]) + ")"
        lines.append(f"            (words({constants_str}, suffix=r'\\b'), Keyword.Constant),")

        # Types
        types_str = "(" + ", ".join(repr(t) for t in keywords["types"]) + ")"
        lines.append("            # Built-in types")
        lines.append(f"            (words({types_str}, suffix=r'\\b'), Keyword.Type),")

        # Strings - all unified under String
        lines.extend(
            [
                "            # F-strings",
                "            (",
                '                r\'[fF]("""(?:[^"\\\\]|\\\\.)*?"""\'',
                "                r\"|'''(?:[^'\\\\]|\\\\.)*?'''\"",
                '                r\'|"(?:[^"\\\\]|\\\\.)*"\'',
                "                r\"|'(?:[^'\\\\]|\\\\.)*')\",",
                "                String,",
                "            ),",
                "            # Regular strings",
                '            (r\'"""(?:[^"\\\\]|\\\\.)*?"""\', String),',
                "            (r\"'''(?:[^'\\\\]|\\\\.)*?'''\", String),",
                '            (r\'"(?:[^"\\\\]|\\\\.)*"\', String),',
                "            (r\"'(?:[^'\\\\]|\\\\.)*'\", String),",
                "            # Numbers (binary, octal, hex, decimal, float)",
                "            (r'0[bB][01]+', Number.Bin),",
                "            (r'0[oO][0-7]+', Number.Oct),",
                "            (r'0[xX][0-9a-fA-F]+', Number.Hex),",
                "            (r'\\d+\\.\\d+([eE][+-]?\\d+)?', Number.Float),",
                "            (r'\\d+[eE][+-]?\\d+', Number.Float),",
                "            (r'\\d+', Number.Integer),",
                "            # Lambda arrow",
                "            (r'=>', Operator),",
                "            # Broadcast operations",
                "            (r'\\.\\[', Punctuation, 'broadcast'),",
                "            # Operators (sorted by length for correct matching)",
            ]
        )

        # Operators (exclude => and .[)
        ops_filtered = [op for op in operators["all"] if op not in ["=>", ".["]]
        if ops_filtered:
            ops_pattern = "|".join(re.escape(op) for op in ops_filtered)
            lines.append(f"            (r'({ops_pattern})', Operator),")

        # Identifiers
        all_kw_pattern = "|".join(keywords["all"])
        lines.extend(
            [
                "            # Punctuation",
                "            (r'[{}()\\[\\],:;.]', Punctuation),",
                "            # Identifiers (excluding keywords)",
                "            (",
                f"                r'(?!(?:{all_kw_pattern})\\b)'",
                "                r'[a-zA-Z_]\\w*',",
                "                Name,",
                "            ),",
                "        ],",
                "        # Broadcast context: .[expression]",
                "        # Broadcasts can contain any Catnip expression",
                "        'broadcast': [",
                "            (r'\\s+', Whitespace),",
                "            # Close bracket exits broadcast mode (check first)",
                "            (r'\\]', Punctuation, '#pop'),",
                "            # Comments",
                "            (r'#.*?$', Comment.Single),",
                "            # Keywords (including broadcast-specific 'if')",
                "            (",
                "                words(",
                "                    (",
            ]
        )

        # All keywords in broadcast
        for i, kw in enumerate(all_keywords):
            comma = "," if i < len(all_keywords) - 1 else ""
            lines.append(f"                        {repr(kw)}{comma}")

        lines.extend(
            [
                "                    ),",
                "                    suffix=r'\\b',",
                "                ),",
                "                Keyword,",
                "            ),",
                "            # Constants",
            ]
        )

        # Constants in broadcast
        lines.append(f"            (words({constants_str}, suffix=r'\\b'), Keyword.Constant),")
        # Types in broadcast
        lines.append(f"            (words({types_str}, suffix=r'\\b'), Keyword.Type),")

        # Strings in broadcast
        lines.extend(
            [
                "            # Strings (F-strings and regular)",
                "            (",
                '                r\'[fF]("""(?:[^"\\\\]|\\\\.)*?"""\'',
                "                r\"|'''(?:[^'\\\\]|\\\\.)*?'''\"",
                '                r\'|"(?:[^"\\\\]|\\\\.)*"\'',
                "                r\"|'(?:[^'\\\\]|\\\\.)*')\",",
                "                String,",
                "            ),",
                '            (r\'"""(?:[^"\\\\]|\\\\.)*?"""\', String),',
                "            (r\"'''(?:[^'\\\\]|\\\\.)*?'''\", String),",
                '            (r\'"(?:[^"\\\\]|\\\\.)*"\', String),',
                "            (r\"'(?:[^'\\\\]|\\\\.)*'\", String),",
                "            # Numbers",
                "            (r'0[bB][01]+', Number.Bin),",
                "            (r'0[oO][0-7]+', Number.Oct),",
                "            (r'0[xX][0-9a-fA-F]+', Number.Hex),",
                "            (r'\\d+\\.\\d+([eE][+-]?\\d+)?', Number.Float),",
                "            (r'\\d+[eE][+-]?\\d+', Number.Float),",
                "            (r'\\d+', Number.Integer),",
                "            # Lambda arrow",
                "            (r'=>', Operator),",
                "            # Operators",
            ]
        )

        # All operators in broadcast (except .[)
        broadcast_ops_filtered = [op for op in operators["all"] if op not in [".["]]
        if broadcast_ops_filtered:
            broadcast_pattern = "|".join(re.escape(op) for op in broadcast_ops_filtered)
            lines.append(f"            (r'({broadcast_pattern})', Operator),")

        lines.extend(
            [
                "            # Punctuation (including nested brackets and braces)",
                "            (r'[{}()\\[\\],:;.]', Punctuation),",
                "            # Identifiers",
                "            (",
                f"                r'(?!(?:{all_kw_pattern})\\b)'",
                "                r'[a-zA-Z_]\\w*',",
                "                Name,",
                "            ),",
                "        ],",
                "    }",
            ]
        )

        lexer_code = "\n".join(lines) + "\n"

        if output_path:
            output_path = Path(output_path)
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_text(lexer_code)
            subprocess.run(["black", "-q", str(output_path)], check=False)

        return lexer_code

    def generate_textmate_grammar(self, output_path: Optional[Path] = None) -> str:
        """Generates a TextMate grammar for VS Code syntax highlighting.

        Args:
            output_path: Optional output path. If provided, writes file.

        Returns:
            JSON string of the TextMate grammar.
        """
        keywords = self.extract_keywords()
        operators = self.extract_operators()

        # Build keyword pattern
        control_kw = "|".join(re.escape(k) for k in keywords["control_flow"])
        constants = "|".join(re.escape(c) for c in keywords["constants"])
        types = "|".join(re.escape(t) for t in keywords["types"])
        builtins = "abs|len|range|print|sum|map|filter|zip"

        # Build operator pattern (sorted by length, longest first)
        ops_sorted = sorted(operators["all"], key=len, reverse=True)
        ops_pattern = "|".join(re.escape(op) for op in ops_sorted if op not in [".[", "=>"])

        grammar = {
            "name": "Catnip",
            "scopeName": "source.catnip",
            "fileTypes": ["cat", "catnip"],
            "patterns": [
                {"name": "keyword.control.pragma.catnip", "match": r"\bpragma\s*\("},
                {"name": "comment.line.catnip", "match": r"#.*$"},
                {"name": "keyword.control.catnip", "match": rf"\b({control_kw})\b"},
                {"name": "constant.language.catnip", "match": rf"\b({constants})\b"},
                {"name": "support.function.builtin.catnip", "match": rf"\b({builtins})\b"},
                {"name": "keyword.type.catnip", "match": rf"\b({types})\b"},
                {
                    "name": "string.quoted.double.catnip",
                    "begin": '"',
                    "end": '"',
                    "patterns": [{"name": "constant.character.escape.catnip", "match": r"\\."}],
                },
                {
                    "name": "string.quoted.single.catnip",
                    "begin": "'",
                    "end": "'",
                    "patterns": [{"name": "constant.character.escape.catnip", "match": r"\\."}],
                },
                {"name": "constant.numeric.hex.catnip", "match": r"\b0[xX][0-9a-fA-F_]+\b"},
                {"name": "constant.numeric.binary.catnip", "match": r"\b0[bB][01_]+\b"},
                {"name": "constant.numeric.octal.catnip", "match": r"\b0[oO][0-7_]+\b"},
                {"name": "constant.numeric.float.catnip", "match": r"\b[0-9][0-9_]*\.[0-9_]*([eE][+-]?[0-9_]+)?\b"},
                {"name": "constant.numeric.integer.catnip", "match": r"\b[0-9][0-9_]*([eE][+-]?[0-9_]+)?\b"},
                {
                    "name": "meta.function.lambda.catnip",
                    "match": r"\(([^)]*)\)\s*=>",
                    "captures": {
                        "1": {
                            "patterns": [{"name": "variable.parameter.catnip", "match": r"\b[a-zA-Z_][a-zA-Z0-9_]*\b"}]
                        }
                    },
                },
                {"name": "keyword.operator.broadcast.catnip", "match": r"\.\["},
                {"name": "keyword.operator.arrow.catnip", "match": r"=>"},
                {"name": "keyword.operator.catnip", "match": rf"({ops_pattern})"},
            ],
        }

        json_str = json.dumps(grammar, indent=2, ensure_ascii=False)

        if output_path:
            output_path = Path(output_path)
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_text(json_str)

        return json_str


def extract_grammar(
    grammar_path: Optional[Path] = None,
    output_json: Optional[Path] = None,
    output_lexer: Optional[Path] = None,
) -> GrammarExtractor:
    """Convenience function to extract grammar.

    Args:
        grammar_path: Path to Tree-sitter grammar.json (optional).
        output_json: JSON output path (optional).
        output_lexer: Pygments lexer output path (optional).

    Returns:
        GrammarExtractor instance with extracted data.
    """
    extractor = GrammarExtractor(grammar_path)

    if output_json:
        extractor.to_json(output_json)

    if output_lexer:
        extractor.generate_pygments_lexer(output_lexer)

    return extractor


@click.command()
@click.option(
    "--json",
    "json_path",
    type=click.Path(path_type=Path),
    help="Output path for JSON export",
)
@click.option(
    "--update-lexer",
    is_flag=True,
    help="Update Pygments lexer in catnip/tools/pygments.py",
)
@click.option(
    "--lexer",
    "lexer_path",
    type=click.Path(path_type=Path),
    help="Custom output path for Pygments lexer",
)
@click.option(
    "--print",
    "print_category",
    type=click.Choice(["keywords", "operators", "terminals", "rules", "all"]),
    help="Display a category of elements",
)
@click.option(
    "--textmate",
    "textmate_path",
    type=click.Path(path_type=Path),
    help="Output path for TextMate grammar (VS Code)",
)
@click.option(
    "--update-vscode",
    is_flag=True,
    help="Update VS Code grammar in dev/vscode/syntaxes/catnip.tmLanguage.json",
)
def main(json_path, update_lexer, lexer_path, print_category, textmate_path, update_vscode):
    """Extract elements from Catnip grammar."""
    extractor = GrammarExtractor()

    # Console display
    if print_category:
        data = extractor.extract_all()
        if print_category == "all":
            click.echo(json.dumps(data, indent=2, ensure_ascii=False))
        else:
            click.echo(json.dumps(data[print_category], indent=2, ensure_ascii=False))

    # JSON export
    if json_path:
        extractor.to_json(json_path)
        click.echo(f"JSON exported to {json_path}")

    # Lexer generation
    if update_lexer:
        lexer_output = Path(__file__).parent / "pygments.py"
        extractor.generate_pygments_lexer(lexer_output)
        click.echo(f"Pygments lexer updated: {lexer_output}")
    elif lexer_path:
        extractor.generate_pygments_lexer(lexer_path)
        click.echo(f"Pygments lexer generated: {lexer_path}")

    # TextMate grammar generation (VS Code)
    if update_vscode:
        vscode_output = Path(__file__).parent.parent.parent / "dev" / "vscode" / "syntaxes" / "catnip.tmLanguage.json"
        extractor.generate_textmate_grammar(vscode_output)
        click.echo(f"VS Code grammar updated: {vscode_output}")
    elif textmate_path:
        extractor.generate_textmate_grammar(textmate_path)
        click.echo(f"TextMate grammar generated: {textmate_path}")

    # Default display if no options
    if not any([json_path, update_lexer, lexer_path, print_category, textmate_path, update_vscode]):
        click.echo("Catnip Grammar Extraction")
        click.echo("=" * 80)
        click.echo()

        keywords = extractor.extract_keywords()
        click.echo("KEYWORDS:")
        click.echo(f"  Control flow: {', '.join(keywords['control_flow'])}")
        click.echo(f"  Constants: {', '.join(keywords['constants'])}")
        click.echo(f"  Types: {', '.join(keywords['types'])}")
        click.echo()

        operators = extractor.extract_operators()
        click.echo("OPERATORS:")
        click.echo(f"  Arithmetic: {', '.join(operators['arithmetic'])}")
        click.echo(f"  Comparison: {', '.join(operators['comparison'])}")
        click.echo(f"  Bitwise: {', '.join(operators['bitwise'])}")
        click.echo(f"  Logical: {', '.join(operators['logical'])}")
        click.echo(f"  Special: {', '.join(operators['special'])}")
        click.echo()

        click.echo(f"Total terminals: {len(extractor.extract_terminals())}")
        click.echo(f"Total rules: {len(extractor.extract_rules())}")
        click.echo()
        click.echo("Use --help to see export options")


if __name__ == "__main__":
    main()
