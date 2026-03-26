// FILE: catnip_tools/src/formatter.rs
use crate::config::FormatConfig;
use crate::token::Token;
use crate::tokenizer::extract_tokens;
use catnip_grammar::node_kinds as NK;
use catnip_grammar::symbols;

/// Keywords that are literal values (not control flow)
fn is_literal_keyword(value: &str) -> bool {
    matches!(value, "True" | "False" | "nil" | "None")
}

const BINARY_OPS: &[&str] = &[
    "PLUS",
    "MINUS",
    "STAR",
    "SLASH",
    "DOUBLE_SLASH",
    "PERCENT",
    "DOUBLE_STAR",
    "EQUAL",
    "EQEQUAL",
    "NOTEQUAL",
    "LESS",
    "LESSEQUAL",
    "GREATER",
    "GREATEREQUAL",
    "AND",
    "OR",
    "VBAR",
    "AMPER",
    "CIRCUMFLEX",
    "LEFTSHIFT",
    "RIGHTSHIFT",
    NK::ADD_SUB_OP,
    NK::MUL_DIV_OP,
    NK::COMP_OP,
    NK::SHIFT_OP,
];

/// Format Catnip source code
pub fn format_code(source: &str, config: &FormatConfig) -> Result<String, String> {
    // Preserve shebang if present
    let (shebang, code) = if source.starts_with("#!") {
        if let Some(newline_pos) = source.find('\n') {
            let shebang = &source[..=newline_pos];
            let code = &source[newline_pos + 1..];
            (Some(shebang), code)
        } else {
            (Some(source), "")
        }
    } else {
        (None, source)
    };

    // Tokenize directly in Rust (no Python roundtrip)
    let language = crate::get_language();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| format!("Failed to set language: {}", e))?;
    let tree = parser.parse(code, None).ok_or("Parse failed")?;
    let mut tokens = Vec::new();
    extract_tokens(tree.root_node(), code, &mut tokens);
    tokens.sort_by(|a, b| a.line.cmp(&b.line).then(a.column.cmp(&b.column)));

    // Inject NEWLINE tokens between lines
    tokens = inject_newlines(tokens);

    // Apply formatting rules (returns line map: output line → source line)
    let (formatted, mut line_map) = apply_formatting_rules(&tokens, config);

    // Join continuation lines that fit within line_length
    let formatted = join_short_lines(&formatted, config, &mut line_map);

    // Wrap long lines
    let formatted = wrap_long_lines(&formatted, config, &mut line_map);

    // Align columns: preserve existing alignment, don't create new
    let formatted = align_columns(&formatted, code, &line_map, config);

    // Normalize newlines (max 2 consecutive)
    let formatted = normalize_newlines(&formatted);

    // Ensure single trailing newline
    let formatted = formatted.trim_end_matches('\n').to_string() + "\n";

    if let Some(shebang) = shebang {
        Ok(format!("{}{}", shebang, formatted))
    } else {
        Ok(formatted)
    }
}

/// Inject NEWLINE tokens between tokens on different lines
fn inject_newlines(tokens: Vec<Token>) -> Vec<Token> {
    let mut result = Vec::new();
    let mut prev_line = 0;

    for token in tokens {
        if token.line > prev_line && prev_line > 0 {
            let gap = token.line - prev_line;
            for j in 0..gap {
                result.push(Token {
                    type_: "_NEWLINE".to_string(),
                    value: "\n".to_string(),
                    line: prev_line + j,
                    column: 999,
                    end_line: prev_line + j,
                    end_column: 999,
                });
            }
        }
        prev_line = token.end_line.max(token.line);
        result.push(token);
    }

    result
}

/// Brace context: block `{ }` vs struct instantiation `Name{...}`
#[derive(Clone, Copy, PartialEq)]
enum BraceKind {
    Block,
    StructInit,
}

