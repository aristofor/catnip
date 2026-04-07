// FILE: catnip_core/src/loader/resolve.rs
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

/// Module kind determined by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    Catnip,
    Python,
    Native,
    Package,
}

/// Valid protocols for import().
pub const PROTOCOLS: &[&str] = &["py", "rs", "cat"];

/// Stdlib modules: (catnip_name, rust_import_name, needs_configure).
// @generated-stdlib-start
pub const STDLIB_MODULES: &[(&str, &str, bool)] = &[("io", "catnip_io", false), ("sys", "catnip_sys", true)];
// @generated-stdlib-end

/// File extensions that are rejected in import specs.
const FILE_EXTENSIONS: &[&str] = &[".cat", ".py", ".pyc", ".pyo", ".so", ".pyd", ".dll"];

/// Path indicators that signal legacy file-path imports.
const PATH_INDICATORS: &[&str] = &["./", "../", "/", "\\"];

// Native suffix is determined at runtime via sysconfig (passed as parameter).

// -- spec validation ---------------------------------------------------------

/// Validate an import spec. Returns Ok(spec) or Err with user-facing message.
pub fn validate_spec(spec: &str) -> Result<&str, String> {
    if spec.is_empty() {
        return Err("import: empty module spec".into());
    }

    // Relative imports: leading dots without '/' pass through
    if spec.starts_with('.') && !spec.starts_with("./") && !spec.starts_with("../") {
        let stripped = spec.trim_start_matches('.');
        if stripped.is_empty() {
            return Err(format!(
                "invalid relative import: '{}'\n  relative imports require a module name after the dots",
                spec
            ));
        }
        return Ok(spec);
    }

    // Detect old path-based imports
    for indicator in PATH_INDICATORS {
        if spec.starts_with(indicator) {
            let stem = Path::new(spec).file_stem().and_then(|s| s.to_str()).unwrap_or(spec);
            return Err(format!(
                "file paths in import() are no longer supported: '{}'\n  use a bare name instead: import(\"{}\")",
                spec, stem
            ));
        }
    }
    if spec.contains('/') || spec.contains('\\') {
        let stem = Path::new(spec).file_stem().and_then(|s| s.to_str()).unwrap_or(spec);
        return Err(format!(
            "file paths in import() are no longer supported: '{}'\n  use a bare name instead: import(\"{}\")",
            spec, stem
        ));
    }

    // Detect file extensions in spec
    if let Some(dot_pos) = spec.rfind('.') {
        let suffix = &spec[dot_pos..];
        if FILE_EXTENSIONS.contains(&suffix) {
            let stem = &spec[..dot_pos];
            let proto = if suffix == ".cat" { "cat" } else { "py" };
            return Err(format!(
                "file extensions in import() are no longer supported: '{}'\n  use: import(\"{}\", protocol=\"{}\") or import(\"{}\")",
                spec, stem, proto, stem
            ));
        }
    }

    Ok(spec)
}

// -- relative spec parsing ---------------------------------------------------

/// Parse leading dots into (level, name). Returns (0, spec) if not relative.
pub fn parse_relative_spec(spec: &str) -> (usize, &str) {
    let level = spec.bytes().take_while(|&b| b == b'.').count();
    if level == 0 { (0, spec) } else { (level, &spec[level..]) }
}

// -- search dirs -------------------------------------------------------------

/// Build resolution directories: caller_dir -> CWD -> CATNIP_PATH (deduplicated).
/// `cwd` should be passed from the Python side to respect mocks.
pub fn search_dirs(caller_dir: Option<&Path>, cwd: Option<&Path>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut dirs = Vec::new();

    let mut push_if_new = |p: PathBuf| {
        if let Ok(resolved) = p.canonicalize() {
            if seen.insert(resolved.clone()) {
                dirs.push(resolved);
            }
        } else if p.is_dir() && seen.insert(p.clone()) {
            dirs.push(p);
        }
    };

    if let Some(d) = caller_dir {
        push_if_new(d.to_path_buf());
    }
    let effective_cwd = cwd.map(|p| p.to_path_buf()).or_else(|| env::current_dir().ok());
    if let Some(c) = effective_cwd {
        push_if_new(c);
    }
    if let Ok(env_path) = env::var("CATNIP_PATH") {
        for entry in env_path.split(if cfg!(windows) { ';' } else { ':' }) {
            if !entry.is_empty() {
                let p = PathBuf::from(entry);
                if p.is_dir() {
                    push_if_new(p);
                }
            }
        }
    }

    dirs
}

// -- extension priority ------------------------------------------------------

/// Return (extension, kind) pairs for a given protocol.
pub fn extensions_for_protocol(protocol: Option<&str>) -> Vec<(&'static str, ModuleKind)> {
    match protocol {
        Some("cat") => vec![(".cat", ModuleKind::Catnip)],
        Some("py") => vec![(".py", ModuleKind::Python)],
        Some("rs") => vec![],
        _ => vec![(".cat", ModuleKind::Catnip), (".py", ModuleKind::Python)],
    }
}

