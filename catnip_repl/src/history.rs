// FILE: catnip_repl/src/history.rs
//! File-backed command history.
//!
//! Replaces reedline::FileBackedHistory with a plain text format.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub struct History {
    entries: Vec<String>,
    max_entries: usize,
    file_path: PathBuf,
}

impl History {
    /// Load history from file (or create an empty history)
    pub fn load(path: &Path, max: usize) -> Self {
        let entries = if path.exists() {
            match fs::read_to_string(path) {
                Ok(content) => content
                    .lines()
                    // Skipper le header reedline #V2 si present
                    .filter(|line| !line.starts_with("#V2") && !line.is_empty())
                    .map(|s| s.to_string())
                    .collect(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        let mut hist = Self {
            entries,
            max_entries: max,
            file_path: path.to_path_buf(),
        };
        hist.truncate();
        hist
    }

    /// Save to disk
    pub fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.file_path, self.entries.join("\n") + "\n")
    }

    /// Add an entry, deduplicate consecutive duplicates, truncate
    pub fn push(&mut self, entry: &str) {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            return;
        }
        // Deduplique le dernier
        if self.entries.last().map(|s| s.as_str()) == Some(trimmed) {
            return;
        }
        self.entries.push(trimmed.to_string());
        self.truncate();
    }

    /// Entry by index from the end (0 = most recent)
    pub fn get(&self, index: usize) -> Option<&str> {
        if index >= self.entries.len() {
            return None;
        }
        let pos = self.entries.len() - 1 - index;
        Some(&self.entries[pos])
    }

    /// Number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Access all entries (for /history)
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    fn truncate(&mut self) {
        if self.entries.len() > self.max_entries {
            let excess = self.entries.len() - self.max_entries;
            self.entries.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_get() {
        let tmp = std::env::temp_dir().join("catnip_test_history");
        let mut hist = History::load(&tmp, 100);
        hist.push("hello");
        hist.push("world");
        assert_eq!(hist.get(0), Some("world"));
        assert_eq!(hist.get(1), Some("hello"));
        assert_eq!(hist.len(), 2);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_dedup_consecutive() {
        let tmp = std::env::temp_dir().join("catnip_test_history_dedup");
        let mut hist = History::load(&tmp, 100);
        hist.push("hello");
        hist.push("hello");
        assert_eq!(hist.len(), 1);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_truncate() {
        let tmp = std::env::temp_dir().join("catnip_test_history_trunc");
        let mut hist = History::load(&tmp, 3);
        hist.push("a");
        hist.push("b");
        hist.push("c");
        hist.push("d");
        assert_eq!(hist.len(), 3);
        assert_eq!(hist.get(0), Some("d"));
        assert_eq!(hist.get(2), Some("b"));
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_save_and_load() {
        let tmp = std::env::temp_dir().join("catnip_test_history_persist");
        {
            let mut hist = History::load(&tmp, 100);
            hist.push("first");
            hist.push("second");
            hist.save().unwrap();
        }
        {
            let hist = History::load(&tmp, 100);
            assert_eq!(hist.len(), 2);
            assert_eq!(hist.get(0), Some("second"));
        }
        let _ = fs::remove_file(&tmp);
    }
}
