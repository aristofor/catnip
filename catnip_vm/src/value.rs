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
use crate::vm::func_table::PureFuncSlot;
use crate::vm::structs::StructCell;
use catnip_core::nanbox::{PAYLOAD_MASK, QNAN_BASE, TAG_BIGINT, TAG_BOOL, TAG_MASK, TAG_NIL, TAG_VMFUNC};
use indexmap::IndexMap;
use rug::Integer;
use std::fmt;
use std::sync::Arc;

// GmpInt is shared with the PyO3 VM (Phase 5): same scalar heap type, and
// its constructors feed the live-bigint ledger both sides observe.
pub use catnip_core::scalar::GmpInt;
use catnip_core::scalar::ScalarValue;

// Struct tags (catnip_vm only)
use catnip_core::nanbox::TAG_SHIFT;
pub(crate) const TAG_STRUCT: u64 = 5 << TAG_SHIFT;
pub(crate) const TAG_STRUCTTYPE: u64 = 14 << TAG_SHIFT;

// Runtime closure tag (catnip_vm only). Tag 4 is unused here (reserved for
// PyObject in catnip_rs). A `TAG_CLOSURE` value is a thin Arc pointer to a
// runtime `PureFuncSlot` (a `MakeFunction` closure or a `m = p.get` bound
// method), exactly like `TAG_STRUCT` -> `Arc<StructCell>`; the Arc strong count
// is the slot's refcount, so the slot frees as soon as no `Value` references it.
// Template function slots stay index-based under `TAG_VMFUNC`.
pub(crate) const TAG_CLOSURE: u64 = 4 << TAG_SHIFT;

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

impl Drop for ModuleNamespace {
    /// Release the strong ref each exported value carries. The loader/plugin
    /// path incref's every value before inserting it into `attrs`, and `Value`
    /// is `Copy` with no `Drop`, so dropping the map alone would leak the heap
    /// payloads (e.g. `sys.argv`, a `NativeList`).
    ///
    /// Also drain `module_globals`: the child pipeline's host dropped without
    /// releasing its per-entry refs (only the *parent* host's `clear_globals`
    /// runs at reset), so this map is the sole remaining owner -- skipping it
    /// leaks one ref per module-level heap value (a `StructCell` per exported
    /// instance, a runtime slot per exported closure). The drain is also the
    /// cycle-break for a module closure whose scope chain holds this same
    /// `Rc` while the map holds the closure. The map is taken out of the
    /// `RefCell` before any decref so a cascading `Drop` (nested module
    /// namespace, closure) can never re-borrow it.
    fn drop(&mut self) {
        for val in self.attrs.values() {
            val.decref();
        }
        crate::host::drain_globals(&self.module_globals);
    }
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

/// Tagged union namespace: maps variant names to their constructor values.
///
/// Payload variants bind a struct type (`TAG_STRUCTTYPE`) registered under
/// the qualified name `"Union.Variant"`; nullary variants bind the interned
/// symbol of the same qualified name. Both flavors are immediates, so the
/// namespace holds plain `Value`s with no refcount management.
pub struct UnionNamespace {
    pub name: String,
    /// Type parameters declared on the union (e.g. `[T]`). Parsed but not
    /// enforced -- kept for display and diagnostics.
    pub type_params: Vec<String>,
    /// (variant_name, binding) in declaration order.
    pub variants: Vec<(String, Value)>,
}

impl UnionNamespace {
    pub fn variant(&self, name: &str) -> Option<Value> {
        self.variants.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }
}

/// Extended value types sharing tag 15.
/// Single tag slot, unlimited future variants.
pub enum ExtendedValue {
    Module(ModuleNamespace),
    /// Enum type marker (type_id into EnumRegistry).
    EnumType(u32),
    /// Tagged union type namespace (variant constructors).
    Union(UnionNamespace),
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
                // SAFETY: drop_fn is the destructor the plugin registered alongside this
                // handle; this is the sole owner being dropped, so it runs exactly once, and
                // catch_unwind keeps a panic from unwinding across the FFI boundary.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                    drop_fn(*handle);
                }));
            }
        }
    }
}

// SAFETY: PureVM is single-threaded; an ExtendedValue is owned by a single VM
// and moved, never shared by reference across threads. `Send` covers that
// move. `Sync` is intentionally NOT implemented: ExtendedValue holds interior
// mutability (RefCell in NativeMeta, Rc in Module) whose borrow flags are not
// atomic, so handing out `&ExtendedValue` to two threads would be a data race.
unsafe impl Send for ExtendedValue {}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

/// NaN-boxed value. Fits in 8 bytes. Pure Rust -- no PyO3 dependency.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Value(u64);

