// FILE: catnip_rs/src/debug/types.rs
// Shared types for debug channels.

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Action sent from debugger frontend to VM callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugAction {
    Continue = 1,
    StepInto = 2,
    StepOver = 3,
    StepOut = 4,
}

impl DebugAction {
    pub fn parse(s: &str) -> Self {
        match s {
            "continue" | "c" => Self::Continue,
            "step_into" | "step" | "s" => Self::StepInto,
            "step_over" | "next" | "n" => Self::StepOver,
            "step_out" | "out" | "o" => Self::StepOut,
            _ => Self::Continue,
        }
    }

    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Snapshot of VM state at a breakpoint.
pub struct PauseEvent {
    pub line: usize,
    pub col: usize,
    pub locals_repr: Vec<(String, String)>,
    pub locals_py: Py<PyDict>,
    pub snippet: String,
    pub call_stack: Vec<(String, u32)>,
    pub start_byte: u32,
}

/// Events sent from VM callback to debugger frontend.
pub enum DebugEvent {
    Paused(PauseEvent),
    Finished(Py<PyAny>),
    Error(String),
}

// Safety: Py<T> is Send. DebugEvent only travels through mpsc channels (moved).
unsafe impl Send for DebugEvent {}

/// Wrapper to pass a raw pointer through Send/Ungil boundaries.
/// Safety: caller guarantees the pointed-to data outlives the closure
/// and is not accessed concurrently.
pub struct SendPtr<T> {
    ptr: *const T,
}

impl<T> SendPtr<T> {
    pub fn new(ptr: *const T) -> Self {
        Self { ptr }
    }

    /// Dereference the inner pointer.
    ///
    /// # Safety
    /// The pointer must still be valid and not concurrently mutated.
    pub unsafe fn as_ref(&self) -> &T {
        &*self.ptr
    }
}

unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

// --- Constants (migrated from Python constants.py) ---

pub const DEBUG_COMMAND_TIMEOUT_SECS: u64 = 60;
pub const DEBUG_COMMAND_MAX_RETRIES: u32 = 5;
pub const DEBUG_PAUSE_CONTEXT_LINES: usize = 2;
pub const DEBUG_EVENT_WAIT_TIMEOUT_SECS: u64 = 30;
pub const DEBUG_LIST_CONTEXT_LINES: usize = 5;
