# FILE: catnip/suggest.py
"""
Name suggestions for undefined variables.

Uses difflib to find similar names for "did you mean?" hints.
"""

from difflib import get_close_matches

__all__ = ('suggest_name',)


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
