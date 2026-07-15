//! Test-only fixtures: build the native plugin cdylibs that tests dlopen.
//!
//! The plugin crates (`catnip_hello`, `catnip-io`, `catnip-sys`) are workspace
//! members but not dependencies, so `cargo test -p catnip_vm` never builds
//! them. Historically the plugin fixture tests built them mid-run, and the 18
//! stdlib io/sys tests only passed when test scheduling ran a fixture first —
//! under load (nested `cargo build` blocked on the target-dir lock) the whole
//! stdlib group failed. Building here, once per process behind a `OnceLock`,
//! removes the ordering dependency and serializes the builds away from any
//! concurrent dlopen.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

fn build(package: &str, no_default_features: bool, lib_stem: &str) -> PathBuf {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "-p", package]);
    if no_default_features {
        // io/sys default to `extension-module` (PyO3 cdylib); the native
        // plugin ABI needs the bare build.
        cmd.arg("--no-default-features");
    }
    let status = cmd.current_dir(&workspace).status().expect("failed to run cargo build");
    assert!(status.success(), "cargo build -p {package} failed");
    workspace
        .join("target/debug")
        .join(format!("lib{lib_stem}{}", crate::plugin::native_suffix()))
}

pub(crate) fn hello_plugin() -> PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| build("catnip_hello", false, "catnip_hello"))
        .clone()
}

pub(crate) fn io_plugin() -> PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| build("catnip-io", true, "catnip_io")).clone()
}

pub(crate) fn sys_plugin() -> PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| build("catnip-sys", true, "catnip_sys")).clone()
}

/// Ensure the native stdlib plugin backing `name` exists before the loader
/// searches for it. No-op for names without a plugin fixture.
pub(crate) fn ensure_stdlib_plugin(name: &str) {
    match name {
        "io" => {
            io_plugin();
        }
        "sys" => {
            sys_plugin();
        }
        _ => {}
    }
}
