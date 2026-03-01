// FILE: catnip_rs/src/vm/value.rs
//! NaN-boxed value representation for the Catnip VM.
//!
//! IEEE 754 quiet NaN with 48-bit payload:
//! [Sign:1][Exponent:11=0x7FF][Quiet:1][Tag:3][Payload:48]
//!   63        62-52            51     50-48    47-0
//!
//! Tags:
//!   0b000 = SmallInt (48-bit signed: -2^47 to 2^47-1)
//!   0b001 = Bool (0=false, 1=true)
//!   0b010 = Nil/None
//!   0b011 = Symbol (interned string index)
//!   0b100 = PyObject* (48-bit pointer)
//!
//! Regular floats are stored directly. Detection via quiet NaN pattern.

use super::structs::{CatnipStructProxy, StructRegistry};
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt};
use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

thread_local! {
    static STRUCT_REGISTRY: Cell<*const StructRegistry> = const { Cell::new(std::ptr::null()) };
}

/// Install a pointer to the active StructRegistry for the current thread.
///
/// # Safety
///
/// The caller must ensure the registry outlives the period during which
/// `to_pyobject` may be called on struct values. Use `clear_struct_registry`
/// (or the RAII guard in the VM) to reset when done.
pub fn set_struct_registry(ptr: *const StructRegistry) {
    STRUCT_REGISTRY.with(|cell| cell.set(ptr));
}

/// Clear the thread-local registry pointer.
pub fn clear_struct_registry() {
    STRUCT_REGISTRY.with(|cell| cell.set(std::ptr::null()));
}

/// Save the current registry pointer (for reentrant VM calls).
pub fn save_struct_registry() -> *const StructRegistry {
    STRUCT_REGISTRY.with(|cell| cell.get())
}

/// Restore a previously saved registry pointer.
///
/// # Safety
///
/// The caller must ensure the pointer is still valid (the owning VM is alive).
pub fn restore_struct_registry(ptr: *const StructRegistry) {
    STRUCT_REGISTRY.with(|cell| cell.set(ptr));
}

/// Quiet NaN base pattern: exponent all 1s + quiet bit
const QNAN_BASE: u64 = 0x7FF8_0000_0000_0000;

/// Mask for tag bits (3 bits at position 48-50)
const TAG_MASK: u64 = 0x0007_0000_0000_0000;

/// Mask for payload (48 bits)
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Sign bit for negative small ints (bit 47 extended to full i64)
const SMALLINT_SIGN_BIT: u64 = 0x0000_8000_0000_0000;
const SMALLINT_SIGN_EXT: u64 = 0xFFFF_0000_0000_0000;

/// Tag values
const TAG_SMALLINT: u64 = 0x0000_0000_0000_0000; // 0b000
const TAG_BOOL: u64 = 0x0001_0000_0000_0000; // 0b001
const TAG_NIL: u64 = 0x0002_0000_0000_0000; // 0b010
const TAG_SYMBOL: u64 = 0x0003_0000_0000_0000; // 0b011
const TAG_PYOBJ: u64 = 0x0004_0000_0000_0000; // 0b100
const TAG_STRUCT: u64 = 0x0005_0000_0000_0000; // 0b101
const TAG_BIGINT: u64 = 0x0006_0000_0000_0000; // 0b110
const TAG_INVALID: u64 = 0x0007_0000_0000_0000; // 0b111

/// Max/min values for small int (48-bit signed)
const SMALLINT_MAX: i64 = (1_i64 << 47) - 1;
const SMALLINT_MIN: i64 = -(1_i64 << 47);

/// NaN-boxed value. Fits in 8 bytes.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Value(u64);

impl Value {
    /// Nil constant (Python None)
    pub const NIL: Value = Value(QNAN_BASE | TAG_NIL);

    /// Boolean true
    pub const TRUE: Value = Value(QNAN_BASE | TAG_BOOL | 1);

    /// Boolean false
    pub const FALSE: Value = Value(QNAN_BASE | TAG_BOOL);

    /// Debug canary for uninitialized slots (tag 0b111).
    /// In debug builds, accessing this value panics instead of silently returning NIL.
    pub const INVALID: Value = Value(QNAN_BASE | TAG_INVALID);

    // --- Constructors ---

