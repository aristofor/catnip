// FILE: catnip_repl/src/signal.rs
//! Temporary SIGINT handler for interrupting VM execution.
//!
//! In crossterm raw mode, ISIG is disabled so Ctrl+C doesn't generate
//! SIGINT. We re-enable ISIG and install a custom handler before
//! execution, then restore everything after.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

static mut INTERRUPT_FLAG: Option<*const AtomicBool> = None;

/// SIGINT handler: set the flag and nothing else (async-signal-safe).
extern "C" fn sigint_handler(_sig: libc::c_int) {
    // SAFETY: async-signal-safe -- only reads the `INTERRUPT_FLAG` pointer and does
    // a relaxed atomic store. The pointer is valid while a `SigintGuard` lives (it
    // owns the `Arc`), which is exactly the window the handler is installed for.
    unsafe {
        if let Some(ptr) = INTERRUPT_FLAG {
            (*ptr).store(true, Ordering::Relaxed);
        }
    }
}

/// RAII guard that installs a SIGINT handler + re-enables ISIG on creation,
/// and restores the previous handler + disables ISIG on drop.
pub struct SigintGuard {
    prev_action: libc::sigaction,
    prev_termios: libc::termios,
    _flag: Arc<AtomicBool>,
}

impl SigintGuard {
    /// Install a SIGINT handler that sets `flag` to true.
    /// Re-enables ISIG so the terminal generates SIGINT on Ctrl+C.
    pub fn new(flag: Arc<AtomicBool>) -> Option<Self> {
        // SAFETY: REPL setup runs single-threaded; the libc termios/sigaction calls
        // act on STDIN/SIGINT with locally zeroed structs, and `INTERRUPT_FLAG` is
        // set to a pointer into `flag`, which the returned guard keeps alive.
        unsafe {
            // Store flag pointer for the signal handler
            INTERRUPT_FLAG = Some(Arc::as_ptr(&flag));

            // Save current termios
            let mut prev_termios: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut prev_termios) != 0 {
                INTERRUPT_FLAG = None;
                return None;
            }

            // Re-enable ISIG so Ctrl+C generates SIGINT
            let mut raw = prev_termios;
            raw.c_lflag |= libc::ISIG;
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);

            // Install SIGINT handler
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = sigint_handler as *const () as usize;
            libc::sigemptyset(&mut sa.sa_mask);
            sa.sa_flags = libc::SA_RESTART;

            let mut prev_action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGINT, &sa, &mut prev_action);

            Some(Self {
                prev_action,
                prev_termios,
                _flag: flag,
            })
        }
    }
}

impl Drop for SigintGuard {
    fn drop(&mut self) {
        // SAFETY: single-threaded teardown -- restores the previously saved SIGINT
        // action and termios, then clears `INTERRUPT_FLAG` before `flag` is dropped,
        // so the handler can no longer dereference it.
        unsafe {
            // Restore previous SIGINT handler
            libc::sigaction(libc::SIGINT, &self.prev_action, std::ptr::null_mut());
            // Restore previous termios (ISIG disabled = raw mode)
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.prev_termios);
            // Clear global pointer
            INTERRUPT_FLAG = None;
        }
    }
}
