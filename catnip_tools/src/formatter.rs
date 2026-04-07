// FILE: catnip_tools/src/formatter.rs
use crate::config::FormatConfig;

/// Format Catnip source code
pub fn format_code(source: &str, config: &FormatConfig) -> Result<String, String> {
    crate::pretty::format_code_pretty(source, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pretty::align::normalize_newlines;

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
    fn test_no_assignment_alignment() {
        let config = align_config();
        // Assignment alignment is disabled -- `=` stays at natural position
        let source = "x = 1\nlonger_name = 2\ny = 3\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = 1\nlonger_name = 2\ny = 3\n");
    }

    #[test]
    fn test_no_assignment_alignment_strips_existing() {
        let config = align_config();
        // Existing padding around `=` is stripped
        let source = "x           = 1\nlonger_name = 2\ny           = 3\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = 1\nlonger_name = 2\ny = 3\n");
    }

    #[test]
    fn test_align_comments_preserves_existing() {
        let config = align_config();
        let source = "code       # short\nmore_code  # longer comment\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "code       # short\nmore_code  # longer comment\n");
    }

    #[test]
    fn test_align_comments_not_forced() {
        let config = align_config();
        let source = "code # short\nmore_code # longer comment\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "code  # short\nmore_code  # longer comment\n");
    }

    #[test]
    fn test_align_enabled_by_default_no_force() {
        let config = FormatConfig::default();
        let source = "x = 1\nlonger_name = 2\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = 1\nlonger_name = 2\n");
    }

    #[test]
    fn test_assignments_no_alignment_padding() {
        let config = align_config();
        let source = "x  = 1\nyy = 2\n\na  = 10\nbb = 20\n";
        let result = format_code(source, &config).unwrap();
        // No alignment: extra spaces before `=` are stripped
        assert_eq!(result, "x = 1\nyy = 2\n\na = 10\nbb = 20\n");
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
        let source = "x = f(a=1, b=2)\nlonger_name = g(c=3)\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = f(a=1, b=2)\nlonger_name = g(c=3)\n");
    }

    #[test]
    fn test_kwargs_no_alignment() {
        let config = align_config();
        let source = "x           = f(a=1, b=2)\nlonger_name = g(c=3)\n";
        let result = format_code(source, &config).unwrap();
        assert_eq!(result, "x = f(a=1, b=2)\nlonger_name = g(c=3)\n");
    }

    #[test]
    fn test_align_multi_kwarg_lines_not_aligned() {
        let config = align_config();
        let source =
            "    name=\"temperature\", unit=\"C\",\n    base=21.0, amplitude=4.0,\n    anomaly_threshold=3.0,\n";
        let result = format_code(source, &config).unwrap();
        assert!(!result.contains("name             ="));
        assert!(!result.contains("base             ="));
    }

    #[test]
    fn test_abs_no_space_before_paren() {
        let config = FormatConfig::default();
        let result = format_code("abs(1)", &config).unwrap();
        assert_eq!(result.trim(), "abs(1)");
    }

    // --- Struct definition tests ---

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

    // --- Keyword before comma ---

    #[test]
    fn test_keyword_before_comma_no_space() {
        let config = FormatConfig::default();
        assert_eq!(format_code("(op, a, b)", &config).unwrap(), "(op, a, b)\n");
    }

    // --- Magic trailing comma with comments ---

    #[test]
    fn test_magic_trailing_comma_with_comment() {
        let config = FormatConfig::default();
        let source = "x = list(\n    a,\n    b,  # comment\n)\n";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("\n    a,\n"));
        assert!(result.contains("\n    b,"));
    }

    // --- Line map / alignment after joining ---

    #[test]
    fn test_no_alignment_after_join() {
        let config = FormatConfig {
            align: true,
            ..Default::default()
        };
        let source = "f(a, b)\nx           = 1\nlonger_name = 2\n";
        let result = format_code(source, &config).unwrap();
        // No assignment alignment: padding stripped
        assert!(result.contains("x = 1"), "no padding: {result}");
        assert!(result.contains("longer_name = 2"), "no padding: {result}");
    }

    #[test]
    fn test_multiline_block_preserved() {
        let config = FormatConfig::default();
        let source = "if cond {\n    result\n}\n";
        let result = format_code(source, &config).unwrap();
        assert!(result.contains("{\n"), "block should stay expanded");
        assert!(result.contains("    result\n"), "content should be indented");
    }
}
