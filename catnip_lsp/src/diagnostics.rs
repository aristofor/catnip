// FILE: catnip_lsp/src/diagnostics.rs
use crate::encoding::PositionEncoding;
use catnip_tools::linter;
use tower_lsp::lsp_types;

/// Convert catnip_tools severity to LSP severity.
fn to_lsp_severity(s: linter::Severity) -> lsp_types::DiagnosticSeverity {
    match s {
        linter::Severity::Error => lsp_types::DiagnosticSeverity::ERROR,
        linter::Severity::Warning => lsp_types::DiagnosticSeverity::WARNING,
        linter::Severity::Info => lsp_types::DiagnosticSeverity::INFORMATION,
        linter::Severity::Hint => lsp_types::DiagnosticSeverity::HINT,
    }
}

/// Convert a catnip_tools Diagnostic to an LSP Diagnostic.
///
/// catnip lines/columns are 1-indexed, LSP is 0-indexed (both axes). The
/// linter's columns are byte offsets within their line, so they are re-encoded
/// to the negotiated `Position.character` unit using the source lines.
fn to_lsp_diagnostic(d: &linter::Diagnostic, lines: &[&str], enc: PositionEncoding) -> lsp_types::Diagnostic {
    let start_line = d.line.saturating_sub(1) as u32;
    let end_line = d.end_line.map(|l| l.saturating_sub(1) as u32).unwrap_or(start_line);

    let start_byte_col = d.column.saturating_sub(1);
    let start_col = enc.encode_column(line_text(lines, start_line), start_byte_col);
    let end_col = match d.end_column {
        Some(c) => enc.encode_column(line_text(lines, end_line), c.saturating_sub(1)),
        None => start_col,
    };

    lsp_types::Diagnostic {
        range: lsp_types::Range {
            start: lsp_types::Position::new(start_line, start_col),
            end: lsp_types::Position::new(end_line, end_col),
        },
        severity: Some(to_lsp_severity(d.severity)),
        code: Some(lsp_types::NumberOrString::String(d.code.clone())),
        source: Some("catnip".to_string()),
        message: d.message.clone(),
        ..Default::default()
    }
}

/// Line text for a 0-indexed line, or empty when out of range.
fn line_text<'a>(lines: &[&'a str], line: u32) -> &'a str {
    lines.get(line as usize).copied().unwrap_or("")
}

/// Run linter on source and return LSP diagnostics in the negotiated encoding.
pub fn lint_to_diagnostics(source: &str, enc: PositionEncoding) -> Vec<lsp_types::Diagnostic> {
    let config = catnip_tools::config::LintConfig::default();
    let lines: Vec<&str> = source.lines().collect();
    match catnip_tools::linter::lint_code(source, &config) {
        Ok(diags) => diags
            .iter()
            // W100 = formatting diff, already handled by the formatter provider
            .filter(|d| d.code != "W100")
            .map(|d| to_lsp_diagnostic(d, &lines, enc))
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::lint_to_diagnostics;
    use crate::encoding::PositionEncoding;
    use tower_lsp::lsp_types::NumberOrString;

    fn w101_start(src: &str, enc: PositionEncoding) -> (u32, u32) {
        let diags = lint_to_diagnostics(src, enc);
        let w101 = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(c)) if c == "W101"))
            .expect("diagnostic W101 trailing-whitespace attendu");
        (w101.range.start.line, w101.range.start.character)
    }

    #[test]
    fn columns_are_zero_indexed() {
        // `x = 1  ` : l'espace de fin (W101) commence à l'octet 5 (0-indexé). Le
        // linter le rapporte en 1-indexé (colonne 6) ; le LSP doit voir 5.
        let (line, col) = w101_start("x = 1  \n", PositionEncoding::Utf16);
        assert_eq!(line, 0, "ligne 0-indexée");
        assert_eq!(col, 5, "colonne 0-indexée (était 6 avant le fix)");
    }

    #[test]
    fn columns_reencode_to_utf16() {
        // Une chaîne accentuée avant l'espace de fin décale l'offset octet par
        // rapport à l'offset UTF-16. `"café"` = 6 octets visibles mais 5 unités
        // UTF-16 ; l'espace de fin suit, donc octet 11 -> UTF-16 10.
        let src = "x = \"café\"  \n";
        let byte_col = src.find("  \n").unwrap(); // début du whitespace de fin
        let (_, utf16_col) = w101_start(src, PositionEncoding::Utf16);
        let (_, utf8_col) = w101_start(src, PositionEncoding::Utf8);
        assert_eq!(utf8_col as usize, byte_col, "UTF-8 = offset octet");
        assert_eq!(
            utf16_col as usize,
            byte_col - 1,
            "é compte 2 octets mais 1 unité UTF-16"
        );
    }
}
