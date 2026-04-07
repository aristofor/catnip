// FILE: catnip_core/src/exception.rs
//! Built-in exception categories shared between PureVM and PyO3 VM.

/// Built-in exception categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExceptionKind {
    Exception,
    TypeError,
    ValueError,
    NameError,
    IndexError,
    KeyError,
    AttributeError,
    ZeroDivisionError,
    RuntimeError,
    MemoryError,
    ArithmeticError,
    LookupError,
}

impl ExceptionKind {
    /// All built-in exception kinds, in registration order.
    /// Parents before children (Exception first, then groups, then leaves).
    pub const ALL: [ExceptionKind; 12] = [
        ExceptionKind::Exception,
        ExceptionKind::TypeError,
        ExceptionKind::ValueError,
        ExceptionKind::NameError,
        ExceptionKind::AttributeError,
        ExceptionKind::RuntimeError,
        ExceptionKind::MemoryError,
        ExceptionKind::ArithmeticError,
        ExceptionKind::ZeroDivisionError,
        ExceptionKind::LookupError,
        ExceptionKind::IndexError,
        ExceptionKind::KeyError,
    ];

    /// Type name as it appears in Catnip code.
    pub fn name(self) -> &'static str {
        match self {
            ExceptionKind::Exception => "Exception",
            ExceptionKind::TypeError => "TypeError",
            ExceptionKind::ValueError => "ValueError",
            ExceptionKind::NameError => "NameError",
            ExceptionKind::IndexError => "IndexError",
            ExceptionKind::KeyError => "KeyError",
            ExceptionKind::AttributeError => "AttributeError",
            ExceptionKind::ZeroDivisionError => "ZeroDivisionError",
            ExceptionKind::RuntimeError => "RuntimeError",
            ExceptionKind::MemoryError => "MemoryError",
            ExceptionKind::ArithmeticError => "ArithmeticError",
            ExceptionKind::LookupError => "LookupError",
        }
    }

    /// Lookup by name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Exception" => Some(ExceptionKind::Exception),
            "TypeError" => Some(ExceptionKind::TypeError),
            "ValueError" => Some(ExceptionKind::ValueError),
            "NameError" => Some(ExceptionKind::NameError),
            "IndexError" => Some(ExceptionKind::IndexError),
            "KeyError" => Some(ExceptionKind::KeyError),
            "AttributeError" => Some(ExceptionKind::AttributeError),
            "ZeroDivisionError" => Some(ExceptionKind::ZeroDivisionError),
            "RuntimeError" => Some(ExceptionKind::RuntimeError),
            "MemoryError" => Some(ExceptionKind::MemoryError),
            "ArithmeticError" => Some(ExceptionKind::ArithmeticError),
            "LookupError" => Some(ExceptionKind::LookupError),
            _ => None,
        }
    }

    /// Direct parent in the hierarchy. None for Exception (root).
    pub fn parent(self) -> Option<ExceptionKind> {
        match self {
            ExceptionKind::Exception => None,
            ExceptionKind::ArithmeticError => Some(ExceptionKind::Exception),
            ExceptionKind::LookupError => Some(ExceptionKind::Exception),
            ExceptionKind::ZeroDivisionError => Some(ExceptionKind::ArithmeticError),
            ExceptionKind::IndexError => Some(ExceptionKind::LookupError),
            ExceptionKind::KeyError => Some(ExceptionKind::LookupError),
            _ => Some(ExceptionKind::Exception),
        }
    }

    /// MRO: [self, parent, ..., Exception].
    pub fn mro(self) -> Vec<String> {
        let mut chain = vec![self.name().to_string()];
        let mut current = self;
        while let Some(p) = current.parent() {
            chain.push(p.name().to_string());
            current = p;
        }
        chain
    }
}

impl std::fmt::Display for ExceptionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// VM exception handler infrastructure
// ---------------------------------------------------------------------------

/// Type of exception handler installed on the handler stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerType {
    Except,
    Finally,
}

/// An active exception handler on the frame's handler stack.
#[derive(Debug, Clone)]
pub struct Handler {
    pub handler_type: HandlerType,
    pub target_addr: usize,
    pub stack_depth: usize,
    pub block_depth: usize,
}

/// Active exception info with MRO for hierarchical matching.
#[derive(Debug, Clone)]
pub struct ExceptionInfo {
    pub type_name: String,
    pub message: String,
    /// [self, parent, ..., Exception]
    pub mro: Vec<String>,
}

impl ExceptionInfo {
    pub fn new(type_name: String, message: String, mro: Vec<String>) -> Self {
        Self {
            type_name,
            message,
            mro,
        }
    }

    /// Build from a known ExceptionKind.
    pub fn from_kind(kind: ExceptionKind, message: String) -> Self {
        Self {
            type_name: kind.name().to_string(),
            mro: kind.mro(),
            message,
        }
    }

