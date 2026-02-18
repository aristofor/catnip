// FILE: catnip_rs/src/debug/mod.rs
// Debug module: Rust channels replace Python Queue for VM↔debugger communication.

pub mod callback;
pub mod console;
pub mod session;
pub mod types;

use pyo3::prelude::*;

pub use callback::DebugCallback;
pub use console::run_debugger;
pub use session::RustDebugSession;
pub use types::{DebugAction, DebugEvent};

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RustDebugSession>()?;
    m.add_class::<DebugCallback>()?;
    m.add_function(wrap_pyfunction!(console::run_debugger, m)?)?;
    Ok(())
}
