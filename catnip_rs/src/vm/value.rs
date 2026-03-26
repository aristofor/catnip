// FILE: catnip_rs/src/vm/value.rs
//! NaN-boxed value representation for the Catnip VM.
//!
//! IEEE 754 quiet NaN with 47-bit payload:
//! [Sign:1][Exponent:11=0x7FF][Quiet:1][Tag:4][Payload:47]
//!   63        62-52            51     50-47    46-0
//!
//! Tags (4 bits, 16 slots):
//!
//!   Immediates (no heap allocation):
//!   0b0000 =  0  SmallInt   47-bit signed integer
//!   0b0001 =  1  Bool       0=false, 1=true
//!   0b0010 =  2  Nil        Python None
//!   0b0011 =  3  Symbol     interned string index (u32)
//!
//!   Heap references (indirection via table or pointer):
//!   0b0100 =  4  PyObject   ObjectTable handle (u32)
//!   0b0101 =  5  Struct     StructRegistry index (u32)
//!   0b0110 =  6  BigInt     Arc<GmpInt> pointer
//!   0b0111 =  7  VMFunc     FunctionTable index (u32)
//!
//!   Reserved:
//!   0b1000 =  8  (available)
//!   0b1001 =  9  (available)
//!   0b1010 = 10  (available)
//!   0b1011 = 11  (available)
//!   0b1100 = 12  (available)
//!   0b1101 = 13  (available)
//!   0b1110 = 14  (available)
//!   0b1111 = 15  (available)
//!
//! Regular floats are stored directly. Detection via quiet NaN pattern.

use super::frame::{CodeObject, NativeClosureScope, PyCodeObject, VMFunction};
use super::structs::{CatnipStructProxy, StructRegistry, cascade_decref_fields};
use catnip_core::nanbox::{
    CANON_NAN, PAYLOAD_MASK, QNAN_BASE, SMALLINT_MAX, SMALLINT_MIN, SMALLINT_SIGN_BIT, SMALLINT_SIGN_EXT, TAG_BIGINT,
    TAG_BOOL, TAG_MASK, TAG_NIL, TAG_SHIFT, TAG_SMALLINT, TAG_SYMBOL, TAG_VMFUNC,
};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt};
use rug::Integer;
use std::cell::Cell;
use std::fmt;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// GmpInt -- Sync wrapper for rug::Integer
// ---------------------------------------------------------------------------

/// Wrapper around `rug::Integer` that implements `Sync`.
///
/// `rug::Integer` is `Send` but not `Sync` (GMP limitation). Since all access
/// happens under the GIL (single-threaded), sharing via `Arc` is safe.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct GmpInt(pub Integer);

// SAFETY: All access to GmpInt is serialized by the GIL. The underlying GMP
// memory is never accessed concurrently from multiple threads.
unsafe impl Sync for GmpInt {}

impl std::ops::Deref for GmpInt {
    type Target = Integer;
    #[inline]
    fn deref(&self) -> &Integer {
        &self.0
    }
}

impl std::ops::DerefMut for GmpInt {
    #[inline]
    fn deref_mut(&mut self) -> &mut Integer {
        &mut self.0
    }
}

impl fmt::Debug for GmpInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for GmpInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

thread_local! {
    static STRUCT_REGISTRY: Cell<*const StructRegistry> = const { Cell::new(std::ptr::null()) };
}

/// Global ObjectTable shared across all threads.
///
/// Unlike StructRegistry (per-VM, thread-local), Python objects are global to
/// the interpreter. Handles must remain valid when Values cross thread
/// boundaries (e.g. ND recursion workers).  Under the GIL the Mutex is never
/// contended, so the lock is a single uncontended CAS (essentially free).
static OBJECT_TABLE: Mutex<ObjectTable> = Mutex::new(ObjectTable::new_const());

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

/// Incref a struct instance via the thread-local registry.
/// No-op if registry is not installed.
pub fn struct_registry_incref(idx: u32) {
    STRUCT_REGISTRY.with(|cell| {
        let ptr = cell.get();
        if !ptr.is_null() {
            let registry = unsafe { &*ptr };
            registry.incref(idx);
        }
    });
}

