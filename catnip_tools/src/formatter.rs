// FILE: catnip_tools/src/formatter.rs
use crate::config::FormatConfig;
use crate::token::Token;
use crate::tokenizer::extract_tokens;

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
    "add_sub_op",
    "mul_div_op",
    "comp_op",
    "shift_op",
];

const CONTROL_KEYWORDS: &[&str] = &[
    "if",
    "elif",
    "else",
    "while",
    "for",
    "match",
    "return",
    "in",
    "not",
    "struct",
    "trait",
    "method",
    "extends",
    "implements",
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

    // Apply formatting rules
    let formatted = apply_formatting_rules(&tokens, config);

    // Join continuation lines that fit within line_length
    let formatted = join_short_lines(&formatted, config);

    // Wrap long lines
    let formatted = wrap_long_lines(&formatted, config);

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
        prev_line = token.line;
        result.push(token);
    }

    result
}

fn apply_formatting_rules(tokens: &[Token], config: &FormatConfig) -> String {
    let mut result = Vec::new();
    let mut indent_level = 0;
    let mut paren_depth: usize = 0;
    let mut at_line_start = true;
    let mut prev_token: Option<&Token> = None;
    let mut prev_significant: Option<&Token> = None;

    for (i, token) in tokens.iter().enumerate() {
        let next_token = tokens.get(i + 1);

        // Handle comments
        if token.type_ == "COMMENT" {
            if at_line_start {
                let effective = indent_level + paren_depth;
                result.push(" ".repeat(effective * config.indent_size));
                at_line_start = false;
            } else if !result.is_empty() && !result.last().unwrap().ends_with(&[' ', '\n'][..]) {
                result.push("  ".to_string());
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
            result.push("\n".to_string());
            at_line_start = true;
            prev_token = Some(token);
            continue;
        }

        // Adjust paren depth before indentation (closing parens align with opener)
        if token.type_ == "RPAR" || token.type_ == "RBRACKET" {
            paren_depth = paren_depth.saturating_sub(1);
        }

        // Indentation at line start
        if at_line_start && token.type_ != "_NEWLINE" {
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

        // Adjust indentation level
        if token.type_ == "LBRACE" {
            if let Some(prev) = prev_token {
                if prev.type_ != "_NEWLINE"
                    && !result.is_empty()
                    && !result.last().unwrap().ends_with(' ')
                {
                    result.push(" ".to_string());
                }
            }
            result.push(token.value.clone());
            indent_level += 1;
        } else if token.type_ == "RBRACE" {
            indent_level = indent_level.saturating_sub(1);
            if !result.is_empty() {
                if let Some(last) = result.last_mut() {
                    if last.chars().all(|c| c == ' ') && !last.is_empty() {
                        let effective = indent_level + paren_depth;
                        *last = " ".repeat(effective * config.indent_size);
                    }
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
            result.push(token.value.clone());
        } else {
            // Space before token if needed
            if needs_space_before(token, unary_context, paren_depth) && !result.is_empty() {
                if let Some(last) = result.last() {
                    if !last.ends_with(&[' ', '\n'][..]) {
                        result.push(" ".to_string());
                    }
                }
            }

            result.push(token.value.clone());

            // Space after token if needed
            if needs_space_after(token, unary_context, next_token, paren_depth) {
                result.push(" ".to_string());
            }
        }

        // Track paren depth after processing (so opening paren's line isn't indented)
        if token.type_ == "LPAR" || token.type_ == "LBRACKET" {
            paren_depth += 1;
        }

        prev_token = Some(token);
        prev_significant = Some(token);
    }

    result.join("")
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
    ) || CONTROL_KEYWORDS.contains(&prev.value.as_str())
}

fn needs_space_before(token: &Token, prev_token: Option<&Token>, paren_depth: usize) -> bool {
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

    // Space after { when on same line
    if prev.type_ == "LBRACE" {
        return true;
    }

    if matches!(token.type_.as_str(), "MINUS" | "PLUS" | "TILDE") && is_unary_context(prev_token) {
        return false;
    }

    if BINARY_OPS.contains(&token.type_.as_str()) {
        return true;
    }

    if matches!(
        token.value.as_str(),
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "and" | "or"
    ) {
        return true;
    }

    if token.value == "=>" {
        return true;
    }

    if CONTROL_KEYWORDS.contains(&token.value.as_str()) {
        return prev.type_ != "_NEWLINE";
    }

    if prev.type_ == "LPAR" {
        return false;
    }

    if token.type_ == "RPAR" {
        return false;
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

    if matches!(token.type_.as_str(), "MINUS" | "PLUS" | "TILDE") && is_unary_context(prev_token) {
        return false;
    }

    if BINARY_OPS.contains(&token.type_.as_str()) {
        return true;
    }

    if matches!(
        token.value.as_str(),
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "and" | "or"
    ) {
        return true;
    }

    if token.type_ == "COMMA" || token.type_ == "SEMICOLON" {
        return true;
    }

    // not(x) is a function call, no space before paren
    if token.value == "not" && next.type_ == "LPAR" {
        return false;
    }

    if CONTROL_KEYWORDS.contains(&token.value.as_str()) {
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

    if token.type_ == "LPAR" || token.value == ".[" {
        return false;
    }

    if next.type_ == "RPAR" {
        return false;
    }

    false
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
        return candidate.ends_with(',');
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

/// Join continuation lines that fit within line_length
fn join_short_lines(text: &str, config: &FormatConfig) -> String {
    let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    let mut i = 0;

    while i + 1 < lines.len() {
        let current = &lines[i];
        let next = &lines[i + 1];

        if current.trim().is_empty() || next.trim().is_empty() {
            i += 1;
            continue;
        }

        // Don't join lines with comments (content after # would be invisible)
        if has_trailing_comment(current) || has_trailing_comment(next) {
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

        let next_trimmed = next.trim_start();
        let separator = if current.trim_end().ends_with(',') {
            " "
        } else if current.trim_end().ends_with('(') || current.trim_end().ends_with('[') {
            ""
        } else if next_trimmed.starts_with(')') || next_trimmed.starts_with(']') {
            ""
        } else {
            " "
        };
        let joined = format!("{}{}{}", current.trim_end(), separator, next_trimmed);

        if joined.len() <= config.line_length {
            lines[i] = joined;
            lines.remove(i + 1);
        } else {
            // Only re-indent for operator continuations, not comma continuations
            // (dict/list items should stay at the same level)
            if !current.trim_end().ends_with(',') {
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
    BeforeOperator,
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
                if rest.starts_with("and ") {
                    3
                } else {
                    2
                }
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

            if op_len > 0 {
                if !(op_len == 1 && rest.starts_with("=>")) {
                    points.push((i + 1, BreakKind::BeforeOperator));
                }
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

    let priority = |kind: BreakKind| -> u8 {
        match kind {
            BreakKind::AfterComma => 0,
            BreakKind::BeforeOperator => 1,
            BreakKind::BeforeArrow => 2,
            BreakKind::AfterOpenParen => 3,
        }
    };

    let mut best: Option<(usize, BreakKind)> = None;
    for &(pos, kind) in &break_points {
        if pos > config.line_length || pos <= indent_len {
            continue;
        }
        match best {
            None => best = Some((pos, kind)),
            Some((best_pos, best_kind)) => {
                if priority(kind) < priority(best_kind)
                    || (priority(kind) == priority(best_kind) && pos > best_pos)
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
                    if priority(kind) < priority(fb_kind) {
                        fallback = Some((pos, kind));
                    }
                }
            }
        }
        best = fallback;
    }

    let Some((break_pos, _break_kind)) = best else {
        return line.to_string();
    };

    let first = line[..break_pos].trim_end();
    let rest = line[break_pos..].trim_start();

    if rest.is_empty() {
        return line.to_string();
    }

    let continuation_indent = " ".repeat(indent_len + config.indent_size);
    let second_line = format!("{}{}", continuation_indent, rest);
    let second_line = wrap_line(&second_line, config);

    format!("{}\n{}", first, second_line)
}

/// Wrap lines that exceed config.line_length
fn wrap_long_lines(text: &str, config: &FormatConfig) -> String {
    let mut result = String::new();
    for line in text.split('\n') {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&wrap_line(line, config));
    }
    result
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
}
