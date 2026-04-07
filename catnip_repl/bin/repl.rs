// FILE: catnip_repl/bin/repl.rs
//! Catnip REPL - Lanceur crossterm + ratatui

use _repl::app::{App, ExitReason};
use _repl::config::ReplConfig;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::io;
use std::panic;

fn main() -> io::Result<()> {
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

    Ok(())
}