/// Decref a struct instance via the thread-local registry, cascade-freeing fields if needed.
/// No-op if registry is not installed.
pub fn struct_registry_release(idx: u32) {
    STRUCT_REGISTRY.with(|cell| {
        let ptr = cell.get();
        if !ptr.is_null() {
            // Safe: original pointer comes from &mut self.struct_registry in execute()
            let registry = unsafe { &mut *(ptr as *mut StructRegistry) };
            if let Some(fields) = registry.decref(idx) {
                cascade_decref_fields(registry, fields);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// ObjectTable -- indirection layer for TAG_PYOBJ handles
// ---------------------------------------------------------------------------

struct ObjSlot {
    obj: Py<PyAny>,
    refcount: u32,
}

/// Table mapping u32 handles to `Py<PyAny>` objects.
/// Replaces raw `*mut ffi::PyObject` pointers in NaN-boxed payloads with
/// safe, opaque indices -- same pattern as StructRegistry for TAG_STRUCT.
pub struct ObjectTable {
    slots: Vec<Option<ObjSlot>>,
    free_list: Vec<u32>,
}

impl ObjectTable {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
        }
    }

    /// Const constructor for static initialization.
    const fn new_const() -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
        }
    }

    /// Insert a `Py<PyAny>` and return its handle.
    pub fn insert(&mut self, obj: Py<PyAny>) -> u32 {
        if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = Some(ObjSlot { obj, refcount: 1 });
            idx
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Some(ObjSlot { obj, refcount: 1 }));
            idx
        }
    }

    /// Get a reference to the stored `Py<PyAny>`.
    #[inline]
    pub fn get(&self, idx: u32) -> &Py<PyAny> {
        &self.slots[idx as usize].as_ref().expect("ObjectTable: dead handle").obj
    }

    /// Increment the handle refcount (Value was duplicated).
    #[inline]
    pub fn clone_handle(&mut self, idx: u32) {
        let slot = self.slots[idx as usize].as_mut().expect("ObjectTable: dead handle");
        slot.refcount += 1;
    }

    /// Decrement the handle refcount; drop `Py<PyAny>` when it reaches 0.
    #[inline]
    pub fn release_handle(&mut self, idx: u32) {
        let slot = self.slots[idx as usize].as_mut().expect("ObjectTable: dead handle");
        slot.refcount -= 1;
        if slot.refcount == 0 {
            self.slots[idx as usize] = None;
            self.free_list.push(idx);
        }
    }

    /// Clone the `Py<PyAny>` (Python refcount bump) for handing to Python.
    #[inline]
    pub fn clone_ref(&self, py: Python<'_>, idx: u32) -> Py<PyAny> {
        self.get(idx).clone_ref(py)
    }
}

impl Default for ObjectTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Access the global ObjectTable mutably.
///
/// Under the GIL the Mutex is never contended; `lock()` is a single
/// uncontended CAS (essentially free on Linux/futex).
#[inline]
fn with_object_table<R>(f: impl FnOnce(&mut ObjectTable) -> R) -> R {
    f(&mut OBJECT_TABLE.lock().unwrap())
}

/// Read-only access to the global ObjectTable.
#[inline]
fn with_object_table_ref<R>(f: impl FnOnce(&ObjectTable) -> R) -> R {
    f(&OBJECT_TABLE.lock().unwrap())
}

// ---------------------------------------------------------------------------
// FunctionTable -- grow-only table for TAG_VMFUNC handles
// ---------------------------------------------------------------------------

/// A VM function slot: code + closure. Slots are never freed.
pub struct FuncSlot {
    pub code: Arc<CodeObject>,
    pub closure: Option<NativeClosureScope>,
    pub code_py: Py<PyCodeObject>,
    pub context: Option<Py<PyAny>>,
}

/// Table mapping u32 indices to VM function data.
/// Grow-only: functions are typically long-lived (defined once, called many
/// times), so slots are never freed. No refcounting needed.
pub struct FunctionTable {
    pub slots: Vec<FuncSlot>,
}

