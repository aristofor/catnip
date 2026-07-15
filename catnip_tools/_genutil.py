# FILE: catnip_tools/_genutil.py
"""Shared helpers for the catnip_tools code generators."""

import re
from pathlib import Path


def replace_between_markers(path: Path, start_marker: str, end_marker: str, new_lines: list[str]) -> bool:
    """Replace content between markers in file. Returns True if changed."""
    content = path.read_text()

    pattern = re.compile(
        rf'^{re.escape(start_marker)}$.*?^{re.escape(end_marker)}$',
        re.MULTILINE | re.DOTALL,
    )
    match = pattern.search(content)
    if not match:
        raise ValueError(f"Markers {start_marker} not found in {path}")

    new_block = "\n".join([start_marker] + new_lines + [end_marker])
    if match.group(0) == new_block:
        return False

    new_content = content[:match.start()] + new_block + content[match.end():]
    path.write_text(new_content)
    return True
