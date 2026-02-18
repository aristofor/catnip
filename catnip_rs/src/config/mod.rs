// FILE: catnip_rs/src/config/mod.rs
//! Configuration management with source tracking.
//!
//! This module provides:
//! - ConfigSource enum for tracking where config values come from
//! - ConfigValue struct with source tracking
//! - ConfigManager for unified config handling with precedence
//! - XDG directory helpers for cross-platform config/cache/data paths

use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3::IntoPyObjectExt;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

const CONFIG_FILE: &str = "catnip.toml";

/// Helper to convert Rust values to Py<PyAny>.
fn to_py_any<'py, T>(py: Python<'py>, value: T) -> Py<PyAny>
where
    T: IntoPyObject<'py>,
{
    value.into_bound_py_any(py).unwrap().unbind()
}

/// Source of a configuration value.
#[pyclass(name = "ConfigSource", from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    #[pyo3(name = "DEFAULT")]
    Default,
    #[pyo3(name = "FILE")]
    File,
    #[pyo3(name = "ENV")]
    Env,
    #[pyo3(name = "CLI")]
    Cli,
}

#[pymethods]
impl ConfigSource {
    fn __repr__(&self) -> String {
        match self {
            ConfigSource::Default => "ConfigSource.DEFAULT".to_string(),
            ConfigSource::File => "ConfigSource.FILE".to_string(),
            ConfigSource::Env => "ConfigSource.ENV".to_string(),
            ConfigSource::Cli => "ConfigSource.CLI".to_string(),
        }
    }

    fn __str__(&self) -> &'static str {
        match self {
            ConfigSource::Default => "default",
            ConfigSource::File => "file",
            ConfigSource::Env => "env",
            ConfigSource::Cli => "cli",
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self == other
    }

    fn __hash__(&self) -> u64 {
        *self as u64
    }

    #[getter]
    fn value(&self) -> &'static str {
        self.__str__()
    }
}

/// Configuration value with source tracking.
#[pyclass(name = "ConfigValue")]
pub struct ConfigValue {
    #[pyo3(get, set)]
    pub value: Py<PyAny>,
    #[pyo3(get)]
    pub source: ConfigSource,
    #[pyo3(get, set)]
    pub source_detail: Option<String>,
}

impl ConfigValue {
    fn clone_with_py(&self, py: Python<'_>) -> Self {
        Self {
            value: self.value.clone_ref(py),
            source: self.source,
            source_detail: self.source_detail.clone(),
        }
    }
}

#[pymethods]
impl ConfigValue {
    #[new]
    fn new(value: Py<PyAny>, source: ConfigSource, source_detail: Option<String>) -> Self {
        Self {
            value,
            source,
            source_detail,
        }
    }

    fn __repr__(&self) -> String {
        let detail = self
            .source_detail
            .as_ref()
            .map(|s| format!(" ({})", s))
            .unwrap_or_default();
        format!("ConfigValue(source={:?}{})", self.source, detail)
    }
}

/// Configuration manager with source tracking and precedence.
///
/// Precedence (lowest to highest):
/// 1. DEFAULT_CONFIG (hardcoded)
/// 2. catnip.toml file
/// 3. Environment variables (CATNIP_CACHE, CATNIP_OPTIMIZE, CATNIP_EXECUTOR, NO_COLOR)
/// 4. CLI options (-o, -x, --no-color)
#[pyclass(name = "ConfigManager")]
pub struct ConfigManager {
    values: HashMap<String, ConfigValue>,
    format_values: HashMap<String, ConfigValue>,
}

#[pymethods]
impl ConfigManager {
    #[new]
    fn new(py: Python<'_>) -> Self {
        let mut manager = Self {
            values: HashMap::new(),
            format_values: HashMap::new(),
        };
        manager.load_defaults(py);
        manager
    }

