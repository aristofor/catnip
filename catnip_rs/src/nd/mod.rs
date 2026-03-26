// FILE: catnip_rs/src/nd/mod.rs
//! ND-recursion module for parallel/concurrent execution.
//!
//! Provides NDScheduler for managing ND lambda execution in sequential,
//! threaded, or process-based modes.

mod declaration;
mod future;
mod recur;
mod scheduler;
mod vm_decl;
pub mod worker_pool;

pub use declaration::NDDeclaration;
pub use future::{NDFuture, NDState};
pub use recur::NDRecur;
pub use scheduler::NDScheduler;
pub use vm_decl::{NDParallelDecl, NDParallelRecur, NDVmDecl, NDVmRecur, check_nd_abort, clear_nd_abort, set_nd_abort};
