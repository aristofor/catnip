// FILE: catnip_repl/src/config_tui.rs
//! Standalone config editor TUI, reusable from `catnip config edit`.

use std::io::{self, Stdout, Write};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal as ct;
use crossterm::{cursor, execute};

use crate::config_editor::{ConfigAction, ConfigEditorState, ConfigItem, ConfigType, GROUPS, Row, StatusKind};
use crate::theme::{ANSI_DIM, ANSI_RESET, ANSI_SELECTED_BG, ANSI_STATUS_ERROR, ANSI_STATUS_INFO, ANSI_STATUS_SUCCESS};

type ConfigEntries = Vec<(String, String, String)>;

/// Load config entries by parsing `debug_report()` output from ConfigManager.
/// Returns (entries, resolved_config_path).
fn load_entries(config_path: Option<&str>) -> Result<(ConfigEntries, String), String> {
    use pyo3::prelude::*;

    Python::attach(|py| {
        let config_mod = py
            .import(pyo3::intern!(py, "catnip.config"))
            .map_err(|e| format!("{e}"))?;
        let cm_cls = config_mod
            .getattr(pyo3::intern!(py, "ConfigManager"))
            .map_err(|e| format!("{e}"))?;
        let cm = cm_cls.call0().map_err(|e| format!("{e}"))?;

        // load_file
        let py_path = match config_path {
            Some(p) => {
                let pathlib = py.import(pyo3::intern!(py, "pathlib")).map_err(|e| format!("{e}"))?;
                Some(
                    pathlib
                        .getattr("Path")
                        .map_err(|e| format!("{e}"))?
                        .call1((p,))
                        .map_err(|e| format!("{e}"))?,
                )
            }
            None => None,
        };
        cm.call_method1("load_file", (py_path,)).map_err(|e| format!("{e}"))?;
        cm.call_method0("load_env").map_err(|e| format!("{e}"))?;

        let lines: Vec<String> = cm
            .call_method0("debug_report")
            .and_then(|r| r.extract())
            .map_err(|e| format!("{e}"))?;

        let mut entries = Vec::new();
        for line in &lines {
            if line.starts_with("---") {
                continue;
            }
            let (raw_key, rest) = match line.split_once(": ") {
                Some(pair) => pair,
                None => continue,
            };
            let key = raw_key.strip_prefix("format.").unwrap_or(raw_key).to_string();
            let (value, source) = if let Some(bracket_pos) = rest.rfind('[') {
                let val = rest[..bracket_pos].trim().to_string();
                let src = rest[bracket_pos + 1..]
                    .trim_end_matches(']')
                    .split_once(' ')
                    .map(|(s, _)| s)
                    .unwrap_or(rest[bracket_pos + 1..].trim_end_matches(']'))
                    .to_string();
                (val, src)
            } else {
                (rest.to_string(), "default".to_string())
            };
            entries.push((key, value, source));
        }

        // Resolve actual config path
        let resolved_path: String = match config_path {
            Some(p) => p.to_string(),
            None => py
                .import(pyo3::intern!(py, "catnip.config"))
                .and_then(|m| m.getattr("get_config_path"))
                .and_then(|f| f.call0())
                .and_then(|p| p.str()?.extract())
                .unwrap_or_else(|_| "catnip.toml".to_string()),
        };

        Ok((entries, resolved_path))
    })
}

/// Apply a config action (save to TOML file via Rust toml_edit).
fn apply_action(action: &ConfigAction) -> Result<(), String> {
    use pyo3::prelude::*;
    use pyo3::types::PyBool;

    Python::attach(|py| {
        let (key, value, is_format) = match action {
            ConfigAction::SetValue { key, value, is_format } => (key.as_str(), value.as_str(), *is_format),
            ConfigAction::SetRepl { .. } => return Ok(()), // no-op in standalone
        };

        let py_value: Py<PyAny> = match value {
            "true" | "on" | "yes" => PyBool::new(py, true).to_owned().into_any().unbind(),
            "false" | "off" | "no" => PyBool::new(py, false).to_owned().into_any().unbind(),
            "unlimited" => py.None(),
            v => {
                if let Ok(i) = v.parse::<i64>() {
                    i.into_pyobject(py).unwrap().into_any().unbind()
                } else {
                    v.into_pyobject(py).unwrap().into_any().unbind()
                }
            }
        };

        let target_key = if is_format {
            format!("format.{}", key)
        } else {
            key.to_string()
        };

        // Call Rust set_config_value directly (via _rs module)
        py.import(pyo3::intern!(py, "catnip._rs"))
            .and_then(|m| m.getattr("set_config_value"))
            .and_then(|f| f.call1((&target_key, &py_value)))
            .map_err(|e| format!("{e}"))?;
        Ok(())
    })
}

// -- Rendering helpers --

