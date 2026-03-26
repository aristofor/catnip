// FILE: catnip_vm/src/collections/list.rs
//! NativeList -- mutable list backed by RefCell<Vec<Value>>.

use crate::error::{VMError, VMResult};
use crate::value::Value;
use std::cell::RefCell;

/// Mutable list. Stored as `Arc<NativeList>` in NaN-boxed Value (tag 9).
pub struct NativeList {
    inner: RefCell<Vec<Value>>,
}

impl NativeList {
    #[inline]
    pub fn new(items: Vec<Value>) -> Self {
        NativeList {
            inner: RefCell::new(items),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// Get item by index (supports negative indexing).
    pub fn get(&self, index: i64) -> VMResult<Value> {
        let inner = self.inner.borrow();
        let idx = normalize_index(index, inner.len())?;
        let v = inner[idx];
        v.clone_refcount();
        Ok(v)
    }

    /// Set item by index (supports negative indexing).
    pub fn set(&self, index: i64, value: Value) -> VMResult<()> {
        let mut inner = self.inner.borrow_mut();
        let idx = normalize_index(index, inner.len())?;
        let old = inner[idx];
        old.decref();
        inner[idx] = value;
        value.clone_refcount();
        Ok(())
    }

    pub fn push(&self, value: Value) {
        value.clone_refcount();
        self.inner.borrow_mut().push(value);
    }

    pub fn pop(&self) -> VMResult<Value> {
        self.inner
            .borrow_mut()
            .pop()
            .ok_or_else(|| VMError::IndexError("pop from empty list".into()))
    }

    pub fn insert(&self, index: i64, value: Value) {
        let mut inner = self.inner.borrow_mut();
        let len = inner.len();
        let idx = if index < 0 {
            let i = index + len as i64;
            if i < 0 { 0 } else { i as usize }
        } else if (index as usize) > len {
            len
        } else {
            index as usize
        };
        value.clone_refcount();
        inner.insert(idx, value);
    }

    /// Remove first occurrence of value. Error if not found.
    pub fn remove(&self, value: Value) -> VMResult<()> {
        let mut inner = self.inner.borrow_mut();
        for (i, v) in inner.iter().enumerate() {
            if *v == value {
                let removed = inner.remove(i);
                removed.decref();
                return Ok(());
            }
        }
        Err(VMError::ValueError("list.remove(x): x not in list".into()))
    }

    pub fn reverse(&self) {
        self.inner.borrow_mut().reverse();
    }

    /// Sort in place (ascending). Only works if all elements are comparable.
    pub fn sort(&self) -> VMResult<()> {
        let mut inner = self.inner.borrow_mut();
        let mut err: Option<VMError> = None;
        inner.sort_by(|a, b| match value_cmp(*a, *b) {
            Ok(ord) => ord,
            Err(e) => {
                if err.is_none() {
                    err = Some(e);
                }
                std::cmp::Ordering::Equal
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
        Ok(())
    }

    pub fn contains(&self, value: Value) -> bool {
        self.inner.borrow().contains(&value)
    }

    /// Index of first occurrence, or error.
    pub fn index(&self, value: Value) -> VMResult<usize> {
        let inner = self.inner.borrow();
        for (i, v) in inner.iter().enumerate() {
            if *v == value {
                return Ok(i);
            }
        }
        Err(VMError::ValueError("value not in list".into()))
    }

    pub fn count(&self, value: Value) -> usize {
        self.inner.borrow().iter().filter(|v| **v == value).count()
    }

    pub fn clear(&self) {
        let mut inner = self.inner.borrow_mut();
        for v in inner.drain(..) {
            v.decref();
        }
    }

    /// Shallow copy (clones refcounts).
    pub fn copy(&self) -> Vec<Value> {
        let inner = self.inner.borrow();
        for v in inner.iter() {
            v.clone_refcount();
        }
        inner.clone()
    }

    pub fn extend(&self, items: &[Value]) {
        let mut inner = self.inner.borrow_mut();
        for v in items {
            v.clone_refcount();
            inner.push(*v);
        }
    }

    /// Slice with Python semantics.
    pub fn slice(&self, start: Option<i64>, end: Option<i64>) -> Vec<Value> {
        let inner = self.inner.borrow();
        let len = inner.len() as i64;
        let s = clamp_slice_index(start.unwrap_or(0), len);
        let e = clamp_slice_index(end.unwrap_or(len), len);
        if s >= e {
            return vec![];
        }
        let result = inner[s..e].to_vec();
        for v in &result {
            v.clone_refcount();
        }
        result
    }

    /// Borrow inner Vec for iteration (short borrow).
    pub fn as_slice_cloned(&self) -> Vec<Value> {
        let inner = self.inner.borrow();
        for v in inner.iter() {
            v.clone_refcount();
        }
        inner.clone()
    }
}

impl Drop for NativeList {
    fn drop(&mut self) {
        for v in self.inner.get_mut().drain(..) {
            v.decref();
        }
    }
}

/// Normalize a Python-style index to a valid usize.
pub fn normalize_index(index: i64, len: usize) -> VMResult<usize> {
    let idx = if index < 0 { index + len as i64 } else { index };
    if idx < 0 || idx >= len as i64 {
        return Err(VMError::IndexError(format!(
            "index {} out of range (len {})",
            index, len
        )));
    }
    Ok(idx as usize)
}

/// Clamp a slice index to [0, len].
fn clamp_slice_index(index: i64, len: i64) -> usize {
    if index < 0 {
        let i = index + len;
        if i < 0 { 0 } else { i as usize }
    } else if index > len {
        len as usize
    } else {
        index as usize
    }
}

/// Compare two Values for ordering.
fn value_cmp(a: Value, b: Value) -> VMResult<std::cmp::Ordering> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(ai.cmp(&bi));
    }
    // Float comparison
    let af = value_to_f64(a);
    let bf = value_to_f64(b);
    if let (Some(af), Some(bf)) = (af, bf) {
        return af
            .partial_cmp(&bf)
            .ok_or_else(|| VMError::TypeError("unorderable types".into()));
    }
    // String comparison
    if a.is_native_str() && b.is_native_str() {
        let sa = unsafe { a.as_native_str_ref().unwrap() };
        let sb = unsafe { b.as_native_str_ref().unwrap() };
        return Ok(sa.cmp(sb));
    }
    Err(VMError::TypeError("'<' not supported between instances".into()))
}

fn value_to_f64(v: Value) -> Option<f64> {
    if let Some(i) = v.as_int() {
        Some(i as f64)
    } else {
        v.as_float()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_basic() {
        let list = NativeList::new(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
        assert_eq!(list.len(), 3);
        assert!(!list.is_empty());
        assert_eq!(list.get(0).unwrap(), Value::from_int(1));
        assert_eq!(list.get(-1).unwrap(), Value::from_int(3));
    }

    #[test]
    fn test_list_set() {
        let list = NativeList::new(vec![Value::from_int(1), Value::from_int(2)]);
        list.set(0, Value::from_int(10)).unwrap();
        assert_eq!(list.get(0).unwrap(), Value::from_int(10));
    }

    #[test]
    fn test_list_push_pop() {
        let list = NativeList::new(vec![]);
        list.push(Value::from_int(1));
        list.push(Value::from_int(2));
        assert_eq!(list.len(), 2);
        let popped = list.pop().unwrap();
        assert_eq!(popped, Value::from_int(2));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_list_insert() {
        let list = NativeList::new(vec![Value::from_int(1), Value::from_int(3)]);
        list.insert(1, Value::from_int(2));
        assert_eq!(list.get(1).unwrap(), Value::from_int(2));
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_list_remove() {
        let list = NativeList::new(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
        list.remove(Value::from_int(2)).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.remove(Value::from_int(99)).is_err());
    }

    #[test]
    fn test_list_contains() {
        let list = NativeList::new(vec![Value::from_int(1), Value::from_int(2)]);
        assert!(list.contains(Value::from_int(1)));
        assert!(!list.contains(Value::from_int(3)));
    }

    #[test]
    fn test_list_sort() {
        let list = NativeList::new(vec![Value::from_int(3), Value::from_int(1), Value::from_int(2)]);
        list.sort().unwrap();
        assert_eq!(list.get(0).unwrap(), Value::from_int(1));
        assert_eq!(list.get(1).unwrap(), Value::from_int(2));
        assert_eq!(list.get(2).unwrap(), Value::from_int(3));
    }

    #[test]
    fn test_list_slice() {
        let list = NativeList::new(vec![
            Value::from_int(0),
            Value::from_int(1),
            Value::from_int(2),
            Value::from_int(3),
        ]);
        let s = list.slice(Some(1), Some(3));
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], Value::from_int(1));
        assert_eq!(s[1], Value::from_int(2));
    }

    #[test]
    fn test_list_index_out_of_range() {
        let list = NativeList::new(vec![Value::from_int(1)]);
        assert!(list.get(5).is_err());
        assert!(list.get(-5).is_err());
    }

    #[test]
    fn test_list_count() {
        let list = NativeList::new(vec![Value::from_int(1), Value::from_int(2), Value::from_int(1)]);
        assert_eq!(list.count(Value::from_int(1)), 2);
        assert_eq!(list.count(Value::from_int(3)), 0);
    }
}
