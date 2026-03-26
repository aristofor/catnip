// FILE: catnip_vm/src/collections/dict.rs
//! NativeDict -- mutable ordered dict backed by RefCell<IndexMap<ValueKey, Value>>.

use crate::collections::ValueKey;
use crate::error::{VMError, VMResult};
use crate::value::Value;
use indexmap::IndexMap;
use std::cell::RefCell;

/// Mutable ordered dict. Stored as `Arc<NativeDict>` in NaN-boxed Value (tag 10).
pub struct NativeDict {
    inner: RefCell<IndexMap<ValueKey, Value>>,
}

impl NativeDict {
    #[inline]
    pub fn new(items: IndexMap<ValueKey, Value>) -> Self {
        NativeDict {
            inner: RefCell::new(items),
        }
    }

    pub fn empty() -> Self {
        Self::new(IndexMap::new())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    pub fn get_item(&self, key: &ValueKey) -> VMResult<Value> {
        let inner = self.inner.borrow();
        match inner.get(key) {
            Some(&v) => {
                v.clone_refcount();
                Ok(v)
            }
            None => Err(VMError::KeyError(format!("{:?}", key))),
        }
    }

    pub fn set_item(&self, key: ValueKey, value: Value) {
        let mut inner = self.inner.borrow_mut();
        if let Some(old) = inner.insert(key, value) {
            old.decref();
        }
        value.clone_refcount();
    }

    pub fn del_item(&self, key: &ValueKey) -> VMResult<()> {
        let mut inner = self.inner.borrow_mut();
        match inner.swap_remove(key) {
            Some(v) => {
                v.decref();
                Ok(())
            }
            None => Err(VMError::KeyError(format!("{:?}", key))),
        }
    }

    /// Get with default value (does not error on missing key).
    pub fn get_default(&self, key: &ValueKey, default: Value) -> Value {
        let inner = self.inner.borrow();
        match inner.get(key) {
            Some(&v) => {
                v.clone_refcount();
                v
            }
            None => {
                default.clone_refcount();
                default
            }
        }
    }

    /// Return keys as Values.
    pub fn keys(&self) -> Vec<Value> {
        self.inner.borrow().keys().map(|k| k.to_value()).collect()
    }

    /// Return values (with refcount incremented).
    pub fn values(&self) -> Vec<Value> {
        let inner = self.inner.borrow();
        inner
            .values()
            .map(|&v| {
                v.clone_refcount();
                v
            })
            .collect()
    }

    /// Return (key, value) pairs as tuple Values.
    pub fn items(&self) -> Vec<Value> {
        let inner = self.inner.borrow();
        inner
            .iter()
            .map(|(k, &v)| {
                v.clone_refcount();
                Value::from_tuple(vec![k.to_value(), v])
            })
            .collect()
    }

    pub fn contains_key(&self, key: &ValueKey) -> bool {
        self.inner.borrow().contains_key(key)
    }

    /// Merge another dict's entries.
    pub fn update(&self, other: &NativeDict) {
        let other_inner = other.inner.borrow();
        let mut inner = self.inner.borrow_mut();
        for (k, &v) in other_inner.iter() {
            v.clone_refcount();
            if let Some(old) = inner.insert(k.clone(), v) {
                old.decref();
            }
        }
    }

    /// Remove key and return its value, or error.
    pub fn pop(&self, key: &ValueKey) -> VMResult<Value> {
        let mut inner = self.inner.borrow_mut();
        match inner.swap_remove(key) {
            Some(v) => Ok(v), // caller owns the refcount
            None => Err(VMError::KeyError(format!("{:?}", key))),
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.borrow_mut();
        for (_, v) in inner.drain(..) {
            v.decref();
        }
    }

    /// Shallow copy (clones refcounts).
    pub fn copy(&self) -> IndexMap<ValueKey, Value> {
        let inner = self.inner.borrow();
        let mut result = IndexMap::with_capacity(inner.len());
        for (k, &v) in inner.iter() {
            v.clone_refcount();
            result.insert(k.clone(), v);
        }
        result
    }

    /// Borrow keys for iteration (short-lived clone).
    pub fn keys_cloned(&self) -> Vec<ValueKey> {
        self.inner.borrow().keys().cloned().collect()
    }
}

impl Drop for NativeDict {
    fn drop(&mut self) {
        for (_, v) in self.inner.get_mut().drain(..) {
            v.decref();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dict_basic() {
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        d.set_item(
            ValueKey::Str(std::sync::Arc::new(crate::value::NativeString::new("key".into()))),
            Value::from_int(20),
        );
        assert_eq!(d.len(), 2);
        assert_eq!(d.get_item(&ValueKey::Int(1)).unwrap(), Value::from_int(10));
    }

    #[test]
    fn test_dict_overwrite() {
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        d.set_item(ValueKey::Int(1), Value::from_int(20));
        assert_eq!(d.len(), 1);
        assert_eq!(d.get_item(&ValueKey::Int(1)).unwrap(), Value::from_int(20));
    }

    #[test]
    fn test_dict_delete() {
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        d.del_item(&ValueKey::Int(1)).unwrap();
        assert!(d.is_empty());
        assert!(d.del_item(&ValueKey::Int(1)).is_err());
    }

    #[test]
    fn test_dict_get_default() {
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        let v = d.get_default(&ValueKey::Int(1), Value::from_int(0));
        assert_eq!(v, Value::from_int(10));
        let v = d.get_default(&ValueKey::Int(2), Value::from_int(0));
        assert_eq!(v, Value::from_int(0));
    }

    #[test]
    fn test_dict_keys_values_items() {
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        d.set_item(ValueKey::Int(2), Value::from_int(20));

        let keys = d.keys();
        assert_eq!(keys.len(), 2);

        let values = d.values();
        assert_eq!(values.len(), 2);

        let items = d.items();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_dict_contains_key() {
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        assert!(d.contains_key(&ValueKey::Int(1)));
        assert!(!d.contains_key(&ValueKey::Int(2)));
    }

    #[test]
    fn test_dict_pop() {
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        let v = d.pop(&ValueKey::Int(1)).unwrap();
        assert_eq!(v, Value::from_int(10));
        assert!(d.is_empty());
    }

    #[test]
    fn test_dict_cross_type_key() {
        // hash(1) == hash(True) == hash(1.0) -- same key
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        assert_eq!(d.get_item(&ValueKey::Bool(true)).unwrap(), Value::from_int(10));
        assert_eq!(
            d.get_item(&ValueKey::Float(1.0_f64.to_bits())).unwrap(),
            Value::from_int(10)
        );
    }

    #[test]
    fn test_dict_order_preserved() {
        let d = NativeDict::empty();
        d.set_item(ValueKey::Int(3), Value::from_int(30));
        d.set_item(ValueKey::Int(1), Value::from_int(10));
        d.set_item(ValueKey::Int(2), Value::from_int(20));
        let keys = d.keys();
        assert_eq!(keys[0].as_int(), Some(3));
        assert_eq!(keys[1].as_int(), Some(1));
        assert_eq!(keys[2].as_int(), Some(2));
    }
}
