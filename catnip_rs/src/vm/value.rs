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
use super::structs::{CatnipStructProxy, StructRegistry};
use catnip_core::nanbox::{PAYLOAD_MASK, QNAN_BASE, TAG_BOOL, TAG_MASK, TAG_NIL, TAG_SHIFT, TAG_VMFUNC};
use pyo3::PyTraverseError;
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyComplex, PyFloat, PyInt};
use rug::Integer;
use std::cell::Cell;
use std::fmt;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Live-allocation ledger -- manually refcounted heap values
// ---------------------------------------------------------------------------

/// Live-allocation counters for the heap values whose refs are managed by
/// hand in the dispatch loop (invisible to both the borrow checker and the
/// Python GC). Incremented at construction, decremented in `Drop`, so a
/// forgotten operand release shows up as a non-zero delta across a full
/// pipeline lifecycle -- the exact class of the operand-audit leaks. Always
/// compiled: the Python suite runs on the release extension, and the cost is
/// one Relaxed `fetch_add` per allocation (dominated by the allocation
/// itself), nothing on incref/decref. Read via [`debug_live_counts`].
pub(crate) use catnip_core::scalar::LIVE_BIGINT;
pub(crate) static LIVE_COMPLEX: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

// GmpInt and its live ledger are shared with the pure VM (Phase 5):
// both Value types wrap the same scalar heap type, one counter observes both.
pub use catnip_core::scalar::GmpInt;
use catnip_core::scalar::ScalarValue;

/// The map of nullary-union-method bindings, keyed by qualified name
/// ("Union.Variant").
type UnionNullaryMethodMap = std::collections::HashMap<String, Arc<indexmap::IndexMap<String, Py<PyAny>>>>;

/// A thread's nullary-union-method bindings behind an `Arc` for copy-on-write
/// sharing: it is read-mostly (looked up at every `TAG_SYMBOL -> variant`
/// round-trip) and written only at union-type build, so snapshot/install is one
/// atomic incref rather than a deep map clone. `Send` (every `Py` is `Send`), so
/// the `Arc` can be installed on another thread -- ND workers inherit the
/// parent's bindings.
pub type UnionNullaryMethodTable = Arc<UnionNullaryMethodMap>;

/// Method tables for nullary union variants, keyed by qualified name ("Union.Variant").
type UnionNullaryMethods = std::cell::RefCell<UnionNullaryMethodTable>;

thread_local! {
    static STRUCT_REGISTRY: Cell<*const StructRegistry> = const { Cell::new(std::ptr::null()) };
    static SYMBOL_TABLE: Cell<*mut catnip_core::symbols::SymbolTable> = const { Cell::new(std::ptr::null_mut()) };
    static ENUM_REGISTRY: Cell<*mut super::enums::EnumRegistry> = const { Cell::new(std::ptr::null_mut()) };
    /// Union methods for nullary variants, keyed by qualified name
    /// ("Union.Variant"). Populated by `build_union_type` so the symbol
    /// round-trip (TAG_SYMBOL -> CatnipEnumVariant) restores method
    /// bindings -- the NaN-boxed symbol itself cannot carry them.
    static UNION_NULLARY_METHODS: UnionNullaryMethods = std::cell::RefCell::new(Arc::new(std::collections::HashMap::new()));
}

/// Register the shared method map for a nullary union variant.
pub fn register_union_nullary_methods(qualified: &str, methods: Arc<indexmap::IndexMap<String, Py<PyAny>>>) {
    UNION_NULLARY_METHODS.with(|cell| {
        // Copy-on-write: `make_mut` clones the map only if an `Arc` snapshot is
        // currently shared (a worker mid-broadcast). At build time the `Arc` is
        // unique -- the main thread isn't parked in `py.detach` -- so this is an
        // in-place insert.
        let mut table = cell.borrow_mut();
        Arc::make_mut(&mut table).insert(qualified.to_string(), methods);
    });
}

/// Look up the method map for a nullary union variant, if any.
pub(crate) fn union_nullary_methods_for(qualified: &str) -> Option<Arc<indexmap::IndexMap<String, Py<PyAny>>>> {
    UNION_NULLARY_METHODS.with(|cell| cell.borrow().get(qualified).cloned())
}

/// Clone the `Arc` handle to the current thread's nullary-union-method bindings
/// -- one atomic incref, independent of table size. `Send`, so it can be
/// propagated to an ND worker thread.
pub fn snapshot_union_nullary_methods() -> UnionNullaryMethodTable {
    UNION_NULLARY_METHODS.with(|cell| Arc::clone(&cell.borrow()))
}

/// Replace the current thread's nullary-union-method bindings. Used by ND
/// workers to install the parent's bindings and to restore the prior table.
pub fn set_union_nullary_methods(table: UnionNullaryMethodTable) {
    // `replace` drops the previous `Arc` (and, if it was the last ref, its `Py`
    // values) AFTER the borrow ends, so a `Py` finalizer that re-enters union
    // registration can't trip a RefCell double-borrow.
    UNION_NULLARY_METHODS.with(|cell| {
        let _prev = cell.replace(table);
    });
}

/// Global ObjectTable shared across all threads.
///
/// Unlike StructRegistry (per-VM, thread-local), Python objects are global to
/// the interpreter. Handles must remain valid when Values cross thread
/// boundaries (e.g. ND recursion workers).  Under the GIL the Mutex is never
/// contended, so the lock is a single uncontended CAS (essentially free).
static OBJECT_TABLE: Mutex<ObjectTable> = Mutex::new(ObjectTable::new_const());

