# FILE: catnip/suggest.py
"""
Name suggestions for undefined variables.

Uses difflib to find similar names for "did you mean?" hints.
"""

from difflib import get_close_matches

__all__ = ('suggest_name', 'format_suggestions')


def suggest_name(name: str, available: set[str] | list[str], max_suggestions: int = 3) -> list[str]:
    """
    Find similar names for suggestions.

    Filters out private names (starting with _) from suggestions.

    Args:
        name: The undefined name
        available: Set of available names in scope
        max_suggestions: Maximum number of suggestions

    Returns:
        List of similar names, ordered by similarity
    """
    # Filter private names
    candidates = [n for n in available if not n.startswith('_')]
    return get_close_matches(name, candidates, n=max_suggestions, cutoff=0.6)


def format_suggestions(suggestions: list[str]) -> str:
    """
    Format suggestions for display.

    Returns empty string if no suggestions.

    Examples:
        [] -> ""
        ["factorial"] -> "Did you mean 'factorial'?"
        ["foo", "bar"] -> "Did you mean one of: 'foo', 'bar'?"
    """
    if not suggestions:
        return ''
    if len(suggestions) == 1:
        return f"Did you mean '{suggestions[0]}'?"
    quoted = ', '.join(f"'{s}'" for s in suggestions)
    return f"Did you mean one of: {quoted}?"
