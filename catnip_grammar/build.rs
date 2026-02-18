fn main() {
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src");

    println!("cargo:rerun-if-changed={}", dir.join("parser.c").display());
    println!("cargo:rerun-if-changed={}", dir.join("scanner.c").display());

    cc::Build::new()
        .include(&dir)
        .file(dir.join("parser.c"))
        .file(dir.join("scanner.c"))
        .compile("tree-sitter-catnip");

    println!("cargo:rustc-link-lib=static=tree-sitter-catnip");
    println!(
        "cargo:rustc-link-search=native={}",
        std::env::var("OUT_DIR").unwrap()
    );
}
