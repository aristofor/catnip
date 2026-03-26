// FILE: catnip_core/src/paths.rs
//! XDG-compliant path helpers - pure Rust, no PyO3.

use std::env;
use std::path::PathBuf;

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
        PathBuf::from(xdg).join("catnip")
    } else {
        get_home_dir().join(".cache").join("catnip")
    }
}
