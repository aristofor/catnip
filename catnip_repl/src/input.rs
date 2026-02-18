// FILE: catnip_repl/src/input.rs
//! Line editor state - zero terminal knowledge.
//!
//! Manages the text buffer, cursor and history navigation.

/// Line editor state
pub struct InputState {
    /// Line buffer (single or multiline)
    lines: Vec<String>,
    /// Cursor position (line, column)
    cursor: (usize, usize),
    /// History index (None = current input)
    history_index: Option<usize>,
    /// Saved input when browsing history
    saved_input: Vec<String>,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: (0, 0),
            history_index: None,
            saved_input: Vec::new(),
        }
    }

    // -- Accessors --

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn cursor(&self) -> (usize, usize) {
        self.cursor
    }

    pub fn current_line(&self) -> &str {
        &self.lines[self.cursor.0]
    }

    /// Full text (lines joined by \n)
    pub fn full_text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    // -- Character operations --

    pub fn insert_char(&mut self, ch: char) {
        let (row, col) = self.cursor;
        self.lines[row].insert(col, ch);
        self.cursor.1 += ch.len_utf8();
    }

    pub fn delete_char_before(&mut self) {
        let (row, col) = self.cursor;
        if col > 0 {
            // Supprimer le caractere juste avant le curseur
            let prev = prev_char_boundary(&self.lines[row], col);
            self.lines[row].drain(prev..col);
            self.cursor.1 = prev;
        } else if row > 0 {
            // Joindre avec la ligne precedente
            let prev_line = self.lines.remove(row);
            self.cursor.0 = row - 1;
            self.cursor.1 = self.lines[self.cursor.0].len();
            self.lines[self.cursor.0].push_str(&prev_line);
        }
    }

    pub fn delete_char_at(&mut self) {
        let (row, col) = self.cursor;
        let line = &self.lines[row];
        if col < line.len() {
            let next = next_char_boundary(line, col);
            self.lines[row].drain(col..next);
        } else if row + 1 < self.lines.len() {
            // Joindre avec la ligne suivante
            let next_line = self.lines.remove(row + 1);
            self.lines[row].push_str(&next_line);
        }
    }

    // -- Cursor movement --

    pub fn move_cursor_left(&mut self) {
        let (row, col) = self.cursor;
        if col > 0 {
            self.cursor.1 = prev_char_boundary(&self.lines[row], col);
        } else if row > 0 {
            self.cursor.0 = row - 1;
            self.cursor.1 = self.lines[self.cursor.0].len();
        }
    }

    pub fn move_cursor_right(&mut self) {
        let (row, col) = self.cursor;
        let line = &self.lines[row];
        if col < line.len() {
            self.cursor.1 = next_char_boundary(line, col);
        } else if row + 1 < self.lines.len() {
            self.cursor.0 = row + 1;
            self.cursor.1 = 0;
        }
    }

    pub fn move_cursor_home(&mut self) {
        self.cursor.1 = 0;
    }

    pub fn move_cursor_end(&mut self) {
        self.cursor.1 = self.lines[self.cursor.0].len();
    }

    pub fn move_cursor_word_left(&mut self) {
        let (row, col) = self.cursor;
        if col == 0 {
            self.move_cursor_left();
            return;
        }
        let line = &self.lines[row];
        let bytes = line.as_bytes();
        let mut pos = col;
        // Skip whitespace
        while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        // Skip word chars
        while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        self.cursor.1 = pos;
    }

    pub fn move_cursor_word_right(&mut self) {
        let (row, col) = self.cursor;
        let line = &self.lines[row];
        if col >= line.len() {
            self.move_cursor_right();
            return;
        }
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut pos = col;
        // Skip word chars
        while pos < len && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip whitespace
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        self.cursor.1 = pos;
    }

    // -- Line operations --

    /// Clear the entire current line
    pub fn clear_line(&mut self) {
        self.lines[self.cursor.0].clear();
        self.cursor.1 = 0;
    }

    /// Delete the word before the cursor (Ctrl+W)
    pub fn delete_word_before(&mut self) {
        let (row, col) = self.cursor;
        if col == 0 {
            return;
        }
        let line = &self.lines[row];
        let bytes = line.as_bytes();
        let mut pos = col;
        // Skip whitespace
        while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        // Skip word chars
        while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        self.lines[row].drain(pos..col);
        self.cursor.1 = pos;
    }

    /// Add a new line (Enter in multiline mode)
    pub fn new_line(&mut self) {
        let (row, col) = self.cursor;
        let rest = self.lines[row][col..].to_string();
        self.lines[row].truncate(col);
        self.lines.insert(row + 1, rest);
        self.cursor.0 = row + 1;
        self.cursor.1 = 0;
    }

    /// Full reset after submit
    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cursor = (0, 0);
        self.history_index = None;
        self.saved_input.clear();
    }

    // -- Mutable access for completion acceptance --

    pub fn lines_mut(&mut self) -> &mut Vec<String> {
        &mut self.lines
    }

    pub fn set_cursor_col(&mut self, col: usize) {
        self.cursor.1 = col;
    }

    // -- History navigation --

    /// Navigate to the previous entry (Up)
    pub fn history_up(&mut self, history: &crate::history::History) {
        let max_idx = history.len();
        if max_idx == 0 {
            return;
        }

        match self.history_index {
            None => {
                // Sauvegarder l'input courant
                self.saved_input = self.lines.clone();
                self.history_index = Some(0);
            }
            Some(idx) if idx + 1 < max_idx => {
                self.history_index = Some(idx + 1);
            }
            _ => return, // Deja au plus vieux
        }

        if let Some(entry) = history.get(self.history_index.unwrap()) {
            self.lines = entry.split('\n').map(|s| s.to_string()).collect();
            let last_row = self.lines.len() - 1;
            self.cursor = (last_row, self.lines[last_row].len());
        }
    }

    /// Navigate to the next entry (Down)
    pub fn history_down(&mut self, _history: &crate::history::History) {
        match self.history_index {
            None => return, // Deja a l'input courant
            Some(0) => {
                // Restaurer l'input sauvegarde
                self.history_index = None;
                if self.saved_input.is_empty() {
                    self.lines = vec![String::new()];
                } else {
                    self.lines = self.saved_input.clone();
                }
                let last_row = self.lines.len() - 1;
                self.cursor = (last_row, self.lines[last_row].len());
            }
            Some(idx) => {
                self.history_index = Some(idx - 1);
                if let Some(entry) = _history.get(idx - 1) {
                    self.lines = entry.split('\n').map(|s| s.to_string()).collect();
                    let last_row = self.lines.len() - 1;
                    self.cursor = (last_row, self.lines[last_row].len());
                }
            }
        }
    }
}

