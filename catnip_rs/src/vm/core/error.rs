//! VM error types, debug step modes, and execution context structs.

use super::*;

/// VM execution error
#[derive(Debug)]
pub enum VMError {
    StackUnderflow,
    FrameOverflow,
    NameError(String),
    AttributeError(String),
    TypeError(String),
    RuntimeError(String),
    ValueError(String),
    IndexError(String),
    KeyError(String),
    ZeroDivisionError(String),
    MemoryLimitExceeded(String),
    /// User-defined or struct-based exception with full MRO.
    UserException(catnip_core::exception::ExceptionInfo),
    Interrupted,
    Exit(i32),
    Return(Value),
    Break,
    Continue,
}

impl std::fmt::Display for VMError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VMError::StackUnderflow => write!(f, "WeirdError: VM stack underflow"),
            VMError::FrameOverflow => write!(f, "WeirdError: VM frame stack overflow"),
            VMError::NameError(s) => {
                // VM creates NameError with bare name, py_interop with full message
                if s.starts_with("name '") {
                    write!(f, "NameError: {}", s)
                } else {
                    write!(f, "NameError: {}", catnip_core::constants::format_name_error(s))
                }
            }
            VMError::AttributeError(s) => write!(f, "AttributeError: {}", s),
            VMError::TypeError(s) => write!(f, "TypeError: {}", s),
            VMError::RuntimeError(s) => write!(f, "{}", s),
            VMError::ValueError(s) => write!(f, "ValueError: {}", s),
            VMError::IndexError(s) => write!(f, "IndexError: {}", s),
            VMError::KeyError(s) => write!(f, "KeyError: {}", s),
            VMError::ZeroDivisionError(s) => write!(f, "ZeroDivisionError: {}", s),
            VMError::MemoryLimitExceeded(s) => write!(f, "MemoryLimitExceeded: {}", s),
            VMError::UserException(info) => write!(f, "{}: {}", info.type_name, info.message),
            VMError::Interrupted => write!(f, "KeyboardInterrupt"),
            VMError::Exit(code) => write!(f, "exit({})", code),
            VMError::Return(_) => write!(f, "return signal"),
            VMError::Break => write!(f, "break signal"),
            VMError::Continue => write!(f, "continue signal"),
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
                        format!("{:?}", other),
                    ))
                }
            }
        }
    }
}

pub(crate) type VMResult<T> = Result<T, VMError>;

/// Debug stepping mode for interactive debugger.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DebugStepMode {
    /// No stepping, only stop at breakpoints
    Disabled = 0,
    /// Continue until next breakpoint
    Continue = 1,
    /// Stop at every instruction
    StepInto = 2,
    /// Stop when returning to same or shallower depth
    StepOver = 3,
    /// Stop when returning to shallower depth
    StepOut = 4,
}

impl DebugStepMode {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Continue,
            2 => Self::StepInto,
            3 => Self::StepOver,
            4 => Self::StepOut,
            _ => Self::Disabled,
        }
    }
}

/// Call frame info for stack traces.
#[derive(Clone, Debug)]
pub struct CallInfo {
    /// Function name (or "<module>")
    pub name: String,
    /// start_byte of the call site in the source
    pub call_start_byte: u32,
}

/// Error context captured when a VMError is raised.
#[derive(Clone, Debug)]
pub struct ErrorContext {
    /// Error type name ("TypeError", "NameError", etc.)
    pub error_type: String,
    /// Error message
    pub message: String,
    /// Position in source (start_byte) where the error occurred
    pub start_byte: u32,
    /// Call stack snapshot: (function_name, start_byte) per frame
    pub call_stack: Vec<(String, u32)>,
}
