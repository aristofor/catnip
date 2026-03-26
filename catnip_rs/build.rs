// FILE: catnip_rs/build.rs
use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

fn main() {
    println!("cargo:rerun-if-changed=visual.toml");

    // Grammar is compiled by catnip_tools (linked via cargo dependency)

    // Generate theme constants from visual.toml
    generate_theme();

    // Link against Python when embedded feature is enabled
    // NOTE: embedded requires --no-default-features to avoid extension-module
    if std::env::var("CARGO_FEATURE_EMBEDDED").is_ok() {
        link_python();
    }
}

// ============================================================================
// Theme generation from visual.toml
// ============================================================================

#[derive(Deserialize)]
#[allow(dead_code)]
struct VisualToml {
    prompts: Prompts,
    ui: BTreeMap<String, RawColor>,
    accent: Accent,
    base: Base,
    stages: BTreeMap<String, RawColor>,
}

#[derive(Deserialize)]
struct Prompts {
    main: String,
    continuation: String,
}

#[derive(Deserialize)]
struct Accent {
    dark: BTreeMap<String, RawColor>,
    light: BTreeMap<String, RawColor>,
}

#[derive(Deserialize)]
struct Base {
    dark: BTreeMap<String, RawColor>,
    light: BTreeMap<String, RawColor>,
}

/// Accepte "oklch(L C H)" ou { color = "oklch(L C H)", bold = true }
#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum RawColor {
    Css(String),
    Table {
        color: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        italic: bool,
    },
}

#[derive(Clone)]
#[allow(dead_code)]
struct ColorEntry {
    l: f64,
    c: f64,
    h: f64,
    bold: bool,
    italic: bool,
}

