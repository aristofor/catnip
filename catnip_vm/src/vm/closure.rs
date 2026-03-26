// FILE: catnip_vm/src/vm/closure.rs
//! Pure Rust closure scope -- no PyO3 dependency.
//!
//! Stripped-down NativeClosureScope without PyGlobals parent.

use crate::Value;
use crate::host::Globals;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

/// Closure parent in the scope chain (pure Rust only).
pub enum PureClosureParent {
    /// No parent (top-level function)
    None,
    /// Parent is another closure scope (nested closures)
    Scope(PureClosureScope),
    /// Terminal: Rust-owned globals
    Globals(Globals),
}

struct ClosureScopeInner {
    captured: RefCell<IndexMap<String, Value>>,
    parent: PureClosureParent,
}

/// Pure Rust closure scope for captured variables.
/// Uses `Rc` (single-threaded sharing) + `RefCell` (interior mutability).
#[derive(Clone)]
pub struct PureClosureScope {
    inner: Rc<ClosureScopeInner>,
}

impl PureClosureScope {
    pub fn new(captured: IndexMap<String, Value>, parent: PureClosureParent) -> Self {
        Self {
            inner: Rc::new(ClosureScopeInner {
                captured: RefCell::new(captured),
                parent,
            }),
        }
    }

    pub fn without_parent(captured: IndexMap<String, Value>) -> Self {
        Self::new(captured, PureClosureParent::None)
    }

    pub fn with_parent(captured: IndexMap<String, Value>, parent: PureClosureScope) -> Self {
        Self::new(captured, PureClosureParent::Scope(parent))
    }

    pub fn with_globals(captured: IndexMap<String, Value>, globals: Globals) -> Self {
        Self::new(captured, PureClosureParent::Globals(globals))
    }

    /// Resolve a variable by walking the scope chain.
    pub fn resolve(&self, name: &str) -> Option<Value> {
        let captured = self.inner.captured.borrow();
        if let Some(&val) = captured.get(name) {
            if !val.is_nil() {
                return Some(val);
            }
        }
        drop(captured);
        match &self.inner.parent {
            PureClosureParent::Scope(parent) => parent.resolve(name),
            PureClosureParent::Globals(globals) => globals.borrow().get(name).copied(),
            PureClosureParent::None => None,
        }
    }

    /// Set a variable in the nearest scope that contains it.
    pub fn set(&self, name: &str, value: Value) -> bool {
        let mut captured = self.inner.captured.borrow_mut();
        if captured.contains_key(name) {
            captured.insert(name.to_string(), value);
            return true;
        }
        drop(captured);
        match &self.inner.parent {
            PureClosureParent::Scope(parent) => parent.set(name, value),
            PureClosureParent::Globals(globals) => {
                let mut g = globals.borrow_mut();
                if g.contains_key(name) {
                    g.insert(name.to_string(), value);
                    true
                } else {
                    false
                }
            }
            PureClosureParent::None => false,
        }
    }

    /// All captured entries in this scope (not parents).
    pub fn captured_entries(&self) -> Vec<(String, Value)> {
        self.inner
            .captured
            .borrow()
            .iter()
            .map(|(k, &v)| (k.clone(), v))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_local() {
        let mut captured = IndexMap::new();
        captured.insert("x".into(), Value::from_int(42));
        let scope = PureClosureScope::without_parent(captured);
        assert_eq!(scope.resolve("x").unwrap().as_int(), Some(42));
        assert!(scope.resolve("y").is_none());
    }

    #[test]
    fn test_resolve_parent_chain() {
        let mut outer = IndexMap::new();
        outer.insert("a".into(), Value::from_int(1));
        let outer_scope = PureClosureScope::without_parent(outer);

        let mut inner = IndexMap::new();
        inner.insert("b".into(), Value::from_int(2));
        let inner_scope = PureClosureScope::with_parent(inner, outer_scope);

        assert_eq!(inner_scope.resolve("b").unwrap().as_int(), Some(2));
        assert_eq!(inner_scope.resolve("a").unwrap().as_int(), Some(1));
        assert!(inner_scope.resolve("c").is_none());
    }

    #[test]
    fn test_set_updates_nearest() {
        let mut outer = IndexMap::new();
        outer.insert("x".into(), Value::from_int(1));
        let outer_scope = PureClosureScope::without_parent(outer);

        let inner = IndexMap::new();
        let inner_scope = PureClosureScope::with_parent(inner, outer_scope.clone());

        // Set x through inner -> finds it in outer
        assert!(inner_scope.set("x", Value::from_int(99)));
        assert_eq!(outer_scope.resolve("x").unwrap().as_int(), Some(99));
    }

    #[test]
    fn test_set_unknown_returns_false() {
        let scope = PureClosureScope::without_parent(IndexMap::new());
        assert!(!scope.set("nope", Value::from_int(1)));
    }

    #[test]
    fn test_resolve_globals_terminal() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut globals_map = IndexMap::new();
        globals_map.insert("g".into(), Value::from_int(7));
        let globals: Globals = Rc::new(RefCell::new(globals_map));

        let scope = PureClosureScope::with_globals(IndexMap::new(), globals);
        assert_eq!(scope.resolve("g").unwrap().as_int(), Some(7));
    }
}
