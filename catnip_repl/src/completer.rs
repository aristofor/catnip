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

impl Default for CompletionState {
    fn default() -> Self {
        Self::new()
    }
}

use catnip_tools::symbols;

// GENERATED FROM catnip/context.py - do not edit manually.
// Run: python catnip_tools/gen_builtins.py
// @generated-completer-builtins-start
const BUILTINS: &[&str] = &[
    "ArithmeticError",
    "AttributeError",
    "Exception",
    "IndexError",
    "KeyError",
    "LookupError",
    "META",
    "MemoryError",
    "ND",
    "NameError",
    "RUNTIME",
    "RuntimeError",
    "TypeError",
    "ValueError",
    "ZeroDivisionError",
    "_cache",
    "abs",
    "all",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "cached",
    "callable",
    "chr",
    "classmethod",
    "complex",
    "debug",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "filter",
    "float",
    "fold",
    "format",
    "freeze",
    "frozenset",
    "getattr",
    "hasattr",
    "hash",
    "hex",
    "id",
    "import",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "jit",
    "len",
    "list",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "ord",
    "pow",
    "property",
    "pure",
    "range",
    "reduce",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "thaw",
    "tuple",
    "typeof",
    "vars",
    "zip",
];
// @generated-completer-builtins-end

/// REPL commands (without leading /) - from single source of truth
fn repl_commands() -> Vec<&'static str> {
    crate::commands::all_command_names()
}

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
    "append", "clear", "copy", "count", "extend", "index", "insert", "pop", "remove", "reverse", "sort",
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
pub fn extract_word_at(line: &str, pos: usize) -> (usize, &str) {
    let start = line[..pos]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + line[i..].chars().next().unwrap().len_utf8())
        .unwrap_or(0);
    (start, &line[start..pos])
}

/// Catnip completer with context awareness
pub struct CatnipCompleter {
    /// Variable names from context
    variables: Vec<String>,
    /// Per-variable attributes (from dir())
    attrs: std::collections::HashMap<String, Vec<String>>,
}

impl CatnipCompleter {
    pub fn new() -> Self {
        Self {
            variables: Vec::new(),
            attrs: std::collections::HashMap::new(),
        }
    }

    /// Met a jour les variables connues (filtre builtins et keywords)
    pub fn set_variables(&mut self, vars: Vec<String>) {
        self.variables = vars
            .into_iter()
            .filter(|v| !BUILTINS.contains(&v.as_str()) && !symbols::is_keyword(v))
            .collect();
    }

