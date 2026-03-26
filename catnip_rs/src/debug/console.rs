// FILE: catnip_rs/src/debug/console.rs
// run_debugger(): Rust console debugger replacing ConsoleDebugger.run().
// Releases GIL for the console loop, reacquires for expression eval.

use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

use crate::constants::*;
use pyo3::prelude::*;

use catnip_tools::debugger::{self, DebugCommand};
use catnip_tools::multiline::{preprocess_multiline, should_continue_multiline};
use catnip_tools::sourcemap::SourceMap;

use super::callback::DebugCallback;
use super::types::*;

/// Shared state for debug command handlers.
struct DebugContext {
    source_bytes: Vec<u8>,
    file_label: String,
    command_tx: mpsc::Sender<DebugAction>,
    inner_vm_ref: Py<PyAny>,
    sm_py: Py<PyAny>,
    catnip_globals: Py<PyAny>,
}

/// Result of the REPL sub-mode.
enum ReplResult {
    /// Resume execution with a debug action (c/s/n/o issued from REPL).
    Resume(DebugAction),
    /// Exit REPL, back to debug prompt.
    Exit,
}

// --- Helpers ---

/// Create a Catnip instance with program globals + pause locals merged in.
fn create_scoped_catnip<'py>(
    py: Python<'py>,
    globals: &Py<PyAny>,
    pause: &PauseEvent,
) -> Result<Bound<'py, PyAny>, String> {
    let catnip_mod = py.import("catnip").map_err(|e| format!("{e}"))?;
    let catnip_cls = catnip_mod.getattr("Catnip").map_err(|e| format!("{e}"))?;
    let c = catnip_cls.call0().map_err(|e| format!("{e}"))?;

    let ctx = c.getattr("context").map_err(|e| format!("{e}"))?;
    let g = ctx.getattr("globals").map_err(|e| format!("{e}"))?;
    g.call_method1("update", (globals.bind(py),))
        .map_err(|e| format!("{e}"))?;
    g.call_method1("update", (pause.locals_py.bind(py),))
        .map_err(|e| format!("{e}"))?;

    Ok(c)
}

/// Parse and execute code in a Catnip instance, return repr of result.
fn catnip_eval(_py: Python<'_>, catnip: &Bound<'_, PyAny>, code: &str) -> Result<String, String> {
    catnip.call_method1("parse", (code,)).map_err(|e| format!("{e}"))?;
    let result = catnip.call_method0("execute").map_err(|e| format!("{e}"))?;
    let repr = result.repr().map_err(|e| format!("{e}"))?.to_string();
    Ok(repr)
}

/// Evaluate an expression in a fresh scope (one-shot, for `p` command).
fn eval_expr(py: Python<'_>, expr: &str, catnip_globals: &Py<PyAny>, pause: &PauseEvent) -> Result<String, String> {
    let c = create_scoped_catnip(py, catnip_globals, pause)?;
    catnip_eval(py, &c, expr)
}

/// Read a possibly-multiline input from stdin. Returns None on EOF or /exit.
fn read_multiline_stdin(reader: &mut io::StdinLock, prompt: &str) -> Option<String> {
    eprint!("{prompt}");
    io::stderr().flush().ok();

    let mut buf = String::new();
    match reader.read_line(&mut buf) {
        Ok(0) | Err(_) => {
            eprintln!();
            return None;
        }
        Ok(_) => {}
    }

    let trimmed = buf.trim();
    if trimmed.is_empty() || trimmed == "/exit" {
        return None;
    }

    while should_continue_multiline(&buf) {
        eprint!("... ");
        io::stderr().flush().ok();
        let mut cont = String::new();
        match reader.read_line(&mut cont) {
            Ok(0) | Err(_) => break,
            Ok(_) => buf.push_str(&cont),
        }
    }

    Some(preprocess_multiline(buf.trim()))
}

// --- Debug command handlers ---

fn handle_break(py: Python<'_>, dctx: &DebugContext, line_num: u32) {
    let sm_bound = dctx.sm_py.bind(py);
    let offset: Option<usize> = sm_bound
        .call_method1("line_to_offset", (line_num,))
        .ok()
        .and_then(|r| r.extract().ok());
    if let Some(offset) = offset {
        let _ = dctx
            .inner_vm_ref
            .bind(py)
            .call_method1("add_debug_breakpoint", (offset as u32,));
    }
    eprintln!("Breakpoint set at line {}", line_num);
}

