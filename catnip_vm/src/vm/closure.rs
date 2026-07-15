// FILE: catnip_vm/src/vm/closure.rs
//! Pure Rust closure scope -- no PyO3 dependency.
//!
//! Stripped-down NativeClosureScope without PyGlobals parent.

use super::func_table::PureFuncSlot;
use crate::Value;
use crate::host::Globals;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Weak;

/// Closure parent in the scope chain (pure Rust only).
#[derive(Clone)]
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
    /// Letrec self-reference: the name the defining function is bound to inside
    /// its own body, plus a *weak* handle to its own runtime slot. Weak (not a
    /// captured strong `Value`) so a self-recursive closure is not pinned by an
    /// Arc cycle: the caller holds a strong ref while the function executes, so
    /// the upgrade in `resolve` always succeeds during a call. `resolve` returns
    /// a refcount-neutral `Value` (the reader `clone_refcount`s), so the
    /// temporary upgrade is released immediately.
    self_ref: RefCell<Option<(String, Weak<PureFuncSlot>)>>,
}

/// The captured map owns one ref per entry: `MakeFunction` and bound-method
/// access `clone_refcount` at capture, `set`/`insert_captured` transfer the
/// incoming ref and release the overwritten one, and `LoadScope` readers clone
/// their own ref. Release the map's refs when the last scope handle dies.
/// `Value::decref` is self-contained here (Arc-backed struct/list/bigint; a
/// VMFunc index is non-refcounted, so its decref is a no-op -- letrec
/// self-references never leak nor double-free). Without this drop a captured
/// heap value (self=p for a bound method, a struct/bigint for a closure) leaks
/// when the grow-only func_table -- hence the scope -- is dropped at reset.
impl Drop for ClosureScopeInner {
    fn drop(&mut self) {
        for (_, v) in self.captured.borrow_mut().drain(..) {
            v.decref();
        }
    }
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
                self_ref: RefCell::new(None),
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
        // Letrec self-reference (weak): return a refcount-neutral handle -- the
        // reader (`LoadScope`) `clone_refcount`s. The slot is alive because the
        // caller holds a strong ref while executing the function, so the upgrade
        // succeeds and the neutral bits stay valid after the temporary strong ref
        // is dropped.
        {
            let sr = self.inner.self_ref.borrow();
            if let Some((sname, weak)) = &*sr {
                if sname == name {
                    if let Some(arc) = weak.upgrade() {
                        return Some(Value::from_closure_neutral(&arc));
                    }
                }
            }
        }
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

    /// Bind this closure's letrec self-reference to a weak handle to its own
    /// runtime slot, under the name the function is bound to inside its body.
    /// Weak so the self-reference does not form an Arc cycle that would pin the
    /// slot; `resolve` upgrades it during a call (the caller holds a strong ref).
    pub fn set_self_ref(&self, name: String, weak: Weak<PureFuncSlot>) {
        *self.inner.self_ref.borrow_mut() = Some((name, weak));
    }

    /// Release every strong capture (and the weak self-ref) held by this scope.
    /// Used by the VM's reset drain to break letrec *mutual*-recursion cycles
    /// (strong `PatchClosure` captures) so cyclic runtime closures are reclaimed
    /// rather than leaked past the VM's life. Idempotent: the later
    /// `ClosureScopeInner::drop` drains an already-empty map.
    pub fn clear_captured(&self) {
        for (_, v) in self.inner.captured.borrow_mut().drain(..) {
            v.decref();
        }
        *self.inner.self_ref.borrow_mut() = None; // weak, nothing to decref
    }

    /// Bind a name directly in this scope's captured set, regardless of the
    /// parent chain (let-rec self-reference binding by MakeFunction).
    pub fn insert_captured(&self, name: &str, value: Value) {
        // Owned-in: release any entry this overwrites (a re-bound letrec name).
        if let Some(old) = self.inner.captured.borrow_mut().insert(name.to_string(), value) {
            old.decref();
        }
    }

    /// Set a variable in the nearest scope that contains it.
    pub fn set(&self, name: &str, value: Value) -> bool {
        let mut captured = self.inner.captured.borrow_mut();
        if captured.contains_key(name) {
            // Owned-in on success: the map takes the incoming ref, the
            // overwritten entry releases hers (mirrors VmHost::store_global).
            if let Some(old) = captured.insert(name.to_string(), value) {
                old.decref();
            }
            return true;
        }
        drop(captured);
        match &self.inner.parent {
            PureClosureParent::Scope(parent) => parent.set(name, value),
            PureClosureParent::Globals(globals) => {
                let mut g = globals.borrow_mut();
                if g.contains_key(name) {
                    if let Some(old) = g.insert(name.to_string(), value) {
                        old.decref();
                    }
                    true
                } else {
                    false
                }
            }
            PureClosureParent::None => false,
        }
    }

    /// A clone of the parent link (Globals `Rc` / parent scope `Rc` / None). Used
    /// by the module loader to rebuild an exported closure's scope chain against
    /// the parent VM's remapped templates.
    pub fn parent(&self) -> PureClosureParent {
        self.inner.parent.clone()
    }

    /// The letrec self-reference name, if any (the loader re-establishes it as a
    /// weak handle to the *remapped* slot).
    pub fn self_ref_name(&self) -> Option<String> {
        self.inner.self_ref.borrow().as_ref().map(|(n, _)| n.clone())
    }

    /// Identity of the underlying `Rc<ClosureScopeInner>` -- a stable key for
    /// memoizing a scope during the loader's cyclic closure-graph remap.
    pub fn inner_ptr(&self) -> usize {
        Rc::as_ptr(&self.inner) as usize
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
