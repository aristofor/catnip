// FILE: catnip_vm/src/value.rs
//! NaN-boxed value representation for the pure-Rust Catnip VM.
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
//!   0b0010 =  2  Nil        None
//!   0b0011 =  3  Symbol     interned string index (u32)
//!
//!   Heap references:
//!   0b0100 =  4  (reserved for PyObject in catnip_rs)
//!   0b0101 =  5  Struct    instance index in PureStructRegistry (u32)
//!   0b0110 =  6  BigInt     Arc<GmpInt> pointer
//!   0b0111 =  7  VMFunc     FunctionTable index (u32)
//!   0b1000 =  8  NativeStr  Arc<str> pointer
//!
//!   0b1001 =  9  NativeList  Arc<NativeList> pointer
//!   0b1010 = 10  NativeDict  Arc<NativeDict> pointer
//!   0b1011 = 11  NativeTuple Arc<NativeTuple> pointer
//!   0b1100 = 12  NativeSet   Arc<NativeSet> pointer
//!   0b1101 = 13  NativeBytes Arc<NativeBytes> pointer
//!
//!   0b1110 = 14  StructType callable struct type (type_id as u32)
//!
//!   Extended (future-proof overflow tag):
//!   0b1111 = 15  Extended    Arc<ExtendedValue> (Module, future: Enum, ...)
//!
//! Regular floats are stored directly. Detection via quiet NaN pattern.

use crate::collections::{NativeBytes, NativeDict, NativeList, NativeSet, NativeTuple};
use catnip_core::nanbox::{
    CANON_NAN, PAYLOAD_MASK, QNAN_BASE, SMALLINT_MAX, SMALLINT_MIN, SMALLINT_SIGN_BIT, SMALLINT_SIGN_EXT, TAG_BIGINT,
    TAG_BOOL, TAG_MASK, TAG_NIL, TAG_SMALLINT, TAG_SYMBOL, TAG_VMFUNC,
};
use indexmap::IndexMap;
use rug::Integer;
use std::fmt;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// GmpInt -- Sync wrapper for rug::Integer
// ---------------------------------------------------------------------------

/// Wrapper around `rug::Integer` that implements `Sync`.
///
/// `rug::Integer` is `Send` but not `Sync` (GMP limitation). In the pure-Rust
/// VM context, the VM is single-threaded so sharing via `Arc` is safe.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct GmpInt(pub Integer);

// SAFETY: In catnip_vm the VM is single-threaded. The Arc is used for
// refcounted sharing within one thread, never across threads.
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

// Struct tags (catnip_vm only)
use catnip_core::nanbox::TAG_SHIFT;
pub(crate) const TAG_STRUCT: u64 = 5 << TAG_SHIFT;
pub(crate) const TAG_STRUCTTYPE: u64 = 14 << TAG_SHIFT;

// Native collection tags (catnip_vm only, 8-13)
pub(crate) const TAG_NATIVESTR: u64 = 8 << TAG_SHIFT;
pub(crate) const TAG_NATIVELIST: u64 = 9 << TAG_SHIFT;
pub(crate) const TAG_NATIVEDICT: u64 = 10 << TAG_SHIFT;
pub(crate) const TAG_NATIVETUPLE: u64 = 11 << TAG_SHIFT;
pub(crate) const TAG_NATIVESET: u64 = 12 << TAG_SHIFT;
pub(crate) const TAG_NATIVEBYTES: u64 = 13 << TAG_SHIFT;
pub(crate) const TAG_EXTENDED: u64 = 15 << TAG_SHIFT;

// ---------------------------------------------------------------------------
// Extended values (tag 15) -- future-proof overflow tag
// ---------------------------------------------------------------------------

/// Module namespace: maps exported attribute names to values.
pub struct ModuleNamespace {
    pub name: String,
    pub attrs: IndexMap<String, Value>,
    /// Child pipeline's globals Rc, kept alive so closure scopes
    /// in exported functions can still resolve module-level names.
    pub module_globals: crate::host::Globals,
}

// ---------------------------------------------------------------------------
// NativeFile -- file handle for io.open in pure mode
// ---------------------------------------------------------------------------

use std::cell::RefCell;

// ---------------------------------------------------------------------------
// NativeMeta -- META object for module export declarations
// ---------------------------------------------------------------------------

/// META namespace: attribute bag for module metadata (file, exports, etc.).
pub struct NativeMeta {
    pub attrs: RefCell<IndexMap<String, Value>>,
}

impl Default for NativeMeta {
    fn default() -> Self {
        Self {
            attrs: RefCell::new(IndexMap::new()),
        }
    }
}

impl NativeMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, name: &str, val: Value) {
        self.attrs.borrow_mut().insert(name.to_string(), val);
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.attrs.borrow().get(name).copied()
    }
}

