// FILE: catnip_repl/src/app.rs
//! Main TUI loop with ratatui inline rendering.
//!
//! Output is pushed into the scrollback via insert_before(),
//! the inline viewport only contains the input (1 line).
//! The completion popup is rendered directly via crossterm,
//! outside the ratatui viewport (which does not support dynamic
//! resize for Viewport::Inline).

use crate::completer::{CatnipCompleter, CompletionState};
use crate::config::{version_info, ReplConfig, HELP_TEXT};
use crate::executor::{ReplExecutor, ValueKind};
use crate::highlighter::CatnipHighlighter;
use crate::hints::HintEngine;
use crate::history::History;
use crate::input::InputState;
use crate::widgets::completion::MAX_VISIBLE;

use crossterm::cursor;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
};
use crossterm::style::{Attribute, ResetColor, SetAttribute};
use crossterm::terminal::{self as ct};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use ratatui::Terminal;
use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

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
        if nanos % 100 == 0 {
            return rare[nanos / 100 % rare.len()];
        }
        let msgs = match self {
            ExitReason::Ok => constants::REPL_EXIT_OK,
            ExitReason::Abort => constants::REPL_EXIT_ABORT,
        };
        msgs[nanos % msgs.len()]
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
    /// Viewport Y position (line 0) in the terminal
    viewport_y: u16,
}

