// FILE: catnip_repl/src/completer.rs
//! Contextual completion for Catnip REPL
//!
//! Provides smart completion for:
//! - REPL commands (/help, /exit, etc.)
//! - Language keywords (if, while, for, etc.)
//! - Builtin functions (print, len, type, etc.)
//! - Variables from context
//! - Attributes (after '.')

/// Single completion suggestion
pub struct Completion {
    pub text: String,
    pub category: &'static str, // "keyword", "builtin", "variable", "command", "method"
    pub replace_start: usize,
    pub replace_end: usize,
}

/// Completion popup state
pub struct CompletionState {
    pub suggestions: Vec<Completion>,
    pub selected: usize,
    pub active: bool,
}

impl CompletionState {
    pub fn new() -> Self {
        Self {
            suggestions: Vec::new(),
            selected: 0,
            active: false,
        }
    }

    pub fn reset(&mut self) {
        self.suggestions.clear();
        self.selected = 0;
        self.active = false;
    }

    pub fn select_next(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected = (self.selected + 1) % self.suggestions.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.suggestions.is_empty() {
            if self.selected == 0 {
                self.selected = self.suggestions.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    /// Return the selected suggestion
    pub fn current(&self) -> Option<&Completion> {
        self.suggestions.get(self.selected)
    }
}

/// Keywords in Catnip language
const KEYWORDS: &[&str] = &[
    "if", "elif", "else", "while", "for", "in", "match", "case", "True", "False", "None", "and",
    "or", "not", "return", "break", "continue", "fn", "lambda", "pragma", "struct", "trait",
    "abstract", "static",
];

/// Builtin functions available in REPL (must match Context.__init__ globals)
const BUILTINS: &[&str] = &[
    // Types et constructeurs
    "str",
    "int",
    "float",
    "list",
    "dict",
    "tuple",
    "set",
    // I/O
    "print",
    "write",
    "write_err",
    // Iteration
    "range",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sorted",
    "reversed",
    // Aggregation
    "len",
    "sum",
    "min",
    "max",
    "abs",
    "round",
    // Formatting
    "format",
    "repr",
    "ascii",
    // Decorateurs et outils
    "jit",
    "pure",
    "cached",
    "debug",
    "logger",
];

/// REPL commands (without leading /)
const REPL_COMMANDS: &[&str] = &[
    "help", "exit", "quit", "clear", "cls", "stats", "version", "jit", "verbose", "history",
    "load", "debug", "time",
];

/// Common string methods
const STRING_METHODS: &[&str] = &[
    "capitalize",
    "casefold",
    "center",
    "count",
    "encode",
    "endswith",
    "find",
    "format",
    "index",
    "isalnum",
    "isalpha",
    "isascii",
    "isdecimal",
    "isdigit",
    "islower",
    "isnumeric",
    "isspace",
    "istitle",
    "isupper",
    "join",
    "ljust",
    "lower",
    "lstrip",
    "replace",
    "rfind",
    "rindex",
    "rjust",
    "rstrip",
    "split",
    "splitlines",
    "startswith",
    "strip",
    "swapcase",
    "title",
    "upper",
    "zfill",
];

/// Common list methods
const LIST_METHODS: &[&str] = &[
    "append", "clear", "copy", "count", "extend", "index", "insert", "pop", "remove", "reverse",
    "sort",
];

/// Common dict methods
const DICT_METHODS: &[&str] = &[
    "clear",
    "copy",
    "fromkeys",
    "get",
    "items",
    "keys",
    "pop",
    "popitem",
    "setdefault",
    "update",
    "values",
];

/// Extract the word at cursor position (reusable by hints)
pub fn extract_word_at<'a>(line: &'a str, pos: usize) -> (usize, &'a str) {
    let start = line[..pos]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    (start, &line[start..pos])
}

/// Catnip completer with context awareness
pub struct CatnipCompleter {
    /// Variable names from context
    variables: Vec<String>,
}

impl CatnipCompleter {
    pub fn new() -> Self {
        Self {
            variables: Vec::new(),
        }
    }

    /// Met a jour les variables connues (filtre builtins et keywords)
    pub fn set_variables(&mut self, vars: Vec<String>) {
        self.variables = vars
            .into_iter()
            .filter(|v| !BUILTINS.contains(&v.as_str()) && !KEYWORDS.contains(&v.as_str()))
            .collect();
    }

    /// Point d'entree principal
    pub fn complete(&self, line: &str, pos: usize) -> Vec<Completion> {
        if line.is_empty() || pos == 0 {
            return Vec::new();
        }

        // Command completion (lines starting with /)
        if line.starts_with('/') {
            return self.complete_command(line, pos);
        }

        // Attribute completion (after '.')
        if self.is_after_dot(line, pos) {
            return self.complete_attribute(line, pos);
        }

        // Regular identifier completion
        self.complete_identifier(line, pos)
    }

    fn complete_command(&self, line: &str, pos: usize) -> Vec<Completion> {
        let prefix = &line[1..pos];
        REPL_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(prefix))
            .map(|cmd| Completion {
                text: format!("/{}", cmd),
                category: "command",
                replace_start: 0,
                replace_end: pos,
            })
            .collect()
    }

    fn complete_attribute(&self, line: &str, pos: usize) -> Vec<Completion> {
        let (start, prefix) = self.extract_word(line, pos);

        let mut suggestions = Vec::new();
        let all_methods = STRING_METHODS
            .iter()
            .chain(LIST_METHODS.iter())
            .chain(DICT_METHODS.iter())
            .filter(|m| m.starts_with(prefix));

        for method in all_methods {
            suggestions.push(Completion {
                text: method.to_string(),
                category: "method",
                replace_start: start,
                replace_end: pos,
            });
        }
        suggestions
    }

    fn complete_identifier(&self, line: &str, pos: usize) -> Vec<Completion> {
        let (start, prefix) = self.extract_word(line, pos);
        let mut suggestions = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Variables from context (highest priority)
        for var in &self.variables {
            if var.starts_with(prefix) {
                seen.insert(var.clone());
                suggestions.push(Completion {
                    text: var.clone(),
                    category: "variable",
                    replace_start: start,
                    replace_end: pos,
                });
            }
        }

        // Keywords (skip if already seen as variable)
        for kw in KEYWORDS {
            if kw.starts_with(prefix) && !seen.contains(*kw) {
                seen.insert(kw.to_string());
                suggestions.push(Completion {
                    text: kw.to_string(),
                    category: "keyword",
                    replace_start: start,
                    replace_end: pos,
                });
            }
        }

        // Builtins (skip if already seen)
        for builtin in BUILTINS {
            if builtin.starts_with(prefix) && !seen.contains(*builtin) {
                suggestions.push(Completion {
                    text: builtin.to_string(),
                    category: "builtin",
                    replace_start: start,
                    replace_end: pos,
                });
            }
        }

        suggestions
    }

    fn extract_word<'a>(&self, line: &'a str, pos: usize) -> (usize, &'a str) {
        extract_word_at(line, pos)
    }

