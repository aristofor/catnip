// FILE: catnip_rs/src/config/mod.rs
//! Configuration management with source tracking.
//!
//! This module provides:
//! - ConfigSource enum for tracking where config values come from
//! - ConfigValue struct with source tracking
//! - ConfigManager for unified config handling with precedence
//! - XDG directory helpers for cross-platform config/cache/data paths

use crate::constants::*;
use crate::policy::{self, ModulePolicy};
use catnip_core::paths::get_cache_dir as core_get_cache_dir;
use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::PyDict;
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
    values: HashMap<&'static str, ConfigValue>,
    format_values: HashMap<&'static str, ConfigValue>,
    module_policy: Option<ModulePolicy>,
    named_policies: HashMap<String, ModulePolicy>,
    auto_modules: Vec<String>,
    auto_modules_by_mode: HashMap<String, Vec<String>>,
}

#[pymethods]
impl ConfigManager {
    #[new]
    fn new(py: Python<'_>) -> Self {
        let mut manager = Self {
            values: HashMap::new(),
            format_values: HashMap::new(),
            module_policy: None,
            named_policies: HashMap::new(),
            auto_modules: vec![],
            auto_modules_by_mode: HashMap::from([
                ("cli".to_string(), vec!["io:!".to_string()]),
                ("repl".to_string(), vec!["io:!".to_string()]),
            ]),
        };
        manager.load_defaults(py);
        manager
    }

