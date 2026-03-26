// FILE: catnip_vm/src/collections/bytes.rs
//! NativeBytes -- immutable byte sequence backed by Box<[u8]>.

use crate::collections::list::normalize_index;
use crate::error::{VMError, VMResult};
use crate::value::Value;

/// Immutable bytes. Stored as `Arc<NativeBytes>` in NaN-boxed Value (tag 13).
pub struct NativeBytes {
    inner: Box<[u8]>,
}

impl NativeBytes {
    #[inline]
    pub fn new(data: Vec<u8>) -> Self {
        NativeBytes {
            inner: data.into_boxed_slice(),
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

    /// Get byte at index (returns int 0-255).
    pub fn get(&self, index: i64) -> VMResult<Value> {
        let idx = normalize_index(index, self.inner.len())?;
        Ok(Value::from_int(self.inner[idx] as i64))
    }

    /// Check if byte value is contained.
    pub fn contains_byte(&self, byte: u8) -> bool {
        self.inner.contains(&byte)
    }

    /// Decode to UTF-8 string.
    pub fn decode(&self) -> VMResult<Value> {
        match std::str::from_utf8(&self.inner) {
            Ok(s) => Ok(Value::from_str(s)),
            Err(e) => Err(VMError::ValueError(format!("cannot decode bytes: {}", e))),
        }
    }

    /// Slice with Python semantics.
    pub fn slice(&self, start: Option<i64>, end: Option<i64>) -> Vec<u8> {
        let len = self.inner.len() as i64;
        let s = clamp_index(start.unwrap_or(0), len);
        let e = clamp_index(end.unwrap_or(len), len);
        if s >= e {
            return vec![];
        }
        self.inner[s..e].to_vec()
    }

    /// Hex representation.
    pub fn hex(&self) -> String {
        self.inner.iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.inner
    }
}

fn clamp_index(index: i64, len: i64) -> usize {
    if index < 0 {
        let i = index + len;
        if i < 0 { 0 } else { i as usize }
    } else if index > len {
        len as usize
    } else {
        index as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_basic() {
        let b = NativeBytes::new(vec![1, 2, 3]);
        assert_eq!(b.len(), 3);
        assert_eq!(b.get(0).unwrap(), Value::from_int(1));
        assert_eq!(b.get(-1).unwrap(), Value::from_int(3));
    }

    #[test]
    fn test_bytes_contains() {
        let b = NativeBytes::new(vec![10, 20, 30]);
        assert!(b.contains_byte(20));
        assert!(!b.contains_byte(40));
    }

    #[test]
    fn test_bytes_decode() {
        let b = NativeBytes::new(b"hello".to_vec());
        let v = b.decode().unwrap();
        assert_eq!(unsafe { v.as_native_str_ref() }, Some("hello"));
        v.decref();
    }

    #[test]
    fn test_bytes_hex() {
        let b = NativeBytes::new(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(b.hex(), "deadbeef");
    }

    #[test]
    fn test_bytes_slice() {
        let b = NativeBytes::new(vec![0, 1, 2, 3, 4]);
        let s = b.slice(Some(1), Some(4));
        assert_eq!(s, vec![1, 2, 3]);
    }
}
