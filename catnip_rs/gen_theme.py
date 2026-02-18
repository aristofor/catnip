#!/usr/bin/env python3
# FILE: catnip_rs/gen_theme.py
"""Generate theme files from visual.toml (OKLCH source of truth).

Flow: visual.toml -> catnip/_theme.py (true color ANSI)
                  -> web/docs/src/highlight.css (oklch native)

Same pattern as gen_opcodes.py: single source, multiple targets.
"""

import math
import re
import tomllib
from pathlib import Path

# ---------------------------------------------------------------------------
# OKLCH parsing + conversion
# ---------------------------------------------------------------------------

_OKLCH_RE = re.compile(r'oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*\)')


def parse_oklch(s: str) -> tuple[float, float, float]:
    """Parse 'oklch(L C H)' -> (L, C, H)."""
    m = _OKLCH_RE.match(s.strip())
    if not m:
        raise ValueError(f"Invalid oklch format: {s}")
    return float(m.group(1)), float(m.group(2)), float(m.group(3))


def parse_color(value) -> dict:
    """Normalize color entry to {L, C, H, bold, italic} dict.

    Accepts:
      - "oklch(L C H)"                          -> plain color
      - { color = "oklch(L C H)", bold = true }  -> color with attributes
    """
    if isinstance(value, str):
        L, C, H = parse_oklch(value)
        return dict(L=L, C=C, H=H, bold=False, italic=False)
    L, C, H = parse_oklch(value['color'])
    return dict(L=L, C=C, H=H, bold=value.get('bold', False), italic=value.get('italic', False))


def normalize_colors(data: dict) -> None:
    """Resolve all color entries in-place to {L, C, H, bold, italic} dicts."""
    for section in ('ui', 'stages'):
        for key, value in data[section].items():
            data[section][key] = parse_color(value)
    for theme in ('dark', 'light'):
        for key, value in data['accent'][theme].items():
            data['accent'][theme][key] = parse_color(value)
        for key, value in data['base'][theme].items():
            data['base'][theme][key] = parse_color(value)


def oklch_to_srgb(L: float, C: float, H: float) -> tuple[int, int, int]:
    """Convert OKLCH to sRGB (0-255).

    1. OKLCH -> OKLAB
    2. OKLAB -> linear sRGB (matrix)
    3. linear sRGB -> sRGB (gamma)
    """
    # OKLCH -> OKLAB
    h_rad = H * math.pi / 180.0
    a = C * math.cos(h_rad)
    b = C * math.sin(h_rad)

    # OKLAB -> LMS (inverse of Bjorn Ottosson's transform)
    l_ = L + 0.3963377774 * a + 0.2158037573 * b
    m_ = L - 0.1055613458 * a - 0.0638541728 * b
    s_ = L - 0.0894841775 * a - 1.2914855480 * b

    # Cube to undo the cbrt
    lms_l = l_ * l_ * l_
    lms_m = m_ * m_ * m_
    lms_s = s_ * s_ * s_

    # LMS -> linear sRGB
    r_lin = +4.0767416621 * lms_l - 3.3077115913 * lms_m + 0.2309699292 * lms_s
    g_lin = -1.2684380046 * lms_l + 2.6097574011 * lms_m - 0.3413193965 * lms_s
    b_lin = -0.0041960863 * lms_l - 0.7034186147 * lms_m + 1.7076147010 * lms_s

    # Linear sRGB -> sRGB (gamma companding)
    def gamma(x: float) -> int:
        if x <= 0.0031308:
            v = 12.92 * x
        else:
            v = 1.055 * (x ** (1.0 / 2.4)) - 0.055
        return max(0, min(255, round(v * 255)))

    return gamma(r_lin), gamma(g_lin), gamma(b_lin)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def color_entry(entry: dict) -> dict:
    """Parse a color entry with optional bold/italic flags."""
    r, g, b = oklch_to_srgb(entry['L'], entry['C'], entry['H'])
    return dict(r=r, g=g, b=b, bold=entry.get('bold', False), italic=entry.get('italic', False))


def ansi_fg(r: int, g: int, b: int) -> str:
    """True color ANSI foreground escape."""
    return f"\\033[38;2;{r};{g};{b}m"


def ansi_const(name: str, entry: dict) -> str:
    """Generate a Python constant for a color entry."""
    esc = ansi_fg(entry['r'], entry['g'], entry['b'])
    return f'{name} = "{esc}"'


def oklch_css(L: float, C: float, H: float) -> str:
    """Format OKLCH as CSS value."""
    return f"oklch({L} {C} {H})"


# ---------------------------------------------------------------------------
# Generate catnip/_theme.py
# ---------------------------------------------------------------------------