    /// Get configuration value.
    fn get(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
        self.values
            .get(key)
            .map(|cv| cv.value.clone_ref(py))
            .ok_or_else(|| {
                pyo3::exceptions::PyKeyError::new_err(format!("Unknown config key: {}", key))
            })
    }

    /// Get configuration value with source info.
    fn get_with_source(&self, py: Python<'_>, key: &str) -> PyResult<Py<ConfigValue>> {
        self.values
            .get(key)
            .map(|cv| Py::new(py, cv.clone_with_py(py)))
            .transpose()?
            .ok_or_else(|| {
                pyo3::exceptions::PyKeyError::new_err(format!("Unknown config key: {}", key))
            })
    }

    /// Get all configuration values (without source info).
    fn items(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let dict = PyDict::new(py);
        for (key, cv) in &self.values {
            dict.set_item(key, cv.value.clone_ref(py))?;
        }
        Ok(dict.into())
    }

    /// Load catnip.toml, overriding defaults.
    #[pyo3(signature = (path=None, mode=None))]
    fn load_file(&mut self, py: Python<'_>, path: Option<PathBuf>, mode: Option<&str>) {
        let config_path = path.unwrap_or_else(get_config_path);
        if !config_path.exists() {
            return;
        }

        let content = match fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let data: toml::Table = match toml::from_str(&content) {
            Ok(d) => d,
            Err(_) => return,
        };

        let path_str = config_path.to_string_lossy().to_string();

        // Load root-level keys (backward compatibility)
        for (key, value) in &data {
            if is_valid_key(key) && !value.is_table() {
                if let Some(py_value) = toml_to_python(py, value) {
                    self.values.insert(
                        key.clone(),
                        ConfigValue {
                            value: py_value,
                            source: ConfigSource::File,
                            source_detail: Some(path_str.clone()),
                        },
                    );
                }
            } else if is_valid_format_key(key) && !value.is_table() {
                if let Some(py_value) = toml_to_python(py, value) {
                    self.format_values.insert(
                        key.clone(),
                        ConfigValue {
                            value: py_value,
                            source: ConfigSource::File,
                            source_detail: Some(path_str.clone()),
                        },
                    );
                }
            }
        }

        // Load [repl] section
        if let Some(repl) = data.get("repl").and_then(|v| v.as_table()) {
            for (key, value) in repl {
                if is_valid_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            key.clone(),
                            ConfigValue {
                                value: py_value,
                                source: ConfigSource::File,
                                source_detail: Some(path_str.clone()),
                            },
                        );
                    }
                }
            }
        }

        // Load [optimize] section
        if let Some(optimize) = data.get("optimize").and_then(|v| v.as_table()) {
            for (key, value) in optimize {
                if is_valid_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            key.clone(),
                            ConfigValue {
                                value: py_value,
                                source: ConfigSource::File,
                                source_detail: Some(path_str.clone()),
                            },
                        );
                    }
                }
            }
        }

        // Load [cache] section
        if let Some(cache) = data.get("cache").and_then(|v| v.as_table()) {
            for (key, value) in cache {
                if is_valid_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            key.clone(),
                            ConfigValue {
                                value: py_value,
                                source: ConfigSource::File,
                                source_detail: Some(path_str.clone()),
                            },
                        );
                    }
                }
            }
        }

        // Load [format] section
        if let Some(format) = data.get("format").and_then(|v| v.as_table()) {
            for (key, value) in format {
                if is_valid_format_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.format_values.insert(
                            key.clone(),
                            ConfigValue {
                                value: py_value,
                                source: ConfigSource::File,
                                source_detail: Some(path_str.clone()),
                            },
                        );
                    }
                }
            }
        }

        // Load mode-specific overrides [mode.{mode}]
        if let Some(mode_name) = mode {
            if let Some(mode_table) = data
                .get("mode")
                .and_then(|v| v.as_table())
                .and_then(|t| t.get(mode_name))
                .and_then(|v| v.as_table())
            {
                let mode_detail = format!("{} [mode.{}]", path_str, mode_name);
                for (key, value) in mode_table {
                    if is_valid_key(key) {
                        if let Some(py_value) = toml_to_python(py, value) {
                            self.values.insert(
                                key.clone(),
                                ConfigValue {
                                    value: py_value,
                                    source: ConfigSource::File,
                                    source_detail: Some(mode_detail.clone()),
                                },
                            );
                        }
                    } else if is_valid_format_key(key) {
                        if let Some(py_value) = toml_to_python(py, value) {
                            self.format_values.insert(
                                key.clone(),
                                ConfigValue {
                                    value: py_value,
                                    source: ConfigSource::File,
                                    source_detail: Some(mode_detail.clone()),
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Load mode-specific overrides from [mode.{mode}] section.
    #[pyo3(signature = (mode, path=None))]
    fn load_mode_overrides(&mut self, py: Python<'_>, mode: &str, path: Option<PathBuf>) {
        let config_path = path.unwrap_or_else(get_config_path);
        if !config_path.exists() {
            return;
        }

        let content = match fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let data: toml::Table = match toml::from_str(&content) {
            Ok(d) => d,
            Err(_) => return,
        };

        let path_str = config_path.to_string_lossy().to_string();

        if let Some(mode_table) = data
            .get("mode")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get(mode))
            .and_then(|v| v.as_table())
        {
            let mode_detail = format!("{} [mode.{}]", path_str, mode);
            for (key, value) in mode_table {
                if is_valid_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            key.clone(),
                            ConfigValue {
                                value: py_value,
                                source: ConfigSource::File,
                                source_detail: Some(mode_detail.clone()),
                            },
                        );
                    }
                } else if is_valid_format_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.format_values.insert(
                            key.clone(),
                            ConfigValue {
                                value: py_value,
                                source: ConfigSource::File,
                                source_detail: Some(mode_detail.clone()),
                            },
                        );
                    }
                }
            }
        }
    }

    /// Load environment variables, overriding file config.
    fn load_env(&mut self, py: Python<'_>) {
        // NO_COLOR (freedesktop.org standard)
        if env::var("NO_COLOR").is_ok() {
            self.values.insert(
                "no_color".to_string(),
                ConfigValue {
                    value: to_py_any(py, true),
                    source: ConfigSource::Env,
                    source_detail: Some("NO_COLOR".to_string()),
                },
            );
        }

        // CATNIP_THEME
        if let Ok(theme) = env::var("CATNIP_THEME") {
            let val = theme.to_lowercase();
            if matches!(val.as_str(), "auto" | "dark" | "light") {
                self.values.insert(
                    "theme".to_string(),
                    ConfigValue {
                        value: to_py_any(py, val),
                        source: ConfigSource::Env,
                        source_detail: Some("CATNIP_THEME".to_string()),
                    },
                );
            }
        }

        // CATNIP_EXECUTOR
        if let Ok(executor) = env::var("CATNIP_EXECUTOR") {
            self.values.insert(
                "executor".to_string(),
                ConfigValue {
                    value: to_py_any(py, executor.to_lowercase()),
                    source: ConfigSource::Env,
                    source_detail: Some("CATNIP_EXECUTOR".to_string()),
                },
            );
        }

        // CATNIP_CACHE - disable disk cache (off/false/0/no)
        if let Ok(val) = env::var("CATNIP_CACHE") {
            let enabled = !matches!(val.to_lowercase().as_str(), "off" | "false" | "0" | "no");
            self.values.insert(
                "enable_cache".to_string(),
                ConfigValue {
                    value: to_py_any(py, enabled),
                    source: ConfigSource::Env,
                    source_detail: Some("CATNIP_CACHE".to_string()),
                },
            );
        }

        // CATNIP_OPTIMIZE - same syntax as -o, comma-separated
        if let Ok(opts) = env::var("CATNIP_OPTIMIZE") {
            for opt in opts.split(',') {
                let opt = opt.trim();
                if !opt.is_empty() {
                    self.apply_optimization(py, opt, ConfigSource::Env, "CATNIP_OPTIMIZE");
                }
            }
        }

        // CATNIP_FORMAT_INDENT_SIZE
        if let Ok(indent_size) = env::var("CATNIP_FORMAT_INDENT_SIZE") {
            if let Ok(size) = indent_size.parse::<i64>() {
                self.format_values.insert(
                    "indent_size".to_string(),
                    ConfigValue {
                        value: to_py_any(py, size),
                        source: ConfigSource::Env,
                        source_detail: Some("CATNIP_FORMAT_INDENT_SIZE".to_string()),
                    },
                );
            }
        }

        // CATNIP_FORMAT_LINE_LENGTH
        if let Ok(line_length) = env::var("CATNIP_FORMAT_LINE_LENGTH") {
            if let Ok(length) = line_length.parse::<i64>() {
                self.format_values.insert(
                    "line_length".to_string(),
                    ConfigValue {
                        value: to_py_any(py, length),
                        source: ConfigSource::Env,
                        source_detail: Some("CATNIP_FORMAT_LINE_LENGTH".to_string()),
                    },
                );
            }
        }
    }

    /// Apply -x/--executor CLI option.
    fn apply_cli_executor(&mut self, py: Python<'_>, executor: &str) {
        self.values.insert(
            "executor".to_string(),
            ConfigValue {
                value: to_py_any(py, executor.to_lowercase()),
                source: ConfigSource::Cli,
                source_detail: Some("-x".to_string()),
            },
        );
    }

    /// Apply --theme CLI option.
    fn apply_cli_theme(&mut self, py: Python<'_>, theme: &str) {
        self.values.insert(
            "theme".to_string(),
            ConfigValue {
                value: to_py_any(py, theme.to_lowercase()),
                source: ConfigSource::Cli,
                source_detail: Some("--theme".to_string()),
            },
        );
    }

    /// Apply --no-color CLI flag.
    fn apply_cli_no_color(&mut self, py: Python<'_>) {
        self.values.insert(
            "no_color".to_string(),
            ConfigValue {
                value: to_py_any(py, true),
                source: ConfigSource::Cli,
                source_detail: Some("--no-color".to_string()),
            },
        );
    }

    /// Apply --no-cache CLI flag.
    fn apply_cli_no_cache(&mut self, py: Python<'_>) {
        self.values.insert(
            "enable_cache".to_string(),
            ConfigValue {
                value: to_py_any(py, false),
                source: ConfigSource::Cli,
                source_detail: Some("--no-cache".to_string()),
            },
        );
    }

    /// Apply -o CLI options.
    fn apply_cli_optimizations(&mut self, py: Python<'_>, optimizations: Vec<String>) {
        for opt in &optimizations {
            let detail = format!("-o {}", opt);
            self.apply_optimization(py, opt, ConfigSource::Cli, &detail);
        }
    }

    /// Apply --indent-size CLI option.
    fn apply_cli_format_indent_size(&mut self, py: Python<'_>, size: i64) {
        self.format_values.insert(
            "indent_size".to_string(),
            ConfigValue {
                value: to_py_any(py, size),
                source: ConfigSource::Cli,
                source_detail: Some("--indent-size".to_string()),
            },
        );
    }

    /// Apply --line-length CLI option.
    fn apply_cli_format_line_length(&mut self, py: Python<'_>, length: i64) {
        self.format_values.insert(
            "line_length".to_string(),
            ConfigValue {
                value: to_py_any(py, length),
                source: ConfigSource::Cli,
                source_detail: Some("--line-length".to_string()),
            },
        );
    }

    /// Get format configuration as FormatConfig instance.
    fn get_format_config(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let format_config_class = py.import("catnip._rs")?.getattr("FormatConfig")?;

        let indent_size = self
            .format_values
            .get("indent_size")
            .map(|cv| cv.value.clone_ref(py))
            .unwrap_or_else(|| to_py_any(py, 4i64));

        let line_length = self
            .format_values
            .get("line_length")
            .map(|cv| cv.value.clone_ref(py))
            .unwrap_or_else(|| to_py_any(py, 120i64));

        let result = format_config_class.call1((indent_size, line_length))?;
        Ok(result.into())
    }

    /// Generate debug report showing sources.
    fn debug_report(&self, py: Python<'_>) -> Vec<String> {
        let mut lines = Vec::new();

        let mut keys: Vec<_> = self.values.keys().collect();
        keys.sort();

        for key in keys {
            let cv = &self.values[key];
            let source = cv.source.__str__();
            let detail = cv
                .source_detail
                .as_ref()
                .map(|s| format!(" ({})", s))
                .unwrap_or_default();
            let value_repr = cv
                .value
                .bind(py)
                .repr()
                .map(|r| r.to_string())
                .unwrap_or_else(|_| "?".to_string());
            lines.push(format!("{}: {}  [{}{}]", key, value_repr, source, detail));
        }

        lines.push("--- format config ---".to_string());

        let mut format_keys: Vec<_> = self.format_values.keys().collect();
        format_keys.sort();

        for key in format_keys {
            let cv = &self.format_values[key];
            let source = cv.source.__str__();
            let detail = cv
                .source_detail
                .as_ref()
                .map(|s| format!(" ({})", s))
                .unwrap_or_default();
            let value_repr = cv
                .value
                .bind(py)
                .repr()
                .map(|r| r.to_string())
                .unwrap_or_else(|_| "?".to_string());
            lines.push(format!(
                "format.{}: {}  [{}{}]",
                key, value_repr, source, detail
            ));
        }

        lines
    }
}