    /// Met a jour les attributs connus par variable
    pub fn set_attrs(&mut self, attrs: std::collections::HashMap<String, Vec<String>>) {
        self.attrs = attrs;
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
        repl_commands()
            .into_iter()
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

        // Extract object name before the dot
        let before_dot = &line[..start.saturating_sub(1)]; // skip the '.'
        let obj_name = before_dot
            .rfind(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| &before_dot[i + 1..])
            .unwrap_or(before_dot);

        // Try dynamic attrs first
        if !obj_name.is_empty() {
            if let Some(obj_attrs) = self.attrs.get(obj_name) {
                let suggestions: Vec<Completion> = obj_attrs
                    .iter()
                    .filter(|a| a.starts_with(prefix))
                    .map(|a| Completion {
                        text: a.clone(),
                        category: "method",
                        replace_start: start,
                        replace_end: pos,
                    })
                    .collect();
                if !suggestions.is_empty() {
                    return suggestions;
                }
            }
        }

        // Fallback: hardcoded str/list/dict methods
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
        for kw in symbols::keywords() {
            if kw.starts_with(prefix) && !seen.contains(kw) {
                seen.insert(kw.clone());
                suggestions.push(Completion {
                    text: kw.clone(),
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
        // Check if the current word is preceded by a dot
        let (start, _) = self.extract_word(line, pos);
        if start == 0 {
            return false;
        }
        // Look at character just before the word start
        line[..start].chars().next_back().map(|c| c == '.').unwrap_or(false)
    }
}

impl Default for CatnipCompleter {
    fn default() -> Self {
        Self::new()
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
        let suggestions = completer.complete("sor", 3);
        let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert!(values.contains(&"sorted"));
        assert_eq!(
            suggestions.iter().find(|s| s.text == "sorted").unwrap().category,
            "builtin"
        );
    }

    #[test]
    fn test_builtin_not_shadowed_by_variable() {
        let mut completer = CatnipCompleter::new();
        // Simule le contexte Python qui exporte "sorted" comme variable
        completer.set_variables(vec!["sorted".to_string(), "my_var".to_string()]);
        let suggestions = completer.complete("sor", 3);
        let sorted_s = suggestions.iter().find(|s| s.text == "sorted").unwrap();
        assert_eq!(sorted_s.category, "builtin");
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

    #[test]
    fn test_empty_line() {
        let completer = CatnipCompleter::new();
        let suggestions = completer.complete("", 0);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_command_no_match() {
        let completer = CatnipCompleter::new();
        let suggestions = completer.complete("/zzzz", 5);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_method_after_dot() {
        let completer = CatnipCompleter::new();
        // Dot completion triggers at cursor right after the dot
        let suggestions = completer.complete("x.", 2);
        let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert!(values.contains(&"upper"));
        assert!(values.contains(&"lower"));
        assert_eq!(
            suggestions.iter().find(|s| s.text == "upper").unwrap().category,
            "method"
        );
    }

    #[test]
    fn test_dynamic_attrs_after_dot() {
        let mut completer = CatnipCompleter::new();
        let mut attrs = std::collections::HashMap::new();
        attrs.insert(
            "io".to_string(),
            vec!["BytesIO".to_string(), "StringIO".to_string(), "open".to_string()],
        );
        completer.set_attrs(attrs);
        completer.set_variables(vec!["io".to_string()]);

        let suggestions = completer.complete("io.", 3);
        let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert!(values.contains(&"BytesIO"));
        assert!(values.contains(&"StringIO"));
        assert!(values.contains(&"open"));
        // Should NOT contain hardcoded str/list/dict methods
        assert!(!values.contains(&"upper"));
        assert!(!values.contains(&"append"));
    }

    #[test]
    fn test_dynamic_attrs_with_prefix() {
        let mut completer = CatnipCompleter::new();
        let mut attrs = std::collections::HashMap::new();
        attrs.insert(
            "io".to_string(),
            vec!["BytesIO".to_string(), "StringIO".to_string(), "open".to_string()],
        );
        completer.set_attrs(attrs);

        let suggestions = completer.complete("io.St", 5);
        let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(values, vec!["StringIO"]);
    }

    #[test]
    fn test_unknown_var_falls_back_to_hardcoded() {
        let completer = CatnipCompleter::new();
        // No attrs set for "x", should fallback to str/list/dict methods
        let suggestions = completer.complete("x.up", 4);
        let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
        assert!(values.contains(&"upper"));
        assert!(values.contains(&"update"));
    }

    #[test]
    fn test_completion_state_nav() {
        let mut state = CompletionState::new();
        for i in 0..3 {
            state.suggestions.push(Completion {
                text: format!("item{i}"),
                category: "variable",
                replace_start: 0,
                replace_end: 1,
            });
        }
        state.active = true;
        assert_eq!(state.current().unwrap().text, "item0");
        state.select_next();
        assert_eq!(state.current().unwrap().text, "item1");
        state.select_next();
        assert_eq!(state.current().unwrap().text, "item2");
        state.select_prev();
        assert_eq!(state.current().unwrap().text, "item1");
    }

    #[test]
    fn test_completion_state_wrap_prev() {
        let mut state = CompletionState::new();
        for name in ["a", "b", "c"] {
            state.suggestions.push(Completion {
                text: name.to_string(),
                category: "variable",
                replace_start: 0,
                replace_end: 1,
            });
        }
        state.active = true;
        // At index 0, select_prev wraps to last
        state.select_prev();
        assert_eq!(state.selected, 2);
        assert_eq!(state.current().unwrap().text, "c");
    }

    #[test]
    fn test_completion_state_empty() {
        let mut state = CompletionState::new();
        // Operations on empty state should not panic
        state.select_next();
        state.select_prev();
        assert_eq!(state.selected, 0);
        assert!(state.current().is_none());
    }

    #[test]
    fn test_completion_state_reset() {
        let mut state = CompletionState::new();
        state.suggestions.push(Completion {
            text: "x".to_string(),
            category: "variable",
            replace_start: 0,
            replace_end: 1,
        });
        state.active = true;
        state.selected = 0;
        state.reset();
        assert!(state.suggestions.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.active);
    }
}
