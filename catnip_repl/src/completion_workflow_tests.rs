// FILE: catnip_repl/src/completion_workflow_tests.rs
//! Interactive autocomplete workflow tests.
//!
//! Simulates the App coordination logic (trigger, navigate, accept,
//! hint/completion mutual exclusion) using the real components
//! without TUI or Python dependencies.

use crate::completer::{CatnipCompleter, CompletionState};
use crate::hints::HintEngine;
use crate::input::InputState;

/// Minimal harness replicating App's completion/hint coordination.
struct Harness {
    input: InputState,
    completer: CatnipCompleter,
    completion: CompletionState,
    hints: HintEngine,
    current_hint: Option<String>,
}

impl Harness {
    fn new() -> Self {
        Self {
            input: InputState::new(),
            completer: CatnipCompleter::new(),
            completion: CompletionState::new(),
            hints: HintEngine::new(),
            current_hint: None,
        }
    }

    /// Type a string character by character, updating hints after each
    fn type_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.input.insert_char(ch);
            self.update_hint();
        }
    }

    /// Backspace once
    fn backspace(&mut self) {
        self.input.delete_char_before();
        if self.completion.active {
            self.trigger_completion();
        }
        self.update_hint();
    }

    // -- Replicates App::trigger_completion --
    fn trigger_completion(&mut self) {
        self.current_hint = None;
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

    // -- Replicates App::update_completion --
    fn update_completion(&mut self) {
        if self.completion.active {
            self.trigger_completion();
        }
    }

    // -- Replicates App::accept_completion --
    fn accept_completion(&mut self) {
        if let Some(suggestion) = self.completion.current() {
            let text = suggestion.text.clone();
            let start = suggestion.replace_start;
            let end = suggestion.replace_end;
            let (row, _) = self.input.cursor();
            let line = &mut self.input.lines_mut()[row];
            line.replace_range(start..end, &text);
            let new_col = start + text.len();
            self.input.set_cursor_col(new_col);
        }
        self.completion.reset();
    }

    // -- Replicates App::update_hint --
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

    // -- Replicates App::accept_hint --
    fn accept_hint(&mut self) {
        if let Some(hint) = self.current_hint.take() {
            let (row, _) = self.input.cursor();
            self.input.lines_mut()[row].push_str(&hint);
            let new_col = self.input.lines()[row].len();
            self.input.set_cursor_col(new_col);
        }
    }

    fn cursor_at_line_end(&self) -> bool {
        let (row, col) = self.input.cursor();
        col == self.input.lines()[row].len()
    }

    /// Simulate Tab keypress (trigger or accept)
    fn tab(&mut self) {
        if self.completion.active {
            self.accept_completion();
        } else {
            self.trigger_completion();
        }
    }

    /// Simulate Enter on completion (accept without submit)
    fn enter_completion(&mut self) {
        if self.completion.active {
            self.accept_completion();
            self.update_hint();
        }
    }

    /// Simulate Right arrow (accept hint or move cursor)
    fn right(&mut self) {
        if self.current_hint.is_some() && self.cursor_at_line_end() {
            self.accept_hint();
        } else {
            self.input.move_cursor_right();
            self.update_hint();
        }
    }
}

// ---- Tab trigger + accept ----

#[test]
fn test_tab_triggers_completion() {
    let mut h = Harness::new();
    h.type_str("sor");
    h.tab();
    assert!(h.completion.active);
    let texts: Vec<_> = h.completion.suggestions.iter().map(|s| s.text.as_str()).collect();
    assert!(texts.contains(&"sorted"));
}

#[test]
fn test_tab_accept_inserts_text() {
    let mut h = Harness::new();
    h.type_str("sor");
    h.tab(); // trigger
    // "sorted" should be in suggestions
    assert!(h.completion.active);
    h.tab(); // accept
    assert!(!h.completion.active);
    assert_eq!(h.input.current_line(), "sorted");
}

