// FILE: catnip_repl/src/theme.rs
//! Conversion u32 hex -> ratatui::style::Color
//!
//! Constants are generated as raw u32 by catnip_rs build.rs.
//! This module provides the conversion to Color::Rgb for ratatui.

use ratatui::style::Color;

pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_DIM: &str = "\x1b[90m";
pub const ANSI_SELECTED_BG: &str = "\x1b[48;2;60;60;80m";
pub const ANSI_STATUS_SUCCESS: &str = "32";
pub const ANSI_STATUS_ERROR: &str = "33";
pub const ANSI_STATUS_INFO: &str = "90";

/// Convert a u32 hex (0xRRGGBB) to Color::Rgb
pub const fn hex(rgb: u32) -> Color {
    Color::Rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    )
}
