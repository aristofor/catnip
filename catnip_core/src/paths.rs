// FILE: catnip_core/src/paths.rs
//! XDG-compliant path helpers - pure Rust, no PyO3.

use std::env;
use std::path::PathBuf;

/// App directory name under XDG bases.
const APP_DIR: &str = "catnip";

/// Get home directory, respecting $HOME env var (for tests).
fn get_home_dir() -> PathBuf {
    if let Ok(home) = env::var("HOME") {
        PathBuf::from(home)
    } else {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Return the Catnip cache directory following XDG conventions.
pub fn get_cache_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join(APP_DIR)
    } else {
        get_home_dir().join(".cache").join(APP_DIR)
    }
}

/// Return the Catnip config directory following XDG conventions.
pub fn get_config_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join(APP_DIR)
    } else {
        get_home_dir().join(".config").join(APP_DIR)
    }
}

/// Return the Catnip state directory following XDG conventions.
pub fn get_state_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg).join(APP_DIR)
    } else {
        get_home_dir().join(".local").join("state").join(APP_DIR)
    }
}

/// Return the Catnip data directory following XDG conventions.
pub fn get_data_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg).join(APP_DIR)
    } else {
        get_home_dir().join(".local").join("share").join(APP_DIR)
    }
}

/// Existing directories listed in $CATNIP_STDLIB_PATH (colon-separated).
pub fn stdlib_env_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(val) = env::var(crate::constants::ENV_STDLIB_PATH) {
        for p in val.split(':') {
            let pb = PathBuf::from(p);
            if pb.is_dir() {
                paths.push(pb);
            }
        }
    }
    paths
}
