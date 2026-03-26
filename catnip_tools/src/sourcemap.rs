// FILE: catnip_tools/src/sourcemap.rs
//! Source map for lazy position calculation.
//!
//! Stores source code and converts byte offsets to line/column on demand.
//! Zero overhead when no error occurs.

/// Maps byte offsets to line/column positions.
///
/// Line offsets are computed lazily on first access.
pub struct SourceMap {
    source: Vec<u8>,
    filename: String,
    line_offsets: Option<Vec<usize>>,
}

impl SourceMap {
    pub fn new(source: Vec<u8>, filename: String) -> Self {
        Self {
            source,
            filename,
            line_offsets: None,
        }
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// Build line offset index (once).
    fn build_line_offsets(&mut self) {
        if self.line_offsets.is_some() {
            return;
        }
        let mut offsets = vec![0usize];
        for (i, &c) in self.source.iter().enumerate() {
            if c == b'\n' {
                offsets.push(i + 1);
            }
        }
        self.line_offsets = Some(offsets);
    }

    /// Convert byte offset to (line, column) - 1-indexed.
    ///
    /// Uses binary search for O(log n) lookup.
    pub fn byte_to_line_col(&mut self, byte_offset: usize) -> (usize, usize) {
        self.build_line_offsets();
        let offsets = self.line_offsets.as_ref().unwrap();
        // Binary search: find rightmost offset <= byte_offset
        let line_idx = match offsets.binary_search(&byte_offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let col = byte_offset - offsets[line_idx];
        (line_idx + 1, col + 1)
    }

    /// Get a single line by 1-indexed line number.
    pub fn get_line(&mut self, line_num: usize) -> String {
        self.build_line_offsets();
        let offsets = self.line_offsets.as_ref().unwrap();
        if line_num < 1 || line_num > offsets.len() {
            return String::new();
        }
        let start = offsets[line_num - 1];
        let end = if line_num < offsets.len() {
            offsets[line_num] - 1 // exclude newline
        } else {
            self.source.len()
        };
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    /// Extract code snippet with pointer.
    ///
    /// Returns formatted snippet with line number and caret pointing to position.
    ///
    /// Example output:
    /// ```text
    ///   12 |     return factoral(n - 1, acc)
    ///      |            ^~~~~~~
    /// ```
    pub fn get_snippet(&mut self, start_byte: usize, end_byte: usize, context_lines: usize) -> String {
        let (line, col) = self.byte_to_line_col(start_byte);
        let (end_line, end_col) = if end_byte > start_byte {
            self.byte_to_line_col(end_byte)
        } else {
            (line, col + 1)
        };

        let mut lines = Vec::new();
        let line_num_width = format!("{}", line + context_lines).len();

        // Context lines before
        let ctx_start = if line > context_lines { line - context_lines } else { 1 };
        for ctx_line in ctx_start..line {
            let text = self.get_line(ctx_line);
            lines.push(format!("  {:>width$} | {}", ctx_line, text, width = line_num_width));
        }

        // Error line
        let error_line_text = self.get_line(line);
        lines.push(format!(
            "  {:>width$} | {}",
            line,
            error_line_text,
            width = line_num_width
        ));

        // Pointer line
        let span_len = if line == end_line {
            (end_col - col).max(1)
        } else {
            (error_line_text.len() - col + 2).max(1)
        };
        let pointer = format!("{}^{}", " ".repeat(col - 1), "~".repeat(span_len.saturating_sub(1)));
        lines.push(format!("  {} | {}", " ".repeat(line_num_width), pointer));

        // Context lines after
        self.build_line_offsets();
        let total_lines = self.line_offsets.as_ref().unwrap().len();
        let ctx_end = (line + context_lines + 1).min(total_lines + 1);
        for ctx_line in (line + 1)..ctx_end {
            let text = self.get_line(ctx_line);
            lines.push(format!("  {:>width$} | {}", ctx_line, text, width = line_num_width));
        }

        lines.join("\n")
    }

    /// Convert a 1-indexed line number to byte offset.
    pub fn line_to_offset(&mut self, line: usize) -> Option<usize> {
        self.build_line_offsets();
        let offsets = self.line_offsets.as_ref().unwrap();
        if line < 1 || line > offsets.len() {
            return None;
        }
        Some(offsets[line - 1])
    }

    /// Total number of lines.
    pub fn line_count(&mut self) -> usize {
        self.build_line_offsets();
        self.line_offsets.as_ref().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_to_line_col() {
        let mut sm = SourceMap::new(b"abc\ndef\nghi".to_vec(), "<test>".into());
        assert_eq!(sm.byte_to_line_col(0), (1, 1)); // 'a'
        assert_eq!(sm.byte_to_line_col(3), (1, 4)); // '\n'
        assert_eq!(sm.byte_to_line_col(4), (2, 1)); // 'd'
        assert_eq!(sm.byte_to_line_col(8), (3, 1)); // 'g'
    }

    #[test]
    fn test_get_line() {
        let mut sm = SourceMap::new(b"abc\ndef\nghi".to_vec(), "<test>".into());
        assert_eq!(sm.get_line(1), "abc");
        assert_eq!(sm.get_line(2), "def");
        assert_eq!(sm.get_line(3), "ghi");
        assert_eq!(sm.get_line(0), "");
        assert_eq!(sm.get_line(4), "");
    }

    #[test]
    fn test_get_snippet() {
        let mut sm = SourceMap::new(b"abc\ndef\nghi".to_vec(), "<test>".into());
        let snippet = sm.get_snippet(4, 5, 0);
        assert!(snippet.contains("def"));
        assert!(snippet.contains("^"));
    }

    #[test]
    fn test_line_to_offset() {
        let mut sm = SourceMap::new(b"abc\ndef\nghi".to_vec(), "<test>".into());
        assert_eq!(sm.line_to_offset(1), Some(0));
        assert_eq!(sm.line_to_offset(2), Some(4));
        assert_eq!(sm.line_to_offset(3), Some(8));
        assert_eq!(sm.line_to_offset(0), None);
        assert_eq!(sm.line_to_offset(4), None);
    }

    #[test]
    fn test_line_count() {
        let mut sm = SourceMap::new(b"abc\ndef\nghi".to_vec(), "<test>".into());
        assert_eq!(sm.line_count(), 3);
    }

    #[test]
    fn test_snippet_with_context() {
        let mut sm = SourceMap::new(b"line1\nline2\nline3\nline4\nline5".to_vec(), "<test>".into());
        let snippet = sm.get_snippet(6, 7, 1);
        assert!(snippet.contains("line1")); // context before
        assert!(snippet.contains("line2")); // error line
        assert!(snippet.contains("line3")); // context after
    }
}
