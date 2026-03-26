// FILE: catnip_rs/src/weird_log.rs
//! Crash logging for internal errors (WeirdErrors).
//!
//! Writes JSON reports to `$XDG_STATE_HOME/catnip/weird/` for post-mortem
//! debugging. Works in both Python (via PyO3) and standalone Rust contexts.

use crate::config::{get_config_path, get_state_dir};
use crate::constants::parse_bool_value;
use pyo3::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::constants::WEIRD_LOG_MAX_DEFAULT;
const SCHEMA_VERSION: u32 = 1;

type PyTracebackFrame = (Option<String>, Option<String>, Option<u32>);

/// Location context for the error.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WeirdLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// A single traceback frame.
#[derive(Debug, Clone, Serialize)]
pub struct TraceFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Error details section.
#[derive(Debug, Clone, Serialize)]
pub struct WeirdErrorInfo {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub details: HashMap<String, serde_json::Value>,
}

/// Full crash report written to disk.
#[derive(Debug, Clone, Serialize)]
pub struct WeirdReport {
    pub version: u32,
    pub timestamp: String,
    pub catnip_version: String,
    pub python_version: Option<String>,
    pub platform: String,
    pub error: WeirdErrorInfo,
    pub location: WeirdLocation,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub traceback: Vec<TraceFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python_traceback: Option<String>,
}

impl WeirdReport {
    /// Build a report from minimal info (standalone Rust context).
    pub fn new(message: String, cause: Option<String>) -> Self {
        Self {
            version: SCHEMA_VERSION,
            timestamp: now_iso8601(),
            catnip_version: env!("CARGO_PKG_VERSION").to_string(),
            python_version: None,
            platform: current_platform(),
            error: WeirdErrorInfo {
                message,
                cause,
                details: HashMap::new(),
            },
            location: WeirdLocation::default(),
            traceback: Vec::new(),
            python_traceback: None,
        }
    }
}

/// Log a weird error report to disk. Silent on any I/O failure.
pub fn log_weird_error(report: &WeirdReport) {
    let _ = log_weird_error_inner(report);
}

fn log_weird_error_inner(report: &WeirdReport) -> Result<(), Box<dyn std::error::Error>> {
    if !is_logging_enabled() {
        return Ok(());
    }

    let weird_dir = get_weird_dir();
    fs::create_dir_all(&weird_dir)?;

    let rand_hex = random_hex_6();
    // Extract YYYYMMDD_HHMMSS from ISO timestamp
    let ts_slug = report
        .timestamp
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14)
        .collect::<String>();
    let filename = format!("weird_{}_{}.json", format_timestamp_slug(&ts_slug), rand_hex);

    let data = serde_json::to_string_pretty(report)?;

    // Atomic write: tmp file + rename
    let tmp_path = weird_dir.join(format!(".tmp_{}", rand_hex));
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, weird_dir.join(&filename))?;

    rotate_logs(&weird_dir, WEIRD_LOG_MAX_DEFAULT);

    Ok(())
}

/// Check if logging is enabled. Resolution: env var > TOML > default(true).
fn is_logging_enabled() -> bool {
    // 1. Env var
    if let Ok(val) = env::var("CATNIP_WEIRD_LOG") {
        return parse_bool_value(&val.to_lowercase()).unwrap_or(false);
    }

    // 2. TOML config
    if let Some(val) = read_toml_setting() {
        return val;
    }

    // 3. Default
    true
}

/// Read log_weird_errors from TOML config file.
fn read_toml_setting() -> Option<bool> {
    let path = get_config_path();
    let content = fs::read_to_string(path).ok()?;
    let data: toml::Table = toml::from_str(&content).ok()?;
    let section = data.get("diagnostics")?.as_table()?;
    let val = section.get("log_weird_errors")?;
    val.as_bool()
}

fn get_weird_dir() -> PathBuf {
    get_state_dir().join("weird")
}

fn rotate_logs(dir: &PathBuf, max_files: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "json")
                && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("weird_"))
        })
        .collect();

    if files.len() <= max_files {
        return;
    }

    files.sort();
    let excess = files.len() - max_files;
    for f in files.iter().take(excess) {
        let _ = fs::remove_file(f);
    }
}

fn now_iso8601() -> String {
    // Use std::time for a portable timestamp without extra deps
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();

    // Convert to UTC datetime components
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    let micros = duration.subsec_micros();

    // Days since 1970-01-01 to Y-M-D (simplified, correct for 1970-2100)
    let (year, month, day) = days_to_ymd(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}+00:00",
        year, month, day, hours, minutes, seconds, micros
    )
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Algorithm from Howard Hinnant (public domain)
    days += 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn format_timestamp_slug(digits: &str) -> String {
    if digits.len() >= 14 {
        // YYYYMMDD_HHMMSS
        format!("{}_{}", &digits[..8], &digits[8..14])
    } else {
        digits.to_string()
    }
}