fn parse_oklch(s: &str) -> (f64, f64, f64) {
    let inner = s
        .trim()
        .strip_prefix("oklch(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or_else(|| panic!("Invalid oklch format: {s}"));
    let parts: Vec<f64> = inner
        .split_whitespace()
        .map(|p| p.parse().unwrap_or_else(|_| panic!("Invalid number in oklch: {p}")))
        .collect();
    assert!(parts.len() == 3, "oklch needs 3 values, got {}: {s}", parts.len());
    (parts[0], parts[1], parts[2])
}

impl RawColor {
    fn resolve(&self) -> ColorEntry {
        match self {
            RawColor::Css(s) => {
                let (l, c, h) = parse_oklch(s);
                ColorEntry {
                    l,
                    c,
                    h,
                    bold: false,
                    italic: false,
                }
            }
            RawColor::Table { color, bold, italic } => {
                let (l, c, h) = parse_oklch(color);
                ColorEntry {
                    l,
                    c,
                    h,
                    bold: *bold,
                    italic: *italic,
                }
            }
        }
    }
}

fn resolve_map(raw: &BTreeMap<String, RawColor>) -> BTreeMap<String, ColorEntry> {
    raw.iter().map(|(k, v)| (k.clone(), v.resolve())).collect()
}

/// OKLCH -> sRGB (0-255). Same math as gen_theme.py.
fn oklch_to_srgb(l: f64, c: f64, h: f64) -> (u8, u8, u8) {
    let h_rad = h * std::f64::consts::PI / 180.0;
    let a = c * h_rad.cos();
    let b = c * h_rad.sin();

    // OKLAB -> LMS
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;

    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;

    // LMS -> linear sRGB
    let r_lin = 4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3;
    let g_lin = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3;
    let b_lin = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3;

    fn gamma(x: f64) -> u8 {
        let v = if x <= 0.0031308 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    }

    (gamma(r_lin), gamma(g_lin), gamma(b_lin))
}

fn hex_u32(entry: &ColorEntry) -> u32 {
    let (r, g, b) = oklch_to_srgb(entry.l, entry.c, entry.h);
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn generate_theme() {
    let toml_str = fs::read_to_string("visual.toml").expect("Failed to read visual.toml");

    let data: VisualToml = toml::from_str(&toml_str).expect("Failed to parse visual.toml");

    let ui = resolve_map(&data.ui);
    let accent_dark = resolve_map(&data.accent.dark);
    let accent_light = resolve_map(&data.accent.light);
    let base_dark = resolve_map(&data.base.dark);
    let base_light = resolve_map(&data.base.light);

    let mut out = String::new();

    writeln!(out, "// GENERATED FROM visual.toml - do not edit").unwrap();
    writeln!(out, "// Run: make compile (build.rs regenerates automatically)").unwrap();
    writeln!(out).unwrap();

    // Prompts
    writeln!(out, "/// Prompt principal (ligne normale)").unwrap();
    writeln!(out, "pub const REPL_PROMPT_MAIN: &str = {:?};", data.prompts.main).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// Prompt de continuation (multiline)").unwrap();
    writeln!(
        out,
        "pub const REPL_PROMPT_CONTINUATION: &str = {:?};",
        data.prompts.continuation
    )
    .unwrap();
    writeln!(out).unwrap();

    // UI colors (ANSI escape sequences)
    writeln!(out, "pub mod colors {{").unwrap();
    for (name, entry) in &ui {
        let (r, g, b) = oklch_to_srgb(entry.l, entry.c, entry.h);
        let const_name = name.to_uppercase();
        writeln!(
            out,
            "    /// {} (OKLCH {:.2} {:.2} {})",
            name, entry.l, entry.c, entry.h
        )
        .unwrap();
        writeln!(
            out,
            "    pub const {}: &str = \"\\x1b[38;2;{};{};{}m\";",
            const_name, r, g, b
        )
        .unwrap();
    }
    writeln!(out, "    /// Reset").unwrap();
    writeln!(out, "    pub const RESET: &str = \"\\x1b[0m\";").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Highlighting module (accent + base dark) - raw u32 hex values
    writeln!(out, "pub mod highlighting {{").unwrap();

    // Accent colors (dark theme)
    let accent_map = [
        ("keyword", "KEYWORD"),
        ("constant", "CONSTANT"),
        ("type", "TYPE"),
        ("number", "NUMBER"),
        ("string", "STRING"),
        ("builtin", "BUILTIN"),
    ];
    for (toml_key, rust_prefix) in &accent_map {
        if let Some(entry) = accent_dark.get(*toml_key) {
            let hex = hex_u32(entry);
            writeln!(out, "    pub const {}_COLOR: u32 = 0x{:06X};", rust_prefix, hex).unwrap();
            writeln!(out, "    pub const {}_BOLD: bool = {};", rust_prefix, entry.bold).unwrap();
        }
    }
    writeln!(out).unwrap();

    // Dark base colors for highlighting
    let base_dark_map = [
        ("comment", "COMMENT"),
        ("operator", "OPERATOR"),
        ("punctuation", "PUNCTUATION"),
    ];
    for (toml_key, rust_name) in &base_dark_map {
        if let Some(entry) = base_dark.get(*toml_key) {
            let hex = hex_u32(entry);
            writeln!(out, "    pub const {}_COLOR: u32 = 0x{:06X};", rust_name, hex).unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // highlighting_light module (accent + base light) - raw u32 hex values
    writeln!(out, "pub mod highlighting_light {{").unwrap();
    for (toml_key, rust_prefix) in &accent_map {
        if let Some(entry) = accent_light.get(*toml_key) {
            let hex = hex_u32(entry);
            writeln!(out, "    pub const {}_COLOR: u32 = 0x{:06X};", rust_prefix, hex).unwrap();
            writeln!(out, "    pub const {}_BOLD: bool = {};", rust_prefix, entry.bold).unwrap();
        }
    }
    writeln!(out).unwrap();
    for (toml_key, rust_name) in &base_dark_map {
        if let Some(entry) = base_light.get(*toml_key) {
            let hex = hex_u32(entry);
            writeln!(out, "    pub const {}_COLOR: u32 = 0x{:06X};", rust_name, hex).unwrap();
        }
    }
    // Light background/foreground
    if let Some(entry) = base_light.get("background") {
        let hex = hex_u32(entry);
        writeln!(out, "    pub const BACKGROUND_COLOR: u32 = 0x{:06X};", hex).unwrap();
    }
    if let Some(entry) = base_light.get("foreground") {
        let hex = hex_u32(entry);
        writeln!(out, "    pub const FOREGROUND_COLOR: u32 = 0x{:06X};", hex).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // UI colors as raw u32 hex values (for REPL config)
    writeln!(out, "pub mod ui_colors {{").unwrap();
    for (name, entry) in &ui {
        let hex = hex_u32(entry);
        let const_name = name.to_uppercase();
        writeln!(out, "    pub const {}: u32 = 0x{:06X};", const_name, hex).unwrap();
    }
    writeln!(out, "}}").unwrap();

    // Write to OUT_DIR
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_path = PathBuf::from(out_dir).join("theme_generated.rs");
    fs::write(&out_path, out).expect("Failed to write theme_generated.rs");
}

// ============================================================================
// Python linking
// ============================================================================

fn link_python() {
    // Try different python-config binary names
    let config_names = ["python3-config", "x86_64-linux-gnu-python3-config"];

    let mut output = None;
    for name in &config_names {
        if let Ok(result) = Command::new(name).args(["--embed", "--ldflags"]).output() {
            if result.status.success() {
                output = Some(result);
                break;
            }
        }
    }

    let output =
        output.expect("Failed to run python3-config. Install python3-dev package or ensure python3-config is in PATH");

    let ldflags = String::from_utf8_lossy(&output.stdout);

    // Parse ldflags and emit cargo instructions
    for flag in ldflags.split_whitespace() {
        if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={}", lib);
        } else if let Some(path) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={}", path);
        }
    }
}