// -- UTF-8 helpers --

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos;
    while p > 0 {
        p -= 1;
        if s.is_char_boundary(p) {
            return p;
        }
    }
    0
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos + 1;
    while p < s.len() {
        if s.is_char_boundary(p) {
            return p;
        }
        p += 1;
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_char() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_char('b');
        assert_eq!(input.full_text(), "ab");
        assert_eq!(input.cursor(), (0, 2));
    }

    #[test]
    fn test_delete_char_before() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_char('b');
        input.delete_char_before();
        assert_eq!(input.full_text(), "a");
        assert_eq!(input.cursor(), (0, 1));
    }

    #[test]
    fn test_move_cursor() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_char('b');
        input.insert_char('c');
        input.move_cursor_left();
        assert_eq!(input.cursor(), (0, 2));
        input.move_cursor_home();
        assert_eq!(input.cursor(), (0, 0));
        input.move_cursor_end();
        assert_eq!(input.cursor(), (0, 3));
    }

    #[test]
    fn test_new_line() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.new_line();
        input.insert_char('b');
        assert_eq!(input.full_text(), "a\nb");
        assert_eq!(input.line_count(), 2);
    }

    #[test]
    fn test_clear() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_char('b');
        input.clear();
        assert!(input.is_empty());
        assert_eq!(input.cursor(), (0, 0));
    }

    #[test]
    fn test_delete_word_before() {
        let mut input = InputState::new();
        for ch in "hello world".chars() {
            input.insert_char(ch);
        }
        input.delete_word_before();
        assert_eq!(input.full_text(), "hello ");
    }

    #[test]
    fn test_word_movement() {
        let mut input = InputState::new();
        for ch in "hello world".chars() {
            input.insert_char(ch);
        }
        input.move_cursor_home();
        input.move_cursor_word_right();
        assert_eq!(input.cursor(), (0, 6)); // after "hello "
        input.move_cursor_word_right();
        assert_eq!(input.cursor(), (0, 11)); // end
        input.move_cursor_word_left();
        assert_eq!(input.cursor(), (0, 6)); // before "world"
    }
}
