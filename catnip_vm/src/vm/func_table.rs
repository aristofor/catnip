// FILE: catnip_vm/src/vm/func_table.rs
//! Pure Rust function table for TAG_VMFUNC handles.
//!
//! Grow-only: functions are defined once, called many times.
//! No PyCodeObject, no Python context.

use super::closure::PureClosureScope;
use crate::compiler::code_object::CodeObject;
use std::sync::Arc;

/// A VM function slot: code + optional closure scope.
pub struct PureFuncSlot {
    pub code: Arc<CodeObject>,
    pub closure: Option<PureClosureScope>,
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
            encoded_ir: None,
        })
    }

    #[test]
    fn test_insert_and_get() {
        let mut table = PureFunctionTable::new();
        let code = dummy_code("f");
        let idx = table.insert(PureFuncSlot {
            code: code.clone(),
            closure: None,
        });
        assert_eq!(idx, 0);
        assert_eq!(table.get(0).unwrap().code.name, "f");
        assert!(table.get(1).is_none());
    }

    #[test]
    fn test_grow_only() {
        let mut table = PureFunctionTable::new();
        for i in 0..10 {
            let idx = table.insert(PureFuncSlot {
                code: dummy_code(&format!("f{i}")),
                closure: None,
            });
            assert_eq!(idx, i);
        }
        assert_eq!(table.len(), 10);
        // All indices remain valid
        for i in 0..10u32 {
            assert!(table.get(i).is_some());
        }
    }
}
