// FILE: catnip_repl/src/config.rs
//! REPL Configuration
//!
//! Uses `catnip_rs::constants` for default values.

use crate::theme::hex;
use ratatui::style::Color;

/// REPL Configuration
pub struct ReplConfig {
    // Prompts
    pub prompt_main: String,
    pub prompt_continuation: String,

    // Colors (ratatui)
    pub color_prompt: Color,
    pub color_output: Color,
    pub color_error: Color,
    pub color_info: Color,
    pub color_success: Color,
    pub color_dim: Color,

    // Messages
    pub welcome_message: String,

    // Behavior
    pub show_parse_time: bool,
    pub show_exec_time: bool,
    pub enable_jit: bool,
    pub jit_threshold: u32,
    pub debug_mode: bool,

    // Theme
    pub is_dark: bool,

    // History
    pub history_file: String,
    pub max_history: usize,
}

/// Detect dark/light terminal background.
///
/// Cascade: CATNIP_THEME env -> COLORFGBG -> default dark.
fn detect_is_dark() -> bool {
    // CATNIP_THEME explicit override
    if let Ok(theme) = std::env::var("CATNIP_THEME") {
        match theme.to_lowercase().as_str() {
            "dark" => return true,
            "light" => return false,
            _ => {} // "auto" or invalid -> fall through
        }
    }

    // COLORFGBG (xterm, rxvt, Konsole): format "fg;bg" or "fg;aux;bg"
    if let Ok(colorfgbg) = std::env::var("COLORFGBG") {
        if let Some(bg_str) = colorfgbg.rsplit(';').next() {
            if let Ok(bg) = bg_str.parse::<u32>() {
                return bg <= 8;
            }
        }
    }

    true // default: dark
}

impl Default for ReplConfig {
    fn default() -> Self {
        use catnip_rs::constants;

        Self {
            prompt_main: constants::REPL_PROMPT_MAIN.to_string(),
            prompt_continuation: constants::REPL_PROMPT_CONTINUATION.to_string(),

            color_prompt: hex(constants::ui_colors::PROMPT),
            color_output: Color::Reset,
            color_error: hex(constants::ui_colors::ERROR),
            color_info: hex(constants::ui_colors::INFO),
            color_success: hex(constants::ui_colors::SUCCESS),
            color_dim: hex(constants::ui_colors::DIM),

            welcome_message: constants::REPL_WELCOME_TEMPLATE.replace("{version}", env!("CARGO_PKG_VERSION")),

            show_parse_time: false,
            show_exec_time: false,
            enable_jit: constants::JIT_ENABLED_DEFAULT,
            jit_threshold: constants::JIT_THRESHOLD_DEFAULT,
            debug_mode: false,

            is_dark: detect_is_dark(),

            history_file: constants::REPL_HISTORY_FILE.to_string(),
            max_history: constants::REPL_MAX_HISTORY,
        }
    }
}

impl ReplConfig {
    /// Enable verbose mode (show timings)
    pub fn verbose(mut self) -> Self {
        self.show_parse_time = true;
        self.show_exec_time = true;
        self
    }

    /// Disable JIT
    pub fn no_jit(mut self) -> Self {
        self.enable_jit = false;
        self
    }

    /// Custom JIT threshold
    pub fn with_jit_threshold(mut self, threshold: u32) -> Self {
        self.jit_threshold = threshold;
        self
    }
}

/// Version info
pub fn version_info() -> String {
    format!(
        "Catnip REPL v{}\n\
         Build: {} mode\n\
         Features: JIT (Cranelift), NaN-boxing VM, Rust builtins",
        env!("CARGO_PKG_VERSION"),
        if cfg!(debug_assertions) { "debug" } else { "release" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ReplConfig::default();
        assert_eq!(config.prompt_main, "▸ ");
        assert!(config.enable_jit);
        assert_eq!(config.jit_threshold, 100);
    }

    #[test]
    fn test_custom_config() {
        let config = ReplConfig::default().verbose().with_jit_threshold(50);
        assert!(config.show_parse_time);
        assert_eq!(config.jit_threshold, 50);
    }
}