    /// Create a float value. NaN floats become canonical NaN.
    #[inline]
    pub fn from_float(f: f64) -> Self {
        let bits = f.to_bits();
        // Check if it's a NaN that would conflict with our tagging
        if (bits & 0x7FF8_0000_0000_0000) == 0x7FF8_0000_0000_0000 {
            // It's a quiet NaN - return canonical NaN to avoid confusion
            Value(f64::NAN.to_bits())
        } else {
            Value(bits)
        }
    }

    /// Create a small integer value. Panics if out of 48-bit range.
    #[inline]
    pub fn from_int(i: i64) -> Self {
        debug_assert!(
            (SMALLINT_MIN..=SMALLINT_MAX).contains(&i),
            "integer out of small int range"
        );
        let payload = (i as u64) & PAYLOAD_MASK;
        Value(QNAN_BASE | TAG_SMALLINT | payload)
    }

    /// Try to create a small integer. Returns None if out of range.
    #[inline]
    pub fn try_from_int(i: i64) -> Option<Self> {
        if (SMALLINT_MIN..=SMALLINT_MAX).contains(&i) {
            Some(Self::from_int(i))
        } else {
            None
        }
    }

    /// Create a boolean value.
    #[inline]
    pub fn from_bool(b: bool) -> Self {
        if b {
            Self::TRUE
        } else {
            Self::FALSE
        }
    }

    /// Create a symbol value (interned string index).
    #[inline]
    pub fn from_symbol(idx: u32) -> Self {
        Value(QNAN_BASE | TAG_SYMBOL | (idx as u64))
    }

    /// Create a PyObject* value. The pointer must fit in 48 bits.
    ///
    /// # Safety
    ///
    /// The caller must ensure the PyObject is kept alive and the pointer
    /// fits within a 48-bit address space.
    #[inline]
    pub unsafe fn from_pyobj_ptr(ptr: *mut pyo3::ffi::PyObject) -> Self {
        let addr = ptr as u64;
        debug_assert!(
            addr & !PAYLOAD_MASK == 0,
            "pointer exceeds 48-bit address space"
        );
        Value(QNAN_BASE | TAG_PYOBJ | (addr & PAYLOAD_MASK))
    }

    /// Create a struct instance value from its index in the StructRegistry.
    #[inline]
    pub fn from_struct_instance(idx: u32) -> Self {
        Value(QNAN_BASE | TAG_STRUCT | (idx as u64))
    }

    /// Create a BigInt value from an Arc<BigInt> pointer.
    #[inline]
    pub fn from_bigint(n: BigInt) -> Self {
        let arc = Arc::new(n);
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(
            ptr & !PAYLOAD_MASK == 0,
            "BigInt Arc pointer exceeds 48-bit address space"
        );
        Value(QNAN_BASE | TAG_BIGINT | (ptr & PAYLOAD_MASK))
    }

    /// Create a BigInt or demote to SmallInt if it fits.
    #[inline]
    pub fn from_bigint_or_demote(n: BigInt) -> Self {
        if let Some(i) = n.to_i64() {
            if let Some(v) = Self::try_from_int(i) {
                return v;
            }
        }
        Self::from_bigint(n)
    }

    /// Get raw bits (for JIT interop).
    #[inline]
    pub fn to_raw(self) -> u64 {
        self.0
    }

    /// Create from raw bits (for JIT interop).
    #[inline]
    pub fn from_raw(bits: u64) -> Self {
        Value(bits)
    }

    // --- Type checks ---

    /// Check if this is a float (not a boxed value).
    #[inline]
    pub fn is_float(self) -> bool {
        // Not a quiet NaN with our pattern
        (self.0 & 0x7FF8_0000_0000_0000) != QNAN_BASE
    }

