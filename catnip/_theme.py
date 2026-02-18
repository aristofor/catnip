# FILE: catnip/_theme.py
# GENERATED FROM catnip_rs/visual.toml
# Do not edit manually. Run: make gen-theme
"""Theme constants generated from visual.toml (OKLCH -> true color ANSI)."""

import os

# Prompts
PROMPT_MAIN = '▸ '
PROMPT_CONTINUATION = '▹ '

# Box drawing
BOX_CORNER_TL = '╭'
BOX_CORNER_TR = '╮'
BOX_CORNER_BL = '╰'
BOX_CORNER_BR = '╯'
BOX_VERTICAL = '│'
BOX_HORIZONTAL = '─'

# UI colors (true color ANSI)
UI_PROMPT = "\033[38;2;0;153;136m"
UI_ERROR = "\033[38;2;240;81;67m"
UI_INFO = "\033[38;2;62;135;244m"
UI_SUCCESS = "\033[38;2;30;162;17m"
UI_DIM = "\033[38;2;160;160;160m"

# Accent colors - dark theme
ACCENT_DARK_KEYWORD = "\033[38;2;88;125;204m"
ACCENT_DARK_KEYWORD_BOLD = True
ACCENT_DARK_CONSTANT = "\033[38;2;48;192;248m"
ACCENT_DARK_CONSTANT_BOLD = True
ACCENT_DARK_TYPE = "\033[38;2;97;195;170m"
ACCENT_DARK_NUMBER = "\033[38;2;180;205;167m"
ACCENT_DARK_STRING = "\033[38;2;243;121;58m"
ACCENT_DARK_BUILTIN = "\033[38;2;223;214;161m"

# Accent colors - light theme
ACCENT_LIGHT_KEYWORD = "\033[38;2;80;116;194m"
ACCENT_LIGHT_KEYWORD_BOLD = True
ACCENT_LIGHT_CONSTANT = "\033[38;2;0;110;179m"
ACCENT_LIGHT_CONSTANT_BOLD = True
ACCENT_LIGHT_TYPE = "\033[38;2;0;119;133m"
ACCENT_LIGHT_NUMBER = "\033[38;2;0;125;80m"
ACCENT_LIGHT_STRING = "\033[38;2;226;106;40m"
ACCENT_LIGHT_BUILTIN = "\033[38;2;135;88;20m"

# Base colors (dark theme)
DARK_BACKGROUND = "\033[38;2;15;23;31m"
DARK_FOREGROUND = "\033[38;2;207;213;219m"
DARK_COMMENT = "\033[38;2;86;137;81m"
DARK_COMMENT_ITALIC = True
DARK_OPERATOR = "\033[38;2;207;213;219m"
DARK_PUNCTUATION = "\033[38;2;209;214;220m"

# Base colors (light theme)
LIGHT_BACKGROUND = "\033[38;2;252;252;252m"
LIGHT_FOREGROUND = "\033[38;2;30;34;38m"
LIGHT_COMMENT = "\033[38;2;34;112;28m"
LIGHT_COMMENT_ITALIC = True
LIGHT_OPERATOR = "\033[38;2;30;34;38m"
LIGHT_PUNCTUATION = "\033[38;2;68;72;77m"

# Pipeline stages
STAGE_PARSE = "\033[38;2;18;146;192m"
STAGE_SEMANTIC = "\033[38;2;209;114;54m"
STAGE_EXECUTE = "\033[38;2;92;155;86m"
STAGE_RESULT = "\033[38;2;187;112;181m"


# Theme detection
def _detect_terminal_bg() -> str:
    """Detect terminal background: COLORFGBG -> dark/light."""
    colorfgbg = os.environ.get("COLORFGBG", "")
    if colorfgbg:
        parts = colorfgbg.split(";")
        try:
            bg = int(parts[-1])
            return "light" if bg > 8 else "dark"
        except (ValueError, IndexError):
            pass
    return "dark"


_theme = os.environ.get("CATNIP_THEME", "").lower()
if _theme not in ("dark", "light"):
    _theme = _detect_terminal_bg()

DETECTED_THEME = _theme

# Dynamic base colors (resolved at import time from detected theme)
BASE_FOREGROUND = DARK_FOREGROUND if _theme == "dark" else LIGHT_FOREGROUND
BASE_COMMENT = DARK_COMMENT if _theme == "dark" else LIGHT_COMMENT
BASE_OPERATOR = DARK_OPERATOR if _theme == "dark" else LIGHT_OPERATOR
BASE_PUNCTUATION = DARK_PUNCTUATION if _theme == "dark" else LIGHT_PUNCTUATION

# Dynamic accent colors (resolved at import time from detected theme)
ACCENT_KEYWORD = ACCENT_DARK_KEYWORD if _theme == "dark" else ACCENT_LIGHT_KEYWORD
ACCENT_CONSTANT = ACCENT_DARK_CONSTANT if _theme == "dark" else ACCENT_LIGHT_CONSTANT
ACCENT_TYPE = ACCENT_DARK_TYPE if _theme == "dark" else ACCENT_LIGHT_TYPE
ACCENT_NUMBER = ACCENT_DARK_NUMBER if _theme == "dark" else ACCENT_LIGHT_NUMBER
ACCENT_STRING = ACCENT_DARK_STRING if _theme == "dark" else ACCENT_LIGHT_STRING
ACCENT_BUILTIN = ACCENT_DARK_BUILTIN if _theme == "dark" else ACCENT_LIGHT_BUILTIN
