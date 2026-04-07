#!/usr/bin/env python3
# FILE: catnip_tools/gen_builtins.py
"""Generate builtin name lists from context.py (source of truth).

Flow: Python context.py (source) → Rust targets (generated)

Targets:
- catnip_tools/src/linter.rs       BUILTINS (scope analysis)
- catnip_tools/src/lint_cfg.rs     BUILTINS (CFG definite-assignment)
- catnip_repl/src/completer.rs     BUILTINS (completion suggestions)
- catnip_core/src/constants.rs     JIT_PURE_BUILTINS (pure function tracking)

Each target uses its own marker pair for replacement.
"""

import re
from pathlib import Path

# Runtime names not in context.py globals but always available at execution.
# Used by both linter (E200 scope) and CFG (W310 definite-assignment).
EXTRA_RUNTIME_NAMES = [
    "True",         # boolean literal (also a name in scope)
    "False",        # boolean literal
    "None",         # none literal
    "print",        # builtin function (injected by Python, not in globals.update)
    "input",        # builtin function
    "open",         # builtin function
    "breakpoint",   # transformed to opcode by semantic analyzer
    "typeof",       # intrinsic, handled by VM directly
    "pragma",       # intrinsic directive
]

# Syntax-only names the linter scope tracker needs to suppress E200,
# but that are NOT runtime variables (decorator keywords, Python internals).
# Excluded from CFG analysis to avoid masking W310 on user variables.
EXTRA_LINTER_ONLY_NAMES = [
    "_",            # wildcard pattern
    "static",       # decorator keyword
    "abstract",     # decorator keyword
    "globals",      # Python builtin (not exposed in Catnip runtime)
    "locals",       # Python builtin
    "exec",         # Python builtin
    "eval",         # Python builtin
    "compile",      # Python builtin
    "__import__",   # Python builtin
]


def extract_context_builtins(context_path: Path) -> list[str]:
    """Extract builtin names from context.py globals dict."""
    content = context_path.read_text()

    pattern = r'self\.globals\.update\(\s*\{(.*?)\}\s*\)'
    match = re.search(pattern, content, re.DOTALL)
    if not match:
        raise ValueError(f"globals.update block not found in {context_path}")

    block = match.group(1)
    return re.findall(r'["\'](\w+)["\'](?:\s*:)', block)


def extract_pure_builtins(context_path: Path) -> list[str]:
    """Extract KNOWN_PURE_FUNCTIONS names from context.py."""
    content = context_path.read_text()

    # Match the fallback tuple in KNOWN_PURE_FUNCTIONS (after `or (`)
    pattern = r'KNOWN_PURE_FUNCTIONS\s*=\s*frozenset\(\s*_RUST_JIT_PURE_BUILTINS\s*or\s*\((.*?)\)\s*\)'
    match = re.search(pattern, content, re.DOTALL)
    if not match:
        raise ValueError(f"KNOWN_PURE_FUNCTIONS not found in {context_path}")

    block = match.group(1)
    return re.findall(r"['\"](\w+)['\"]", block)


def generate_rust_array(names: list[str], const_decl: str) -> list[str]:
    """Generate sorted Rust &[&str] array lines."""
    lines = [f"{const_decl} = &["]
    for name in sorted(set(names)):
        lines.append(f'    "{name}",')
    lines.append("];")
    return lines


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


def main():
    base = Path(__file__).parent.parent
    context_path = base / "catnip" / "context.py"

    # ── Extract from source of truth ──────────────────────────────────
    print("Extracting from context.py...")
    globals_names = extract_context_builtins(context_path)
    pure_names = extract_pure_builtins(context_path)
    print(f"  globals: {len(globals_names)} names")
    print(f"  pure:    {len(pure_names)} names")

    # ── 1. Linter builtins ────────────────────────────────────────────
    linter_path = base / "catnip_tools" / "src" / "linter.rs"
    linter_names = globals_names + EXTRA_RUNTIME_NAMES + EXTRA_LINTER_ONLY_NAMES
    linter_lines = generate_rust_array(linter_names, "const BUILTINS: &[&str]")

    changed = replace_between_markers(
        linter_path,
        "// @generated-builtins-start",
        "// @generated-builtins-end",
        linter_lines,
    )
    print(f"  linter:    {'updated' if changed else 'up to date'} ({len(sorted(set(linter_names)))} names)")

    # ── 2. CFG linter builtins ────────────────────────────────────────
    # Runtime names only (no decorator/syntax-only pseudo builtins).
    cfg_path = base / "catnip_tools" / "src" / "lint_cfg.rs"
    cfg_names = globals_names + EXTRA_RUNTIME_NAMES
    cfg_lines = generate_rust_array(cfg_names, "const BUILTINS: &[&str]")

    changed = replace_between_markers(
        cfg_path,
        "// @generated-cfg-builtins-start",
        "// @generated-cfg-builtins-end",
        cfg_lines,
    )
    print(f"  cfg lint:  {'updated' if changed else 'up to date'} ({len(sorted(set(cfg_names)))} names)")

    # ── 3. REPL completer builtins ────────────────────────────────────
    # Intrinsics not in context.py but available to users
    extra_completer = ["typeof", "breakpoint"]
    completer_path = base / "catnip_repl" / "src" / "completer.rs"
    completer_lines = generate_rust_array(globals_names + extra_completer, "const BUILTINS: &[&str]")

    changed = replace_between_markers(
        completer_path,
        "// @generated-completer-builtins-start",
        "// @generated-completer-builtins-end",
        completer_lines,
    )
    print(f"  completer: {'updated' if changed else 'up to date'} ({len(sorted(set(globals_names)))} names)")

    # ── 4. JIT pure builtins ──────────────────────────────────────────
    constants_path = base / "catnip_core" / "src" / "constants.rs"
    pure_lines = generate_rust_array(pure_names, "pub const JIT_PURE_BUILTINS: &[&str]")

    changed = replace_between_markers(
        constants_path,
        "// @generated-pure-builtins-start",
        "// @generated-pure-builtins-end",
        pure_lines,
    )
    print(f"  jit pure:  {'updated' if changed else 'up to date'} ({len(sorted(set(pure_names)))} names)")

    print("\nDone! Flow: context.py (source) -> linter.rs + lint_cfg.rs + completer.rs + constants.rs")


if __name__ == "__main__":
    main()
