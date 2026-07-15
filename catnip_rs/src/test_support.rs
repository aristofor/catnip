//! Test-only: initialize embedded Python and pre-import the `catnip` package.
//!
//! Multi-threaded `cargo test` runs used to hit importlib's `_DeadlockError`
//! (`_ModuleLock('catnip.cachesys')`): several test threads performing their
//! first `py.import("catnip.X")` acquire per-module import locks in different
//! orders. Importing once here, behind a `Once`, leaves the package in
//! `sys.modules`, so later imports are lock-free cache hits.

use std::sync::Once;

/// Must be called before a test's first `Python::attach`, or GIL-released
/// (`py.detach`) if the GIL is already held — blocking on the `Once` while
/// holding the GIL would deadlock against the warming thread.
pub(crate) fn init_python() {
    static WARM: Once = Once::new();
    pyo3::Python::initialize();
    WARM.call_once(|| {
        pyo3::Python::attach(|py| {
            // Best-effort: pure-Rust tests must keep running without the
            // Python package; tests that need it surface the import error.
            if let Err(e) = py.import("catnip") {
                eprintln!("test_support: warm import of catnip failed: {e}");
            }
            let _ = py.import(crate::constants::PY_MOD_SEMANTIC_OPCODE);
        });
    });
}
