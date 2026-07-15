// FILE: catnip_vm/src/vm/func_table.rs
//! Pure Rust function table for TAG_VMFUNC handles.
//!
//! Grow-only: functions are defined once, called many times.
//! No PyCodeObject, no Python context.

use super::closure::PureClosureScope;
use crate::Value;
use crate::compiler::code_object::CodeObject;
use std::sync::Arc;

#[cfg(test)]
thread_local! {
    /// Test-only: number of live *runtime* closure slots (Arc-backed
    /// `PureFuncSlot`s not yet dropped) on the current thread. A runtime slot is
    /// born at `PureFuncSlot::new_runtime` (`MakeFunction` / bound-method access)
    /// and dies when its backing `Arc` drops to zero. A function-scoped program
    /// must return this to its baseline once the slots' last `Value` dies -- the
    /// leak the Arc-in-value model closes. Template slots (index-based in the
    /// grow-only table) are *not* counted, so the baseline is unpolluted by
    /// per-source compilation. Thread-local (like `live_struct_instances`) so
    /// parallel tests never race; correct because a catnip_vm slot never crosses
    /// threads. Read only by the leak oracles.
    static LIVE_FUNC_SLOTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only: number of live runtime closure slots on the current thread.
#[cfg(test)]
#[inline]
pub fn live_func_slots() -> usize {
    LIVE_FUNC_SLOTS.with(std::cell::Cell::get)
}

/// A VM function slot: code + optional closure scope.
pub struct PureFuncSlot {
    pub code: Arc<CodeObject>,
    pub closure: Option<PureClosureScope>,
    /// Curried receiver for a bound method (`m = p.get`): prepended to the
    /// call's positional args at Call/TailCall so `self` binds to the method's
    /// first parameter -- exactly like the direct `p.get()` path, which prepends
    /// the receiver. `None` for a plain function or closure. The slot owns this
    /// ref (clone_refcount at bind); the Drop releases it when the slot's last
    /// `Value` dies (runtime slot) or the table drops (template).
    pub bound_self: Option<Value>,
    /// True for a runtime slot (Arc-backed, born at `new_runtime`), false for a
    /// template slot (index-based in the grow-only table). Only the runtime slots
    /// participate in the live-count -- templates persist for the VM's life and
    /// would pollute the leak oracles' baseline. Test-only: the count exists to
    /// verify reclamation, and production carries no per-slot bit.
    #[cfg(test)]
    counted: bool,
}

impl PureFuncSlot {
    /// A template slot: index-based in the grow-only table, permanent for the
    /// VM's life. Populated at compile/load (and carried by module transplant,
    /// which may bring a captured closure); never counted, never freed before the
    /// whole table drops. Templates never carry a curried receiver.
    pub fn template(code: Arc<CodeObject>, closure: Option<PureClosureScope>) -> Self {
        PureFuncSlot {
            code,
            closure,
            bound_self: None,
            #[cfg(test)]
            counted: false,
        }
    }

    /// A runtime slot: Arc-backed, born at `MakeFunction` (closure) or bound-method
    /// access (`bound_self`). Reclaimed when its last `Value` dies.
    pub fn new_runtime(code: Arc<CodeObject>, closure: Option<PureClosureScope>, bound_self: Option<Value>) -> Self {
        #[cfg(test)]
        LIVE_FUNC_SLOTS.with(|c| c.set(c.get() + 1));
        PureFuncSlot {
            code,
            closure,
            bound_self,
            #[cfg(test)]
            counted: true,
        }
    }

    /// Wrap a runtime slot in its refcounting `Arc`. The Arc strong count is the
    /// slot's refcount; a `TAG_CLOSURE` `Value` is a thin pointer into it.
    #[allow(clippy::arc_with_non_send_sync)] // VM is single-threaded, Arc for refcounting only
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}

impl Drop for PureFuncSlot {
    fn drop(&mut self) {
        #[cfg(test)]
        if self.counted {
            LIVE_FUNC_SLOTS.with(|c| c.set(c.get().saturating_sub(1)));
        }
        if let Some(v) = self.bound_self {
            v.decref();
        }
    }
}

/// Grow-only table mapping u32 indices to function data.
pub struct PureFunctionTable {
    slots: Vec<PureFuncSlot>,
}

impl PureFunctionTable {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Insert a function and return its index.
    pub fn insert(&mut self, slot: PureFuncSlot) -> u32 {
        let idx = self.slots.len() as u32;
        self.slots.push(slot);
        idx
    }

    /// Get a function slot by index.
    #[inline]
    pub fn get(&self, idx: u32) -> Option<&PureFuncSlot> {
        self.slots.get(idx as usize)
    }

    /// Number of registered functions.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

impl Default for PureFunctionTable {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_code(name: &str) -> Arc<CodeObject> {
        Arc::new(CodeObject {
            instructions: vec![],
            constants: vec![],
            names: vec![],
            nlocals: 0,
            varnames: vec![],
            slotmap: Default::default(),
            nargs: 0,
            defaults: vec![],
            name: name.into(),
            freevars: vec![],
            vararg_idx: -1,
            is_pure: false,
            complexity: 0,
            line_table: vec![],
            patterns: vec![],
            union_checks: vec![],
            composite_checks: vec![],
            generic_checks: vec![],
            encoded_ir: None,
        })
    }

    #[test]
    fn test_insert_and_get() {
        let mut table = PureFunctionTable::new();
        let code = dummy_code("f");
        let idx = table.insert(PureFuncSlot::template(code.clone(), None));
        assert_eq!(idx, 0);
        assert_eq!(table.get(0).unwrap().code.name, "f");
        assert!(table.get(1).is_none());
    }

    #[test]
    fn test_grow_only() {
        let mut table = PureFunctionTable::new();
        for i in 0..10 {
            let idx = table.insert(PureFuncSlot::template(dummy_code(&format!("f{i}")), None));
            assert_eq!(idx, i);
        }
        assert_eq!(table.len(), 10);
        // All indices remain valid
        for i in 0..10u32 {
            assert!(table.get(i).is_some());
        }
    }
}
