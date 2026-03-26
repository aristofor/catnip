// FILE: catnip_rs/src/debug/session.rs
// DebugSession: Rust replacement for catnip/debug/session.py.
// Owns the mpsc channels and spawns the VM execution thread.

use std::collections::HashSet;
use std::sync::mpsc;
use std::time::Duration;

use crate::constants::*;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use catnip_tools::sourcemap::SourceMap;

use super::callback::DebugCallback;
use super::types::*;

/// Session state constants matching Python DebugState.
const STATE_IDLE: u8 = 0;
const STATE_RUNNING: u8 = 1;
const STATE_PAUSED: u8 = 2;
const STATE_FINISHED: u8 = 3;
const STATE_ERROR: u8 = 4;

fn state_name(s: u8) -> &'static str {
    match s {
        STATE_IDLE => "idle",
        STATE_RUNNING => "running",
        STATE_PAUSED => "paused",
        STATE_FINISHED => "finished",
        STATE_ERROR => "error",
        _ => "unknown",
    }
}

/// Wrapper to satisfy pyclass Send+Sync for mpsc::Receiver.
struct SyncReceiver(mpsc::Receiver<DebugEvent>);
unsafe impl Sync for SyncReceiver {}

#[pyclass]
pub struct DebugSession {
    command_tx: Option<mpsc::Sender<DebugAction>>,
    event_rx: Option<SyncReceiver>,
    state: u8,
    breakpoints: HashSet<u32>,
    source_text: String,
    source_bytes: Vec<u8>,
    // Last pause data (for inspect/eval)
    last_pause_line: Option<usize>,
    last_pause_col: Option<usize>,
    last_pause_snippet: Option<String>,
    last_pause_locals_repr: Option<Vec<(String, String)>>,
    last_pause_locals_py: Option<Py<PyDict>>,
    last_pause_call_stack: Option<Vec<(String, u32)>>,
    last_pause_start_byte: Option<u32>,
    // Refs for dynamic breakpoints
    vm_ref: Option<Py<PyAny>>,
    sourcemap_ref: Option<Py<PyAny>>,
    catnip_globals: Option<Py<PyAny>>,
}

#[pymethods]
impl DebugSession {
    #[new]
    fn new(source_text: String) -> Self {
        let source_bytes = source_text.as_bytes().to_vec();
        Self {
            command_tx: None,
            event_rx: None,
            state: STATE_IDLE,
            breakpoints: HashSet::new(),
            source_text,
            source_bytes,
            last_pause_line: None,
            last_pause_col: None,
            last_pause_snippet: None,
            last_pause_locals_repr: None,
            last_pause_locals_py: None,
            last_pause_call_stack: None,
            last_pause_start_byte: None,
            vm_ref: None,
            sourcemap_ref: None,
            catnip_globals: None,
        }
    }

    fn add_breakpoint(&mut self, line: u32) {
        self.breakpoints.insert(line);
    }

    fn remove_breakpoint(&mut self, line: u32) {
        self.breakpoints.remove(&line);
    }

