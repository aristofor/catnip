// FILE: catnip_tools/src/ffi.rs
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic;

use crate::config::{FormatConfig, LintConfig};
use crate::formatter;
use crate::linter;

/// ABI version string (static, caller must not free)
const VERSION: &[u8] = b"0.0.4\0";

#[unsafe(no_mangle)]
pub extern "C" fn catnip_tools_version() -> *const c_char {
    VERSION.as_ptr() as *const c_char
}

/// Format source code. Returns JSON `{"ok":"..."}` or `{"err":"..."}`.
/// Caller must free the result with `catnip_tools_free`.
#[unsafe(no_mangle)]
pub extern "C" fn catnip_tools_format(
    source: *const c_char,
    indent_size: u32,
    line_length: u32,
) -> *mut c_char {
    let result = panic::catch_unwind(|| {
        let source = unsafe { CStr::from_ptr(source) }
            .to_str()
            .map_err(|e| format!("Invalid UTF-8: {}", e))?;

        let config = FormatConfig {
            indent_size: indent_size as usize,
            line_length: line_length as usize,
        };

        formatter::format_code(source, &config)
    });

    match result {
        Ok(Ok(formatted)) => json_ok(&formatted),
        Ok(Err(e)) => json_err(&e),
        Err(_) => json_err("panic in format_code"),
    }
}

/// Lint source code. Returns JSON `{"ok":[...]}` or `{"err":"..."}`.
/// Caller must free the result with `catnip_tools_free`.
#[unsafe(no_mangle)]
pub extern "C" fn catnip_tools_lint(
    source: *const c_char,
    check_syntax: bool,
    check_style: bool,
    check_semantic: bool,
) -> *mut c_char {
    let result = panic::catch_unwind(|| {
        let source = unsafe { CStr::from_ptr(source) }
            .to_str()
            .map_err(|e| format!("Invalid UTF-8: {}", e))?;

        let config = LintConfig {
            check_syntax,
            check_style,
            check_semantic,
            check_ir: false,
        };

        linter::lint_code(source, &config)
    });

    match result {
        Ok(Ok(diagnostics)) => match serde_json::to_string(&diagnostics) {
            Ok(json) => json_ok_raw(&json),
            Err(e) => json_err(&format!("JSON serialization failed: {}", e)),
        },
        Ok(Err(e)) => json_err(&e),
        Err(_) => json_err("panic in lint_code"),
    }
}

/// Free a string returned by format/lint functions.
#[unsafe(no_mangle)]
pub extern "C" fn catnip_tools_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            drop(CString::from_raw(ptr));
        }
    }
}

// --- JSON helpers ---

fn json_ok(value: &str) -> *mut c_char {
    let json = serde_json::json!({"ok": value});
    to_cstring(&json.to_string())
}

fn json_ok_raw(json_array: &str) -> *mut c_char {
    let out = format!(r#"{{"ok":{}}}"#, json_array);
    to_cstring(&out)
}

fn json_err(msg: &str) -> *mut c_char {
    let json = serde_json::json!({"err": msg});
    to_cstring(&json.to_string())
}

fn to_cstring(s: &str) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}
