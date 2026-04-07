# FILE: catnip/tools/pygments.py
"""Pygments lexer for the Catnip programming language.

Auto-generated from Tree-sitter grammar - DO NOT EDIT MANUALLY.
Run `python -m catnip.tools.extract_grammar --update-lexer` to regenerate.
"""

from pygments.lexer import RegexLexer, words
from pygments.token import (
    Comment,
    Keyword,
    Name,
    Number,
    Operator,
    Punctuation,
    String,
    Whitespace,
)

__all__ = ['CatnipLexer']


class CatnipLexer(RegexLexer):
    """Lexer for the Catnip programming language.

    Catnip is a Python-inspired functional language with support for:
    - Lambda expressions: (params) => { body }
    - Broadcasting operations: .[op]
    - Match expressions: match expr { cases }
    - F-strings and built-in collection types
    """

    name = 'Catnip'
    aliases = ['catnip']
    filenames = ['*.cat', '*.catnip']
    mimetypes = ['text/x-catnip']

    tokens = {
        'root': [
            # Comments
            (r'#.*?$', Comment.Single),
            # Whitespace
            (r'\s+', Whitespace),
            # Keywords
            (
                words(
                    (
                        'and',
                        'break',
                        'continue',
                        'elif',
                        'else',
                        'enum',
                        'except',
                        'extends',
                        'finally',
                        'for',
                        'if',
                        'implements',
                        'in',
                        'is',
                        'match',
                        'not',
                        'op',
                        'or',
                        'raise',
                        'return',
                        'struct',
                        'trait',
                        'try',
                        'while',
                        'with',
                        'pragma',
                    ),
                    suffix=r'\b',
                ),
                Keyword,
            ),
            # Constants
            (words(('False', 'None', 'True'), suffix=r'\b'), Keyword.Constant),
            # Built-in types
            (
                words(
                    (
                        'Decimal',
                        'bool',
                        'bytes',
                        'complex',
                        'dict',
                        'float',
                        'frozenset',
                        'int',
                        'list',
                        'set',
                        'str',
                        'tuple',
                    ),
                    suffix=r'\b',
                ),
                Keyword.Type,
            ),
            # F-strings
            (
                r'[fF]("""(?:[^"\\]|\\.)*?"""' r"|'''(?:[^'\\]|\\.)*?'''" r'|"(?:[^"\\]|\\.)*"' r"|'(?:[^'\\]|\\.)*')",
                String,
            ),
            # Regular strings
            (r'"""(?:[^"\\]|\\.)*?"""', String),
            (r"'''(?:[^'\\]|\\.)*?'''", String),
            (r'"(?:[^"\\]|\\.)*"', String),
            (r"'(?:[^'\\]|\\.)*'", String),
            # Numbers (binary, octal, hex, decimal, float)
            (r'0[bB][01]+', Number.Bin),
            (r'0[oO][0-7]+', Number.Oct),
            (r'0[xX][0-9a-fA-F]+', Number.Hex),
            (r'\d+\.\d+([eE][+-]?\d+)?', Number.Float),
            (r'\d+[eE][+-]?\d+', Number.Float),
            (r'\d+', Number.Integer),
            # Lambda arrow
            (r'=>', Operator),
            # Broadcast operations
            (r'\.\[', Punctuation, 'broadcast'),
            # Operators (sorted by length for correct matching)
            (r'(\~\[\]|\~\~|\~>|\?\?|>>|>=|==|<=|<<|//|\*\*|!=|\~|\||\^|@|>|=|<|/|\-|\+|\*|\&|%|!)', Operator),
            # Builtin functions
            (
                words(
                    (
                        'abs',
                        'all',
                        'any',
                        'ascii',
                        'bin',
                        'callable',
                        'chr',
                        'delattr',
                        'dir',
                        'divmod',
                        'enumerate',
                        'filter',
                        'fold',
                        'format',
                        'freeze',
                        'getattr',
                        'hasattr',
                        'hash',
                        'hex',
                        'id',
                        'input',
                        'isinstance',
                        'issubclass',
                        'iter',
                        'len',
                        'map',
                        'max',
                        'min',
                        'next',
                        'oct',
                        'ord',
                        'pow',
                        'print',
                        'range',
                        'reduce',
                        'repr',
                        'reversed',
                        'round',
                        'setattr',
                        'slice',
                        'sorted',
                        'sum',
                        'thaw',
                        'vars',
                        'zip',
                    ),
                    suffix=r'\b',
                ),
                Name.Builtin,
            ),
            # Punctuation
            (r'[{}()\[\],:;.]', Punctuation),
            # Identifiers (excluding keywords)
            (
                r'(?!(?:False|None|True|abs|and|break|continue|dict|elif|else|enum|except|extends|finally|for|if|implements|in|is|list|match|not|op|or|pragma|raise|return|set|struct|trait|try|tuple|while|with)\b)'
                r'[a-zA-Z_]\w*',
                Name,
            ),
        ],
        # Broadcast context: .[expression]
        # Broadcasts can contain any Catnip expression
        'broadcast': [
            (r'\s+', Whitespace),
            # Close bracket exits broadcast mode (check first)
            (r'\]', Punctuation, '#pop'),
            # Comments
            (r'#.*?$', Comment.Single),
            # Keywords (including broadcast-specific 'if')
            (
                words(
                    (
                        'and',
                        'break',
                        'continue',
                        'elif',
                        'else',
                        'enum',
                        'except',
                        'extends',
                        'finally',
                        'for',
                        'if',
                        'implements',
                        'in',
                        'is',
                        'match',
                        'not',
                        'op',
                        'or',
                        'raise',
                        'return',
                        'struct',
                        'trait',
                        'try',
                        'while',
                        'with',
                        'pragma',
                    ),
                    suffix=r'\b',
                ),
                Keyword,
            ),
            # Constants
            (words(('False', 'None', 'True'), suffix=r'\b'), Keyword.Constant),
            (
                words(
                    (
                        'Decimal',
                        'bool',
                        'bytes',
                        'complex',
                        'dict',
                        'float',
                        'frozenset',
                        'int',
                        'list',
                        'set',
                        'str',
                        'tuple',
                    ),
                    suffix=r'\b',
                ),
                Keyword.Type,
            ),
            # Strings (F-strings and regular)
            (
                r'[fF]("""(?:[^"\\]|\\.)*?"""' r"|'''(?:[^'\\]|\\.)*?'''" r'|"(?:[^"\\]|\\.)*"' r"|'(?:[^'\\]|\\.)*')",
                String,
            ),
            (r'"""(?:[^"\\]|\\.)*?"""', String),
            (r"'''(?:[^'\\]|\\.)*?'''", String),
            (r'"(?:[^"\\]|\\.)*"', String),
            (r"'(?:[^'\\]|\\.)*'", String),
            # Numbers
            (r'0[bB][01]+', Number.Bin),
            (r'0[oO][0-7]+', Number.Oct),
            (r'0[xX][0-9a-fA-F]+', Number.Hex),
            (r'\d+\.\d+([eE][+-]?\d+)?', Number.Float),
            (r'\d+[eE][+-]?\d+', Number.Float),
            (r'\d+', Number.Integer),
            # Lambda arrow
            (r'=>', Operator),
            # Operators
            (r'(\~\[\]|\~\~|\~>|\?\?|>>|>=|==|<=|<<|//|\*\*|!=|\~|\||\^|@|>|=|<|/|\-|\+|\*|\&|%|!)', Operator),
            # Builtin functions
            (
                words(
                    (
                        'abs',
                        'all',
                        'any',
                        'ascii',
                        'bin',
                        'callable',
                        'chr',
                        'delattr',
                        'dir',
                        'divmod',
                        'enumerate',
                        'filter',
                        'fold',
                        'format',
                        'freeze',
                        'getattr',
                        'hasattr',
                        'hash',
                        'hex',
                        'id',
                        'input',
                        'isinstance',
                        'issubclass',
                        'iter',
                        'len',
                        'map',
                        'max',
                        'min',
                        'next',
                        'oct',
                        'ord',
                        'pow',
                        'print',
                        'range',
                        'reduce',
                        'repr',
                        'reversed',
                        'round',
                        'setattr',
                        'slice',
                        'sorted',
                        'sum',
                        'thaw',
                        'vars',
                        'zip',
                    ),
                    suffix=r'\b',
                ),
                Name.Builtin,
            ),
            # Punctuation (including nested brackets and braces)
            (r'[{}()\[\],:;.]', Punctuation),
            # Identifiers
            (
                r'(?!(?:False|None|True|abs|and|break|continue|dict|elif|else|enum|except|extends|finally|for|if|implements|in|is|list|match|not|op|or|pragma|raise|return|set|struct|trait|try|tuple|while|with)\b)'
                r'[a-zA-Z_]\w*',
                Name,
            ),
        ],
    }