    /// Start the debug session. Spawns a background thread for VM execution.
    fn start(&mut self, py: Python<'_>, catnip: &Bound<'_, PyAny>) -> PyResult<()> {
        // Parse source
        catnip.call_method1("parse", (&self.source_text,))?;

        // Convert line breakpoints to byte offsets
        let mut sm_for_offsets = SourceMap::new(self.source_bytes.clone(), "<input>".to_string());
        let mut byte_offsets: HashSet<u32> = HashSet::new();
        for &line in &self.breakpoints {
            if let Some(offset) = sm_for_offsets.line_to_offset(line as usize) {
                byte_offsets.insert(offset as u32);
            }
        }

        // Force VM mode
        catnip.setattr("vm_mode", "on")?;

        // Create VMExecutor
        let registry = catnip.getattr("registry")?;
        let context = catnip.getattr("context")?;

        // Store globals ref for eval
        self.catnip_globals = Some(context.getattr("globals")?.unbind());

        let vm_executor_mod = py.import("catnip.vm.executor")?;
        let vm_executor_cls = vm_executor_mod.getattr("VMExecutor")?;
        let executor = vm_executor_cls.call1((&registry, &context))?;

        // Set source for error reporting
        let vm_wrapper = executor.getattr("vm")?;
        vm_wrapper.call_method1("set_source", (self.source_bytes.as_slice(), "<input>"))?;

        // Get inner Rust VM for breakpoint setup
        let inner_vm = vm_wrapper.getattr("_vm")?;

        // Compile to get line_table for breakpoint mapping
        let compiler_cls = py.import(PY_MOD_RS)?.getattr("Compiler")?;
        let compiler = compiler_cls.call0()?;
        let code_attr = catnip.getattr("code")?;
        let code_obj = compiler.call_method1("compile", (&code_attr, "<module>"))?;
        let line_table: Vec<u32> = code_obj.getattr("line_table")?.extract()?;

        // Map line breakpoints to bytecode byte offsets
        let mut bp_bytes: HashSet<u32> = HashSet::new();
        let mut seen_lines: HashSet<usize> = HashSet::new();
        let mut sm_for_mapping = SourceMap::new(self.source_bytes.clone(), "<input>".to_string());
        for src_byte in &line_table {
            let (line, _) = sm_for_mapping.byte_to_line_col(*src_byte as usize);
            if self.breakpoints.contains(&(line as u32)) && !seen_lines.contains(&line) {
                bp_bytes.insert(*src_byte);
                seen_lines.insert(line);
            }
        }
        bp_bytes.extend(&byte_offsets);

        // Create channels
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        // Build SourceMap for callback
        let sm = SourceMap::new(self.source_bytes.clone(), "<input>".to_string());

        // Set debug callback and breakpoints on the VM
        let callback = DebugCallback::new(command_rx, event_tx.clone(), sm);
        let callback_py = Py::new(py, callback)?;
        inner_vm.call_method1("set_debug_callback", (callback_py,))?;

        for offset in &bp_bytes {
            inner_vm.call_method1("add_debug_breakpoint", (*offset,))?;
        }

        self.command_tx = Some(command_tx);
        self.event_rx = Some(SyncReceiver(event_rx));
        self.vm_ref = Some(inner_vm.unbind());

        // Store sourcemap ref as PyObject for dynamic breakpoints
        let sm_py_cls = py.import(PY_MOD_RS)?.getattr("SourceMap")?;
        let sm_py = sm_py_cls.call1((self.source_bytes.as_slice(),))?;
        self.sourcemap_ref = Some(sm_py.unbind());

        self.state = STATE_RUNNING;

        // Spawn execution thread
        let executor_ref: Py<PyAny> = executor.unbind();
        let code_ref: Py<PyAny> = code_attr.unbind();
        let event_tx_finish = event_tx;

        std::thread::spawn(move || {
            Python::attach(|py| {
                let executor = executor_ref.bind(py);
                let code = code_ref.bind(py);
                match executor.call_method1("execute", (code,)) {
                    Ok(result) => {
                        let _ = event_tx_finish.send(DebugEvent::Finished(result.unbind()));
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let _ = event_tx_finish.send(DebugEvent::Error(msg));
                    }
                }
            });
        });

        Ok(())
    }

    /// Send a debug action to the paused VM.
    fn send_command(&self, action: &str) -> PyResult<()> {
        let act = DebugAction::parse(action);
        if let Some(ref tx) = self.command_tx {
            tx.send(act)
                .map_err(|_| pyo3::exceptions::PyRuntimeError::new_err("Debug session channel closed"))?;
        }
        Ok(())
    }

