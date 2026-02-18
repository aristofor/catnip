// FILE: catnip_rs/src/debug/console.rs
// run_debugger(): Rust console debugger replacing ConsoleDebugger.run().
// Releases GIL for the console loop, reacquires for expression eval.

use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

use pyo3::prelude::*;

use catnip_tools::debugger::{self, DebugCommand};
use catnip_tools::sourcemap::SourceMap;

use super::callback::DebugCallback;
use super::types::*;

/// Launch an interactive debug session from Rust.
///
/// This replaces ConsoleDebugger.run() entirely:
/// - Creates channels and DebugCallback internally
/// - Releases GIL for the console loop
/// - Reacquires GIL only for expression evaluation
#[pyfunction]
#[pyo3(signature = (source, breakpoints, verbose=false, no_color=false))]
pub fn run_debugger(
    py: Python<'_>,
    source: String,
    breakpoints: Vec<u32>,
    verbose: bool,
    no_color: bool,
) -> PyResult<i32> {
    let _ = verbose;
    let _ = no_color;
    let source_bytes = source.as_bytes().to_vec();

    // --- Setup phase (with GIL) ---

    // Create Catnip instance and parse
    let catnip_mod = py.import("catnip")?;
    let catnip_cls = catnip_mod.getattr("Catnip")?;
    let catnip = catnip_cls.call0()?;
    catnip.call_method1("parse", (&source,))?;
    catnip.setattr("vm_mode", "on")?;

    // Build SourceMap for callback
    let sm = SourceMap::new(source_bytes.clone(), "<input>".to_string());

    // Convert line breakpoints to byte offsets
    let mut sm_for_offsets = SourceMap::new(source_bytes.clone(), "<input>".to_string());
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
    vm_wrapper.call_method1("set_source", (source_bytes.as_slice(), "<input>"))?;
    let inner_vm = vm_wrapper.getattr("_vm")?;

    // Compile to get line_table
    let compiler_cls = py.import("catnip._rs")?.getattr("Compiler")?;
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

    // Keep refs for dynamic breakpoints
    let inner_vm_ref: Py<PyAny> = inner_vm.unbind();
    let sm_py_cls = py.import("catnip._rs")?.getattr("SourceMap")?;
    let sm_py: Py<PyAny> = sm_py_cls.call1((source_bytes.as_slice(),))?.unbind();

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

    let source_bytes_for_loop = source_bytes.clone();
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

                    // Prompt loop
                    let stdin = io::stdin();
                    let mut reader = stdin.lock();
                    loop {
                        eprint!("(catnip-dbg:L{}) > ", pause.line);
                        io::stderr().flush().ok();

                        let mut line = String::new();
                        match reader.read_line(&mut line) {
                            Ok(0) | Err(_) => {
                                eprintln!();
                                let _ = command_tx.send(DebugAction::Continue);
                                break;
                            }
                            Ok(_) => {}
                        }

                        let cmd = debugger::parse_command(line.trim());
                        match cmd {
                            DebugCommand::Repeat => {
                                let _ = command_tx.send(last_action);
                                break;
                            }
                            DebugCommand::Continue => {
                                last_action = DebugAction::Continue;
                                let _ = command_tx.send(DebugAction::Continue);
                                break;
                            }
                            DebugCommand::StepInto => {
                                last_action = DebugAction::StepInto;
                                let _ = command_tx.send(DebugAction::StepInto);
                                break;
                            }
                            DebugCommand::StepOver => {
                                last_action = DebugAction::StepOver;
                                let _ = command_tx.send(DebugAction::StepOver);
                                break;
                            }
                            DebugCommand::StepOut => {
                                last_action = DebugAction::StepOut;
                                let _ = command_tx.send(DebugAction::StepOut);
                                break;
                            }
                            DebugCommand::Break(line_num) => {
                                // Dynamic breakpoint: needs GIL
                                Python::attach(|py| {
                                    let sm_bound = sm_py.bind(py);
                                    let offset: Option<usize> = sm_bound
                                        .call_method1("line_to_offset", (line_num,))
                                        .ok()
                                        .and_then(|r| r.extract().ok());
                                    if let Some(offset) = offset {
                                        let _ = inner_vm_ref
                                            .bind(py)
                                            .call_method1("add_debug_breakpoint", (offset as u32,));
                                    }
                                });
                                eprintln!("Breakpoint set at line {}", line_num);
                            }
                            DebugCommand::RemoveBreak(line_num) => {
                                Python::attach(|py| {
                                    let sm_bound = sm_py.bind(py);
                                    let offset: Option<usize> = sm_bound
                                        .call_method1("line_to_offset", (line_num,))
                                        .ok()
                                        .and_then(|r| r.extract().ok());
                                    if let Some(offset) = offset {
                                        let _ = inner_vm_ref.bind(py).call_method1(
                                            "remove_debug_breakpoint",
                                            (offset as u32,),
                                        );
                                    }
                                });
                                eprintln!("Breakpoint removed at line {}", line_num);
                            }
                            DebugCommand::Print(expr) => {
                                // Eval: needs GIL
                                Python::attach(|py| {
                                    match eval_expr(py, &expr, &catnip_globals, &pause) {
                                        Ok(result) => eprintln!("  = {}", result),
                                        Err(e) => eprintln!("  Error: {}", e),
                                    }
                                });
                            }
                            DebugCommand::Vars => {
                                eprintln!("{}", debugger::format_vars(&pause.locals_repr));
                            }
                            DebugCommand::List => {
                                let mut sm_list = SourceMap::new(
                                    source_bytes_for_loop.clone(),
                                    "<input>".to_string(),
                                );
                                let snippet = sm_list.get_snippet(
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
                            DebugCommand::Backtrace => {
                                let mut sm_bt = SourceMap::new(
                                    source_bytes_for_loop.clone(),
                                    "<input>".to_string(),
                                );
                                let frames: Vec<(String, u32)> = pause
                                    .call_stack
                                    .iter()
                                    .rev()
                                    .map(|(name, sb)| {
                                        let (line, _) = sm_bt.byte_to_line_col(*sb as usize);
                                        (name.clone(), line as u32)
                                    })
                                    .collect();
                                eprintln!("{}", debugger::format_backtrace(&frames));
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

    Ok(exit_code)
}

/// Evaluate an expression in the current debug scope (called with GIL held).
fn eval_expr(
    py: Python<'_>,
    expr: &str,
    catnip_globals: &Py<PyAny>,
    pause: &PauseEvent,
) -> Result<String, String> {
    let catnip_mod = py
        .import("catnip")
        .map_err(|e| format!("import error: {e}"))?;
    let catnip_cls = catnip_mod
        .getattr("Catnip")
        .map_err(|e| format!("getattr error: {e}"))?;
    let c = catnip_cls
        .call0()
        .map_err(|e| format!("Catnip() error: {e}"))?;

    let ctx = c
        .getattr("context")
        .map_err(|e| format!("context error: {e}"))?;
    let globals = ctx
        .getattr("globals")
        .map_err(|e| format!("globals error: {e}"))?;

    // Copy parent globals
    globals
        .call_method1("update", (catnip_globals.bind(py),))
        .map_err(|e| format!("update globals error: {e}"))?;

    // Copy pause locals
    globals
        .call_method1("update", (pause.locals_py.bind(py),))
        .map_err(|e| format!("update locals error: {e}"))?;

    // Parse and execute
    c.call_method1("parse", (expr,))
        .map_err(|e| format!("parse error: {e}"))?;
    let result = c.call_method0("execute").map_err(|e| format!("{e}"))?;
    let repr = result
        .repr()
        .map_err(|e| format!("repr error: {e}"))?
        .to_string();
    Ok(repr)
}