/// Permanent debug tracing of one object's handle life (leak hunts).
/// `CATNIP_TABLE_TRACE_PTR=<hex id(obj)>` traces every insert/clone/release
/// of EVERY slot that ever held that object (several slots can hold the same
/// pyobj simultaneously: one `Py` each), with a short backtrace. A slot
/// recycled to another object is evicted from the traced set (logged as
/// `recycle`), so the per-idx event balance is exact. Off = one atomic load
/// on the hot paths (the env is read once).
pub(crate) mod table_trace {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Mutex, Once};

    /// Traced object pointer; 0 = disarmed. Events match on the SLOT's current
    /// pointer (not on the idx seen at insert), so the ledger can be armed
    /// mid-run -- from Python via `_rs._debug_set_table_trace_ptr(id(obj))` --
    /// after the object already entered the table. The env var still arms it
    /// at first use for whole-run ledgers.
    static TARGET: AtomicUsize = AtomicUsize::new(0);
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static ENV_INIT: Once = Once::new();
    static CTX: Mutex<String> = Mutex::new(String::new());

    /// Whether tracing is armed (one relaxed load after the one-time env
    /// check; callers gate their formatting work on this).
    #[inline]
    pub(crate) fn active() -> bool {
        ENV_INIT.call_once(|| {
            if let Some(t) = std::env::var("CATNIP_TABLE_TRACE_PTR")
                .ok()
                .and_then(|p| usize::from_str_radix(p.trim_start_matches("0x"), 16).ok())
            {
                set_target(t);
            }
        });
        ACTIVE.load(Ordering::Relaxed)
    }

    /// Arm (ptr = id(obj)) or disarm (ptr = 0) the ledger at runtime.
    pub(crate) fn set_target(ptr: usize) {
        TARGET.store(ptr, Ordering::Relaxed);
        ACTIVE.store(ptr != 0, Ordering::Relaxed);
    }

    /// Publish the execution context shown by the next events (the VM loop
    /// posts "Op ip=N"; teardown paths post a label). Backtraces through the
    /// optimized cdylib resolve to <unknown>, an opcode ledger does not.
    pub(crate) fn set_ctx(ctx: String) {
        *CTX.lock().unwrap() = ctx;
    }

    fn ctx_string() -> String {
        CTX.lock().unwrap().clone()
    }

    fn matched(ptr: usize) -> bool {
        active() && TARGET.load(Ordering::Relaxed) == ptr
    }

    pub(super) fn on_insert(idx: u32, ptr: usize) {
        if matched(ptr) {
            eprintln!("TBLT insert idx={idx} at [{}]\n{}", ctx_string(), short_backtrace());
        }
    }

    #[inline]
    pub(super) fn on_clone(idx: u32, ptr: usize) {
        if matched(ptr) {
            eprintln!("TBLT clone idx={idx} at [{}]\n{}", ctx_string(), short_backtrace());
        }
    }

    #[inline]
    pub(super) fn on_release(idx: u32, ptr: usize) {
        if matched(ptr) {
            eprintln!("TBLT release idx={idx} at [{}]\n{}", ctx_string(), short_backtrace());
        }
    }

    fn short_backtrace() -> String {
        // Wide mode for leak hunts: CATNIP_TABLE_TRACE_WIDE=1 keeps non-catnip
        // frames and goes deeper (who ultimately holds the handle).
        let wide = std::env::var_os("CATNIP_TABLE_TRACE_WIDE").is_some();
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let take = if wide { 60 } else { 8 };
        bt.lines()
            .filter(|l| wide || l.contains("catnip"))
            .filter(|l| l.trim_start().starts_with("at ") || l.contains("catnip") || wide)
            .take(take)
            .collect::<Vec<_>>()
            .join("\n")
    }
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
///
/// Set-and-leave like `func_table`: a non-null leftover can name an `Executor`
/// that already returned. See [`save_func_table`] for the use-after-free hazard
/// and the `vm_depth() > 0` rule before capturing this pointer at top level and
/// writing through it later (e.g. a struct transplant in the import path).
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

/// Install a pointer to the active SymbolTable for the current thread.
///
/// # Safety
/// Uses `*mut` to allow lazy interning of symbols from imported modules.
/// Safe under the GIL (single-threaded access).
pub fn set_symbol_table(ptr: *mut catnip_core::symbols::SymbolTable) {
    SYMBOL_TABLE.with(|cell| cell.set(ptr));
}

/// Clear the thread-local symbol table pointer.
pub fn clear_symbol_table() {
    SYMBOL_TABLE.with(|cell| cell.set(std::ptr::null_mut()));
}

/// Save the current symbol table pointer (for reentrant VM calls).
pub fn save_symbol_table() -> *mut catnip_core::symbols::SymbolTable {
    SYMBOL_TABLE.with(|cell| cell.get())
}

/// Restore a previously saved symbol table pointer.
pub fn restore_symbol_table(ptr: *mut catnip_core::symbols::SymbolTable) {
    SYMBOL_TABLE.with(|cell| cell.set(ptr));
}

/// Resolve a symbol index to its interned string via the thread-local table.
pub(crate) fn resolve_symbol(idx: u32) -> Option<String> {
    SYMBOL_TABLE.with(|cell| {
        let ptr = cell.get();
        if ptr.is_null() {
            None
        } else {
            // SAFETY: ptr is non-null (checked above) and names the live thread-local SymbolTable installed by the active VM; access is single-threaded under the GIL and the borrow does not outlive this call.
            let table = unsafe { &*ptr };
            table.resolve(idx).map(|s| s.to_string())
        }
    })
}

/// Resolve a symbol name to its index via the thread-local table.
pub fn resolve_symbol_by_name(name: &str) -> Option<u32> {
    SYMBOL_TABLE.with(|cell| {
        let ptr = cell.get();
        if ptr.is_null() {
            None
        } else {
            // SAFETY: ptr is non-null (checked above) and names the live thread-local SymbolTable; read-only access, single-threaded under the GIL, borrow does not outlive this call.
            let table = unsafe { &*ptr };
            table.lookup(name)
        }
    })
}

/// Intern a symbol name in the thread-local table, returning its index.
/// Used for lazy registration of enum variants from imported modules.
pub fn intern_symbol(name: &str) -> Option<u32> {
    SYMBOL_TABLE.with(|cell| {
        let ptr = cell.get();
        if ptr.is_null() {
            None
        } else {
            // SAFETY: ptr is non-null (checked above) and names the live thread-local SymbolTable; the &mut is unique because access is single-threaded under the GIL.
            let table = unsafe { &mut *ptr };
            Some(table.intern(name))
        }
    })
}