fn apply_formatting_rules(tokens: &[Token], config: &FormatConfig) -> (String, Vec<usize>) {
    let mut result = Vec::new();
    let mut indent_level = 0;
    let mut paren_depth: usize = 0;
    let mut at_line_start = true;
    let mut prev_token: Option<&Token> = None;
    let mut prev_significant: Option<&Token> = None;
    let mut brace_stack: Vec<BraceKind> = Vec::new();
    let mut after_block_keyword = false;
    let mut after_at = false; // decorator tracking: true after seeing '@'

    // Line map: for each output line, the source line it comes from
    let mut line_map: Vec<usize> = Vec::new();
    let mut current_source_line: usize = 0;
    let mut output_line_started = false;

    for (i, token) in tokens.iter().enumerate() {
        let next_token = tokens.get(i + 1);

        // Handle comments
        if token.type_ == "COMMENT" {
            current_source_line = token.line.saturating_sub(1); // 1-based -> 0-based
            if at_line_start {
                if !output_line_started {
                    line_map.push(current_source_line);
                    output_line_started = true;
                }
                let effective = indent_level + paren_depth;
                result.push(" ".repeat(effective * config.indent_size));
                at_line_start = false;
            } else {
                // Remove trailing space fragments (from needs_space_after) and
                // ensure exactly 2 spaces before inline comment
                while result.last().is_some_and(|s| s == " ") {
                    result.pop();
                }
                if !result.is_empty() && !result.last().unwrap().ends_with('\n') {
                    result.push("  ".to_string());
                }
            }
            result.push(token.value.clone());
            prev_token = Some(token);
            prev_significant = Some(token);
            continue;
        }

        // Handle newlines
        if token.type_ == "_NEWLINE" {
            // Avoid trailing whitespace at end of formatted lines.
            if let Some(last) = result.last_mut() {
                let trimmed = last.trim_end_matches(' ').to_string();
                *last = trimmed;
            }
            // If no token started this line yet, record a blank line
            if !output_line_started {
                line_map.push(current_source_line);
            }
            result.push("\n".to_string());
            at_line_start = true;
            output_line_started = false;
            prev_token = Some(token);
            continue;
        }

        // Adjust paren depth before indentation (closing parens align with opener)
        if token.type_ == "RPAR" || token.type_ == "RBRACKET" {
            paren_depth = paren_depth.saturating_sub(1);
        }

        // Track source line for non-synthetic tokens (1-based -> 0-based)
        current_source_line = token.line.saturating_sub(1);

        // Indentation at line start
        if at_line_start && token.type_ != "_NEWLINE" {
            if !output_line_started {
                line_map.push(current_source_line);
                output_line_started = true;
            }
            let effective = indent_level + paren_depth;
            let indent_str = " ".repeat(effective * config.indent_size);
            if !indent_str.is_empty() {
                result.push(indent_str);
            }
            at_line_start = false;
        }

        // For unary detection: use prev_significant (skips newlines)
        let unary_context = if prev_token.is_some() && prev_token.unwrap().type_ == "_NEWLINE" {
            prev_significant
        } else {
            prev_token
        };

        let in_struct_init = brace_stack.last() == Some(&BraceKind::StructInit);

        // Adjust indentation level
        if token.type_ == "LBRACE" {
            // Struct instantiation: Name{...} - no space, no indent
            let is_struct_init = prev_token.is_some_and(|p| p.type_ == "NAME") && !after_block_keyword;

            if is_struct_init {
                brace_stack.push(BraceKind::StructInit);
            } else {
                if let Some(prev) = prev_token {
                    if prev.type_ != "_NEWLINE" && !result.is_empty() && !result.last().unwrap().ends_with(' ') {
                        result.push(" ".to_string());
                    }
                }
                brace_stack.push(BraceKind::Block);
                indent_level += 1;
            }
            result.push(token.value.clone());
            after_block_keyword = false;
        } else if token.type_ == "RBRACE" {
            let kind = brace_stack.pop().unwrap_or(BraceKind::Block);

            if kind == BraceKind::Block {
                indent_level = indent_level.saturating_sub(1);
                // Re-indent leading whitespace only when } starts a new line
                if result.len() >= 2 {
                    let is_line_indent = result[result.len() - 1].chars().all(|c| c == ' ')
                        && !result[result.len() - 1].is_empty()
                        && result[result.len() - 2].ends_with('\n');
                    if is_line_indent {
                        let effective = indent_level + paren_depth;
                        *result.last_mut().unwrap() = " ".repeat(effective * config.indent_size);
                    }
                }
                // Space before } when on same line as content (not empty block)
                if let Some(prev) = prev_token {
                    if prev.type_ != "_NEWLINE" && prev.type_ != "LBRACE" {
                        if let Some(last) = result.last() {
                            if !last.ends_with(' ') && !last.ends_with('\n') {
                                result.push(" ".to_string());
                            }
                        }
                    }
                }
            }
            // StructInit: no space before }, no indent adjustment
            result.push(token.value.clone());
        } else {
            // Space before token if needed
            if needs_space_before(token, unary_context, paren_depth, in_struct_init) && !result.is_empty() {
                if let Some(last) = result.last() {
                    if !last.ends_with(&[' ', '\n'][..]) {
                        result.push(" ".to_string());
                    }
                }
            }

            result.push(token.value.clone());

            // Multi-line tokens (strings): each embedded newline creates an
            // output line that needs a line_map entry.
            // Also reset state so the next token correctly starts a new line.
            let extra_newlines = token.value.matches('\n').count();
            if extra_newlines > 0 {
                let base = token.line.saturating_sub(1);
                for k in 1..extra_newlines {
                    line_map.push(base + k);
                }
                // Last newline: the content after it is on end_line
                line_map.push(token.end_line.saturating_sub(1));
                current_source_line = token.end_line.saturating_sub(1);
                output_line_started = true; // the closing quote IS on this line
                at_line_start = false;
            }

            // Decorator isolation: force newline after decorator name (@name\n)
            if after_at && token.type_ == "NAME" {
                after_at = false;
                let next_is_newline = next_token.is_none_or(|t| t.type_ == "_NEWLINE");
                if !next_is_newline {
                    result.push("\n".to_string());
                    at_line_start = true;
                    output_line_started = false;
                    prev_token = Some(token);
                    prev_significant = Some(token);
                    continue;
                }
            } else {
                after_at = token.type_ == "@";
            }

            // Space after token if needed
            if needs_space_after(token, unary_context, next_token, paren_depth) {
                result.push(" ".to_string());
            }
        }

        // Track paren depth after processing (so opening paren's line isn't indented)
        // ".[" is a single tree-sitter token (broadcast open), counts as bracket.
        if token.type_ == "LPAR" || token.type_ == "LBRACKET" || token.value == ".[" {
            paren_depth += 1;
        }

        // Track block-starter keywords for struct instantiation detection
        if token.type_ != "_NEWLINE" && token.type_ != "COMMENT" {
            if symbols::is_block_starter_keyword(&token.value) {
                after_block_keyword = true;
            }
            if token.value == "=>" || token.type_ == "SEMICOLON" {
                after_block_keyword = false;
            }
        }

        prev_token = Some(token);
        prev_significant = Some(token);
    }

    // Record the last output line if it wasn't followed by a newline
    if output_line_started {
        // already recorded
    } else if !result.is_empty() {
        line_map.push(current_source_line);
    }

    (result.join(""), line_map)
}

/// Determine if a MINUS/PLUS/TILDE is unary based on the previous token
fn is_unary_context(prev_token: Option<&Token>) -> bool {
    let Some(prev) = prev_token else {
        return true;
    };

    matches!(
        prev.type_.as_str(),
        "LPAR"
            | "LBRACKET"
            | "COMMA"
            | "_NEWLINE"
            | "ARROW"
            | "EQUAL"
            | "SEMICOLON"
            | "PLUS"
            | "MINUS"
            | "STAR"
            | "SLASH"
            | "DOUBLE_SLASH"
            | "PERCENT"
            | "DOUBLE_STAR"
            | "EQEQUAL"
            | "NOTEQUAL"
            | "LESS"
            | "LESSEQUAL"
            | "GREATER"
            | "GREATEREQUAL"
            | "AND"
            | "OR"
            | "NOT"
            | "VBAR"
            | "AMPER"
            | "CIRCUMFLEX"
            | "LEFTSHIFT"
            | "RIGHTSHIFT"
            | "LBRACE"
            | "COLON"
    ) || symbols::is_keyword(&prev.value)
}

