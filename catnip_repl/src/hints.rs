// FILE: catnip_repl/src/hints.rs
//! Fish-like inline ghost text hints for the REPL.
//!
//! Produces a single ghost text string displayed after the cursor,
//! accepted with Right arrow. Mutually exclusive with the completion popup.

use crate::completer::extract_word_at;

/// What the cursor is sitting on
enum HintContext<'a> {
    /// Typing an identifier prefix: `pri|` -> suggest `print(values...)`
    IdentifierPrefix { prefix: &'a str },
    /// Inside a function call: `map(fn, |` -> suggest remaining params
    FunctionCall { func_name: &'a str, arg_index: usize },
    /// Typing a keyword prefix: `whi|` -> suggest `le condition { body }`
    KeywordPrefix { prefix: &'a str },
}

/// (name, full_params_display, individual_params)
const BUILTIN_SIGNATURES: &[(&str, &str, &[&str])] = &[
    ("print", "values...", &["values..."]),
    ("write", "values...", &["values..."]),
    ("write_err", "values...", &["values..."]),
    ("str", "value", &["value"]),
    ("int", "value", &["value"]),
    ("float", "value", &["value"]),
    ("list", "iterable", &["iterable"]),
    ("dict", "mapping", &["mapping"]),
    ("tuple", "iterable", &["iterable"]),
    ("set", "iterable", &["iterable"]),
    ("range", "start, stop, step", &["start", "stop", "step"]),
    ("enumerate", "iterable, start", &["iterable", "start"]),
    ("zip", "iter1, iter2, ...", &["iter1", "iter2", "..."]),
    ("map", "fn, iterable", &["fn", "iterable"]),
    ("filter", "fn, iterable", &["fn", "iterable"]),
    ("sorted", "iterable, key, reverse", &["iterable", "key", "reverse"]),
    ("reversed", "iterable", &["iterable"]),
    ("len", "obj", &["obj"]),
    ("sum", "iterable, start", &["iterable", "start"]),
    ("min", "values...", &["values..."]),
    ("max", "values...", &["values..."]),
    ("abs", "value", &["value"]),
    ("round", "value, ndigits", &["value", "ndigits"]),
    ("format", "value, spec", &["value", "spec"]),
    ("repr", "obj", &["obj"]),
    ("ascii", "obj", &["obj"]),
    ("jit", "fn", &["fn"]),
    ("pure", "fn", &["fn"]),
    ("cached", "fn", &["fn"]),
];

/// (keyword_prefix, template_suffix)
/// The suffix is what gets appended after the completed keyword.
const KEYWORD_TEMPLATES: &[(&str, &str)] = &[
    ("if", " condition { body }"),
    ("elif", " condition { body }"),
    ("else", " { body }"),
    ("while", " condition { body }"),
    ("for", " x in iterable { body }"),
    ("match", " value { case pattern => result }"),
    ("return", " value"),
    ("pragma", "(\"name\", value)"),
];

/// Hint engine state
pub struct HintEngine {
    variables: Vec<String>,
}

impl HintEngine {
    pub fn new() -> Self {
        Self { variables: Vec::new() }
    }

    pub fn set_variables(&mut self, vars: Vec<String>) {
        self.variables = vars;
    }

    /// Main entry point: returns ghost text to display after cursor
    pub fn get_hint(&self, line: &str, pos: usize) -> Option<String> {
        if line.is_empty() || pos == 0 {
            return None;
        }

        // Detect context
        // Priority: function call > keyword (at statement start) > identifier
        // Use and_then to fall through when hint_for_context returns None
        if let Some(hint) = self
            .detect_function_call(line, pos)
            .and_then(|ctx| self.hint_for_context(&ctx))
        {
            return Some(hint);
        }
        if let Some(hint) = self
            .detect_keyword_prefix(line, pos)
            .and_then(|ctx| self.hint_for_context(&ctx))
        {
            return Some(hint);
        }
        if let Some(hint) = self
            .detect_identifier_prefix(line, pos)
            .and_then(|ctx| self.hint_for_context(&ctx))
        {
            return Some(hint);
        }

        None
    }

    /// Detect if cursor is inside a function call: `func(arg1, |`
    fn detect_function_call<'a>(&self, line: &'a str, pos: usize) -> Option<HintContext<'a>> {
        let bytes = &line.as_bytes()[..pos];
        let mut depth = 0i32;

        // Scan backwards to find unmatched '('
        for i in (0..bytes.len()).rev() {
            match bytes[i] {
                b')' => depth += 1,
                b'(' => {
                    if depth == 0 {
                        // Found unmatched '(', extract function name before it
                        let before_paren = &line[..i];
                        let func_end = before_paren.trim_end().len();
                        if func_end == 0 {
                            return None;
                        }
                        let (start, name) = extract_word_at(before_paren, func_end);
                        if name.is_empty() {
                            return None;
                        }

                        // Count commas between '(' and pos to determine arg_index
                        let inside = &line[i + 1..pos];
                        let arg_index = count_commas_at_depth_0(inside);

                        return Some(HintContext::FunctionCall {
                            func_name: &line[start..func_end],
                            arg_index,
                        });
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }

        None
    }

    /// Detect keyword prefix at cursor
    fn detect_keyword_prefix<'a>(&self, line: &'a str, pos: usize) -> Option<HintContext<'a>> {
        let (start, prefix) = extract_word_at(line, pos);
        if prefix.len() < 2 {
            return None;
        }

        // Only suggest if prefix is at a "statement start" position
        // (start of line or after whitespace only)
        let before = line[..start].trim_end();
        // Allow keyword hints at line start, after '=', or after '{' / ';'
        let ok_position = before.is_empty() || before.ends_with('=') || before.ends_with('{') || before.ends_with(';');

        if !ok_position {
            return None;
        }

        // Check if prefix matches any keyword template
        for (kw, _) in KEYWORD_TEMPLATES {
            if kw.starts_with(prefix) && *kw != prefix {
                return Some(HintContext::KeywordPrefix { prefix });
            }
        }

        None
    }

    /// Detect identifier prefix at cursor
    fn detect_identifier_prefix<'a>(&self, line: &'a str, pos: usize) -> Option<HintContext<'a>> {
        let (_start, prefix) = extract_word_at(line, pos);
        if prefix.len() < 2 {
            return None;
        }

        // Check builtins first, then variables
        let has_match = BUILTIN_SIGNATURES
            .iter()
            .any(|(name, _, _)| name.starts_with(prefix) && *name != prefix)
            || self
                .variables
                .iter()
                .any(|v| v.starts_with(prefix) && v.as_str() != prefix);

        if has_match {
            Some(HintContext::IdentifierPrefix { prefix })
        } else {
            None
        }
    }

    /// Generate ghost text for a given context
    fn hint_for_context(&self, ctx: &HintContext) -> Option<String> {
        match ctx {
            HintContext::FunctionCall { func_name, arg_index } => {
                // Find signature
                let sig = BUILTIN_SIGNATURES.iter().find(|(name, _, _)| *name == *func_name)?;
                let params = sig.2;
                if *arg_index >= params.len() {
                    return Some(")".to_string());
                }
                // Show remaining params from arg_index
                let remaining: Vec<&str> = params[*arg_index..].to_vec();
                Some(format!("{})", remaining.join(", ")))
            }
            HintContext::KeywordPrefix { prefix } => {
                // Find best matching keyword template
                let (kw, template) = KEYWORD_TEMPLATES.iter().find(|(kw, _)| kw.starts_with(*prefix))?;
                // Ghost = rest of keyword + template
                let rest = &kw[prefix.len()..];
                Some(format!("{}{}", rest, template))
            }
            HintContext::IdentifierPrefix { prefix } => {
                // Try builtins first (with signature)
                if let Some((name, params, _)) = BUILTIN_SIGNATURES
                    .iter()
                    .find(|(name, _, _)| name.starts_with(*prefix) && *name != *prefix)
                {
                    let rest = &name[prefix.len()..];
                    return Some(format!("{}({})", rest, params));
                }
                // Then variables (just complete the name)
                if let Some(var) = self
                    .variables
                    .iter()
                    .find(|v| v.starts_with(*prefix) && v.as_str() != *prefix)
                {
                    let rest = &var[prefix.len()..];
                    return Some(rest.to_string());
                }
                None
            }
        }
    }
}