def generate_theme_py(data: dict) -> str:
    lines = [
        '# FILE: catnip/_theme.py',
        '# GENERATED FROM catnip_rs/visual.toml',
        '# Do not edit manually. Run: make gen-theme',
        '"""Theme constants generated from visual.toml (OKLCH -> true color ANSI)."""',
        '',
        'import os',
        '',
        '',
    ]

    # Prompts
    lines.append('# Prompts')
    lines.append(f"PROMPT_MAIN = {data['prompts']['main']!r}")
    lines.append(f"PROMPT_CONTINUATION = {data['prompts']['continuation']!r}")
    lines.append('')

    # Box drawing
    lines.append('# Box drawing')
    box = data['box']
    for key in ('corner_tl', 'corner_tr', 'corner_bl', 'corner_br', 'vertical', 'horizontal'):
        lines.append(f"BOX_{key.upper()} = {box[key]!r}")
    lines.append('')

    # UI colors
    lines.append('# UI colors (true color ANSI)')
    for name, entry in data['ui'].items():
        ce = color_entry(entry)
        lines.append(ansi_const(f'UI_{name.upper()}', ce))
    lines.append('')

    # Accent colors - dark theme
    lines.append('# Accent colors - dark theme')
    for name, entry in data['accent']['dark'].items():
        ce = color_entry(entry)
        lines.append(ansi_const(f'ACCENT_DARK_{name.upper()}', ce))
        if ce['bold']:
            lines.append(f'ACCENT_DARK_{name.upper()}_BOLD = True')
        if ce['italic']:
            lines.append(f'ACCENT_DARK_{name.upper()}_ITALIC = True')
    lines.append('')

    # Accent colors - light theme
    lines.append('# Accent colors - light theme')
    for name, entry in data['accent']['light'].items():
        ce = color_entry(entry)
        lines.append(ansi_const(f'ACCENT_LIGHT_{name.upper()}', ce))
        if ce['bold']:
            lines.append(f'ACCENT_LIGHT_{name.upper()}_BOLD = True')
        if ce['italic']:
            lines.append(f'ACCENT_LIGHT_{name.upper()}_ITALIC = True')
    lines.append('')

    # Base dark
    lines.append('# Base colors (dark theme)')
    for name, entry in data['base']['dark'].items():
        ce = color_entry(entry)
        lines.append(ansi_const(f'DARK_{name.upper()}', ce))
        if ce['italic']:
            lines.append(f'DARK_{name.upper()}_ITALIC = True')
    lines.append('')

    # Base light
    lines.append('# Base colors (light theme)')
    for name, entry in data['base']['light'].items():
        ce = color_entry(entry)
        lines.append(ansi_const(f'LIGHT_{name.upper()}', ce))
        if ce['italic']:
            lines.append(f'LIGHT_{name.upper()}_ITALIC = True')
    lines.append('')

    # Pipeline stages
    lines.append('# Pipeline stages')
    for name, entry in data['stages'].items():
        ce = color_entry(entry)
        lines.append(ansi_const(f'STAGE_{name.upper()}', ce))
    lines.append('')

    # Theme detection + dynamic BASE_* bindings
    # Collect base color names (both dark and light have the same keys)
    base_keys = [k for k in data['base']['dark'].keys() if k != 'background']

    lines.append('')
    lines.append('# Theme detection')
    lines.append('def _detect_terminal_bg() -> str:')
    lines.append('    """Detect terminal background: COLORFGBG -> dark/light."""')
    lines.append('    colorfgbg = os.environ.get("COLORFGBG", "")')
    lines.append('    if colorfgbg:')
    lines.append('        parts = colorfgbg.split(";")')
    lines.append('        try:')
    lines.append('            bg = int(parts[-1])')
    lines.append('            return "light" if bg > 8 else "dark"')
    lines.append('        except (ValueError, IndexError):')
    lines.append('            pass')
    lines.append('    return "dark"')
    lines.append('')
    lines.append('')
    lines.append('_theme = os.environ.get("CATNIP_THEME", "").lower()')
    lines.append('if _theme not in ("dark", "light"):')
    lines.append('    _theme = _detect_terminal_bg()')
    lines.append('')
    lines.append('DETECTED_THEME = _theme')
    lines.append('')

    # Generate BASE_* dynamic bindings
    lines.append('# Dynamic base colors (resolved at import time from detected theme)')
    for key in base_keys:
        upper = key.upper()
        lines.append(
            f'BASE_{upper} = DARK_{upper} if _theme == "dark" else LIGHT_{upper}'
        )
    lines.append('')

    # Generate ACCENT_* dynamic bindings
    accent_keys = list(data['accent']['dark'].keys())
    lines.append('# Dynamic accent colors (resolved at import time from detected theme)')
    for key in accent_keys:
        upper = key.upper()
        lines.append(
            f'ACCENT_{upper} = ACCENT_DARK_{upper} if _theme == "dark" else ACCENT_LIGHT_{upper}'
        )
    lines.append('')

    return '\n'.join(lines)


