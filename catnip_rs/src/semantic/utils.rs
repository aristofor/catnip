// FILE: catnip_rs/src/semantic/utils.rs
//! Shared utilities for optimization passes

use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

/// Extract variable name from a SET_LOCALS names argument.
///
/// The names node can be:
/// - Lvalue(value="x") -> "x"
/// - Ref(ident="x") -> "x"
/// - ("x",) tuple containing a single Lvalue/Ref -> "x"
/// - A plain string "x" (backward compat) -> "x"
///
/// Returns None for complex patterns (multi-element tuples, star patterns, etc.)
pub(crate) fn extract_var_name(node: &Bound<'_, PyAny>) -> Option<String> {
    let type_name = node.get_type().name().ok()?;
    let type_str = type_name.to_str().ok()?;

    match type_str {
        t if t == catnip::LVALUE => node.getattr("value").ok()?.extract::<String>().ok(),
        t if t == catnip::REF => node.getattr("ident").ok()?.extract::<String>().ok(),
        _ => {
            // Try as plain string (backward compat)
            if let Ok(s) = node.extract::<String>() {
                return Some(s);
            }
            // Try as single-element tuple: (Lvalue("x"),)
            if let Ok(tuple) = node.cast::<PyTuple>() {
                if tuple.len() == 1 {
                    return extract_var_name(&tuple.get_item(0).ok()?);
                }
            }
            None
        }
    }
}