    /// Get configuration value.
    fn get(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
        self.values
            .get(key)
            .map(|cv| cv.value.clone_ref(py))
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(format!("Unknown config key: {}", key)))
    }

    /// Get configuration value with source info.
    fn get_with_source(&self, py: Python<'_>, key: &str) -> PyResult<Py<ConfigValue>> {
        self.values
            .get(key)
            .map(|cv| Py::new(py, cv.clone_with_py(py)))
            .transpose()?
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(format!("Unknown config key: {}", key)))
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
            if let Some(k) = str_to_config_key(key) {
                if !value.is_table() {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            k,
                            ConfigValue {
                                value: py_value,
                                source: ConfigSource::File,
                                source_detail: Some(path_str.clone()),
                            },
                        );
                    }
                }
            } else if let Some(k) = str_to_format_key(key) {
                if !value.is_table() {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.format_values.insert(
                            k,
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

        // Load [repl] section
        if let Some(repl) = data.get("repl").and_then(|v| v.as_table()) {
            for (key, value) in repl {
                if let Some(k) = str_to_config_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            k,
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
                if let Some(k) = str_to_config_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            k,
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
                if let Some(k) = str_to_config_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            k,
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

        // Load [diagnostics] section
        if let Some(diagnostics) = data.get("diagnostics").and_then(|v| v.as_table()) {
            for (key, value) in diagnostics {
                if let Some(k) = str_to_config_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            k,
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

        // Load [modules] section
        if let Some(modules) = data.get("modules").and_then(|v| v.as_table()) {
            // Only create a policy if actual policy keys are present
            let has_policy_keys =
                modules.contains_key("policy") || modules.contains_key("allow") || modules.contains_key("deny");
            if has_policy_keys {
                if let Ok(p) = policy::parse_profile(modules) {
                    self.module_policy = Some(p);
                }
            }
            if let Some(arr) = modules.get("auto").and_then(|v| v.as_array()) {
                self.auto_modules = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
            }
            // Per-mode auto-import: [modules.repl], [modules.cli], [modules.dsl]
            for mode in &["repl", "cli", "dsl"] {
                if let Some(sub) = modules.get(*mode).and_then(|v| v.as_table()) {
                    if let Some(arr) = sub.get("auto").and_then(|v| v.as_array()) {
                        let mods: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                        self.auto_modules_by_mode.insert(mode.to_string(), mods);
                    }
                }
            }
            // Named policies: [modules.policies.<name>]
            if let Some(policies) = modules.get("policies").and_then(|v| v.as_table()) {
                for (name, value) in policies {
                    if let Some(table) = value.as_table() {
                        if let Ok(p) = policy::parse_profile(table) {
                            self.named_policies.insert(name.clone(), p);
                        }
                    }
                }
            }
        }

        // Load [format] section
        if let Some(format) = data.get("format").and_then(|v| v.as_table()) {
            for (key, value) in format {
                if let Some(k) = str_to_format_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.format_values.insert(
                            k,
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
                    if let Some(k) = str_to_config_key(key) {
                        if let Some(py_value) = toml_to_python(py, value) {
                            self.values.insert(
                                k,
                                ConfigValue {
                                    value: py_value,
                                    source: ConfigSource::File,
                                    source_detail: Some(mode_detail.clone()),
                                },
                            );
                        }
                    } else if let Some(k) = str_to_format_key(key) {
                        if let Some(py_value) = toml_to_python(py, value) {
                            self.format_values.insert(
                                k,
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
                if let Some(k) = str_to_config_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.values.insert(
                            k,
                            ConfigValue {
                                value: py_value,
                                source: ConfigSource::File,
                                source_detail: Some(mode_detail.clone()),
                            },
                        );
                    }
                } else if let Some(k) = str_to_format_key(key) {
                    if let Some(py_value) = toml_to_python(py, value) {
                        self.format_values.insert(
                            k,
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
    fn load_env(&mut self, py: Python<'_>) -> PyResult<()> {
        // NO_COLOR (freedesktop.org standard)
        if env::var("NO_COLOR").is_ok() {
            self.values.insert(
                CFG_NO_COLOR,
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
                    CFG_THEME,
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
                CFG_EXECUTOR,
                ConfigValue {
                    value: to_py_any(py, executor.to_lowercase()),
                    source: ConfigSource::Env,
                    source_detail: Some("CATNIP_EXECUTOR".to_string()),
                },
            );
        }

        // CATNIP_CACHE - disable disk cache (off/false/0/no)
        if let Ok(val) = env::var("CATNIP_CACHE") {
            let enabled = parse_bool_value(&val.to_lowercase()).unwrap_or(true);
            self.values.insert(
                CFG_ENABLE_CACHE,
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
                    self.apply_optimization(py, opt, ConfigSource::Env, "CATNIP_OPTIMIZE")?;
                }
            }
        }

        // CATNIP_FORMAT_INDENT_SIZE
        if let Ok(indent_size) = env::var("CATNIP_FORMAT_INDENT_SIZE") {
            if let Ok(size) = indent_size.parse::<i64>() {
                self.format_values.insert(
                    CFG_FMT_INDENT_SIZE,
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
                    CFG_FMT_LINE_LENGTH,
                    ConfigValue {
                        value: to_py_any(py, length),
                        source: ConfigSource::Env,
                        source_detail: Some("CATNIP_FORMAT_LINE_LENGTH".to_string()),
                    },
                );
            }
        }

        Ok(())
    }

    /// Apply -x/--executor CLI option.
    fn apply_cli_executor(&mut self, py: Python<'_>, executor: &str) {
        self.values.insert(
            CFG_EXECUTOR,
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
            CFG_THEME,
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
            CFG_NO_COLOR,
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
            CFG_ENABLE_CACHE,
            ConfigValue {
                value: to_py_any(py, false),
                source: ConfigSource::Cli,
                source_detail: Some("--no-cache".to_string()),
            },
        );
    }

    /// Apply -o CLI options.
    fn apply_cli_optimizations(&mut self, py: Python<'_>, optimizations: Vec<String>) -> PyResult<()> {
        for opt in &optimizations {
            let detail = format!("-o {}", opt);
            self.apply_optimization(py, opt, ConfigSource::Cli, &detail)?;
        }
        Ok(())
    }

    /// Apply --indent-size CLI option.
    fn apply_cli_format_indent_size(&mut self, py: Python<'_>, size: i64) {
        self.format_values.insert(
            CFG_FMT_INDENT_SIZE,
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
            CFG_FMT_LINE_LENGTH,
            ConfigValue {
                value: to_py_any(py, length),
                source: ConfigSource::Cli,
                source_detail: Some("--line-length".to_string()),
            },
        );
    }

    /// Apply --align CLI flag.
    fn apply_cli_format_align(&mut self, py: Python<'_>, value: bool) {
        self.format_values.insert(
            CFG_FMT_ALIGN,
            ConfigValue {
                value: to_py_any(py, value),
                source: ConfigSource::Cli,
                source_detail: Some("--align".to_string()),
            },
        );
    }

    /// Get format configuration as FormatConfig instance.
    fn get_format_config(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let format_config_class = py.import(PY_MOD_RS)?.getattr("FormatConfig")?;

        let indent_size = self
            .format_values
            .get(CFG_FMT_INDENT_SIZE)
            .map(|cv| cv.value.clone_ref(py))
            .unwrap_or_else(|| to_py_any(py, 4i64));

        let line_length = self
            .format_values
            .get(CFG_FMT_LINE_LENGTH)
            .map(|cv| cv.value.clone_ref(py))
            .unwrap_or_else(|| to_py_any(py, 120i64));

        let align = self
            .format_values
            .get(CFG_FMT_ALIGN)
            .map(|cv| cv.value.clone_ref(py))
            .unwrap_or_else(|| to_py_any(py, true));

        let result = format_config_class.call1((indent_size, line_length, align))?;
        Ok(result.into())
    }

    /// Get module policy (if configured).
    /// If name is given, returns the named policy from [modules.policies.<name>].
    /// Otherwise returns the default policy from [modules].
    #[pyo3(signature = (name=None))]
    fn get_module_policy(&self, name: Option<&str>) -> Option<ModulePolicy> {
        if let Some(n) = name {
            return self.named_policies.get(n).cloned();
        }
        self.module_policy.clone()
    }

    /// List named policy profiles from [modules.policies.*].
    fn list_policy_profiles(&self) -> Vec<String> {
        let mut names: Vec<String> = self.named_policies.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get auto-import module list.
    /// If mode is given (repl/cli/dsl), returns [modules.<mode>].auto if defined,
    /// otherwise falls back to [modules].auto.
    #[pyo3(signature = (mode=None))]
    fn get_auto_modules(&self, mode: Option<&str>) -> Vec<String> {
        if let Some(m) = mode {
            if let Some(mods) = self.auto_modules_by_mode.get(m) {
                return mods.clone();
            }
        }
        self.auto_modules.clone()
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
            lines.push(format!("format.{}: {}  [{}{}]", key, value_repr, source, detail));
        }

        // Modules section
        if !self.auto_modules.is_empty() || self.module_policy.is_some() || !self.named_policies.is_empty() {
            lines.push("--- modules ---".to_string());
            if !self.auto_modules.is_empty() {
                lines.push(format!("auto: [{}]", self.auto_modules.join(", ")));
            }
            for (mode, mods) in &self.auto_modules_by_mode {
                if !mods.is_empty() {
                    lines.push(format!("{}.auto: [{}]", mode, mods.join(", ")));
                }
            }
            if let Some(ref mp) = self.module_policy {
                lines.push(format!("policy: {}", mp._summary()));
            }
            for (name, p) in &self.named_policies {
                lines.push(format!("policies.{}: {}", name, p._summary()));
            }
        }

        lines
    }
}

impl ConfigManager {
    fn load_defaults(&mut self, py: Python<'_>) {
        // [repl] section
        self.values.insert(
            CFG_NO_COLOR,
            ConfigValue {
                value: to_py_any(py, false),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );

        // [cache] section
        self.values.insert(
            CFG_ENABLE_CACHE,
            ConfigValue {
                value: to_py_any(py, true),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            CFG_CACHE_MAX_SIZE_MB,
            ConfigValue {
                value: to_py_any(py, CACHE_DISK_MAX_SIZE_MB_DEFAULT as i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            CFG_CACHE_TTL_SECONDS,
            ConfigValue {
                value: to_py_any(py, CACHE_DISK_TTL_DEFAULT as i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );

        // [optimize] section
        self.values.insert(
            CFG_JIT,
            ConfigValue {
                value: to_py_any(py, false),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            CFG_TCO,
            ConfigValue {
                value: to_py_any(py, TCO_ENABLED_DEFAULT),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            CFG_OPTIMIZE,
            ConfigValue {
                value: to_py_any(py, 3i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            CFG_EXECUTOR,
            ConfigValue {
                value: to_py_any(py, "vm".to_string()),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            CFG_THEME,
            ConfigValue {
                value: to_py_any(py, "auto".to_string()),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );

        // [diagnostics] section
        self.values.insert(
            CFG_LOG_WEIRD_ERRORS,
            ConfigValue {
                value: to_py_any(py, true),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.values.insert(
            CFG_MAX_WEIRD_LOGS,
            ConfigValue {
                value: to_py_any(py, WEIRD_LOG_MAX_DEFAULT as i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );

        // [optimize] memory limit
        self.values.insert(
            CFG_MEMORY_LIMIT,
            ConfigValue {
                value: to_py_any(py, MEMORY_LIMIT_DEFAULT_MB as i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );

        // [format] section
        self.format_values.insert(
            CFG_FMT_INDENT_SIZE,
            ConfigValue {
                value: to_py_any(py, FORMAT_INDENT_SIZE_DEFAULT as i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.format_values.insert(
            CFG_FMT_LINE_LENGTH,
            ConfigValue {
                value: to_py_any(py, FORMAT_LINE_LENGTH_DEFAULT as i64),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
        self.format_values.insert(
            CFG_FMT_ALIGN,
            ConfigValue {
                value: to_py_any(py, FORMAT_ALIGN_DEFAULT),
                source: ConfigSource::Default,
                source_detail: None,
            },
        );
    }

    fn apply_optimization(&mut self, py: Python<'_>, opt: &str, source: ConfigSource, detail: &str) -> PyResult<()> {
        let opt_lower = opt.to_lowercase();

        if opt_lower.starts_with("tco") {
            let value = self.parse_bool_opt(opt, "tco")?;
            self.values.insert(
                CFG_TCO,
                ConfigValue {
                    value: to_py_any(py, value),
                    source,
                    source_detail: Some(detail.to_string()),
                },
            );
        } else if opt_lower.starts_with("jit") {
            let value = self.parse_bool_opt(opt, "jit")?;
            self.values.insert(
                CFG_JIT,
                ConfigValue {
                    value: to_py_any(py, value),
                    source,
                    source_detail: Some(detail.to_string()),
                },
            );
        } else if opt_lower.starts_with("memory") {
            let (_, val) = opt.split_once(':').ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("Option 'memory' requires a value: memory:<MB>")
            })?;
            let mb = val.parse::<i64>().map_err(|_| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "Invalid memory value '{}': expected integer (MB)",
                    val,
                ))
            })?;
            self.values.insert(
                CFG_MEMORY_LIMIT,
                ConfigValue {
                    value: to_py_any(py, mb),
                    source,
                    source_detail: Some(detail.to_string()),
                },
            );
        } else if opt_lower.starts_with("level") {
            let (_, level_str) = opt.split_once(':').ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(
                    "Option 'level' requires a value: level:<0-3|none|low|medium|high>",
                )
            })?;
            let level_lower = level_str.to_lowercase();
            let level = match level_lower.as_str() {
                "none" | "off" => 0,
                "low" => 1,
                "medium" => 2,
                "high" => 3,
                _ => {
                    let n = level_str.parse::<i64>().map_err(|_| {
                        pyo3::exceptions::PyValueError::new_err(format!(
                            "Invalid optimization level '{}': expected 0-3 or none/low/medium/high",
                            level_str,
                        ))
                    })?;
                    if !(0..=3).contains(&n) {
                        return Err(pyo3::exceptions::PyValueError::new_err(format!(
                            "Optimization level must be 0-3, got {}",
                            n,
                        )));
                    }
                    n
                }
            };
            self.values.insert(
                CFG_OPTIMIZE,
                ConfigValue {
                    value: to_py_any(py, level),
                    source,
                    source_detail: Some(detail.to_string()),
                },
            );
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Unknown optimization option '{}'. Valid options: tco, jit, level, memory",
                opt,
            )));
        }

        Ok(())
    }

    /// Parse a boolean optimization option (e.g. "tco", "tco:on", "tco:off").
    fn parse_bool_opt(&self, opt: &str, name: &str) -> PyResult<bool> {
        if let Some((_, val)) = opt.split_once(':') {
            parse_bool_value(&val.to_lowercase()).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "Invalid value '{}' for '{}': expected on/off/true/false/1/0/yes/no",
                    val, name,
                ))
            })
        } else {
            Ok(true) // name alone means enable
        }
    }
}

// --- Helper functions ---

fn str_to_config_key(key: &str) -> Option<&'static str> {
    match key {
        "no_color" => Some(CFG_NO_COLOR),
        "jit" => Some(CFG_JIT),
        "tco" => Some(CFG_TCO),
        "optimize" => Some(CFG_OPTIMIZE),
        "executor" => Some(CFG_EXECUTOR),
        "theme" => Some(CFG_THEME),
        "enable_cache" => Some(CFG_ENABLE_CACHE),
        "cache_max_size_mb" => Some(CFG_CACHE_MAX_SIZE_MB),
        "cache_ttl_seconds" => Some(CFG_CACHE_TTL_SECONDS),
        "log_weird_errors" => Some(CFG_LOG_WEIRD_ERRORS),
        "max_weird_logs" => Some(CFG_MAX_WEIRD_LOGS),
        "memory_limit" => Some(CFG_MEMORY_LIMIT),
        _ => None,
    }
}

fn str_to_format_key(key: &str) -> Option<&'static str> {
    match key {
        "indent_size" => Some(CFG_FMT_INDENT_SIZE),
        "line_length" => Some(CFG_FMT_LINE_LENGTH),
        "align" => Some(CFG_FMT_ALIGN),
        _ => None,
    }
}

fn toml_to_python(py: Python<'_>, value: &toml::Value) -> Option<Py<PyAny>> {
    match value {
        toml::Value::String(s) => {
            // Coerce "True"/"False" strings from legacy Python _write_toml
            match s.as_str() {
                "True" => Some(to_py_any(py, true)),
                "False" => Some(to_py_any(py, false)),
                _ => Some(to_py_any(py, s.clone())),
            }
        }
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
    core_get_cache_dir()
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
///
/// Precedence: `CATNIP_CONFIG` env var > `~/.config/catnip/catnip.toml`
#[pyfunction]
pub fn get_config_path() -> PathBuf {
    if let Ok(p) = env::var("CATNIP_CONFIG") {
        return PathBuf::from(p);
    }
    get_config_dir().join(CONFIG_FILE)
}

/// Map a flat config key to its TOML section.
fn key_to_section(key: &str) -> Option<&'static str> {
    match key {
        CFG_NO_COLOR | CFG_THEME => Some("repl"),
        CFG_JIT | CFG_TCO | CFG_OPTIMIZE | CFG_EXECUTOR | CFG_MEMORY_LIMIT => Some("optimize"),
        CFG_ENABLE_CACHE | CFG_CACHE_MAX_SIZE_MB | CFG_CACHE_TTL_SECONDS => Some("cache"),
        CFG_LOG_WEIRD_ERRORS | CFG_MAX_WEIRD_LOGS => Some("diagnostics"),
        CFG_FMT_INDENT_SIZE | CFG_FMT_LINE_LENGTH | CFG_FMT_ALIGN => Some("format"),
        _ => None,
    }
}

/// Set a single config value in the TOML file, preserving comments and formatting.
///
/// `key` can be a flat key ("jit") or prefixed ("format.indent_size").
/// `value` is a Python object (bool, int, str, or None for "unlimited").
#[pyfunction]
#[pyo3(signature = (key, value, path=None))]
fn set_config_value(_py: Python<'_>, key: &str, value: Bound<'_, PyAny>, path: Option<PathBuf>) -> PyResult<()> {
    use toml_edit::DocumentMut;

    // Resolve "format.key" prefix
    let (section, bare_key) = if let Some(fk) = key.strip_prefix("format.") {
        if str_to_format_key(fk).is_none() {
            return Err(pyo3::exceptions::PyKeyError::new_err(format!(
                "Unknown format config key: {fk}"
            )));
        }
        ("format", fk)
    } else {
        let section = key_to_section(key)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(format!("Unknown config key: {key}")))?;
        (section, key)
    };

    let file_path = path.unwrap_or_else(get_config_path);

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Read existing file or start empty
    let content = fs::read_to_string(&file_path).unwrap_or_default();
    let mut doc = content
        .parse::<DocumentMut>()
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("Invalid TOML: {e}")))?;

    // Ensure the section table exists
    if !doc.contains_table(section) {
        doc[section] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    // Convert Python value to toml_edit value
    if value.is_none() {
        // None = remove key (commented out as "unlimited")
        if let Some(table) = doc[section].as_table_mut() {
            table.remove(bare_key);
        }
    } else if let Ok(b) = value.extract::<bool>() {
        doc[section][bare_key] = toml_edit::value(b);
    } else if let Ok(i) = value.extract::<i64>() {
        doc[section][bare_key] = toml_edit::value(i);
    } else if let Ok(s) = value.extract::<String>() {
        doc[section][bare_key] = toml_edit::value(s);
    } else {
        return Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "Unsupported value type for config key '{key}'"
        )));
    }

    fs::write(&file_path, doc.to_string())
        .map_err(|e| pyo3::exceptions::PyIOError::new_err(format!("{}: {e}", file_path.display())))?;

    Ok(())
}

/// Shell completion values for -o/--optimize.
///
/// Kept next to `apply_optimization()` so additions stay in sync.
const OPTIMIZATION_COMPLETIONS: &[&str] = &[
    "tco", "tco:on", "tco:off", "jit", "jit:on", "jit:off", "level:0", "level:1", "level:2", "level:3", "memory:",
];

/// Return valid -o/--optimize completions for shell completion scripts.
#[pyfunction]
fn optimization_completions() -> Vec<&'static str> {
    OPTIMIZATION_COMPLETIONS.to_vec()
}

/// Return valid config keys from Rust source of truth.
#[pyfunction]
fn valid_config_keys() -> Vec<&'static str> {
    catnip_core::constants::CONFIG_VALID_KEYS.to_vec()
}

/// Return valid format config keys from Rust source of truth.
#[pyfunction]
fn valid_format_keys() -> Vec<&'static str> {
    catnip_core::constants::CONFIG_VALID_FORMAT_KEYS.to_vec()
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
    m.add_function(wrap_pyfunction!(set_config_value, m)?)?;
    m.add_function(wrap_pyfunction!(optimization_completions, m)?)?;
    m.add_function(wrap_pyfunction!(valid_config_keys, m)?)?;
    m.add_function(wrap_pyfunction!(valid_format_keys, m)?)?;
    Ok(())
}
