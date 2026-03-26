// FILE: catnip_repl/src/app.rs
//! Main TUI loop with ratatui inline rendering.
//!
//! Output is pushed into the scrollback via insert_before(),
//! the inline viewport only contains the input (1 line).
//! The completion popup is rendered directly via crossterm,
//! outside the ratatui viewport (which does not support dynamic
//! resize for Viewport::Inline).

use crate::commands::generate_help_text;
use crate::completer::{CatnipCompleter, CompletionState};
use crate::config::{ReplConfig, version_info};
use crate::config_editor::{ConfigAction, ConfigEditorState};
use crate::executor::{ReplExecutor, ValueKind};
use crate::highlighter::CatnipHighlighter;
use crate::hints::HintEngine;
use crate::history::History;
use crate::input::InputState;
use crate::theme::{ANSI_DIM, ANSI_RESET, ANSI_SELECTED_BG, ANSI_STATUS_ERROR, ANSI_STATUS_INFO, ANSI_STATUS_SUCCESS};
use crate::widgets::completion::MAX_VISIBLE;

use crossterm::cursor;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, ResetColor, SetAttribute};
use crossterm::terminal::{self as ct};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use std::io::{self, Stdout, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};
use unicode_width::UnicodeWidthStr;

const USAGE_LOAD: &str = "Usage: /load <file.cat>";
const USAGE_TIME: &str = "Usage: /time <expression>";
const USAGE_CONFIG_GET: &str = "Usage: /config get KEY";
const USAGE_CONFIG_SET: &str = "Usage: /config set KEY VALUE";
const USAGE_CONFIG: &str = "Usage: /config [show|get KEY|set KEY VALUE|path]";
const EXPRESSION_FAILED: &str = "Expression failed";
const EXPRESSION_FAILED_WARMUP: &str = "Expression failed during warmup";
const EXPRESSION_FAILED_BENCHMARK: &str = "Expression failed during benchmark";
const NO_HISTORY_YET: &str = "No history yet";
const NO_USER_VARIABLES_DEFINED: &str = "No user variables defined";
const TYPE_HELP_FOR_COMMANDS: &str = "Type /help for available commands";
const FILE_LOADED_SUCCESSFULLY: &str = "File loaded successfully";
const CONFIG_SAVE_SUFFIX: &str = " (saved)";
const CONFIG_EDITOR_ERROR_PREFIX: &str = "Error: ";

/// REPL exit reason
pub enum ExitReason {
    Ok,
    Abort,
}

impl ExitReason {
    /// Pick a random message from the corresponding array
    pub fn message(&self) -> &'static str {
        use catnip_rs::constants;

        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        // 1% weird
        let rare = constants::REPL_EXIT_RARE;
        if nanos.is_multiple_of(100) {
            return rare[nanos / 100 % rare.len()];
        }
        let msgs = match self {
            ExitReason::Ok => constants::REPL_EXIT_OK,
            ExitReason::Abort => constants::REPL_EXIT_ABORT,
        };
        msgs[nanos % msgs.len()]
    }
}

/// Reverse incremental search state (Ctrl+R)
struct SearchState {
    /// Current search query
    query: String,
    /// Matching history indices (from `history.search()`)
    matches: Vec<usize>,
    /// Current position in the matches list
    match_index: usize,
    /// Whether search mode is active
    active: bool,
}

impl SearchState {
    fn new() -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            match_index: 0,
            active: false,
        }
    }

    fn reset(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.match_index = 0;
        self.active = false;
    }
}

pub struct App {
    config: ReplConfig,
    executor: ReplExecutor,
    input: InputState,
    history: History,
    completer: CatnipCompleter,
    hints: HintEngine,
    highlighter: Option<CatnipHighlighter>,
    completion: CompletionState,
    /// Ghost text displayed after cursor (fish-like)
    current_hint: Option<String>,
    exit_reason: Option<ExitReason>,
    /// Number of popup lines displayed on last render (for clearing)
    last_popup_lines: u16,
    /// Number of continuation lines displayed (for clearing)
    last_continuation_lines: u16,
    /// Number of config editor lines displayed on last render (for clearing)
    last_config_editor_lines: u16,
    /// Interactive config editor overlay
    config_editor: ConfigEditorState,
    /// Viewport Y position (line 0) in the terminal
    viewport_y: u16,
    /// Reverse search state (Ctrl+R)
    search: SearchState,
}

impl App {
    fn format_saved_assignment(key: &str, value: &str) -> String {
        format!("{key} = {value}{CONFIG_SAVE_SUFFIX}")
    }

    fn format_unknown_command(command: &str) -> String {
        format!("Unknown command: /{command}")
    }

    fn format_variable_not_found(name: &str) -> String {
        format!("Variable '{name}' not found")
    }

    fn format_unknown_repl_key(key: &str) -> String {
        format!("Unknown REPL key: {key}")
    }

    fn format_config_editor_error(err: &dyn std::fmt::Display) -> String {
        format!("{CONFIG_EDITOR_ERROR_PREFIX}{err}")
    }

    fn format_loading_file(filename: &str) -> String {
        format!("Loading {filename}...")
    }

    fn format_file_read_error(filename: &str, err: &dyn std::fmt::Display) -> String {
        format!("Failed to read {filename}: {err}")
    }

    fn format_file_line_error(line: usize, err: &str) -> String {
        format!("Line {line}: {err}")
    }

    fn format_benchmarking(expression: &str) -> String {
        format!("Benchmarking: {expression}")
    }

    pub fn new(config: ReplConfig) -> Result<Self, String> {
        let executor = ReplExecutor::new()?;

        let history_path = get_history_path(&config);
        let history = History::load(&history_path, config.max_history);

        let completer = CatnipCompleter::new();
        let hints = HintEngine::new();
        let highlighter = CatnipHighlighter::new(config.is_dark).ok();

        Ok(Self {
            config,
            executor,
            input: InputState::new(),
            history,
            completer,
            hints,
            highlighter,
            completion: CompletionState::new(),
            current_hint: None,
            exit_reason: None,
            last_popup_lines: 0,
            last_continuation_lines: 0,
            last_config_editor_lines: 0,
            config_editor: ConfigEditorState::new(),
            viewport_y: 0,
            search: SearchState::new(),
        })
    }