fn needs_space_before(token: &Token, prev_token: Option<&Token>, paren_depth: usize, in_struct_init: bool) -> bool {
    let Some(prev) = prev_token else {
        return false;
    };

    // After DOT: never add space (method/attribute access)
    if prev.type_ == "DOT" {
        return false;
    }

    // Keyword argument: no space before = inside parens (dict(name="x"), f(a=1))
    if token.type_ == "EQUAL" && paren_depth > 0 {
        return false;
    }

    if prev.value == ".[" {
        return false;
    }

    // Space after { when on same line (but not inside struct init: Name{x, y})
    if prev.type_ == "LBRACE" {
        return !in_struct_init;
    }

    // No space after ( or [
    if prev.type_ == "LPAR" || prev.type_ == "LBRACKET" {
        return false;
    }

    // No space before ) or ]
    if token.type_ == "RPAR" || token.type_ == "RBRACKET" {
        return false;
    }

    if matches!(token.type_.as_str(), "MINUS" | "PLUS" | "TILDE" | "STAR") && is_unary_context(prev_token) {
        return false;
    }

    if BINARY_OPS.contains(&token.type_.as_str()) {
        return true;
    }

    if matches!(
        token.value.as_str(),
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "and" | "or" | "??"
    ) {
        return true;
    }

    if token.value == "=>" {
        return true;
    }

    if symbols::is_keyword(&token.value) && !is_literal_keyword(&token.value) {
        // No space after = in kwargs (dict(numbers=list(...)))
        if prev.type_ == "EQUAL" && paren_depth > 0 {
            return false;
        }
        return prev.type_ != "_NEWLINE";
    }

    // Two consecutive identifiers need a space (e.g. @abstract display)
    if token.type_ == "NAME" && prev.type_ == "NAME" {
        return true;
    }

    if token.type_ == "COMMA" || token.type_ == "SEMICOLON" {
        return false;
    }

    false
}

fn needs_space_after(
    token: &Token,
    prev_token: Option<&Token>,
    next_token: Option<&Token>,
    paren_depth: usize,
) -> bool {
    let Some(next) = next_token else {
        return false;
    };

    // Keyword argument: no space after = inside parens
    if token.type_ == "EQUAL" && paren_depth > 0 {
        return false;
    }

    // Method call: keyword used as method name after DOT (re.match, obj.return_value)
    if let Some(prev) = prev_token {
        if prev.type_ == "DOT" {
            return false;
        }
    }

    // No space before ) or ]
    if next.type_ == "RPAR" || next.type_ == "RBRACKET" {
        return false;
    }

    // No space after ( or [
    if token.type_ == "LPAR" || token.type_ == "LBRACKET" || token.value == ".[" {
        return false;
    }

    // Operator overload: op +(self) - no space between operator symbol and (
    if next.type_ == "LPAR" {
        if let Some(prev) = prev_token {
            if prev.value == "op" {
                return false;
            }
        }
    }

    if matches!(token.type_.as_str(), "MINUS" | "PLUS" | "TILDE" | "STAR") && is_unary_context(prev_token) {
        return false;
    }

    if BINARY_OPS.contains(&token.type_.as_str()) {
        return true;
    }

    if matches!(
        token.value.as_str(),
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "and" | "or" | "??"
    ) {
        return true;
    }

    if token.type_ == "COMMA" || token.type_ == "SEMICOLON" {
        return true;
    }

    // No space before ( unless it's a block starter: if (...), while (...), etc.
    if next.type_ == "LPAR" && !symbols::is_block_starter_keyword(&token.value) {
        return false;
    }

    if symbols::is_keyword(&token.value) && !is_literal_keyword(&token.value) {
        // No space before comma/semicolon (keyword used as value, e.g. (op, a, b))
        if next.type_ == "COMMA" || next.type_ == "SEMICOLON" {
            return false;
        }
        return true;
    }

    if token.type_ == "EQUAL" && next.type_ != "EQUAL" {
        return true;
    }

    if token.value == "=>" {
        return true;
    }

    if token.type_ == "RPAR" && (next.value == "=>" || next.value == "{") {
        return true;
    }

    false
}

/// Track multiline string state: count triple-quote delimiters
fn is_in_multiline_string(line: &str, in_string: bool) -> bool {
    let mut state = in_string;
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if i + 2 < len
            && ((bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"')
                || (bytes[i] == b'\'' && bytes[i + 1] == b'\'' && bytes[i + 2] == b'\''))
        {
            state = !state;
            i += 3;
            continue;
        }
        if state && bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        i += 1;
    }
    state
}

/// Check if a line ends with a continuation token
fn is_continuation_end(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.ends_with(',')
        || trimmed.ends_with('(')
        || trimmed.ends_with('[')
        || trimmed.ends_with('+')
        || trimmed.ends_with('-')
        || trimmed.ends_with('*')
        || trimmed.ends_with('/')
        || trimmed.ends_with('|')
        || trimmed.ends_with('&')
        || trimmed.ends_with('^')
        || trimmed.ends_with('\\')
        || trimmed.ends_with(" and")
        || trimmed.ends_with(" or")
        || trimmed.ends_with("=>")
        || trimmed.ends_with('=')
}

/// Check if a line starts with a continuation token
fn is_continuation_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('+')
        || trimmed.starts_with('-')
        || trimmed.starts_with('*')
        || trimmed.starts_with('/')
        || trimmed.starts_with('|')
        || trimmed.starts_with('&')
        || trimmed.starts_with('^')
        || trimmed.starts_with("and ")
        || trimmed.starts_with("or ")
        || trimmed.starts_with("=>")
        || trimmed.starts_with(')')
        || trimmed.starts_with(']')
        || is_postfix_start(trimmed)
}

/// Check if a line starts with a postfix operator (broadcast .[, method .name, call .())
fn is_postfix_start(trimmed: &str) -> bool {
    if !trimmed.starts_with('.') || trimmed.len() < 2 {
        return false;
    }
    let next = trimmed.as_bytes()[1];
    next == b'[' || next == b'(' || next.is_ascii_alphabetic() || next == b'_'
}

/// Check if a line has a trailing comment (# not inside a string)
fn has_trailing_comment(line: &str) -> bool {
    let mut in_string = false;
    let mut string_char = ' ';
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if !in_string {
            if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
            } else if c == '#' {
                return true;
            }
        } else {
            if c == '\\' {
                chars.next();
            } else if c == string_char {
                in_string = false;
            }
        }
    }
    false
}

fn starts_with_closer(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with(')') || trimmed.starts_with(']')
}

/// A line that contains only closing delimiters (and optional comma/semicolon).
fn is_closer_only(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.chars().all(|c| matches!(c, ')' | ']' | ',' | ';'))
}

