// FILE: catnip_lsp/src/diagnostics.rs
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
fn to_lsp_diagnostic(d: &linter::Diagnostic) -> lsp_types::Diagnostic {
    // catnip lines are 1-indexed, LSP is 0-indexed
    let start_line = d.line.saturating_sub(1) as u32;
    let start_col = d.column as u32;
    let end_line = d.end_line.map(|l| l.saturating_sub(1) as u32).unwrap_or(start_line);
    let end_col = d.end_column.map(|c| c as u32).unwrap_or(start_col);

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

/// Run linter on source and return LSP diagnostics.
pub fn lint_to_diagnostics(source: &str) -> Vec<lsp_types::Diagnostic> {
    let config = catnip_tools::config::LintConfig::default();
    match catnip_tools::linter::lint_code(source, &config) {
        Ok(diags) => diags
            .iter()
            // W200 = formatting diff, already handled by the formatter provider
            .filter(|d| d.code != "W200")
            .map(to_lsp_diagnostic)
            .collect(),
        Err(_) => Vec::new(),
    }
}
