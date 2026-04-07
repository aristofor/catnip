// FILE: catnip_core/src/pipeline/mod.rs
//! Standalone components - pure Rust, no PyO3.

pub mod diagnostic;
pub mod semantic;

pub use diagnostic::{AnalysisResult, SemanticDiagnostic, SemanticSeverity};
pub use semantic::SemanticAnalyzer;
