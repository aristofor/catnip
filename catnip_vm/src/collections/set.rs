// FILE: catnip_vm/src/collections/set.rs
//! NativeSet -- mutable set backed by RefCell<IndexSet<ValueKey>>.

use crate::collections::ValueKey;
use crate::error::{VMError, VMResult};
use crate::value::Value;
use indexmap::IndexSet;
use std::cell::RefCell;

/// Mutable set. Stored as `Arc<NativeSet>` in NaN-boxed Value (tag 12).
pub struct NativeSet {
    inner: RefCell<IndexSet<ValueKey>>,
}

impl NativeSet {
    #[inline]
    pub fn new(items: IndexSet<ValueKey>) -> Self {
        NativeSet {
            inner: RefCell::new(items),
        }
    }

    pub fn empty() -> Self {
        Self::new(IndexSet::new())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    pub fn add(&self, key: ValueKey) {
        self.inner.borrow_mut().insert(key);
    }

    /// Remove key, error if not present.
    pub fn remove(&self, key: &ValueKey) -> VMResult<()> {
        if self.inner.borrow_mut().swap_remove(key) {
            Ok(())
        } else {
            Err(VMError::KeyError(format!("{:?}", key)))
        }
    }

    /// Remove key if present, no error otherwise.
    pub fn discard(&self, key: &ValueKey) {
        self.inner.borrow_mut().swap_remove(key);
    }

    pub fn contains(&self, key: &ValueKey) -> bool {
        self.inner.borrow().contains(key)
    }

    /// Remove and return an arbitrary element.
    pub fn pop(&self) -> VMResult<ValueKey> {
        self.inner
            .borrow_mut()
            .pop()
            .ok_or_else(|| VMError::KeyError("pop from an empty set".into()))
    }

    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    pub fn copy(&self) -> IndexSet<ValueKey> {
        self.inner.borrow().clone()
    }

    pub fn union(&self, other: &NativeSet) -> IndexSet<ValueKey> {
        let a = self.inner.borrow();
        let b = other.inner.borrow();
        a.union(&*b).cloned().collect()
    }

    pub fn intersection(&self, other: &NativeSet) -> IndexSet<ValueKey> {
        let a = self.inner.borrow();
        let b = other.inner.borrow();
        a.intersection(&*b).cloned().collect()
    }

    pub fn difference(&self, other: &NativeSet) -> IndexSet<ValueKey> {
        let a = self.inner.borrow();
        let b = other.inner.borrow();
        a.difference(&*b).cloned().collect()
    }

    /// Clone keys as Values for iteration.
    pub fn to_values(&self) -> Vec<Value> {
        self.inner.borrow().iter().map(|k| k.to_value()).collect()
    }
}

// NativeSet doesn't need Drop cascade because ValueKey owns its Arcs
// and IndexSet will drop them automatically.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_basic() {
        let s = NativeSet::empty();
        s.add(ValueKey::Int(1));
        s.add(ValueKey::Int(2));
        s.add(ValueKey::Int(1)); // duplicate
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn test_set_remove() {
        let s = NativeSet::empty();
        s.add(ValueKey::Int(1));
        s.remove(&ValueKey::Int(1)).unwrap();
        assert!(s.is_empty());
        assert!(s.remove(&ValueKey::Int(1)).is_err());
    }

    #[test]
    fn test_set_discard() {
        let s = NativeSet::empty();
        s.add(ValueKey::Int(1));
        s.discard(&ValueKey::Int(1));
        s.discard(&ValueKey::Int(1)); // no error
        assert!(s.is_empty());
    }

    #[test]
    fn test_set_contains() {
        let s = NativeSet::empty();
        s.add(ValueKey::Int(1));
        assert!(s.contains(&ValueKey::Int(1)));
        assert!(!s.contains(&ValueKey::Int(2)));
    }

    #[test]
    fn test_set_union_intersection_difference() {
        let a = NativeSet::empty();
        a.add(ValueKey::Int(1));
        a.add(ValueKey::Int(2));
        a.add(ValueKey::Int(3));

        let b = NativeSet::empty();
        b.add(ValueKey::Int(2));
        b.add(ValueKey::Int(3));
        b.add(ValueKey::Int(4));

        let union = a.union(&b);
        assert_eq!(union.len(), 4);

        let inter = a.intersection(&b);
        assert_eq!(inter.len(), 2);

        let diff = a.difference(&b);
        assert_eq!(diff.len(), 1);
        assert!(diff.contains(&ValueKey::Int(1)));
    }
}
