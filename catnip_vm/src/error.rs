// FILE: catnip_vm/src/error.rs
//! VM error types -- pure Rust, no PyO3.

use std::fmt;

/// Errors produced by the pure-Rust VM.
#[derive(Debug, Clone)]
pub enum VMError {
    StackUnderflow,
    FrameOverflow,
    NameError(String),
    TypeError(String),
    RuntimeError(String),
    ZeroDivisionError(String),
    ValueError(String),
    IndexError(String),
    KeyError(String),
    MemoryLimitExceeded(String),
    Interrupted,
    Exit(i32),
    Return(super::value::Value),
    Break,
    Continue,
}

impl fmt::Display for VMError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VMError::StackUnderflow => write!(f, "WeirdError: VM stack underflow"),
            VMError::FrameOverflow => write!(f, "WeirdError: VM frame stack overflow"),
            VMError::NameError(s) => write!(f, "{}", catnip_core::constants::format_name_error(s)),
            VMError::TypeError(s) => write!(f, "TypeError: {}", s),
            VMError::RuntimeError(s) => write!(f, "{}", s),
            VMError::ZeroDivisionError(s) => write!(f, "{}", s),
            VMError::ValueError(s) => write!(f, "ValueError: {}", s),
            VMError::IndexError(s) => write!(f, "IndexError: {}", s),
            VMError::KeyError(s) => write!(f, "KeyError: {}", s),
            VMError::MemoryLimitExceeded(s) => write!(f, "{}", s),
            VMError::Interrupted => write!(f, "KeyboardInterrupt"),
            VMError::Exit(code) => write!(f, "exit({})", code),
            VMError::Return(_) => write!(f, "return signal"),
            VMError::Break => write!(f, "break signal"),
            VMError::Continue => write!(f, "continue signal"),
        }
    }
}

impl std::error::Error for VMError {}

pub type VMResult<T> = Result<T, VMError>;
