// FILE: catnip_rs/tests/run_imports.rs
/// Regression tests for `.cat` module imports in the standalone binary.
///
/// The embedded binary links catnip_rs statically and registers it as
/// `catnip._rs`, so the parent VM and the child `Catnip()` that loads a module
/// share one set of thread-local tables. This lets the loader transplant the
/// child's func_table / symbol / enum / struct registries into the parent so
/// exported functions (and the siblings they call by name) keep resolving after
/// the child VM is dropped.
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::run_binary;

/// Create a unique temp directory for a test's module files.
fn temp_dir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("catnip_import_test_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Write a file under `dir` and return nothing (panics on error).
fn write(dir: &Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).expect("write module file");
}

/// Run the binary on `main.cat` inside `dir` and return trimmed stdout.
fn run_main(dir: &Path) -> String {
    let output = Command::new(run_binary())
        .arg(dir.join("main.cat"))
        .output()
        .expect("Failed to execute catnip");
    assert!(
        output.status.success(),
        "binary failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn test_import_inter_function_call() {
    let dir = temp_dir("inter");
    write(
        &dir,
        "m.cat",
        "double = (x) => { x * 2 }\nquad = (x) => { double(double(x)) }",
    );
    write(&dir, "main.cat", "m = import(\"m\")\nprint(m.quad(3))");
    assert_eq!(run_main(&dir), "12");
}

#[test]
fn test_import_recursion_and_private_helper() {
    let dir = temp_dir("rec");
    write(
        &dir,
        "m.cat",
        "_helper = (x) => { x * 10 }\n\
         scale = (x) => { _helper(x) + 1 }\n\
         fact = (n) => { match n { 0 => { 1 } _ => { n * fact(n - 1) } } }",
    );
    write(
        &dir,
        "main.cat",
        "m = import(\"m\")\nprint(m.scale(4))\nprint(m.fact(5))",
    );
    assert_eq!(run_main(&dir), "41\n120");
}

#[test]
fn test_import_union_variant_match() {
    // A union variant returned from a module function still matches in the
    // caller (symbol ids remapped across the transplant). The type must be
    // brought into scope (alias) since enum patterns raise NameError on an
    // out-of-scope type.
    let dir = temp_dir("union");
    write(
        &dir,
        "m.cat",
        "union Color { red; green; blue }\nfavorite = () => { Color.green }",
    );
    write(
        &dir,
        "main.cat",
        "m = import(\"m\")\n\
         Color = m.Color\n\
         c = m.favorite()\n\
         print(match c { Color.red => { \"r\" } Color.green => { \"g\" } _ => { \"?\" } })",
    );
    assert_eq!(run_main(&dir), "g");
}