    /// Build from a type name, looking up built-in MRO or using [name, Exception] fallback.
    pub fn from_name(type_name: String, message: String) -> Self {
        let mro = if let Some(kind) = ExceptionKind::from_name(&type_name) {
            kind.mro()
        } else {
            // User-defined exception not in built-in hierarchy: [self, Exception]
            vec![type_name.clone(), "Exception".to_string()]
        };
        Self {
            type_name,
            message,
            mro,
        }
    }

    /// Check if this exception matches a given type name (by MRO inclusion).
    pub fn matches(&self, type_name: &str) -> bool {
        self.mro.iter().any(|t| t == type_name)
    }
}

/// Saved unwind state when a finally block must execute before propagation.
#[derive(Debug, Clone)]
pub enum PendingUnwind {
    Exception(ExceptionInfo),
    /// return was in progress (value already on stack or in slot)
    Return,
    Break,
    Continue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exception_kind_names() {
        assert_eq!(ExceptionKind::TypeError.name(), "TypeError");
        assert_eq!(ExceptionKind::ZeroDivisionError.name(), "ZeroDivisionError");
        assert_eq!(ExceptionKind::MemoryError.name(), "MemoryError");
        assert_eq!(ExceptionKind::Exception.name(), "Exception");
        assert_eq!(ExceptionKind::ArithmeticError.name(), "ArithmeticError");
        assert_eq!(ExceptionKind::LookupError.name(), "LookupError");
    }

    #[test]
    fn test_exception_kind_from_name() {
        assert_eq!(ExceptionKind::from_name("TypeError"), Some(ExceptionKind::TypeError));
        assert_eq!(ExceptionKind::from_name("ValueError"), Some(ExceptionKind::ValueError));
        assert_eq!(ExceptionKind::from_name("Exception"), Some(ExceptionKind::Exception));
        assert_eq!(
            ExceptionKind::from_name("ArithmeticError"),
            Some(ExceptionKind::ArithmeticError)
        );
        assert_eq!(
            ExceptionKind::from_name("LookupError"),
            Some(ExceptionKind::LookupError)
        );
        assert_eq!(ExceptionKind::from_name("NotARealError"), None);
    }

    #[test]
    fn test_exception_kind_all() {
        assert_eq!(ExceptionKind::ALL.len(), 12);
        for kind in ExceptionKind::ALL {
            assert_eq!(ExceptionKind::from_name(kind.name()), Some(kind));
        }
    }

    #[test]
    fn test_exception_hierarchy() {
        // Exception is root
        assert_eq!(ExceptionKind::Exception.parent(), None);
        assert_eq!(ExceptionKind::Exception.mro(), vec!["Exception"]);

        // Direct children of Exception
        assert_eq!(ExceptionKind::TypeError.parent(), Some(ExceptionKind::Exception));
        assert_eq!(ExceptionKind::TypeError.mro(), vec!["TypeError", "Exception"]);

        // ArithmeticError group
        assert_eq!(ExceptionKind::ArithmeticError.parent(), Some(ExceptionKind::Exception));
        assert_eq!(
            ExceptionKind::ZeroDivisionError.parent(),
            Some(ExceptionKind::ArithmeticError)
        );
        assert_eq!(
            ExceptionKind::ZeroDivisionError.mro(),
            vec!["ZeroDivisionError", "ArithmeticError", "Exception"]
        );

        // LookupError group
        assert_eq!(ExceptionKind::LookupError.parent(), Some(ExceptionKind::Exception));
        assert_eq!(ExceptionKind::IndexError.parent(), Some(ExceptionKind::LookupError));
        assert_eq!(
            ExceptionKind::IndexError.mro(),
            vec!["IndexError", "LookupError", "Exception"]
        );
        assert_eq!(
            ExceptionKind::KeyError.mro(),
            vec!["KeyError", "LookupError", "Exception"]
        );
    }

    #[test]
    fn test_exception_info_matches() {
        let info = ExceptionInfo::from_kind(ExceptionKind::ZeroDivisionError, "div by zero".into());
        assert!(info.matches("ZeroDivisionError"));
        assert!(info.matches("ArithmeticError"));
        assert!(info.matches("Exception"));
        assert!(!info.matches("TypeError"));
        assert!(!info.matches("LookupError"));
    }

    #[test]
    fn test_exception_info_from_name_builtin() {
        let info = ExceptionInfo::from_name("IndexError".into(), "out of range".into());
        assert_eq!(info.mro, vec!["IndexError", "LookupError", "Exception"]);
    }

    #[test]
    fn test_exception_info_from_name_unknown() {
        let info = ExceptionInfo::from_name("HttpError".into(), "404".into());
        assert_eq!(info.mro, vec!["HttpError", "Exception"]);
    }
}
