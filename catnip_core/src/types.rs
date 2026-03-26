// FILE: catnip_core/src/types.rs
//! Type name constants for runtime type checking.
//!
//! This module provides string constants for Python type names used in type checks
//! throughout the codebase. Using constants instead of hardcoded strings provides:
//! - Type safety via compile-time checking
//! - IDE autocomplete support
//! - Easy refactoring (change constant → all usages updated)
//! - Centralized documentation

/// Catnip AST and execution node types
pub mod catnip {
    /// Reference to an identifier: `<Ref name>`
    pub const REF: &str = "Ref";

    /// Left-value for assignment: `<Lvalue value>`
    pub const LVALUE: &str = "Lvalue";

    /// Tail-call optimization signal: `<TailCall func args kwargs>`
    pub const TAIL_CALL: &str = "TailCall";

    /// Broadcast operation: `<Broadcast target op operand>`
    pub const BROADCAST: &str = "Broadcast";

    /// Executable operation node: `<Op opcode args kwargs>`
    pub const OP: &str = "Op";

    /// Intermediate representation node: `<IR opcode args kwargs>`
    pub const IR: &str = "IR";

    /// Identifier wrapper: `<Identifier name>`
    pub const IDENTIFIER: &str = "Identifier";

    /// Call expression wrapper: `<Call func args kwargs>`
    pub const CALL: &str = "Call";

    /// Pattern matching literal: `<PatternLiteral value>`
    pub const PATTERN_LITERAL: &str = "PatternLiteral";
}

/// Exception types used for control flow
pub mod exceptions {
    /// Break statement exception
    pub const BREAK_LOOP: &str = "BreakLoop";

    /// Continue statement exception
    pub const CONTINUE_LOOP: &str = "ContinueLoop";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_are_unique() {
        // Ensure no duplicate type names
        let types = vec![
            catnip::REF,
            catnip::LVALUE,
            catnip::TAIL_CALL,
            catnip::BROADCAST,
            catnip::OP,
            catnip::IR,
            catnip::IDENTIFIER,
            catnip::CALL,
            catnip::PATTERN_LITERAL,
            exceptions::BREAK_LOOP,
            exceptions::CONTINUE_LOOP,
        ];

        let mut unique = types.clone();
        unique.sort();
        unique.dedup();

        assert_eq!(types.len(), unique.len(), "Duplicate type name constants detected");
    }

    #[test]
    fn test_constants_are_not_empty() {
        // Ensure all constants are non-empty
        assert!(!catnip::REF.is_empty());
        assert!(!catnip::LVALUE.is_empty());
        assert!(!catnip::TAIL_CALL.is_empty());
        assert!(!catnip::BROADCAST.is_empty());
        assert!(!catnip::OP.is_empty());
        assert!(!catnip::IR.is_empty());
        assert!(!catnip::IDENTIFIER.is_empty());
        assert!(!catnip::CALL.is_empty());
        assert!(!catnip::PATTERN_LITERAL.is_empty());
        assert!(!exceptions::BREAK_LOOP.is_empty());
        assert!(!exceptions::CONTINUE_LOOP.is_empty());
    }
}