/// Install a pointer to the active EnumRegistry for the current thread.
pub fn set_enum_registry(ptr: *mut super::enums::EnumRegistry) {
    ENUM_REGISTRY.with(|cell| cell.set(ptr));
}

/// Clear the thread-local enum registry pointer.
pub fn clear_enum_registry() {
    ENUM_REGISTRY.with(|cell| cell.set(std::ptr::null_mut()));
}

/// Save the current enum registry pointer (for reentrant VM calls).
pub fn save_enum_registry() -> *mut super::enums::EnumRegistry {
    ENUM_REGISTRY.with(|cell| cell.get())
}

/// Restore a previously saved enum registry pointer.
pub fn restore_enum_registry(ptr: *mut super::enums::EnumRegistry) {
    ENUM_REGISTRY.with(|cell| cell.set(ptr));
}

/// Lazily register an enum variant from an imported module.
/// Interns the symbol and registers the enum type if not already present.
/// Returns the symbol_id on success.
pub fn lazy_register_enum_variant(enum_name: &str, variant_name: &str) -> Option<u32> {
    SYMBOL_TABLE.with(|sym_cell| {
        ENUM_REGISTRY.with(|reg_cell| {
            let sym_ptr = sym_cell.get();
            let reg_ptr = reg_cell.get();
            if sym_ptr.is_null() || reg_ptr.is_null() {
                return None;
            }
            // SAFETY: sym_ptr is non-null (checked above) and names the live thread-local SymbolTable; the &mut is unique under the single-threaded GIL.
            let symbols = unsafe { &mut *sym_ptr };
            // SAFETY: reg_ptr is non-null (checked above) and names the live thread-local EnumRegistry; the &mut is unique under the single-threaded GIL.
            let registry = unsafe { &mut *reg_ptr };

            let qname = catnip_core::symbols::qualified_name(enum_name, variant_name);

            // Already registered?
            if let Some(idx) = symbols.lookup(&qname) {
                return Some(idx);
            }

            // Not known yet -- register the whole enum type with this single variant.
            // If the type already exists, just add the variant.
            if let Some(ety) = registry.find_by_name(enum_name) {
                // Type exists, check if variant is already there
                if let Some(sym_id) = ety.variant_symbol(variant_name) {
                    return Some(sym_id);
                }
            }

            // Register as a new type with just this variant (others will be added lazily)
            // This is a best-effort approach for cross-VM imports.
            let sym_id = symbols.intern(&qname);
            // Register type if needed, reusing existing if present
            if registry.find_by_name(enum_name).is_none() {
                registry.register(enum_name, &[variant_name.to_string()], symbols);
            }
            Some(sym_id)
        })
    })
}

/// Incref a struct instance via the thread-local registry.
/// No-op if registry is not installed.
pub fn struct_registry_incref(idx: u32) {
    STRUCT_REGISTRY.with(|cell| {
        let ptr = cell.get();
        if !ptr.is_null() {
            // SAFETY: ptr is non-null (checked above) and names the live thread-local StructRegistry; single-threaded under the GIL.
            let registry = unsafe { &*ptr };
            registry.incref(idx);
        }
    });
}

// ---------------------------------------------------------------------------
// Registry identity table -- proxies resolve *their own* registry, not the TL
// ---------------------------------------------------------------------------