#[test]
fn test_enter_accepts_completion() {
    let mut h = Harness::new();
    h.type_str("sor");
    h.tab(); // trigger
    h.enter_completion();
    assert!(!h.completion.active);
    assert_eq!(h.input.current_line(), "sorted");
}

// ---- Navigation ----

#[test]
fn test_up_down_navigate_suggestions() {
    let mut h = Harness::new();
    h.type_str("re");
    h.tab();
    assert!(h.completion.active);
    assert!(
        h.completion.suggestions.len() >= 2,
        "need at least 2 suggestions for re"
    );
    let first = h.completion.current().unwrap().text.clone();
    h.completion.select_next();
    let second = h.completion.current().unwrap().text.clone();
    assert_ne!(first, second);
    h.completion.select_prev();
    assert_eq!(h.completion.current().unwrap().text, first);
}

#[test]
fn test_navigate_then_accept() {
    let mut h = Harness::new();
    h.type_str("re");
    h.tab();
    assert!(h.completion.suggestions.len() >= 2);
    h.completion.select_next(); // move to second suggestion
    let selected = h.completion.current().unwrap().text.clone();
    h.tab(); // accept
    assert_eq!(h.input.current_line(), selected);
}

// ---- Typing updates completions ----

#[test]
fn test_typing_filters_active_completion() {
    let mut h = Harness::new();
    h.type_str("re");
    h.tab();
    let count_re = h.completion.suggestions.len();
    assert!(count_re >= 2); // repr, reduce, reversed, ...

    // Type one more char while completion is active
    h.input.insert_char('p');
    h.update_completion();
    let count_rep = h.completion.suggestions.len();
    assert!(count_rep <= count_re);
    assert!(count_rep >= 1); // at least "repr"
}

#[test]
fn test_backspace_updates_completion() {
    let mut h = Harness::new();
    h.type_str("sort");
    h.tab();
    let count_sort = h.completion.suggestions.len();
    h.backspace(); // now "sor"
    let count_sor = h.completion.suggestions.len();
    assert!(count_sor >= count_sort);
}

// ---- Empty and edge cases ----

#[test]
fn test_tab_on_empty_input_no_completion() {
    let mut h = Harness::new();
    h.tab();
    assert!(!h.completion.active);
}

#[test]
fn test_no_match_resets_completion() {
    let mut h = Harness::new();
    h.type_str("zzzzz");
    h.tab();
    assert!(!h.completion.active);
}

// ---- Command completion ----

#[test]
fn test_slash_command_completion() {
    let mut h = Harness::new();
    h.type_str("/he");
    h.tab();
    assert!(h.completion.active);
    assert_eq!(h.completion.suggestions[0].text, "/help");
    h.tab(); // accept
    assert_eq!(h.input.current_line(), "/help");
}

// ---- Hint/completion mutual exclusion ----

#[test]
fn test_hint_clears_when_completion_active() {
    let mut h = Harness::new();
    h.type_str("sor");
    assert!(h.current_hint.is_some(), "hint should show for 'sor'");
    h.tab(); // trigger completion
    assert!(h.completion.active);
    assert!(h.current_hint.is_none(), "hint must be None when completion is active");
}

#[test]
fn test_hint_returns_after_completion_dismiss() {
    let mut h = Harness::new();
    h.type_str("sor");
    h.tab(); // trigger
    h.tab(); // accept -> inserts "sorted"
    h.update_hint();
    // After accepting "sorted", no more hint (exact match)
    assert!(h.current_hint.is_none());
}

// ---- Hint acceptance ----

#[test]
fn test_right_arrow_accepts_hint() {
    let mut h = Harness::new();
    h.type_str("sor");
    assert!(h.current_hint.is_some());
    h.right(); // accept hint
    assert!(h.current_hint.is_none());
    // "sor" + hint suffix "ted(iterable, key, reverse)" = "sorted(iterable, key, reverse)"
    assert_eq!(h.input.current_line(), "sorted(iterable, key, reverse)");
}

