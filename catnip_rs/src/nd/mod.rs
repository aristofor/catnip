// FILE: catnip_rs/src/nd/mod.rs
//! ND-recursion module for parallel/concurrent execution.
//!
//! Provides NDScheduler for managing ND lambda execution in sequential,
//! threaded, or process-based modes.

mod future;
mod recur;
mod scheduler;

pub use future::{NDFuture, NDState};
pub use recur::NDRecur;
pub use scheduler::NDScheduler;
