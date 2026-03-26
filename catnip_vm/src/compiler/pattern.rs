// FILE: catnip_vm/src/compiler/pattern.rs
//! Native VM pattern types for match expressions.
//!
//! Pure Rust version -- no PyO3 dependency.

use crate::Value;

/// Pre-compiled pattern for VM-native matching.
#[derive(Debug, Clone)]
pub enum VMPattern {
    /// Matches anything, no bindings
    Wildcard,
    /// Matches if value == literal (NaN-boxed comparison)
    Literal(Value),
    /// Matches anything, binds to local slot
    Var(usize),
    /// Tries sub-patterns in order, returns first match
    Or(Vec<VMPattern>),
    /// Matches and destructures iterables
    Tuple(Vec<VMPatternElement>),
    /// Matches a struct instance by type name and extracts fields into slots
    Struct {
        name: String,
        field_slots: Vec<(String, usize)>,
    },
}

/// Element of a tuple pattern (regular or star/rest).
#[derive(Debug, Clone)]
pub enum VMPatternElement {
    /// Regular sub-pattern
    Pattern(VMPattern),
    /// Star pattern (*rest): captures remaining elements into slot (usize::MAX = no binding)
    Star(usize),
}