impl App {
    pub fn new(config: ReplConfig) -> Result<Self, String> {
        let executor = ReplExecutor::new()?;

        let history_path = get_history_path(&config);
        migrate_history(&history_path);
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
            viewport_y: 0,
        })
    }

    pub fn run(
        mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<ExitReason> {
        // Welcome message
        self.print_dim(terminal, &self.config.welcome_message.clone());

        let mut stdout = io::stdout();
        crossterm::execute!(stdout, EnableBracketedPaste)?;

        loop {
            // Track previous extra lines for cleanup
            let prev_extra = self.last_continuation_lines + self.last_popup_lines;

            // Hide cursor during render to prevent flicker
            crossterm::queue!(
                stdout,
                cursor::Hide,
                SetAttribute(Attribute::Reset),
                ResetColor
            )?;

            // Draw input line 0 via ratatui (viewport = 1 ligne)
            // viewport_y is set inside render_inline from f.area().y
            terminal.draw(|f| self.render_inline(f))?;

            // Scroller si pas assez de place pour continuation + popup
            self.ensure_space_below(terminal)?;
            // Continuation lines via crossterm (queued, not flushed)
            self.draw_continuation_lines(&mut stdout)?;
            // Popup via crossterm (queued, not flushed)
            self.draw_completion_popup(&mut stdout)?;

            // Clear excess lines from previous frame
            let curr_extra = self.last_continuation_lines + self.last_popup_lines;
            for i in curr_extra..prev_extra {
                let y = self.viewport_y + 1 + i;
                crossterm::queue!(
                    stdout,
                    cursor::MoveTo(0, y),
                    ct::Clear(ct::ClearType::CurrentLine)
                )?;
            }

            // Position cursor and show
            let (crow, ccol) = self.input.cursor();
            let prompt_len = if crow == 0 {
                self.config.prompt_main.chars().count()
            } else {
                self.config.prompt_continuation.chars().count()
            };
            let cursor_x = prompt_len as u16 + ccol as u16;
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
        let prompt_len = prompt.chars().count();
        if crow == 0 {
            let cursor_x = area.x + prompt_len as u16 + ccol as u16;
            f.set_cursor_position((cursor_x, area.y));
        } else {
            let cursor_x = area.x + prompt_len as u16 + line_text.len() as u16;
            f.set_cursor_position((cursor_x, area.y));
        }
    }

    // -- Scroll + continuation + popup (rendu via crossterm hors viewport) --

    /// Scroll the terminal if not enough room for continuation + popup
    fn ensure_space_below(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<()> {
        let line_count = self.input.line_count();
        let extra_lines = if line_count > 1 { line_count - 1 } else { 0 };

        let popup_needed = if self.completion.active && !self.completion.suggestions.is_empty() {
            let total = self.completion.suggestions.len();
            let max_visible = total.min(MAX_VISIBLE);
            max_visible + if total > max_visible { 1 } else { 0 }
        } else {
            0
        };

        let total_needed = extra_lines + popup_needed;
        if total_needed == 0 {
            return Ok(());
        }

        let (term_w, term_rows) = ct::size()?;
        let space_below = term_rows.saturating_sub(self.viewport_y).saturating_sub(1) as usize;

        if space_below < total_needed {
            let scroll = (total_needed - space_below) as u16;
            let mut stdout = io::stdout();
            for _ in 0..scroll {
                write!(stdout, "\n")?;
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
            crossterm::queue!(
                stdout,
                cursor::MoveTo(0, y),
                ct::Clear(ct::ClearType::CurrentLine)
            )?;

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
            crossterm::queue!(
                stdout,
                cursor::MoveTo(2, y),
                ct::Clear(ct::ClearType::CurrentLine)
            )?;

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
                    "\x1b[48;2;60;60;80m\x1b[1m {:<tw$} \x1b[90m{:>cw$} \x1b[0m",
                    text_display,
                    cat,
                    tw = text_w,
                    cw = cat_w
                )?;
            } else {
                write!(
                    stdout,
                    " {:<tw$} \x1b[90m{:>cw$}\x1b[0m",
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
            crossterm::queue!(
                stdout,
                cursor::MoveTo(2, y),
                ct::Clear(ct::ClearType::CurrentLine)
            )?;
            write!(
                stdout,
                "\x1b[90m ({}/{})\x1b[0m",
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

    fn handle_key_event(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<()> {
        match (key.modifiers, key.code) {
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
                        self.input.new_line();
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

        // Normalise universal newlines:
        // - Windows: \r\n
        // - old Mac / some terminals: \r
        // - Unix: \n
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");

        for ch in normalized.chars() {
            match ch {
                '\n' => self.input.new_line(),
                _ => self.input.insert_char(ch),
            }
        }

        self.update_hint();
    }

    fn clear_screen(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<()> {
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
        self.viewport_y = 0;
        self.completion.reset();

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

    fn submit_input(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<()> {
        let text = self.input.full_text();
        let trimmed = text.trim().to_string();
        self.input.clear();

        if trimmed.is_empty() {
            return Ok(());
        }

        // Add to history
        self.history.push(&trimmed);

        // Print the input above (echo)
        let echo = if trimmed.contains('\n') {
            // Multiline: show with continuation prompts
            trimmed
                .lines()
                .enumerate()
                .map(|(i, l)| {
                    if i == 0 {
                        format!("{}{}", self.config.prompt_main, l)
                    } else {
                        format!("{}{}", self.config.prompt_continuation, l)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            format!("{}{}", self.config.prompt_main, trimmed)
        };
        self.print_output(terminal, &echo);

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

        // Capture stdout during execution so print() output goes through
        // ratatui's insert_before instead of writing directly to the terminal
        // (which would desync the viewport position).
        let captured = capture_stdout(|| self.executor.execute(code));
        let stdout_output = captured.output;
        let result = captured.result;

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
                self.print_error(terminal, &e);
            }
        }
    }

    // -- Commands --

    fn handle_command(
        &mut self,
        command: &str,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<bool> {
        let parts: Vec<&str> = command[1..].split_whitespace().collect();
        if parts.is_empty() {
            return Ok(false);
        }

        match parts[0] {
            "help" | "h" => {
                self.print_dim(terminal, HELP_TEXT);
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
                    if self.config.enable_jit {
                        "enabled"
                    } else {
                        "disabled"
                    }
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
                    if self.config.debug_mode {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                self.print_output(terminal, &msg);
            }
            "history" => {
                let entries = self.history.entries();
                if entries.is_empty() {
                    self.print_output(terminal, "No history yet");
                } else {
                    let total = entries.len();
                    let start = if total > 20 { total - 20 } else { 0 };
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
                    self.print_error(terminal, "Usage: /load <file.cat>");
                } else {
                    self.load_and_execute(parts[1], terminal);
                }
            }
            "time" => {
                if parts.len() < 2 {
                    self.print_error(terminal, "Usage: /time <expression>");
                } else {
                    let expression = command[6..].trim().to_string();
                    self.benchmark_expression(&expression, terminal);
                }
            }
            _ => {
                self.print_error(terminal, &format!("Unknown command: /{}", parts[0]));
                self.print_output(terminal, "Type /help for available commands");
            }
        }

        Ok(false)
    }

    fn load_and_execute(
        &mut self,
        filename: &str,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) {
        match std::fs::read_to_string(filename) {
            Ok(code) => {
                self.print_output(terminal, &format!("Loading {}...", filename));
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
                            self.print_error(terminal, &format!("Line {}: {}", i + 1, e));
                            return;
                        }
                    }
                }
                self.print_output(terminal, "File loaded successfully");
            }
            Err(e) => {
                self.print_error(terminal, &format!("Failed to read {}: {}", filename, e));
            }
        }
    }

    fn benchmark_expression(
        &mut self,
        expression: &str,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) {
        self.print_output(terminal, &format!("Benchmarking: {}", expression));

        // Warmup
        for _ in 0..10 {
            if self.executor.execute(expression).is_err() {
                self.print_error(terminal, "Expression failed during warmup");
                return;
            }
        }

        // Determine iterations
        let single_run = Instant::now();
        if self.executor.execute(expression).is_err() {
            self.print_error(terminal, "Expression failed");
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
                self.print_error(terminal, "Expression failed during benchmark");
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
            if ops_per_sec.is_finite() {
                ops_per_sec
            } else {
                0.0
            }
        );
        self.print_output(terminal, &result);
    }

    // -- Output helpers --

    fn print_output(&self, terminal: &mut Terminal<CrosstermBackend<Stdout>>, text: &str) {
        let lines: Vec<Line> = text
            .lines()
            .map(|l| Line::from(Span::raw(l.to_string())))
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

    fn print_result(
        &self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        text: &str,
        kind: ValueKind,
    ) {
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

/// Migrate ~/.catnip_history to XDG location if needed.
fn migrate_history(new_path: &Path) {
    if new_path.exists() {
        return;
    }
    let old_path = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".catnip_history"));
    if let Some(old) = old_path.filter(|p| p.exists()) {
        let _ = std::fs::rename(&old, new_path);
    }
}

// -- Multiline helpers (inline, avoids cross-crate dep for internal use) --

/// Continuation operators
const CONTINUATION_OPS: &[&str] = &[
    "**", "//", "+", "-", "*", "/", "%", "<<", ">>", "&", "|", "^", "==", "!=", "<=", ">=", "<",
    ">", ",", "=",
];

/// Continuation keywords
const CONTINUATION_KEYWORDS: &[&str] = &["if", "elif", "else", "while", "for", "match"];

fn should_continue_multiline(text: &str) -> bool {
    let stripped = text.trim_end();

    let brace_count = text.matches('{').count() as i32 - text.matches('}').count() as i32;
    let paren_count = text.matches('(').count() as i32 - text.matches(')').count() as i32;
    let bracket_count = text.matches('[').count() as i32 - text.matches(']').count() as i32;

    if brace_count > 0 || paren_count > 0 || bracket_count > 0 {
        return true;
    }

    for op in CONTINUATION_OPS {
        if stripped.ends_with(op) {
            return true;
        }
    }

    if let Some(last_word) = stripped.split_whitespace().last() {
        if CONTINUATION_KEYWORDS.contains(&last_word) {
            return true;
        }
    }

    false
}

fn has_continuation_op(line: &str) -> bool {
    let stripped = line.trim_end();
    CONTINUATION_OPS.iter().any(|op| stripped.ends_with(op))
}

fn preprocess_multiline(code: &str) -> String {
    let lines: Vec<&str> = code.split('\n').collect();
    let n_lines = lines.len();

    if n_lines == 1 {
        return code.to_string();
    }

    let mut processed = Vec::new();
    let mut i = 0;

    while i < n_lines {
        let line = lines[i];
        let stripped = line.trim_end();

        if has_continuation_op(stripped) {
            let mut accumulated = vec![stripped.to_string()];
            let mut j = i + 1;

            while j < n_lines {
                let next_line = lines[j].trim_start();
                accumulated.push(next_line.to_string());

                if has_continuation_op(next_line) {
                    j += 1;
                } else {
                    j += 1;
                    break;
                }
            }

            processed.push(accumulated.join(" "));
            i = j;
        } else {
            processed.push(line.to_string());
            i += 1;
        }
    }

    processed.join("\n")
}

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
        let mut pipe_fds: [RawFd; 2] = [0; 0 + 2];
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
            let _ = py.run(
                pyo3::ffi::c_str!("import sys; sys.stdout.flush()"),
                None,
                None,
            );
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
