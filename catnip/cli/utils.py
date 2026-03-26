# FILE: catnip/cli/utils.py
"""Shared CLI utilities."""

BUILTIN_COMMANDS_PREFIX = 'catnip.cli.commands.'


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