    /// Wait for the next debug event.
    ///
    /// Returns: tuple (type_str, data) or None on timeout.
    ///   - ("paused", (line, col, locals_repr, snippet, call_stack, start_byte, locals_py))
    ///   - ("finished", result_repr_str)
    ///   - ("error", error_msg_str)
    #[pyo3(signature = (timeout=None))]
    fn wait_for_event(&mut self, py: Python<'_>, timeout: Option<f64>) -> PyResult<Option<Py<PyAny>>> {
        let rx = match &self.event_rx {
            Some(rx) => &rx.0,
            None => return Ok(None),
        };

        let dur = Duration::from_secs_f64(timeout.unwrap_or(DEBUG_EVENT_WAIT_TIMEOUT_SECS as f64));

        // Release GIL while waiting
        // Safety: rx points to self.event_rx which outlives the closure
        let rx_ptr = SendPtr::new(rx as *const mpsc::Receiver<DebugEvent>);
        let event = py.detach(move || {
            let rx = unsafe { rx_ptr.as_ref() };
            rx.recv_timeout(dur).ok()
        });

        match event {
            Some(DebugEvent::Paused(pause)) => {
                // Store last pause data
                self.last_pause_line = Some(pause.line);
                self.last_pause_col = Some(pause.col);
                self.last_pause_snippet = Some(pause.snippet.clone());
                self.last_pause_locals_repr = Some(pause.locals_repr.clone());
                self.last_pause_locals_py = Some(pause.locals_py.clone_ref(py));
                self.last_pause_call_stack = Some(pause.call_stack.clone());
                self.last_pause_start_byte = Some(pause.start_byte);
                self.state = STATE_PAUSED;

                // Build Python tuple: (line, col, locals_repr, snippet, call_stack, start_byte, locals_py)
                let locals_repr_list = PyList::new(
                    py,
                    pause
                        .locals_repr
                        .iter()
                        .map(|(k, v)| PyTuple::new(py, [k.as_str(), v.as_str()]).unwrap()),
                )?;
                let cs_list = PyList::new(
                    py,
                    pause.call_stack.iter().map(|(name, sb)| {
                        PyTuple::new(
                            py,
                            [
                                name.clone().into_pyobject(py).unwrap().into_any().unbind(),
                                (*sb).into_pyobject(py).unwrap().into_any().unbind(),
                            ],
                        )
                        .unwrap()
                    }),
                )?;

                let data = PyTuple::new(
                    py,
                    [
                        pause.line.into_pyobject(py)?.into_any().unbind(),
                        pause.col.into_pyobject(py)?.into_any().unbind(),
                        locals_repr_list.into_any().unbind(),
                        pause.snippet.into_pyobject(py)?.into_any().unbind(),
                        cs_list.into_any().unbind(),
                        pause.start_byte.into_pyobject(py)?.into_any().unbind(),
                        pause.locals_py.into_any(),
                    ],
                )?;

                let result = PyTuple::new(
                    py,
                    [
                        "paused".into_pyobject(py)?.into_any().unbind(),
                        data.into_any().unbind(),
                    ],
                )?;
                Ok(Some(result.unbind().into_any()))
            }
            Some(DebugEvent::Finished(py_result)) => {
                self.state = STATE_FINISHED;
                let result = PyTuple::new(
                    py,
                    ["finished".into_pyobject(py)?.into_any().unbind(), py_result.into_any()],
                )?;
                Ok(Some(result.unbind().into_any()))
            }
            Some(DebugEvent::Error(msg)) => {
                self.state = STATE_ERROR;
                let result = PyTuple::new(
                    py,
                    [
                        "error".into_pyobject(py)?.into_any().unbind(),
                        msg.into_pyobject(py)?.into_any().unbind(),
                    ],
                )?;
                Ok(Some(result.unbind().into_any()))
            }
            None => Ok(None),
        }
    }

    // --- Getters ---

    #[getter]
    fn state(&self) -> &str {
        state_name(self.state)
    }

    #[getter]
    fn last_pause_line(&self) -> Option<usize> {
        self.last_pause_line
    }

    #[getter]
    fn last_pause_col(&self) -> Option<usize> {
        self.last_pause_col
    }

    #[getter]
    fn last_pause_snippet(&self) -> Option<&str> {
        self.last_pause_snippet.as_deref()
    }

    #[getter]
    fn last_pause_start_byte(&self) -> Option<u32> {
        self.last_pause_start_byte
    }

    #[getter]
    fn last_pause_locals_repr(&self) -> Option<Vec<(String, String)>> {
        self.last_pause_locals_repr.clone()
    }

    #[getter]
    fn last_pause_call_stack(&self) -> Option<Vec<(String, u32)>> {
        self.last_pause_call_stack.clone()
    }

    /// Return the Python dict of locals from the last pause.
    fn get_last_pause_locals_py(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.last_pause_locals_py.as_ref().map(|d| d.clone_ref(py).into_any())
    }

    /// Return the sourcemap PyObject for dynamic breakpoints.
    fn get_sourcemap(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.sourcemap_ref.as_ref().map(|s| s.clone_ref(py).into_any())
    }

    /// Return the inner VM PyObject for dynamic breakpoints.
    fn get_vm(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.vm_ref.as_ref().map(|v| v.clone_ref(py).into_any())
    }

    /// Return the catnip globals PyObject for eval.
    fn get_catnip_globals(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.catnip_globals.as_ref().map(|g| g.clone_ref(py).into_any())
    }

    /// Add a breakpoint at runtime (while paused).
    fn add_runtime_breakpoint(&self, py: Python<'_>, line: u32) -> PyResult<()> {
        if let (Some(vm), Some(sm)) = (&self.vm_ref, &self.sourcemap_ref) {
            let sm_bound = sm.bind(py);
            let offset: Option<usize> = sm_bound.call_method1("line_to_offset", (line,))?.extract()?;
            if let Some(offset) = offset {
                vm.bind(py).call_method1("add_debug_breakpoint", (offset as u32,))?;
            }
        }
        Ok(())
    }

    /// Remove a breakpoint at runtime (while paused).
    fn remove_runtime_breakpoint(&self, py: Python<'_>, line: u32) -> PyResult<()> {
        if let (Some(vm), Some(sm)) = (&self.vm_ref, &self.sourcemap_ref) {
            let sm_bound = sm.bind(py);
            let offset: Option<usize> = sm_bound.call_method1("line_to_offset", (line,))?.extract()?;
            if let Some(offset) = offset {
                vm.bind(py).call_method1("remove_debug_breakpoint", (offset as u32,))?;
            }
        }
        Ok(())
    }
}
