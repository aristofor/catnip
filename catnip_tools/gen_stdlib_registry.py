#!/usr/bin/env python3
# FILE: catnip_tools/gen_stdlib_registry.py
"""Generate stdlib module registry from spec.toml files (source of truth).

Flow: catnip_libs/*/spec.toml → 4 targets (generated)

Targets:
- catnip/loader.py                   _STDLIB_MODULES dict
- catnip_core/src/loader/resolve.rs  STDLIB_MODULES const
- setup.py                           RustExtension entries
- Cargo.toml                         workspace members
"""

import re
import tomllib
from pathlib import Path


def discover_modules(libs_dir: Path) -> list[dict]:
    """Discover stdlib modules from spec.toml files, sorted by name."""
    modules = []
    for spec_path in sorted(libs_dir.glob("*/spec.toml")):
        with open(spec_path, 'rb') as f:
            spec = tomllib.load(f)
        mod = spec['module']
        name = mod['name']
        backends = spec.get('backends', {})
        rust_backend = backends.get('rust', '')
        has_rust = bool(rust_backend) and (spec_path.parent / rust_backend / 'Cargo.toml').exists()
        has_pyo3 = has_rust and backends.get('pyo3', True)  # default True for backwards compat
        modules.append(dict(
            name=name,
            import_name=f"catnip_{name}",
            needs_configure=mod.get('needs_configure', False),
            dir_name=spec_path.parent.name,
            rust_path=rust_backend.rstrip('/') if has_rust else '',
            has_rust=has_rust,
            has_pyo3=has_pyo3,
        ))
    return modules


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


def gen_python_loader(modules: list[dict]) -> list[str]:
    """Generate _STDLIB_MODULES dict for catnip/loader.py (only modules with PyO3 backend)."""
    lines = ["_STDLIB_MODULES = {"]
    for m in modules:
        if not m['has_pyo3']:
            continue
        lines.append(f"    '{m['name']}': ('{m['import_name']}', {m['needs_configure']}),")
    lines.append("}")
    return lines


def gen_rust_resolve(modules: list[dict]) -> list[str]:
    """Generate STDLIB_MODULES const for catnip_core/src/loader/resolve.rs (only modules with PyO3 backend)."""
    entries = ", ".join(
        f'("{m["name"]}", "{m["import_name"]}", {str(m["needs_configure"]).lower()})'
        for m in modules if m['has_pyo3']
    )
    return [f"pub const STDLIB_MODULES: &[(&str, &str, bool)] = &[{entries}];"]


def gen_setup_extensions(modules: list[dict]) -> list[str]:
    """Generate RustExtension entries for setup.py (only modules with PyO3 backend)."""
    lines = []
    for m in modules:
        if not m['has_pyo3']:
            continue
        lines.append(f"    RustExtension(")
        lines.append(f'        "catnip.{m["import_name"]}",')
        lines.append(f'        path="catnip_libs/{m["dir_name"]}/{m["rust_path"]}/Cargo.toml",')
        lines.append(f"        binding=Binding.PyO3,")
        lines.append(f"        debug=False,")
        lines.append(f"        args=_profile_args,")
        lines.append(f"    ),")
    return lines


def gen_cargo_workspace(modules: list[dict]) -> list[str]:
    """Generate workspace members for Cargo.toml (only modules with Rust backend)."""
    return [f'    "catnip_libs/{m["dir_name"]}/{m["rust_path"]}",' for m in modules if m['has_rust']]


def main():
    base = Path(__file__).parent.parent
    libs_dir = base / "catnip_libs"

    modules = discover_modules(libs_dir)
    if not modules:
        print("No modules found in catnip_libs/")
        return

    print(f"Found {len(modules)} stdlib modules: {', '.join(m['name'] for m in modules)}")

    # 1. Python loader
    changed = replace_between_markers(
        base / "catnip" / "loader.py",
        "# @generated-stdlib-start",
        "# @generated-stdlib-end",
        gen_python_loader(modules),
    )
    print(f"  loader.py:   {'updated' if changed else 'up to date'}")

    # 2. Rust resolve
    changed = replace_between_markers(
        base / "catnip_core" / "src" / "loader" / "resolve.rs",
        "// @generated-stdlib-start",
        "// @generated-stdlib-end",
        gen_rust_resolve(modules),
    )
    print(f"  resolve.rs:  {'updated' if changed else 'up to date'}")

    # 3. setup.py extensions
    changed = replace_between_markers(
        base / "setup.py",
        "    # @generated-stdlib-extensions-start",
        "    # @generated-stdlib-extensions-end",
        gen_setup_extensions(modules),
    )
    print(f"  setup.py:    {'updated' if changed else 'up to date'}")

    # 4. Cargo.toml workspace
    changed = replace_between_markers(
        base / "Cargo.toml",
        "    # @generated-stdlib-workspace-start",
        "    # @generated-stdlib-workspace-end",
        gen_cargo_workspace(modules),
    )
    print(f"  Cargo.toml:  {'updated' if changed else 'up to date'}")

    print("\nNote: PureVM stdlib (catnip_vm/src/stdlib.rs) doit etre mis a jour manuellement")
    print("Done! Flow: catnip_libs/*/spec.toml (source) -> loader.py + resolve.rs + setup.py + Cargo.toml")


if __name__ == "__main__":
    main()
