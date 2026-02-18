# FILE: catnip/colors.py
"""
ANSI color utilities for terminal output with true color support.

Theme values are generated from visual.toml via gen_theme.py.
"""

import sys

from ._theme import (
    ACCENT_BUILTIN,
    ACCENT_CONSTANT,
    ACCENT_DARK_BUILTIN,
    ACCENT_DARK_CONSTANT,
    ACCENT_DARK_KEYWORD,
    ACCENT_DARK_NUMBER,
    ACCENT_DARK_STRING,
    ACCENT_KEYWORD,
    ACCENT_LIGHT_BUILTIN,
    ACCENT_LIGHT_CONSTANT,
    ACCENT_LIGHT_KEYWORD,
    ACCENT_LIGHT_NUMBER,
    ACCENT_LIGHT_STRING,
    ACCENT_NUMBER,
    ACCENT_STRING,
    BASE_COMMENT,
    BASE_FOREGROUND,
    BASE_OPERATOR,
    DARK_COMMENT,
    DARK_FOREGROUND,
    DARK_OPERATOR,
    DETECTED_THEME,
    LIGHT_COMMENT,
    LIGHT_FOREGROUND,
    LIGHT_OPERATOR,
    STAGE_EXECUTE,
    STAGE_PARSE,
    STAGE_RESULT,
    STAGE_SEMANTIC,
    UI_DIM,
    UI_ERROR,
    UI_INFO,
    UI_PROMPT,
    UI_SUCCESS,
)