impl ConfigManager {
    fn load_defaults(&mut self, py: Python<'_>) {
        // [repl] section
        self.values.insert(
            "no_color".to_string(),
            ConfigValue {
                value: to_py_any(py, false),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );

        // [cache] section
        self.values.insert(
            "enable_cache".to_string(),
            ConfigValue {
                value: to_py_any(py, true),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            "cache_max_size_mb".to_string(),
            ConfigValue {
                value: to_py_any(py, 100i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            "cache_ttl_seconds".to_string(),
            ConfigValue {
                value: to_py_any(py, 86400i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );

        // [optimize] section
        self.values.insert(
            "jit".to_string(),
            ConfigValue {
                value: to_py_any(py, false),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            "tco".to_string(),
            ConfigValue {
                value: to_py_any(py, true),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            "optimize".to_string(),
            ConfigValue {
                value: to_py_any(py, 3i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            "executor".to_string(),
            ConfigValue {
                value: to_py_any(py, "vm".to_string()),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            "theme".to_string(),
            ConfigValue {
                value: to_py_any(py, "auto".to_string()),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );

        // [format] section
        self.format_values.insert(
            "indent_size".to_string(),
            ConfigValue {
                value: to_py_any(py, 4i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.format_values.insert(
            "line_length".to_string(),
            ConfigValue {
                value: to_py_any(py, 120i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
    }

    fn apply_optimization(
        &mut self,
        py: Python<'_>,
        opt: &str,
        source: ConfigSource,
        detail: &str,
    ) {
        let opt_lower = opt.to_lowercase();

        if opt_lower.starts_with("tco") {
            let value = if let Some((_, val)) = opt.split_once(':') {
                matches!(val.to_lowercase().as_str(), "on" | "true" | "1" | "yes")
            } else {
                true // 'tco' alone means enable
            };
            self.values.insert(
                "tco".to_string(),
                ConfigValue {
                    value: to_py_any(py, value),
                    source,
                    source_detail: Some(detail.to_string()),
                },
            );
        } else if opt_lower.starts_with("jit") {
            let value = if let Some((_, val)) = opt.split_once(':') {
                matches!(val.to_lowercase().as_str(), "on" | "true" | "1" | "yes")
            } else {
                true // 'jit' alone means enable
            };
            self.values.insert(
                "jit".to_string(),
                ConfigValue {
                    value: to_py_any(py, value),
                    source,
                    source_detail: Some(detail.to_string()),
                },
            );
        } else if opt_lower.starts_with("level") {
            if let Some((_, level_str)) = opt.split_once(':') {
                let level_str = level_str.to_lowercase();
                let level = match level_str.as_str() {
                    "none" | "off" => Some(0),
                    "low" => Some(1),
                    "medium" => Some(2),
                    "high" | "full" | "aggressive" => Some(3),
                    _ => level_str.parse::<i64>().ok(),
                };

                if let Some(lvl) = level {
                    if (0..=3).contains(&lvl) {
                        self.values.insert(
                            "optimize".to_string(),
                            ConfigValue {
                                value: to_py_any(py, lvl),
                                source,
                                source_detail: Some(detail.to_string()),
                            },
                        );
                    }
                }
            }
        }
    }
}

// --- Helper functions ---

fn is_valid_key(key: &str) -> bool {
    matches!(
        key,
        "no_color"
            | "jit"
            | "tco"
            | "optimize"
            | "executor"
            | "theme"
            | "enable_cache"
            | "cache_max_size_mb"
            | "cache_ttl_seconds"
    )
}

fn is_valid_format_key(key: &str) -> bool {
    matches!(key, "indent_size" | "line_length")
}

fn toml_to_python(py: Python<'_>, value: &toml::Value) -> Option<Py<PyAny>> {
    match value {
        toml::Value::String(s) => Some(to_py_any(py, s.clone())),
        toml::Value::Integer(i) => Some(to_py_any(py, *i)),
        toml::Value::Float(f) => Some(to_py_any(py, *f)),
        toml::Value::Boolean(b) => Some(to_py_any(py, *b)),
        _ => None,
    }
}

// --- XDG directory helpers ---

/// Get home directory, respecting $HOME env var (for tests).
fn get_home_dir() -> PathBuf {
    if let Ok(home) = env::var("HOME") {
        PathBuf::from(home)
    } else {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Return the Catnip config directory following XDG conventions.
#[pyfunction]
pub fn get_config_dir() -> PathBuf {
    if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg_config_home).join("catnip")
    } else {
        get_home_dir().join(".config").join("catnip")
    }
}

/// Return the Catnip state directory following XDG conventions.
#[pyfunction]
pub fn get_state_dir() -> PathBuf {
    if let Ok(xdg_state_home) = env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg_state_home).join("catnip")
    } else {
        get_home_dir().join(".local").join("state").join("catnip")
    }
}

/// Return the Catnip cache directory following XDG conventions.
#[pyfunction]
pub fn get_cache_dir() -> PathBuf {
    if let Ok(xdg_cache_home) = env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg_cache_home).join("catnip")
    } else {
        get_home_dir().join(".cache").join("catnip")
    }
}

/// Return the Catnip data directory following XDG conventions.
#[pyfunction]
pub fn get_data_dir() -> PathBuf {
    if let Ok(xdg_data_home) = env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg_data_home).join("catnip")
    } else {
        get_home_dir().join(".local").join("share").join("catnip")
    }
}

/// Return the path to the config file.
#[pyfunction]
pub fn get_config_path() -> PathBuf {
    get_config_dir().join(CONFIG_FILE)
}

/// Register config module functions and classes.
pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ConfigSource>()?;
    m.add_class::<ConfigValue>()?;
    m.add_class::<ConfigManager>()?;
    m.add_function(wrap_pyfunction!(get_config_dir, m)?)?;
    m.add_function(wrap_pyfunction!(get_state_dir, m)?)?;
    m.add_function(wrap_pyfunction!(get_cache_dir, m)?)?;
    m.add_function(wrap_pyfunction!(get_data_dir, m)?)?;
    m.add_function(wrap_pyfunction!(get_config_path, m)?)?;
    Ok(())
}