    pub fn run(mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<ExitReason> {
        // Welcome message
        self.print_dim(terminal, &self.config.welcome_message.clone());

        let mut stdout = io::stdout();
        crossterm::execute!(stdout, EnableBracketedPaste)?;

        loop {
            // Track previous extra lines for cleanup
            let prev_extra = self.last_continuation_lines + self.last_popup_lines + self.last_config_editor_lines;

            // Hide cursor during render to prevent flicker
            crossterm::queue!(stdout, cursor::Hide, SetAttribute(Attribute::Reset), ResetColor)?;

            // Draw input line 0 via ratatui (viewport = 1 ligne)
            // viewport_y is set inside render_inline from f.area().y
            terminal.draw(|f| self.render_inline(f))?;

            // Scroller si pas assez de place pour continuation + popup
            self.ensure_space_below(terminal)?;
            // Continuation lines via crossterm (queued, not flushed)
            self.draw_continuation_lines(&mut stdout)?;
            // Popup via crossterm (queued, not flushed)
            self.draw_completion_popup(&mut stdout)?;
            // Config editor overlay (queued, not flushed)
            self.draw_config_editor(&mut stdout)?;

            // Clear excess lines from previous frame
            let curr_extra = self.last_continuation_lines + self.last_popup_lines + self.last_config_editor_lines;
            for i in curr_extra..prev_extra {
                let y = self.viewport_y + 1 + i;
                crossterm::queue!(stdout, cursor::MoveTo(0, y), ct::Clear(ct::ClearType::CurrentLine))?;
            }

            // Position cursor and show
            let (crow, ccol) = self.input.cursor();
            let prompt_len = if crow == 0 {
                self.config.prompt_main.width()
            } else {
                self.config.prompt_continuation.width()
            };
            let display_col = self.input.lines()[crow][..ccol].width();
            let cursor_x = prompt_len as u16 + display_col as u16;
            let cursor_y = self.viewport_y + crow as u16;
            crossterm::queue!(stdout, cursor::MoveTo(cursor_x, cursor_y), cursor::Show)?;

            // Single atomic flush
            stdout.flush()?;

            // Wait for event
            match event::read()? {
                Event::Key(key) => {
                    self.handle_key_event(key, terminal)?;
                }
                Event::Paste(text) => {
                    self.handle_paste_event(text);
                }
                _ => {}
            }

            if let Some(reason) = &self.exit_reason {
                let msg = reason.message();
                self.print_dim(terminal, msg);
                break;
            }
        }

        let reason = self.exit_reason.unwrap_or(ExitReason::Abort);

        // Ensure cursor visible on exit
        crossterm::execute!(stdout, cursor::Show, DisableBracketedPaste)?;

        // Save history
        let _ = self.history.save();
        Ok(reason)
    }

    // -- Rendering (viewport ratatui = input seulement) --

    fn render_inline(&mut self, f: &mut ratatui::Frame) {
        let area = f.area();
        // Track viewport position from ratatui (avoids extra cursor::position() query)
        self.viewport_y = area.y;
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Seule la ligne 0 est rendue dans le viewport inline (1 ligne)
        // Les lignes de continuation sont rendues via crossterm
        let line_text = &self.input.lines()[0];

        // In search mode, replace the prompt with the search indicator
        if self.search.active {
            let search_prompt = format!("(reverse-i-search)`{}': ", self.search.query);
            let dim = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
            let mut spans = vec![Span::styled(search_prompt.clone(), dim)];
            if let Some(ref hl) = self.highlighter {
                spans.extend(hl.highlight_line(line_text));
            } else {
                spans.push(Span::raw(line_text.as_str()));
            }
            let line_area = Rect::new(area.x, area.y, area.width, 1);
            Widget::render(Clear, line_area, f.buffer_mut());
            Widget::render(Line::from(spans), line_area, f.buffer_mut());

            // Cursor at end of search prompt (after the query)
            let cursor_x = area.x + search_prompt.width() as u16 + line_text.width() as u16;
            f.set_cursor_position((cursor_x, area.y));
            return;
        }

        let prompt = &self.config.prompt_main;
        let prompt_style = Style::default().fg(self.config.color_prompt);

        let mut spans = vec![Span::styled(prompt.as_str(), prompt_style)];
        if let Some(ref hl) = self.highlighter {
            spans.extend(hl.highlight_line(line_text));
        } else {
            spans.push(Span::raw(line_text.as_str()));
        }

        // Ghost text hint on line 0 (only if single-line and cursor at end)
        let (crow, _) = self.input.cursor();
        if crow == 0 {
            if let Some(ref hint) = self.current_hint {
                let dim_style = Style::default().fg(Color::DarkGray);
                spans.push(Span::styled(hint.as_str(), dim_style));
            }
        }

        let line_area = Rect::new(area.x, area.y, area.width, 1);
        Widget::render(Clear, line_area, f.buffer_mut());
        Widget::render(Line::from(spans), line_area, f.buffer_mut());

        // Cursor: toujours sur la ligne 0 pour ratatui
        // draw_continuation_lines repositionne si crow > 0
        let (crow, ccol) = self.input.cursor();
        let prompt_w = prompt.width();
        if crow == 0 {
            let display_col = self.input.lines()[0][..ccol].width();
            let cursor_x = area.x + prompt_w as u16 + display_col as u16;
            f.set_cursor_position((cursor_x, area.y));
        } else {
            let cursor_x = area.x + prompt_w as u16 + line_text.width() as u16;
            f.set_cursor_position((cursor_x, area.y));
        }
    }

    // -- Scroll + continuation + popup (rendu via crossterm hors viewport) --

    /// Scroll the terminal if not enough room for continuation + popup
    fn ensure_space_below(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        let line_count = self.input.line_count();
        let extra_lines = line_count.saturating_sub(1);

        let popup_needed = if self.completion.active && !self.completion.suggestions.is_empty() {
            let total = self.completion.suggestions.len();
            let max_visible = total.min(MAX_VISIBLE);
            max_visible + if total > max_visible { 1 } else { 0 }
        } else {
            0
        };

        let config_editor_needed = self.config_editor.visible_lines();

        let total_needed = extra_lines + popup_needed + config_editor_needed;
        if total_needed == 0 {
            return Ok(());
        }

        let (term_w, term_rows) = ct::size()?;
        let space_below = term_rows.saturating_sub(self.viewport_y).saturating_sub(1) as usize;

        if space_below < total_needed {
            let scroll = (total_needed - space_below) as u16;
            let mut stdout = io::stdout();
            for _ in 0..scroll {
                writeln!(stdout)?;
            }
            crossterm::execute!(stdout, cursor::MoveUp(scroll))?;
            stdout.flush()?;

            terminal.resize(Rect::new(0, 0, term_w, term_rows))?;
            // viewport_y is set inside render_inline from f.area().y
            terminal.draw(|f| self.render_inline(f))?;
        }

        Ok(())
    }

    /// Render continuation lines (2+) via crossterm (queued, caller flushes)
    fn draw_continuation_lines(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        let line_count = self.input.line_count();
        if line_count <= 1 {
            self.last_continuation_lines = 0;
            return Ok(());
        }

        let prompt = &self.config.prompt_continuation;
        let prompt_fg = color_to_ansi_fg(self.config.color_prompt);

        for i in 1..line_count {
            let y = self.viewport_y + i as u16;
            crossterm::queue!(stdout, cursor::MoveTo(0, y), ct::Clear(ct::ClearType::CurrentLine))?;

            // Prompt coloré
            write!(stdout, "{}{}\x1b[0m", prompt_fg, prompt)?;

            // Contenu avec highlighting
            let line_text = self.input.lines()[i].clone();
            if let Some(ref hl) = self.highlighter {
                let spans = hl.highlight_line(&line_text);
                for span in &spans {
                    write!(stdout, "{}", span_to_ansi(span))?;
                }
            } else {
                write!(stdout, "{}", line_text)?;
            }

            // Ghost text hint on last continuation line
            if i == line_count - 1 {
                if let Some(ref hint) = self.current_hint {
                    write!(stdout, "\x1b[90m{}\x1b[0m", hint)?;
                }
            }
        }

        self.last_continuation_lines = (line_count - 1) as u16;
        Ok(())
    }

    /// Render the completion popup below all input lines (queued, caller flushes)
    fn draw_completion_popup(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        if !self.completion.active || self.completion.suggestions.is_empty() {
            self.last_popup_lines = 0;
            return Ok(());
        }

        let total = self.completion.suggestions.len();
        let max_visible = total.min(MAX_VISIBLE);

        // Le popup commence sous la dernière ligne d'input
        let popup_base_y = self.viewport_y + self.input.line_count() as u16;

        // Scroll offset
        let scroll_offset = if self.completion.selected >= max_visible {
            self.completion.selected - max_visible + 1
        } else {
            0
        };

        let popup_width = 42usize;

        for i in 0..max_visible {
            let idx = scroll_offset + i;
            if idx >= total {
                break;
            }

            let y = popup_base_y + i as u16;
            crossterm::queue!(stdout, cursor::MoveTo(2, y), ct::Clear(ct::ClearType::CurrentLine))?;

            let s = &self.completion.suggestions[idx];
            let cat = s.category;
            let cat_w = cat.len();
            let text_w = popup_width.saturating_sub(cat_w + 3);
            let text_display = if s.text.len() > text_w {
                &s.text[..text_w]
            } else {
                &s.text
            };

            if idx == self.completion.selected {
                write!(
                    stdout,
                    "{ANSI_SELECTED_BG}\x1b[1m {:<tw$} {ANSI_DIM}{:>cw$}{ANSI_RESET}",
                    text_display,
                    cat,
                    tw = text_w,
                    cw = cat_w
                )?;
            } else {
                write!(
                    stdout,
                    " {:<tw$} {ANSI_DIM}{:>cw$}{ANSI_RESET}",
                    text_display,
                    cat,
                    tw = text_w,
                    cw = cat_w
                )?;
            }
        }

        // Indicateur de scroll
        if total > max_visible {
            let y = popup_base_y + max_visible as u16;
            crossterm::queue!(stdout, cursor::MoveTo(2, y), ct::Clear(ct::ClearType::CurrentLine))?;
            write!(
                stdout,
                "{ANSI_DIM} ({}/{}){ANSI_RESET}",
                scroll_offset + max_visible,
                total
            )?;
            self.last_popup_lines = max_visible as u16 + 1;
        } else {
            self.last_popup_lines = max_visible as u16;
        }

        Ok(())
    }

    // -- Key handling --

    fn handle_key_event(&mut self, key: KeyEvent, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        // Config editor intercepts all keys when active
        if self.config_editor.active {
            return self.handle_config_editor_key(key, terminal);
        }

        // Search mode intercepts all keys when active
        if self.search.active {
            return self.handle_search_key(key, terminal);
        }

        match (key.modifiers, key.code) {
            // Ctrl+R : start reverse search
            (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
                self.search.active = true;
                self.search.query.clear();
                self.search.matches.clear();
                self.search.match_index = 0;
                self.completion.reset();
                self.current_hint = None;
            }

            // Ctrl+D : quit
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                if self.input.is_empty() {
                    self.exit_reason = Some(ExitReason::Ok);
                }
            }

            // Ctrl+C : cancel input, or abort if empty
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                if self.input.is_empty() {
                    self.exit_reason = Some(ExitReason::Abort);
                } else {
                    self.completion.reset();
                    self.current_hint = None;
                    self.input.clear();
                    self.print_output(terminal, "^C");
                }
            }

            // Ctrl+L : clear screen
            (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                self.clear_screen(terminal)?;
            }

            // Ctrl+U : clear line
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.input.clear_line();
                self.completion.reset();
                self.current_hint = None;
            }

