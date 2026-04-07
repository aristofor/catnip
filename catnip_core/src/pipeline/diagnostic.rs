// FILE: catnip_core/src/pipeline/diagnostic.rs
//! Diagnostics emitted during semantic analysis (non-fatal warnings/hints).

use super::super::ir::IR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticSeverity {
    Warning,
    Hint,
}

#[derive(Debug, Clone)]
pub struct SemanticDiagnostic {
    pub code: String,
    pub message: String,
    pub severity: SemanticSeverity,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Result of semantic analysis: transformed IR + non-fatal diagnostics.
pub struct AnalysisResult {
    pub ir: IR,
    pub diagnostics: Vec<SemanticDiagnostic>,
}