    fn is_after_dot(&self, line: &str, pos: usize) -> bool {
        if pos == 0 {
            return false;
        }
        line[..pos]
            .chars()
            .rev()
            .find(|c| !c.is_whitespace())
            .map(|c| c == '.')
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_completion() {
        let completer = CatnipCompleter::new();
        let suggestions = completer.complete("/he", 3);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].text, "/help");
    }

    #[test]
    fn test_keyword_completion() {
        let completer = CatnipCompleter::new();
        let suggestions = completer.complete("wh", 2);
        let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert!(values.contains(&"while"));
    }

    #[test]
    fn test_variable_completion() {
        let mut completer = CatnipCompleter::new();
        completer.set_variables(vec!["x".to_string(), "xyz".to_string()]);
        let suggestions = completer.complete("xy", 2);
        let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert!(values.contains(&"xyz"));
    }

    #[test]
    fn test_builtin_completion() {
        let completer = CatnipCompleter::new();
        let suggestions = completer.complete("pri", 3);
        let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert!(values.contains(&"print"));
        assert_eq!(
            suggestions
                .iter()
                .find(|s| s.text == "print")
                .unwrap()
                .category,
            "builtin"
        );
    }

    #[test]
    fn test_builtin_not_shadowed_by_variable() {
        let mut completer = CatnipCompleter::new();
        // Simule le contexte Python qui exporte "print" comme variable
        completer.set_variables(vec!["print".to_string(), "my_var".to_string()]);
        let suggestions = completer.complete("pri", 3);
        let print_s = suggestions.iter().find(|s| s.text == "print").unwrap();
        assert_eq!(print_s.category, "builtin");
    }

    #[test]
    fn test_is_after_dot() {
        let completer = CatnipCompleter::new();
        assert!(completer.is_after_dot("obj.", 4));
        assert!(!completer.is_after_dot("obj", 3));
        assert!(!completer.is_after_dot("", 0));
    }

    #[test]
    fn test_completion_state() {
        let mut state = CompletionState::new();
        assert!(!state.active);
        state.suggestions.push(Completion {
            text: "hello".to_string(),
            category: "variable",
            replace_start: 0,
            replace_end: 2,
        });
        state.suggestions.push(Completion {
            text: "help".to_string(),
            category: "builtin",
            replace_start: 0,
            replace_end: 2,
        });
        state.active = true;
        assert_eq!(state.selected, 0);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 0); // wrap around
        state.select_prev();
        assert_eq!(state.selected, 1); // wrap back
    }
}
