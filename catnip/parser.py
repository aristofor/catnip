# FILE: catnip/parser.py
"""Catnip parser module - Tree-sitter based parser with Rust core."""

from catnip._rs import TreeSitterParser


class Parser:
    """Parser with same interface for use with Catnip class."""

    def __init__(self, transformer=True, **kwargs):
        self._parser = TreeSitterParser()
        self._transform = transformer is not None and transformer is not False

    def parse(self, text, **kwargs):
        """Parse text and return IR as list or parse tree."""
        if self._transform:
            # Return IR (level 1 = IR only)
            ir = self._parser.parse(text, level=1)
            return ir if ir is not None else []
        else:
            # Raw parse tree mode (level 0)
            return self._parser.parse(text, level=0)


__all__ = ('Parser',)
