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
    /// Matches a struct instance by type name and extracts fields into slots.
    /// `variant` is `Some` for union variants (`Option.Some{...}`), `None` for
    /// plain struct patterns (`Point{...}`).
    Struct {
        name: String,
        variant: Option<String>,
        field_slots: Vec<(String, usize)>,
    },
    /// Matches an enum variant by type and variant name
    Enum { enum_name: String, variant_name: String },
}

/// Element of a tuple pattern (regular or star/rest).
#[derive(Debug, Clone)]
pub enum VMPatternElement {
    /// Regular sub-pattern
    Pattern(VMPattern),
    /// Star pattern (*rest): captures remaining elements into slot (usize::MAX = no binding)
    Star(usize),
}

impl VMPattern {
    /// Apply `f` to every literal `Value` embedded in this pattern (recursively).
    fn for_each_literal(&self, f: &mut impl FnMut(Value)) {
        match self {
            VMPattern::Literal(v) => f(*v),
            VMPattern::Or(subs) => subs.iter().for_each(|p| p.for_each_literal(f)),
            VMPattern::Tuple(elems) => {
                for e in elems {
                    if let VMPatternElement::Pattern(p) = e {
                        p.for_each_literal(f);
                    }
                }
            }
            VMPattern::Wildcard | VMPattern::Var(_) | VMPattern::Struct { .. } | VMPattern::Enum { .. } => {}
        }
    }

    /// Take a reference on each embedded literal (bit-copy duplication of the
    /// owning pool -- `CodeObject::clone`).
    pub fn incref_values(&self) {
        self.for_each_literal(&mut Value::clone_refcount);
    }

    /// Release the reference held on each embedded literal. Called by the
    /// owning pool's teardown (`CodeObject::drop`); the match engine only
    /// borrows literals, so the pool holds their reference.
    pub fn decref_values(&self) {
        self.for_each_literal(&mut Value::decref);
    }
}
