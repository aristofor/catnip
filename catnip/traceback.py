# FILE: catnip/traceback.py
"""
Catnip call stack and traceback.

Provides CatnipFrame and CatnipTraceback for tracking function calls
and formatting error traces.
"""

from dataclasses import dataclass

__all__ = ('CatnipFrame', 'CatnipTraceback')


@dataclass(slots=True)
class CatnipFrame:
    """A frame in the Catnip call stack."""

    name: str  # Function name or '<lambda>'
    filename: str  # Source filename
    start_byte: int  # Start position in source
    end_byte: int  # End position in source

    def format(self, sourcemap=None) -> str:
        """Format frame for display."""
        if sourcemap and self.start_byte >= 0:
            line, col = sourcemap.byte_to_line_col(self.start_byte)
            return f'  File "{self.filename}", line {line}, in {self.name}'
        return f'  File "{self.filename}", in {self.name}'


class CatnipTraceback:
    """
    Catnip call stack trace.

    Tracks function calls for error reporting.
    """

    __slots__ = ('frames',)

    def __init__(self):
        self.frames: list[CatnipFrame] = []

    def push(self, frame: CatnipFrame):
        """Add a frame to the call stack."""
        self.frames.append(frame)

    def pop(self) -> CatnipFrame | None:
        """Remove and return the top frame."""
        if self.frames:
            return self.frames.pop()
        return None

    def copy(self) -> 'CatnipTraceback':
        """Create a snapshot of the current traceback."""
        tb = CatnipTraceback()
        tb.frames = list(self.frames)
        return tb

    def __len__(self) -> int:
        return len(self.frames)

    def __bool__(self) -> bool:
        return bool(self.frames)

    def format(self, sourcemap=None) -> str:
        """
        Format traceback for display.

        Example output:
        Traceback (most recent call last):
          File "script.cat", line 12, in factorial
          File "script.cat", line 20, in main
        """
        if not self.frames:
            return ''

        lines = ['Traceback (most recent call last):']
        for frame in self.frames:
            lines.append(frame.format(sourcemap))

        return '\n'.join(lines)