fn format_value(item: &ConfigItem) -> String {
    match &item.config_type {
        ConfigType::Bool => {
            let is_true = item.value == "True" || item.value == "true";
            if is_true {
                "\x1b[32mon\x1b[0m".to_string()
            } else {
                "\x1b[90moff\x1b[0m".to_string()
            }
        }
        ConfigType::Choice(choices) => {
            let current = item.value.trim_matches('\'');
            let parts: Vec<String> = choices
                .iter()
                .map(|c| {
                    if *c == current {
                        format!("\x1b[1m{}\x1b[0m", c)
                    } else {
                        format!("\x1b[90m{}\x1b[0m", c)
                    }
                })
                .collect();
            parts.join("\x1b[90m | \x1b[0m")
        }
        ConfigType::Int { .. } => {
            format!("\x1b[1m{}\x1b[0m", item.value)
        }
    }
}

fn format_source(source: &str) -> String {
    match source {
        "default" => "\x1b[90mdefault\x1b[0m".to_string(),
        "file" => format!("\x1b[0m{}\x1b[0m", source),
        "env" => format!("\x1b[36m{}\x1b[0m", source),
        "cli" => format!("\x1b[96;1m{}\x1b[0m", source),
        _ => format!("\x1b[90m{}\x1b[0m", source),
    }
}

fn draw(stdout: &mut Stdout, state: &mut ConfigEditorState) -> io::Result<()> {
    let (_, term_h) = ct::size()?;
    let rows = state.rows();

    // Full screen: header(1) + status(1) + help reserve
    let avail_h = (term_h as usize).saturating_sub(3);
    let max_rows = avail_h.max(3).min(rows.len());
    state.ensure_visible(max_rows + 2);

    let visible_start = state.scroll_offset;
    let visible_end = (visible_start + max_rows).min(rows.len());
    let has_scroll_up = visible_start > 0;
    let has_scroll_down = visible_end < rows.len();

    // Header
    execute!(stdout, cursor::MoveTo(0, 0), ct::Clear(ct::ClearType::CurrentLine))?;
    write!(
        stdout,
        "\x1b[1m {}\x1b[0m\x1b[90m  (? help  q quit)\x1b[0m",
        state.title
    )?;

    let mut line_y: u16 = 1;

    for (ri, row) in rows.iter().enumerate().take(visible_end).skip(visible_start) {
        execute!(stdout, cursor::MoveTo(0, line_y), ct::Clear(ct::ClearType::CurrentLine))?;

        match row {
            Row::GroupHeader(gi) => {
                let label = GROUPS[*gi].group.label();
                let indicator = if ri == visible_start && has_scroll_up {
                    " \x1b[90m\u{25b2}\x1b[0m"
                } else if ri == visible_end - 1 && has_scroll_down {
                    " \x1b[90m\u{25bc}\x1b[0m"
                } else {
                    ""
                };
                write!(stdout, "  \x1b[1m{}\x1b[0m{}", label, indicator)?;
            }
            Row::Item(idx) => {
                let item = &state.items[*idx];
                let is_selected = *idx == state.selected;

                let modified = if item.is_modified() { "*" } else { " " };
                let marker = if is_selected { ">" } else { " " };

                let value_display = if is_selected {
                    if let Some(ref edit) = state.edit {
                        format!("\x1b[1m{}\x1b[0m\x1b[90m\u{2502}\x1b[0m", edit.buffer)
                    } else {
                        format_value(item)
                    }
                } else {
                    format_value(item)
                };

                let source_str = format_source(&item.source);

                let range_hint = if is_selected {
                    if let ConfigType::Int { min, max } = &item.config_type {
                        format!("  \x1b[90m{}..{}\x1b[0m", min, max)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                if is_selected {
                    write!(
                        stdout,
                        "{ANSI_SELECTED_BG}{}{} {:<18} {}{}  {}{ANSI_RESET}",
                        modified, marker, item.key, value_display, range_hint, source_str,
                    )?;
                } else {
                    write!(
                        stdout,
                        "{}  {:<18} {}  {}",
                        modified, item.key, value_display, source_str,
                    )?;
                }
            }
        }
        line_y += 1;
    }

    // Status line
    execute!(stdout, cursor::MoveTo(0, line_y), ct::Clear(ct::ClearType::CurrentLine))?;
    if let Some((ref msg, ref kind)) = state.status_message {
        let color = match kind {
            StatusKind::Success => ANSI_STATUS_SUCCESS,
            StatusKind::Error => ANSI_STATUS_ERROR,
            StatusKind::Info => ANSI_STATUS_INFO,
        };
        write!(stdout, "  \x1b[{color}m{msg}{ANSI_RESET}")?;
    } else if state.edit.is_some() {
        write!(stdout, "  {ANSI_DIM}Enter save  Esc cancel{ANSI_RESET}")?;
    } else {
        write!(
            stdout,
            "  {ANSI_DIM}Enter toggle  \u{2191}\u{2193} nav  Tab group  r reset  ? help  q quit{ANSI_RESET}"
        )?;
    }
    line_y += 1;

    // Help overlay
    if state.show_help {
        let help_lines = [
            "\u{2191}\u{2193}/jk  navigate       Enter/Space  toggle/edit",
            "Tab   next group     Shift+Tab    prev group",
            "g     first          G            last",
            "r     reset default  ?            toggle help",
            "q/Esc quit",
        ];
        for hl in &help_lines {
            execute!(stdout, cursor::MoveTo(0, line_y), ct::Clear(ct::ClearType::CurrentLine))?;
            write!(stdout, "  \x1b[90m{}\x1b[0m", hl)?;
            line_y += 1;
        }
    }

    // Clear remaining lines
    while line_y < term_h {
        execute!(stdout, cursor::MoveTo(0, line_y), ct::Clear(ct::ClearType::CurrentLine))?;
        line_y += 1;
    }

    stdout.flush()?;
    Ok(())
}

fn handle_key(state: &mut ConfigEditorState, key: KeyEvent) -> bool {
    // Edit mode
    if state.edit.is_some() {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                state.cancel_edit();
            }
            (_, KeyCode::Enter) => {
                if let Some(action) = state.confirm_edit() {
                    apply_and_report(state, &action);
                }
            }
            (_, KeyCode::Backspace) => state.edit_backspace(),
            (_, KeyCode::Left) => state.edit_move_left(),
            (_, KeyCode::Right) => state.edit_move_right(),
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(ch)) => state.edit_insert_char(ch),
            _ => {}
        }
        return false;
    }

    // Navigation mode
    match (key.modifiers, key.code) {
        (_, KeyCode::Esc) | (_, KeyCode::Char('q')) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => return true,
        (_, KeyCode::Up) | (_, KeyCode::Char('k')) => state.select_prev(),
        (_, KeyCode::Down) | (_, KeyCode::Char('j')) => state.select_next(),
        (_, KeyCode::Enter) | (_, KeyCode::Char(' ')) => {
            if let Some(action) = state.toggle_or_enter_edit() {
                apply_and_report(state, &action);
            }
        }
        (_, KeyCode::Tab) => state.jump_next_group(),
        (KeyModifiers::SHIFT, KeyCode::BackTab) => state.jump_prev_group(),
        (_, KeyCode::Home) | (_, KeyCode::Char('g')) => state.select_first(),
        (KeyModifiers::SHIFT, KeyCode::Char('G')) | (_, KeyCode::End) => state.select_last(),
        (_, KeyCode::PageDown) => state.page_down(5),
        (_, KeyCode::PageUp) => state.page_up(5),
        (_, KeyCode::Char('r')) => {
            if let Some(action) = state.reset_selected() {
                apply_and_report(state, &action);
            }
        }
        (_, KeyCode::Char('?')) => state.show_help = !state.show_help,
        _ => {}
    }
    false
}