impl FunctionTable {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Insert a function and return its index.
    pub fn insert(&mut self, slot: FuncSlot) -> u32 {
        let idx = self.slots.len() as u32;
        self.slots.push(slot);
        idx
    }

    /// Get a function slot by index.
    #[inline]
    pub fn get(&self, idx: u32) -> Option<&FuncSlot> {
        self.slots.get(idx as usize)
    }
}

impl Default for FunctionTable {
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    static FUNC_TABLE: Cell<*const FunctionTable> = const { Cell::new(std::ptr::null()) };
}

/// Install a pointer to the active FunctionTable for the current thread.
pub fn set_func_table(ptr: *const FunctionTable) {
    FUNC_TABLE.with(|cell| cell.set(ptr));
}

/// Clear the thread-local FunctionTable pointer.
pub fn clear_func_table() {
    FUNC_TABLE.with(|cell| cell.set(std::ptr::null()));
}

/// Save the current FunctionTable pointer (for reentrant VM calls).
pub fn save_func_table() -> *const FunctionTable {
    FUNC_TABLE.with(|cell| cell.get())
}

/// Restore a previously saved FunctionTable pointer.
pub fn restore_func_table(ptr: *const FunctionTable) {
    FUNC_TABLE.with(|cell| cell.set(ptr));
}