impl catnip_core::scalar::ScalarValue for Value {
    #[inline]
    fn bits(self) -> u64 {
        self.0
    }
    #[inline]
    fn from_bits(bits: u64) -> Self {
        Value(bits)
    }
    #[inline]
    fn scalar_is_complex(self) -> bool {
        self.is_complex()
    }
    #[inline]
    unsafe fn scalar_as_complex_parts(&self) -> Option<(f64, f64)> {
        // SAFETY: same contract as the trait method -- the caller guarantees
        // the heap payload outlives the read; forwarded verbatim.
        unsafe { self.as_complex_parts() }
    }
    #[inline]
    fn scalar_from_complex(real: f64, imag: f64) -> Self {
        Self::from_complex(real, imag)
    }
}

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
        <Self as ScalarValue>::scalar_from_float(f)
    }

    /// Create a small integer value. Panics if out of 47-bit range.
    #[inline]
    pub fn from_int(i: i64) -> Self {
        <Self as ScalarValue>::scalar_from_int(i)
    }

    /// Try to create a small integer. Returns None if out of range.
    #[inline]
    pub fn try_from_int(i: i64) -> Option<Self> {
        <Self as ScalarValue>::scalar_try_from_int(i)
    }

    /// Create a boolean value.
    #[inline]
    pub fn from_bool(b: bool) -> Self {
        <Self as ScalarValue>::scalar_from_bool(b)
    }

    /// Create a symbol value (interned string index).
    #[inline]
    pub fn from_symbol(idx: u32) -> Self {
        <Self as ScalarValue>::scalar_from_symbol(idx)
    }

    /// Create a BigInt value from an Arc<GmpInt> pointer.
    #[inline]
    pub fn from_bigint(n: Integer) -> Self {
        <Self as ScalarValue>::scalar_from_bigint(n)
    }

    /// Create a BigInt or demote to SmallInt if it fits.
    #[inline]
    pub fn from_bigint_or_demote(n: Integer) -> Self {
        <Self as ScalarValue>::scalar_from_bigint_or_demote(n)
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

    /// Create a struct instance value from an owned `StructCell`.
    ///
    /// Stores `Arc<StructCell>` (thin pointer) in the 47-bit payload, exactly
    /// like the native collections. The Arc strong count is the instance
    /// refcount; `clone_refcount`/`decref` manage it.
    #[inline]
    #[allow(clippy::arc_with_non_send_sync)] // VM is single-threaded, Arc for refcounting only
    pub fn from_struct_instance(cell: StructCell) -> Self {
        let arc = Arc::new(cell);
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(ptr & !PAYLOAD_MASK == 0, "StructCell pointer exceeds 47-bit");
        Value(QNAN_BASE | TAG_STRUCT | (ptr & PAYLOAD_MASK))
    }

    /// Create a callable struct type value from its type_id.
    #[inline]
    pub fn from_struct_type(type_id: u32) -> Self {
        Value(QNAN_BASE | TAG_STRUCTTYPE | (type_id as u64))
    }

    /// Wrap a runtime closure slot in a `TAG_CLOSURE` value. The caller passes an
    /// already-built `Arc` (so the VM can register a `Weak` for the reset drain
    /// first); this transfers one strong ref into the NaN box, balanced by
    /// `clone_refcount`/`decref`. Mirrors `from_struct_instance`.
    #[inline]
    #[allow(clippy::arc_with_non_send_sync)] // VM is single-threaded, Arc for refcounting only
    pub fn from_arc_closure(slot: Arc<PureFuncSlot>) -> Self {
        let ptr = Arc::into_raw(slot) as u64;
        debug_assert!(ptr & !PAYLOAD_MASK == 0, "PureFuncSlot pointer exceeds 47-bit");
        Value(QNAN_BASE | TAG_CLOSURE | (ptr & PAYLOAD_MASK))
    }

    /// A refcount-neutral closure `Value` from a live Arc (does *not* transfer a
    /// ref). For resolve-style readers that `clone_refcount` before use; the Arc
    /// must stay alive until the reader takes its own ref. Used by the letrec
    /// weak self-ref path.
    #[inline]
    pub fn from_closure_neutral(slot: &Arc<PureFuncSlot>) -> Self {
        let ptr = Arc::as_ptr(slot) as u64;
        Value(QNAN_BASE | TAG_CLOSURE | (ptr & PAYLOAD_MASK))
    }

    /// A `Weak` handle to this closure's slot, without disturbing the strong
    /// count. Used by the VM's reset drain to break letrec mutual cycles. Returns
    /// `None` for a non-closure value.
    #[inline]
    pub fn closure_weak(self) -> Option<std::sync::Weak<PureFuncSlot>> {
        if self.is_closure() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const PureFuncSlot;
            // SAFETY: is_closure() guards a live Arc<PureFuncSlot>. from_raw adopts
            // the existing ref (no strong change), downgrade bumps only the weak
            // count, into_raw hands the strong ref back -- net strong unchanged.
            unsafe {
                let arc = Arc::from_raw(ptr);
                let weak = Arc::downgrade(&arc);
                let _ = Arc::into_raw(arc);
                Some(weak)
            }
        } else {
            None
        }
    }

    /// Create an Extended value (module, future: enum, union, ...).
    #[inline]
    #[allow(clippy::arc_with_non_send_sync)] // VM is single-threaded, Arc for refcounting only
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

    /// Create a tagged union type value.
    pub fn from_union_type(ns: UnionNamespace) -> Self {
        Self::from_extended(ExtendedValue::Union(ns))
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
        ScalarValue::scalar_is_float(self)
    }

    /// Check if this is a small integer.
    #[inline]
    pub fn is_int(self) -> bool {
        ScalarValue::scalar_is_int(self)
    }

    /// Check if this is a boolean.
    #[inline]
    pub fn is_bool(self) -> bool {
        ScalarValue::scalar_is_bool(self)
    }

    /// Check if this is nil (None).
    #[inline]
    pub fn is_nil(self) -> bool {
        ScalarValue::scalar_is_nil(self)
    }

    /// Check if this is a symbol.
    #[inline]
    pub fn is_symbol(self) -> bool {
        ScalarValue::scalar_is_symbol(self)
    }

    /// Check if this is a BigInt.
    #[inline]
    pub fn is_bigint(self) -> bool {
        ScalarValue::scalar_is_bigint(self)
    }

    /// Check if this is a native VM function.
    #[inline]
    pub fn is_vmfunc(self) -> bool {
        ScalarValue::scalar_is_vmfunc(self)
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

    /// Check if this is a runtime closure (`TAG_CLOSURE`, Arc-backed).
    #[inline]
    pub fn is_closure(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_CLOSURE)
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
        ScalarValue::scalar_as_float(self)
    }

    /// Extract as integer. Returns None if not an integer.
    #[inline]
    pub fn as_int(self) -> Option<i64> {
        ScalarValue::scalar_as_int(self)
    }

    /// Extract as boolean. Returns None if not a boolean.
    #[inline]
    pub fn as_bool(self) -> Option<bool> {
        ScalarValue::scalar_as_bool(self)
    }

    /// Extract as symbol index. Returns None if not a symbol.
    #[inline]
    pub fn as_symbol(self) -> Option<u32> {
        ScalarValue::scalar_as_symbol(self)
    }

    /// Extract VM function table index.
    #[inline]
    pub fn as_vmfunc_idx(self) -> u32 {
        debug_assert!(self.is_vmfunc() && !self.is_invalid());
        (self.0 & PAYLOAD_MASK) as u32
    }

    /// Borrow the `StructCell` behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive for the duration of the borrow
    /// -- in particular, must not `decref` this `Value` (which is `Copy`) while
    /// the returned reference is live. Same discipline as `as_native_list_ref`.
    #[inline]
    pub unsafe fn as_struct_ref(&self) -> Option<&StructCell> {
        if self.is_struct_instance() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const StructCell;
            // SAFETY: the is_struct_instance() guard proves a live Arc<StructCell>;
            // the caller upholds that the Arc outlives the returned borrow.
            Some(unsafe { &*ptr })
        } else {
            None
        }
    }

    /// Borrow the runtime `PureFuncSlot` behind the Arc without cloning.
    ///
    /// # Safety
    ///
    /// Caller must ensure the Arc is still alive for the duration of the borrow
    /// -- in particular, must not `decref` this `Value` (which is `Copy`) while
    /// the returned reference is live. Same discipline as `as_struct_ref`.
    #[inline]
    pub unsafe fn as_closure_ref(&self) -> Option<&PureFuncSlot> {
        if self.is_closure() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const PureFuncSlot;
            // SAFETY: the is_closure() guard proves a live Arc<PureFuncSlot>; the
            // caller upholds that the Arc outlives the returned borrow.
            Some(unsafe { &*ptr })
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
        // SAFETY: is_extended() was verified above, so the payload is a live
        // Arc<ExtendedValue>; as_extended_ref() borrows it without outliving it.
        matches!(unsafe { self.as_extended_ref() }, Some(ExtendedValue::Module(_)))
    }

    /// Check if this is an enum type marker.
    #[inline]
    pub fn is_enum_type(self) -> bool {
        if !self.is_extended() {
            return false;
        }
        // SAFETY: is_extended() was verified above, so the payload is a live
        // Arc<ExtendedValue>; as_extended_ref() borrows it without outliving it.
        matches!(unsafe { self.as_extended_ref() }, Some(ExtendedValue::EnumType(_)))
    }

    /// Extract enum type_id if this is an enum type marker.
    #[inline]
    pub fn as_enum_type_id(self) -> Option<u32> {
        if !self.is_extended() {
            return None;
        }
        // SAFETY: is_extended() was verified above, so the payload is a live
        // Arc<ExtendedValue>; as_extended_ref() borrows it without outliving it.
        match unsafe { self.as_extended_ref()? } {
            ExtendedValue::EnumType(id) => Some(*id),
            _ => None,
        }
    }

    /// Check if this is a tagged union type.
    #[inline]
    pub fn is_union_type(self) -> bool {
        if !self.is_extended() {
            return false;
        }
        // SAFETY: is_extended() was verified above, so the payload is a live
        // Arc<ExtendedValue>; as_extended_ref() borrows it without outliving it.
        matches!(unsafe { self.as_extended_ref() }, Some(ExtendedValue::Union(_)))
    }

    /// Borrow the UnionNamespace behind the Arc.
    ///
    /// # Safety
    /// Caller must ensure the Arc is still alive and the value is a Union.
    #[inline]
    pub unsafe fn as_union_ref(&self) -> Option<&UnionNamespace> {
        match self.as_extended_ref()? {
            ExtendedValue::Union(ns) => Some(ns),
            _ => None,
        }
    }

    /// Check if this is a META object.
    #[inline]
    pub fn is_meta(self) -> bool {
        if !self.is_extended() {
            return false;
        }
        // SAFETY: is_extended() was verified above, so the payload is a live
        // Arc<ExtendedValue>; as_extended_ref() borrows it without outliving it.
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
            // SAFETY: is_extended() was verified above, so the payload is a live
            // Arc<ExtendedValue>; as_extended_ref() borrows it without outliving it.
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
        // SAFETY: is_extended() was verified above, so the payload is a live
        // Arc<ExtendedValue>; as_extended_ref() borrows it without outliving it.
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
            // SAFETY: BIGINT tag => the payload is a live Arc<GmpInt>; this incref is balanced by a matching decref (NaN-box refcount discipline).
            TAG_BIGINT => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const GmpInt) },
            // SAFETY: NATIVESTR tag => a live Arc<NativeString>; this incref is balanced by a matching decref.
            TAG_NATIVESTR => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeString) },
            // SAFETY: NATIVELIST tag => a live Arc<NativeList>; this incref is balanced by a matching decref.
            TAG_NATIVELIST => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeList) },
            // SAFETY: NATIVEDICT tag => a live Arc<NativeDict>; this incref is balanced by a matching decref.
            TAG_NATIVEDICT => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeDict) },
            // SAFETY: NATIVETUPLE tag => a live Arc<NativeTuple>; this incref is balanced by a matching decref.
            TAG_NATIVETUPLE => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeTuple) },
            // SAFETY: NATIVESET tag => a live Arc<NativeSet>; this incref is balanced by a matching decref.
            TAG_NATIVESET => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeSet) },
            // SAFETY: NATIVEBYTES tag => a live Arc<NativeBytes>; this incref is balanced by a matching decref.
            TAG_NATIVEBYTES => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const NativeBytes) },
            // SAFETY: EXTENDED tag => a live Arc<ExtendedValue>; this incref is balanced by a matching decref.
            TAG_EXTENDED => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const ExtendedValue) },
            // SAFETY: STRUCT tag => a live Arc<StructCell>; this incref is balanced by a matching decref.
            TAG_STRUCT => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const StructCell) },
            // SAFETY: CLOSURE tag => a live Arc<PureFuncSlot>; this incref is balanced by a matching decref.
            TAG_CLOSURE => unsafe { Arc::increment_strong_count((self.0 & PAYLOAD_MASK) as *const PureFuncSlot) },
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
            // SAFETY: BIGINT tag => the payload is a live Arc<GmpInt>; this decref balances the incref that created/cloned it (NaN-box refcount discipline).
            TAG_BIGINT => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const GmpInt) },
            // SAFETY: NATIVESTR tag => a live Arc<NativeString>; this decref balances the incref that created/cloned it.
            TAG_NATIVESTR => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeString) },
            // SAFETY: NATIVELIST tag => a live Arc<NativeList>; this decref balances the incref that created/cloned it.
            TAG_NATIVELIST => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeList) },
            // SAFETY: NATIVEDICT tag => a live Arc<NativeDict>; this decref balances the incref that created/cloned it.
            TAG_NATIVEDICT => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeDict) },
            // SAFETY: NATIVETUPLE tag => a live Arc<NativeTuple>; this decref balances the incref that created/cloned it.
            TAG_NATIVETUPLE => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeTuple) },
            // SAFETY: NATIVESET tag => a live Arc<NativeSet>; this decref balances the incref that created/cloned it.
            TAG_NATIVESET => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeSet) },
            // SAFETY: NATIVEBYTES tag => a live Arc<NativeBytes>; this decref balances the incref that created/cloned it.
            TAG_NATIVEBYTES => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const NativeBytes) },
            // SAFETY: EXTENDED tag => a live Arc<ExtendedValue>; this decref balances the incref that created/cloned it.
            TAG_EXTENDED => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const ExtendedValue) },
            // SAFETY: STRUCT tag => a live Arc<StructCell>; this decref balances the incref that created/cloned it.
            // At strong count 0 the StructCell's Drop cascades a decref into each field.
            TAG_STRUCT => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const StructCell) },
            // SAFETY: CLOSURE tag => a live Arc<PureFuncSlot>; this decref balances the incref that created/cloned it.
            // At strong count 0 the slot's Drop releases bound_self and drops its PureClosureScope (captures decref).
            TAG_CLOSURE => unsafe { Arc::decrement_strong_count((self.0 & PAYLOAD_MASK) as *const PureFuncSlot) },
            _ => {}
        }
    }

    /// Decrement refcount only for BigInt values.
    #[inline]
    pub fn decref_bigint(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
            // SAFETY: is_bigint() proves the payload is a live Arc<GmpInt>; this decref
            // balances the incref that created/cloned it (NaN-box refcount discipline).
            unsafe { Arc::decrement_strong_count(ptr) };
        }
    }

    /// Test-only: strong count of the underlying BigInt Arc, read without
    /// touching it (`from_raw`/`into_raw` round-trip is refcount-neutral).
    #[cfg(test)]
    pub fn bigint_strong_count(self) -> usize {
        debug_assert!(self.is_bigint());
        let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
        // SAFETY: ptr was produced by Arc::into_raw on the same GmpInt (from_bigint); from_raw
        // reconstructs it exactly once here, and into_raw below re-leaks it, leaving the strong
        // count unchanged.
        let arc = unsafe { Arc::from_raw(ptr) };
        let count = Arc::strong_count(&arc);
        let _ = Arc::into_raw(arc);
        count
    }

    /// Test-only: strong count of the underlying NativeList Arc, read without
    /// touching it (refcount-neutral round-trip). Same probe as
    /// `bigint_strong_count`.
    #[cfg(test)]
    pub fn native_list_strong_count(self) -> usize {
        debug_assert!(self.is_native_list());
        let ptr = (self.0 & PAYLOAD_MASK) as *const NativeList;
        // SAFETY: ptr was produced by Arc::into_raw on the same NativeList
        // (from_list); from_raw reconstructs it exactly once here, and into_raw
        // below re-leaks it, leaving the strong count unchanged.
        let arc = unsafe { Arc::from_raw(ptr) };
        let count = Arc::strong_count(&arc);
        let _ = Arc::into_raw(arc);
        count
    }

    /// Test-only: strong count of the underlying ExtendedValue Arc, read
    /// without touching it (`from_raw`/`into_raw` round-trip is
    /// refcount-neutral). Same probe as `bigint_strong_count`.
    #[cfg(test)]
    pub fn extended_strong_count(self) -> usize {
        debug_assert!(self.is_extended());
        let ptr = (self.0 & PAYLOAD_MASK) as *const ExtendedValue;
        // SAFETY: ptr was produced by Arc::into_raw on the same ExtendedValue
        // (from_extended); from_raw reconstructs it exactly once here, and
        // into_raw below re-leaks it, leaving the strong count unchanged.
        let arc = unsafe { Arc::from_raw(ptr) };
        let count = Arc::strong_count(&arc);
        let _ = Arc::into_raw(arc);
        count
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
            // SAFETY: the is_bigint() guard proves a live Arc<GmpInt>; the borrow stays in this branch.
            unsafe { self.as_bigint_ref().unwrap().cmp0() != std::cmp::Ordering::Equal }
        } else if self.is_native_str() {
            // SAFETY: the is_native_str() guard proves a live Arc<NativeString>; the borrow stays in this branch.
            unsafe { !self.as_native_str_ref().unwrap().is_empty() }
        } else if self.is_native_list() {
            // SAFETY: the is_native_list() guard proves a live Arc<NativeList>; the borrow stays in this branch.
            unsafe { !self.as_native_list_ref().unwrap().is_empty() }
        } else if self.is_native_dict() {
            // SAFETY: the is_native_dict() guard proves a live Arc<NativeDict>; the borrow stays in this branch.
            unsafe { !self.as_native_dict_ref().unwrap().is_empty() }
        } else if self.is_native_tuple() {
            // SAFETY: the is_native_tuple() guard proves a live Arc<NativeTuple>; the borrow stays in this branch.
            unsafe { !self.as_native_tuple_ref().unwrap().is_empty() }
        } else if self.is_native_set() {
            // SAFETY: the is_native_set() guard proves a live Arc<NativeSet>; the borrow stays in this branch.
            unsafe { !self.as_native_set_ref().unwrap().is_empty() }
        } else if self.is_native_bytes() {
            // SAFETY: the is_native_bytes() guard proves a live Arc<NativeBytes>; the borrow stays in this branch.
            unsafe { !self.as_native_bytes_ref().unwrap().is_empty() }
        } else if self.is_complex() {
            // SAFETY: the is_complex() guard proves a live Arc<ExtendedValue>::Complex; the copied parts outlive the borrow.
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
            // SAFETY: the is_bigint() guard proves a live Arc<GmpInt>; the borrow does not outlive this branch.
            let n = unsafe { self.as_bigint_ref().unwrap() };
            n.to_string()
        } else if self.is_native_str() {
            // SAFETY: the is_native_str() guard proves a live Arc<NativeString>; the borrow does not outlive this branch.
            unsafe { self.as_native_str_ref().unwrap().to_string() }
        } else if self.is_native_list() {
            // SAFETY: the is_native_list() guard proves a live Arc<NativeList>; the borrow does not outlive this branch.
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
            // SAFETY: the is_native_tuple() guard proves a live Arc<NativeTuple>; the borrow does not outlive this branch.
            let tuple = unsafe { self.as_native_tuple_ref().unwrap() };
            let items = tuple.as_slice();
            let parts: Vec<String> = items.iter().map(|v| v.repr_string()).collect();
            if items.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            }
        } else if self.is_native_dict() {
            // SAFETY: the is_native_dict() guard proves a live Arc<NativeDict>; the borrow does not outlive this branch.
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
            // SAFETY: the is_native_set() guard proves a live Arc<NativeSet>; the borrow does not outlive this branch.
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
            // SAFETY: the is_native_bytes() guard proves a live Arc<NativeBytes>; the borrow does not outlive this branch.
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
        } else if self.is_closure() {
            "<function>".to_string()
        } else if self.is_struct_instance() {
            // Detailed display requires registry access (type name + fields) --
            // handled in VM dispatch. The bare payload is a pointer, not a name.
            "<struct>".to_string()
        } else if self.is_struct_type() {
            format!("<type #{}>", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_module() {
            // SAFETY: the is_module() guard proves a live Arc<ExtendedValue>::Module; the borrow does not outlive this branch.
            let ns = unsafe { self.as_module_ref().unwrap() };
            format!("<module '{}'>", ns.name)
        } else if self.is_union_type() {
            // str() form -- the bare union name, like the PyO3 __str__.
            // SAFETY: the is_union_type() guard proves a live Arc<ExtendedValue>::Union; the borrow does not outlive this branch.
            let ns = unsafe { self.as_union_ref().unwrap() };
            ns.name.clone()
        } else if self.is_meta() {
            "<META>".to_string()
        } else if self.is_complex() {
            // SAFETY: the is_complex() guard proves a live Arc<ExtendedValue>::Complex; the copied parts outlive the borrow.
            let (r, i) = unsafe { self.as_complex_parts().unwrap() };
            format_complex(r, i)
        } else if self.is_plugin_object() {
            // SAFETY: the is_plugin_object() guard proves a live Arc<ExtendedValue>::PluginObject; the borrow does not outlive this branch.
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
            // SAFETY: the is_native_str() guard proves a live Arc<NativeString>; the borrow does not outlive this branch.
            let s = unsafe { self.as_native_str_ref().unwrap() };
            format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
        } else if self.is_union_type() {
            // SAFETY: the is_union_type() guard proves a live Arc<ExtendedValue>::Union; the borrow does not outlive this branch.
            let ns = unsafe { self.as_union_ref().unwrap() };
            if ns.type_params.is_empty() {
                format!("<union '{}'>", ns.name)
            } else {
                format!("<union '{}[{}]'>", ns.name, ns.type_params.join(", "))
            }
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
            // SAFETY: the is_bigint() guard proves a live Arc<GmpInt>; the borrow does not outlive this branch.
            let n = unsafe { self.as_bigint_ref().unwrap() };
            write!(f, "{}n", n)
        } else if self.is_native_str() {
            // SAFETY: the is_native_str() guard proves a live Arc<NativeString>; the borrow does not outlive this branch.
            let s = unsafe { self.as_native_str_ref().unwrap() };
            write!(f, "\"{}\"", s)
        } else if self.is_native_list() {
            // SAFETY: the is_native_list() guard proves a live Arc<NativeList>; the borrow does not outlive this branch.
            write!(f, "<list len={}>", unsafe { self.as_native_list_ref().unwrap().len() })
        } else if self.is_native_dict() {
            // SAFETY: the is_native_dict() guard proves a live Arc<NativeDict>; the borrow does not outlive this branch.
            write!(f, "<dict len={}>", unsafe { self.as_native_dict_ref().unwrap().len() })
        } else if self.is_native_tuple() {
            // SAFETY: the is_native_tuple() guard proves a live Arc<NativeTuple>; the borrow does not outlive this branch.
            write!(f, "<tuple len={}>", unsafe {
                self.as_native_tuple_ref().unwrap().len()
            })
        } else if self.is_native_set() {
            // SAFETY: the is_native_set() guard proves a live Arc<NativeSet>; the borrow does not outlive this branch.
            write!(f, "<set len={}>", unsafe { self.as_native_set_ref().unwrap().len() })
        } else if self.is_native_bytes() {
            // SAFETY: the is_native_bytes() guard proves a live Arc<NativeBytes>; the borrow does not outlive this branch.
            write!(f, "<bytes len={}>", unsafe {
                self.as_native_bytes_ref().unwrap().len()
            })
        } else if let Some(sym) = self.as_symbol() {
            write!(f, "sym#{}", sym)
        } else if self.is_invalid() {
            write!(f, "INVALID(canary)")
        } else if self.is_vmfunc() {
            write!(f, "vmfunc#{}", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_closure() {
            write!(f, "closure@{:x}", self.0 & PAYLOAD_MASK)
        } else if self.is_struct_instance() {
            write!(f, "struct#{}", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_struct_type() {
            write!(f, "structtype#{}", (self.0 & PAYLOAD_MASK) as u32)
        } else if self.is_module() {
            // SAFETY: the is_module() guard proves a live Arc<ExtendedValue>::Module; the borrow does not outlive this branch.
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

/// Structural equality of native containers (list, tuple, dict, set, bytes) +
/// identity/bits equality of leaves: struct instances compare by slot, symbols
/// by bits. This has **no struct registry**, so two distinct instances with
/// equal fields are *not* equal here. For registry-aware equality (struct
/// payloads resolved by fields), use `deep_eq` in the VM, which the Eq/Ne,
/// `in`, and `index`/`count` paths route through.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        // Bitwise match covers immediates (SmallInt, Bool, Nil, Symbol, VMFunc)
        if self.0 == other.0 {
            return true;
        }
        if self.is_float() && other.is_float() {
            self.as_float() == other.as_float()
        } else if self.is_bigint() && other.is_bigint() {
            // SAFETY: both is_bigint() guards prove a live Arc<GmpInt> on each side; the borrows do not outlive this branch.
            unsafe { self.as_bigint_ref().unwrap() == other.as_bigint_ref().unwrap() }
        } else if self.is_native_str() && other.is_native_str() {
            // SAFETY: both is_native_str() guards prove a live Arc<NativeString> on each side; the borrows do not outlive this branch.
            unsafe { self.as_native_str_ref().unwrap() == other.as_native_str_ref().unwrap() }
        } else if self.is_native_list() && other.is_native_list() {
            // Compare element-by-element
            // SAFETY: the is_native_list() guard on self proves a live Arc<NativeList>; the borrow does not outlive this branch.
            let a = unsafe { self.as_native_list_ref().unwrap() };
            // SAFETY: the is_native_list() guard on other proves a live Arc<NativeList>; the borrow does not outlive this branch.
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
            // SAFETY: the is_native_tuple() guard on self proves a live Arc<NativeTuple>; the borrow does not outlive this branch.
            let a = unsafe { self.as_native_tuple_ref().unwrap() };
            // SAFETY: the is_native_tuple() guard on other proves a live Arc<NativeTuple>; the borrow does not outlive this branch.
            let b = unsafe { other.as_native_tuple_ref().unwrap() };
            a.as_slice() == b.as_slice()
        } else if self.is_native_bytes() && other.is_native_bytes() {
            // SAFETY: the is_native_bytes() guard on self proves a live Arc<NativeBytes>; the borrow does not outlive this branch.
            let a = unsafe { self.as_native_bytes_ref().unwrap() };
            // SAFETY: the is_native_bytes() guard on other proves a live Arc<NativeBytes>; the borrow does not outlive this branch.
            let b = unsafe { other.as_native_bytes_ref().unwrap() };
            a.as_bytes() == b.as_bytes()
        } else if self.is_complex() && other.is_complex() {
            // SAFETY: both is_complex() guards prove a live Arc<ExtendedValue>::Complex on each side; the copied parts outlive the borrows.
            unsafe {
                let (ar, ai) = self.as_complex_parts().unwrap();
                let (br, bi) = other.as_complex_parts().unwrap();
                ar == br && ai == bi
            }
        } else if self.is_native_dict() && other.is_native_dict() {
            // SAFETY: the is_native_dict() guard on self proves a live Arc<NativeDict>; the borrow does not outlive this branch.
            let a = unsafe { self.as_native_dict_ref().unwrap() };
            // SAFETY: the is_native_dict() guard on other proves a live Arc<NativeDict>; the borrow does not outlive this branch.
            let b = unsafe { other.as_native_dict_ref().unwrap() };
            if a.len() != b.len() {
                return false;
            }
            for k in a.keys_cloned() {
                if !b.contains_key(&k) {
                    return false;
                }
                let va = a.get_item(&k).unwrap_or(Value::NIL);
                let vb = b.get_item(&k).unwrap_or(Value::NIL);
                let eq = va == vb;
                va.decref();
                vb.decref();
                if !eq {
                    return false;
                }
            }
            true
        } else if self.is_native_set() && other.is_native_set() {
            // Set members are ValueKeys (hashables only): key equality suffices.
            // SAFETY: the is_native_set() guard on self proves a live Arc<NativeSet>; the borrow does not outlive this branch.
            let a = unsafe { self.as_native_set_ref().unwrap() };
            // SAFETY: the is_native_set() guard on other proves a live Arc<NativeSet>; the borrow does not outlive this branch.
            let b = unsafe { other.as_native_set_ref().unwrap() };
            a.len() == b.len() && a.copy().iter().all(|k| b.contains(k))
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catnip_core::nanbox::{SMALLINT_MAX, SMALLINT_MIN, TAG_BIGINT, TAG_SMALLINT, TAG_SYMBOL};

    // Conformance mirror for the boundary lock (CatnipBoundaryProof): this
    // crate's scalar tags are exactly {0,1,2,3}, and from_raw_scalar rejects
    // every index/pointer-backed tag this Value uses (native collections,
    // bigint, struct types).
    #[test]
    fn boundary_classification_conforms() {
        use catnip_core::nanbox::{from_raw_scalar, is_scalar_tag};
        let tagged = |tag: u64, p: u64| QNAN_BASE | tag | (p & PAYLOAD_MASK);

        for tag in [TAG_SMALLINT, TAG_BOOL, TAG_NIL, TAG_SYMBOL] {
            let w = tagged(tag, 1);
            assert!(is_scalar_tag(tag >> TAG_SHIFT));
            assert_eq!(from_raw_scalar(w), Some(w));
        }
        let fb = 2.5f64.to_bits();
        assert_eq!(from_raw_scalar(fb), Some(fb));

        // Index: Struct, VMFunc, StructType. Pointer: Closure, BigInt + native collections + Extended.
        for tag in [
            TAG_STRUCT,
            TAG_VMFUNC,
            TAG_STRUCTTYPE,
            TAG_CLOSURE,
            TAG_BIGINT,
            TAG_NATIVESTR,
            TAG_NATIVELIST,
            TAG_NATIVEDICT,
            TAG_NATIVETUPLE,
            TAG_NATIVESET,
            TAG_NATIVEBYTES,
            TAG_EXTENDED,
        ] {
            assert!(!is_scalar_tag(tag >> TAG_SHIFT));
            assert_eq!(
                from_raw_scalar(tagged(tag, 1)),
                None,
                "tag {} must be rejected",
                tag >> TAG_SHIFT
            );
        }
    }

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
        // SAFETY: v is a live BigInt constructed just above and not yet decref'd.
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
        // SAFETY: v is a live BigInt constructed just above and not yet decref'd.
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
        // SAFETY: v is a live NativeStr constructed just above and not yet decref'd.
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
        // SAFETY: v is a live NativeStr (strong count >= 1) and not yet fully decref'd.
        assert_eq!(unsafe { v.as_native_str_ref() }, Some("test"));
        v.decref();
    }

    // A ModuleNamespace owns one strong ref per exported value (incref'd at
    // insertion in the loader/plugin path). Without a Drop, dropping the map
    // leaks those heap payloads -- sys.argv being the canonical case.
    #[test]
    fn module_namespace_drop_releases_attrs() {
        use crate::collections::ValueKey;

        let payload = Value::from_str("exported"); // NativeString strong=1
        let witness = match payload.to_key().unwrap() {
            ValueKey::Str(a) => a, // strong=2 (attr ref + witness)
            _ => unreachable!(),
        };
        assert_eq!(Arc::strong_count(&witness), 2, "witness setup");

        // Mirror the loader: attrs takes ownership of the value's strong ref.
        let mut attrs = IndexMap::new();
        attrs.insert("x".to_string(), payload);
        let module = Value::from_module(ModuleNamespace {
            name: "m".to_string(),
            attrs,
            module_globals: std::rc::Rc::new(RefCell::new(IndexMap::new())),
        });
        assert_eq!(Arc::strong_count(&witness), 2, "module alive: ref held by attrs");

        // Drop the module: ExtendedValue::Module -> ModuleNamespace::drop -> decref.
        module.decref();
        assert_eq!(
            Arc::strong_count(&witness),
            1,
            "ModuleNamespace::drop leaked an exported attr"
        );
    }

    // Two namespaces aliasing one shared value -- the plugin static-descriptor
    // pattern, where every load borrows the same attr bits. Each must own its
    // own ref, so dropping one namespace must not free what the other reads.
    #[test]
    fn module_namespace_aliased_attr_survives_sibling_drop() {
        use crate::collections::ValueKey;

        let shared = Value::from_str("rust"); // strong=1 (the "static descriptor")
        let witness = match shared.to_key().unwrap() {
            ValueKey::Str(a) => a, // strong=2
            _ => unreachable!(),
        };

        // Each namespace borrows `shared` and takes its own ref (mirrors load()).
        let make_ns = || {
            shared.clone_refcount();
            let mut attrs = IndexMap::new();
            attrs.insert("PROTOCOL".to_string(), shared);
            Value::from_module(ModuleNamespace {
                name: "io".to_string(),
                attrs,
                module_globals: std::rc::Rc::new(RefCell::new(IndexMap::new())),
            })
        };
        let ns1 = make_ns(); // strong=3
        let ns2 = make_ns(); // strong=4
        assert_eq!(Arc::strong_count(&witness), 4, "each namespace owns its own ref");

        ns1.decref(); // drop ns1 -> releases only its ref
        assert_eq!(Arc::strong_count(&witness), 3, "sibling drop released exactly one ref");

        // ns2 still reads a valid string -- no use-after-free.
        // SAFETY: ns2 is a live module; its "PROTOCOL" attr is a live NativeStr held by
        // ns2's attrs map, so borrowing it here cannot use-after-free.
        let proto = unsafe {
            ns2.as_module_ref().unwrap().attrs["PROTOCOL"]
                .as_native_str_ref()
                .unwrap()
        };
        assert_eq!(proto, "rust");

        ns2.decref(); // strong=2 (shared + witness)
        shared.decref(); // release the "static" ref -> strong=1 (witness only)
        assert_eq!(Arc::strong_count(&witness), 1);
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
            Value::from_struct_instance(StructCell::new(0, vec![])),
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
        // cleanup heap values (bigint through bytes, plus the struct instance --
        // all Arc-backed; vmfunc/struct-type tags are immediates, decref is a no-op)
        for v in &values[5..14] {
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
        use crate::collections::ValueKey;
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

        // dict: structural, order-independent
        let mut m1 = indexmap::IndexMap::new();
        m1.insert(ValueKey::Int(1), Value::from_int(10));
        m1.insert(ValueKey::Int(2), Value::from_int(20));
        let mut m2 = indexmap::IndexMap::new();
        m2.insert(ValueKey::Int(2), Value::from_int(20));
        m2.insert(ValueKey::Int(1), Value::from_int(10));
        let d1 = Value::from_dict(m1);
        let d2 = Value::from_dict(m2);
        assert_eq!(d1, d2);
        let mut m3 = indexmap::IndexMap::new();
        m3.insert(ValueKey::Int(1), Value::from_int(99));
        let d3 = Value::from_dict(m3);
        assert_ne!(d1, d3);
        d1.decref();
        d2.decref();
        d3.decref();

        // set: structural, order-independent
        let mut s1 = indexmap::IndexSet::new();
        s1.insert(ValueKey::Int(1));
        s1.insert(ValueKey::Int(2));
        let mut s2 = indexmap::IndexSet::new();
        s2.insert(ValueKey::Int(2));
        s2.insert(ValueKey::Int(1));
        let set1 = Value::from_set(s1);
        let set2 = Value::from_set(s2);
        assert_eq!(set1, set2);
        set1.decref();
        set2.decref();
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