fn random_hex_6() -> String {
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = fs::File::open("/dev/urandom") {
            let mut buf = [0u8; 3];
            if f.read_exact(&mut buf).is_ok() {
                return format!("{:02x}{:02x}{:02x}", buf[0], buf[1], buf[2]);
            }
        }
    }
    // Fallback
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:06x}", ns & 0xFFFFFF)
}

fn current_platform() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    format!("{}-{}", os, arch)
}

// ---------------------------------------------------------------------------
// PyO3 interface
// ---------------------------------------------------------------------------

/// Log a weird error from Python. Collects Python-specific info and delegates
/// to the Rust core.
#[pyfunction]
#[pyo3(signature = (message, cause=None, details=None, filename=None, line=None, column=None, context=None, traceback_frames=None, python_traceback=None, python_version=None))]
#[allow(clippy::too_many_arguments)]
pub fn log_weird_error_py(
    message: String,
    cause: Option<String>,
    details: Option<HashMap<String, Py<PyAny>>>,
    filename: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
    context: Option<String>,
    traceback_frames: Option<Vec<PyTracebackFrame>>,
    python_traceback: Option<String>,
    python_version: Option<String>,
) {
    // This function is called from Python, so GIL is already held.
    // Use Python::attach to access it.
    let py_details = Python::attach(|py| {
        details
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| {
                let bound = v.bind(py);
                if let Ok(b) = bound.extract::<bool>() {
                    Some((k, serde_json::Value::Bool(b)))
                } else if let Ok(i) = bound.extract::<i64>() {
                    Some((k, serde_json::Value::Number(i.into())))
                } else if let Ok(s) = bound.extract::<String>() {
                    Some((k, serde_json::Value::String(s)))
                } else if let Ok(repr) = bound.repr() {
                    Some((k, serde_json::Value::String(repr.to_string())))
                } else {
                    None
                }
            })
            .collect()
    });

    let frames = traceback_frames
        .unwrap_or_default()
        .into_iter()
        .map(|(func, file, ln)| TraceFrame {
            function: func,
            filename: file,
            line: ln,
        })
        .collect();

    let report = WeirdReport {
        version: SCHEMA_VERSION,
        timestamp: now_iso8601(),
        catnip_version: env!("CARGO_PKG_VERSION").to_string(),
        python_version,
        platform: current_platform(),
        error: WeirdErrorInfo {
            message,
            cause,
            details: py_details,
        },
        location: WeirdLocation {
            filename,
            line,
            column,
            context,
        },
        traceback: frames,
        python_traceback,
    };

    log_weird_error(&report);
}

/// Check if weird error logging is enabled (for testing).
#[pyfunction]
pub fn is_weird_log_enabled() -> bool {
    is_logging_enabled()
}

/// Register weird_log module functions.
pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(log_weird_error_py, m)?)?;
    m.add_function(wrap_pyfunction!(is_weird_log_enabled, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_iso8601_format() {
        let ts = now_iso8601();
        // Should match YYYY-MM-DDTHH:MM:SS.ffffff+00:00
        assert!(ts.contains('T'));
        assert!(ts.ends_with("+00:00"));
        assert!(ts.len() >= 32);
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // 2026-02-28 = day 20512 since epoch
        let (y, m, d) = days_to_ymd(20512);
        assert_eq!((y, m, d), (2026, 2, 28));
    }

    #[test]
    fn test_random_hex_length() {
        let hex = random_hex_6();
        assert_eq!(hex.len(), 6);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_format_timestamp_slug() {
        assert_eq!(format_timestamp_slug("20260228153245"), "20260228_153245");
    }

    #[test]
    fn test_report_serialization() {
        let report = WeirdReport::new("stack underflow".into(), Some("vm".into()));
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("stack underflow"));
        assert!(json.contains("\"cause\":\"vm\""));
    }

    // Env-var tests must be serialized: they share process-wide state.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_logging_disabled_via_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { env::set_var("CATNIP_WEIRD_LOG", "off") };
        assert!(!is_logging_enabled());
        unsafe { env::remove_var("CATNIP_WEIRD_LOG") };
    }

    #[test]
    fn test_logging_enabled_via_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { env::set_var("CATNIP_WEIRD_LOG", "on") };
        assert!(is_logging_enabled());
        unsafe { env::remove_var("CATNIP_WEIRD_LOG") };
    }

    #[test]
    fn test_logging_default_enabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { env::remove_var("CATNIP_WEIRD_LOG") };
        // Without TOML config, default is true
        assert!(is_logging_enabled());
    }
}
