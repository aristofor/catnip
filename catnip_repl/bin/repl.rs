// FILE: catnip_repl/bin/repl.rs
//! Catnip REPL - Lanceur crossterm + ratatui

use _repl::app::{App, ExitReason};
use _repl::config::ReplConfig;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use pyo3::prelude::*;
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::io;
use std::panic;

fn main() -> io::Result<()> {
    // Application marker: catnip/__init__.py configures logging when it sees
    // it. A sys attribute is process-local — unlike an env var it does not
    // leak into child processes importing catnip as a library.
    pyo3::Python::attach(|py| {
        if let Ok(sys) = py.import("sys") {
            let _ = sys.setattr("_catnip_embedded", true);
        }
    });

    let config = ReplConfig::default();

    let app = match App::new(config) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("Failed to initialize REPL: {}", e);
            std::process::exit(1);
        }
    };

    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(1),
        },
    )?;

    // Panic hook : restore terminal + abort message
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        eprintln!("{}", ExitReason::Abort.message());
        default_hook(info);
    }));

    let reason = app.run(&mut terminal)?;

    disable_raw_mode()?;

    // Exit code selon la raison
    match reason {
        ExitReason::Ok => {}
        ExitReason::Abort => std::process::exit(130),
    }

    // Finalize embedded Python: runs atexit hooks (multiprocessing cleanup,
    // logging.shutdown), joins non-daemon threads, triggers the final GC.
    // Skipped on Abort where immediate exit is the point.
    unsafe {
        if pyo3::ffi::Py_IsInitialized() != 0 {
            pyo3::ffi::PyGILState_Ensure();
            pyo3::ffi::Py_FinalizeEx();
        }
    }

    Ok(())
}