    /// Check if this is a small integer.
    #[inline]
    pub fn is_int(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_SMALLINT)
    }

    /// Check if this is a boolean.
    #[inline]
    pub fn is_bool(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_BOOL)
    }

    /// Check if this is nil (None).
    #[inline]
    pub fn is_nil(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_NIL)
    }

    /// Check if this is a symbol.
    #[inline]
    pub fn is_symbol(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_SYMBOL)
    }

    /// Check if this is a PyObject pointer.
    #[inline]
    pub fn is_pyobj(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_PYOBJ)
    }

    /// Check if this is a native struct instance.
    #[inline]
    pub fn is_struct_instance(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_STRUCT)
    }

    /// Check if this is a BigInt.
    #[inline]
    pub fn is_bigint(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_BIGINT)
    }

    /// Check if this is the INVALID canary (debug-only sentinel for uninitialized slots).
    #[inline]
    pub fn is_invalid(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_INVALID)
    }

    /// True if this value uses a native VM tag that would lose fidelity
    /// through a Python round-trip (Value -> PyObject -> Value).
    #[inline]
    pub fn has_native_tag(self) -> bool {
        self.is_struct_instance() || self.is_bigint()
    }

    // --- Extractors ---

    /// Extract as float. Returns None if not a float.
    #[inline]
    pub fn as_float(self) -> Option<f64> {
        if self.is_float() {
            Some(f64::from_bits(self.0))
        } else {
            None
        }
    }

    /// Extract as integer. Returns None if not an integer.
    #[inline]
    pub fn as_int(self) -> Option<i64> {
        if self.is_int() {
            let payload = self.0 & PAYLOAD_MASK;
            // Sign extend from 48 bits
            let result = if payload & SMALLINT_SIGN_BIT != 0 {
                (payload | SMALLINT_SIGN_EXT) as i64
            } else {
                payload as i64
            };
            Some(result)
        } else {
            None
        }
    }

    /// Extract as boolean. Returns None if not a boolean.
    #[inline]
    pub fn as_bool(self) -> Option<bool> {
        if self.is_bool() {
            Some((self.0 & 1) != 0)
        } else {
            None
        }
    }

    /// Extract as symbol index. Returns None if not a symbol.
    #[inline]
    pub fn as_symbol(self) -> Option<u32> {
        if self.is_symbol() {
            Some((self.0 & PAYLOAD_MASK) as u32)
        } else {
            None
        }
    }

    /// Extract as PyObject pointer. Returns None if not a PyObject.
    ///
    /// # Safety
    ///
    /// Caller must ensure the original PyObject is still alive before
    /// dereferencing the returned pointer.
    #[inline]
    pub unsafe fn as_pyobj_ptr(self) -> Option<*mut pyo3::ffi::PyObject> {
        if self.is_pyobj() {
            Some((self.0 & PAYLOAD_MASK) as *mut pyo3::ffi::PyObject)
        } else {
            None
        }
    }

    /// Extract struct instance index. Returns None if not a struct.
    #[inline]
    pub fn as_struct_instance_idx(self) -> Option<u32> {
        if self.is_struct_instance() {
            Some((self.0 & PAYLOAD_MASK) as u32)
        } else {
            None
        }
    }

    /// Borrow the BigInt behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive (not yet decremented to 0).
    #[inline]
    pub unsafe fn as_bigint_ref(&self) -> Option<&BigInt> {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const BigInt;
            Some(&*ptr)
        } else {
            None
        }
    }

    /// Increment refcount for reference-counted values (BigInt Arc, PyObject).
    /// Must be called when duplicating a Value (e.g. DupTop).
    #[inline]
    pub fn clone_refcount(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const BigInt;
            unsafe { Arc::increment_strong_count(ptr) };
        } else if self.is_pyobj() {
            unsafe {
                let ptr = self.as_pyobj_ptr().unwrap();
                pyo3::ffi::Py_IncRef(ptr);
            }
        }
    }

    /// Increment refcount only for BigInt values.
    /// Used in LoadLocal to track shared references without affecting PyObject refcounting.
    #[inline]
    pub fn clone_refcount_bigint(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const BigInt;
            unsafe { Arc::increment_strong_count(ptr) };
        }
    }

    // --- Truthiness ---

    /// Check if value is truthy (Python semantics).
    ///
    /// For PyObjects, this is a fast approximation that returns true.
    /// Use `is_truthy_py()` for accurate Python truthiness checking.
    #[inline]
    pub fn is_truthy(self) -> bool {
        if self.is_nil() {
            false
        } else if let Some(b) = self.as_bool() {
            b
        } else if let Some(i) = self.as_int() {
            i != 0
        } else if let Some(f) = self.as_float() {
            f != 0.0
        } else if self.is_bigint() {
            // SAFETY: value is alive (we just checked the tag)
            unsafe { !self.as_bigint_ref().unwrap().is_zero() }
        } else if self.is_struct_instance() {
            true
        } else {
            // PyObject - fast path returns true
            // For accurate checking, use is_truthy_py()
            true
        }
    }

    /// Check if value is truthy using Python semantics.
    ///
    /// This delegates to Python's __bool__() for PyObjects.
    #[inline]
    pub fn is_truthy_py(self, py: pyo3::Python<'_>) -> bool {
        if self.is_nil() {
            false
        } else if let Some(b) = self.as_bool() {
            b
        } else if let Some(i) = self.as_int() {
            i != 0
        } else if let Some(f) = self.as_float() {
            f != 0.0
        } else if self.is_bigint() {
            unsafe { !self.as_bigint_ref().unwrap().is_zero() }
        } else if self.is_struct_instance() {
            true
        } else {
            // PyObject - delegate to Python's __bool__()
            let obj = self.to_pyobject(py);
            obj.bind(py).is_truthy().unwrap_or(true)
        }
    }

    /// Raw bits for debugging.
    #[inline]
    pub fn bits(self) -> u64 {
        self.0
    }
}