#[test]
fn test_hint_only_at_line_end() {
    let mut h = Harness::new();
    h.type_str("sor");
    assert!(h.current_hint.is_some());
    h.input.move_cursor_left();
    h.update_hint();
    assert!(h.current_hint.is_none(), "hint should disappear when cursor not at end");
}

// ---- Variable completion ----

#[test]
fn test_variable_completion_workflow() {
    let mut h = Harness::new();
    h.completer
        .set_variables(vec!["fibonacci".to_string(), "factor".to_string()]);
    h.hints
        .set_variables(vec!["fibonacci".to_string(), "factor".to_string()]);

    h.type_str("fib");
    // Hint should suggest variable
    assert_eq!(h.current_hint, Some("onacci".to_string()));

    h.tab(); // trigger completion
    assert!(h.completion.active);
    let texts: Vec<_> = h.completion.suggestions.iter().map(|s| s.text.as_str()).collect();
    assert!(texts.contains(&"fibonacci"));
    h.tab(); // accept
    assert_eq!(h.input.current_line(), "fibonacci");
}

// ---- Attribute completion ----

#[test]
fn test_dot_completion_workflow() {
    let mut h = Harness::new();
    let mut attrs = std::collections::HashMap::new();
    attrs.insert("io".to_string(), vec!["BytesIO".to_string(), "StringIO".to_string()]);
    h.completer.set_attrs(attrs);

    h.type_str("io.");
    h.tab();
    assert!(h.completion.active);
    let texts: Vec<_> = h.completion.suggestions.iter().map(|s| s.text.as_str()).collect();
    assert!(texts.contains(&"BytesIO"));
    assert!(texts.contains(&"StringIO"));
}

#[test]
fn test_dot_completion_with_prefix() {
    let mut h = Harness::new();
    let mut attrs = std::collections::HashMap::new();
    attrs.insert("io".to_string(), vec!["BytesIO".to_string(), "StringIO".to_string()]);
    h.completer.set_attrs(attrs);

    h.type_str("io.B");
    h.tab();
    assert!(h.completion.active);
    assert_eq!(h.completion.suggestions.len(), 1);
    h.tab(); // accept
    assert_eq!(h.input.current_line(), "io.BytesIO");
}

// ---- Replace range correctness ----

#[test]
fn test_completion_replaces_prefix_not_whole_line() {
    let mut h = Harness::new();
    h.type_str("x = sor");
    h.tab();
    assert!(h.completion.active);
    h.tab(); // accept "sorted"
    assert_eq!(h.input.current_line(), "x = sorted");
}

#[test]
fn test_cursor_position_after_accept() {
    let mut h = Harness::new();
    h.type_str("sor");
    h.tab(); // trigger
    h.tab(); // accept "sorted"
    assert_eq!(h.input.cursor(), (0, 6)); // cursor after "sorted"
}

// ---- Multiple completions in sequence ----

#[test]
fn test_successive_completions() {
    let mut h = Harness::new();
    // First completion
    h.type_str("sor");
    h.tab();
    h.tab(); // accept "sorted"
    assert_eq!(h.input.current_line(), "sorted");

    // Continue typing
    h.type_str("(le");
    h.tab();
    assert!(h.completion.active);
    let texts: Vec<_> = h.completion.suggestions.iter().map(|s| s.text.as_str()).collect();
    assert!(texts.contains(&"len"));
    h.tab(); // accept "len"
    assert_eq!(h.input.current_line(), "sorted(len");
}

// ---- Wrap-around navigation ----

#[test]
fn test_navigation_wraps_around() {
    let mut h = Harness::new();
    h.type_str("re");
    h.tab();
    let n = h.completion.suggestions.len();
    assert!(n >= 2);

    // Go forward n times to wrap around
    for _ in 0..n {
        h.completion.select_next();
    }
    assert_eq!(h.completion.selected, 0); // wrapped back to start

    // Go backward from 0
    h.completion.select_prev();
    assert_eq!(h.completion.selected, n - 1); // wrapped to last
}