/// Return true when the current multiline block has a "magic trailing comma"
/// before a closing ')' or ']'.
fn has_magic_trailing_comma_ahead(lines: &[String], from: usize) -> bool {
    let mut close_idx = None;
    let mut j = from + 1;
    while j < lines.len() {
        if starts_with_closer(&lines[j]) {
            close_idx = Some(j);
            break;
        }
        j += 1;
    }

    let Some(close_idx) = close_idx else {
        return false;
    };

    let mut k = close_idx;
    while k > from {
        k -= 1;
        let candidate = lines[k].trim_end();
        if candidate.is_empty() {
            continue;
        }
        // Strip trailing comment before checking for comma
        let effective = if let Some(col) = find_comment_column(candidate) {
            candidate[..col].trim_end()
        } else {
            candidate
        };
        return effective.ends_with(',');
    }

    false
}

/// Preserve explicit multiline layout when a magic trailing comma is present.
fn should_preserve_multiline(lines: &[String], i: usize) -> bool {
    let current = &lines[i];
    let next = &lines[i + 1];
    let current_trimmed = current.trim_end();

    if !has_magic_trailing_comma_ahead(lines, i) {
        return false;
    }

    if current_trimmed.ends_with('(') || current_trimmed.ends_with('[') {
        return true;
    }

    current_trimmed.ends_with(',') || starts_with_closer(next)
}

/// Detect multiline string concatenation: `"..." +\n "..."`
/// The coder explicitly split a string concat across lines for readability.
fn is_string_concat_lines(current: &str, next_trimmed: &str) -> bool {
    let cur = current.trim_end();
    if !cur.ends_with('+') {
        return false;
    }
    let before_plus = cur[..cur.len() - 1].trim_end();
    let ends_with_string = before_plus.ends_with('"') || before_plus.ends_with('\'');
    let starts_with_string = next_trimmed.starts_with('"') || next_trimmed.starts_with('\'');
    ends_with_string && starts_with_string
}

/// Join continuation lines that fit within line_length
fn join_short_lines(text: &str, config: &FormatConfig, line_map: &mut Vec<usize>) -> String {
    let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    let mut i = 0;
    let mut in_string = false;

    while i + 1 < lines.len() {
        let current = &lines[i];
        let next = &lines[i + 1];

        // Skip lines inside or opening multiline strings
        let after_current = is_in_multiline_string(current, in_string);
        if in_string || after_current {
            in_string = after_current;
            i += 1;
            continue;
        }

        if current.trim().is_empty() || next.trim().is_empty() {
            i += 1;
            continue;
        }

        // Postfix chains (.[broadcast], .method(), .field): fix indent even on
        // commented lines. This runs before the comment/join checks because
        // comments prevent joining but not indent correction.
        let next_trimmed = next.trim_start();
        if is_postfix_start(next_trimmed) {
            // Search backward for existing postfix line in same chain
            let expected_indent = {
                let mut found = None;
                for k in (0..=i).rev() {
                    if is_postfix_start(lines[k].trim_start()) {
                        found = Some(lines[k].len() - lines[k].trim_start().len());
                        break;
                    }
                }
                found.unwrap_or_else(|| {
                    let base = lines[i].len() - lines[i].trim_start().len();
                    base + config.indent_size
                })
            };
            let next_indent = lines[i + 1].len() - lines[i + 1].trim_start().len();
            if next_indent != expected_indent {
                let next_content = lines[i + 1].trim_start().to_string();
                lines[i + 1] = format!("{}{}", " ".repeat(expected_indent), next_content);
            }
            i += 1;
            continue;
        }

        // Don't join lines with comments (content after # would be invisible)
        if has_trailing_comment(current) || has_trailing_comment(next) {
            i += 1;
            continue;
        }

        // Don't join content after { - block content is a new scope, not a continuation.
        // This prevents `-1` after `{` from being pulled up as a binary minus continuation.
        if current.trim_end().ends_with('{') {
            i += 1;
            continue;
        }

        let joinable = is_continuation_end(current) || is_continuation_start(next);
        if !joinable {
            i += 1;
            continue;
        }

        // Preserve stacked closing delimiters at different indent levels
        if is_closer_only(current) && starts_with_closer(next) {
            i += 1;
            continue;
        }

        if should_preserve_multiline(&lines, i) {
            i += 1;
            continue;
        }

        // Preserve explicit multiline string concatenation:
        // the coder chose to split across lines for readability
        if is_string_concat_lines(current, next_trimmed) {
            i += 1;
            continue;
        }

        let separator = if current.trim_end().ends_with(',')
            || (!current.trim_end().ends_with('(')
                && !current.trim_end().ends_with('[')
                && !next_trimmed.starts_with(')')
                && !next_trimmed.starts_with(']'))
        {
            " "
        } else {
            ""
        };
        let joined = format!("{}{}{}", current.trim_end(), separator, next_trimmed);

        if joined.len() <= config.line_length {
            lines[i] = joined;
            lines.remove(i + 1);
            if i + 1 < line_map.len() {
                line_map.remove(i + 1);
            }
        } else {
            // Only re-indent for operator continuations, not comma continuations
            // (dict/list items should stay at the same level).
            // Never re-indent closing delimiters - they already have the correct
            // indent from the first pass (aligned with the opening level).
            if !current.trim_end().ends_with(',') && !starts_with_closer(&lines[i + 1]) {
                let current_indent = lines[i].len() - lines[i].trim_start().len();
                let expected_indent = current_indent + config.indent_size;
                let next_indent = lines[i + 1].len() - lines[i + 1].trim_start().len();
                if next_indent < expected_indent {
                    let next_content = lines[i + 1].trim_start().to_string();
                    lines[i + 1] = format!("{}{}", " ".repeat(expected_indent), next_content);
                }
            }
            i += 1;
        }
    }

    lines.join("\n")
}

/// Break point types for line wrapping, ordered by priority
#[derive(Debug, Clone, Copy, PartialEq)]
enum BreakKind {
    AfterComma,
    BeforeOperator(usize), // operator length; split AFTER operator for line continuation
    BeforeArrow,
    AfterOpenParen,
}

