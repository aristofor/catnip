// FILE: catnip_vm/src/collections/tuple.rs
//! NativeTuple -- immutable sequence backed by Box<[Value]>.

use super::clamp_slice_index;
use crate::collections::list::normalize_index;
use crate::error::{VMError, VMResult};
use crate::value::Value;

/// Immutable tuple. Stored as `Arc<NativeTuple>` in NaN-boxed Value (tag 11).
pub struct NativeTuple {
    inner: Box<[Value]>,
}

impl NativeTuple {
    #[inline]
    pub fn new(items: Vec<Value>) -> Self {
        NativeTuple {
            inner: items.into_boxed_slice(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn get(&self, index: i64) -> VMResult<Value> {
        let idx = normalize_index(index, self.inner.len())?;
        let v = self.inner[idx];
        v.clone_refcount();
        Ok(v)
    }

    pub fn contains(&self, value: Value) -> bool {
        self.inner.contains(&value)
    }

    pub fn index(&self, value: Value) -> VMResult<usize> {
        for (i, v) in self.inner.iter().enumerate() {
            if *v == value {
                return Ok(i);
            }
        }
        Err(VMError::ValueError("value not in tuple".into()))
    }

    pub fn count(&self, value: Value) -> usize {
        self.inner.iter().filter(|v| **v == value).count()
    }

    /// Slice with Python semantics.
    pub fn slice(&self, start: Option<i64>, end: Option<i64>) -> Vec<Value> {
        let len = self.inner.len() as i64;
        let s = clamp_slice_index(start.unwrap_or(0), len);
        let e = clamp_slice_index(end.unwrap_or(len), len);
        if s >= e {
            return vec![];
        }
        let result = self.inner[s..e].to_vec();
        for v in &result {
            v.clone_refcount();
        }
        result
    }

    #[inline]
    pub fn as_slice(&self) -> &[Value] {
        &self.inner
    }
}

impl Drop for NativeTuple {
    fn drop(&mut self) {
        for v in self.inner.iter() {
            v.decref();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tuple_basic() {
        let t = NativeTuple::new(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
        assert_eq!(t.len(), 3);
        assert_eq!(t.get(0).unwrap(), Value::from_int(1));
        assert_eq!(t.get(-1).unwrap(), Value::from_int(3));
    }

    #[test]
    fn test_tuple_contains() {
        let t = NativeTuple::new(vec![Value::from_int(1), Value::from_int(2)]);
        assert!(t.contains(Value::from_int(1)));
        assert!(!t.contains(Value::from_int(3)));
    }

    #[test]
    fn test_tuple_immutability() {
        // NativeTuple has no set/push/pop methods -- immutability is structural
        let t = NativeTuple::new(vec![Value::from_int(1)]);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn test_tuple_slice() {
        let t = NativeTuple::new(vec![Value::from_int(0), Value::from_int(1), Value::from_int(2)]);
        let s = t.slice(Some(1), None);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], Value::from_int(1));
    }

    #[test]
    fn test_tuple_count() {
        let t = NativeTuple::new(vec![Value::from_int(1), Value::from_int(2), Value::from_int(1)]);
        assert_eq!(t.count(Value::from_int(1)), 2);
    }
}