            // Ctrl+W : delete word before
            (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                self.input.delete_word_before();
                self.update_completion();
                self.update_hint();
            }

            // Ctrl+A : home
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                self.input.move_cursor_home();
            }

            // Ctrl+E : end
            (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
                self.input.move_cursor_end();
            }

            // Ctrl+Left : word left
            (KeyModifiers::CONTROL, KeyCode::Left) => {
                self.input.move_cursor_word_left();
            }

            // Ctrl+Right : word right
            (KeyModifiers::CONTROL, KeyCode::Right) => {
                self.input.move_cursor_word_right();
            }

            // Escape : close popup
            (_, KeyCode::Esc) => {
                self.completion.reset();
                self.update_hint();
            }

            // Tab : trigger or accept completion
            (_, KeyCode::Tab) => {
                if self.completion.active {
                    self.accept_completion();
                } else {
                    self.trigger_completion();
                }
            }

            // BackTab (Shift+Tab) : previous completion
            (KeyModifiers::SHIFT, KeyCode::BackTab) | (_, KeyCode::BackTab) => {
                if self.completion.active {
                    self.completion.select_prev();
                }
            }

            // Up : history or completion
            (_, KeyCode::Up) => {
                if self.completion.active {
                    self.completion.select_prev();
                } else {
                    self.input.history_up(&self.history);
                    self.update_hint();
                }
            }

            // Down : history or completion
            (_, KeyCode::Down) => {
                if self.completion.active {
                    self.completion.select_next();
                } else {
                    self.input.history_down(&self.history);
                    self.update_hint();
                }
            }

            // Enter : accept completion, or submit/multiline
            (_, KeyCode::Enter) => {
                if self.completion.active {
                    self.accept_completion();
                    self.update_hint();
                } else {
                    self.current_hint = None;
                    let text = self.input.full_text();
                    if should_continue_multiline(&text) {
                        let indent = catnip_tools::indentation::compute_next_indent(&text, 4);
                        self.input.new_line_with_indent(indent);
                        self.update_hint();
                    } else {
                        self.submit_input(terminal)?;
                    }
                }
            }

            // Left/Right
            (_, KeyCode::Left) => {
                self.input.move_cursor_left();
                self.update_hint();
            }
            (_, KeyCode::Right) => {
                // Accept hint if cursor at end of line and hint present
                if self.current_hint.is_some() && self.cursor_at_line_end() {
                    self.accept_hint();
                } else {
                    self.input.move_cursor_right();
                    self.update_hint();
                }
            }
            (_, KeyCode::Home) => {
                self.input.move_cursor_home();
                self.update_hint();
            }
            (_, KeyCode::End) => {
                self.input.move_cursor_end();
                self.update_hint();
            }

            // Backspace
            (_, KeyCode::Backspace) => {
                self.input.delete_char_before();
                self.update_completion();
                self.update_hint();
            }

            // Delete
            (_, KeyCode::Delete) => {
                self.input.delete_char_at();
                self.update_completion();
                self.update_hint();
            }

            // Regular char
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
                self.input.insert_char(ch);
                self.update_completion();
                self.update_hint();
            }

            _ => {}
        }

        Ok(())
    }

    fn handle_paste_event(&mut self, text: String) {
        self.completion.reset();
        self.current_hint = None;

        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");

        for ch in normalized.chars() {
            match ch {
                '\n' => self.input.new_line(),
                _ => self.input.insert_char(ch),
            }
        }

        self.update_hint();
    }

    fn clear_screen(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        let mut stdout = io::stdout();
        crossterm::execute!(
            stdout,
            SetAttribute(Attribute::Reset),
            ResetColor,
            ct::Clear(ct::ClearType::All),
            cursor::MoveTo(0, 0)
        )?;
        stdout.flush()?;

        // Reset render bookkeeping after full screen wipe.
        self.last_popup_lines = 0;
        self.last_continuation_lines = 0;
        self.last_config_editor_lines = 0;
        self.viewport_y = 0;
        self.completion.reset();
        self.config_editor.reset();

        // Keep ratatui internal buffers in sync with terminal state.
        terminal.clear()?;

        Ok(())
    }

    // -- Completion --

    fn trigger_completion(&mut self) {
        self.current_hint = None; // exclusion mutuelle
        let line = self.input.current_line().to_string();
        let col = self.input.cursor().1;
        let suggestions = self.completer.complete(&line, col);

        if suggestions.is_empty() {
            self.completion.reset();
        } else {
            self.completion.suggestions = suggestions;
            self.completion.selected = 0;
            self.completion.active = true;
        }
    }

    fn update_completion(&mut self) {
        if self.completion.active {
            self.trigger_completion();
        }
    }

    fn accept_completion(&mut self) {
        if let Some(suggestion) = self.completion.current() {
            let text = suggestion.text.clone();
            let start = suggestion.replace_start;
            let end = suggestion.replace_end;

            // Remplacer dans la ligne courante
            let (row, _) = self.input.cursor();
            let line = &mut self.input.lines_mut()[row];
            line.replace_range(start..end, &text);

            // Mettre a jour le curseur
            let new_col = start + text.len();
            self.input.set_cursor_col(new_col);
        }
        self.completion.reset();
    }

    // -- Hints (ghost text) --

    /// Recompute ghost text based on current state
    fn update_hint(&mut self) {
        if self.completion.active {
            self.current_hint = None;
            return;
        }
        if !self.cursor_at_line_end() {
            self.current_hint = None;
            return;
        }
        let line = self.input.current_line().to_string();
        let col = self.input.cursor().1;
        self.current_hint = self.hints.get_hint(&line, col);
    }

    /// Insert ghost text into input and clear the hint
    fn accept_hint(&mut self) {
        if let Some(hint) = self.current_hint.take() {
            let (row, _) = self.input.cursor();
            self.input.lines_mut()[row].push_str(&hint);
            let new_col = self.input.lines()[row].len();
            self.input.set_cursor_col(new_col);
        }
    }

    /// Check if the cursor is at the end of the current line
    fn cursor_at_line_end(&self) -> bool {
        let (row, col) = self.input.cursor();
        col >= self.input.lines()[row].len()
    }

    // -- Submit --

    fn submit_input(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        // Clear continuation lines before insert_before (they live outside ratatui)
        if self.last_continuation_lines > 0 {
            let mut stdout = io::stdout();
            for i in 0..self.last_continuation_lines {
                let y = self.viewport_y + 1 + i;
                crossterm::queue!(stdout, cursor::MoveTo(0, y), ct::Clear(ct::ClearType::CurrentLine))?;
            }
            stdout.flush()?;
            self.last_continuation_lines = 0;
        }

        let text = self.input.full_text();
        let trimmed = text.trim().to_string();
        self.input.clear();

        if trimmed.is_empty() {
            return Ok(());
        }

        // Add to history
        self.history.push(&trimmed);

        // Print the input above (echo) with syntax highlighting
        let prompt_style = Style::default().fg(self.config.color_prompt);
        let echo_lines: Vec<Line> = trimmed
            .lines()
            .enumerate()
            .map(|(i, l)| {
                let prompt = if i == 0 {
                    &self.config.prompt_main
                } else {
                    &self.config.prompt_continuation
                };
                let mut spans = vec![Span::styled(prompt.as_str(), prompt_style)];
                if let Some(ref hl) = self.highlighter {
                    spans.extend(hl.highlight_line(l));
                } else {
                    spans.push(Span::raw(l.to_string()));
                }
                Line::from(spans)
            })
            .collect();
        self.print_lines(terminal, echo_lines);

        // Command handling
        if trimmed.starts_with('/') {
            let should_exit = self.handle_command(&trimmed, terminal)?;
            if should_exit {
                self.exit_reason = Some(ExitReason::Ok);
            }
            return Ok(());
        }

        // Preprocess multiline
        let code = preprocess_multiline(&trimmed);

        // Execute
        self.execute_line(&code, terminal);

        // Update completer and hint engine variables
        let vars = self.executor.get_variable_names();
        self.completer.set_variables(vars.clone());
        self.hints.set_variables(vars);
        let attrs = self.executor.get_variable_attrs();
        self.completer.set_attrs(attrs);

        Ok(())
    }

    // -- Execution --

    fn execute_line(&mut self, code: &str, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
        let total_start = Instant::now();

        if self.config.debug_mode {
            match self.executor.debug_pipeline(code) {
                Ok(output) => self.print_output(terminal, &output),
                Err(e) => self.print_error(terminal, &e),
            }
            return;
        }

        // Enable SIGINT during execution so Ctrl+C can interrupt the VM.
        let flag = self.executor.interrupt_flag();
        let _guard = crate::signal::SigintGuard::new(flag);

        // Capture stdout during execution so print() output goes through
        // ratatui's insert_before instead of writing directly to the terminal
        // (which would desync the viewport position).
        let captured = capture_stdout(|| self.executor.execute(code));
        let stdout_output = captured.output;
        let result = captured.result;

        // Guard dropped here, SIGINT handler restored
        drop(_guard);

        // Display captured print() output via ratatui
        if !stdout_output.is_empty() {
            let text = stdout_output.trim_end_matches('\n');
            if !text.is_empty() {
                self.print_output(terminal, text);
            }
        }

        match result {
            Ok((text, kind)) => {
                let total_time = total_start.elapsed();
                if !text.is_empty() {
                    self.print_result(terminal, &text, kind);
                }

                if self.config.show_exec_time {
                    self.print_output(terminal, &format!("Execution time: {:?}", total_time));
                }
            }
            Err(e) => {
                // exit(N) -> quit REPL
                if e.starts_with("exit(") {
                    self.exit_reason = Some(ExitReason::Ok);
                    return;
                }
                self.print_error(terminal, &e);
            }
        }
    }

    // -- Commands --

    fn handle_command(&mut self, command: &str, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<bool> {
        let parts: Vec<&str> = command[1..].split_whitespace().collect();
        if parts.is_empty() {
            return Ok(false);
        }

        match parts[0] {
            "help" | "h" => {
                self.print_dim(terminal, &generate_help_text());
            }
            "exit" | "quit" | "q" => {
                return Ok(true);
            }
            "clear" | "cls" => {
                let mut stdout = io::stdout();
                write!(stdout, "\x1B[2J\x1B[1;1H")?;
                stdout.flush()?;
            }
            "version" | "v" => {
                self.print_dim(terminal, &version_info());
            }
            "stats" => {
                let var_count = self.executor.get_variable_names().len();
                let stats = format!(
                    "=== Execution Statistics ===\n\
                     Variables defined: {}\n\
                     JIT enabled:       {}\n\
                     JIT threshold:     {}",
                    var_count,
                    if self.config.enable_jit { "yes" } else { "no" },
                    self.config.jit_threshold
                );
                self.print_output(terminal, &stats);
            }
            "jit" => {
                self.config.enable_jit = !self.config.enable_jit;
                let msg = format!(
                    "JIT compiler: {}",
                    if self.config.enable_jit { "enabled" } else { "disabled" }
                );
                self.print_output(terminal, &msg);
            }
            "verbose" => {
                self.config.show_parse_time = !self.config.show_parse_time;
                self.config.show_exec_time = !self.config.show_exec_time;
                let msg = format!(
                    "Verbose mode: {}",
                    if self.config.show_parse_time {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                self.print_output(terminal, &msg);
            }
            "debug" => {
                self.config.debug_mode = !self.config.debug_mode;
                let msg = format!(
                    "Debug mode: {} (shows IR and bytecode)",
                    if self.config.debug_mode { "enabled" } else { "disabled" }
                );
                self.print_output(terminal, &msg);
            }
            "history" => {
                let entries = self.history.entries();
                if entries.is_empty() {
                    self.print_output(terminal, NO_HISTORY_YET);
                } else {
                    let total = entries.len();
                    let start = total.saturating_sub(20);
                    let mut output = String::from("=== Command History ===\n");
                    for (i, entry) in entries.iter().enumerate().skip(start) {
                        output.push_str(&format!("  {:3}: {}\n", i + 1, entry));
                    }
                    output.push_str(&format!("\nTotal: {} entries", total));
                    self.print_output(terminal, &output);
                }
            }
            "load" => {
                if parts.len() < 2 {
                    self.print_error(terminal, USAGE_LOAD);
                } else {
                    self.load_and_execute(parts[1], terminal);
                }
            }
            "time" => {
                if parts.len() < 2 {
                    self.print_error(terminal, USAGE_TIME);
                } else {
                    let expression = command[6..].trim().to_string();
                    self.benchmark_expression(&expression, terminal);
                }
            }
            "context" | "ctx" => {
                if parts.len() >= 2 {
                    let detail = self.executor.get_variable_detail(parts[1]);
                    match detail {
                        Some(text) => self.print_output(terminal, &text),
                        None => self.print_error(terminal, &Self::format_variable_not_found(parts[1])),
                    }
                } else {
                    let entries = self.executor.get_context_display();
                    if entries.is_empty() {
                        self.print_output(terminal, NO_USER_VARIABLES_DEFINED);
                    } else {
                        let mut out = String::from("=== Context ===\n");
                        for (name, typ, repr) in &entries {
                            out.push_str(&format!("  {:<16} {:<12} {}\n", name, typ, repr));
                        }
                        self.print_output(terminal, out.trim_end());
                    }
                }
            }
            "config" => {
                self.handle_config_command(&parts[1..], terminal);
            }
            _ => {
                self.print_error(terminal, &Self::format_unknown_command(parts[0]));
                self.print_output(terminal, TYPE_HELP_FOR_COMMANDS);
            }
        }

        Ok(false)
    }

    /// Create a ConfigManager via Python, with file + env loaded.
    fn make_config_manager<'py>(&self, py: pyo3::Python<'py>) -> pyo3::PyResult<pyo3::Bound<'py, pyo3::PyAny>> {
        use pyo3::prelude::*;
        let rs = py.import(pyo3::intern!(py, "catnip._rs"))?;
        let cm = rs.getattr("ConfigManager")?.call0()?;
        cm.call_method0("load_file")?;
        cm.call_method0("load_env")?;
        Ok(cm)
    }

    /// Parse a raw config value string into the appropriate Python type.
    fn parse_config_value(&self, py: pyo3::Python<'_>, raw: &str) -> pyo3::Py<pyo3::PyAny> {
        use pyo3::prelude::*;
        use pyo3::types::PyBool;
        match raw {
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
        }
    }

    fn handle_config_command(&mut self, args: &[&str], terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
        use pyo3::prelude::*;

        match args.first().copied() {
            None => {
                // Open interactive config editor
                self.open_config_editor(terminal);
            }
            Some("show") => {
                Python::attach(|py| {
                    let cm = match self.make_config_manager(py) {
                        Ok(cm) => cm,
                        Err(e) => {
                            self.print_error(terminal, &format!("{}", e));
                            return;
                        }
                    };
                    let lines: Vec<String> = match cm.call_method0("debug_report").and_then(|r| r.extract()) {
                        Ok(l) => l,
                        Err(e) => {
                            self.print_error(terminal, &format!("{}", e));
                            return;
                        }
                    };

                    let path = catnip_rs::config::get_config_path();
                    let mut output = format!("# {}\n", path.display());
                    for line in &lines {
                        output.push_str(line);
                        output.push('\n');
                    }
                    self.print_output(terminal, output.trim_end());
                });
            }
            Some("get") => {
                if args.len() < 2 {
                    self.print_error(terminal, USAGE_CONFIG_GET);
                    return;
                }
                let key = args[1];
                Python::attach(|py| {
                    let cm = match self.make_config_manager(py) {
                        Ok(cm) => cm,
                        Err(e) => {
                            self.print_error(terminal, &format!("{}", e));
                            return;
                        }
                    };
                    match cm.call_method1("get", (key,)) {
                        Ok(val) => {
                            let repr = val.repr().map(|r| r.to_string()).unwrap_or_else(|_| "?".to_string());
                            self.print_output(terminal, &format!("{}: {}", key, repr));
                        }
                        Err(e) => {
                            self.print_error(terminal, &format!("{}", e));
                        }
                    }
                });
            }
            Some("set") => {
                if args.len() < 3 {
                    self.print_error(terminal, USAGE_CONFIG_SET);
                    return;
                }
                let key = args[1];
                let raw_value = args[2..].join(" ");
                Python::attach(|py| {
                    let py_value = self.parse_config_value(py, &raw_value);
                    match py
                        .import(pyo3::intern!(py, "catnip._rs"))
                        .and_then(|m| m.getattr("set_config_value"))
                        .and_then(|f| f.call1((key, py_value)))
                    {
                        Ok(_) => {
                            self.print_output(terminal, &Self::format_saved_assignment(key, &raw_value));
                        }
                        Err(e) => {
                            self.print_error(terminal, &format!("{}", e));
                        }
                    }
                });
            }
            Some("path") => {
                let path = catnip_rs::config::get_config_path();
                self.print_output(terminal, &path.to_string_lossy());
            }
            _ => {
                self.print_error(terminal, USAGE_CONFIG);
            }
        }
    }

    // -- Config editor --

    /// Load config data from Python and activate the editor overlay.
    fn open_config_editor(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
        use pyo3::prelude::*;

        Python::attach(|py| {
            let cm = match self.make_config_manager(py) {
                Ok(cm) => cm,
                Err(e) => {
                    self.print_error(terminal, &format!("{}", e));
                    return;
                }
            };

            // Extract config entries via debug_report
            let lines: Vec<String> = match cm.call_method0("debug_report").and_then(|r| r.extract()) {
                Ok(l) => l,
                Err(e) => {
                    self.print_error(terminal, &format!("{}", e));
                    return;
                }
            };

            let mut entries = Vec::new();

            for line in &lines {
                if line.starts_with("---") {
                    continue;
                }

                // Parse "key: value_repr  [source (detail)]" or "format.key: ..."
                let (raw_key, rest) = match line.split_once(": ") {
                    Some(pair) => pair,
                    None => continue,
                };

                let key = raw_key.strip_prefix("format.").unwrap_or(raw_key).to_string();

                // Split "value_repr  [source...]"
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

            // REPL-local settings
            let repl_entries = vec![
                ("show_parse_time".to_string(), self.config.show_parse_time.to_string()),
                ("show_exec_time".to_string(), self.config.show_exec_time.to_string()),
                ("debug_mode".to_string(), self.config.debug_mode.to_string()),
                ("max_history".to_string(), self.config.max_history.to_string()),
            ];

            self.config_editor.load(entries, repl_entries);

            // Set title with config file path
            let path: String = py
                .import(pyo3::intern!(py, "catnip.config"))
                .and_then(|m| m.getattr("get_config_path"))
                .and_then(|f| f.call0())
                .and_then(|p| p.str()?.extract())
                .unwrap_or_else(|_| "catnip.toml".to_string());
            self.config_editor.title = path;
        });
    }

    // -- Reverse search (Ctrl+R) --

    fn handle_search_key(
        &mut self,
        key: KeyEvent,
        _terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<()> {
        match (key.modifiers, key.code) {
            // Ctrl+R again: cycle to next match
            (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
                if !self.search.matches.is_empty() {
                    self.search.match_index = (self.search.match_index + 1) % self.search.matches.len();
                    self.apply_search_match();
                }
            }

            // Ctrl+C / Escape: cancel search, restore empty input
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                self.search.reset();
                self.input.clear();
            }

            // Enter: accept current match, exit search
            (_, KeyCode::Enter) => {
                self.search.reset();
                // Input already contains the matched entry
            }

            // Backspace: remove last char from query
            (_, KeyCode::Backspace) => {
                self.search.query.pop();
                self.update_search();
            }

            // Regular char: append to query
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
                self.search.query.push(ch);
                self.update_search();
            }

            // Any other key: accept match and replay the key as normal input
            _ => {
                self.search.reset();
                // Don't clear input -- keep the matched text
            }
        }
        Ok(())
    }

    fn update_search(&mut self) {
        self.search.matches = self.history.search(&self.search.query);
        self.search.match_index = 0;
        self.apply_search_match();
    }

    fn apply_search_match(&mut self) {
        if let Some(&hist_idx) = self.search.matches.get(self.search.match_index) {
            if let Some(entry) = self.history.get(hist_idx) {
                let lines: Vec<String> = entry.split('\n').map(|s| s.to_string()).collect();
                let last_row = lines.len() - 1;
                let last_col = lines[last_row].len();
                *self.input.lines_mut() = lines;
                self.input.set_cursor_col(last_col);
                // Set cursor row to last line
                self.input.set_cursor_row(last_row);
            }
        } else {
            self.input.clear();
        }
    }

    /// Handle key events while the config editor is active.
    fn handle_config_editor_key(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<()> {
        // Edit mode: intercept typing keys
        if self.config_editor.edit.is_some() {
            match (key.modifiers, key.code) {
                (_, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                    self.config_editor.cancel_edit();
                }
                (_, KeyCode::Enter) => {
                    if let Some(action) = self.config_editor.confirm_edit() {
                        self.apply_config_action(action, terminal);
                    }
                }
                (_, KeyCode::Backspace) => {
                    self.config_editor.edit_backspace();
                }
                (_, KeyCode::Left) => {
                    self.config_editor.edit_move_left();
                }
                (_, KeyCode::Right) => {
                    self.config_editor.edit_move_right();
                }
                (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
                    self.config_editor.edit_insert_char(ch);
                }
                _ => {}
            }
            return Ok(());
        }

        // Navigation mode
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) | (_, KeyCode::Char('q')) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.config_editor.reset();
            }
            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                self.config_editor.select_prev();
            }
            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                self.config_editor.select_next();
            }
            (_, KeyCode::Enter) | (_, KeyCode::Char(' ')) => {
                if let Some(action) = self.config_editor.toggle_or_enter_edit() {
                    self.apply_config_action(action, terminal);
                }
            }
            (_, KeyCode::Tab) => {
                self.config_editor.jump_next_group();
            }
            (KeyModifiers::SHIFT, KeyCode::BackTab) => {
                self.config_editor.jump_prev_group();
            }
            (_, KeyCode::Home) | (_, KeyCode::Char('g')) => {
                self.config_editor.select_first();
            }
            (KeyModifiers::SHIFT, KeyCode::Char('G')) | (_, KeyCode::End) => {
                self.config_editor.select_last();
            }
            (_, KeyCode::PageDown) => {
                self.config_editor.page_down(5);
            }
            (_, KeyCode::PageUp) => {
                self.config_editor.page_up(5);
            }
            (_, KeyCode::Char('r')) => {
                if let Some(action) = self.config_editor.reset_selected() {
                    self.apply_config_action(action, terminal);
                }
            }
            (_, KeyCode::Char('?')) => {
                self.config_editor.show_help = !self.config_editor.show_help;
            }
            _ => {}
        }

        Ok(())
    }

    /// Apply a config change via Python's set_config_value or REPL-local mutation.
    fn apply_config_action(&mut self, action: ConfigAction, _terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
        use crate::config_editor::StatusKind;
        use pyo3::prelude::*;

        match action {
            ConfigAction::SetValue { key, value, is_format } => {
                Python::attach(|py| {
                    let py_value = self.parse_config_value(py, &value);
                    let target_key = if is_format {
                        format!("format.{}", key)
                    } else {
                        key.clone()
                    };
                    match py
                        .import(pyo3::intern!(py, "catnip._rs"))
                        .and_then(|m| m.getattr("set_config_value"))
                        .and_then(|f| f.call1((&target_key, py_value)))
                    {
                        Ok(_) => {
                            self.config_editor.status_message =
                                Some((Self::format_saved_assignment(&key, &value), StatusKind::Success));
                        }
                        Err(e) => {
                            self.config_editor.status_message =
                                Some((Self::format_config_editor_error(&e), StatusKind::Error));
                        }
                    }
                });
            }
            ConfigAction::SetRepl { key, value } => {
                let ok = match key.as_str() {
                    "show_parse_time" => {
                        self.config.show_parse_time = value == "true";
                        true
                    }
                    "show_exec_time" => {
                        self.config.show_exec_time = value == "true";
                        true
                    }
                    "debug_mode" => {
                        self.config.debug_mode = value == "true";
                        true
                    }
                    "max_history" => {
                        if let Ok(n) = value.parse::<usize>() {
                            self.config.max_history = n;
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if ok {
                    self.config_editor.status_message = Some((format!("{} = {}", key, value), StatusKind::Success));
                } else {
                    self.config_editor.status_message = Some((Self::format_unknown_repl_key(&key), StatusKind::Error));
                }
            }
        }
    }

    /// Render the config editor overlay below input lines (queued, caller flushes).
    fn draw_config_editor(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        use crate::config_editor::{ConfigType, Row, StatusKind};

        if !self.config_editor.active || self.config_editor.total_items() == 0 {
            self.last_config_editor_lines = 0;
            return Ok(());
        }

        let base_y = self.viewport_y + self.input.line_count() as u16 + self.last_popup_lines;
        let (_, term_h) = ct::size()?;

        let rows = self.config_editor.rows();

        // Viewport: reserve 2 lines (status + help)
        let avail_h = term_h.saturating_sub(base_y).saturating_sub(2) as usize;
        let max_rows = avail_h.max(3).min(rows.len());
        self.config_editor.ensure_visible(max_rows + 2);

        let visible_start = self.config_editor.scroll_offset;
        let visible_end = (visible_start + max_rows).min(rows.len());
        let has_scroll_up = visible_start > 0;
        let has_scroll_down = visible_end < rows.len();

        let mut line_idx: u16 = 0;

        // Title with config file path
        if !self.config_editor.title.is_empty() {
            crossterm::queue!(
                stdout,
                cursor::MoveTo(0, base_y + line_idx),
                ct::Clear(ct::ClearType::CurrentLine)
            )?;
            write!(stdout, "  \x1b[90m{}\x1b[0m", self.config_editor.title)?;
            line_idx += 1;
        }

        // Visible rows
        for (ri, row) in rows.iter().enumerate().take(visible_end).skip(visible_start) {
            crossterm::queue!(
                stdout,
                cursor::MoveTo(0, base_y + line_idx),
                ct::Clear(ct::ClearType::CurrentLine)
            )?;

            match row {
                Row::GroupHeader(gi) => {
                    let label = crate::config_editor::GROUPS[*gi].group.label();
                    // Scroll indicators on first/last header
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
                    let item = &self.config_editor.items[*idx];
                    let is_selected = *idx == self.config_editor.selected;

                    let modified = if item.is_modified() { "*" } else { " " };
                    let marker = if is_selected { ">" } else { " " };

                    // Value display (type-aware)
                    let value_display = if is_selected {
                        if let Some(ref edit) = self.config_editor.edit {
                            format!("\x1b[1m{}\x1b[0m\x1b[90m\u{2502}\x1b[0m", edit.buffer)
                        } else {
                            self.format_config_value(item)
                        }
                    } else {
                        self.format_config_value(item)
                    };

                    // Source color
                    let source_str = self.format_config_source(&item.source);

                    // Range hint for selected int
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
            line_idx += 1;
        }

        // Status / help line
        crossterm::queue!(
            stdout,
            cursor::MoveTo(0, base_y + line_idx),
            ct::Clear(ct::ClearType::CurrentLine)
        )?;
        if let Some((ref msg, ref kind)) = self.config_editor.status_message {
            let color = match kind {
                StatusKind::Success => ANSI_STATUS_SUCCESS,
                StatusKind::Error => ANSI_STATUS_ERROR,
                StatusKind::Info => ANSI_STATUS_INFO,
            };
            write!(stdout, "  \x1b[{color}m{msg}{ANSI_RESET}")?;
        } else if self.config_editor.edit.is_some() {
            write!(stdout, "  {ANSI_DIM}Enter save  Esc cancel{ANSI_RESET}")?;
        } else {
            write!(
                stdout,
                "  {ANSI_DIM}Enter toggle  \u{2191}\u{2193} nav  Tab group  r reset  ? help  Esc close{ANSI_RESET}"
            )?;
        }
        line_idx += 1;

        // Help overlay
        if self.config_editor.show_help {
            let help_lines = [
                "\u{2191}\u{2193}/jk  navigate       Enter/Space  toggle/edit",
                "Tab   next group     Shift+Tab    prev group",
                "g     first          G            last",
                "r     reset default  ?            toggle help",
                "Esc/q close",
            ];
            for hl in &help_lines {
                crossterm::queue!(
                    stdout,
                    cursor::MoveTo(0, base_y + line_idx),
                    ct::Clear(ct::ClearType::CurrentLine)
                )?;
                write!(stdout, "  \x1b[90m{}\x1b[0m", hl)?;
                line_idx += 1;
            }
        }

        self.last_config_editor_lines = line_idx;
        Ok(())
    }

    /// Format a config value with type-aware ANSI styling.
    fn format_config_value(&self, item: &crate::config_editor::ConfigItem) -> String {
        use crate::config_editor::ConfigType;

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

    /// Format source tag with color coding.
    fn format_config_source(&self, source: &str) -> String {
        match source {
            "default" => "\x1b[90mdefault\x1b[0m".to_string(),
            "file" => format!("\x1b[0m{}\x1b[0m", source),
            "env" => format!("\x1b[36m{}\x1b[0m", source),
            "cli" => format!("\x1b[96;1m{}\x1b[0m", source),
            "session" => format!("\x1b[35m{}\x1b[0m", source),
            _ => format!("\x1b[90m{}\x1b[0m", source),
        }
    }

    fn load_and_execute(&mut self, filename: &str, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
        match std::fs::read_to_string(filename) {
            Ok(code) => {
                self.print_output(terminal, &Self::format_loading_file(filename));
                for (i, line) in code.lines().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    let _ = crossterm::terminal::disable_raw_mode();
                    let res = self.executor.execute(trimmed);
                    let _ = crossterm::terminal::enable_raw_mode();
                    match res {
                        Ok((text, kind)) => self.print_result(terminal, &text, kind),
                        Err(e) => {
                            self.print_error(terminal, &Self::format_file_line_error(i + 1, &e));
                            return;
                        }
                    }
                }
                self.print_output(terminal, FILE_LOADED_SUCCESSFULLY);
            }
            Err(e) => {
                self.print_error(terminal, &Self::format_file_read_error(filename, &e));
            }
        }
    }

    fn benchmark_expression(&mut self, expression: &str, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
        self.print_output(terminal, &Self::format_benchmarking(expression));

        // Warmup
        for _ in 0..10 {
            if self.executor.execute(expression).is_err() {
                self.print_error(terminal, EXPRESSION_FAILED_WARMUP);
                return;
            }
        }

        // Determine iterations
        let single_run = Instant::now();
        if self.executor.execute(expression).is_err() {
            self.print_error(terminal, EXPRESSION_FAILED);
            return;
        }
        let single_time = single_run.elapsed();

        let iterations = if single_time.as_micros() < 1000 {
            10000
        } else if single_time.as_millis() < 10 {
            1000
        } else if single_time.as_millis() < 100 {
            100
        } else {
            10
        };

        let start = Instant::now();
        for _ in 0..iterations {
            if self.executor.execute(expression).is_err() {
                self.print_error(terminal, EXPRESSION_FAILED_BENCHMARK);
                return;
            }
        }
        let total_time = start.elapsed();
        let avg_time = total_time / iterations;
        let ops_per_sec = if avg_time.as_nanos() > 0 {
            1_000_000_000.0 / avg_time.as_nanos() as f64
        } else {
            f64::INFINITY
        };

        let result = format!(
            "=== Benchmark Results ===\n\
             Iterations:     {}\n\
             Total time:     {:?}\n\
             Average time:   {:?}\n\
             Throughput:     {:.2} ops/sec",
            iterations,
            total_time,
            avg_time,
            if ops_per_sec.is_finite() { ops_per_sec } else { 0.0 }
        );
        self.print_output(terminal, &result);
    }

    // -- Output helpers --

    fn print_output(&self, terminal: &mut Terminal<CrosstermBackend<Stdout>>, text: &str) {
        let lines: Vec<Line> = text.lines().map(|l| Line::from(Span::raw(l.to_string()))).collect();
        self.print_lines(terminal, lines);
    }

    fn print_lines(&self, terminal: &mut Terminal<CrosstermBackend<Stdout>>, lines: Vec<Line<'_>>) {
        let count = lines.len() as u16;
        if count > 0 {
            let _ = terminal.insert_before(count, |buf| {
                for (i, line) in lines.into_iter().enumerate() {
                    if (i as u16) < buf.area.height {
                        let area = Rect::new(buf.area.x, buf.area.y + i as u16, buf.area.width, 1);
                        Widget::render(line, area, buf);
                    }
                }
            });
        }
    }

    fn result_style(&self, kind: ValueKind) -> Style {
        if let Some(ref hl) = self.highlighter {
            match kind {
                ValueKind::Int | ValueKind::Float => Style::default().fg(hl.number_color()),
                ValueKind::Bool => Style::default().fg(hl.constant_color()),
                ValueKind::String => Style::default().fg(hl.string_color()),
                _ => Style::default(),
            }
        } else {
            Style::default()
        }
    }

    fn print_result(&self, terminal: &mut Terminal<CrosstermBackend<Stdout>>, text: &str, kind: ValueKind) {
        let style = self.result_style(kind);
        let lines: Vec<Line> = text
            .lines()
            .map(|l| Line::from(Span::styled(l.to_string(), style)))
            .collect();
        let count = lines.len() as u16;
        if count > 0 {
            let _ = terminal.insert_before(count, |buf| {
                for (i, line) in lines.into_iter().enumerate() {
                    if (i as u16) < buf.area.height {
                        let area = Rect::new(buf.area.x, buf.area.y + i as u16, buf.area.width, 1);
                        Widget::render(line, area, buf);
                    }
                }
            });
        }
    }

    fn print_dim(&self, terminal: &mut Terminal<CrosstermBackend<Stdout>>, text: &str) {
        let dim_style = Style::default().fg(self.config.color_dim);
        let lines: Vec<Line> = text
            .lines()
            .map(|l| Line::from(Span::styled(l.to_string(), dim_style)))
            .collect();
        let count = lines.len() as u16;
        if count > 0 {
            let _ = terminal.insert_before(count, |buf| {
                for (i, line) in lines.into_iter().enumerate() {
                    if (i as u16) < buf.area.height {
                        let area = Rect::new(buf.area.x, buf.area.y + i as u16, buf.area.width, 1);
                        Widget::render(line, area, buf);
                    }
                }
            });
        }
    }

    fn print_error(&self, terminal: &mut Terminal<CrosstermBackend<Stdout>>, text: &str) {
        let error_style = Style::default().fg(self.config.color_error);
        let lines: Vec<Line> = text
            .lines()
            .map(|l| Line::from(Span::styled(format!("Error: {}", l), error_style)))
            .collect();
        let count = lines.len() as u16;
        if count > 0 {
            let _ = terminal.insert_before(count, |buf| {
                for (i, line) in lines.into_iter().enumerate() {
                    if (i as u16) < buf.area.height {
                        let area = Rect::new(buf.area.x, buf.area.y + i as u16, buf.area.width, 1);
                        Widget::render(line, area, buf);
                    }
                }
            });
        }
    }
}

// -- Helpers pour rendu ANSI hors viewport --

fn span_to_ansi(span: &Span) -> String {
    let style = span.style;
    let has_fg = style.fg.is_some();
    let has_bold = style.add_modifier.contains(Modifier::BOLD);

    if !has_fg && !has_bold {
        return span.content.to_string();
    }

    let mut out = String::new();
    if has_bold {
        out.push_str("\x1b[1m");
    }
    if let Some(Color::Rgb(r, g, b)) = style.fg {
        out.push_str(&format!("\x1b[38;2;{};{};{}m", r, g, b));
    }
    out.push_str(&span.content);
    out.push_str("\x1b[0m");
    out
}

fn color_to_ansi_fg(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("\x1b[38;2;{};{};{}m", r, g, b),
        _ => String::new(),
    }
}

fn get_history_path(config: &ReplConfig) -> PathBuf {
    let dir = catnip_rs::config::get_state_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join(&config.history_file)
}

// Multiline helpers: shared with debug console via catnip_tools
use catnip_tools::multiline::{preprocess_multiline, should_continue_multiline};

// -- Stdout capture --

struct CapturedExec<T> {
    output: String,
    result: T,
}

/// Redirect fd 1 to a pipe, run `f`, restore fd 1, return captured output.
fn capture_stdout<T, F: FnOnce() -> T>(f: F) -> CapturedExec<T> {
    use std::os::unix::io::{FromRawFd, RawFd};

    unsafe {
        // Save original stdout fd
        let saved_fd: RawFd = libc::dup(1);
        if saved_fd < 0 {
            // dup failed, run without capture
            let result = f();
            return CapturedExec {
                output: String::new(),
                result,
            };
        }

        // Create pipe
        let mut pipe_fds: [RawFd; 2] = [0; 2];
        if libc::pipe(pipe_fds.as_mut_ptr()) != 0 {
            libc::close(saved_fd);
            let result = f();
            return CapturedExec {
                output: String::new(),
                result,
            };
        }
        let (read_fd, write_fd) = (pipe_fds[0], pipe_fds[1]);

        // Redirect stdout to pipe write end
        libc::dup2(write_fd, 1);
        libc::close(write_fd);

        let result = f();

        // Flush Python's stdout buffer into the pipe
        // (Python buffers writes to fd 1)
        pyo3::Python::attach(|py| {
            let _ = py.run(pyo3::ffi::c_str!("import sys; sys.stdout.flush()"), None, None);
        });

        // Restore original stdout
        libc::dup2(saved_fd, 1);
        libc::close(saved_fd);

        // Read captured output (non-blocking)
        let mut captured = String::new();
        let mut file = std::fs::File::from_raw_fd(read_fd);
        use std::io::Read;
        // Set non-blocking to avoid hanging if pipe is empty
        libc::fcntl(read_fd, libc::F_SETFL, libc::O_NONBLOCK);
        let _ = file.read_to_string(&mut captured);

        CapturedExec {
            output: captured,
            result,
        }
    }
}