/// Find break points in a line for wrapping
fn find_break_points(line: &str) -> Vec<(usize, BreakKind)> {
    let mut points = Vec::new();
    let mut in_string = false;
    let mut string_char = ' ';
    let mut paren_depth: i32 = 0;
    let bytes = line.as_bytes();
    let len = bytes.len();

    let mut i = 0;
    while i < len {
        let c = bytes[i] as char;

        if !in_string && (c == '"' || c == '\'') {
            in_string = true;
            string_char = c;
            i += 1;
            continue;
        }
        if in_string {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == string_char {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '(' || c == '[' {
            paren_depth += 1;
            if i + 1 < len {
                points.push((i + 1, BreakKind::AfterOpenParen));
            }
        } else if c == ')' || c == ']' {
            paren_depth -= 1;
        }

        if c == ',' && paren_depth > 0 {
            let break_pos = if i + 1 < len && bytes[i + 1] == b' ' {
                i + 2
            } else {
                i + 1
            };
            points.push((break_pos, BreakKind::AfterComma));
        }

        if c == ' ' && i + 1 < len {
            let rest = &line[i + 1..];
            let op_len = if rest.starts_with("and ") || rest.starts_with("or ") {
                if rest.starts_with("and ") { 3 } else { 2 }
            } else if rest.starts_with("+ ")
                || rest.starts_with("- ")
                || rest.starts_with("* ")
                || rest.starts_with("/ ")
                || rest.starts_with("| ")
                || rest.starts_with("& ")
                || rest.starts_with("^ ")
                || rest.starts_with("% ")
            {
                1
            } else if rest.starts_with("== ")
                || rest.starts_with("!= ")
                || rest.starts_with("<= ")
                || rest.starts_with(">= ")
                || rest.starts_with("** ")
                || rest.starts_with("// ")
                || rest.starts_with("<< ")
                || rest.starts_with(">> ")
            {
                2
            } else {
                0
            };

            if op_len > 0 && !(op_len == 1 && rest.starts_with("=>")) && paren_depth == 0 {
                points.push((i + 1, BreakKind::BeforeOperator(op_len)));
            }

            if rest.starts_with("=> ") {
                points.push((i + 1, BreakKind::BeforeArrow));
            }
        }

        i += 1;
    }

    points
}

/// Wrap a single line that exceeds line_length
fn wrap_line(line: &str, config: &FormatConfig) -> String {
    if line.len() <= config.line_length {
        return line.to_string();
    }

    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return line.to_string();
    }

    let break_points = find_break_points(line);
    if break_points.is_empty() {
        return line.to_string();
    }

    let indent_len = line.len() - line.trim_start().len();

    let priority = |kind: &BreakKind| -> u8 {
        match kind {
            BreakKind::AfterComma => 0,
            BreakKind::BeforeOperator(_) => 1,
            BreakKind::BeforeArrow => 2,
            BreakKind::AfterOpenParen => 3,
        }
    };

    // effective line length after split (operator stays on first line)
    let effective_len = |pos: usize, kind: &BreakKind| -> usize {
        match kind {
            BreakKind::BeforeOperator(op_len) => pos + op_len,
            _ => pos,
        }
    };

    let mut best: Option<(usize, BreakKind)> = None;
    for &(pos, kind) in &break_points {
        if effective_len(pos, &kind) > config.line_length || pos <= indent_len {
            continue;
        }
        match best {
            None => best = Some((pos, kind)),
            Some((best_pos, best_kind)) => {
                if priority(&kind) < priority(&best_kind) || (priority(&kind) == priority(&best_kind) && pos > best_pos)
                {
                    best = Some((pos, kind));
                }
            }
        }
    }

    if best.is_none() {
        let mut fallback: Option<(usize, BreakKind)> = None;
        for &(pos, kind) in &break_points {
            if pos <= indent_len {
                continue;
            }
            match fallback {
                None => fallback = Some((pos, kind)),
                Some((_, fb_kind)) => {
                    if priority(&kind) < priority(&fb_kind) {
                        fallback = Some((pos, kind));
                    }
                }
            }
        }
        best = fallback;
    }

    let Some((break_pos, break_kind)) = best else {
        return line.to_string();
    };

    // For operators, split AFTER operator so it stays at end of first line
    // (parser requires trailing operator for line continuation)
    let split_pos = match break_kind {
        BreakKind::BeforeOperator(op_len) => (break_pos + op_len + 1).min(line.len()),
        _ => break_pos,
    };
    let first = line[..split_pos].trim_end();
    let rest = line[split_pos..].trim_start();

    if rest.is_empty() {
        return line.to_string();
    }

    let continuation_indent = " ".repeat(indent_len + config.indent_size);
    let second_line = format!("{}{}", continuation_indent, rest);
    let second_line = wrap_line(&second_line, config);

    format!("{}\n{}", first, second_line)
}

/// Wrap lines that exceed config.line_length
fn wrap_long_lines(text: &str, config: &FormatConfig, line_map: &mut Vec<usize>) -> String {
    let mut result = String::new();
    let mut new_map: Vec<usize> = Vec::new();
    let mut in_string = false;
    for (i, line) in text.split('\n').enumerate() {
        if !result.is_empty() {
            result.push('\n');
        }
        let source_line = line_map.get(i).copied().unwrap_or(0);
        if in_string {
            result.push_str(line);
            new_map.push(source_line);
        } else {
            let wrapped = wrap_line(line, config);
            let output_lines = wrapped.split('\n').count();
            for _ in 0..output_lines {
                new_map.push(source_line);
            }
            result.push_str(&wrapped);
        }
        in_string = is_in_multiline_string(line, in_string);
    }
    *line_map = new_map;
    result
}

/// Find the column of the first alignable `=` in a line.
/// Returns None if no valid assignment `=` is found.
/// Skips `==`, `!=`, `<=`, `>=`, `=>`, and `=` inside parens/brackets/strings.
/// Also returns None when multiple `=` appear at depth 0 (multi-kwarg line).
fn find_assignment_column(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut string_char = ' ';
    let mut depth: usize = 0;
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut first_eq: Option<usize> = None;

    while i < len {
        let c = bytes[i] as char;

        if !in_string {
            if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
                i += 1;
                continue;
            }
            if c == '(' || c == '[' {
                depth += 1;
                i += 1;
                continue;
            }
            if c == ')' || c == ']' {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            if c == '#' {
                break;
            }
            if c == '=' && depth == 0 {
                // Check it's not ==, !=, <=, >=, =>
                let prev = if i > 0 { bytes[i - 1] as char } else { ' ' };
                let next = if i + 1 < len { bytes[i + 1] as char } else { ' ' };
                if prev == '!' || prev == '<' || prev == '>' {
                    i += 1;
                    continue;
                }
                if next == '=' || next == '>' {
                    i += 2;
                    continue;
                }
                // Also skip +=, -=, *=, /=, %=, &=, |=, ^=
                if matches!(prev, '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^') {
                    i += 1;
                    continue;
                }
                if first_eq.is_some() {
                    // Multiple `=` at depth 0: kwargs, not an assignment
                    return None;
                }
                first_eq = Some(i);
            }
        } else {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == string_char {
                in_string = false;
            }
        }
        i += 1;
    }
    first_eq
}

/// Find the column of the trailing `#` comment in a line.
/// Returns None if no trailing comment or if the line is a comment-only line.
fn find_comment_column(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return None; // Comment-only line, not a trailing comment
    }

    let mut in_string = false;
    let mut string_char = ' ';
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let c = bytes[i] as char;

        if !in_string {
            if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
            } else if c == '#' {
                return Some(i);
            }
        } else {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == string_char {
                in_string = false;
            }
        }
        i += 1;
    }
    None
}