fn handle_remove_break(py: Python<'_>, dctx: &DebugContext, line_num: u32) {
    let sm_bound = dctx.sm_py.bind(py);
    let offset: Option<usize> = sm_bound
        .call_method1("line_to_offset", (line_num,))
        .ok()
        .and_then(|r| r.extract().ok());
    if let Some(offset) = offset {
        let _ = dctx
            .inner_vm_ref
            .bind(py)
            .call_method1("remove_debug_breakpoint", (offset as u32,));
    }
    eprintln!("Breakpoint removed at line {}", line_num);
}

fn handle_list(source_bytes: &[u8], file_label: &str, pause: &PauseEvent) {
    let mut sm = SourceMap::new(source_bytes.to_vec(), file_label.to_string());
    let snippet = sm.get_snippet(
        pause.start_byte as usize,
        pause.start_byte as usize + 1,
        DEBUG_LIST_CONTEXT_LINES,
    );
    if snippet.is_empty() {
        eprintln!("  (no source available)");
    } else {
        for s in snippet.lines() {
            eprintln!("  {}", s);
        }
    }
}

fn handle_backtrace(source_bytes: &[u8], file_label: &str, pause: &PauseEvent) {
    let mut sm = SourceMap::new(source_bytes.to_vec(), file_label.to_string());
    let frames: Vec<(String, u32)> = pause
        .call_stack
        .iter()
        .rev()
        .map(|(name, sb)| {
            let (line, _) = sm.byte_to_line_col(*sb as usize);
            (name.clone(), line as u32)
        })
        .collect();
    eprintln!("{}", debugger::format_backtrace(&frames));
}

// --- REPL sub-mode ---

/// Interactive REPL sub-mode with debug command dispatch.
///
/// Expressions are evaluated in a persistent scope. Debug commands (vars,
/// list, backtrace, break, etc.) are available. Movement commands (c/s/n/o)
/// exit the REPL and resume execution.
fn run_repl_submode(py: Python<'_>, dctx: &DebugContext, pause: &PauseEvent) -> ReplResult {
    let catnip = match create_scoped_catnip(py, &dctx.catnip_globals, pause) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  Error initializing REPL: {e}");
            return ReplResult::Exit;
        }
    };

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        let prompt = format!("(repl:L{}) => ", pause.line);
        let code = match read_multiline_stdin(&mut reader, &prompt) {
            Some(c) => c,
            None => return ReplResult::Exit,
        };

        match debugger::parse_command(&code) {
            // Movement -> resume execution directly from REPL
            DebugCommand::Continue => return ReplResult::Resume(DebugAction::Continue),
            DebugCommand::StepInto => return ReplResult::Resume(DebugAction::StepInto),
            DebugCommand::StepOver => return ReplResult::Resume(DebugAction::StepOver),
            DebugCommand::StepOut => return ReplResult::Resume(DebugAction::StepOut),

            // Debug info commands
            DebugCommand::Vars => eprintln!("{}", debugger::format_vars(&pause.locals_repr)),
            DebugCommand::List => handle_list(&dctx.source_bytes, &dctx.file_label, pause),
            DebugCommand::Backtrace => handle_backtrace(&dctx.source_bytes, &dctx.file_label, pause),
            DebugCommand::Break(n) => handle_break(py, dctx, n),
            DebugCommand::RemoveBreak(n) => handle_remove_break(py, dctx, n),
            DebugCommand::Help => eprintln!("{}", debugger::format_help()),

            // Explicit eval with prefix
            DebugCommand::Print(expr) => match catnip_eval(py, &catnip, &expr) {
                Ok(repr) => eprintln!("  = {repr}"),
                Err(e) => eprintln!("  Error: {e}"),
            },

            DebugCommand::Quit => {
                eprintln!("Aborting.");
                std::process::exit(0);
            }
            DebugCommand::Repl => eprintln!("Already in REPL mode."),
            DebugCommand::Repeat => {} // empty line handled by read_multiline_stdin

            // Unknown -> evaluate as Catnip expression (fallback)
            DebugCommand::Unknown(_) => match catnip_eval(py, &catnip, &code) {
                Ok(repr) => {
                    if repr != "None" {
                        eprintln!("  {repr}");
                    }
                }
                Err(e) => eprintln!("  Error: {e}"),
            },
        }
    }
}

// --- Main entry point ---