# ---------------------------------------------------------------------------
# Generate web/docs/src/highlight.css
# ---------------------------------------------------------------------------

def css_color(entry: dict) -> str:
    """CSS oklch() from raw TOML entry."""
    return oklch_css(entry['L'], entry['C'], entry['H'])


def css_style(entry: dict) -> str:
    """Extra CSS properties from entry flags."""
    parts = []
    if entry.get('bold'):
        parts.append('font-weight: bold;')
    if entry.get('italic'):
        parts.append('font-style: italic;')
    return ' '.join(parts)


def generate_pygments_css(data: dict) -> str:
    accent_dark = data['accent']['dark']
    accent_light = data['accent']['light']
    dark = data['base']['dark']
    light = data['base']['light']
    ui = data['ui']

    error_c = css_color(ui['error'])

    def mode_block(base: dict, accent: dict, indent: str = '    ') -> str:
        """Generate CSS rules for a base+accent theme (light or dark)."""
        fg = css_color(base['foreground'])
        bg = css_color(base['background'])
        comment_c = css_color(base['comment'])
        punctuation_c = css_color(base['punctuation'])
        keyword_c = css_color(accent['keyword'])
        constant_c = css_color(accent['constant'])
        type_c = css_color(accent['type'])
        number_c = css_color(accent['number'])
        string_c = css_color(accent['string'])
        builtin_c = css_color(accent['builtin'])

        # comment style flags
        comment_extra = ''
        if base['comment'].get('italic'):
            comment_extra = f'\n{indent}    font-style: italic;'

        return f"""{indent}color: {fg};

{indent}& pre {{
{indent}    line-height: 125%;
{indent}}}

{indent}& td.linenos .normal,
{indent}& span.linenos {{
{indent}    color: {comment_c};
{indent}    background-color: {bg};
{indent}    padding-left: 5px;
{indent}    padding-right: 5px;
{indent}}}

{indent}& td.linenos .special,
{indent}& span.linenos.special {{
{indent}    color: oklch(0 0 0);
{indent}    background-color: oklch(0.98 0.08 102);
{indent}    padding-left: 5px;
{indent}    padding-right: 5px;
{indent}}}

{indent}& .hll {{
{indent}    background-color: {bg};
{indent}}}

{indent}& .c,   /* Comment */
{indent}& .ch,  /* Comment.Hashbang */
{indent}& .cm,  /* Comment.Multiline */
{indent}& .c1,  /* Comment.Single */
{indent}& .cs {{
{indent}    /* Comment.Special */
{indent}    color: {comment_c};{comment_extra}
{indent}}}

{indent}& .err {{
{indent}    color: {fg};
{indent}    background-color: {error_c};
{indent}}} /* Error */

{indent}& .esc, /* Escape */
{indent}& .g,   /* Generic */
{indent}& .l,   /* Literal */
{indent}& .n,   /* Name */
{indent}& .x,   /* Other */
{indent}& .p {{
{indent}    /* Punctuation */
{indent}    color: {punctuation_c};
{indent}}}

{indent}& .k,  /* Keyword */
{indent}& .kp, /* Keyword.Pseudo */
{indent}& .kr, /* Keyword.Reserved */
{indent}& .ow {{
{indent}    /* Operator.Word */
{indent}    color: {keyword_c};
{indent}}}

{indent}& .o,   /* Operator */
{indent}& .cpf {{
{indent}    /* Comment.PreprocFile */
{indent}    color: {punctuation_c};
{indent}}}

{indent}& .cp {{
{indent}    color: oklch(0.61 0.19 353);
{indent}}} /* Comment.Preproc */

{indent}& .gd, /* Generic.Deleted */
{indent}& .gr {{
{indent}    /* Generic.Error */
{indent}    color: {error_c};
{indent}}}

{indent}& .ge {{
{indent}    color: {fg};
{indent}    font-style: italic;
{indent}}} /* Generic.Emph */

{indent}& .ges {{
{indent}    color: {fg};
{indent}    font-weight: bold;
{indent}    font-style: italic;
{indent}}} /* Generic.EmphStrong */

{indent}& .gh {{
{indent}    color: {fg};
{indent}    font-weight: bold;
{indent}}} /* Generic.Heading */

{indent}& .gi {{
{indent}    color: {keyword_c};
{indent}}} /* Generic.Inserted */

{indent}& .go {{
{indent}    color: {fg};
{indent}}} /* Generic.Output */

{indent}& .gp {{
{indent}    color: {builtin_c};
{indent}    font-weight: bold;
{indent}}} /* Generic.Prompt */

{indent}& .gs {{
{indent}    color: {fg};
{indent}    font-weight: bold;
{indent}}} /* Generic.Strong */

{indent}& .gu {{
{indent}    color: {fg};
{indent}    text-decoration: underline;
{indent}}} /* Generic.Subheading */

{indent}& .gt {{
{indent}    color: {builtin_c};
{indent}}} /* Generic.Traceback */

{indent}& .kc, /* Keyword.Constant */
{indent}& .kd {{
{indent}    /* Keyword.Declaration */
{indent}    color: {constant_c};
{indent}}}

{indent}& .kn {{
{indent}    color: oklch(0.59 0.17 51);
{indent}}} /* Keyword.Namespace */

{indent}& .kt {{
{indent}    color: {type_c};
{indent}}} /* Keyword.Type */

{indent}& .ld {{
{indent}    color: {fg};
{indent}}} /* Literal.Date */

{indent}& .m,  /* Literal.Number */
{indent}& .mb, /* Literal.Number.Bin */
{indent}& .mf, /* Literal.Number.Float */
{indent}& .mh, /* Literal.Number.Hex */
{indent}& .mi, /* Literal.Number.Integer */
{indent}& .mo, /* Literal.Number.Oct */
{indent}& .il {{
{indent}    /* Literal.Number.Integer.Long */
{indent}    color: {number_c};
{indent}}}

{indent}& .s,  /* Literal.String */
{indent}& .sa, /* Literal.String.Affix */
{indent}& .sb, /* Literal.String.Backtick */
{indent}& .sc, /* Literal.String.Char */
{indent}& .dl, /* Literal.String.Delimiter */
{indent}& .s2, /* Literal.String.Double */
{indent}& .se, /* Literal.String.Escape */
{indent}& .sh, /* Literal.String.Heredoc */
{indent}& .si, /* Literal.String.Interpol */
{indent}& .sx, /* Literal.String.Other */
{indent}& .s1, /* Literal.String.Single */
{indent}& .ss {{
{indent}    /* Literal.String.Symbol */
{indent}    color: {string_c};
{indent}}}

{indent}& .sr {{
{indent}    color: oklch(0.59 0.17 51);
{indent}}} /* Literal.String.Regex */

{indent}& .sd {{
{indent}    color: {comment_c};
{indent}}} /* Literal.String.Doc */

{indent}& .na, /* Name.Attribute */
{indent}& .nx, /* Name.Other */
{indent}& .py, /* Name.Property */
{indent}& .pm, /* Punctuation.Marker */
{indent}& .w {{
{indent}    /* Text.Whitespace */
{indent}    color: {fg};
{indent}}}

{indent}& .nb, /* Name.Builtin */
{indent}& .nc, /* Name.Class */
{indent}& .no, /* Name.Constant */
{indent}& .nd, /* Name.Decorator */
{indent}& .ni, /* Name.Entity */
{indent}& .ne, /* Name.Exception */
{indent}& .nf, /* Name.Function */
{indent}& .nl, /* Name.Label */
{indent}& .nn, /* Name.Namespace */
{indent}& .nt, /* Name.Tag */
{indent}& .nv, /* Name.Variable */
{indent}& .bp, /* Name.Builtin.Pseudo */
{indent}& .fm, /* Name.Function.Magic */
{indent}& .vc, /* Name.Variable.Class */
{indent}& .vg, /* Name.Variable.Global */
{indent}& .vi, /* Name.Variable.Instance */
{indent}& .vm {{
{indent}    /* Name.Variable.Magic */
{indent}    color: {builtin_c};
{indent}}}"""

    light_css = mode_block(light, accent_light, '    ')
    dark_css = mode_block(dark, accent_dark, '        ')

    return f""".codehilite {{
    /* Light mode (default) - generated from visual.toml */
{light_css}

    /* Dark mode - generated from visual.toml */
    &[data-theme="dark"] {{
{dark_css}
    }}
}}
"""


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    base = Path(__file__).parent
    toml_path = base / 'visual.toml'
    data = tomllib.loads(toml_path.read_text())
    normalize_colors(data)

    # Generate Python theme
    print("Generating theme from visual.toml...")
    theme_py = generate_theme_py(data)
    py_output = base.parent / 'catnip' / '_theme.py'
    py_output.write_text(theme_py)
    print(f"  Generated {py_output}")

    # Generate Pygments CSS
    css = generate_pygments_css(data)
    css_output = base.parent / 'web' / 'docs' / 'src' / 'highlight.css'
    css_output.write_text(css)
    print(f"  Generated {css_output}")

    print("\nDone! Flow: visual.toml -> _theme.py + highlight.css")


if __name__ == '__main__':
    main()
