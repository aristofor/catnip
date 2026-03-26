// FILE: catnip_rs/src/debug/callback.rs
// DebugCallback: Rust callable passed to vm.set_debug_callback().
// Receives pause notifications from VM, sends them via mpsc channel,
// then blocks (releasing GIL) waiting for the next command.

use std::sync::mpsc;
use std::time::Duration;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use catnip_tools::sourcemap::SourceMap;

use super::types::*;

/// Wrapper to satisfy pyclass Send+Sync requirement for mpsc::Receiver.
/// Safety: only the VM thread ever calls __call__, so Receiver access is
/// single-threaded. The Sync bound is required by pyclass but never exercised.
struct SyncReceiver(mpsc::Receiver<DebugAction>);

unsafe impl Sync for SyncReceiver {}

#[pyclass]
pub struct DebugCallback {
    command_rx: SyncReceiver,
    event_tx: mpsc::Sender<DebugEvent>,
    source_map: std::sync::Mutex<SourceMap>,
}

impl DebugCallback {
    pub fn new(
        command_rx: mpsc::Receiver<DebugAction>,
        event_tx: mpsc::Sender<DebugEvent>,
        source_map: SourceMap,
    ) -> Self {
        Self {
            command_rx: SyncReceiver(command_rx),
            event_tx,
            source_map: std::sync::Mutex::new(source_map),
        }
    }
}

#[pymethods]
impl DebugCallback {
    /// Called from Rust VM at breakpoints/steps.
    ///
    /// Args: (start_byte, locals_dict, call_stack)
    /// Returns: i32 matching DebugAction
    #[pyo3(signature = (start_byte, locals_dict, call_stack))]
    fn __call__(
        &self,
        py: Python<'_>,
        start_byte: u32,
        locals_dict: &Bound<'_, PyDict>,
        call_stack: &Bound<'_, PyList>,
    ) -> PyResult<i32> {
        // Build PauseEvent with GIL held
        let (line, col) = {
            let mut sm = self.source_map.lock().unwrap();
            sm.byte_to_line_col(start_byte as usize)
        };
        let snippet = {
            let mut sm = self.source_map.lock().unwrap();
            sm.get_snippet(start_byte as usize, start_byte as usize + 1, DEBUG_PAUSE_CONTEXT_LINES)
        };

        // Build locals repr
        let mut locals_repr = Vec::new();
        for (key, value) in locals_dict.iter() {
            let name: String = key.extract()?;
            let repr_str = value.repr()?.to_string();
            locals_repr.push((name, repr_str));
        }
        locals_repr.sort_by(|a, b| a.0.cmp(&b.0));

        // Clone the dict for Python-side access
        let locals_py: Py<PyDict> = locals_dict.copy()?.unbind();

        // Build call stack
        let mut cs = Vec::new();
        for item in call_stack.iter() {
            let tuple: &Bound<'_, PyTuple> = item.cast::<PyTuple>()?;
            let name: String = tuple.get_item(0)?.extract()?;
            let sb: u32 = tuple.get_item(1)?.extract()?;
            cs.push((name, sb));
        }

        let pause = PauseEvent {
            line,
            col,
            locals_repr,
            locals_py,
            snippet,
            call_stack: cs,
            start_byte,
        };

        // Send pause event (ignore error if receiver dropped)
        let _ = self.event_tx.send(DebugEvent::Paused(pause));

        // Release GIL while waiting for command.
        // Safety: rx_ptr points to self.command_rx which outlives the closure
        // (self is alive during __call__). The VM thread is the sole consumer.
        let rx_ptr = SendPtr::new(&self.command_rx.0 as *const mpsc::Receiver<DebugAction>);
        let timeout = Duration::from_secs(DEBUG_COMMAND_TIMEOUT_SECS);
        let max_retries = DEBUG_COMMAND_MAX_RETRIES;

        let action = py.detach(move || {
            let rx = unsafe { rx_ptr.as_ref() };
            for attempt in 0..max_retries {
                match rx.recv_timeout(timeout) {
                    Ok(action) => return action,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        eprintln!(
                            "debug callback: no command received (attempt {}/{})",
                            attempt + 1,
                            max_retries
                        );
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return DebugAction::Continue;
                    }
                }
            }
            eprintln!("debug callback: timeout expired, auto-continuing");
            DebugAction::Continue
        });

        Ok(action.as_i32())
    }
}