/// Launch an interactive debug session from Rust.
///
/// This replaces ConsoleDebugger.run() entirely:
/// - Creates channels and DebugCallback internally
/// - Releases GIL for the console loop
/// - Reacquires GIL only for expression evaluation
#[pyfunction]
#[pyo3(signature = (source, breakpoints, verbose=false, no_color=false, catnip_instance=None, filename=None))]
pub fn run_debugger(
    py: Python<'_>,
    source: String,
    breakpoints: Vec<u32>,
    verbose: bool,
    no_color: bool,
    catnip_instance: Option<&Bound<'_, PyAny>>,
    filename: Option<String>,
) -> PyResult<i32> {
    let _ = verbose;
    let _ = no_color;
    let source_bytes = source.as_bytes().to_vec();
    let file_label = filename.as_deref().unwrap_or("<input>").to_string();

    // --- Setup phase (with GIL) ---

    // Use provided Catnip instance or create a fresh one
    let catnip = if let Some(inst) = catnip_instance {
        inst.clone()
    } else {
        let catnip_mod = py.import("catnip")?;
        let catnip_cls = catnip_mod.getattr("Catnip")?;
        catnip_cls.call0()?
    };

    // Save original vm_mode for restoration after debug session
    let original_vm_mode: String = catnip
        .getattr("vm_mode")
        .and_then(|v| v.extract())
        .unwrap_or_else(|_| "on".to_string());
    let catnip_ref: Py<PyAny> = catnip.clone().unbind();

    catnip.call_method1("parse", (&source,))?;
    catnip.setattr("vm_mode", "on")?;

    // Set META.file if a filename was provided
    if let Some(ref fname) = filename {
        if let Ok(ctx) = catnip.getattr("context") {
            if let Ok(g) = ctx.getattr("globals") {
                if let Ok(meta) = g.call_method1("get", ("META",)) {
                    if !meta.is_none() {
                        let _ = meta.setattr("file", fname.as_str());
                    }
                }
            }
        }
    }

    // Build SourceMap for callback
    let sm = SourceMap::new(source_bytes.clone(), file_label.clone());

    // Convert line breakpoints to byte offsets
    let mut sm_for_offsets = SourceMap::new(source_bytes.clone(), file_label.clone());
    let mut line_bp_offsets: Vec<u32> = Vec::new();
    for &line in &breakpoints {
        if let Some(offset) = sm_for_offsets.line_to_offset(line as usize) {
            line_bp_offsets.push(offset as u32);
        }
    }

    // Create VMExecutor
    let registry = catnip.getattr("registry")?;
    let context = catnip.getattr("context")?;
    let catnip_globals: Py<PyAny> = context.getattr("globals")?.unbind();

    let vm_executor_mod = py.import("catnip.vm.executor")?;
    let vm_executor_cls = vm_executor_mod.getattr("VMExecutor")?;
    let executor = vm_executor_cls.call1((&registry, &context))?;

    // Set source for error reporting
    let vm_wrapper = executor.getattr("vm")?;
    vm_wrapper.call_method1("set_source", (source_bytes.as_slice(), file_label.as_str()))?;
    let inner_vm = vm_wrapper.getattr("_vm")?;

    // Compile to get line_table
    let compiler_cls = py.import(PY_MOD_RS)?.getattr("Compiler")?;
    let compiler = compiler_cls.call0()?;
    let code_attr = catnip.getattr("code")?;
    let code_obj = compiler.call_method1("compile", (&code_attr, "<module>"))?;
    let line_table: Vec<u32> = code_obj.getattr("line_table")?.extract()?;

    // Map line breakpoints to bytecode offsets
    let bp_set: std::collections::HashSet<u32> = breakpoints.iter().copied().collect();
    let mut bp_bytes: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut seen_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut sm_for_mapping = SourceMap::new(source_bytes.clone(), "<input>".to_string());
    for src_byte in &line_table {
        let (line, _) = sm_for_mapping.byte_to_line_col(*src_byte as usize);
        if bp_set.contains(&(line as u32)) && !seen_lines.contains(&line) {
            bp_bytes.insert(*src_byte);
            seen_lines.insert(line);
        }
    }
    for offset in &line_bp_offsets {
        bp_bytes.insert(*offset);
    }

    // Create channels
    let (command_tx, command_rx) = mpsc::channel::<DebugAction>();
    let (event_tx, event_rx) = mpsc::channel::<DebugEvent>();

    // Set debug callback
    let callback = DebugCallback::new(command_rx, event_tx.clone(), sm);
    let callback_py = Py::new(py, callback)?;
    inner_vm.call_method1("set_debug_callback", (callback_py,))?;
    for offset in &bp_bytes {
        inner_vm.call_method1("add_debug_breakpoint", (*offset,))?;
    }

    // Build DebugContext
    let inner_vm_ref: Py<PyAny> = inner_vm.unbind();
    let sm_py_cls = py.import(PY_MOD_RS)?.getattr("SourceMap")?;
    let sm_py: Py<PyAny> = sm_py_cls.call1((source_bytes.as_slice(),))?.unbind();

    let dctx = DebugContext {
        source_bytes: source_bytes.clone(),
        file_label: file_label.clone(),
        command_tx,
        inner_vm_ref,
        sm_py,
        catnip_globals,
    };

    // Spawn VM execution thread
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

    // --- Console loop (without GIL) ---

    let exit_code = py.detach(move || -> i32 {
        eprintln!("{}", debugger::format_header());

        let mut last_action = DebugAction::StepInto;
        let timeout = Duration::from_secs(DEBUG_EVENT_WAIT_TIMEOUT_SECS);

        loop {
            let event = match event_rx.recv_timeout(timeout) {
                Ok(ev) => ev,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break 0,
            };

            match event {
                DebugEvent::Paused(pause) => {
                    eprintln!(
                        "{}",
                        debugger::format_pause(pause.line as u32, pause.col as u32, &pause.snippet)
                    );

                    'prompt: loop {
                        // Per-read stdin lock (released before REPL submode)
                        let line = {
                            let stdin = io::stdin();
                            let mut reader = stdin.lock();
                            eprint!("(catnip-dbg:L{}) > ", pause.line);
                            io::stderr().flush().ok();

                            let mut line = String::new();
                            match reader.read_line(&mut line) {
                                Ok(0) | Err(_) => {
                                    eprintln!();
                                    let _ = dctx.command_tx.send(DebugAction::Continue);
                                    break 'prompt;
                                }
                                Ok(_) => {}
                            }
                            line
                        };

                        let cmd = debugger::parse_command(line.trim());
                        match cmd {
                            DebugCommand::Repeat => {
                                let _ = dctx.command_tx.send(last_action);
                                break;
                            }
                            DebugCommand::Continue => {
                                last_action = DebugAction::Continue;
                                let _ = dctx.command_tx.send(DebugAction::Continue);
                                break;
                            }
                            DebugCommand::StepInto => {
                                last_action = DebugAction::StepInto;
                                let _ = dctx.command_tx.send(DebugAction::StepInto);
                                break;
                            }
                            DebugCommand::StepOver => {
                                last_action = DebugAction::StepOver;
                                let _ = dctx.command_tx.send(DebugAction::StepOver);
                                break;
                            }
                            DebugCommand::StepOut => {
                                last_action = DebugAction::StepOut;
                                let _ = dctx.command_tx.send(DebugAction::StepOut);
                                break;
                            }
                            DebugCommand::Break(line_num) => {
                                Python::attach(|py| handle_break(py, &dctx, line_num));
                            }
                            DebugCommand::RemoveBreak(line_num) => {
                                Python::attach(|py| handle_remove_break(py, &dctx, line_num));
                            }
                            DebugCommand::Print(expr) => {
                                Python::attach(|py| match eval_expr(py, &expr, &dctx.catnip_globals, &pause) {
                                    Ok(result) => eprintln!("  = {}", result),
                                    Err(e) => eprintln!("  Error: {}", e),
                                });
                            }
                            DebugCommand::Vars => {
                                eprintln!("{}", debugger::format_vars(&pause.locals_repr));
                            }
                            DebugCommand::List => handle_list(&dctx.source_bytes, &dctx.file_label, &pause),
                            DebugCommand::Backtrace => handle_backtrace(&dctx.source_bytes, &dctx.file_label, &pause),
                            DebugCommand::Repl => {
                                eprintln!("Entering REPL (type /exit or empty line to return to debugger)");
                                let result = Python::attach(|py| run_repl_submode(py, &dctx, &pause));
                                match result {
                                    ReplResult::Resume(action) => {
                                        last_action = action;
                                        let _ = dctx.command_tx.send(action);
                                        break 'prompt;
                                    }
                                    ReplResult::Exit => {
                                        eprintln!("Leaving REPL, back to debugger.");
                                    }
                                }
                            }
                            DebugCommand::Quit => {
                                eprintln!("Aborting.");
                                std::process::exit(0);
                            }
                            DebugCommand::Help => {
                                eprintln!("{}", debugger::format_help());
                            }
                            DebugCommand::Unknown(s) => {
                                eprintln!("{}", debugger::format_unknown_command(&s));
                            }
                        }
                    }
                }
                DebugEvent::Finished(py_result) => {
                    let repr = Python::attach(|py| {
                        py_result
                            .bind(py)
                            .repr()
                            .map_or_else(|_| "???".to_string(), |r| r.to_string())
                    });
                    eprintln!("\nExecution finished. Result: {}", repr);
                    break 0;
                }
                DebugEvent::Error(msg) => {
                    eprintln!("\nExecution error: {}", msg);
                    break 1;
                }
            }
        }
    });

    // Restore original vm_mode (F16)
    Python::attach(|py| {
        let _ = catnip_ref.bind(py).setattr("vm_mode", original_vm_mode.as_str());
    });

    Ok(exit_code)
}
