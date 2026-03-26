// FILE: catnip_vm/src/compiler/error.rs
//! Compiler error types for the pure Rust compiler.

use std::fmt;

/// Errors raised during IR → bytecode compilation.
#[derive(Debug, Clone)]
pub enum CompileError {
    /// Syntax-level error (invalid IR structure)
    SyntaxError(String),
    /// Type mismatch during compilation
    TypeError(String),
    /// Invalid value during compilation
    ValueError(String),
    /// Index out of range during compilation
    IndexError(String),
    /// Literal type not supported in standalone mode (Decimal, Imaginary)
    UnsupportedLiteral(String),
    /// Feature not yet implemented in the pure compiler
    NotImplemented(String),
}

/// Result type for compiler operations.
pub type CompileResult<T> = Result<T, CompileError>;

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::SyntaxError(msg) => write!(f, "SyntaxError: {}", msg),
            CompileError::TypeError(msg) => write!(f, "TypeError: {}", msg),
            CompileError::ValueError(msg) => write!(f, "ValueError: {}", msg),
            CompileError::IndexError(msg) => write!(f, "IndexError: {}", msg),
            CompileError::UnsupportedLiteral(msg) => write!(f, "UnsupportedLiteral: {}", msg),
            CompileError::NotImplemented(msg) => write!(f, "NotImplemented: {}", msg),
        }
    }
}

impl std::error::Error for CompileError {}