// --- PyO3 conversions ---

impl Value {
    /// Convert a Python object to a Value.
    pub fn from_pyobject(_py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        // Try fast paths first
        if obj.is_none() {
            return Ok(Value::NIL);
        }

        if let Ok(b) = obj.cast::<PyBool>() {
            return Ok(Value::from_bool(b.is_true()));
        }

        if let Ok(i) = obj.cast::<PyInt>() {
            // Try to extract as i64
            if let Ok(val) = i.extract::<i64>() {
                if let Some(v) = Value::try_from_int(val) {
                    return Ok(v);
                }
                // Fits i64 but not SmallInt -> BigInt
                return Ok(Value::from_bigint(BigInt::from(val)));
            }
            // Overflow i64 -> extract as BigInt through PyLong native conversion.
            if let Ok(n) = i.extract::<BigInt>() {
                return Ok(Value::from_bigint_or_demote(n));
            }
            // Fall through to PyObject as last resort
        }

        if let Ok(f) = obj.cast::<PyFloat>() {
            let val: f64 = f.extract()?;
            return Ok(Value::from_float(val));
        }

        // Recognize CatnipStructProxy → restore native TAG_STRUCT for VM round-trip.
        // This handles structs stored in Python collections (list, tuple) and extracted
        // back during iteration: the proxy carries the original instance index.
        // Only convert if the CURRENT struct registry owns this index (a nested VM
        // call via Python would have a different registry).
        if let Ok(proxy) = obj.cast::<CatnipStructProxy>() {
            if let Some(idx) = proxy.borrow().native_instance_idx {
                let is_owned = STRUCT_REGISTRY.with(|cell| {
                    let ptr = cell.get();
                    if ptr.is_null() {
                        return false;
                    }
                    let registry = unsafe { &*ptr };
                    registry.get_instance(idx).is_some()
                });
                if is_owned {
                    return Ok(Value::from_struct_instance(idx));
                }
            }
        }

        // Store as PyObject pointer
        // SAFETY: We increment refcount and the Value is tied to this execution
        let ptr = obj.as_ptr();
        unsafe {
            pyo3::ffi::Py_IncRef(ptr);
            Ok(Value::from_pyobj_ptr(ptr))
        }
    }

    /// Convert a Value back to a Python object.
    pub fn to_pyobject(self, py: Python<'_>) -> Py<PyAny> {
        debug_assert!(
            !self.is_invalid(),
            "to_pyobject called on INVALID canary value — uninitialized slot or stack corruption"
        );
        if self.is_nil() {
            py.None()
        } else if let Some(b) = self.as_bool() {
            PyBool::new(py, b).to_owned().into_any().unbind()
        } else if let Some(i) = self.as_int() {
            i.into_pyobject(py).unwrap().unbind().into_any()
        } else if let Some(f) = self.as_float() {
            f.into_pyobject(py).unwrap().unbind().into_any()
        } else if self.is_bigint() {
            // BigInt -> Python int via native PyLong conversion.
            unsafe { self.as_bigint_ref().unwrap() }
                .into_pyobject(py)
                .unwrap()
                .unbind()
                .into_any()
        } else if self.is_struct_instance() {
            let idx = self.as_struct_instance_idx().unwrap();
            STRUCT_REGISTRY.with(|cell| {
                let ptr = cell.get();
                assert!(
                    !ptr.is_null(),
                    "to_pyobject on struct: no StructRegistry installed"
                );
                // SAFETY: The VM installs a valid pointer before execution and
                // clears it after. The registry is owned by the VM and lives
                // for the entire execution. Single-threaded (GIL).
                let registry = unsafe { &*ptr };
                registry.instance_to_pyobject(py, idx).unwrap_or_else(|_| {
                    panic!(
                        "struct instance #{idx} not found in StructRegistry — \
                         likely a TAG_STRUCT leaked into a child VM closure"
                    )
                })
            })
        } else if self.is_pyobj() {
            // SAFETY: We trust the pointer is valid
            unsafe {
                let ptr = self.as_pyobj_ptr().unwrap();
                pyo3::Bound::from_borrowed_ptr(py, ptr).unbind()
            }
        } else {
            // Symbol - for now return as int
            let idx = self.as_symbol().unwrap_or(0);
            (idx as i64).into_pyobject(py).unwrap().unbind().into_any()
        }
    }