/// Extended value types sharing tag 15.
/// Single tag slot, unlimited future variants.
pub enum ExtendedValue {
    Module(ModuleNamespace),
    /// Enum type marker (type_id into EnumRegistry).
    EnumType(u32),
    /// META object for module metadata.
    Meta(NativeMeta),
    /// Opaque plugin object with method/attr/drop callbacks.
    PluginObject {
        handle: u64,
        callbacks: crate::plugin::PluginObjectCallbacks,
    },
    /// Native complex number (real, imag).
    Complex(f64, f64),
}

impl Drop for ExtendedValue {
    fn drop(&mut self) {
        if let ExtendedValue::PluginObject { handle, callbacks } = self {
            if let Some(drop_fn) = callbacks.drop {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                    drop_fn(*handle);
                }));
            }
        }
    }
}

// SAFETY: PureVM is single-threaded. ExtendedValue may contain Rc (via
// module_globals) which is !Send, but we never share across threads.
unsafe impl Send for ExtendedValue {}
unsafe impl Sync for ExtendedValue {}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

/// NaN-boxed value. Fits in 8 bytes. Pure Rust -- no PyO3 dependency.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Value(u64);

impl Value {
    /// Nil constant (None)
    pub const NIL: Value = Value(QNAN_BASE | TAG_NIL);

    /// Boolean true
    pub const TRUE: Value = Value(QNAN_BASE | TAG_BOOL | 1);

    /// Boolean false
    pub const FALSE: Value = Value(QNAN_BASE | TAG_BOOL);

    /// Debug canary for uninitialized slots.
    pub const INVALID: Value = Value(QNAN_BASE | TAG_VMFUNC | PAYLOAD_MASK);

    // --- Constructors ---

    /// Create a float value. NaN floats use a signaling NaN sentinel
    /// to avoid collision with the quiet NaN tagging scheme.
    #[inline]
    pub fn from_float(f: f64) -> Self {
        let bits = f.to_bits();
        if (bits & 0x7FF8_0000_0000_0000) == 0x7FF8_0000_0000_0000 {
            Value(CANON_NAN)
        } else {
            Value(bits)
        }
    }

