#!/usr/bin/env python3
# FILE: catnip_tools/gen_builtins.py
"""Generate builtin name lists from context.py (source of truth).

Flow: context.py (globals) + catnip_core constants.rs (pure builtins) → Rust targets (generated)

Targets:
- catnip_tools/src/linter/semantic.rs  BUILTINS (scope analysis)
- catnip_tools/src/lint_cfg/mod.rs     BUILTINS (CFG definite-assignment)
- catnip_repl/src/completer.rs     BUILTINS (completion suggestions)

Each target uses its own marker pair for replacement.

Note: JIT_PURE_BUILTINS flows the other way since context.py imports it from
catnip._rs (constants.rs is the source of truth, no generation needed).
"""

import re
from pathlib import Path

from _genutil import replace_between_markers

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


def extract_pure_builtins(constants_path: Path) -> list[str]:
    """Extract JIT_PURE_BUILTINS from catnip_core constants.rs (source of truth).

    context.py's KNOWN_PURE_FUNCTIONS is a runtime reflection of this array
    (its literal fallback tuple was removed), so the generator reads the Rust
    source directly, like gen_opcodes does.
    """
    content = constants_path.read_text()
    pattern = r'pub const JIT_PURE_BUILTINS: &\[&str\] = &\[(.*?)\];'
    match = re.search(pattern, content, re.DOTALL)
    if not match:
        raise ValueError(f"JIT_PURE_BUILTINS not found in {constants_path}")

    return re.findall(r'"(\w+)"', match.group(1))


def generate_rust_array(names: list[str], const_decl: str) -> list[str]:
    """Generate sorted Rust &[&str] array lines."""
    lines = [f"{const_decl} = &["]
    for name in sorted(set(names)):
        lines.append(f'    "{name}",')
    lines.append("];")
    return lines


def main():
    base = Path(__file__).parent.parent
    context_path = base / "catnip" / "context.py"

    # ── Extract from source of truth ──────────────────────────────────
    print("Extracting from context.py...")
    globals_names = extract_context_builtins(context_path)
    pure_names = extract_pure_builtins(base / "catnip_core" / "src" / "constants.rs")
    print(f"  globals: {len(globals_names)} names")
    print(f"  pure:    {len(pure_names)} names")

    # ── 1. Linter builtins ────────────────────────────────────────────
    linter_path = base / "catnip_tools" / "src" / "linter" / "semantic.rs"
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
    cfg_path = base / "catnip_tools" / "src" / "lint_cfg" / "mod.rs"
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

    print("\nDone! Flow: context.py (globals) + catnip_core constants.rs (pure) -> linter + completer + constants")


if __name__ == "__main__":
    main()