class Colors:
    """ANSI color codes for terminal support."""

    # Basic styles
    RESET = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[2m"
    ITALIC = "\033[3m"
    UNDERLINE = "\033[4m"

    # Standard foreground colors (kept for direct use)
    BLACK = "\033[30m"
    RED = "\033[31m"
    GREEN = "\033[32m"
    YELLOW = "\033[33m"
    BLUE = "\033[34m"
    MAGENTA = "\033[35m"
    CYAN = "\033[36m"
    WHITE = "\033[37m"

    # Bright foreground colors
    BRIGHT_BLACK = "\033[90m"
    BRIGHT_RED = "\033[91m"
    BRIGHT_GREEN = "\033[92m"
    BRIGHT_YELLOW = "\033[93m"
    BRIGHT_BLUE = "\033[94m"
    BRIGHT_MAGENTA = "\033[95m"
    BRIGHT_CYAN = "\033[96m"
    BRIGHT_WHITE = "\033[97m"

    # Background colors
    BG_BLACK = "\033[40m"
    BG_RED = "\033[41m"
    BG_GREEN = "\033[42m"
    BG_YELLOW = "\033[43m"
    BG_BLUE = "\033[44m"
    BG_MAGENTA = "\033[45m"
    BG_CYAN = "\033[46m"
    BG_WHITE = "\033[47m"

    @staticmethod
    def fg_256(color_code):
        """Set foreground color using 256-color palette (0-255)."""
        return f"\033[38;5;{color_code}m"

    @staticmethod
    def bg_256(color_code):
        """Set background color using 256-color palette (0-255)."""
        return f"\033[48;5;{color_code}m"

    @staticmethod
    def rgb(r, g, b):
        """Set foreground color using true RGB (0-255 each)."""
        return f"\033[38;2;{r};{g};{b}m"

    @staticmethod
    def bg_rgb(r, g, b):
        """Set background color using true RGB (0-255 each)."""
        return f"\033[48;2;{r};{g};{b}m"


class Theme:
    """Color theme for Catnip syntax highlighting and output.

    Values sourced from visual.toml via _theme.py (true color).
    Base colors adapt to detected terminal background (dark/light).
    """

    # Disable colors if not in a TTY
    enabled = sys.stdout.isatty()

    # Current theme name
    current_theme = DETECTED_THEME

    # Syntax highlighting (from visual.toml accent)
    KEYWORD = ACCENT_KEYWORD
    NUMBER = ACCENT_NUMBER
    STRING = ACCENT_STRING
    OPERATOR = BASE_OPERATOR
    IDENTIFIER = BASE_FOREGROUND
    COMMENT = BASE_COMMENT
    BUILTIN = ACCENT_BUILTIN
    CONSTANT = ACCENT_CONSTANT

    # REPL UI (from visual.toml ui)
    PROMPT = UI_PROMPT
    PROMPT_CONTINUE = UI_DIM
    ERROR = UI_ERROR
    WARNING = UI_ERROR  # reuses error hue
    SUCCESS = UI_SUCCESS
    INFO = UI_INFO

    # Output formatting (reuse accent colors for value types)
    TYPE_STR = ACCENT_STRING
    TYPE_NUM = ACCENT_NUMBER
    TYPE_BOOL = ACCENT_CONSTANT
    TYPE_NONE = UI_DIM
    TYPE_LIST = ACCENT_BUILTIN
    TYPE_DICT = ACCENT_BUILTIN

    # Pipeline stages (from visual.toml stages)
    STAGE_PARSE = STAGE_PARSE
    STAGE_SEMANTIC = STAGE_SEMANTIC
    STAGE_EXECUTE = STAGE_EXECUTE
    STAGE_RESULT = STAGE_RESULT

    # Box drawing
    BOX_LIGHT = UI_DIM
    BOX_HEAVY = BASE_FOREGROUND

    @classmethod
    def disable(cls):
        """Disable all colors (for non-TTY or when --no-color is used)."""
        cls.enabled = False

    @classmethod
    def enable(cls):
        """Enable colors."""
        cls.enabled = True

    @classmethod
    def set_theme(cls, name):
        """Switch base and accent colors to dark or light palette."""
        if name == "dark":
            cls.OPERATOR = DARK_OPERATOR
            cls.IDENTIFIER = DARK_FOREGROUND
            cls.COMMENT = DARK_COMMENT
            cls.BOX_HEAVY = DARK_FOREGROUND
            cls.KEYWORD = ACCENT_DARK_KEYWORD
            cls.NUMBER = ACCENT_DARK_NUMBER
            cls.STRING = ACCENT_DARK_STRING
            cls.BUILTIN = ACCENT_DARK_BUILTIN
            cls.CONSTANT = ACCENT_DARK_CONSTANT
        elif name == "light":
            cls.OPERATOR = LIGHT_OPERATOR
            cls.IDENTIFIER = LIGHT_FOREGROUND
            cls.COMMENT = LIGHT_COMMENT
            cls.BOX_HEAVY = LIGHT_FOREGROUND
            cls.KEYWORD = ACCENT_LIGHT_KEYWORD
            cls.NUMBER = ACCENT_LIGHT_NUMBER
            cls.STRING = ACCENT_LIGHT_STRING
            cls.BUILTIN = ACCENT_LIGHT_BUILTIN
            cls.CONSTANT = ACCENT_LIGHT_CONSTANT
        cls.current_theme = name
        # Update output formatting aliases
        cls.TYPE_STR = cls.STRING
        cls.TYPE_NUM = cls.NUMBER
        cls.TYPE_BOOL = cls.CONSTANT
        cls.TYPE_LIST = cls.BUILTIN
        cls.TYPE_DICT = cls.BUILTIN


def colorize(text, color):
    """Apply color to text if colors are enabled."""
    if Theme.enabled:
        return f"{color}{text}{Colors.RESET}"
    return text


def format_value(value):
    """Format a Python value with appropriate colors."""
    if not Theme.enabled:
        return repr(value)

    if value is None:
        return colorize('None', Theme.TYPE_NONE)
    elif isinstance(value, bool):
        return colorize(str(value), Theme.TYPE_BOOL)
    elif isinstance(value, (int, float)):
        return colorize(repr(value), Theme.TYPE_NUM)
    elif isinstance(value, str):
        return colorize(repr(value), Theme.TYPE_STR)
    elif isinstance(value, list):
        items = ", ".join(format_value(v) for v in value)
        return f"{colorize('[', Theme.TYPE_LIST)}{items}{colorize(']', Theme.TYPE_LIST)}"
    elif isinstance(value, dict):
        items = ", ".join(f"{format_value(k)}: {format_value(v)}" for k, v in value.items())
        return f"{colorize('{', Theme.TYPE_DICT)}{items}{colorize('}', Theme.TYPE_DICT)}"
    else:
        return repr(value)


def print_stage(stage_name, content, stage_color):
    """Print a pipeline stage with formatting."""
    if not Theme.enabled:
        print(f"[{stage_name}]")
        print(content)
        return

    # Box drawing characters
    top = f"{colorize('╭─', Theme.BOX_LIGHT)} {colorize(stage_name, stage_color)} {colorize('─' * (60 - len(stage_name)), Theme.BOX_LIGHT)}"

    print(top)
    for line in str(content).split("\n"):
        print(f"{colorize('│', Theme.BOX_LIGHT)} {line}")


def print_error(message):
    """Print an error message with formatting."""
    if Theme.enabled:
        print(f"{Theme.ERROR}✗ Error:{Colors.RESET} {message}")
    else:
        print(f"Error: {message}")


def print_exception(exception):
    """
    Print an exception with appropriate formatting.

    For CatnipError instances, displays type and formatted message with context.
    For other exceptions, displays standard error message.
    """
    from .exc import CatnipError

    if isinstance(exception, CatnipError):
        error_type = type(exception).__name__

        # Print traceback if available
        if exception.traceback:
            if Theme.enabled:
                print(f"{Colors.DIM}Traceback (most recent call last):{Colors.RESET}")
            else:
                print("Traceback (most recent call last):")
            for frame in exception.traceback.frames:
                line_info = f", line {frame.line}" if hasattr(frame, 'line') and frame.line else ""
                print(f'  File "{frame.filename}"{line_info}, in {frame.name}')

        # Print error type and message
        if Theme.enabled:
            print(f"{Theme.ERROR}{error_type}:{Colors.RESET} {exception.message}")
        else:
            print(f"{error_type}: {exception.message}")

        # Print code snippet if available
        if exception.context:
            if Theme.enabled:
                print(f"{Colors.DIM}{exception.context}{Colors.RESET}")
            else:
                print(exception.context)
    else:
        # Standard Python exception
        print_error(str(exception))


def print_warning(message):
    """Print a warning message with formatting."""
    if Theme.enabled:
        print(f"{Theme.WARNING}⚠ Warning:{Colors.RESET} {message}")
    else:
        print(f"Warning: {message}")


def print_success(message):
    """Print a success message with formatting."""
    if Theme.enabled:
        print(f"{Theme.SUCCESS}✓ {message}{Colors.RESET}")
    else:
        print(message)


def print_info(message):
    """Print an info message with formatting."""
    if Theme.enabled:
        print(f"{Theme.INFO}ℹ {message}{Colors.RESET}")
    else:
        print(message)
