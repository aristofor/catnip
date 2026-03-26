// FILE: catnip_mcp/src/tools/mod.rs
pub mod check;
pub mod debug;
pub mod eval;
pub mod format;
pub mod parse;

use catnip_vm::Value;

/// Get a type name string for a Value (delegates to Value::type_name()).
pub(crate) fn value_type_name(v: &Value) -> &'static str {
    v.type_name()
}