/// Find the column of `=>` (arrow) in a match arm line.
/// Returns None if no `=>` found or if the `=>` is inside parens/brackets/strings.
fn find_arrow_column(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut string_char = ' ';
    let mut depth: usize = 0;
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let c = bytes[i] as char;

        if !in_string {
            if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
                i += 1;
                continue;
            }
            if c == '(' || c == '[' {
                depth += 1;
                i += 1;
                continue;
            }
            if c == ')' || c == ']' {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            if c == '#' {
                return None;
            }
            if c == '=' && depth == 0 && i + 1 < len && bytes[i + 1] as char == '>' {
                return Some(i);
            }
        } else {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == string_char {
                in_string = false;
            }
        }
        i += 1;
    }
    None
}

/// Get the leading indentation length (number of spaces) of a line.
fn indent_len(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Per-line mask: true if the line starts inside a string literal.
fn string_line_mask(lines: &[String]) -> Vec<bool> {
    let mut mask = Vec::with_capacity(lines.len());
    let mut in_string = false;
    let mut string_char: u8 = 0;
    let mut triple = false;

    for line in lines {
        mask.push(in_string);
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            let c = bytes[i];
            if in_string {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if triple {
                    if c == string_char && i + 2 < len && bytes[i + 1] == string_char && bytes[i + 2] == string_char {
                        in_string = false;
                        i += 3;
                        continue;
                    }
                } else if c == string_char {
                    in_string = false;
                }
            } else {
                // Comment: rest of line is not code
                if c == b'#' {
                    break;
                }
                if c == b'\'' || c == b'"' {
                    if i + 2 < len && bytes[i + 1] == c && bytes[i + 2] == c {
                        in_string = true;
                        string_char = c;
                        triple = true;
                        i += 3;
                        continue;
                    }
                    in_string = true;
                    string_char = c;
                    triple = false;
                }
            }
            i += 1;
        }
    }
    mask
}

/// Align columns in formatted text (post-processing pass).
/// Alignment triggers when the first 2 lines of a group have the symbol
/// at the same column in the original source.
fn align_columns(text: &str, original: &str, line_map: &[usize], config: &FormatConfig) -> String {
    if !config.align {
        return text.to_string();
    }

    let orig_lines: Vec<&str> = original.split('\n').collect();
    let mut result: Vec<String> = text.split('\n').map(|l| l.to_string()).collect();

    let in_string = string_line_mask(&result);

    // Map each formatted line to its original source line
    let mapped_orig: Vec<&str> = (0..result.len())
        .map(|i| {
            line_map
                .get(i)
                .and_then(|&idx| orig_lines.get(idx))
                .copied()
                .unwrap_or("")
        })
        .collect();

    align_symbol(&mut result, &mapped_orig, &in_string, find_assignment_column, false);
    align_symbol(&mut result, &mapped_orig, &in_string, find_arrow_column, false);
    align_symbol(&mut result, &mapped_orig, &in_string, find_comment_column, false);

    result.join("\n")
}

/// Generic alignment pass: groups consecutive non-empty lines at the same
/// indent level that contain the symbol (found by `find_col`), then pads
/// each line so the symbol aligns to the rightmost column in the group.
///
/// Trigger modes:
/// - `always_align = false`: only if the first 2 original lines have the
///   symbol at the same column (preserve existing alignment)
/// - `always_align = true`: whenever >= 2 lines form a group (create alignment)
///
/// Lines inside string literals (in_string[i] == true) break groups.
fn align_symbol<F>(lines: &mut [String], orig_lines: &[&str], in_string: &[bool], find_col: F, always_align: bool)
where
    F: Fn(&str) -> Option<usize>,
{
    let len = lines.len();
    let mut i = 0;

    while i < len {
        // Skip empty lines and lines inside string literals
        if lines[i].trim().is_empty() || in_string.get(i).copied().unwrap_or(false) {
            i += 1;
            continue;
        }

        let group_indent = indent_len(&lines[i]);

        // Collect group: consecutive non-empty lines at same indent with the symbol
        let mut group: Vec<(usize, usize)> = Vec::new(); // (line_index, symbol_col)
        let mut j = i;
        while j < len {
            let line = &lines[j];
            if line.trim().is_empty() || indent_len(line) != group_indent || in_string.get(j).copied().unwrap_or(false)
            {
                break;
            }
            if let Some(col) = find_col(line) {
                group.push((j, col));
            } else {
                // Line without the symbol breaks the group
                break;
            }
            j += 1;
        }

        let triggered = group.len() >= 2
            && (always_align
                || orig_lines
                    .get(group[0].0)
                    .and_then(|l| find_col(l))
                    .zip(orig_lines.get(group[1].0).and_then(|l| find_col(l)))
                    .is_some_and(|(c0, c1)| c0 == c1));
        if triggered {
            let max_col = group.iter().map(|&(_, col)| col).max().unwrap();

            for &(line_idx, col) in &group {
                if col < max_col {
                    let line = &lines[line_idx];
                    let mut new_line = String::with_capacity(line.len() + (max_col - col));
                    let before = line[..col].trim_end();
                    new_line.push_str(before);
                    let needed = max_col - before.len();
                    for _ in 0..needed {
                        new_line.push(' ');
                    }
                    new_line.push_str(&line[col..]);
                    lines[line_idx] = new_line;
                }
            }
            i = j;
        } else {
            i = j.max(i + 1);
        }
    }
}