    /// Decrement refcount if this is a PyObject or BigInt.
    /// Call when value is no longer needed.
    pub fn decref(self) {
        if self.is_pyobj() {
            unsafe {
                let ptr = self.as_pyobj_ptr().unwrap();
                pyo3::ffi::Py_DecRef(ptr);
            }
        } else if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const BigInt;
            unsafe { Arc::decrement_strong_count(ptr) };
        }
    }

    /// Decrement refcount only for BigInt values.
    /// Used in StoreLocal/PopTop to free intermediate BigInts without
    /// affecting PyObject refcounting (which is managed by Python GC).
    #[inline]
    pub fn decref_bigint(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const BigInt;
            unsafe { Arc::decrement_strong_count(ptr) };
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_nil() {
            write!(f, "nil")
        } else if let Some(b) = self.as_bool() {
            write!(f, "{}", b)
        } else if let Some(i) = self.as_int() {
            write!(f, "{}", i)
        } else if let Some(fl) = self.as_float() {
            write!(f, "{}", fl)
        } else if self.is_bigint() {
            let n = unsafe { self.as_bigint_ref().unwrap() };
            write!(f, "{}n", n)
        } else if let Some(sym) = self.as_symbol() {
            write!(f, "sym#{}", sym)
        } else if let Some(idx) = self.as_struct_instance_idx() {
            write!(f, "struct#{}", idx)
        } else if self.is_pyobj() {
            write!(f, "pyobj@{:x}", self.0 & PAYLOAD_MASK)
        } else if self.is_invalid() {
            write!(f, "INVALID(canary)")
        } else {
            write!(f, "???({:#x})", self.0)
        }
    }
}

