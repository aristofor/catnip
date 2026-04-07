# FILE: catnip/cli/commands/new_lib.py
"""Scaffold a new stdlib extension module in catnip_libs/."""

from pathlib import Path

import click

_SPEC_TEMPLATE = """\
[module]
name = "{name}"
version = "0.1.0"
description = ""

[exports]
symbols = ["PROTOCOL", "VERSION"]

[backends]
rust = "rust/"
"""

_CARGO_TEMPLATE = """\
[package]
name = "catnip-{name}"
version.workspace = true
edition.workspace = true

[lib]
name = "catnip_{name}"
crate-type = ["cdylib", "rlib"]

[features]
default = ["extension-module"]
extension-module = ["pyo3/extension-module"]
embedded = ["pyo3/auto-initialize"]

[dependencies]
pyo3 = {{ workspace = true }}
"""

_LIB_TEMPLATE = """\
//! Catnip `{name}` module.

use pyo3::prelude::*;

/// Build the module for embedded use.
pub fn build_module(py: Python<'_>) -> PyResult<Py<PyModule>> {{
    let m = PyModule::new(py, "{name}")?;
    register_items(&m)?;
    Ok(m.unbind())
}}

fn register_items(m: &Bound<'_, PyModule>) -> PyResult<()> {{
    m.add("PROTOCOL", "rust")?;
    m.add("VERSION", "0.1.0")?;
    Ok(())
}}

#[pymodule]
fn catnip_{name}(m: &Bound<'_, PyModule>) -> PyResult<()> {{
    register_items(m)
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use pyo3::Python;

    #[test]
    fn test_build_module() {{
        Python::attach(|py| {{
            let m = build_module(py).unwrap();
            let m = m.bind(py);
            assert_eq!(m.getattr("PROTOCOL").unwrap().extract::<String>().unwrap(), "rust");
            assert_eq!(m.getattr("VERSION").unwrap().extract::<String>().unwrap(), "0.1.0");
        }});
    }}
}}
"""

_TEST_TEMPLATE = """\
import('{name}')

if {name}.PROTOCOL != "rust" {{ raise("{name}.PROTOCOL should be 'rust'") }}
if {name}.VERSION != "0.1.0" {{ raise("{name}.VERSION should be '0.1.0'") }}
"""


@click.command('new-lib')
@click.argument('name')
@click.option('--path', default='catnip_libs', help="Parent directory", show_default=True)
def cmd_new_lib(name, path):
    """Scaffold a new stdlib extension module."""
    base = Path(path) / name
    if base.exists():
        raise click.ClickException(f"{base} already exists")

    # Validate name
    if not name.isidentifier():
        raise click.ClickException(f"'{name}' is not a valid Python identifier")

    rust_src = base / "rust" / "src"
    tests_dir = base / "tests"

    rust_src.mkdir(parents=True)
    tests_dir.mkdir(parents=True)

    (base / "spec.toml").write_text(_SPEC_TEMPLATE.format(name=name))
    (base / "rust" / "Cargo.toml").write_text(_CARGO_TEMPLATE.format(name=name))
    (rust_src / "lib.rs").write_text(_LIB_TEMPLATE.format(name=name))
    (tests_dir / "test_constants.cat").write_text(_TEST_TEMPLATE.format(name=name))

    click.echo(f"Created {base}/")
    click.echo()
    click.echo("Next steps:")
    click.echo("  1. make gen-stdlib-registry")
    click.echo("  2. pip install -e .")
    click.echo("  3. cd catnip_libs && make test")
