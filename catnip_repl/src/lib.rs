// FILE: catnip_repl/src/lib.rs
//! Catnip REPL - Interactive TUI shell (ratatui + crossterm)

// The catnip_rs crate has [lib] name = "_rs" (for PyO3).
// Re-export it under a readable name.
extern crate _rs as catnip_rs;

pub mod app;
pub mod commands;
pub mod completer;
pub mod config;
pub mod config_editor;
pub mod config_tui;
pub mod context;
pub mod executor;
pub mod highlighter;
pub mod hints;
pub mod history;
pub mod input;
pub mod signal;
pub mod theme;
pub mod widgets;

#[cfg(test)]
mod completion_workflow_tests;
#[cfg(test)]
mod integration_tests;

use pyo3::prelude::*;

use std::io;
use std::panic;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};

use app::{App, ExitReason};
use config::ReplConfig;

/// Launch the standalone config editor TUI.
#[pyfunction]
#[pyo3(signature = (config_path=None))]
pub fn run_config_editor(py: Python, config_path: Option<String>) -> PyResult<()> {
    py.detach(|| -> Result<(), String> { config_tui::run(config_path.as_deref()) })
        .map_err(pyo3::exceptions::PyRuntimeError::new_err)
}

/// Launch the ratatui REPL from Python via PyO3.
///
/// Releases the GIL during the TUI loop; the ReplExecutor
/// re-acquires it via Python::attach() on each execution.
#[pyfunction]
#[pyo3(signature = (verbose=false))]
pub fn run_repl(py: Python, verbose: bool) -> PyResult<i32> {
    let _ = verbose;

    py.detach(|| -> Result<i32, String> {
        let config = ReplConfig::default();
        let app = App::new(config)?;

        enable_raw_mode().map_err(|e| format!("Failed to enable raw mode: {e}"))?;
        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(1),
            },
        )
        .map_err(|e| format!("Failed to create terminal: {e}"))?;

        // Panic hook pour restaurer le terminal
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            eprintln!("\n{}", ExitReason::Abort.message());
            default_hook(info);
        }));

        let reason = app.run(&mut terminal).map_err(|e| format!("REPL error: {e}"))?;

        disable_raw_mode().map_err(|e| format!("Failed to disable raw mode: {e}"))?;
        println!();

        Ok(match reason {
            ExitReason::Ok => 0,
            ExitReason::Abort => 130,
        })
    })
    .map_err(pyo3::exceptions::PyRuntimeError::new_err)
}

#[pymodule]
fn _repl(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_repl, m)?)?;
    m.add_function(wrap_pyfunction!(run_config_editor, m)?)?;
    Ok(())
}