thread_local! {
    /// Maps every live `StructRegistry` id to its address. A Python
    /// `CatnipStruct` proxy outlives the execution that created it, so it
    /// cannot trust the set-and-leave `STRUCT_REGISTRY` thread-local (which may
    /// name a sibling VM's registry). It stores its origin registry id and
    /// resolves through this table instead. Entries are added when a proxy is
    /// materialized and removed by `StructRegistry::drop`, so a present pointer
    /// always names a live registry. See [`save_struct_registry`].
    static REGISTRY_TABLE: std::cell::RefCell<std::collections::HashMap<u64, *const StructRegistry>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Record a registry's address under its id (idempotent). Called when a proxy
/// is materialized, so any registry a proxy can reach is resolvable.
pub fn register_struct_registry(id: u64, ptr: *const StructRegistry) {
    if id == 0 || ptr.is_null() {
        return;
    }
    REGISTRY_TABLE.with(|t| {
        t.borrow_mut().insert(id, ptr);
    });
}

/// Forget a registry's address. Called from `StructRegistry::drop` so a stale
/// pointer is never resolved. Uses `try_with` because a registry may be dropped
/// during thread teardown, after this thread-local is already destroyed.
pub fn unregister_struct_registry(id: u64) {
    if id == 0 {
        return;
    }
    let _ = REGISTRY_TABLE.try_with(|t| {
        t.borrow_mut().remove(&id);
    });
}

/// Run `f` against the registry identified by `id`, if it is still alive.
/// The table pointer is copied out before `f` runs so the entry borrow does
/// not overlap a reentrant decref cascade.
///
/// Passes a SHARED `&StructRegistry`: the registry's mutable state lives behind
/// a `RefCell`, so a reentrant call here (a pyobj `__del__` dropping another
/// proxy of the same registry mid-cascade) takes another shared borrow instead
/// of a second `&mut *ptr` -- the old aliasing UB is gone by construction.
fn with_proxy_registry<R>(id: u64, f: impl FnOnce(&StructRegistry) -> R) -> Option<R> {
    if id == 0 {
        return None;
    }
    // `try_with`: a proxy can decref during thread teardown, after this
    // thread-local is destroyed (same guard as `unregister_struct_registry`).
    // Reachable since `StructRegistry::drop` now releases recorded proxies.
    let ptr = REGISTRY_TABLE
        .try_with(|t| t.borrow().get(&id).copied())
        .ok()
        .flatten()?;
    // SAFETY: the entry is removed by `StructRegistry::drop`, so a present
    // pointer names a live registry. Single-threaded under the GIL. The shared
    // `&*ptr` never aliases a `&mut` -- all registry mutation goes through the
    // interior `RefCell`.
    Some(f(unsafe { &*ptr }))
}

/// Incref a struct slot in the proxy's own registry (no-op if it is gone).
pub fn proxy_registry_incref(registry_id: u64, idx: u32) {
    with_proxy_registry(registry_id, |reg| reg.incref(idx));
}

/// Freeze a struct slot in the proxy's own registry (no-op if it is gone).
pub fn proxy_registry_freeze(registry_id: u64, idx: u32) {
    with_proxy_registry(registry_id, |reg| reg.freeze(idx));
}

/// True if the slot is frozen in the proxy's own registry.
pub fn proxy_registry_is_frozen(registry_id: u64, idx: u32) -> bool {
    with_proxy_registry(registry_id, |reg| reg.is_frozen(idx)).unwrap_or(false)
}

/// Decref a struct slot in the proxy's own registry, releasing the reference a
/// `CatnipStructProxy` held. No-op if the registry is already gone (a proxy
/// outliving its broadcast child) or the slot was already freed. Cascades into
/// struct fields when the slot reaches zero.
pub fn proxy_registry_decref(registry_id: u64, idx: u32) {
    with_proxy_registry(registry_id, |reg| super::structs::decref_slot(reg, idx));
}

/// Run `f` with the registry named by `id` installed as the thread-local
/// `STRUCT_REGISTRY`, restoring the previous one afterwards. `id == 0` or a
/// registry already gone installs nothing (f runs against the ambient
/// thread-local).
///
/// A Python-facing proxy that converts values (`from_pyobject` on write) must
/// index struct instances into ITS OWN registry, not whatever registry a prior
/// execution left in the set-and-leave thread-local -- otherwise the stored
/// `TAG_STRUCT` carries an index from a foreign registry, and a later
/// struct-aware release against the proxy's id would decref an unrelated
/// instance. Same discipline as `Executor::export_to_pydict`.
pub fn with_struct_registry_installed<R>(id: u64, f: impl FnOnce() -> R) -> R {
    let ptr = if id == 0 {
        None
    } else {
        REGISTRY_TABLE.try_with(|t| t.borrow().get(&id).copied()).ok().flatten()
    };
    match ptr {
        Some(ptr) => {
            let prev = save_struct_registry();
            set_struct_registry(ptr as *const _);
            let r = f();
            restore_struct_registry(prev);
            r
        }
        None => f(),
    }
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
        let ptr = obj.as_ptr() as usize;
        let idx = if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = Some(ObjSlot { obj, refcount: 1 });
            idx
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Some(ObjSlot { obj, refcount: 1 }));
            idx
        };
        table_trace::on_insert(idx, ptr);
        idx
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
        if table_trace::active() {
            table_trace::on_clone(idx, slot.obj.as_ptr() as usize);
        }
        slot.refcount += 1;
    }

    /// Decrement the handle refcount; when it reaches 0 the slot is freed and
    /// the `Py<PyAny>` is returned so the CALLER drops it after the table lock
    /// is released -- a Python decref can run arbitrary code (`__del__`, GC)
    /// that re-enters this table, and the Mutex is not reentrant.
    #[inline]
    pub fn release_handle(&mut self, idx: u32) -> Option<Py<PyAny>> {
        let slot = self.slots[idx as usize].as_mut().expect("ObjectTable: dead handle");
        if table_trace::active() {
            table_trace::on_release(idx, slot.obj.as_ptr() as usize);
        }
        slot.refcount -= 1;
        if slot.refcount == 0 {
            self.free_list.push(idx);
            return self.slots[idx as usize].take().map(|s| s.obj);
        }
        None
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

/// Surface, for CPython's cyclic GC, the `Py<PyAny>` a PyObject-handle `Value`
/// points to. The global `OBJECT_TABLE` holds a strong reference on behalf of
/// every PyObject handle, and that reference is invisible to the collector. A
/// pyclass that owns such handles (e.g. the VM globals kept alive by
/// `ImportLoader`) must report them from `__traverse__`, or the objects they
/// pin -- and any reference cycle running through them -- can never be
/// collected. No-op for non-PyObject values. `try_lock` keeps it panic-free if
/// a re-entrant GC fires while the table is momentarily borrowed.
pub fn visit_obj_handle(value: Value, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
    if let Some(handle) = value.as_obj_handle() {
        if let Ok(table) = OBJECT_TABLE.try_lock() {
            if let Some(Some(slot)) = table.slots.get(handle as usize) {
                visit.call(&slot.obj)?;
            }
        }
    }
    Ok(())
}

/// Surface a collection of handle-bearing `Value`s for the cyclic GC,
/// deduplicating by handle index. An `ObjectTable` slot owns a **single** strong
/// `Py` reference shared by every `Value` handle that points to it (a pyobj
/// bound under several names -- `v = P` -- yields the same index more than
/// once). Reporting that one reference once per handle over-counts it; the
/// resulting negative `gc_refs` makes the collector treat the object as
/// externally rooted, pinning it and every cycle through it. Visiting each
/// distinct index exactly once matches the one reference the table actually
/// owns. This is the only correct way to traverse a `Value` collection -- a
/// bare `for v in .. { visit_obj_handle(v) }` loop reintroduces the over-count.
pub fn visit_obj_handles<I>(values: I, visit: &PyVisit<'_>) -> Result<(), PyTraverseError>
where
    I: IntoIterator<Item = Value>,
{
    let iter = values.into_iter();
    let mut seen = std::collections::HashSet::with_capacity(iter.size_hint().0);
    for value in iter {
        if let Some(idx) = value.as_obj_handle() {
            if seen.insert(idx) {
                visit_obj_handle(value, visit)?;
            }
        }
    }
    Ok(())
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
/// Debug: count live slots pointing at `ptr` and their total handle refcount.
/// Returns `(slot_count, total_refcount)`. Leak-hunt only.
pub fn debug_slots_for_ptr(ptr: usize) -> (usize, u32) {
    with_object_table_ref(|table| {
        let mut n = 0;
        let mut rc = 0;
        for slot in table.slots.iter().flatten() {
            if slot.obj.as_ptr() as usize == ptr {
                n += 1;
                rc += slot.refcount;
            }
        }
        (n, rc)
    })
}

fn with_object_table_ref<R>(f: impl FnOnce(&ObjectTable) -> R) -> R {
    f(&OBJECT_TABLE.lock().unwrap())
}

/// Ledger probe companion: (idx, refcount, truncated repr) of every live
/// OBJECT_TABLE slot, so a non-zero slot delta can be attributed without
/// knowing the leaked object's identity in advance (TBLT needs a ptr).
///
/// The repr/get_type work runs OUTSIDE the table lock: `repr()` can run
/// arbitrary Python (a `__repr__`, or an allocation that triggers cyclic GC ->
/// `PyPipeline::__clear__` -> `Value::decref` -> `OBJECT_TABLE.lock()`), and
/// the mutex is not reentrant -- doing it under the lock self-deadlocks. So
/// snapshot `(idx, rc, owned handle)` under the lock, drop it, then format.
pub fn debug_live_slot_types(py: Python<'_>) -> Vec<(u32, u32, String)> {
    let snapshot: Vec<(u32, u32, Py<PyAny>)> = with_object_table_ref(|table| {
        table
            .slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.as_ref().map(|s| (idx as u32, s.refcount, s.obj.clone_ref(py))))
            .collect()
    });
    snapshot
        .into_iter()
        .map(|(idx, rc, obj)| {
            let bound = obj.bind(py);
            let ty = bound
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "<?>".into());
            let mut r = bound
                .repr()
                .map(|r| r.to_string())
                .unwrap_or_else(|_| "<repr failed>".into());
            r.truncate(120);
            (idx, rc, format!("{ty}: {r}"))
        })
        .collect()
}

/// Ledger probe: live counts of the manually refcounted heap classes --
/// (occupied `OBJECT_TABLE` slots, their summed handle refcounts, live BigInt
/// allocations, live Complex allocations, live struct instance slots). The
/// refcount-ledger tests assert a zero delta of this tuple across a full
/// pipeline lifecycle -- and, for struct instances (whose registry dies with
/// the pipeline), across repeated runs on a REUSED pipeline; a leaked operand
/// release surfaces here as an exact per-class count instead of an RSS
/// threshold.
pub fn debug_live_counts() -> (usize, u64, isize, isize, isize) {
    let (slots, refs) = with_object_table_ref(|table| {
        let mut n = 0usize;
        let mut rc = 0u64;
        for slot in table.slots.iter().flatten() {
            n += 1;
            rc += slot.refcount as u64;
        }
        (n, rc)
    });
    (
        slots,
        refs,
        LIVE_BIGINT.load(std::sync::atomic::Ordering::Relaxed),
        LIVE_COMPLEX.load(std::sync::atomic::Ordering::Relaxed),
        crate::vm::structs::LIVE_STRUCT_INSTANCES.load(std::sync::atomic::Ordering::Relaxed),
    )
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

    /// Iterate mutably over all slots.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut FuncSlot> {
        self.slots.iter_mut()
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
///
/// Hazard -- read before reusing this pattern. `func_table` (like
/// `struct_registry`, unlike `symbol_table`/`enum_registry`) is set-and-leave:
/// `execute*` installs it but never restores the previous one on return, so the
/// pointer can outlive the `Executor` it names. A non-null leftover is normally
/// still a live owner because `Executor::drop` clears the TL only when it still
/// names that VM's table (clears-if-mine), nulling it when the owner is dropped.
///
/// The exception that caused a use-after-free: a second top-level `execute`
/// captured this leftover into a local before overwriting the TL with its own,
/// then wrote through it later (the import func_table transplant). There the
/// clears-if-mine guard cannot help -- the live copy is in a local, not the TL
/// -- and the sibling `Executor` (Python-owned, GC-eligible, not on the stack)
/// can be freed mid-run. Rule for any caller that captures this pointer at a
/// possible top level and dereferences it later: only trust it when
/// `vm_depth() > 0`, i.e. a genuine parent hosting the nested call is suspended
/// on the stack and therefore alive. Mid-dispatch readers (broadcast callbacks
/// in `frame.rs`, ND workers in `host.rs`) are already in that regime.
pub fn save_func_table() -> *const FunctionTable {
    FUNC_TABLE.with(|cell| cell.get())
}

/// Restore a previously saved FunctionTable pointer.
pub fn restore_func_table(ptr: *const FunctionTable) {
    FUNC_TABLE.with(|cell| cell.set(ptr));
}

thread_local! {
    static VM_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Number of VM dispatch loops currently active on this thread.
///
/// `func_table` is installed by `execute*` and never restored on return
/// (unlike `symbol_table`/`enum_registry`), so its thread-local pointer can
/// outlive the `Executor` it names. A genuine parent VM hosting an `import`
/// is always suspended in its own dispatch on the stack, so depth > 0 marks
/// the only case where reading `save_func_table()` yields a live parent;
/// at depth 0 the pointer may be a stale leftover the GC is about to free.
pub fn vm_depth() -> u32 {
    VM_DEPTH.with(|cell| cell.get())
}

/// RAII guard that increments the VM dispatch depth for its lifetime.
pub struct VmDepthGuard;

impl VmDepthGuard {
    pub fn enter() -> Self {
        VM_DEPTH.with(|cell| cell.set(cell.get() + 1));
        VmDepthGuard
    }
}

impl Drop for VmDepthGuard {
    fn drop(&mut self) {
        VM_DEPTH.with(|cell| cell.set(cell.get().saturating_sub(1)));
    }
}

// PyO3-specific tags (catnip_rs only)
const TAG_PYOBJ: u64 = 4 << TAG_SHIFT;
const TAG_STRUCT: u64 = 5 << TAG_SHIFT;
const TAG_COMPLEX: u64 = 8 << TAG_SHIFT;

/// Native complex number for NaN-box storage.
///
/// Construct via [`NativeComplex::new`] so the live ledger stays exact
/// (`Drop` decrements it).
pub struct NativeComplex(pub f64, pub f64);

impl NativeComplex {
    #[inline]
    pub fn new(real: f64, imag: f64) -> Self {
        LIVE_COMPLEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        NativeComplex(real, imag)
    }
}

impl Drop for NativeComplex {
    fn drop(&mut self) {
        LIVE_COMPLEX.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// NaN-boxed value. Fits in 8 bytes.
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
        <Self as ScalarValue>::scalar_from_float(f)
    }

    /// Create a small integer value. Panics if out of 48-bit range.
    #[inline]
    pub fn from_int(i: i64) -> Self {
        <Self as ScalarValue>::scalar_from_int(i)
    }

    /// Safe creation from any i64. Uses SmallInt when in range, BigInt otherwise.
    #[inline]
    pub fn from_i64(i: i64) -> Self {
        Self::try_from_int(i).unwrap_or_else(|| Self::from_bigint(Integer::from(i)))
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
        <Self as ScalarValue>::scalar_from_bigint(n)
    }

    /// Create a BigInt or demote to SmallInt if it fits.
    #[inline]
    pub fn from_bigint_or_demote(n: Integer) -> Self {
        <Self as ScalarValue>::scalar_from_bigint_or_demote(n)
    }

    /// Create a complex number value.
    #[inline]
    pub fn from_complex(real: f64, imag: f64) -> Self {
        let arc = Arc::new(NativeComplex::new(real, imag));
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(
            ptr & !PAYLOAD_MASK == 0,
            "NativeComplex Arc pointer exceeds 47-bit address space"
        );
        Value(QNAN_BASE | TAG_COMPLEX | (ptr & PAYLOAD_MASK))
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

    /// Boundary-safe reconstruction of a JIT-produced word. The Cranelift
    /// codegen only boxes inline scalars (int/float/bool), so a scalar passes
    /// through; any pointer/index tag -- which the codegen never emits -- maps
    /// to the INVALID canary instead of reconstructing an unvalidated pointer.
    /// A codegen regression then surfaces as a contained canary (caught by the
    /// `to_pyobject` debug assert) rather than a silent deref of garbage.
    /// See `catnip_core::nanbox::from_raw_scalar` / `CatnipBoundaryProof`.
    #[inline]
    pub fn from_raw_scalar(bits: u64) -> Self {
        match catnip_core::nanbox::from_raw_scalar(bits) {
            Some(b) => Value(b),
            None => Value::INVALID,
        }
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
        ScalarValue::scalar_is_bigint(self)
    }

    /// Check if this is a native VM function.
    #[inline]
    pub fn is_vmfunc(self) -> bool {
        ScalarValue::scalar_is_vmfunc(self)
    }

    /// Check if this is the INVALID canary (debug-only sentinel for uninitialized slots).
    #[inline]
    pub fn is_invalid(self) -> bool {
        self.0 == Self::INVALID.0
    }

    /// Check if this is a complex number.
    #[inline]
    pub fn is_complex(self) -> bool {
        (self.0 & (0x7FF8_0000_0000_0000 | TAG_MASK)) == (QNAN_BASE | TAG_COMPLEX)
    }

    /// Extract (real, imag) parts from a complex value.
    ///
    /// # Safety
    /// Caller must ensure the Arc is still alive.
    #[inline]
    pub unsafe fn as_complex_parts(&self) -> Option<(f64, f64)> {
        if self.is_complex() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeComplex;
            let c = &*ptr;
            Some((c.0, c.1))
        } else {
            None
        }
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
            // SAFETY: the BigInt tag was checked, so the payload is a live Arc<GmpInt>; this increment is balanced by a matching decref.
            unsafe { Arc::increment_strong_count(ptr) };
        } else if self.is_complex() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeComplex;
            // SAFETY: the complex tag was checked, so the payload is a live Arc<NativeComplex>; this increment is balanced by a matching decref.
            unsafe { Arc::increment_strong_count(ptr) };
        } else if self.is_pyobj() {
            let handle = (self.0 & PAYLOAD_MASK) as u32;
            with_object_table(|table| table.clone_handle(handle));
        } else if self.is_struct_instance() {
            let idx = self.as_struct_instance_idx().unwrap();
            struct_registry_incref(idx);
        }
    }

    /// Increment refcount for Arc-backed values (BigInt, Complex).
    /// Used in Load opcodes to track shared references without affecting PyObject refcounting.
    #[inline]
    pub fn clone_refcount_bigint(self) {
        if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
            // SAFETY: the BigInt tag was checked, so the payload is a live Arc<GmpInt>; this increment is balanced by a matching decref.
            unsafe { Arc::increment_strong_count(ptr) };
        } else if self.is_complex() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeComplex;
            // SAFETY: the complex tag was checked, so the payload is a live Arc<NativeComplex>; this increment is balanced by a matching decref.
            unsafe { Arc::increment_strong_count(ptr) };
        }
    }

    /// Increment the ObjectTable handle refcount for a PyObject value; no-op
    /// otherwise. The PyObject counterpart of [`clone_refcount_bigint`], for
    /// callers that already manage struct refcounts via the local registry but
    /// must keep a duplicated PyObject handle alive on its own (e.g. a struct
    /// field returned by `GetAttr`, which must survive the receiver's decref).
    #[inline]
    pub fn clone_refcount_pyobj(self) {
        if self.is_pyobj() {
            let handle = (self.0 & PAYLOAD_MASK) as u32;
            with_object_table(|table| table.clone_handle(handle));
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
        } else if self.is_complex() {
            // SAFETY: is_complex() was checked above, so the payload is a live Arc<NativeComplex> owned by self; the borrow does not outlive it.
            let (r, i) = unsafe { self.as_complex_parts().unwrap() };
            r != 0.0 || i != 0.0
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
            // SAFETY: is_bigint() was checked above, so the payload is a live Arc<GmpInt> owned by self; the borrow does not outlive it.
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
pub(crate) fn pyobject_to_integer(obj: &Bound<'_, PyAny>) -> PyResult<Integer> {
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

        if let Ok(c) = obj.cast::<PyComplex>() {
            return Ok(Value::from_complex(c.real(), c.imag()));
        }

        // Recognize VMFunction -> restore native TAG_VMFUNC for VM round-trip.
        if let Ok(vmfunc) = obj.cast::<VMFunction>() {
            if let Some(idx) = vmfunc.borrow().func_table_idx {
                let is_owned = FUNC_TABLE.with(|cell| {
                    let ptr = cell.get();
                    if ptr.is_null() {
                        return false;
                    }
                    // SAFETY: ptr is non-null (checked above) and names the live thread-local FunctionTable; single-threaded under the GIL, borrow does not outlive this closure.
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
            let proxy_frozen = proxy.borrow().frozen;
            let proxy_registry_id = proxy.borrow().native_registry_id;
            if let Some(idx) = proxy.borrow().native_instance_idx {
                let restored = STRUCT_REGISTRY.with(|cell| {
                    let ptr = cell.get();
                    if ptr.is_null() {
                        return false;
                    }
                    // SAFETY: ptr is non-null (checked above) and names the live thread-local StructRegistry; single-threaded under the GIL.
                    let registry = unsafe { &*ptr };
                    // Trust the proxy's idx only within its own clone lineage: the
                    // proxy's registry or a clone of it (broadcast child) shares the
                    // index space, so a live slot is the same instance. A foreign
                    // registry may hold an unrelated instance at that index, so fall
                    // through to value re-creation below.
                    if registry.origin_id() == proxy_registry_id && registry.has_instance(idx) {
                        registry.incref(idx);
                        // If the proxy was hashed (possibly outside the VM), the
                        // native slot must also become frozen so VM SetAttr can't
                        // mutate behind the proxy's back.
                        if proxy_frozen {
                            registry.freeze(idx);
                        }
                        true
                    } else {
                        false
                    }
                });
                if restored {
                    return Ok(Value::from_struct_instance(idx));
                }
            }
            // Orphaned proxy from a child VM -- re-register in current registry.
            // Same pattern as lazy_register_enum_variant below.
            // Step 1: look up type_id (read-only registry access, no aliasing)
            let p = proxy.borrow();
            let type_name = p.type_name.clone();
            let old_native_idx = p.native_instance_idx;
            let field_py_values: Vec<pyo3::Py<PyAny>> = p.field_values.iter().map(|v| v.clone_ref(_py)).collect();
            drop(p);

            let type_id = STRUCT_REGISTRY.with(|cell| {
                let ptr = cell.get();
                if ptr.is_null() {
                    return None;
                }
                // SAFETY: ptr is non-null (checked above) and names the live thread-local StructRegistry; read-only access, single-threaded under the GIL.
                let registry = unsafe { &*ptr };
                registry.find_type_by_name(&type_name).map(|ty| ty.id)
            });

            if let Some(type_id) = type_id {
                // Step 2: convert fields recursively (may re-enter from_pyobject)
                let mut fields = Vec::with_capacity(field_py_values.len());
                for fv in &field_py_values {
                    fields.push(Value::from_pyobject(_py, fv.bind(_py))?);
                }
                // Step 3: create instance (mutable registry access, no aliasing).
                // The proxy now belongs to the current registry, so retarget its
                // identity and make that registry reachable by its later Drop.
                let (new_idx, new_registry_id) = STRUCT_REGISTRY.with(|cell| {
                    let ptr = cell.get();
                    // SAFETY: reaching this branch means type_id was resolved through the same non-null thread-local StructRegistry (checked above), still installed on this thread; single-threaded under the GIL. A shared `&` (never `&mut`) so a reentrant `with_proxy_registry` on the same registry cannot alias an exclusive borrow -- all mutation goes through the interior RefCell.
                    let registry = unsafe { &*ptr };
                    let idx = registry.create_instance(type_id, fields);
                    if proxy_frozen {
                        registry.freeze(idx);
                    }
                    // create_instance gives rc=1 for the returned VM value; the
                    // retargeted proxy claims its own ref, released by its Drop.
                    registry.incref(idx);
                    let rid = registry.id();
                    register_struct_registry(rid, ptr);
                    (idx, rid)
                });
                // The proxy abandons its old slot for the freshly created one.
                // Release the reference it still held on its previous registry: a
                // live sibling (a separate VM, or a nested execution) would
                // otherwise keep that slot pinned for its whole life. No-op if the
                // old registry is already gone (table miss) or the slot was freed.
                if let Some(old_idx) = old_native_idx {
                    proxy_registry_decref(proxy_registry_id, old_idx);
                }
                {
                    let mut pm = proxy.borrow_mut();
                    pm.native_instance_idx = Some(new_idx);
                    pm.native_registry_id = new_registry_id;
                }
                return Ok(Value::from_struct_instance(new_idx));
            }
        }

        // Recognize CatnipEnumVariant -> restore native TAG_SYMBOL for VM round-trip.
        // If the symbol isn't in the current table (e.g. from an imported module),
        // lazily register it so cross-VM enum variants resolve correctly.
        if let Ok(variant) = obj.cast::<super::enums::CatnipEnumVariant>() {
            let v = variant.borrow();
            let qname = catnip_core::symbols::qualified_name(&v.enum_name, &v.variant_name);
            if let Some(sym_id) = resolve_symbol_by_name(&qname) {
                return Ok(Value::from_symbol(sym_id));
            }
            if let Some(sym_id) = lazy_register_enum_variant(&v.enum_name, &v.variant_name) {
                return Ok(Value::from_symbol(sym_id));
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
            // SAFETY: is_bigint() was checked above, so the payload is a live Arc<GmpInt> owned by self; the borrow does not outlive it.
            let n = unsafe { self.as_bigint_ref().unwrap() };
            integer_to_pyobject(py, n)
        } else if self.is_struct_instance() {
            let idx = self.as_struct_instance_idx().unwrap();
            STRUCT_REGISTRY.with(|cell| {
                let ptr = cell.get();
                if ptr.is_null() {
                    return py.None();
                }
                // SAFETY: ptr is non-null (checked above) and names the live thread-local StructRegistry; single-threaded under the GIL.
                let registry = unsafe { &*ptr };
                registry.instance_to_pyobject(py, idx).unwrap_or_else(|_| py.None())
            })
        } else if self.is_complex() {
            // SAFETY: is_complex() was checked above, so the payload is a live Arc<NativeComplex> owned by self; the borrow does not outlive it.
            let (r, i) = unsafe { self.as_complex_parts().unwrap() };
            PyComplex::from_doubles(py, r, i).into_any().unbind()
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
                // SAFETY: ptr is non-null (checked above) and names the live thread-local FunctionTable; single-threaded under the GIL.
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
            // Symbol (enum variant) - resolve to opaque CatnipEnumVariant
            let idx = self.as_symbol().unwrap_or(0);
            match resolve_symbol(idx) {
                Some(qualified) => {
                    // qualified = "EnumName.variant"
                    if let Some(dot) = qualified.find('.') {
                        let enum_name = qualified[..dot].to_string();
                        let variant_name = qualified[dot + 1..].to_string();
                        let variant = match union_nullary_methods_for(&qualified) {
                            Some(methods) => super::enums::CatnipEnumVariant::new_with_methods(
                                enum_name,
                                variant_name,
                                qualified,
                                methods,
                            ),
                            None => super::enums::CatnipEnumVariant::new_from_parts(enum_name, variant_name, qualified),
                        };
                        Py::new(py, variant).unwrap().into_any()
                    } else {
                        qualified.into_pyobject(py).unwrap().unbind().into_any()
                    }
                }
                None => (idx as i64).into_pyobject(py).unwrap().unbind().into_any(),
            }
        }
    }

    /// Decrement refcount for a PyObject, BigInt, or Complex value. **NO-OP on a
    /// TAG_STRUCT** (a struct instance is refcounted in a `StructRegistry`, out
    /// of reach of a bare `Value`). That silent no-op is a leak footgun: a
    /// stack / local / field value that COULD be a struct must be released with
    /// `core::decref_discard(registry, v)` (struct-aware), never this method.
    /// Call this directly ONLY when the value is provably non-struct (e.g. a
    /// BigInt arithmetic intermediate) or from a scoped helper that already split
    /// off the struct case (`decref_discard`, `decref_pyobj`).
    pub fn decref(self) {
        if self.is_pyobj() {
            let handle = (self.0 & PAYLOAD_MASK) as u32;
            let released = with_object_table(|table| table.release_handle(handle));
            // Dropped outside the table lock (see release_handle).
            drop(released);
        } else if self.is_bigint() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const GmpInt;
            // SAFETY: the BigInt tag was checked, so the payload is a live Arc<GmpInt>; this decrement balances the incref made when the Value was duplicated.
            unsafe { Arc::decrement_strong_count(ptr) };
        } else if self.is_complex() {
            let ptr = (self.0 & PAYLOAD_MASK) as *const NativeComplex;
            // SAFETY: the complex tag was checked, so the payload is a live Arc<NativeComplex>; this decrement balances the incref made when the Value was duplicated.
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
            // SAFETY: the BigInt tag was checked, so the payload is a live Arc<GmpInt>; this decrement balances a prior incref.
            unsafe { Arc::decrement_strong_count(ptr) };
        }
    }

    /// Test-only: strong count of the underlying BigInt Arc, read without
    /// touching it (`from_raw`/`into_raw` round-trip is refcount-neutral).
    /// Mirrors the catnip_vm probe of the same name.
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
            // SAFETY: is_bigint() was checked above, so the payload is a live Arc<GmpInt> owned by self; the borrow does not outlive it.
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
            // SAFETY: both operands' BigInt tags were checked above, so each payload is a live Arc<GmpInt>; the borrows do not outlive them.
            unsafe { self.as_bigint_ref().unwrap() == other.as_bigint_ref().unwrap() }
        } else if self.is_complex() && other.is_complex() {
            // SAFETY: both operands' complex tags were checked above, so each payload is a live Arc<NativeComplex>; the borrows do not outlive them.
            unsafe {
                let (ar, ai) = self.as_complex_parts().unwrap();
                let (br, bi) = other.as_complex_parts().unwrap();
                ar == br && ai == bi
            }
        } else {
            self.0 == other.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catnip_core::nanbox::{SMALLINT_MAX, SMALLINT_MIN, TAG_BIGINT, TAG_SMALLINT, TAG_SYMBOL};

    // Conformance mirror for the boundary lock (CatnipBoundaryProof): this
    // crate's scalar tags are exactly {0,1,2,3}, and from_raw_scalar rejects
    // every index/pointer-backed tag this Value uses.
    #[test]
    fn boundary_classification_conforms() {
        use catnip_core::nanbox::{from_raw_scalar, is_scalar_tag};
        let tagged = |tag: u64, p: u64| QNAN_BASE | tag | (p & PAYLOAD_MASK);

        // Scalar tags and floats pass.
        for tag in [TAG_SMALLINT, TAG_BOOL, TAG_NIL, TAG_SYMBOL] {
            let w = tagged(tag, 1);
            assert!(is_scalar_tag(tag >> TAG_SHIFT));
            assert_eq!(from_raw_scalar(w), Some(w));
        }
        let fb = 2.5f64.to_bits();
        assert_eq!(from_raw_scalar(fb), Some(fb));

        // Index (PyObj, Struct, VMFunc) and Pointer (BigInt, Complex) rejected.
        for tag in [TAG_PYOBJ, TAG_STRUCT, TAG_VMFUNC, TAG_BIGINT, TAG_COMPLEX] {
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
            // SAFETY: v is a live BigInt created just above and not yet decref'd, so its Arc is alive.
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
            // SAFETY: back is a live BigInt created just above and not yet decref'd, so its Arc is alive.
            assert_eq!(unsafe { back.as_bigint_ref().unwrap() }, &n);
            v.decref();
            back.decref();
        });
    }
}
