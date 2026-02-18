// FILE: catnip_repl/src/theme.rs
//! Conversion u32 hex -> ratatui::style::Color
//!
//! Constants are generated as raw u32 by catnip_rs build.rs.
//! This module provides the conversion to Color::Rgb for ratatui.

use ratatui::style::Color;

/// Convert a u32 hex (0xRRGGBB) to Color::Rgb
pub const fn hex(rgb: u32) -> Color {
    Color::Rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    )
}
