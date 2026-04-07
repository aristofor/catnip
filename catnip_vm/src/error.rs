// FILE: catnip_vm/src/error.rs
//! VM error types -- pure Rust, no PyO3.

use std::fmt;

/// Higher-order builtin function kind.
#[derive(Debug, Clone, Copy)]
pub enum HofKind {
    Map,
    Filter,
    Fold,
    Reduce,
}

/// Signal from dispatch_inner requesting synchronous HOF execution.
#[derive(Debug, Clone)]
pub struct HofCall {
    pub kind: HofKind,
    pub func: super::value::Value,
    pub iterable: super::value::Value,
    pub init: Option<super::value::Value>,
}

/// Errors produced by the pure-Rust VM.
#[derive(Debug, Clone)]
pub enum VMError {
    StackUnderflow,
    FrameOverflow,
    NameError(String),
    AttributeError(String),
    TypeError(String),
    RuntimeError(String),
    ZeroDivisionError(String),
    ValueError(String),
    IndexError(String),
    KeyError(String),
    MemoryLimitExceeded(String),
    /// User-defined or struct-based exception with full MRO.
    UserException(catnip_core::exception::ExceptionInfo),
    Interrupted,
    Exit(i32),
    Return(super::value::Value),
    Break,
    Continue,
    /// Signal for higher-order builtin execution (not a real error).
    HofBuiltin(HofCall),
}

impl fmt::Display for VMError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VMError::StackUnderflow => write!(f, "WeirdError: VM stack underflow"),
            VMError::FrameOverflow => write!(f, "WeirdError: VM frame stack overflow"),
            VMError::NameError(s) => write!(f, "{}", catnip_core::constants::format_name_error(s)),
            VMError::AttributeError(s) => write!(f, "AttributeError: {}", s),
            VMError::TypeError(s) => write!(f, "TypeError: {}", s),
            VMError::RuntimeError(s) => write!(f, "{}", s),
            VMError::ZeroDivisionError(s) => write!(f, "{}", s),
            VMError::ValueError(s) => write!(f, "ValueError: {}", s),
            VMError::IndexError(s) => write!(f, "IndexError: {}", s),
            VMError::KeyError(s) => write!(f, "KeyError: {}", s),
            VMError::MemoryLimitExceeded(s) => write!(f, "{}", s),
            VMError::UserException(info) => write!(f, "{}: {}", info.type_name, info.message),
            VMError::Interrupted => write!(f, "KeyboardInterrupt"),
            VMError::Exit(code) => write!(f, "exit({})", code),
            VMError::Return(_) => write!(f, "return signal"),
            VMError::Break => write!(f, "break signal"),
            VMError::Continue => write!(f, "continue signal"),
            VMError::HofBuiltin(_) => write!(f, "HOF builtin signal"),
        }
    }
}

impl std::error::Error for VMError {}

impl VMError {
    /// Extract ExceptionInfo for catchable exceptions.
    pub fn exception_info(&self) -> Option<catnip_core::exception::ExceptionInfo> {
        use catnip_core::exception::{ExceptionInfo, ExceptionKind};
        match self {
            VMError::TypeError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::TypeError, msg.clone())),
            VMError::ValueError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::ValueError, msg.clone())),
            VMError::NameError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::NameError, msg.clone())),
            VMError::IndexError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::IndexError, msg.clone())),
            VMError::KeyError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::KeyError, msg.clone())),
            VMError::AttributeError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::AttributeError, msg.clone())),
            VMError::ZeroDivisionError(msg) => {
                Some(ExceptionInfo::from_kind(ExceptionKind::ZeroDivisionError, msg.clone()))
            }
            VMError::RuntimeError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::RuntimeError, msg.clone())),
            VMError::MemoryLimitExceeded(msg) => {
                Some(ExceptionInfo::from_kind(ExceptionKind::MemoryError, msg.clone()))
            }
            VMError::UserException(info) => Some(info.clone()),
            _ => None,
        }
    }

    /// Reconstruct VMError from stored exception info.
    pub fn from_exception_info(type_name: &str, msg: &str) -> VMError {
        match type_name {
            "TypeError" => VMError::TypeError(msg.into()),
            "ValueError" => VMError::ValueError(msg.into()),
            "NameError" => VMError::NameError(msg.into()),
            "IndexError" => VMError::IndexError(msg.into()),
            "KeyError" => VMError::KeyError(msg.into()),
            "AttributeError" => VMError::AttributeError(msg.into()),
            "ZeroDivisionError" => VMError::ZeroDivisionError(msg.into()),
            "MemoryError" => VMError::MemoryLimitExceeded(msg.into()),
            _ => VMError::RuntimeError(msg.into()),
        }
    }

    /// True for user-catchable exceptions (not control flow or internal errors).
    pub fn is_catchable(&self) -> bool {
        matches!(
            self,
            VMError::TypeError(_)
                | VMError::ValueError(_)
                | VMError::NameError(_)
                | VMError::IndexError(_)
                | VMError::KeyError(_)
                | VMError::AttributeError(_)
                | VMError::ZeroDivisionError(_)
                | VMError::RuntimeError(_)
                | VMError::MemoryLimitExceeded(_)
                | VMError::UserException(_)
        )
    }

    /// Convert to PendingUnwind for finally block processing.
    pub fn to_pending_unwind(&self) -> catnip_core::exception::PendingUnwind {
        use catnip_core::exception::PendingUnwind;
        match self {
            VMError::Return(_) => PendingUnwind::Return,
            VMError::Break => PendingUnwind::Break,
            VMError::Continue => PendingUnwind::Continue,
            other => {
                if let Some(info) = other.exception_info() {
                    PendingUnwind::Exception(info)
                } else {
                    PendingUnwind::Exception(catnip_core::exception::ExceptionInfo::from_name(
                        "RuntimeError".into(),
                        other.to_string(),
                    ))
                }
            }
        }
    }
}

pub type VMResult<T> = Result<T, VMError>;
