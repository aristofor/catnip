// FILE: catnip_mcp/src/debug_session.rs
//! Debug session for MCP: runs PurePipeline in a background thread,
//! communicates via mpsc channels.

use std::sync::mpsc;
use std::time::Duration;

use catnip_tools::sourcemap::SourceMap;
use catnip_vm::pipeline::PurePipeline;
use catnip_vm::vm::{DebugCommand, DebugHook, PauseInfo};

/// Event sent from the VM thread to the MCP handler.
pub enum DebugEvent {
    Paused(PausedState),
    Finished(String),
    Error(String),
}

/// Snapshot of VM state at a pause point, enriched with source mapping.
#[derive(Clone)]
pub struct PausedState {
    pub line: usize,
    pub col: usize,
    pub snippet: String,
    pub locals: Vec<(String, String)>,
}

/// Command sent from the MCP handler to the VM thread.
pub enum SessionCommand {
    Continue,
    StepInto,
    StepOver,
    StepOut,
}

/// DebugHook implementation that communicates via channels.
struct ChannelHook {
    event_tx: mpsc::Sender<DebugEvent>,
    command_rx: mpsc::Receiver<SessionCommand>,
    source_map: SourceMap,
}

impl DebugHook for ChannelHook {
    fn on_pause(&mut self, info: &PauseInfo) -> DebugCommand {
        let (line, col) = self.source_map.byte_to_line_col(info.start_byte as usize);
        let snippet = self
            .source_map
            .get_snippet(info.start_byte as usize, info.start_byte as usize + 1, 2);

        let state = PausedState {
            line,
            col,
            snippet,
            locals: info.locals.clone(),
        };

        // Send pause event
        if self.event_tx.send(DebugEvent::Paused(state)).is_err() {
            return DebugCommand::Continue;
        }

        // Wait for command (with timeout to avoid hanging forever)
        match self.command_rx.recv_timeout(Duration::from_secs(60)) {
            Ok(SessionCommand::Continue) => DebugCommand::Continue,
            Ok(SessionCommand::StepInto) => DebugCommand::StepInto,
            Ok(SessionCommand::StepOver) => DebugCommand::StepOver,
            Ok(SessionCommand::StepOut) => DebugCommand::StepOut,
            Err(_) => DebugCommand::Continue,
        }
    }
}

/// An active debug session that owns the VM thread.
pub struct McpDebugSession {
    command_tx: mpsc::Sender<SessionCommand>,
    event_rx: mpsc::Receiver<DebugEvent>,
    last_paused: Option<PausedState>,
    /// Shared handle to the VM's breakpoint set for dynamic add/remove.
    breakpoint_handle: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<usize>>>,
    _thread: std::thread::JoinHandle<()>,
}

impl McpDebugSession {
    /// Start a new debug session. Compiles the source, sets breakpoints,
    /// spawns a thread, and waits for the first event.
    pub fn start(source: String, breakpoints: &[i32]) -> Result<(Self, DebugEvent), String> {
        let (event_tx, event_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        // Channel to receive the breakpoint handle from the VM thread
        let (bp_tx, bp_rx) = mpsc::channel();

        let bp_lines: Vec<usize> = breakpoints.iter().filter(|&&l| l > 0).map(|&l| l as usize).collect();
        let source_clone = source.clone();
        let thread = std::thread::spawn(move || {
            let mut pipeline = match PurePipeline::new() {
                Ok(p) => p,
                Err(e) => {
                    let _ = event_tx.send(DebugEvent::Error(e));
                    return;
                }
            };

            // Set source for line resolution and breakpoints
            pipeline.set_source(&source_clone);
            for &line in &bp_lines {
                pipeline.add_breakpoint(line);
            }

            // Send the breakpoint handle back to the main thread
            let _ = bp_tx.send(pipeline.breakpoint_lines_handle());

            // Set debug hook
            let hook = ChannelHook {
                event_tx: event_tx.clone(),
                command_rx,
                source_map: SourceMap::new(source_clone.as_bytes().to_vec(), "<input>".to_string()),
            };
            pipeline.set_debug_hook(Box::new(hook));

            // Execute
            match pipeline.execute(&source_clone) {
                Ok(val) => {
                    let _ = event_tx.send(DebugEvent::Finished(val.repr_string()));
                }
                Err(e) => {
                    let _ = event_tx.send(DebugEvent::Error(e.to_string()));
                }
            }
        });

        // Receive breakpoint handle from VM thread
        let bp_handle = bp_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Timeout waiting for breakpoint handle".to_string())?;

        // Wait for first event (breakpoint hit or execution complete)
        let first_event = event_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "Timeout waiting for debug session to start".to_string())?;

        let last_paused = if let DebugEvent::Paused(ref state) = first_event {
            Some(state.clone())
        } else {
            None
        };

        let session = McpDebugSession {
            command_tx,
            event_rx,
            last_paused,
            breakpoint_handle: bp_handle,
            _thread: thread,
        };

        Ok((session, first_event))
    }

    /// Send a command and wait for the next event.
    pub fn send_and_wait(&mut self, cmd: SessionCommand) -> Result<DebugEvent, String> {
        self.command_tx
            .send(cmd)
            .map_err(|_| "Debug session channel closed".to_string())?;

        let event = self
            .event_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "Timeout waiting for debug event".to_string())?;

        if let DebugEvent::Paused(ref state) = event {
            self.last_paused = Some(state.clone());
        }

        Ok(event)
    }

    /// Get the last paused state (for inspect).
    pub fn last_paused(&self) -> Option<&PausedState> {
        self.last_paused.as_ref()
    }

    /// Evaluate an expression using a fresh pipeline with the paused locals as globals.
    pub fn eval_expr(&self, expr: &str) -> Result<String, String> {
        let locals = self.last_paused.as_ref().map(|p| &p.locals);

        let mut pipeline = PurePipeline::new()?;
        if let Some(locals) = locals {
            for (name, repr) in locals {
                // Validate name is a safe identifier before constructing code
                if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || name.is_empty()
                    || name.as_bytes()[0].is_ascii_digit()
                {
                    continue;
                }
                // repr comes from Value::repr_string() which produces valid Catnip
                // literals for primitives. Complex types (structs, functions) may
                // fail to parse -- silently skipped.
                let assign = format!("{name} = {repr}");
                let _ = pipeline.execute(&assign);
            }
        }
        match pipeline.execute(expr) {
            Ok(val) => Ok(val.repr_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Add a breakpoint at runtime (thread-safe, takes effect on next instruction check).
    pub fn add_breakpoint(&self, line: usize) {
        self.breakpoint_handle.lock().unwrap().insert(line);
    }

    /// Remove a breakpoint at runtime (thread-safe).
    pub fn remove_breakpoint(&self, line: usize) {
        self.breakpoint_handle.lock().unwrap().remove(&line);
    }
}