// PyO3-specific tags (catnip_rs only)
const TAG_PYOBJ: u64 = 4 << TAG_SHIFT;
const TAG_STRUCT: u64 = 5 << TAG_SHIFT;

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

    /// Debug canary for uninitialized slots.
    /// Uses TAG_VMFUNC with all payload bits set (impossible u32 index).
    pub const INVALID: Value = Value(QNAN_BASE | TAG_VMFUNC | PAYLOAD_MASK);

    // --- Constructors ---

    /// Create a float value. NaN floats use a signaling NaN sentinel
    /// to avoid collision with the quiet NaN tagging scheme.
    #[inline]
    pub fn from_float(f: f64) -> Self {
        let bits = f.to_bits();
        // Any quiet NaN (bits 62-52 all 1s + quiet bit 51) would collide with
        // our tagged value space. Redirect to CANON_NAN (a signaling NaN whose
        // quiet bit is 0, so is_float() returns true).
        if (bits & 0x7FF8_0000_0000_0000) == 0x7FF8_0000_0000_0000 {
            Value(CANON_NAN)
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

    /// Safe creation from any i64. Uses SmallInt when in range, BigInt otherwise.
    #[inline]
    pub fn from_i64(i: i64) -> Self {
        Self::try_from_int(i).unwrap_or_else(|| Self::from_bigint(Integer::from(i)))
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
        if b { Self::TRUE } else { Self::FALSE }
    }

    /// Create a symbol value (interned string index).
    #[inline]
    pub fn from_symbol(idx: u32) -> Self {
        Value(QNAN_BASE | TAG_SYMBOL | (idx as u64))
    }

    /// Create a TAG_PYOBJ value from an ObjectTable handle.
    #[inline]
    pub fn from_obj_handle(handle: u32) -> Self {
        Value(QNAN_BASE | TAG_PYOBJ | (handle as u64))
    }

    /// Extract the ObjectTable handle. Returns None if not a PyObject.
    #[inline]
    pub fn as_obj_handle(self) -> Option<u32> {
        if self.is_pyobj() {
            Some((self.0 & PAYLOAD_MASK) as u32)
        } else {
            None
        }
    }

    /// Insert an owned `Py<PyAny>` into the ObjectTable and return a Value.
    #[inline]
    pub fn from_owned_pyobject(obj: Py<PyAny>) -> Self {
        let handle = with_object_table(|table| table.insert(obj));
        Self::from_obj_handle(handle)
    }

    /// Create a struct instance value from its index in the StructRegistry.
    #[inline]
    pub fn from_struct_instance(idx: u32) -> Self {
        Value(QNAN_BASE | TAG_STRUCT | (idx as u64))
    }

    /// Create a VM function value from its index in the FunctionTable.
    #[inline]
    pub fn from_vmfunc(idx: u32) -> Self {
        Value(QNAN_BASE | TAG_VMFUNC | (idx as u64))
    }

    /// Create a BigInt value from an Arc<GmpInt> pointer.
    #[inline]
    pub fn from_bigint(n: Integer) -> Self {
        let arc = Arc::new(GmpInt(n));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(
            ptr & !PAYLOAD_MASK == 0,
            "BigInt Arc pointer exceeds 47-bit address space"
        );
        Value(QNAN_BASE | TAG_BIGINT | (ptr & PAYLOAD_MASK))
    }

    /// Create a BigInt or demote to SmallInt if it fits.
    #[inline]
    pub fn from_bigint_or_demote(n: Integer) -> Self {
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

    /// Check if this is a native VM function.
    #[inline]
    pub fn is_vmfunc(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_VMFUNC)
    }

    /// Check if this is the INVALID canary (debug-only sentinel for uninitialized slots).
    #[inline]
    pub fn is_invalid(self) -> bool {
        self.0 == Self::INVALID.0
    }

    /// True if this value uses a native VM tag that would lose fidelity
    /// through a Python round-trip (Value -> PyObject -> Value).
    #[inline]
    pub fn has_native_tag(self) -> bool {
        self.is_struct_instance() || self.is_bigint() || self.is_vmfunc()
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
        if self.is_bool() { Some((self.0 & 1) != 0) } else { None }
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

    /// Retrieve the `Py<PyAny>` from the ObjectTable (clone_ref for Python).
    #[inline]
    pub fn as_pyobject(self, py: Python<'_>) -> Option<Py<PyAny>> {
        if self.is_pyobj() {
            let handle = (self.0 & PAYLOAD_MASK) as u32;
            Some(with_object_table_ref(|table| table.clone_ref(py, handle)))
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

    /// Extract VM function table index.
    #[inline]
    pub fn as_vmfunc_idx(self) -> u32 {
        debug_assert!(self.is_vmfunc() && !self.is_invalid());
        (self.0 & PAYLOAD_MASK) as u32
    }

    /// Borrow the Integer behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive (not yet decremented to 0).
    #[inline]
    pub unsafe fn as_bigint_ref(&self) -> Option<&Integer> {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
            Some(&(*ptr).0)
        } else {
            None
        }
    }

    /// Increment refcount for reference-counted values (BigInt Arc, PyObject handle, Struct instance).
    /// Must be called when duplicating a Value (e.g. DupTop, StoreScope to globals).
    #[inline]
    pub fn clone_refcount(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
            unsafe { Arc::increment_strong_count(ptr) };
        } else if self.is_pyobj() {
            let handle = (self.0 & PAYLOAD_MASK) as u32;
            with_object_table(|table| table.clone_handle(handle));
        } else if self.is_struct_instance() {
            let idx = self.as_struct_instance_idx().unwrap();
            struct_registry_incref(idx);
        }
    }

    /// Increment refcount only for BigInt values.
    /// Used in LoadLocal to track shared references without affecting PyObject refcounting.
    #[inline]
    pub fn clone_refcount_bigint(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
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
            unsafe { self.as_bigint_ref().unwrap().cmp0() != std::cmp::Ordering::Equal }
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
            unsafe { self.as_bigint_ref().unwrap().cmp0() != std::cmp::Ordering::Equal }
        } else if self.is_struct_instance() || self.is_vmfunc() {
            true
        } else {
            // PyObject - delegate to Python's __bool__()
            let obj = self.to_pyobject(py);
            obj.bind(py).is_truthy().unwrap_or(true)
        }
    }

    /// Python `is` identity test.
    ///
    /// Immediate types (int, bool, nil, struct) compare by bits.
    /// PyObjects compare by Python object identity (`a is b`).
    #[inline]
    pub fn is_identical(self, py: pyo3::Python<'_>, other: Value) -> bool {
        // Fast path: identical bits = identical value
        if self.0 == other.0 {
            return true;
        }
        // PyObject identity: different handles may alias the same Python object
        if self.is_pyobj() && other.is_pyobj() {
            let a = self.to_pyobject(py);
            let b = other.to_pyobject(py);
            return a.bind(py).is(b.bind(py));
        }
        false
    }

    /// Raw bits for debugging.
    #[inline]
    pub fn bits(self) -> u64 {
        self.0
    }
}

// --- PyO3 conversions ---

/// Convert a rug::Integer to a Python int via GMP digit export.
fn integer_to_pyobject(py: Python<'_>, n: &Integer) -> Py<PyAny> {
    if let Some(i) = n.to_i64() {
        return i.into_pyobject(py).unwrap().unbind().into_any();
    }
    // Export absolute value as little-endian bytes
    let is_neg = n.cmp0() == std::cmp::Ordering::Less;
    let abs_n;
    let src = if is_neg {
        abs_n = Integer::from(-n);
        &abs_n
    } else {
        n
    };
    let digits = src.to_digits::<u8>(rug::integer::Order::Lsf);
    let bytes = pyo3::types::PyBytes::new(py, &digits);
    let int_type = py.get_type::<PyInt>();
    let py_int = int_type
        .call_method("from_bytes", (bytes, "little"), None)
        .expect("int.from_bytes failed");
    if is_neg {
        let neg = py_int.neg().expect("negation failed");
        neg.unbind()
    } else {
        py_int.unbind()
    }
}

/// Convert a Python int to a rug::Integer via bytes.
fn pyobject_to_integer(obj: &Bound<'_, PyAny>) -> PyResult<Integer> {
    // Fast path: fits in i64
    if let Ok(val) = obj.extract::<i64>() {
        return Ok(Integer::from(val));
    }
    // Slow path: big int via bytes (unsigned abs + separate sign)
    let py = obj.py();
    let bit_length: usize = obj.call_method0("bit_length")?.extract()?;
    let byte_length = bit_length.div_ceil(8);
    // Get absolute value bytes (unsigned, little-endian)
    let abs_obj = obj.call_method0("__abs__")?;
    let kwargs = pyo3::types::PyDict::new(py);
    kwargs.set_item("signed", false)?;
    let bytes_obj = abs_obj.call_method("to_bytes", (byte_length, "little"), Some(&kwargs))?;
    let bytes: &[u8] = bytes_obj.extract()?;
    let mut result = Integer::from_digits(bytes, rug::integer::Order::Lsf);
    // Restore sign
    let is_neg = obj.lt(0)?;
    if is_neg {
        result = -result;
    }
    Ok(result)
}

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

        if let Ok(_i) = obj.cast::<PyInt>() {
            // Try to extract as i64
            if let Ok(val) = obj.extract::<i64>() {
                if let Some(v) = Value::try_from_int(val) {
                    return Ok(v);
                }
                // Fits i64 but not SmallInt -> BigInt
                return Ok(Value::from_bigint(Integer::from(val)));
            }
            // Overflow i64 -> extract via bytes
            if let Ok(n) = pyobject_to_integer(obj) {
                return Ok(Value::from_bigint_or_demote(n));
            }
        }

        if let Ok(f) = obj.cast::<PyFloat>() {
            let val: f64 = f.extract()?;
            return Ok(Value::from_float(val));
        }

        // Recognize VMFunction -> restore native TAG_VMFUNC for VM round-trip.
        if let Ok(vmfunc) = obj.cast::<VMFunction>() {
            if let Some(idx) = vmfunc.borrow().func_table_idx {
                let is_owned = FUNC_TABLE.with(|cell| {
                    let ptr = cell.get();
                    if ptr.is_null() {
                        return false;
                    }
                    let table = unsafe { &*ptr };
                    (idx as usize) < table.slots.len()
                });
                if is_owned {
                    return Ok(Value::from_vmfunc(idx));
                }
            }
        }

        // Recognize CatnipStructProxy -> restore native TAG_STRUCT for VM round-trip.
        if let Ok(proxy) = obj.cast::<CatnipStructProxy>() {
            if let Some(idx) = proxy.borrow().native_instance_idx {
                let restored = STRUCT_REGISTRY.with(|cell| {
                    let ptr = cell.get();
                    if ptr.is_null() {
                        return false;
                    }
                    let registry = unsafe { &*ptr };
                    if registry.get_instance(idx).is_some() {
                        registry.incref(idx);
                        true
                    } else {
                        false
                    }
                });
                if restored {
                    return Ok(Value::from_struct_instance(idx));
                }
            }
        }

        // Store as handle in ObjectTable
        Ok(Value::from_owned_pyobject(obj.clone().unbind()))
    }

    /// Convert a Value back to a Python object.
    pub fn to_pyobject(self, py: Python<'_>) -> Py<PyAny> {
        debug_assert!(
            !self.is_invalid(),
            "to_pyobject called on INVALID canary value - uninitialized slot or stack corruption"
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
            let n = unsafe { self.as_bigint_ref().unwrap() };
            integer_to_pyobject(py, n)
        } else if self.is_struct_instance() {
            let idx = self.as_struct_instance_idx().unwrap();
            STRUCT_REGISTRY.with(|cell| {
                let ptr = cell.get();
                if ptr.is_null() {
                    return py.None();
                }
                let registry = unsafe { &*ptr };
                registry.instance_to_pyobject(py, idx).unwrap_or_else(|_| py.None())
            })
        } else if self.is_pyobj() {
            let handle = (self.0 & PAYLOAD_MASK) as u32;
            with_object_table_ref(|table| table.clone_ref(py, handle))
        } else if self.is_vmfunc() {
            let idx = (self.0 & PAYLOAD_MASK) as u32;
            FUNC_TABLE.with(|cell| {
                let ptr = cell.get();
                if ptr.is_null() {
                    return py.None();
                }
                let table = unsafe { &*ptr };
                match table.get(idx) {
                    Some(slot) => {
                        let mut func = VMFunction::create_native(
                            py,
                            slot.code_py.clone_ref(py),
                            slot.closure.clone(),
                            slot.context.as_ref().map(|c| c.clone_ref(py)),
                        );
                        func.func_table_idx = Some(idx);
                        Py::new(py, func).unwrap().into_any()
                    }
                    None => py.None(),
                }
            })
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
            let handle = (self.0 & PAYLOAD_MASK) as u32;
            with_object_table(|table| table.release_handle(handle));
        } else if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
            unsafe { Arc::decrement_strong_count(ptr) };
        }
    }

    /// Decrement refcount only for BigInt values.
    /// Used in StoreLocal/PopTop to free intermediate BigInts without
    /// affecting PyObject refcounting (which is managed by Python GC).
    #[inline]
    pub fn decref_bigint(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
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
            write!(f, "pyobj#{}", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_invalid() {
            write!(f, "INVALID(canary)")
        } else if self.is_vmfunc() {
            write!(f, "vmfunc#{}", (self.0 & PAYLOAD_MASK) as u32)
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
        let expected = std::f64::consts::PI;
        let v = Value::from_float(expected);
        assert!(v.is_float());
        assert!((v.as_float().unwrap() - expected).abs() < 1e-10);

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
    fn test_nan_not_zero() {
        // Regression: f64::NAN.to_bits() == QNAN_BASE == SmallInt(0).
        // from_float(NaN) must produce a value that is_float() and is_nan(),
        // not SmallInt(0).
        let v = Value::from_float(f64::NAN);
        assert!(v.is_float(), "NaN must be detected as float, not tagged value");
        assert!(!v.is_int(), "NaN must not be detected as int");
        let f = v.as_float().unwrap();
        assert!(f.is_nan(), "NaN round-trip must preserve NaN");
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

    #[test]
    fn test_bigint_truthy() {
        let zero = Value::from_bigint(Integer::from(0));
        assert!(!zero.is_truthy());
        zero.decref();

        let nonzero = Value::from_bigint(Integer::from(999));
        assert!(nonzero.is_truthy());
        nonzero.decref();
    }

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
        let v = Value::from_bigint(Integer::from(42));
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
            let expected = (Integer::from(1_u8) << 512_u32) + Integer::from(123_456_789_u64);
            assert_eq!(unsafe { v.as_bigint_ref().unwrap() }, &expected);
            v.decref();
        });
    }

    #[test]
    fn test_bigint_to_pyobject_huge_int_roundtrip() {
        Python::attach(|py| {
            let n: Integer = (Integer::from(1_u8) << 600_u32) - Integer::from(7_u8);
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