    /// Create a small integer value. Panics if out of 47-bit range.
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
        if b { Self::TRUE } else { Self::FALSE }
    }

    /// Create a symbol value (interned string index).
    #[inline]
    pub fn from_symbol(idx: u32) -> Self {
        Value(QNAN_BASE | TAG_SYMBOL | (idx as u64))
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

    /// Create an integer value from any i64 (SmallInt if it fits, BigInt otherwise).
    #[inline]
    pub fn from_i64(i: i64) -> Self {
        Self::try_from_int(i).unwrap_or_else(|| Self::from_bigint(Integer::from(i)))
    }

    /// Create a VM function value from its index in the FunctionTable.
    #[inline]
    pub fn from_vmfunc(idx: u32) -> Self {
        Value(QNAN_BASE | TAG_VMFUNC | (idx as u64))
    }

    /// Create a struct instance value from its index in PureStructRegistry.
    #[inline]
    pub fn from_struct_instance(idx: u32) -> Self {
        Value(QNAN_BASE | TAG_STRUCT | (idx as u64))
    }

    /// Create a callable struct type value from its type_id.
    #[inline]
    pub fn from_struct_type(type_id: u32) -> Self {
        Value(QNAN_BASE | TAG_STRUCTTYPE | (type_id as u64))
    }

    /// Create an Extended value (module, future: enum, union, ...).
    #[inline]
    pub fn from_extended(ext: ExtendedValue) -> Self {
        let arc = Arc::new(ext);
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(
            ptr & !PAYLOAD_MASK == 0,
            "Extended Arc pointer exceeds 47-bit address space"
        );
        Value(QNAN_BASE | TAG_EXTENDED | (ptr & PAYLOAD_MASK))
    }

    /// Convenience: create a Module value.
    #[inline]
    pub fn from_module(ns: ModuleNamespace) -> Self {
        Self::from_extended(ExtendedValue::Module(ns))
    }

    /// Create an enum type marker value.
    #[inline]
    pub fn from_enum_type(type_id: u32) -> Self {
        Self::from_extended(ExtendedValue::EnumType(type_id))
    }

    /// Create a META object value.
    #[inline]
    pub fn from_meta(m: NativeMeta) -> Self {
        Self::from_extended(ExtendedValue::Meta(m))
    }

    /// Create a plugin object value from an opaque handle and callbacks.
    #[inline]
    pub fn from_plugin_object(handle: u64, callbacks: crate::plugin::PluginObjectCallbacks) -> Self {
        Self::from_extended(ExtendedValue::PluginObject { handle, callbacks })
    }

    /// Create a complex number value.
    #[inline]
    pub fn from_complex(real: f64, imag: f64) -> Self {
        Self::from_extended(ExtendedValue::Complex(real, imag))
    }

    // NativeStr constructors are defined in the second impl block below,
    // after the NativeString type definition.

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

    /// Check if this is a native string.
    #[inline]
    pub fn is_native_str(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_NATIVESTR)
    }

    /// Check if this is a native list.
    #[inline]
    pub fn is_native_list(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_NATIVELIST)
    }

    /// Check if this is a native dict.
    #[inline]
    pub fn is_native_dict(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_NATIVEDICT)
    }

    /// Check if this is a native tuple.
    #[inline]
    pub fn is_native_tuple(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_NATIVETUPLE)
    }

    /// Check if this is a native set.
    #[inline]
    pub fn is_native_set(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_NATIVESET)
    }

    /// Check if this is a native bytes.
    #[inline]
    pub fn is_native_bytes(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_NATIVEBYTES)
    }

    /// Check if this is a struct instance.
    #[inline]
    pub fn is_struct_instance(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_STRUCT)
    }

    /// Check if this is a callable struct type.
    #[inline]
    pub fn is_struct_type(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_STRUCTTYPE)
    }

    /// Check if this is an Extended value (tag 15).
    #[inline]
    pub fn is_extended(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_EXTENDED)
    }

    /// Check if this is the INVALID canary.
    #[inline]
    pub fn is_invalid(self) -> bool {
        self.0 == Self::INVALID.0
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

    /// Extract VM function table index.
    #[inline]
    pub fn as_vmfunc_idx(self) -> u32 {
        debug_assert!(self.is_vmfunc() && !self.is_invalid());
        (self.0 & PAYLOAD_MASK) as u32
    }

    /// Extract struct instance index from PureStructRegistry.
    #[inline]
    pub fn as_struct_instance_idx(self) -> Option<u32> {
        if self.is_struct_instance() {
            Some((self.0 & PAYLOAD_MASK) as u32)
        } else {
            None
        }
    }

    /// Extract struct type id.
    #[inline]
    pub fn as_struct_type_id(self) -> Option<u32> {
        if self.is_struct_type() {
            Some((self.0 & PAYLOAD_MASK) as u32)
        } else {
            None
        }
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

    /// Borrow the str behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive (not yet decremented to 0).
    #[inline]
    pub unsafe fn as_native_str_ref(&self) -> Option<&str> {
        if self.is_native_str() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const u8;
            // Reconstruct Arc<str> metadata: the fat pointer is stored as
            // a thin pointer to the Arc's data. Arc<str> stores
            // (strong, weak, len, data...) so we need the full Arc.
            // Instead, we clone the Arc to get a reference.
            //
            // Actually, Arc<str> is a fat pointer (data ptr + len).
            // We can't store a fat pointer in 47 bits.
            // We need to use Arc<String> instead (thin pointer).
            //
            // This is handled by storing Arc<NativeString> (thin pointer).
            let ns = &*(ptr as *const NativeString);
            Some(ns.as_str())
        } else {
            None
        }
    }

    /// Borrow the NativeList behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_native_list_ref(&self) -> Option<&NativeList> {
        if self.is_native_list() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeList;
            Some(&*ptr)
        } else {
            None
        }
    }

    /// Borrow the NativeDict behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_native_dict_ref(&self) -> Option<&NativeDict> {
        if self.is_native_dict() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeDict;
            Some(&*ptr)
        } else {
            None
        }
    }

    /// Borrow the NativeTuple behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_native_tuple_ref(&self) -> Option<&NativeTuple> {
        if self.is_native_tuple() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeTuple;
            Some(&*ptr)
        } else {
            None
        }
    }

    /// Borrow the NativeSet behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_native_set_ref(&self) -> Option<&NativeSet> {
        if self.is_native_set() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeSet;
            Some(&*ptr)
        } else {
            None
        }
    }

    /// Borrow the NativeBytes behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_native_bytes_ref(&self) -> Option<&NativeBytes> {
        if self.is_native_bytes() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeBytes;
            Some(&*ptr)
        } else {
            None
        }
    }

    /// Borrow the ExtendedValue behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_extended_ref(&self) -> Option<&ExtendedValue> {
        if self.is_extended() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const ExtendedValue;
            Some(&*ptr)
        } else {
            None
        }
    }

    /// Borrow the ModuleNamespace behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive and the value is a Module.
    #[inline]
    pub unsafe fn as_module_ref(&self) -> Option<&ModuleNamespace> {
        match self.as_extended_ref()? {
            ExtendedValue::Module(ns) => Some(ns),
            _ => None,
        }
    }

    /// Check if this is a module namespace.
    #[inline]
    pub fn is_module(self) -> bool {
        if !self.is_extended() {
            return false;
        }
        matches!(unsafe { self.as_extended_ref() }, Some(ExtendedValue::Module(_)))
    }

    /// Check if this is an enum type marker.
    #[inline]
    pub fn is_enum_type(self) -> bool {
        if !self.is_extended() {
            return false;
        }
        matches!(unsafe { self.as_extended_ref() }, Some(ExtendedValue::EnumType(_)))
    }

    /// Extract enum type_id if this is an enum type marker.
    #[inline]
    pub fn as_enum_type_id(self) -> Option<u32> {
        if !self.is_extended() {
            return None;
        }
        match unsafe { self.as_extended_ref()? } {
            ExtendedValue::EnumType(id) => Some(*id),
            _ => None,
        }
    }

    /// Check if this is a META object.
    #[inline]
    pub fn is_meta(self) -> bool {
        if !self.is_extended() {
            return false;
        }
        matches!(unsafe { self.as_extended_ref() }, Some(ExtendedValue::Meta(_)))
    }

    /// Borrow the NativeMeta behind the Arc.
    ///
    /// # Safety
    /// Caller must ensure the Arc is still alive and the value is a Meta.
    #[inline]
    pub unsafe fn as_meta_ref(&self) -> Option<&NativeMeta> {
        match self.as_extended_ref()? {
            ExtendedValue::Meta(m) => Some(m),
            _ => None,
        }
    }

    /// Check if this is a plugin object.
    #[inline]
    pub fn is_plugin_object(self) -> bool {
        if !self.is_extended() {
            return false;
        }
        matches!(
            unsafe { self.as_extended_ref() },
            Some(ExtendedValue::PluginObject { .. })
        )
    }

    /// Borrow the plugin object handle and callbacks.
    ///
    /// # Safety
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_plugin_object_ref(&self) -> Option<(u64, crate::plugin::PluginObjectCallbacks)> {
        match self.as_extended_ref()? {
            ExtendedValue::PluginObject { handle, callbacks } => Some((*handle, callbacks.clone())),
            _ => None,
        }
    }

    /// Check if this is a complex number.
    #[inline]
    pub fn is_complex(self) -> bool {
        if !self.is_extended() {
            return false;
        }
        matches!(unsafe { self.as_extended_ref() }, Some(ExtendedValue::Complex(_, _)))
    }

    /// Extract (real, imag) parts from a complex value.
    ///
    /// # Safety
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_complex_parts(&self) -> Option<(f64, f64)> {
        match self.as_extended_ref()? {
            ExtendedValue::Complex(r, i) => Some((*r, *i)),
            _ => None,
        }
    }

    // --- Refcount management ---

    /// Increment refcount for heap-allocated values.
    #[inline]
    pub fn clone_refcount(self) {
        if self.is_float() {
            return;
        }
        let tag = self.0 & TAG_MASK;
        match tag {
            TAG_BIGINT => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const GmpInt) },
            TAG_NATIVESTR => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeString) },
            TAG_NATIVELIST => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeList) },
            TAG_NATIVEDICT => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeDict) },
            TAG_NATIVETUPLE => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeTuple) },
            TAG_NATIVESET => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeSet) },
            TAG_NATIVEBYTES => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeBytes) },
            TAG_EXTENDED => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const ExtendedValue) },
            _ => {}
        }
    }

    /// Decrement refcount. Call when value is no longer needed.
    pub fn decref(self) {
        if self.is_float() {
            return;
        }
        let tag = self.0 & TAG_MASK;
        match tag {
            TAG_BIGINT => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const GmpInt) },
            TAG_NATIVESTR => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeString) },
            TAG_NATIVELIST => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeList) },
            TAG_NATIVEDICT => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeDict) },
            TAG_NATIVETUPLE => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeTuple) },
            TAG_NATIVESET => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeSet) },
            TAG_NATIVEBYTES => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeBytes) },
            TAG_EXTENDED => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const ExtendedValue) },
            _ => {}
        }
    }

    /// Decrement refcount only for BigInt values.
    #[inline]
    pub fn decref_bigint(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
            unsafe { Arc::decrement_strong_count(ptr) };
        }
    }

    // --- Truthiness ---

    /// Check if value is truthy.
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
            unsafe { self.as_bigint_ref().unwrap().cmp0() != std::cmp::Ordering::Equal }
        } else if self.is_native_str() {
            unsafe { !self.as_native_str_ref().unwrap().is_empty() }
        } else if self.is_native_list() {
            unsafe { !self.as_native_list_ref().unwrap().is_empty() }
        } else if self.is_native_dict() {
            unsafe { !self.as_native_dict_ref().unwrap().is_empty() }
        } else if self.is_native_tuple() {
            unsafe { !self.as_native_tuple_ref().unwrap().is_empty() }
        } else if self.is_native_set() {
            unsafe { !self.as_native_set_ref().unwrap().is_empty() }
        } else if self.is_native_bytes() {
            unsafe { !self.as_native_bytes_ref().unwrap().is_empty() }
        } else if self.is_complex() {
            let (r, i) = unsafe { self.as_complex_parts().unwrap() };
            r != 0.0 || i != 0.0
        } else {
            // Struct instances, struct types, extended values, and
            // any other non-nil heap object are truthy.
            true
        }
    }

    /// Raw bits for debugging.
    #[inline]
    pub fn bits(self) -> u64 {
        self.0
    }

    /// Convert Value to a displayable string (pure Rust).
    pub fn display_string(&self) -> String {
        if self.is_nil() {
            "None".to_string()
        } else if let Some(b) = self.as_bool() {
            if b { "True".to_string() } else { "False".to_string() }
        } else if let Some(i) = self.as_int() {
            i.to_string()
        } else if let Some(f) = self.as_float() {
            format_float(f)
        } else if self.is_bigint() {
            let n = unsafe { self.as_bigint_ref().unwrap() };
            n.to_string()
        } else if self.is_native_str() {
            unsafe { self.as_native_str_ref().unwrap().to_string() }
        } else if self.is_native_list() {
            let list = unsafe { self.as_native_list_ref().unwrap() };
            let items = list.as_slice_cloned();
            let parts: Vec<String> = items
                .iter()
                .map(|v| {
                    let s = v.repr_string();
                    v.decref();
                    s
                })
                .collect();
            format!("[{}]", parts.join(", "))
        } else if self.is_native_tuple() {
            let tuple = unsafe { self.as_native_tuple_ref().unwrap() };
            let items = tuple.as_slice();
            let parts: Vec<String> = items.iter().map(|v| v.repr_string()).collect();
            if items.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            }
        } else if self.is_native_dict() {
            let dict = unsafe { self.as_native_dict_ref().unwrap() };
            let keys = dict.keys_cloned();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    let kv = k.to_value();
                    let vv = dict.get_item(k).unwrap_or(Value::NIL);
                    let s = format!("{}: {}", kv.repr_string(), vv.repr_string());
                    kv.decref();
                    vv.decref();
                    s
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        } else if self.is_native_set() {
            let set = unsafe { self.as_native_set_ref().unwrap() };
            let vals = set.to_values();
            if vals.is_empty() {
                "set()".to_string()
            } else {
                let parts: Vec<String> = vals
                    .iter()
                    .map(|v| {
                        let s = v.repr_string();
                        v.decref();
                        s
                    })
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
        } else if self.is_native_bytes() {
            let bytes = unsafe { self.as_native_bytes_ref().unwrap() };
            format!(
                "b'{}'",
                bytes
                    .as_bytes()
                    .iter()
                    .map(|&b| {
                        if b.is_ascii_graphic() || b == b' ' {
                            (b as char).to_string()
                        } else {
                            format!("\\x{:02x}", b)
                        }
                    })
                    .collect::<String>()
            )
        } else if self.is_vmfunc() {
            format!("<function #{}>", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_struct_instance() {
            // Detailed display requires registry access -- handled in VM dispatch
            format!("<struct #{}>", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_struct_type() {
            format!("<type #{}>", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_module() {
            let ns = unsafe { self.as_module_ref().unwrap() };
            format!("<module '{}'>", ns.name)
        } else if self.is_meta() {
            "<META>".to_string()
        } else if self.is_complex() {
            let (r, i) = unsafe { self.as_complex_parts().unwrap() };
            format_complex(r, i)
        } else if self.is_plugin_object() {
            let (handle, _) = unsafe { self.as_plugin_object_ref().unwrap() };
            format!("<plugin object {:#x}>", handle)
        } else if self.is_extended() {
            "<extended>".to_string()
        } else {
            format!("???({:#x})", self.0)
        }
    }

    /// Convert Value to its repr string (with quotes for strings).
    pub fn repr_string(&self) -> String {
        if self.is_native_str() {
            let s = unsafe { self.as_native_str_ref().unwrap() };
            format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
        } else {
            self.display_string()
        }
    }
}

/// Format a float like Python: no trailing .0 for ints that happen to be floats,
/// but always show decimal point.
fn format_float(f: f64) -> String {
    if f.is_infinite() {
        if f > 0.0 { "inf".to_string() } else { "-inf".to_string() }
    } else if f.is_nan() {
        "nan".to_string()
    } else if f == f.trunc() && f.abs() < 1e16 {
        format!("{:.1}", f)
    } else {
        format!("{}", f)
    }
}

/// Format a complex number like Python:
/// - real == 0: "{imag}j"
/// - real != 0: "({real}+{imag}j)" or "({real}-{imag}j)"
fn format_complex(r: f64, i: f64) -> String {
    let imag_str = format_float(i.abs());
    if r == 0.0 && !r.is_sign_negative() {
        // Pure imaginary
        if i < 0.0 || (i == 0.0 && i.is_sign_negative()) {
            format!("-{}j", imag_str)
        } else {
            format!("{}j", imag_str)
        }
    } else {
        let real_str = format_float(r);
        if i < 0.0 || (i == 0.0 && i.is_sign_negative()) {
            format!("({}-{}j)", real_str, imag_str)
        } else {
            format!("({}+{}j)", real_str, imag_str)
        }
    }
}

// ---------------------------------------------------------------------------
// NativeString -- thin-pointer wrapper for Arc storage
// ---------------------------------------------------------------------------

/// Thin-pointer string wrapper for NaN-box storage.
///
/// `Arc<str>` is a fat pointer (ptr + len) which can't fit in 47 bits.
/// `Arc<NativeString>` is a thin pointer to a sized struct.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeString(String);

impl NativeString {
    #[inline]
    pub fn new(s: String) -> Self {
        NativeString(s)
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for NativeString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\"", self.0)
    }
}

impl fmt::Display for NativeString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// --- Rewrite from_native_str / from_arc_str to use NativeString (thin pointer) ---

impl Value {
    /// Create a NativeStr value from a Rust &str.
    ///
    /// Stores `Arc<NativeString>` (thin pointer) in 47-bit payload.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Self::from_string(s.to_string())
    }

    /// Create a NativeStr value from an owned String.
    #[inline]
    pub fn from_string(s: String) -> Self {
        let arc = Arc::new(NativeString(s));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(
            ptr & !PAYLOAD_MASK == 0,
            "NativeStr Arc pointer exceeds 47-bit address space"
        );
        Value(QNAN_BASE | TAG_NATIVESTR | (ptr & PAYLOAD_MASK))
    }
}

// --- Collection constructors ---

impl Value {
    /// Create a NativeList value from a Vec of Values.
    /// Takes ownership of refcounts -- caller should NOT decref items.
    #[inline]
    #[allow(clippy::arc_with_non_send_sync)] // VM is single-threaded, Arc for refcounting only
    pub fn from_list(items: Vec<Value>) -> Self {
        let arc = Arc::new(NativeList::new(items));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(ptr & !PAYLOAD_MASK == 0, "NativeList pointer exceeds 47-bit");
        Value(QNAN_BASE | TAG_NATIVELIST | (ptr & PAYLOAD_MASK))
    }

    /// Create a NativeDict value from an IndexMap.
    #[inline]
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn from_dict(items: indexmap::IndexMap<crate::collections::ValueKey, Value>) -> Self {
        let arc = Arc::new(NativeDict::new(items));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(ptr & !PAYLOAD_MASK == 0, "NativeDict pointer exceeds 47-bit");
        Value(QNAN_BASE | TAG_NATIVEDICT | (ptr & PAYLOAD_MASK))
    }

    /// Create an empty NativeDict.
    #[inline]
    pub fn from_empty_dict() -> Self {
        Self::from_dict(indexmap::IndexMap::new())
    }

    /// Create a NativeTuple value from a Vec of Values.
    #[inline]
    pub fn from_tuple(items: Vec<Value>) -> Self {
        let arc = Arc::new(NativeTuple::new(items));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(ptr & !PAYLOAD_MASK == 0, "NativeTuple pointer exceeds 47-bit");
        Value(QNAN_BASE | TAG_NATIVETUPLE | (ptr & PAYLOAD_MASK))
    }

    /// Create a NativeSet value from an IndexSet.
    #[inline]
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn from_set(items: indexmap::IndexSet<crate::collections::ValueKey>) -> Self {
        let arc = Arc::new(NativeSet::new(items));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(ptr & !PAYLOAD_MASK == 0, "NativeSet pointer exceeds 47-bit");
        Value(QNAN_BASE | TAG_NATIVESET | (ptr & PAYLOAD_MASK))
    }

    /// Create a NativeBytes value from a Vec<u8>.
    #[inline]
    pub fn from_bytes(data: Vec<u8>) -> Self {
        let arc = Arc::new(NativeBytes::new(data));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(ptr & !PAYLOAD_MASK == 0, "NativeBytes pointer exceeds 47-bit");
        Value(QNAN_BASE | TAG_NATIVEBYTES | (ptr & PAYLOAD_MASK))
    }
}

// --- Debug / Display / PartialEq ---

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
        } else if self.is_native_str() {
            let s = unsafe { self.as_native_str_ref().unwrap() };
            write!(f, "\"{}\"", s)
        } else if self.is_native_list() {
            write!(f, "<list len={}>", unsafe { self.as_native_list_ref().unwrap().len() })
        } else if self.is_native_dict() {
            write!(f, "<dict len={}>", unsafe { self.as_native_dict_ref().unwrap().len() })
        } else if self.is_native_tuple() {
            write!(f, "<tuple len={}>", unsafe {
                self.as_native_tuple_ref().unwrap().len()
            })
        } else if self.is_native_set() {
            write!(f, "<set len={}>", unsafe { self.as_native_set_ref().unwrap().len() })
        } else if self.is_native_bytes() {
            write!(f, "<bytes len={}>", unsafe {
                self.as_native_bytes_ref().unwrap().len()
            })
        } else if let Some(sym) = self.as_symbol() {
            write!(f, "sym#{}", sym)
        } else if self.is_invalid() {
            write!(f, "INVALID(canary)")
        } else if self.is_vmfunc() {
            write!(f, "vmfunc#{}", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_struct_instance() {
            write!(f, "struct#{}", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_struct_type() {
            write!(f, "structtype#{}", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_module() {
            let ns = unsafe { self.as_module_ref().unwrap() };
            write!(f, "module({})", ns.name)
        } else if self.is_extended() {
            write!(f, "extended")
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
        // Bitwise match covers immediates (SmallInt, Bool, Nil, Symbol, VMFunc)
        if self.0 == other.0 {
            return true;
        }
        if self.is_float() && other.is_float() {
            self.as_float() == other.as_float()
        } else if self.is_bigint() && other.is_bigint() {
            unsafe { self.as_bigint_ref().unwrap() == other.as_bigint_ref().unwrap() }
        } else if self.is_native_str() && other.is_native_str() {
            unsafe { self.as_native_str_ref().unwrap() == other.as_native_str_ref().unwrap() }
        } else if self.is_native_list() && other.is_native_list() {
            // Compare element-by-element
            let a = unsafe { self.as_native_list_ref().unwrap() };
            let b = unsafe { other.as_native_list_ref().unwrap() };
            let av = a.as_slice_cloned();
            let bv = b.as_slice_cloned();
            let eq = av.len() == bv.len() && av.iter().zip(bv.iter()).all(|(x, y)| x == y);
            for v in &av {
                v.decref();
            }
            for v in &bv {
                v.decref();
            }
            eq
        } else if self.is_native_tuple() && other.is_native_tuple() {
            let a = unsafe { self.as_native_tuple_ref().unwrap() };
            let b = unsafe { other.as_native_tuple_ref().unwrap() };
            a.as_slice() == b.as_slice()
        } else if self.is_native_bytes() && other.is_native_bytes() {
            let a = unsafe { self.as_native_bytes_ref().unwrap() };
            let b = unsafe { other.as_native_bytes_ref().unwrap() };
            a.as_bytes() == b.as_bytes()
        } else if self.is_complex() && other.is_complex() {
            unsafe {
                let (ar, ai) = self.as_complex_parts().unwrap();
                let (br, bi) = other.as_complex_parts().unwrap();
                ar == br && ai == bi
            }
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size() {
        assert_eq!(std::mem::size_of::<Value>(), 8);
    }

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
    fn test_nan_display() {
        let v = Value::from_float(f64::NAN);
        assert_eq!(v.display_string(), "nan");
    }

    #[test]
    fn test_nan_truthy() {
        // Python: bool(float('nan')) == True
        let v = Value::from_float(f64::NAN);
        assert!(v.is_truthy());
    }

    #[test]
    fn test_bigint() {
        let v = Value::from_bigint(Integer::from(42));
        assert!(v.is_bigint());
        assert_eq!(unsafe { v.as_bigint_ref() }, Some(&Integer::from(42)));
        v.decref();
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
    fn test_bigint_or_demote() {
        let small = Value::from_bigint_or_demote(Integer::from(42));
        assert!(small.is_int());
        assert_eq!(small.as_int(), Some(42));

        let big = Integer::from(1_u64 << 50);
        let v = Value::from_bigint_or_demote(big.clone());
        assert!(v.is_bigint());
        assert_eq!(unsafe { v.as_bigint_ref() }, Some(&big));
        v.decref();
    }

    #[test]
    fn test_bigint_clone_refcount() {
        let v = Value::from_bigint(Integer::from(42));
        v.clone_refcount();
        v.decref();
        assert!(v.is_bigint());
        v.decref();
    }

    #[test]
    fn test_native_str() {
        let v = Value::from_str("hello");
        assert!(v.is_native_str());
        assert!(!v.is_int());
        assert!(!v.is_float());
        assert!(!v.is_nil());
        assert_eq!(unsafe { v.as_native_str_ref() }, Some("hello"));
        v.decref();
    }

    #[test]
    fn test_native_str_empty() {
        let v = Value::from_str("");
        assert!(v.is_native_str());
        assert!(!v.is_truthy()); // empty string is falsy
        v.decref();
    }

    #[test]
    fn test_native_str_truthy() {
        let v = Value::from_str("x");
        assert!(v.is_truthy());
        v.decref();
    }

    #[test]
    fn test_native_str_equality() {
        let a = Value::from_str("hello");
        let b = Value::from_str("hello");
        assert_eq!(a, b);

        let c = Value::from_str("world");
        assert_ne!(a, c);

        a.decref();
        b.decref();
        c.decref();
    }

    #[test]
    fn test_native_str_clone_refcount() {
        let v = Value::from_str("test");
        v.clone_refcount();
        v.decref();
        assert!(v.is_native_str());
        assert_eq!(unsafe { v.as_native_str_ref() }, Some("test"));
        v.decref();
    }

    #[test]
    fn test_native_str_display() {
        let v = Value::from_str("hello");
        assert_eq!(v.display_string(), "hello");
        assert_eq!(v.repr_string(), "'hello'");
        v.decref();
    }

    #[test]
    fn test_invalid_tag() {
        assert!(Value::INVALID.is_invalid());
        assert!(!Value::NIL.is_invalid());
        assert!(!Value::TRUE.is_invalid());
        assert!(!Value::from_int(0).is_invalid());
    }

    #[test]
    fn test_type_discrimination() {
        // Each type check should be exclusive
        let values: Vec<Value> = vec![
            Value::from_int(1),
            Value::from_float(1.0),
            Value::TRUE,
            Value::NIL,
            Value::from_symbol(0),
            Value::from_bigint(Integer::from(1)),
            Value::from_vmfunc(0),
            Value::from_str("x"),
            Value::from_list(vec![]),
            Value::from_empty_dict(),
            Value::from_tuple(vec![]),
            Value::from_set(indexmap::IndexSet::new()),
            Value::from_bytes(vec![]),
            Value::from_struct_instance(0),
            Value::from_struct_type(0),
        ];
        for (i, a) in values.iter().enumerate() {
            let checks = [
                a.is_int(),
                a.is_float(),
                a.is_bool(),
                a.is_nil(),
                a.is_symbol(),
                a.is_bigint(),
                a.is_vmfunc(),
                a.is_native_str(),
                a.is_native_list(),
                a.is_native_dict(),
                a.is_native_tuple(),
                a.is_native_set(),
                a.is_native_bytes(),
                a.is_struct_instance(),
                a.is_struct_type(),
            ];
            let true_count = checks.iter().filter(|&&b| b).count();
            assert_eq!(
                true_count, 1,
                "value {:?} (index {}) has {} true type checks: {:?}",
                a, i, true_count, checks
            );
        }
        // cleanup heap values (bigint, vmfunc, str, collections -- struct tags are index-based, no decref)
        for v in &values[5..13] {
            v.decref();
        }
    }

    #[test]
    fn test_collection_truthiness() {
        let empty_list = Value::from_list(vec![]);
        assert!(!empty_list.is_truthy());
        empty_list.decref();

        let nonempty_list = Value::from_list(vec![Value::from_int(1)]);
        assert!(nonempty_list.is_truthy());
        nonempty_list.decref();

        let empty_tuple = Value::from_tuple(vec![]);
        assert!(!empty_tuple.is_truthy());
        empty_tuple.decref();

        let empty_dict = Value::from_empty_dict();
        assert!(!empty_dict.is_truthy());
        empty_dict.decref();

        let empty_bytes = Value::from_bytes(vec![]);
        assert!(!empty_bytes.is_truthy());
        empty_bytes.decref();
    }

    #[test]
    fn test_collection_display() {
        let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
        assert_eq!(list.display_string(), "[1, 2]");
        list.decref();

        let tuple = Value::from_tuple(vec![Value::from_int(1)]);
        assert_eq!(tuple.display_string(), "(1,)");
        tuple.decref();

        let tuple2 = Value::from_tuple(vec![Value::from_int(1), Value::from_int(2)]);
        assert_eq!(tuple2.display_string(), "(1, 2)");
        tuple2.decref();

        let bytes = Value::from_bytes(b"hi".to_vec());
        assert_eq!(bytes.display_string(), "b'hi'");
        bytes.decref();
    }

    #[test]
    fn test_collection_equality() {
        let a = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
        let b = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
        assert_eq!(a, b);
        a.decref();
        b.decref();

        let t1 = Value::from_tuple(vec![Value::from_int(1)]);
        let t2 = Value::from_tuple(vec![Value::from_int(1)]);
        assert_eq!(t1, t2);
        t1.decref();
        t2.decref();

        let b1 = Value::from_bytes(vec![1, 2, 3]);
        let b2 = Value::from_bytes(vec![1, 2, 3]);
        assert_eq!(b1, b2);
        b1.decref();
        b2.decref();
    }

    #[test]
    fn test_collection_refcount() {
        let list = Value::from_list(vec![Value::from_int(1)]);
        list.clone_refcount();
        list.decref();
        assert!(list.is_native_list());
        list.decref();

        let tuple = Value::from_tuple(vec![Value::from_int(1)]);
        tuple.clone_refcount();
        tuple.decref();
        assert!(tuple.is_native_tuple());
        tuple.decref();
    }
}