impl Default for Value {
    fn default() -> Self {
        Value::NIL
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        // For floats, use proper float comparison
        if self.is_float() && other.is_float() {
            self.as_float() == other.as_float()
        } else if self.is_bigint() && other.is_bigint() {
            // Compare BigInt by value, not by pointer
            unsafe { self.as_bigint_ref().unwrap() == other.as_bigint_ref().unwrap() }
        } else {
            self.0 == other.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smallint() {
        let v = Value::from_int(42);
        assert!(v.is_int());
        assert_eq!(v.as_int(), Some(42));

        let v = Value::from_int(-42);
        assert!(v.is_int());
        assert_eq!(v.as_int(), Some(-42));

        let v = Value::from_int(0);
        assert!(v.is_int());
        assert_eq!(v.as_int(), Some(0));

        // Edge cases
        let v = Value::from_int(SMALLINT_MAX);
        assert_eq!(v.as_int(), Some(SMALLINT_MAX));

        let v = Value::from_int(SMALLINT_MIN);
        assert_eq!(v.as_int(), Some(SMALLINT_MIN));
    }

    #[test]
    fn test_float() {
        let v = Value::from_float(3.14);
        assert!(v.is_float());
        assert!((v.as_float().unwrap() - 3.14).abs() < 1e-10);

        let v = Value::from_float(-0.0);
        assert!(v.is_float());

        let v = Value::from_float(f64::INFINITY);
        assert!(v.is_float());
    }

    #[test]
    fn test_bool() {
        assert!(Value::TRUE.is_bool());
        assert!(Value::FALSE.is_bool());
        assert_eq!(Value::TRUE.as_bool(), Some(true));
        assert_eq!(Value::FALSE.as_bool(), Some(false));
    }

    #[test]
    fn test_nil() {
        assert!(Value::NIL.is_nil());
        assert!(!Value::NIL.is_truthy());
    }

    #[test]
    fn test_truthiness() {
        assert!(!Value::NIL.is_truthy());
        assert!(!Value::FALSE.is_truthy());
        assert!(Value::TRUE.is_truthy());
        assert!(!Value::from_int(0).is_truthy());
        assert!(Value::from_int(1).is_truthy());
        assert!(Value::from_int(-1).is_truthy());
        assert!(!Value::from_float(0.0).is_truthy());
        assert!(Value::from_float(0.1).is_truthy());
    }

    #[test]
    fn test_struct_tag() {
        let v = Value::from_struct_instance(42);
        assert!(v.is_struct_instance());
        assert_eq!(v.as_struct_instance_idx(), Some(42));

        // round-trip with 0
        let v0 = Value::from_struct_instance(0);
        assert_eq!(v0.as_struct_instance_idx(), Some(0));

        // round-trip with large index
        let vmax = Value::from_struct_instance(u32::MAX);
        assert_eq!(vmax.as_struct_instance_idx(), Some(u32::MAX));
    }

    #[test]
    fn test_struct_is_truthy() {
        assert!(Value::from_struct_instance(0).is_truthy());
        assert!(Value::from_struct_instance(999).is_truthy());
    }

    #[test]
    fn test_struct_not_other_types() {
        let v = Value::from_struct_instance(7);
        assert!(!v.is_int());
        assert!(!v.is_float());
        assert!(!v.is_bool());
        assert!(!v.is_nil());
        assert!(!v.is_symbol());
        assert!(!v.is_pyobj());
    }

    #[test]
    fn test_size() {
        assert_eq!(std::mem::size_of::<Value>(), 8);
    }

    // Removed: test_bigint_roundtrip — bigint_roundtrip (CatnipNanBoxProof.v)
    // Pointer round-trip; runtime covered by test_bigint_to_pyobject_huge_int_roundtrip

    // Removed: test_bigint_demote — demotion_preserves_value, needs_bigint_true_iff_not_small (CatnipNanBoxProof.v)
    // SmallInt/BigInt classification; runtime covered by test_bigint_to_pyobject_huge_int_roundtrip

    #[test]
    fn test_bigint_truthy() {
        let zero = Value::from_bigint(BigInt::from(0));
        assert!(!zero.is_truthy());
        zero.decref();

        let nonzero = Value::from_bigint(BigInt::from(999));
        assert!(nonzero.is_truthy());
        nonzero.decref();
    }

    // Removed: test_bigint_equality — encode_injective (CatnipNanBoxProof.v)
    // Value equality; runtime covered by test_bigint_to_pyobject_huge_int_roundtrip

    #[test]
    fn test_invalid_tag() {
        assert!(Value::INVALID.is_invalid());
        assert!(!Value::NIL.is_invalid());
        assert!(!Value::TRUE.is_invalid());
        assert!(!Value::from_int(0).is_invalid());
    }

    #[test]
    fn test_invalid_not_other_types() {
        let v = Value::INVALID;
        assert!(!v.is_int());
        assert!(!v.is_float());
        assert!(!v.is_bool());
        assert!(!v.is_nil());
        assert!(!v.is_symbol());
        assert!(!v.is_pyobj());
        assert!(!v.is_struct_instance());
        assert!(!v.is_bigint());
    }

    #[test]
    fn test_invalid_display() {
        let s = format!("{:?}", Value::INVALID);
        assert_eq!(s, "INVALID(canary)");
    }

    #[test]
    fn test_bigint_clone_refcount() {
        let v = Value::from_bigint(BigInt::from(42));
        v.clone_refcount(); // refcount = 2
        v.decref(); // refcount = 1
                    // v still usable
        assert!(v.is_bigint());
        v.decref(); // refcount = 0, freed
    }

    #[test]
    fn test_from_pyobject_huge_int_no_string_parse() {
        Python::attach(|py| {
            let huge = py.eval(pyo3::ffi::c_str!("(1 << 512) + 123456789"), None, None);
            let huge = huge.unwrap();
            let v = Value::from_pyobject(py, &huge).unwrap();
            assert!(v.is_bigint());
            assert_eq!(
                unsafe { v.as_bigint_ref().unwrap() },
                &((BigInt::from(1_u8) << 512) + BigInt::from(123_456_789_u64))
            );
            v.decref();
        });
    }

    #[test]
    fn test_bigint_to_pyobject_huge_int_roundtrip() {
        Python::attach(|py| {
            let n: BigInt = (BigInt::from(1_u8) << 600_u32) - BigInt::from(7_u8);
            let v = Value::from_bigint(n.clone());
            let py_obj = v.to_pyobject(py);
            let back = Value::from_pyobject(py, py_obj.bind(py)).unwrap();
            assert!(back.is_bigint());
            assert_eq!(unsafe { back.as_bigint_ref().unwrap() }, &n);
            v.decref();
            back.decref();
        });
    }
}