// -- bare name resolution ----------------------------------------------------

/// Search directories for a bare name. Returns (path, kind) or None.
pub fn resolve_bare_name(
    name: &str,
    caller_dir: Option<&Path>,
    protocol: Option<&str>,
    native_suffix: &str,
    cwd: Option<&Path>,
) -> Option<(PathBuf, ModuleKind)> {
    let file_name = name.replace('.', "/");
    let exts = extensions_for_protocol(protocol);
    let dirs = search_dirs(caller_dir, cwd);

    // Check packages first (dirs with lib.toml), only for non-dotted names
    if !name.contains('.') {
        for d in &dirs {
            let pkg_dir = d.join(name);
            if pkg_dir.is_dir() && pkg_dir.join("lib.toml").is_file() {
                return Some((pkg_dir, ModuleKind::Package));
            }
        }
    }

    // Search for files with extensions
    for d in &dirs {
        for &(ext, kind) in &exts {
            let candidate = d.join(format!("{}{}", file_name, ext));
            if candidate.is_file() {
                return Some((candidate, kind));
            }
        }
        // Also check native extensions (.so/.dylib)
        if protocol != Some("cat") && !native_suffix.is_empty() {
            // Try multiple naming conventions: name.so, libname.so, libcatnip_name.so
            for prefix in ["", "lib", "libcatnip_"] {
                let candidate = d.join(format!("{}{}{}", prefix, file_name, native_suffix));
                if candidate.is_file() {
                    return Some((candidate, ModuleKind::Native));
                }
            }
        }
    }

    None
}

/// Resolve a relative import from caller_dir. Returns (path, kind) or error.
pub fn resolve_relative(
    spec: &str,
    caller_dir: &Path,
    protocol: Option<&str>,
    native_suffix: &str,
) -> Result<(PathBuf, ModuleKind), String> {
    let (level, name) = parse_relative_spec(spec);

    // Walk up: level 1 = same dir, level 2 = parent, etc.
    let mut base = caller_dir.to_path_buf();
    for _ in 0..level.saturating_sub(1) {
        base = base.parent().unwrap_or(&base).to_path_buf();
    }

    let file_name = name.replace('.', "/");
    let exts = extensions_for_protocol(protocol);

    // Check packages first
    if !name.contains('.') {
        let pkg_dir = base.join(name);
        if pkg_dir.is_dir() && pkg_dir.join("lib.toml").is_file() {
            return Ok((pkg_dir, ModuleKind::Package));
        }
    }

    // Search for files
    for &(ext, kind) in &exts {
        let candidate = base.join(format!("{}{}", file_name, ext));
        if candidate.is_file() {
            return Ok((candidate, kind));
        }
    }

    // Native extensions
    if protocol != Some("cat") && !native_suffix.is_empty() {
        for prefix in ["", "lib", "libcatnip_"] {
            let candidate = base.join(format!("{}{}{}", prefix, file_name, native_suffix));
            if candidate.is_file() {
                return Ok((candidate, ModuleKind::Native));
            }
        }
    }

    Err(format!(
        "relative import '{}' not found\n  looked in: {}",
        spec,
        base.display()
    ))
}

/// Look up a stdlib module entry. Returns (rust_import_name, needs_configure) or None.
pub fn lookup_stdlib(name: &str) -> Option<(&'static str, bool)> {
    STDLIB_MODULES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, import_name, needs_configure)| (*import_name, *needs_configure))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_spec_valid() {
        assert!(validate_spec("math").is_ok());
        assert!(validate_spec("os.path").is_ok());
        assert!(validate_spec(".utils").is_ok());
        assert!(validate_spec("..config").is_ok());
    }

    #[test]
    fn test_validate_spec_path_rejected() {
        assert!(validate_spec("./foo").is_err());
        assert!(validate_spec("../bar").is_err());
        assert!(validate_spec("/abs/path").is_err());
    }

    #[test]
    fn test_validate_spec_extension_rejected() {
        assert!(validate_spec("foo.py").is_err());
        assert!(validate_spec("bar.cat").is_err());
        assert!(validate_spec("baz.so").is_err());
    }

    #[test]
    fn test_validate_spec_dots_only_rejected() {
        assert!(validate_spec("..").is_err());
        assert!(validate_spec(".").is_err());
    }

    #[test]
    fn test_parse_relative_spec() {
        assert_eq!(parse_relative_spec("math"), (0, "math"));
        assert_eq!(parse_relative_spec(".utils"), (1, "utils"));
        assert_eq!(parse_relative_spec("..config"), (2, "config"));
        assert_eq!(parse_relative_spec("...deep"), (3, "deep"));
    }

    #[test]
    fn test_lookup_stdlib() {
        assert_eq!(lookup_stdlib("io"), Some(("catnip_io", false)));
        assert_eq!(lookup_stdlib("sys"), Some(("catnip_sys", true)));
        assert_eq!(lookup_stdlib("math"), None);
    }
}