impl Default for HintEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Count commas at parenthesis depth 0
fn count_commas_at_depth_0(s: &str) -> usize {
    let mut depth = 0i32;
    let mut count = 0;
    for b in s.bytes() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> HintEngine {
        HintEngine::new()
    }

    #[test]
    fn test_identifier_print() {
        let e = engine();
        let hint = e.get_hint("pri", 3);
        assert_eq!(hint, Some("nt(values...)".to_string()));
    }

    #[test]
    fn test_identifier_map() {
        let e = engine();
        // "ma" at statement start -> keyword "match" takes priority
        let hint = e.get_hint("ma", 2);
        assert_eq!(hint, Some("tch value { case pattern => result }".to_string()));
        // "ma" in non-keyword position -> identifier "map" via fallthrough
        let hint = e.get_hint("foo(ma", 6);
        assert_eq!(hint, Some("p(fn, iterable)".to_string()));
    }

    #[test]
    fn test_identifier_full_name_no_hint() {
        let e = engine();
        // Exact match should not hint
        assert_eq!(e.get_hint("print", 5), None);
    }

    #[test]
    fn test_function_call_first_arg() {
        let e = engine();
        let hint = e.get_hint("map(", 4);
        assert_eq!(hint, Some("fn, iterable)".to_string()));
    }

    #[test]
    fn test_function_call_second_arg() {
        let e = engine();
        let hint = e.get_hint("map(f, ", 7);
        assert_eq!(hint, Some("iterable)".to_string()));
    }

    #[test]
    fn test_function_call_excess_args() {
        let e = engine();
        let hint = e.get_hint("map(f, x, ", 10);
        assert_eq!(hint, Some(")".to_string()));
    }

    #[test]
    fn test_keyword_while() {
        let e = engine();
        let hint = e.get_hint("whi", 3);
        assert_eq!(hint, Some("le condition { body }".to_string()));
    }

    #[test]
    fn test_keyword_for() {
        let e = engine();
        let hint = e.get_hint("fo", 2);
        assert_eq!(hint, Some("r x in iterable { body }".to_string()));
    }

    #[test]
    fn test_keyword_full_no_hint() {
        let e = engine();
        // Full keyword should not produce a keyword hint
        // (but may produce an identifier hint if it matches a builtin)
        let hint = e.get_hint("while", 5);
        assert_eq!(hint, None);
    }

    #[test]
    fn test_variable_hint() {
        let mut e = engine();
        e.set_variables(vec!["fibonacci".to_string(), "factor".to_string()]);
        let hint = e.get_hint("fib", 3);
        assert_eq!(hint, Some("onacci".to_string()));
    }

    #[test]
    fn test_empty_input() {
        let e = engine();
        assert_eq!(e.get_hint("", 0), None);
    }

    #[test]
    fn test_single_char_no_hint() {
        let e = engine();
        // Single char prefix too short for identifier hint
        assert_eq!(e.get_hint("p", 1), None);
    }

    #[test]
    fn test_keyword_not_at_statement_start() {
        let e = engine();
        // "whi" after an expression should not suggest keyword
        // but should fall through to identifier hint if there's a match
        let hint = e.get_hint("x + whi", 7);
        // No keyword hint (not at statement start), no builtin match
        assert_eq!(hint, None);
    }

    #[test]
    fn test_nested_parens() {
        let e = engine();
        // map(filter(...), |  -> should suggest second arg of map
        let hint = e.get_hint("map(filter(x), ", 15);
        assert_eq!(hint, Some("iterable)".to_string()));
    }

    #[test]
    fn test_hint_unknown_function() {
        let e = engine();
        // Unknown function call -> no param hint
        let hint = e.get_hint("my_func(", 8);
        assert_eq!(hint, None);
    }

    #[test]
    fn test_keyword_after_assignment() {
        let e = engine();
        // "if" after "= " is at statement-start position for hint detection
        let hint = e.get_hint("x = if", 6);
        // Depending on implementation: either keyword hint or None
        // "if" at position 4 is after "= ", detect_keyword_prefix checks statement start
        // The word "if" is a full keyword, so no suffix to suggest
        assert_eq!(hint, None);
    }

    #[test]
    fn test_keyword_not_mid_identifier() {
        let e = engine();
        // "while_loop" should not trigger "while" keyword hint
        let hint = e.get_hint("while_loop", 10);
        assert_eq!(hint, None);
    }

    #[test]
    fn test_variable_hint_suffix() {
        let mut e = engine();
        e.set_variables(vec!["fibonacci".to_string()]);
        let hint = e.get_hint("fib", 3);
        assert_eq!(hint, Some("onacci".to_string()));
    }

    #[test]
    fn test_builtin_priority_over_variable() {
        let mut e = engine();
        e.set_variables(vec!["printer".to_string()]);
        // "pri" should hint "print" (builtin) not "printer" (variable)
        let hint = e.get_hint("pri", 3);
        assert_eq!(hint, Some("nt(values...)".to_string()));
    }
}
