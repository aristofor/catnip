# FILE: catnip/cli/utils.py
"""Shared CLI utilities."""

from glob import glob
from pathlib import Path

_GLOB_CHARS = frozenset('*?[')

BUILTIN_COMMANDS_PREFIX = 'catnip.cli.commands.'


def expand_file_args(patterns, extension='.cat'):
    """Expand file arguments: directories, globs, and plain paths.

    Returns a list of Path objects. Directories are walked recursively
    for files matching *extension. Glob patterns are expanded and
    filtered by extension (directories in glob results are skipped,
    preventing quoted globs like ``"src/*"`` from recursing into
    matched subdirectories). Plain paths are kept as-is (validation
    is the caller's responsibility).
    """
    result = []
    for pattern in patterns:
        p = Path(pattern)
        if p.is_dir():
            result.extend(sorted(p.rglob(f'*{extension}')))
        elif any(c in pattern for c in _GLOB_CHARS):
            matches = glob(pattern, recursive=True)
            if not matches:
                result.append(p)
            else:
                for m in matches:
                    mp = Path(m)
                    if not mp.is_dir() and mp.suffix == extension:
                        result.append(mp)
        else:
            result.append(p)
    return list(dict.fromkeys(result))


def format_table(rows, headers):
    """Format rows as an aligned text table."""
    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))

    last = len(headers) - 1
    header_line = "  ".join(headers[i].ljust(widths[i]) if i < last else headers[i] for i in range(len(headers)))
    sep_line = "  ".join("-" * widths[i] for i in range(len(headers)))
    body_lines = [
        "  ".join(row[i].ljust(widths[i]) if i < last else row[i] for i in range(len(headers))) for row in rows
    ]
    return "\n".join([header_line, sep_line, *body_lines])