fn apply_and_report(state: &mut ConfigEditorState, action: &ConfigAction) {
    match apply_action(action) {
        Ok(()) => {
            let (key, value) = match action {
                ConfigAction::SetValue { key, value, .. } => (key.as_str(), value.as_str()),
                ConfigAction::SetRepl { key, value } => (key.as_str(), value.as_str()),
            };
            state.status_message = Some((format!("{} = {} (saved)", key, value), StatusKind::Success));
        }
        Err(e) => {
            state.status_message = Some((format!("Error: {}", e), StatusKind::Error));
        }
    }
}

/// Run the standalone config editor TUI. Returns when user quits.
pub fn run(config_path: Option<&str>) -> Result<(), String> {
    // Load data
    let (entries, resolved_path) = load_entries(config_path)?;
    let mut state = ConfigEditorState::new();
    // No REPL entries in standalone mode
    state.load(entries, vec![]);
    state.title = resolved_path;

    // Enter alternate screen + raw mode
    let mut stdout = io::stdout();
    ct::enable_raw_mode().map_err(|e| format!("{e}"))?;
    execute!(stdout, ct::EnterAlternateScreen, cursor::Hide).map_err(|e| format!("{e}"))?;

    let result = (|| -> io::Result<()> {
        draw(&mut stdout, &mut state)?;
        loop {
            if let Event::Key(key) = event::read()? {
                if handle_key(&mut state, key) {
                    break;
                }
                draw(&mut stdout, &mut state)?;
            }
        }
        Ok(())
    })();

    // Restore terminal
    let _ = execute!(stdout, cursor::Show, ct::LeaveAlternateScreen);
    let _ = ct::disable_raw_mode();

    result.map_err(|e| format!("{e}"))
}
