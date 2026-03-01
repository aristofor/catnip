// FILE: catnip_rs/src/cfg/edge.rs
//! CFG edge representation.

use std::fmt;

/// Type of control flow edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeType {
    /// Unconditional jump (always taken)
    Unconditional,
    /// Conditional jump - true branch
    ConditionalTrue,
    /// Conditional jump - false branch
    ConditionalFalse,
    /// Fallthrough to next block (no explicit jump)
    Fallthrough,
    /// Exception edge (for try/except)
    Exception,
    /// Return edge (function exit)
    Return,
    /// Break edge (exit loop)
    Break,
    /// Continue edge (next iteration)
    Continue,
}

impl EdgeType {
    pub fn is_conditional(&self) -> bool {
        matches!(self, EdgeType::ConditionalTrue | EdgeType::ConditionalFalse)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeType::Unconditional => "unconditional",
            EdgeType::ConditionalTrue => "conditional_true",
            EdgeType::ConditionalFalse => "conditional_false",
            EdgeType::Fallthrough => "fallthrough",
            EdgeType::Exception => "exception",
            EdgeType::Return => "return",
            EdgeType::Break => "break",
            EdgeType::Continue => "continue",
        }
    }
}

impl fmt::Display for EdgeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Control flow graph edge.
///
/// Connects two basic blocks with a specific edge type.
#[derive(Debug, Clone)]
pub struct CFGEdge {
    /// Source block ID
    pub source: usize,
    /// Target block ID
    pub target: usize,
    /// Type of control flow edge
    pub edge_type: EdgeType,
    /// Optional label (for switch cases, etc.)
    pub label: Option<String>,
}

impl CFGEdge {
    pub fn new(source: usize, target: usize, edge_type: EdgeType) -> Self {
        Self {
            source,
            target,
            edge_type,
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Check if edge is a back edge (target < source).
    ///
    /// Back edges typically indicate loops.
    pub fn is_back_edge(&self) -> bool {
        self.target < self.source
    }
}

impl fmt::Display for CFGEdge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(label) = &self.label {
            write!(
                f,
                "BB{} -> BB{} ({} [{}])",
                self.source, self.target, self.edge_type, label
            )
        } else {
            write!(
                f,
                "BB{} -> BB{} ({})",
                self.source, self.target, self.edge_type
            )
        }
    }
}
