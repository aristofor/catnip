// FILE: catnip_vm/src/vm/mod.rs
//! Pure Rust VM execution engine (no PyO3).

pub mod broadcast;
pub mod closure;
pub mod core;
pub mod debug;
pub mod frame;
pub mod func_table;
pub mod structs;

pub use closure::PureClosureScope;
pub use core::PureVM;
pub use debug::{DebugCommand, DebugHook, PauseInfo};
pub use frame::{PureFrame, PureFramePool};
pub use func_table::{PureFuncSlot, PureFunctionTable};
pub use structs::{PureStructRegistry, PureTraitRegistry};