fn normalize_newlines(text: &str) -> String {
    let text = text.trim_start_matches('\n');
    let mut result = String::new();
    let mut newline_count = 0;

    for c in text.chars() {
        if c == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push(c);
            }
        } else {
            newline_count = 0;
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_simple_expression() {
        let config = FormatConfig::default();
        let result = format_code("x=1+2", &config).unwrap();
        assert_eq!(result, "x = 1 + 2\n");
    }

    #[test]
    fn test_format_binary_ops_spacing() {
        let config = FormatConfig::default();
        let result = format_code("a+b-c*d/e", &config).unwrap();
        assert_eq!(result, "a + b - c * d / e\n");
    }

    #[test]
    fn test_format_unary_minus() {
        let config = FormatConfig::default();
        let result = format_code("x = -1", &config).unwrap();
        assert_eq!(result, "x = -1\n");
    }

    #[test]
    fn test_format_unary_in_expr() {
        let config = FormatConfig::default();
        let result = format_code("y = a + -b", &config).unwrap();
        assert_eq!(result, "y = a + -b\n");
    }

    #[test]
    fn test_format_preserves_shebang() {
        let config = FormatConfig::default();
        let source = "#!/usr/bin/env catnip\nx=1";
        let result = format_code(source, &config).unwrap();
        assert!(result.starts_with("#!/usr/bin/env catnip\n"));
        assert!(result.contains("x = 1"));
    }

    #[test]
    fn test_format_custom_indent_size() {
        let config = FormatConfig {
            indent_size: 2,
            line_length: 120,
            ..Default::default()
        };
        let source = "{x=1}";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("  x") || result.contains("x = 1"));
    }

    #[test]
    fn test_format_empty_source() {
        let config = FormatConfig::default();
        let result = format_code("", &config).unwrap();
        assert_eq!(result, "\n");
    }

    #[test]
    fn test_normalize_newlines() {
        let text = "a\n\n\n\nb";
        let result = normalize_newlines(text);
        assert_eq!(result, "a\n\nb");
    }

    #[test]
    fn test_normalize_strips_leading_newlines() {
        let text = "\n\n\na = 1";
        let result = normalize_newlines(text);
        assert_eq!(result, "a = 1");
    }

    #[test]
    fn test_no_space_before_closing_paren() {
        let config = FormatConfig::default();
        assert_eq!(format_code("f(not x)", &config).unwrap(), "f(not x)\n");
        assert_eq!(format_code("f(not true)", &config).unwrap(), "f(not true)\n");
        assert_eq!(format_code("g(a, not b)", &config).unwrap(), "g(a, not b)\n");
        assert_eq!(format_code("f(a and b)", &config).unwrap(), "f(a and b)\n");
        assert_eq!(format_code("f(a or b)", &config).unwrap(), "f(a or b)\n");
    }

    #[test]
    fn test_multiline_string_preserved() {
        let config = FormatConfig::default();
        let source = "x = \"\"\"\n  - auth\n  - logging\n  - cache\n\"\"\"";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("  - auth\n  - logging\n  - cache\n"));
    }

    #[test]
    fn test_multiline_string_comma_after_close() {
        let config = FormatConfig::default();
        // Closing triple-quote followed by comma must stay on same line
        let source = "f(\"\"\"\nhello\n\"\"\", x)";
        let result = format_code(source, &config).unwrap();
        assert!(
            result.contains("\"\"\", x)"),
            "comma should stay after closing triple-quote: {result}"
        );
    }

    #[test]
    fn test_multiline_string_concat_preserved() {
        let config = FormatConfig::default();
        // Explicit multiline string concat: formatter must not join
        let source = "msg = \"hello \" +\n    \"world\"";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("+\n"), "string concat should stay multiline: {result}");
    }

    #[test]
    fn test_multiline_non_string_concat_joined() {
        let config = FormatConfig::default();
        // Non-string concat: formatter should join if it fits
        let source = "x = a +\n    b";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = a + b\n");
    }

    #[test]
    fn test_literal_keywords_no_extra_space() {
        let config = FormatConfig::default();
        assert_eq!(
            format_code("f(id=4, active=True, score=88)", &config).unwrap(),
            "f(id=4, active=True, score=88)\n"
        );
        assert_eq!(format_code("x = True", &config).unwrap(), "x = True\n");
        assert_eq!(format_code("x = False", &config).unwrap(), "x = False\n");
        assert_eq!(format_code("x = nil", &config).unwrap(), "x = nil\n");
        assert_eq!(format_code("f(True, False)", &config).unwrap(), "f(True, False)\n");
    }

    // --- Alignment tests ---

    fn align_config() -> FormatConfig {
        FormatConfig {
            align: true,
            ..Default::default()
        }
    }

    #[test]
    fn test_align_preserves_existing_alignment() {
        let config = align_config();
        // Source already aligned (x has extra padding) → preserve
        let source = "x           = 1\nlonger_name = 2\ny           = 3\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x           = 1\nlonger_name = 2\ny           = 3\n");
    }

    #[test]
    fn test_align_does_not_force_unaligned() {
        let config = align_config();
        // Source NOT aligned → don't touch
        let source = "x = 1\nlonger_name = 2\ny = 3\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = 1\nlonger_name = 2\ny = 3\n");
    }

    #[test]
    fn test_align_fixes_broken_alignment() {
        let config = align_config();
        // Source was aligned but new line breaks it → re-align
        let source = "x           = 1\nlonger_name = 2\nvery_long_name = 3\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x              = 1\nlonger_name    = 2\nvery_long_name = 3\n");
    }

    #[test]
    fn test_align_comments_preserves_existing() {
        let config = align_config();
        // Comments already aligned (code has extra padding) → preserve
        let source = "code       # short\nmore_code  # longer comment\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "code       # short\nmore_code  # longer comment\n");
    }

    #[test]
    fn test_align_comments_not_forced() {
        let config = align_config();
        // Comments NOT aligned → don't touch (both have natural 2-space minimum)
        let source = "code # short\nmore_code # longer comment\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "code  # short\nmore_code  # longer comment\n");
    }

    #[test]
    fn test_align_enabled_by_default_no_force() {
        let config = FormatConfig::default();
        // align=true by default, but unaligned source stays unaligned
        let source = "x = 1\nlonger_name = 2\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = 1\nlonger_name = 2\n");
    }

    #[test]
    fn test_align_assignments_blank_line_breaks_group() {
        let config = align_config();
        // Aligned groups separated by blank line
        let source = "x  = 1\nyy = 2\n\na  = 10\nbb = 20\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x  = 1\nyy = 2\n\na  = 10\nbb = 20\n");
    }

    #[test]
    fn test_align_assignments_different_indent_breaks_group() {
        let config = align_config();
        let source = "x = 1\n{\n    y = 2\n}\n";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("x = 1"));
        assert!(result.contains("    y = 2"));
    }

    #[test]
    fn test_align_skips_comparison_operators() {
        let config = align_config();
        let source = "x = 1\nif a == b { }\n";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("x = 1"));
        assert!(result.contains("a == b"));
    }

    #[test]
    fn test_align_single_line_no_change() {
        let config = align_config();
        let source = "x = 1\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = 1\n");
    }

    #[test]
    fn test_align_idempotent_unaligned() {
        let config = align_config();
        let source = "x = 1\nlonger_name = 2\ny = 3\n";
        let first = format_code(source, &config).unwrap();
        let second = format_code(&first, &config).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn test_align_idempotent_aligned() {
        let config = align_config();
        let source = "x           = 1\nlonger_name = 2\n";
        let first = format_code(source, &config).unwrap();
        let second = format_code(&first, &config).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn test_align_comment_only_line_skipped() {
        let config = align_config();
        let source = "# full line comment\ncode  # trailing\n";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("# full line comment"));
    }

    #[test]
    fn test_align_kwargs_not_aligned() {
        let config = align_config();
        // Source not aligned → outer = not aligned either
        let source = "x = f(a=1, b=2)\nlonger_name = g(c=3)\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = f(a=1, b=2)\nlonger_name = g(c=3)\n");
    }

    #[test]
    fn test_align_kwargs_preserves_aligned() {
        let config = align_config();
        // Source already aligned → preserve
        let source = "x           = f(a=1, b=2)\nlonger_name = g(c=3)\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x           = f(a=1, b=2)\nlonger_name = g(c=3)\n");
    }

    #[test]
    fn test_align_multi_kwarg_lines_not_aligned() {
        let config = align_config();
        // Multi-kwarg continuation lines must not trigger alignment
        let source =
            "    name=\"temperature\", unit=\"C\",\n    base=21.0, amplitude=4.0,\n    anomaly_threshold=3.0,\n";
        let result = format_code(source, &config).unwrap();
        // Lines with multiple = at depth 0 are excluded from alignment groups,
        // so single-kwarg line stays untouched too (group < 2)
        assert!(!result.contains("name             ="));
        assert!(!result.contains("base             ="));
    }

    #[test]
    fn test_abs_no_space_before_paren() {
        let config = FormatConfig::default();
        let result = format_code("abs(1)", &config).unwrap();
        assert_eq!(result.trim(), "abs(1)");
    }

    // --- Struct instantiation tests ---

    #[test]
    fn test_struct_instantiation_no_space() {
        let config = FormatConfig::default();
        assert_eq!(format_code("Point{x, y}", &config).unwrap(), "Point{x, y}\n");
    }

    #[test]
    fn test_struct_instantiation_in_call() {
        let config = FormatConfig::default();
        assert_eq!(
            format_code("f(Point{1, 2}, Point{3, 4})", &config).unwrap(),
            "f(Point{1, 2}, Point{3, 4})\n"
        );
    }

    #[test]
    fn test_struct_def_keeps_space() {
        let config = FormatConfig::default();
        let source = "struct Point {\n    x; y;\n}\n";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("struct Point {"));
    }

    #[test]
    fn test_if_block_keeps_space() {
        let config = FormatConfig::default();
        assert_eq!(format_code("if x { 1 }", &config).unwrap(), "if x { 1 }\n");
    }

    #[test]
    fn test_while_block_keeps_space() {
        let config = FormatConfig::default();
        let result = format_code("while x { 1 }", &config).unwrap();
        assert!(result.contains("while x { 1 }"));
    }

    #[test]
    fn test_struct_init_in_expression() {
        let config = FormatConfig::default();
        assert_eq!(format_code("x = Point{1, 2}", &config).unwrap(), "x = Point{1, 2}\n");
    }

    #[test]
    fn test_struct_init_after_arrow() {
        let config = FormatConfig::default();
        let result = format_code("f = () => Point{1, 2}", &config).unwrap();
        assert!(result.contains("=> Point{1, 2}"));
    }

    // --- Keyword before comma ---

    #[test]
    fn test_keyword_before_comma_no_space() {
        let config = FormatConfig::default();
        assert_eq!(format_code("(op, a, b)", &config).unwrap(), "(op, a, b)\n");
    }

    #[test]
    fn test_keyword_before_semicolon_no_space() {
        let config = FormatConfig::default();
        let result = format_code("struct S { op; x; }", &config).unwrap();
        assert!(result.contains("op;"));
    }

    // --- Magic trailing comma with comments ---

    #[test]
    fn test_magic_trailing_comma_with_comment() {
        let config = FormatConfig::default();
        let source = "x = list(\n    a,\n    b,  # comment\n)\n";
        let result = format_code(source, &config).unwrap();
        // Should preserve multiline because of trailing comma before )
        assert!(result.contains("\n    a,\n"));
        assert!(result.contains("\n    b,"));
    }

    // --- Line map / alignment after joining ---

    #[test]
    fn test_align_after_join() {
        let config = FormatConfig {
            align: true,
            ..Default::default()
        };
        // join_short_lines will collapse f(\n    a, b\n) into f(a, b),
        // reducing line count. Alignment must still find the original lines.
        let source = "f(\n    a, b\n)\nx           = 1\nlonger_name = 2\n";
        let result = format_code(source, &config).unwrap();
        let assignment_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.contains('=') && !l.contains("=>") && !l.contains('('))
            .collect();
        assert!(assignment_lines.len() >= 2);
        let cols: Vec<usize> = assignment_lines.iter().map(|l| l.find('=').unwrap()).collect();
        assert_eq!(cols[0], cols[1], "alignment should be preserved after line joining");
    }

    #[test]
    fn test_multiline_block_preserved() {
        let config = FormatConfig::default();
        let source = "if cond {\n    result\n}\n";
        let result = format_code(source, &config).unwrap();
        // Block should stay multi-line (not collapsed to inline)
        assert!(result.contains("{\n"), "block should stay expanded");
        assert!(result.contains("    result\n"), "content should be indented");
    }
}
