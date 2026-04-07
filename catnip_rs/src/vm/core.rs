// FILE: catnip_rs/src/vm/core.rs
//! Catnip Virtual Machine with O(1) dispatch via Rust match.
//!
//! Stack-based VM that executes bytecode without growing the Python stack.

use super::OpCode;
use super::enums::{CatnipEnumType, EnumRegistry};
use super::frame::{CodeObject, Frame, FramePool, NativeClosureScope, PyCodeObject, VMFunction};
use super::host::{BinaryOp, VmHost};
use super::iter::SeqIter;
use super::pattern::{VMPattern, VMPatternElement};
use super::py_interop::{
    PyResultExt, cast_tuple, convert_code_object, portabilize_struct_values, tuple_extract, tuple_get,
};
use super::structs::{
    CatnipStructType, MethodKey, StructField, StructMethods, StructParents, StructRegistry, StructType, StructTypeId,
    cascade_decref_fields,
};
use super::traits::{TraitDef, TraitField, TraitRegistry};
use super::value::resolve_symbol_by_name;
use super::value::{FuncSlot, FunctionTable, Value};
use crate::constants::*;
use crate::jit::builtin_dispatch::builtin_name_to_id;
use crate::jit::{HotLoopDetector, JITExecutor, TraceOp, TraceRecorder};
use catnip_core::symbols::{SymbolTable, qualified_name};
use indexmap::IndexMap;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};
use rug::Integer;
use rug::ops::{DivRounding, Pow, RemRounding};
use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Resolve a captured variable's integer value for JIT guard validation.
/// Searches: closure scope -> host globals -> VM globals.
#[inline]
fn resolve_jit_guard_value(
    py: Python<'_>,
    name: &str,
    closure: &Option<NativeClosureScope>,
    host: &(impl VmHost + ?Sized),
    globals: &IndexMap<String, Value>,
) -> Option<i64> {
    if let Some(ref closure) = closure {
        if let Some(val) = closure.resolve_with_py(py, name).and_then(|v| v.as_int()) {
            return Some(val);
        }
    }
    if let Some(val) = host.lookup_global(py, name).ok().flatten().and_then(|v| v.as_int()) {
        return Some(val);
    }
    globals.get(name).and_then(|v| v.as_int())
}

/// Decref a Value being discarded (PopTop, StoreLocal overwrite, SetAttr old value).
/// Handles BigInt (Arc) and Struct (registry slot + field cascade).
/// PyObj is managed by Python GC -- not touched here.
#[inline]
fn decref_discard(registry: &mut StructRegistry, val: Value) {
    if val.is_bigint() {
        val.decref_bigint();
    } else if val.is_complex() {
        val.decref();
    } else if val.is_struct_instance() {
        let idx = val.as_struct_instance_idx().unwrap();
        if let Some(fields) = registry.decref(idx) {
            cascade_decref_fields(registry, fields);
        }
    }
}

/// Check abstract struct guard. Returns Err if struct has unimplemented abstract methods.
#[inline]
fn check_abstract_guard(registry: &StructRegistry, type_id: StructTypeId) -> VMResult<()> {
    let ty = registry.get_type(type_id).unwrap();
    if !ty.abstract_methods.is_empty() {
        let mut names: Vec<&str> = ty.abstract_methods.iter().map(|k| k.name.as_str()).collect();
        names.sort();
        return Err(VMError::RuntimeError(format!(
            "cannot instantiate abstract struct '{}' (unimplemented: {})",
            ty.name,
            names.iter().map(|n| format!("'{}'", n)).collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(())
}

/// Build an error message for a missing attribute on a struct, with "did you mean?" suggestion.
fn attr_error_msg(ty: &StructType, attr: &str) -> String {
    let candidates = ty.available_names();
    let candidates_ref: Vec<&str> = candidates.to_vec();
    let suggestions = catnip_tools::suggest::suggest_similar(attr, &candidates_ref, 1, 0.6);
    match catnip_tools::suggest::format_suggestion(&suggestions) {
        Some(hint) => format!("'{}' has no attribute '{}'. {}", ty.name, attr, hint),
        None => format!("'{}' has no attribute '{}'", ty.name, attr),
    }
}

/// Safely index into code.names with bounds check.
#[inline(always)]
fn get_name(code: &CodeObject, arg: u32) -> Result<&String, VMError> {
    let idx = arg as usize;
    code.names
        .get(idx)
        .ok_or_else(|| VMError::RuntimeError(format!("invalid name index {} (names len={})", idx, code.names.len())))
}

/// Build an error message for a missing attribute on a Python object, via dir().
pub(crate) fn py_attr_error_msg(py_bound: &Bound<'_, PyAny>, attr: &str, original_msg: &str) -> String {
    if let Ok(dir_list) = py_bound.dir() {
        let candidates: Vec<String> = dir_list
            .iter()
            .filter_map(|item| item.extract::<String>().ok())
            .collect();
        let refs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
        let suggestions = catnip_tools::suggest::suggest_similar(attr, &refs, 1, 0.6);
        if let Some(hint) = catnip_tools::suggest::format_suggestion(&suggestions) {
            let base = original_msg.strip_prefix("AttributeError: ").unwrap_or(original_msg);
            return format!("AttributeError: {base}. {hint}");
        }
    }
    original_msg.to_string()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VMFallbackStats {
    pub py_binary_div: u64,
    pub py_binary_floordiv: u64,
    pub py_binary_mod: u64,
    pub py_compare_eq: u64,
    pub py_compare_ne: u64,
    pub py_pattern_literal_eq: u64,
}

/// Global generation counter for StoreScope mutations across all VM instances.
/// Used to detect re-entrant modifications (VMFunction called from Python creates
/// a new VM that shares ctx_globals but not self.globals).
static GLOBALS_GEN: AtomicU64 = AtomicU64::new(0);

static PY_BINARY_DIV_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_BINARY_FLOORDIV_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_BINARY_MOD_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_COMPARE_EQ_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_COMPARE_NE_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static PY_PATTERN_LITERAL_EQ_FALLBACKS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn inc(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

pub fn reset_vm_fallback_stats() {
    PY_BINARY_DIV_FALLBACKS.store(0, Ordering::Relaxed);
    PY_BINARY_FLOORDIV_FALLBACKS.store(0, Ordering::Relaxed);
    PY_BINARY_MOD_FALLBACKS.store(0, Ordering::Relaxed);
    PY_COMPARE_EQ_FALLBACKS.store(0, Ordering::Relaxed);
    PY_COMPARE_NE_FALLBACKS.store(0, Ordering::Relaxed);
    PY_PATTERN_LITERAL_EQ_FALLBACKS.store(0, Ordering::Relaxed);
}

pub fn get_vm_fallback_stats() -> VMFallbackStats {
    VMFallbackStats {
        py_binary_div: PY_BINARY_DIV_FALLBACKS.load(Ordering::Relaxed),
        py_binary_floordiv: PY_BINARY_FLOORDIV_FALLBACKS.load(Ordering::Relaxed),
        py_binary_mod: PY_BINARY_MOD_FALLBACKS.load(Ordering::Relaxed),
        py_compare_eq: PY_COMPARE_EQ_FALLBACKS.load(Ordering::Relaxed),
        py_compare_ne: PY_COMPARE_NE_FALLBACKS.load(Ordering::Relaxed),
        py_pattern_literal_eq: PY_PATTERN_LITERAL_EQ_FALLBACKS.load(Ordering::Relaxed),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BigIntOpsBenchResult {
    pub bits: u32,
    pub iterations: usize,
    pub add_ns: f64,
    pub mul_ns: f64,
    pub floordiv_ns: f64,
    pub mod_ns: f64,
    pub div_ns: f64,
    pub fallback_delta: VMFallbackStats,
}

#[inline]
fn elapsed_ns_per_op(start: Instant, iterations: usize) -> f64 {
    start.elapsed().as_secs_f64() * 1_000_000_000.0 / iterations as f64
}

pub fn bench_bigint_ops(bits: u32, iterations: usize) -> BigIntOpsBenchResult {
    Python::attach(|_py| {
        let mut result = BigIntOpsBenchResult {
            bits,
            iterations,
            ..BigIntOpsBenchResult::default()
        };
        let a_big = (Integer::from(1_u8) << bits) + Integer::from(123_456_789_u64);
        let b_big = (Integer::from(1_u8) << bits.saturating_sub(1)) + Integer::from(987_654_321_u64);
        let a = Value::from_bigint(a_big);
        let b = Value::from_bigint(b_big);
        let before = get_vm_fallback_stats();

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_add(a, b).expect("binary_add failed");
            black_box(v.bits());
            if v.is_bigint() {
                v.decref();
            }
        }
        result.add_ns = elapsed_ns_per_op(start, iterations);

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_mul(a, b).expect("binary_mul failed");
            black_box(v.bits());
            if v.is_bigint() {
                v.decref();
            }
        }
        result.mul_ns = elapsed_ns_per_op(start, iterations);

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_floordiv(a, b).expect("binary_floordiv failed");
            black_box(v.bits());
            if v.is_bigint() {
                v.decref();
            }
        }
        result.floordiv_ns = elapsed_ns_per_op(start, iterations);

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_mod(a, b).expect("binary_mod failed");
            black_box(v.bits());
            if v.is_bigint() {
                v.decref();
            }
        }
        result.mod_ns = elapsed_ns_per_op(start, iterations);

        let start = Instant::now();
        for _ in 0..iterations {
            let v = binary_div(a, b).expect("binary_div failed");
            black_box(v.bits());
        }
        result.div_ns = elapsed_ns_per_op(start, iterations);

        let after = get_vm_fallback_stats();
        result.fallback_delta = VMFallbackStats {
            py_binary_div: after.py_binary_div.saturating_sub(before.py_binary_div),
            py_binary_floordiv: after.py_binary_floordiv.saturating_sub(before.py_binary_floordiv),
            py_binary_mod: after.py_binary_mod.saturating_sub(before.py_binary_mod),
            py_compare_eq: after.py_compare_eq.saturating_sub(before.py_compare_eq),
            py_compare_ne: after.py_compare_ne.saturating_sub(before.py_compare_ne),
            py_pattern_literal_eq: after.py_pattern_literal_eq.saturating_sub(before.py_pattern_literal_eq),
        };

        a.decref();
        b.decref();
        result
    })
}

/// VM execution error
#[derive(Debug)]
pub enum VMError {
    StackUnderflow,
    FrameOverflow,
    NameError(String),
    AttributeError(String),
    TypeError(String),
    RuntimeError(String),
    ValueError(String),
    IndexError(String),
    KeyError(String),
    ZeroDivisionError(String),
    MemoryLimitExceeded(String),
    /// User-defined or struct-based exception with full MRO.
    UserException(catnip_core::exception::ExceptionInfo),
    Interrupted,
    Exit(i32),
    Return(Value),
    Break,
    Continue,
}

impl std::fmt::Display for VMError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VMError::StackUnderflow => write!(f, "WeirdError: VM stack underflow"),
            VMError::FrameOverflow => write!(f, "WeirdError: VM frame stack overflow"),
            VMError::NameError(s) => {
                // VM creates NameError with bare name, py_interop with full message
                if s.starts_with("name '") {
                    write!(f, "NameError: {}", s)
                } else {
                    write!(f, "NameError: {}", catnip_core::constants::format_name_error(s))
                }
            }
            VMError::AttributeError(s) => write!(f, "AttributeError: {}", s),
            VMError::TypeError(s) => write!(f, "TypeError: {}", s),
            VMError::RuntimeError(s) => write!(f, "{}", s),
            VMError::ValueError(s) => write!(f, "ValueError: {}", s),
            VMError::IndexError(s) => write!(f, "IndexError: {}", s),
            VMError::KeyError(s) => write!(f, "KeyError: {}", s),
            VMError::ZeroDivisionError(s) => write!(f, "ZeroDivisionError: {}", s),
            VMError::MemoryLimitExceeded(s) => write!(f, "MemoryLimitExceeded: {}", s),
            VMError::UserException(info) => write!(f, "{}: {}", info.type_name, info.message),
            VMError::Interrupted => write!(f, "KeyboardInterrupt"),
            VMError::Exit(code) => write!(f, "exit({})", code),
            VMError::Return(_) => write!(f, "return signal"),
            VMError::Break => write!(f, "break signal"),
            VMError::Continue => write!(f, "continue signal"),
        }
    }
}

impl std::error::Error for VMError {}

impl VMError {
    /// Extract ExceptionInfo for catchable exceptions.
    pub fn exception_info(&self) -> Option<catnip_core::exception::ExceptionInfo> {
        use catnip_core::exception::{ExceptionInfo, ExceptionKind};
        match self {
            VMError::TypeError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::TypeError, msg.clone())),
            VMError::ValueError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::ValueError, msg.clone())),
            VMError::NameError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::NameError, msg.clone())),
            VMError::IndexError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::IndexError, msg.clone())),
            VMError::KeyError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::KeyError, msg.clone())),
            VMError::AttributeError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::AttributeError, msg.clone())),
            VMError::ZeroDivisionError(msg) => {
                Some(ExceptionInfo::from_kind(ExceptionKind::ZeroDivisionError, msg.clone()))
            }
            VMError::RuntimeError(msg) => Some(ExceptionInfo::from_kind(ExceptionKind::RuntimeError, msg.clone())),
            VMError::MemoryLimitExceeded(msg) => {
                Some(ExceptionInfo::from_kind(ExceptionKind::MemoryError, msg.clone()))
            }
            VMError::UserException(info) => Some(info.clone()),
            _ => None,
        }
    }

    /// Reconstruct VMError from stored exception info.
    pub fn from_exception_info(type_name: &str, msg: &str) -> VMError {
        match type_name {
            "TypeError" => VMError::TypeError(msg.into()),
            "ValueError" => VMError::ValueError(msg.into()),
            "NameError" => VMError::NameError(msg.into()),
            "IndexError" => VMError::IndexError(msg.into()),
            "KeyError" => VMError::KeyError(msg.into()),
            "AttributeError" => VMError::AttributeError(msg.into()),
            "ZeroDivisionError" => VMError::ZeroDivisionError(msg.into()),
            "MemoryError" => VMError::MemoryLimitExceeded(msg.into()),
            _ => VMError::RuntimeError(msg.into()),
        }
    }

    /// True for user-catchable exceptions (not control flow or internal errors).
    pub fn is_catchable(&self) -> bool {
        matches!(
            self,
            VMError::TypeError(_)
                | VMError::ValueError(_)
                | VMError::NameError(_)
                | VMError::IndexError(_)
                | VMError::KeyError(_)
                | VMError::AttributeError(_)
                | VMError::ZeroDivisionError(_)
                | VMError::RuntimeError(_)
                | VMError::MemoryLimitExceeded(_)
                | VMError::UserException(_)
        )
    }

    /// Convert to PendingUnwind for finally block processing.
    pub fn to_pending_unwind(&self) -> catnip_core::exception::PendingUnwind {
        use catnip_core::exception::PendingUnwind;
        match self {
            VMError::Return(_) => PendingUnwind::Return,
            VMError::Break => PendingUnwind::Break,
            VMError::Continue => PendingUnwind::Continue,
            other => {
                if let Some(info) = other.exception_info() {
                    PendingUnwind::Exception(info)
                } else {
                    PendingUnwind::Exception(catnip_core::exception::ExceptionInfo::from_name(
                        "RuntimeError".into(),
                        format!("{:?}", other),
                    ))
                }
            }
        }
    }
}

type VMResult<T> = Result<T, VMError>;

/// Debug stepping mode for interactive debugger.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DebugStepMode {
    /// No stepping, only stop at breakpoints
    Disabled = 0,
    /// Continue until next breakpoint
    Continue = 1,
    /// Stop at every instruction
    StepInto = 2,
    /// Stop when returning to same or shallower depth
    StepOver = 3,
    /// Stop when returning to shallower depth
    StepOut = 4,
}

impl DebugStepMode {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Continue,
            2 => Self::StepInto,
            3 => Self::StepOver,
            4 => Self::StepOut,
            _ => Self::Disabled,
        }
    }
}

/// Call frame info for stack traces.
#[derive(Clone, Debug)]
pub struct CallInfo {
    /// Function name (or "<module>")
    pub name: String,
    /// start_byte of the call site in the source
    pub call_start_byte: u32,
}

/// Error context captured when a VMError is raised.
#[derive(Clone, Debug)]
pub struct ErrorContext {
    /// Error type name ("TypeError", "NameError", etc.)
    pub error_type: String,
    /// Error message
    pub message: String,
    /// Position in source (start_byte) where the error occurred
    pub start_byte: u32,
    /// Call stack snapshot: (function_name, start_byte) per frame
    pub call_stack: Vec<(String, u32)>,
}

/// Tracking entry for an ND recursion frame pushed onto the VM stack.
struct NdRecurEntry {
    /// frame_stack.len() before the ND frame was pushed
    caller_depth: usize,
    /// Reference to the NDVmRecur for depth/cache updates on pop
    recur_py: Py<PyAny>,
    /// Cache key for memoization (None if disabled or unhashable)
    memo_key: Option<u64>,
}

/// Stack-based virtual machine for Catnip bytecode.
pub struct VM {
    /// Frame stack
    frame_stack: Vec<Frame>,
    /// Frame pool for reuse
    frame_pool: FramePool,
    /// Global variables (VM-owned)
    globals: IndexMap<String, Value>,
    /// Python context for name resolution fallback
    py_context: Option<Py<PyAny>>,
    /// Cached iter() builtin for GetIter
    cached_iter_fn: Option<Py<PyAny>>,
    /// Cached operator module for binary ops fallback
    cached_operator: Option<Py<PyAny>>,
    /// Cached operator.add for Add fallback
    cached_op_add: Option<Py<PyAny>>,
    /// Cached operator.sub for Sub fallback
    cached_op_sub: Option<Py<PyAny>>,
    /// Cached operator.mul for Mul fallback
    cached_op_mul: Option<Py<PyAny>>,
    /// Cached operator.truediv for Div fallback
    cached_op_truediv: Option<Py<PyAny>>,
    /// Cached operator.floordiv for FloorDiv fallback
    cached_op_floordiv: Option<Py<PyAny>>,
    /// Cached operator.mod for Mod fallback
    cached_op_mod: Option<Py<PyAny>>,
    /// Cached operator.pow for Pow fallback
    cached_op_pow: Option<Py<PyAny>>,
    /// Cached operator.lt for Lt fallback
    cached_op_lt: Option<Py<PyAny>>,
    /// Cached operator.le for Le fallback
    cached_op_le: Option<Py<PyAny>>,
    /// Cached operator.gt for Gt fallback
    cached_op_gt: Option<Py<PyAny>>,
    /// Cached operator.ge for Ge fallback
    cached_op_ge: Option<Py<PyAny>>,
    /// Cached operator.contains for In/NotIn
    cached_op_contains: Option<Py<PyAny>>,
    /// Cached NDTopos singleton for NdEmptyTopos
    cached_nd_topos: Option<Py<PyAny>>,
    /// Cached builtins.set for BuildSet
    cached_set_type: Option<Py<PyAny>>,
    /// Execution tracing enabled
    pub trace: bool,
    /// Profiling enabled
    pub profile: bool,
    /// Opcode counts for profiling
    pub profile_counts: HashMap<u8, u64>,
    /// Hot loop detector for JIT
    pub jit_detector: HotLoopDetector,
    /// Trace recorder (single-thread, no mutex needed)
    pub jit_recorder: TraceRecorder,
    /// JIT executor (behind Mutex for Sync - only used for compilation)
    pub jit: Mutex<Option<JITExecutor>>,
    /// JIT enabled flag
    pub jit_enabled: bool,
    /// Currently tracing flag
    jit_tracing: bool,
    /// Loop offset being traced
    jit_tracing_offset: usize,
    /// Function ID being traced (for function traces)
    jit_tracing_func_id: Option<String>,
    /// Frame stack depth when function tracing started (to detect when to stop)
    jit_tracing_depth: usize,
    /// Recursive call depth during tracing (suspend recording when > 0)
    jit_recursive_depth: usize,
    /// Pending trace: loop became hot, waiting for next iteration to start tracing
    jit_pending_trace: Option<usize>,
    /// Pending function trace: function became hot, waiting for next top-level call
    jit_pending_function_trace: Option<String>,
    /// Guard failed at this loop offset - skip JIT for one iteration
    jit_guard_failed: Option<usize>,
    /// Call stack for source-level stack traces
    call_stack: Vec<CallInfo>,
    /// Source code bytes (set before execution for error reporting)
    source: Option<Vec<u8>>,
    /// Source filename
    filename: String,
    /// Last error context (captured on VMError)
    pub last_error_context: Option<ErrorContext>,
    /// Debug callback (Python callable), called at breakpoints
    pub debug_callback: Option<Py<PyAny>>,
    /// Debug breakpoints (byte offsets in source)
    pub debug_breakpoints: HashSet<u32>,
    /// Current debug stepping mode
    pub debug_step_mode: DebugStepMode,
    /// Frame depth when stepping started (for step over/out)
    pub debug_step_depth: usize,
    /// Last byte offset where we paused (to avoid double-pause on same position)
    debug_last_paused_byte: Option<u32>,
    /// Native VM function table (grow-only, no refcounting)
    pub func_table: FunctionTable,
    /// Native struct type and instance registry
    pub struct_registry: StructRegistry,
    /// PyObject ptr -> StructTypeId, populated by MakeStruct
    struct_type_map: HashMap<usize, StructTypeId>,
    /// Trait registry for trait composition
    pub trait_registry: TraitRegistry,
    /// Enum type registry
    pub enum_registry: EnumRegistry,
    /// Symbol interning table (used by enums)
    pub symbol_table: SymbolTable,
    /// PyObject ptr -> enum_type_id, populated by MakeEnum
    enum_type_map: HashMap<usize, u32>,
    /// Stack of pre-existing global names at each module-level PushBlock
    block_globals_snapshot: Vec<Vec<String>>,
    /// Last source byte offset seen in dispatch loop (for error context)
    last_src_byte: u32,
    /// Memory limit in bytes (0 = disabled)
    memory_limit_bytes: u64,
    /// Instruction counter for periodic RSS checks
    instruction_count: u64,
    /// Interrupt flag (set by external signal to abort execution)
    interrupt_flag: Arc<AtomicBool>,
    /// ND recursion frame tracking stack
    nd_recur_stack: Vec<NdRecurEntry>,
    /// Loop offsets already checked against JIT trace cache (warm-start)
    jit_cache_checked: HashSet<usize>,
}

impl VM {
    /// Create a new VM.
    pub fn new() -> Self {
        Self {
            frame_stack: Vec::with_capacity(crate::constants::VM_FRAME_STACK_INIT),
            frame_pool: FramePool::default(),
            globals: IndexMap::new(),
            py_context: None,
            cached_iter_fn: None,
            cached_operator: None,
            cached_op_add: None,
            cached_op_sub: None,
            cached_op_mul: None,
            cached_op_truediv: None,
            cached_op_floordiv: None,
            cached_op_mod: None,
            cached_op_pow: None,
            cached_op_lt: None,
            cached_op_le: None,
            cached_op_gt: None,
            cached_op_ge: None,
            cached_op_contains: None,
            cached_nd_topos: None,
            cached_set_type: None,
            trace: false,
            profile: false,
            profile_counts: HashMap::new(),
            jit_detector: HotLoopDetector::new(JIT_THRESHOLD_DEFAULT),
            jit_recorder: TraceRecorder::new(),
            jit: Mutex::new(None), // Lazy init
            jit_enabled: false,    // Controlled by Python ConfigManager
            jit_tracing: false,
            jit_tracing_offset: 0,
            jit_tracing_func_id: None,
            jit_tracing_depth: 0,
            jit_recursive_depth: 0,
            jit_pending_trace: None,
            jit_pending_function_trace: None,
            jit_guard_failed: None,
            call_stack: Vec::new(),
            source: None,
            filename: "<input>".to_string(),
            last_error_context: None,
            debug_callback: None,
            debug_breakpoints: HashSet::new(),
            debug_step_mode: DebugStepMode::Disabled,
            debug_step_depth: 0,
            debug_last_paused_byte: None,
            func_table: FunctionTable::new(),
            struct_registry: StructRegistry::new(),
            struct_type_map: HashMap::new(),
            trait_registry: TraitRegistry::new(),
            enum_registry: EnumRegistry::new(),
            symbol_table: SymbolTable::new(),
            enum_type_map: HashMap::new(),
            block_globals_snapshot: Vec::new(),
            last_src_byte: 0,
            memory_limit_bytes: MEMORY_LIMIT_DEFAULT_MB * 1024 * 1024,
            instruction_count: 0,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            nd_recur_stack: Vec::new(),
            jit_cache_checked: HashSet::new(),
        }
    }

    /// Get a clone of the interrupt flag for external signal handlers.
    pub fn interrupt_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.interrupt_flag)
    }

    /// Enable JIT compilation with custom threshold.
    pub fn enable_jit_with_threshold(&mut self, threshold: u32) {
        self.jit_enabled = true;
        // Reset detector with new threshold
        self.jit_detector = HotLoopDetector::new(threshold);
        // Lazy init the JIT executor
        let mut jit = self.jit.lock().unwrap();
        if jit.is_none() {
            *jit = Some(JITExecutor::new(threshold));
        }
    }

    /// Enable JIT compilation.
    pub fn enable_jit(&mut self) {
        self.enable_jit_with_threshold(JIT_THRESHOLD_DEFAULT);
    }

    /// Disable JIT compilation.
    pub fn disable_jit(&mut self) {
        self.jit_enabled = false;
        // Reset JIT state to avoid stale traces when re-enabling
        self.jit_detector = HotLoopDetector::new(JIT_THRESHOLD_DEFAULT);
        self.jit_recorder = TraceRecorder::new();
        self.jit_tracing = false;
        self.jit_tracing_offset = 0;
        self.jit_guard_failed = None;
        // Clear compiled traces
        *self.jit.lock().unwrap() = None;
    }

    /// Handle frame pop for ND recursion: decrement depth, cache result.
    #[inline]
    fn handle_nd_frame_pop(&mut self, py: Python<'_>, result: Value) {
        if let Some(entry) = self.nd_recur_stack.last() {
            if self.frame_stack.len() == entry.caller_depth {
                let entry = self.nd_recur_stack.pop().unwrap();
                if let Ok(nd_recur) = entry.recur_py.bind(py).cast::<crate::nd::NDVmRecur>() {
                    let r = nd_recur.borrow();
                    let d = r.depth_cell().get();
                    if d > 0 {
                        r.depth_cell().set(d - 1);
                    }
                    if let Some(k) = entry.memo_key {
                        r.cache_ref().borrow_mut().insert(k, result.to_pyobject(py));
                    }
                }
            }
        }
    }

    /// Update the JIT executor's bytecode hash from a CodeObject (lazy, cached).
    #[inline]
    fn update_jit_bytecode_hash(&self, code: &CodeObject) {
        if !self.jit_enabled {
            return;
        }
        self.update_jit_bytecode_hash_value(code.bytecode_hash());
    }

    /// Set a pre-computed bytecode hash on the JIT executor.
    #[inline]
    fn update_jit_bytecode_hash_value(&self, hash: u64) {
        if !self.jit_enabled {
            return;
        }
        if let Ok(mut jit) = self.jit.lock() {
            if let Some(ref mut executor) = *jit {
                executor.set_bytecode_hash(hash);
            }
        }
    }

    /// Set memory limit in MB (0 = disabled).
    pub fn set_memory_limit(&mut self, mb: u64) {
        self.memory_limit_bytes = mb * 1024 * 1024;
    }

    /// Set the Python context for name resolution.
    pub fn set_context(&mut self, context: Py<PyAny>) {
        self.py_context = Some(context);
    }

    /// Borrow the Python context reference (used by `ContextHost::new()`).
    #[inline]
    pub fn py_context(&self) -> &Option<Py<PyAny>> {
        &self.py_context
    }

    /// Get the cached iter() builtin (used by `ContextHost::new()`).
    /// Panics if `ensure_builtins_cached` hasn't been called.
    #[inline]
    pub fn cached_iter_fn(&self) -> &Py<PyAny> {
        self.cached_iter_fn.as_ref().expect("iter_fn should be cached")
    }

    /// Get cached `operator.contains` ref (used by `ContextHost::new()`).
    /// Panics if `ensure_builtins_cached` hasn't been called.
    #[inline]
    pub fn cached_contains_fn(&self) -> &Py<PyAny> {
        self.cached_op_contains.as_ref().expect("contains_fn should be cached")
    }

    /// Get a cached operator ref by enum variant (used by `ContextHost::new()`).
    /// Panics if `ensure_builtins_cached` hasn't been called.
    #[inline]
    pub fn cached_op(&self, op: super::host::CachedOp) -> &Py<PyAny> {
        use super::host::CachedOp;
        match op {
            CachedOp::Add => self.cached_op_add.as_ref().unwrap(),
            CachedOp::Sub => self.cached_op_sub.as_ref().unwrap(),
            CachedOp::Mul => self.cached_op_mul.as_ref().unwrap(),
            CachedOp::TrueDiv => self.cached_op_truediv.as_ref().unwrap(),
            CachedOp::FloorDiv => self.cached_op_floordiv.as_ref().unwrap(),
            CachedOp::Mod => self.cached_op_mod.as_ref().unwrap(),
            CachedOp::Pow => self.cached_op_pow.as_ref().unwrap(),
            CachedOp::Lt => self.cached_op_lt.as_ref().unwrap(),
            CachedOp::Le => self.cached_op_le.as_ref().unwrap(),
            CachedOp::Gt => self.cached_op_gt.as_ref().unwrap(),
            CachedOp::Ge => self.cached_op_ge.as_ref().unwrap(),
        }
    }

    /// Set source code and filename for error reporting.
    pub fn set_source(&mut self, source: Vec<u8>, filename: String) {
        self.source = Some(source);
        self.filename = filename;
    }

    /// Get the last error context (if any).
    pub fn take_last_error_context(&mut self) -> Option<ErrorContext> {
        self.last_error_context.take()
    }

    /// Invoke debug callback with pre-collected data (avoids borrow conflicts).
    fn invoke_debug_callback(
        &mut self,
        py: Python<'_>,
        start_byte: u32,
        locals_data: &[(String, Value)],
        call_stack_data: &[(String, u32)],
    ) -> Result<DebugStepMode, VMError> {
        let cb = match &self.debug_callback {
            Some(cb) => cb.clone_ref(py),
            None => return Ok(DebugStepMode::Continue),
        };

        let locals_dict = PyDict::new(py);
        for (name, val) in locals_data {
            let _ = locals_dict.set_item(name, val.to_pyobject(py));
        }

        let call_stack = PyList::new(
            py,
            call_stack_data.iter().map(|(name, sb)| {
                PyTuple::new(
                    py,
                    [
                        name.clone().into_pyobject(py).unwrap().into_any().unbind(),
                        (*sb).into_pyobject(py).unwrap().into_any().unbind(),
                    ],
                )
                .unwrap()
                .into_any()
                .unbind()
            }),
        )
        .to_vm(py)?;

        let result = cb.call1(py, (start_byte, locals_dict, call_stack)).to_vm(py)?;
        let action_int: i32 = result.extract(py).unwrap_or(1);
        Ok(DebugStepMode::from_i32(action_int))
    }

    /// Capture error context from current VM state.
    fn capture_error_context(&mut self, error: &VMError) {
        let (error_type, message) = match error {
            VMError::NameError(s) => ("NameError".to_string(), s.clone()),
            VMError::AttributeError(s) => ("AttributeError".to_string(), s.clone()),
            VMError::TypeError(s) => ("TypeError".to_string(), s.clone()),
            VMError::ValueError(s) => ("ValueError".to_string(), s.clone()),
            VMError::IndexError(s) => ("IndexError".to_string(), s.clone()),
            VMError::KeyError(s) => ("KeyError".to_string(), s.clone()),
            VMError::ZeroDivisionError(s) => ("ZeroDivisionError".to_string(), s.clone()),
            VMError::RuntimeError(s) => ("RuntimeError".to_string(), s.clone()),
            VMError::MemoryLimitExceeded(s) => ("MemoryError".to_string(), s.clone()),
            VMError::UserException(info) => (info.type_name.clone(), info.message.clone()),
            VMError::StackUnderflow => ("RuntimeError".to_string(), "WeirdError: VM stack underflow".to_string()),
            VMError::FrameOverflow => (
                "RuntimeError".to_string(),
                "WeirdError: VM frame stack overflow".to_string(),
            ),
            VMError::Interrupted => ("KeyboardInterrupt".to_string(), "execution interrupted".to_string()),
            // Exit and control flow signals - no error context needed
            VMError::Exit(_) | VMError::Return(_) | VMError::Break | VMError::Continue => return,
        };

        // Use last_src_byte tracked in dispatch loop (always up-to-date)
        let start_byte = self.last_src_byte;

        // Snapshot the call stack
        let call_stack: Vec<(String, u32)> = self
            .call_stack
            .iter()
            .map(|ci| (ci.name.clone(), ci.call_start_byte))
            .collect();

        self.last_error_context = Some(ErrorContext {
            error_type,
            message,
            start_byte,
            call_stack,
        });
    }

    /// Cache builtins for fast access in dispatch loop.
    fn ensure_builtins_cached(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.cached_iter_fn.is_none() {
            let builtins = py.import("builtins")?;
            self.cached_iter_fn = Some(builtins.getattr("iter")?.unbind());
            let op_mod = py.import("operator")?;
            self.cached_op_add = Some(op_mod.getattr("add")?.unbind());
            self.cached_op_sub = Some(op_mod.getattr("sub")?.unbind());
            self.cached_op_mul = Some(op_mod.getattr("mul")?.unbind());
            self.cached_op_truediv = Some(op_mod.getattr("truediv")?.unbind());
            self.cached_op_floordiv = Some(op_mod.getattr("floordiv")?.unbind());
            self.cached_op_mod = Some(op_mod.getattr("mod")?.unbind());
            self.cached_op_pow = Some(op_mod.getattr("pow")?.unbind());
            self.cached_op_lt = Some(op_mod.getattr("lt")?.unbind());
            self.cached_op_le = Some(op_mod.getattr("le")?.unbind());
            self.cached_op_gt = Some(op_mod.getattr("gt")?.unbind());
            self.cached_op_ge = Some(op_mod.getattr("ge")?.unbind());
            self.cached_op_contains = Some(op_mod.getattr("contains")?.unbind());
            self.cached_operator = Some(op_mod.unbind().into());
        }
        Ok(())
    }

    /// Push an init frame for a struct instance if init_fn is Some.
    /// Returns true if a frame was pushed (caller should `continue`).
    fn push_struct_init_frame(
        &mut self,
        py: Python<'_>,
        inst_val: Value,
        init_fn: Option<Py<PyAny>>,
        frame: &mut Frame,
    ) -> VMResult<bool> {
        let Some(init_fn) = init_fn else { return Ok(false) };
        let init_bound = init_fn.bind(py);
        let init_data = if let Ok(f) = init_bound.cast::<VMFunction>() {
            let r = f.borrow();
            let code = Arc::clone(&r.vm_code.borrow(py).inner);
            let cl = r.native_closure.clone();
            drop(r);
            Some((code, cl))
        } else if let Ok(vm_code) = init_bound.getattr("vm_code") {
            Some((convert_code_object(py, &vm_code).to_vm(py)?, None))
        } else {
            None
        };
        if let Some((new_code, native_closure)) = init_data {
            self.struct_registry.incref(inst_val.as_struct_instance_idx().unwrap());
            frame.push(inst_val);
            let mut new_frame = Frame::with_code(new_code);
            new_frame.set_local(0, inst_val);
            new_frame.closure_scope = native_closure;
            new_frame.discard_return = true;
            self.setup_super_proxy(py, inst_val, None, &mut new_frame)?;
            let old = std::mem::replace(frame, new_frame);
            self.frame_stack.push(old);
            return Ok(true);
        }
        Ok(false)
    }

    /// Setup a super proxy on a frame for method calls on struct instances.
    /// If `super_source_type` is Some, resolve super from that type's parent (chain resolution).
    /// Otherwise, resolve from the instance's own type.
    fn setup_super_proxy(
        &self,
        py: Python<'_>,
        inst_val: Value,
        super_source_type: Option<String>,
        frame: &mut Frame,
    ) -> VMResult<()> {
        // Resolve the instance's real type name
        let real_type_name = if let Some(idx) = inst_val.as_struct_instance_idx() {
            self.struct_registry
                .get_instance(idx)
                .and_then(|inst| self.struct_registry.get_type(inst.type_id).map(|ty| ty.name.clone()))
        } else {
            let inst_py = inst_val.to_pyobject(py);
            let inst_bound = inst_py.bind(py);
            inst_bound
                .cast::<super::structs::CatnipStructProxy>()
                .ok()
                .map(|proxy| proxy.borrow().type_name.clone())
        };

        let Some(real_name) = real_type_name else {
            return Ok(());
        };

        let Some(real_type) = self.struct_registry.find_type_by_name(&real_name) else {
            return Ok(());
        };

        // Only types with parents need super
        if real_type.parent_names.is_empty() {
            return Ok(());
        }

        let mro = &real_type.mro;

        // Find position: super_source_type tells us which type's method we're in
        let start_pos = if let Some(ref source) = super_source_type {
            // Find source in MRO and skip past it
            mro.iter().position(|n| n == source).map(|p| p + 1).unwrap_or(1)
        } else {
            // Normal call from the struct's own method: skip self (pos 0)
            1
        };

        if start_pos >= mro.len() {
            return Ok(());
        }

        // Collect methods from MRO[start_pos:], first-wins, with provenance
        let mut methods: IndexMap<String, Py<PyAny>> = IndexMap::new();
        let mut method_sources: HashMap<String, String> = HashMap::new();
        for mro_type_name in &mro[start_pos..] {
            if let Some(ty) = self.struct_registry.find_type_by_name(mro_type_name) {
                for (k, v) in &ty.methods {
                    if !methods.contains_key(k) {
                        methods.insert(k.clone(), v.clone_ref(py));
                        method_sources.insert(k.clone(), mro_type_name.clone());
                    }
                }
            }
        }

        if !methods.is_empty() {
            let inst_py = inst_val.to_pyobject(py);
            let native_idx = inst_val.as_struct_instance_idx();
            let proxy = Py::new(
                py,
                super::structs::SuperProxy {
                    methods,
                    instance: inst_py,
                    method_sources,
                    native_instance_idx: native_idx,
                },
            )
            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
            frame.super_proxy = Some(proxy.into_any());
        }
        Ok(())
    }

    /// Execute a code object and return the result.
    pub fn execute(&mut self, py: Python<'_>, code: Arc<CodeObject>, args: &[Value]) -> VMResult<Value> {
        self.execute_with_closure(py, code, args, None)
    }

    /// Execute a code object with an optional native closure scope.
    pub fn execute_with_closure(
        &mut self,
        py: Python<'_>,
        code: Arc<CodeObject>,
        args: &[Value],
        closure_scope: Option<NativeClosureScope>,
    ) -> VMResult<Value> {
        // Set bytecode hash for JIT trace cache and reset warm-start tracking
        self.update_jit_bytecode_hash(&code);
        self.jit_cache_checked.clear();

        // Create initial frame
        let mut frame = Frame::with_code(code);
        frame.bind_args(py, args, None);
        frame.closure_scope = closure_scope;

        self.frame_stack.push(frame);

        // Clear previous error context
        self.last_error_context = None;
        self.call_stack.clear();

        // Install thread-local pointers for Value conversions.
        // Save previous pointers so re-entrant VM calls (e.g. import) restore them.
        let prev_sym = super::value::save_symbol_table();
        let prev_enum = super::value::save_enum_registry();
        super::value::set_struct_registry(&self.struct_registry as *const _);
        super::value::set_func_table(&self.func_table as *const _);
        super::value::set_symbol_table(&self.symbol_table as *const _ as *mut _);
        super::value::set_enum_registry(&self.enum_registry as *const _ as *mut _);

        // Run dispatch loop
        let result = match self.run(py) {
            Ok(v) => v,
            Err(e) => {
                self.capture_error_context(&e);
                self.nd_recur_stack.clear();
                while let Some(frame) = self.frame_stack.pop() {
                    self.frame_pool.free(frame, &mut self.struct_registry);
                }
                super::value::restore_symbol_table(prev_sym);
                super::value::restore_enum_registry(prev_enum);
                return Err(e);
            }
        };

        // Clean up
        while let Some(frame) = self.frame_stack.pop() {
            self.frame_pool.free(frame, &mut self.struct_registry);
        }

        super::value::restore_symbol_table(prev_sym);
        super::value::restore_enum_registry(prev_enum);
        Ok(result)
    }

    /// Execute a code object with a custom VmHost (no Python Context needed).
    pub fn execute_with_host(
        &mut self,
        py: Python<'_>,
        code: Arc<CodeObject>,
        args: &[Value],
        host: &dyn super::host::VmHost,
        closure_scope: Option<NativeClosureScope>,
    ) -> VMResult<Value> {
        let mut frame = Frame::with_code(code);
        frame.bind_args(py, args, None);
        frame.closure_scope = closure_scope;

        self.frame_stack.push(frame);
        self.last_error_context = None;
        self.call_stack.clear();

        let prev_sym = super::value::save_symbol_table();
        let prev_enum = super::value::save_enum_registry();
        super::value::set_struct_registry(&self.struct_registry as *const _);
        super::value::set_func_table(&self.func_table as *const _);
        super::value::set_symbol_table(&self.symbol_table as *const _ as *mut _);
        super::value::set_enum_registry(&self.enum_registry as *const _ as *mut _);

        let result = match self.run_with_host(py, host) {
            Ok(v) => v,
            Err(e) => {
                self.capture_error_context(&e);
                self.nd_recur_stack.clear();
                while let Some(frame) = self.frame_stack.pop() {
                    self.frame_pool.free(frame, &mut self.struct_registry);
                }
                super::value::restore_symbol_table(prev_sym);
                super::value::restore_enum_registry(prev_enum);
                return Err(e);
            }
        };

        while let Some(frame) = self.frame_stack.pop() {
            self.frame_pool.free(frame, &mut self.struct_registry);
        }

        super::value::restore_symbol_table(prev_sym);
        super::value::restore_enum_registry(prev_enum);
        Ok(result)
    }

    /// Get globals as a HashMap reference for syncing back to Python.
    pub fn get_globals(&self) -> &IndexMap<String, Value> {
        &self.globals
    }

    /// Main dispatch loop with the default ContextHost.
    fn run(&mut self, py: Python<'_>) -> VMResult<Value> {
        // Cache builtins once at start of execution
        self.ensure_builtins_cached(py).to_vm(py)?;

        // Build host: owns ctx_globals, operator refs, py_context, and iter_fn
        let host = super::host::ContextHost::new(py, self);
        self.dispatch(py, &host)
    }

    /// Main dispatch loop with a custom host.
    fn run_with_host(&mut self, py: Python<'_>, host: &dyn super::host::VmHost) -> VMResult<Value> {
        self.dispatch(py, host)
    }

    /// Outer dispatch loop with exception unwinding.
    fn dispatch(&mut self, py: Python<'_>, host: &dyn super::host::VmHost) -> VMResult<Value> {
        let mut frame = match self.frame_stack.pop() {
            Some(f) => f,
            None => return Ok(Value::NIL),
        };
        'outer: loop {
            match self.dispatch_inner(&mut frame, py, host) {
                Ok(val) => {
                    // Normal exit: opcodes balanced refcounts, just drop
                    drop(frame);
                    return Ok(val);
                }
                Err(err) => {
                    // Try to unwind to an exception handler
                    if self.unwind_exception(&mut frame, &err) {
                        continue 'outer;
                    }
                    // No handler found. Handle Return specially (frame pop).
                    if let VMError::Return(val) = err {
                        if let Some(caller) = self.frame_stack.pop() {
                            let discard = frame.discard_return;
                            let old = std::mem::replace(&mut frame, caller);
                            drop(old);
                            self.handle_nd_frame_pop(py, val);
                            if discard {
                                // discard_return: don't push result to caller
                            } else {
                                frame.push(val);
                            }
                            continue 'outer;
                        }
                        drop(frame);
                        return Ok(val);
                    }
                    // Error path: frames may have unbalanced refcounts, use free for cleanup
                    while let Some(f) = self.frame_stack.pop() {
                        self.frame_pool.free(f, &mut self.struct_registry);
                    }
                    self.frame_pool.free(frame, &mut self.struct_registry);
                    return Err(err);
                }
            }
        }
    }

    /// Inner dispatch loop. Returns Ok on clean exit, Err on any signal/exception.
    fn dispatch_inner(&mut self, frame: &mut Frame, py: Python<'_>, host: &dyn super::host::VmHost) -> VMResult<Value> {
        #[allow(unused_assignments)]
        let mut last_result = Value::NIL;

        loop {
            let code = match &frame.code {
                Some(c) => c.clone(),
                None => return Ok(Value::NIL),
            };
            // SAFETY: code Arc is kept alive by the frame (never replaced during execution).
            // Raw pointer avoids atomic refcount on every instruction fetch.
            let code: &CodeObject = unsafe { &*Arc::as_ptr(&code) };

            // Check if we've reached the end of bytecode
            if frame.ip >= code.instructions.len() {
                let result = if !frame.stack.is_empty() {
                    frame.pop()
                } else {
                    Value::NIL
                };
                if let Some(caller) = self.frame_stack.pop() {
                    let discard = frame.discard_return;
                    let old = std::mem::replace(frame, caller);
                    // Don't call frame_pool.free: opcodes balance refcounts,
                    // so decref in free() would double-decrement.
                    drop(old);
                    self.handle_nd_frame_pop(py, result);
                    if !discard {
                        frame.push(result);
                    }
                    continue;
                }
                return Ok(result);
            }

            // Fetch instruction + source position
            let instr = code.instructions[frame.ip];
            let _current_src_byte = code.line_table.get(frame.ip).copied().unwrap_or(0);
            self.last_src_byte = _current_src_byte;
            frame.ip += 1;

            // Periodic checks (every ~65k instructions)
            self.instruction_count = self.instruction_count.wrapping_add(1);
            if self.instruction_count & MEMORY_CHECK_INTERVAL == 0 {
                // Interrupt check (Ctrl+C from REPL)
                if self.interrupt_flag.load(Ordering::Relaxed) {
                    self.interrupt_flag.store(false, Ordering::Relaxed);
                    return Err(VMError::Interrupted);
                }
                // RSS memory guard
                if self.memory_limit_bytes > 0 {
                    if let Some(rss) = super::memory::get_rss_bytes() {
                        if rss > self.memory_limit_bytes {
                            let rss_mb = rss / (1024 * 1024);
                            let limit_mb = self.memory_limit_bytes / (1024 * 1024);
                            return Err(VMError::MemoryLimitExceeded(format!(
                                "memory limit exceeded ({rss_mb} MB / {limit_mb} MB)\n\
                                 Increase: catnip -o memory:{}\n\
                                 Disable:  catnip -o memory:0",
                                limit_mb * 2
                            )));
                        }
                    }
                }
            }

            if self.trace {
                eprintln!(
                    "[TRACE] ip={} {:?} arg={} stack_len={}",
                    frame.ip - 1,
                    instr.op,
                    instr.arg,
                    frame.stack.len()
                );
            }

            if self.profile {
                *self.profile_counts.entry(instr.op as u8).or_insert(0) += 1;
            }

            // Debug hook: determine if we should pause (before dispatch)
            let debug_should_pause = if self.debug_callback.is_some() {
                // Clear last_paused tracking when we move to a new source position
                if self.debug_last_paused_byte != Some(_current_src_byte) {
                    self.debug_last_paused_byte = None;
                }
                let is_step = matches!(
                    self.debug_step_mode,
                    DebugStepMode::StepInto | DebugStepMode::StepOver | DebugStepMode::StepOut
                );
                match instr.op {
                    OpCode::Breakpoint => {
                        // Always pause on explicit breakpoint()
                        self.debug_last_paused_byte != Some(_current_src_byte)
                    }
                    _ if is_step => match self.debug_step_mode {
                        DebugStepMode::StepInto => true,
                        DebugStepMode::StepOver => self.call_stack.len() <= self.debug_step_depth,
                        DebugStepMode::StepOut => self.call_stack.len() < self.debug_step_depth,
                        _ => false,
                    },
                    _ => {
                        // Byte-offset breakpoints: skip if same position as last pause
                        self.debug_breakpoints.contains(&_current_src_byte)
                            && self.debug_last_paused_byte != Some(_current_src_byte)
                    }
                }
            } else {
                false
            };

            // JIT trace recording (skip opcodes handled specially)
            // Only record if not inside a recursive call (jit_recursive_depth == 0)
            if self.jit_tracing
                && self.jit_recursive_depth == 0
                && instr.op != OpCode::ForRangeInt
                && instr.op != OpCode::LoadScope
                && instr.op != OpCode::StoreScope
            {
                let ip = frame.ip - 1;
                // Detect if we're working with integers, booleans, or floats
                // Booleans are treated as ints (True=1, False=0) in JIT
                let is_int_value = match instr.op {
                    // Binary ops: check top of stack
                    OpCode::Add
                    | OpCode::Sub
                    | OpCode::Mul
                    | OpCode::Div
                    | OpCode::Mod
                    | OpCode::Lt
                    | OpCode::Le
                    | OpCode::Gt
                    | OpCode::Ge
                    | OpCode::Eq
                    | OpCode::Ne => {
                        if !frame.stack.is_empty() {
                            let v = frame.stack[frame.stack.len() - 1];
                            v.is_int() || v.is_bool()
                        } else {
                            true
                        }
                    }
                    // LoadLocal: check the value being loaded
                    OpCode::LoadLocal => {
                        let slot = instr.arg as usize;
                        if slot < frame.locals.len() {
                            let v = frame.locals[slot];
                            v.is_int() || v.is_bool()
                        } else {
                            true
                        }
                    }
                    // Default to int for other ops
                    _ => true,
                };
                if !self.jit_recorder.record_opcode(instr.op, instr.arg, is_int_value, ip) {
                    // Trace was aborted (e.g. exception opcodes) -- reset tracing state
                    self.jit_tracing = false;
                    self.jit_tracing_func_id = None;
                }
            }

            // Dispatch via match - compiles to jump table
            match instr.op {
                // --- Stack operations ---
                OpCode::LoadConst => {
                    let idx = instr.arg as usize;
                    let value = if idx < code.constants.len() {
                        code.constants[idx]
                    } else {
                        Value::NIL
                    };
                    // Record constant value for JIT (only if not suspended)
                    if self.jit_tracing && self.jit_recursive_depth == 0 {
                        let ip = frame.ip - 1;
                        if let Some(i) = value.as_int() {
                            self.jit_recorder.record_const_int(i, ip);
                        } else if let Some(f) = value.as_float() {
                            self.jit_recorder.record_const_float(f, ip);
                        } else if let Some(b) = value.as_bool() {
                            // Treat booleans as ints for JIT (0 or 1)
                            self.jit_recorder.record_const_int(if b { 1 } else { 0 }, ip);
                        } else {
                            // Other constants (None, strings, etc.) - record as 0 to balance stack
                            // These will likely prevent compilation (fallback to interpreter)
                            self.jit_recorder.record_const_int(0, ip);
                        }
                    }
                    // Incref: const is shared with stack
                    value.clone_refcount_bigint();
                    if value.is_struct_instance() {
                        self.struct_registry.incref(value.as_struct_instance_idx().unwrap());
                    }
                    frame.push(value);
                }

                OpCode::LoadLocal => {
                    let value = frame.get_local(instr.arg as usize);
                    value.clone_refcount_bigint();
                    if value.is_struct_instance() {
                        self.struct_registry.incref(value.as_struct_instance_idx().unwrap());
                    }
                    frame.push(value);
                }

                OpCode::StoreLocal => {
                    let value = frame.pop();
                    let old = frame.get_local(instr.arg as usize);
                    decref_discard(&mut self.struct_registry, old);
                    frame.set_local(instr.arg as usize, value);
                }

                OpCode::LoadScope => {
                    let name = get_name(code, instr.arg)?;
                    let resolved_value: Value;

                    // 0. Check super proxy (for extends parent method access)
                    if name == "super" {
                        if let Some(ref proxy) = frame.super_proxy {
                            let value = Value::from_pyobject(py, proxy.bind(py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            frame.push(value);
                            continue;
                        }
                    }

                    // 1. Check closure captured vars (no parent chain, pure Rust)
                    if let Some(ref closure) = frame.closure_scope {
                        if let Some(value) = closure.resolve_captured_only(name) {
                            resolved_value = value;
                            resolved_value.clone_refcount_bigint();
                            if resolved_value.is_struct_instance() {
                                self.struct_registry
                                    .incref(resolved_value.as_struct_instance_idx().unwrap());
                            }
                            frame.push(resolved_value);
                            if self.jit_tracing {
                                if let Some(int_val) = resolved_value.as_int() {
                                    let ip = frame.ip - 1;
                                    self.jit_recorder.record_load_scope(name, int_val, ip);
                                }
                            }
                            continue;
                        }
                    }
                    // 2. Check VM globals (Rust HashMap, O(1), always in sync)
                    if let Some(&value) = self.globals.get(name.as_str()) {
                        resolved_value = value;
                        resolved_value.clone_refcount_bigint();
                        if resolved_value.is_struct_instance() {
                            self.struct_registry
                                .incref(resolved_value.as_struct_instance_idx().unwrap());
                        }
                        frame.push(resolved_value);
                        if self.jit_tracing {
                            if let Some(int_val) = resolved_value.as_int() {
                                let ip = frame.ip - 1;
                                self.jit_recorder.record_load_scope(name, int_val, ip);
                            }
                        }
                        continue;
                    }
                    // 3. Check closure parent chain (may hit PyGlobals)
                    if let Some(ref closure) = frame.closure_scope {
                        if let Some(value) = closure.resolve_with_py(py, name) {
                            resolved_value = value;
                            resolved_value.clone_refcount_bigint();
                            if resolved_value.is_struct_instance() {
                                self.struct_registry
                                    .incref(resolved_value.as_struct_instance_idx().unwrap());
                            }
                            frame.push(resolved_value);
                            if self.jit_tracing {
                                if let Some(int_val) = resolved_value.as_int() {
                                    let ip = frame.ip - 1;
                                    self.jit_recorder.record_load_scope(name, int_val, ip);
                                }
                            }
                            continue;
                        }
                    }
                    // 4. Fallback to ctx_globals (Python builtins, modules)
                    if let Some(value) = host.lookup_global(py, name)? {
                        resolved_value = value;
                        resolved_value.clone_refcount_bigint();
                        if resolved_value.is_struct_instance() {
                            self.struct_registry
                                .incref(resolved_value.as_struct_instance_idx().unwrap());
                        }
                        frame.push(resolved_value);
                        if self.jit_tracing {
                            if let Some(int_val) = resolved_value.as_int() {
                                let ip = frame.ip - 1;
                                self.jit_recorder.record_load_scope(name, int_val, ip);
                            }
                        }
                        continue;
                    }
                    return Err(VMError::NameError(name.to_owned()));
                }

                OpCode::StoreScope => {
                    let name = get_name(code, instr.arg)?;

                    // Check slotmap before recording
                    let slot_idx = code.slotmap.get(name.as_str()).copied();

                    // Record StoreScope during tracing (BEFORE pop, while value is on stack)
                    // Pass the existing slot from slotmap if available
                    let trace_slot = if self.jit_tracing {
                        let ip = frame.ip - 1;
                        self.jit_recorder.record_store_scope(name, ip, slot_idx)
                    } else {
                        None
                    };

                    let value = frame.pop();

                    // During tracing, also store to the trace slot to keep frame.locals synchronized
                    // Track whether we already wrote to the local slot (to avoid double-decref)
                    let mut local_slot_written = false;
                    if let Some(slot) = trace_slot {
                        if slot >= frame.locals.len() {
                            frame.locals.resize(slot + 1, Value::NIL);
                        }
                        let old = frame.get_local(slot);
                        decref_discard(&mut self.struct_registry, old);
                        frame.set_local(slot, value);
                        local_slot_written = true;
                    } else if self.jit_enabled {
                        // When JIT is enabled (but not currently tracing), still sync frame.locals
                        // using the slotmap so that JIT code can read correct values
                        if let Some(slot) = slot_idx {
                            if slot >= frame.locals.len() {
                                frame.locals.resize(slot + 1, Value::NIL);
                            }
                            let old = frame.get_local(slot);
                            decref_discard(&mut self.struct_registry, old);
                            frame.set_local(slot, value);
                            local_slot_written = true;
                        }
                    }

                    // 1. Try to update closure_scope first (for mutable closures)
                    let mut stored_in_closure = false;
                    if let Some(ref closure) = frame.closure_scope {
                        if closure.resolve_with_py(py, name.as_str()).is_some() {
                            closure.set_with_py(py, name.as_str(), value);
                            stored_in_closure = true;
                        }
                    }

                    // 2. Store to local slot if name is in slotmap
                    // Skip if already written above (same slot) to avoid double-decref.
                    // Also skip if the slot already holds `value` (StoreLocal ran before
                    // StoreScope in the DupTop;StoreLocal;StoreScope sequence).
                    if !local_slot_written {
                        if let Some(idx) = slot_idx {
                            let old = frame.get_local(idx);
                            if old.bits() != value.bits() {
                                decref_discard(&mut self.struct_registry, old);
                                frame.set_local(idx, value);
                            }
                        }
                    }

                    // Keep VM globals in sync for module-level vars modified via closures
                    if stored_in_closure {
                        if let Some(&old_global) = self.globals.get(name.as_str()) {
                            decref_discard(&mut self.struct_registry, old_global);
                        }
                        value.clone_refcount();
                        if let Some(existing) = self.globals.get_mut(name.as_str()) {
                            *existing = value;
                        }
                        // Signal change even if this VM doesn't own the var
                        // (re-entrant VMs write to ctx_globals via closure chain)
                        GLOBALS_GEN.fetch_add(1, Ordering::Relaxed);
                    }

                    // 3. Store to globals for name resolution (if not in closure)
                    if !stored_in_closure {
                        // Decref old value in globals (BigInt/Struct refcount)
                        if let Some(&old_global) = self.globals.get(name.as_str()) {
                            decref_discard(&mut self.struct_registry, old_global);
                        }
                        // Incref new value going into globals (separate ownership)
                        value.clone_refcount();
                        if let Some(existing) = self.globals.get_mut(name.as_str()) {
                            *existing = value;
                        } else {
                            self.globals.insert(name.clone(), value);
                        }
                        GLOBALS_GEN.fetch_add(1, Ordering::Relaxed);
                        // Also sync to Python context.globals immediately
                        // so closures created later can access these values
                        host.store_global(py, name.as_str(), value)?;
                    }
                }

                OpCode::LoadGlobal => {
                    let name = get_name(code, instr.arg)?;
                    if let Some(&value) = self.globals.get(name.as_str()) {
                        value.clone_refcount_bigint();
                        if value.is_struct_instance() {
                            self.struct_registry.incref(value.as_struct_instance_idx().unwrap());
                        }
                        frame.push(value);
                    } else if let Some(value) = host.lookup_global(py, name.as_str())? {
                        value.clone_refcount_bigint();
                        if value.is_struct_instance() {
                            self.struct_registry.incref(value.as_struct_instance_idx().unwrap());
                        }
                        frame.push(value);
                    } else {
                        return Err(VMError::NameError(name.to_owned()));
                    }

                    // JIT: pure builtins are handled in the Call handler (record_builtin)
                    // Other globals: emit Fallback (trace not compilable)
                    if self.jit_tracing && self.jit_recursive_depth == 0 {
                        let is_pure_builtin = JIT_PURE_BUILTINS.contains(&name.as_str());
                        if !is_pure_builtin {
                            let ip = frame.ip - 1;
                            self.jit_recorder.record_fallback(OpCode::LoadGlobal, ip);
                        }
                    }
                }

                // --- Stack manipulation ---
                OpCode::PopTop => {
                    let val = frame.pop();
                    decref_discard(&mut self.struct_registry, val);
                }

                OpCode::DupTop => {
                    let value = frame.peek();
                    value.clone_refcount();
                    frame.push(value);
                }

                OpCode::RotTwo => {
                    let len = frame.stack.len();
                    if len >= 2 {
                        frame.stack.swap(len - 1, len - 2);
                    }
                }

                // --- Arithmetic (binary) ---
                OpCode::Add => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match binary_add(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            // Struct operator overload (stays in VM)
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_add")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_add"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // Fallback to Python for strings, lists, etc.
                            host.binary_op(py, BinaryOp::Add, a, b)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Sub => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match binary_sub(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_sub")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_sub"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            host.binary_op(py, BinaryOp::Sub, a, b)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Mul => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match binary_mul(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_mul")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_mul"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            // Fallback to Python for string * int, etc.
                            host.binary_op(py, BinaryOp::Mul, a, b)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Div => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_div")
                        .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_div"))
                    {
                        let mut new_frame = Frame::with_code(code);
                        for (i, arg) in args.iter().enumerate() {
                            new_frame.set_local(i, *arg);
                        }
                        new_frame.closure_scope = closure;
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    }
                    let result = match binary_div(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            inc(&PY_BINARY_DIV_FALLBACKS);
                            host.binary_op(py, BinaryOp::TrueDiv, a, b)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::FloorDiv => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some((code, closure, args)) =
                        try_struct_binop(&self.struct_registry, py, a, b, "op_floordiv")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_floordiv"))
                    {
                        let mut new_frame = Frame::with_code(code);
                        for (i, arg) in args.iter().enumerate() {
                            new_frame.set_local(i, *arg);
                        }
                        new_frame.closure_scope = closure;
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    }
                    let result = match binary_floordiv(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            inc(&PY_BINARY_FLOORDIV_FALLBACKS);
                            host.binary_op(py, BinaryOp::FloorDiv, a, b)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Mod => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_mod")
                        .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_mod"))
                    {
                        let mut new_frame = Frame::with_code(code);
                        for (i, arg) in args.iter().enumerate() {
                            new_frame.set_local(i, *arg);
                        }
                        new_frame.closure_scope = closure;
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    }
                    let result = match binary_mod(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            inc(&PY_BINARY_MOD_FALLBACKS);
                            host.binary_op(py, BinaryOp::Mod, a, b)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Pow => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match binary_pow(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_pow")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_pow"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            host.binary_op(py, BinaryOp::Pow, a, b)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                // --- Arithmetic (unary) ---
                OpCode::Neg => {
                    let a = frame.pop();
                    let result = match unary_neg(a) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_unaryop(&self.struct_registry, py, a, "op_neg")
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            let py_a = a.to_pyobject(py);
                            let py_result = py_a.call_method0(py, "__neg__").to_vm(py)?;
                            Value::from_pyobject(py, py_result.bind(py)).to_vm(py)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Pos => {
                    let a = frame.peek();
                    if a.as_struct_instance_idx().is_some() {
                        frame.pop();
                        if let Some((code, closure, args)) = try_struct_unaryop(&self.struct_registry, py, a, "op_pos")
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                        return Err(VMError::TypeError("bad operand type for unary +: struct".to_string()));
                    }
                    // No-op for native numbers
                }

                // --- Bitwise ---
                OpCode::BOr => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_or(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_bor")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_bor"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            return Err(VMError::TypeError("unsupported operand type(s) for |".to_string()));
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::BXor => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_xor(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_bxor")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_bxor"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            return Err(VMError::TypeError("unsupported operand type(s) for ^".to_string()));
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::BAnd => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_and(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_band")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_band"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            return Err(VMError::TypeError("unsupported operand type(s) for &".to_string()));
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::BNot => {
                    let a = frame.pop();
                    let result = match bitwise_not(a) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_unaryop(&self.struct_registry, py, a, "op_bnot")
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            return Err(VMError::TypeError(errors::ERR_BAD_UNARY_NOT.to_string()));
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::LShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_lshift(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_lshift")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_lshift"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            return Err(VMError::TypeError("unsupported operand type(s) for <<".to_string()));
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::RShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match bitwise_rshift(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => {
                            if let Some((code, closure, args)) =
                                try_struct_binop(&self.struct_registry, py, a, b, "op_rshift")
                                    .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_rshift"))
                            {
                                let mut new_frame = Frame::with_code(code);
                                for (i, arg) in args.iter().enumerate() {
                                    new_frame.set_local(i, *arg);
                                }
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            }
                            return Err(VMError::TypeError("unsupported operand type(s) for >>".to_string()));
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                // --- Comparison ---
                OpCode::Lt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_lt")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_gt"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = match compare_lt(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => host.binary_op(py, BinaryOp::Lt, a, b)?,
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Le => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_le")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_ge"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = match compare_le(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => host.binary_op(py, BinaryOp::Le, a, b)?,
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Gt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_gt")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_lt"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = match compare_gt(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => host.binary_op(py, BinaryOp::Gt, a, b)?,
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Ge => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_ge")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_le"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = match compare_ge(a, b) {
                        Ok(v) => v,
                        Err(VMError::TypeError(_)) => host.binary_op(py, BinaryOp::Ge, a, b)?,
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Eq => {
                    let b = frame.pop();
                    let a = frame.pop();
                    // Try op_eq method first
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_eq")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_eq"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    // Fallback: structural equality for structs
                    let result =
                        if let (Some(idx_a), Some(idx_b)) = (a.as_struct_instance_idx(), b.as_struct_instance_idx()) {
                            let inst_a = self.struct_registry.get_instance(idx_a).unwrap();
                            let inst_b = self.struct_registry.get_instance(idx_b).unwrap();
                            if inst_a.type_id != inst_b.type_id {
                                Value::FALSE
                            } else {
                                let mut equal = true;
                                for (fa, fb) in inst_a.fields.iter().zip(inst_b.fields.iter()) {
                                    let eq = compare_eq(py, *fa, *fb)?;
                                    if eq.as_bool() != Some(true) {
                                        equal = false;
                                        break;
                                    }
                                }
                                Value::from_bool(equal)
                            }
                        } else {
                            compare_eq(py, a, b)?
                        };
                    frame.push(result);
                }

                OpCode::Ne => {
                    let b = frame.pop();
                    let a = frame.pop();
                    // Try op_ne method first
                    if a.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, a, b, "op_ne")
                            .or_else(|| try_struct_rbinop(&self.struct_registry, py, a, b, "op_ne"))
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    // Fallback: structural inequality for structs
                    let result =
                        if let (Some(idx_a), Some(idx_b)) = (a.as_struct_instance_idx(), b.as_struct_instance_idx()) {
                            let inst_a = self.struct_registry.get_instance(idx_a).unwrap();
                            let inst_b = self.struct_registry.get_instance(idx_b).unwrap();
                            if inst_a.type_id != inst_b.type_id {
                                Value::TRUE
                            } else {
                                let mut not_equal = false;
                                for (fa, fb) in inst_a.fields.iter().zip(inst_b.fields.iter()) {
                                    let eq = compare_eq(py, *fa, *fb)?;
                                    if eq.as_bool() != Some(true) {
                                        not_equal = true;
                                        break;
                                    }
                                }
                                Value::from_bool(not_equal)
                            }
                        } else {
                            compare_ne(py, a, b)?
                        };
                    frame.push(result);
                }

                // --- Membership ---
                OpCode::In => {
                    let b = frame.pop(); // container
                    let a = frame.pop(); // item
                    // Try op_in method on container
                    if b.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) = try_struct_binop(&self.struct_registry, py, b, a, "op_in")
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = host.contains_op(py, a, b)?;
                    frame.push(result);
                }
                OpCode::NotIn => {
                    let b = frame.pop(); // container
                    let a = frame.pop(); // item
                    // Dispatch op_not_in on struct (mirrors Eq/Ne pattern)
                    if b.as_struct_instance_idx().is_some() {
                        if let Some((code, closure, args)) =
                            try_struct_binop(&self.struct_registry, py, b, a, "op_not_in")
                        {
                            let mut new_frame = Frame::with_code(code);
                            for (i, arg) in args.iter().enumerate() {
                                new_frame.set_local(i, *arg);
                            }
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                    }
                    let result = host.contains_op(py, a, b)?;
                    let negated = Value::from_bool(!result.is_truthy_py(py));
                    frame.push(negated);
                }

                // --- Identity ---
                OpCode::Is => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = a.is_identical(py, b);
                    frame.push(Value::from_bool(result));
                }
                OpCode::IsNot => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = !a.is_identical(py, b);
                    frame.push(Value::from_bool(result));
                }

                // --- Logic ---
                OpCode::Not => {
                    let a = frame.pop();
                    let result = Value::from_bool(!a.is_truthy_py(py));
                    frame.push(result);
                }

                // --- Control flow ---
                OpCode::Jump => {
                    let target = instr.arg as usize;
                    let is_backward = target < frame.ip;

                    // Backward jump = loop, check for JIT opportunities
                    if is_backward && self.jit_enabled {
                        let loop_offset = target;
                        let is_for_range_header = frame
                            .code
                            .as_ref()
                            .and_then(|code| code.instructions.get(loop_offset))
                            .map(|instr| instr.op == OpCode::ForRangeInt)
                            .unwrap_or(false);

                        if !is_for_range_header {
                            // Check if we have compiled code for this loop
                            if !self.jit_tracing {
                                let has_compiled = {
                                    let jit = self.jit.lock().unwrap();
                                    jit.as_ref().map(|e| e.has_compiled(loop_offset)).unwrap_or(false)
                                };

                                if has_compiled {
                                    // Validate guards before executing JIT code
                                    let guards = {
                                        let jit = self.jit.lock().unwrap();
                                        jit.as_ref().and_then(|e| e.get_guards(loop_offset)).cloned()
                                    };

                                    let mut guards_pass = true;
                                    let mut guard_locals: Vec<(usize, i64)> = Vec::new();

                                    if let Some(ref guards) = guards {
                                        for (name, expected_value, slot) in guards {
                                            // Resolve current value of name
                                            let current_value = resolve_jit_guard_value(
                                                py,
                                                name,
                                                &frame.closure_scope,
                                                host,
                                                &self.globals,
                                            );

                                            match current_value {
                                                Some(val) if val == *expected_value => {
                                                    // Guard passed - store value for this slot
                                                    guard_locals.push((*slot, val));
                                                }
                                                _ => {
                                                    // Guard failed - skip JIT execution
                                                    guards_pass = false;
                                                    break;
                                                }
                                            }
                                        }
                                    }

                                    // Skip JIT if any local holds a non-SmallInt value
                                    // (BigInt, PyObj, etc.). The JIT operates on raw i64
                                    // and can't handle heap-allocated types.
                                    if guards_pass {
                                        for v in frame.locals.iter() {
                                            if !(v.is_int() || v.is_bool() || v.is_nil()) {
                                                guards_pass = false;
                                                break;
                                            }
                                        }
                                    }

                                    if guards_pass {
                                        // Execute compiled code (pass NaN-boxed bits)
                                        let mut locals_i64: Vec<i64> =
                                            frame.locals.iter().map(|v| v.bits() as i64).collect();

                                        // Extend locals array for LoadScope slots
                                        let max_slot = guard_locals.iter().map(|(s, _)| s).max().copied();
                                        if let Some(max_slot) = max_slot {
                                            if max_slot >= locals_i64.len() {
                                                locals_i64.resize(max_slot + 1, 0);
                                            }
                                        }

                                        // Copy guard values into locals array
                                        for (slot, value) in guard_locals {
                                            locals_i64[slot] = value;
                                        }

                                        // Snapshot pre-JIT values to detect which slots changed
                                        let snapshot: Vec<i64> = locals_i64.clone();

                                        let result = {
                                            let jit = self.jit.lock().unwrap();
                                            if let Some(ref executor) = *jit {
                                                unsafe { executor.execute(loop_offset, &mut locals_i64) }
                                            } else {
                                                None
                                            }
                                        };

                                        if let Some(_ret) = result {
                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Executed compiled trace for while loop at {}",
                                                    loop_offset
                                                );
                                            }
                                            // Restore only slots actually modified by JIT
                                            // (values are NaN-boxed by codegen)
                                            for (i, &val) in locals_i64.iter().enumerate() {
                                                if i < frame.locals.len() && val != snapshot[i] {
                                                    let new_val = Value::from_raw(val as u64);
                                                    let old = frame.locals[i];
                                                    decref_discard(&mut self.struct_registry, old);
                                                    frame.locals[i] = new_val;
                                                    if i < code.varnames.len() {
                                                        host.store_global(py, &code.varnames[i], new_val)?;
                                                    }
                                                }
                                            }
                                            // Loop completed, jump to condition check
                                            frame.ip = target;
                                            continue;
                                        }
                                    }
                                    // If guards didn't pass, fall through to interpreter
                                }
                            }

                            // If we're tracing and jumping back to loop start
                            if self.jit_tracing && self.jit_tracing_offset == loop_offset {
                                let ip = frame.ip - 1;
                                self.jit_recorder.record_loop_back(ip);
                                // Stop after 1 iteration
                                const TRACE_SINGLE_ITERATIONS: u32 = 1;
                                if self.jit_recorder.iterations() >= TRACE_SINGLE_ITERATIONS {
                                    let trace = self.jit_recorder.stop();
                                    self.jit_tracing = false;

                                    if let Some(t) = trace {
                                        if self.trace {
                                            eprintln!(
                                                "[JIT] While loop trace recorded: {} ops, {} iterations",
                                                t.ops.len(),
                                                t.iterations
                                            );
                                        }
                                        if t.is_compilable() {
                                            let mut jit = self.jit.lock().unwrap();
                                            if let Some(ref mut executor) = *jit {
                                                match executor.compile_trace(t) {
                                                    Ok(true) => {
                                                        if self.trace {
                                                            eprintln!("[JIT] While loop trace compiled!");
                                                        }
                                                    }
                                                    Ok(false) => {
                                                        if self.trace {
                                                            eprintln!("[JIT] While loop trace not compilable");
                                                        }
                                                    }
                                                    Err(e) => {
                                                        if self.trace {
                                                            eprintln!("[JIT] While loop compilation failed: {}", e);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if !self.jit_tracing {
                                // Warm-start: check trace cache on first encounter
                                if !self.jit_cache_checked.contains(&loop_offset) {
                                    self.jit_cache_checked.insert(loop_offset);
                                    let hit = {
                                        let mut jit = self.jit.lock().unwrap();
                                        jit.as_mut()
                                            .map(|e| e.try_compile_from_cache(loop_offset))
                                            .unwrap_or(false)
                                    };
                                    if hit && self.trace {
                                        eprintln!("[JIT] Warm-start: while loop at {} loaded from cache", loop_offset);
                                    }
                                }

                                // Not tracing, check if loop becomes hot
                                if self.jit_detector.record_loop_header(loop_offset) {
                                    // Try cache first - skip recording if trace already cached
                                    let compiled_from_cache = {
                                        let mut jit = self.jit.lock().unwrap();
                                        jit.as_mut()
                                            .map(|e| e.try_compile_from_cache(loop_offset))
                                            .unwrap_or(false)
                                    };
                                    if compiled_from_cache {
                                        if self.trace {
                                            eprintln!("[JIT] While loop at offset {} compiled from cache", loop_offset);
                                        }
                                    } else {
                                        // Cache miss - start tracing
                                        let num_locals = frame.locals.len();
                                        self.jit_recorder.start(loop_offset, num_locals);
                                        self.jit_tracing = true;
                                        self.jit_tracing_offset = loop_offset;

                                        if self.trace {
                                            eprintln!("[JIT] Started tracing while loop at offset {}", loop_offset);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    frame.ip = target;
                }

                OpCode::JumpIfFalse => {
                    let cond = frame.pop();
                    let took_jump = !cond.is_truthy_py(py);
                    if took_jump {
                        frame.ip = instr.arg as usize;
                    }
                    // Record for JIT after execution (we now know if we jumped)
                    // Only record if not suspended in recursive call
                    if self.jit_tracing && self.jit_recursive_depth == 0 {
                        let ip = frame.ip.saturating_sub(1);
                        self.jit_recorder.record_conditional_jump(took_jump, true, ip);
                    }
                }

                OpCode::JumpIfTrue => {
                    let cond = frame.pop();
                    let took_jump = cond.is_truthy_py(py);
                    if took_jump {
                        frame.ip = instr.arg as usize;
                    }
                    // Record for JIT after execution
                    if self.jit_tracing {
                        let ip = frame.ip.saturating_sub(1);
                        self.jit_recorder.record_conditional_jump(took_jump, false, ip);
                    }
                }

                OpCode::JumpIfFalseOrPop => {
                    let cond = frame.peek();
                    if !cond.is_truthy_py(py) {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                    }
                }

                OpCode::JumpIfTrueOrPop => {
                    let cond = frame.peek();
                    if cond.is_truthy_py(py) {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                    }
                }

                // --- Iteration ---
                OpCode::GetIter => {
                    let obj = frame.pop();
                    let py_obj = obj.to_pyobject(py);
                    let py_obj_bound = py_obj.bind(py);

                    if let Ok(list) = py_obj_bound.cast::<PyList>() {
                        let iter = Py::new(py, SeqIter::from_list(list).to_vm(py)?).to_vm(py)?;
                        frame.push(Value::from_owned_pyobject(iter.into_any()));
                        continue;
                    }

                    if let Ok(tuple) = py_obj_bound.cast::<PyTuple>() {
                        let iter = Py::new(py, SeqIter::from_tuple(tuple).to_vm(py)?).to_vm(py)?;
                        frame.push(Value::from_owned_pyobject(iter.into_any()));
                        continue;
                    }

                    // Fallback: use host's iter() builtin
                    let iterator = host.get_iter(py, py_obj_bound)?;
                    frame.push(Value::from_owned_pyobject(iterator.unbind()));
                }

                OpCode::ForIter => {
                    // TOS is the iterator. Try to get next item.
                    // If exhausted, jump to end of loop (arg is jump target).
                    let iter_val = frame.peek();
                    let py_iter = iter_val.to_pyobject(py);
                    let py_iter_bound = py_iter.bind(py);

                    if let Ok(iter_ref) = py_iter_bound.cast::<SeqIter>() {
                        let mut iter = iter_ref.borrow_mut();
                        match iter.next_value(py).to_vm(py)? {
                            Some(value) => frame.push(value),
                            None => {
                                frame.pop();
                                frame.ip = instr.arg as usize;
                            }
                        }
                        continue;
                    }

                    // Fallback: use CPython tp_iternext directly (avoids sentinel + clone_ref)
                    // Safety: verify tp_iternext is non-NULL to avoid segfault on
                    // corrupted stack or objects that slipped past GetIter
                    let tp_iternext = unsafe { (*(*py_iter_bound.as_ptr()).ob_type).tp_iternext };
                    if tp_iternext.is_none() {
                        let type_name = py_iter_bound
                            .get_type()
                            .name()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|_| "?".to_string());
                        return Err(VMError::RuntimeError(format!(
                            "ForIter: object of type '{type_name}' is not a valid iterator (NULL tp_iternext)"
                        )));
                    }
                    let next_ptr = unsafe { pyo3::ffi::PyIter_Next(py_iter_bound.as_ptr()) };
                    if next_ptr.is_null() {
                        // NULL = exhausted (StopIteration) or real error
                        if let Some(err) = PyErr::take(py) {
                            if !err.is_instance_of::<pyo3::exceptions::PyStopIteration>(py) {
                                return Err(VMError::RuntimeError(err.to_string()));
                            }
                        }
                        frame.pop();
                        frame.ip = instr.arg as usize;
                    } else {
                        let result = unsafe { pyo3::Bound::from_owned_ptr(py, next_ptr) };
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);
                    }
                }

                // --- Function calls ---
                OpCode::Call => {
                    let nargs = instr.arg as usize;
                    // Read args in order from stack, then pop all + function
                    let stack_len = frame.stack.len();
                    let args_start = stack_len - nargs;

                    // Peek at the function (below args) to try the fast path
                    // before allocating a Vec for args
                    let func_pos = args_start - 1;
                    let func = frame.stack[func_pos];

                    // FAST PATH: native VM function (skip PyO3 boundary + avoid Vec alloc)
                    if func.is_vmfunc() {
                        let idx = func.as_vmfunc_idx();
                        let (new_code, native_closure) = {
                            let slot = self.func_table.get(idx).ok_or_else(|| {
                                VMError::RuntimeError(format!(
                                    "invalid function index {idx} (table has {} entries)",
                                    self.func_table.slots.len()
                                ))
                            })?;
                            (Arc::clone(&slot.code), slot.closure.clone())
                        };
                        let func_id = new_code.func_id();

                        // JIT trace recording
                        if self.jit_recorder.is_recording() {
                            let ip = frame.ip - 1;
                            let is_recursive = self.jit_tracing_func_id.as_ref() == Some(&func_id);
                            if is_recursive {
                                if self.jit_recursive_depth == 0 {
                                    self.jit_recorder.record_call(&func_id, nargs, new_code.is_pure, ip);
                                }
                                self.jit_recursive_depth += 1;
                            } else {
                                self.jit_recorder.record_call(&func_id, nargs, new_code.is_pure, ip);
                            }
                        }

                        // JIT pure function registration + hot detection
                        if self.jit_enabled {
                            if new_code.is_pure {
                                if let Ok(mut jit) = self.jit.lock() {
                                    if let Some(ref mut executor) = *jit {
                                        crate::jit::executor::register_pure_function(
                                            executor,
                                            func_id.clone(),
                                            &new_code,
                                        );
                                    }
                                }
                            }
                            self.jit_detector.record_call_internal(&func_id);
                        }

                        // Setup new frame - copy args directly from caller stack
                        let jit_hash = new_code.bytecode_hash();
                        let call_start_byte = _current_src_byte;
                        let fn_name = new_code.name.clone();
                        let has_varargs = new_code.vararg_idx >= 0;
                        // Snapshot args into inline buffer, then release frame borrow
                        // to access self.frame_pool
                        let mut arg_buf = [Value::NIL; 8];
                        let use_pool = !has_varargs && nargs <= 8;
                        if use_pool {
                            arg_buf[..nargs].copy_from_slice(&frame.stack[args_start..(nargs + args_start)]);
                            frame.stack.truncate(func_pos);
                        }
                        // Allocate frame: pool (fast) or new (fallback)
                        let mut new_frame = if use_pool {
                            self.frame_pool.alloc_with_code(new_code)
                        } else {
                            Frame::with_code(new_code)
                        };
                        if use_pool {
                            let nparams = new_frame.locals.len().min(nargs);
                            new_frame.locals[..nparams].copy_from_slice(&arg_buf[..nparams]);
                            // Fill defaults for missing args
                            if let Some(ref fc) = new_frame.code {
                                let code_nargs = fc.nargs;
                                let ndefaults = fc.defaults.len();
                                if ndefaults > 0 && nargs < code_nargs {
                                    let default_start = code_nargs.saturating_sub(ndefaults);
                                    for i in nargs.max(default_start)..code_nargs {
                                        let default_idx = i - default_start;
                                        if default_idx < ndefaults {
                                            let val = fc.defaults[default_idx];
                                            val.clone_refcount();
                                            new_frame.locals[i] = val;
                                        }
                                    }
                                }
                            }
                        } else {
                            // Varargs or >8 args: use bind_args with Vec
                            let args: Vec<Value> = frame.stack[args_start..args_start + nargs].to_vec();
                            frame.stack.truncate(func_pos);
                            new_frame.bind_args(py, &args, None);
                        }
                        new_frame.closure_scope = native_closure;
                        if self.jit_enabled {
                            if let Ok(mut jit) = self.jit.lock() {
                                if let Some(ref mut executor) = *jit {
                                    executor.set_bytecode_hash(jit_hash);
                                }
                            }
                        }
                        self.call_stack.push(CallInfo {
                            name: fn_name,
                            call_start_byte,
                        });
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    }

                    // SLOW PATH: pop args into Vec
                    let args: Vec<Value> = frame.stack[args_start..].to_vec();
                    frame.stack.truncate(args_start);
                    // Pop function
                    frame.pop();

                    // SLOW PATH: PyObject (struct instantiation, bound methods, Python callables)
                    let py_func = func.to_pyobject(py);
                    let py_func_bound = py_func.bind(py);

                    // ND recursion fast path: push frame instead of creating new VM
                    if let Ok(nd_recur) = py_func_bound.cast::<crate::nd::NDVmRecur>() {
                        let r = nd_recur.borrow();
                        if let Some(code) = r.vm_code_arc().cloned() {
                            // Depth guard
                            let depth = r.depth_cell().get();
                            if depth >= ND_MAX_RECURSION_DEPTH {
                                drop(r);
                                crate::nd::set_nd_abort();
                                return Err(VMError::RuntimeError("maximum ND recursion depth exceeded".to_string()));
                            }

                            // Memo cache check
                            let key = if r.is_memoize() && !args.is_empty() {
                                let py_val = args[0].to_pyobject(py);
                                py_val.bind(py).hash().ok().map(|h| h as u64)
                            } else {
                                None
                            };
                            let cache_hit = if let Some(k) = key {
                                let guard = r.cache_ref().borrow();
                                guard.get(&k).map(|c| c.clone_ref(py))
                            } else {
                                None
                            };
                            if let Some(cached) = cache_hit {
                                drop(r);
                                let value = Value::from_pyobject(py, cached.bind(py))
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                frame.push(value);
                                continue;
                            }

                            // Increment depth
                            r.depth_cell().set(depth + 1);
                            let closure = r.vm_closure_ref().cloned();
                            drop(r);

                            // Build args: [value, recur] - inject self as 2nd arg
                            let recur_value = Value::from_pyobject(py, py_func_bound)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let mut lambda_args = Vec::with_capacity(args.len() + 1);
                            lambda_args.extend_from_slice(&args);
                            lambda_args.push(recur_value);

                            let caller_depth = self.frame_stack.len();
                            let mut new_frame = Frame::with_code(code);
                            new_frame.bind_args(py, &lambda_args, None);
                            new_frame.closure_scope = closure;

                            self.nd_recur_stack.push(NdRecurEntry {
                                caller_depth,
                                recur_py: py_func.clone_ref(py),
                                memo_key: key,
                            });

                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                        // No vm_code: fall through to Python slow path
                    }

                    // Native struct instantiation (fast path)
                    {
                        let ptr = py_func_bound.as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            check_abstract_guard(&self.struct_registry, type_id)?;
                            // Extract type info before mutable borrow
                            let (num_fields, min_args, type_name, defaults, init_func) = {
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                let nf = ty.fields.len();
                                let ma = ty.fields.iter().filter(|f| !f.has_default).count();
                                let tn = ty.name.clone();
                                let defs: Vec<Value> = ty.fields.iter().map(|f| f.default).collect();
                                let init = ty.methods.get("init").map(|f| f.clone_ref(py));
                                (nf, ma, tn, defs, init)
                            };
                            if nargs < min_args {
                                return Err(VMError::TypeError(format!(
                                    "{}() missing {} required argument(s)",
                                    type_name,
                                    min_args - nargs
                                )));
                            }
                            if nargs > num_fields {
                                return Err(VMError::TypeError(format!(
                                    "{}() takes {} argument(s) but {} were given",
                                    type_name, num_fields, nargs
                                )));
                            }
                            let mut field_values = args;
                            field_values.extend(defaults.iter().take(num_fields).skip(nargs).copied());
                            let idx = self.struct_registry.create_instance(type_id, field_values);
                            let inst_val = Value::from_struct_instance(idx);

                            if self.push_struct_init_frame(py, inst_val, init_func, frame)? {
                                continue;
                            }
                            frame.push(inst_val);
                            continue;
                        }
                    }

                    // Unwrap BoundCatnipMethod: extract inner func, prepend instance to args
                    let (actual_func_ref, unwrapped_args);
                    let actual_func: &Bound<'_, PyAny>;
                    let mut bound_instance: Option<Value> = None;
                    let mut super_source_type: Option<String> = None;
                    if let Ok(bound_method) = py_func_bound.cast::<crate::core::BoundCatnipMethod>() {
                        let bm = bound_method.borrow();
                        actual_func_ref = bm.func.bind(py).clone();
                        // Use native struct index if available (avoids CatnipStructProxy round-trip)
                        let instance_val = if let Some(idx) = bm.native_instance_idx {
                            self.struct_registry.incref(idx);
                            Value::from_struct_instance(idx)
                        } else {
                            Value::from_pyobject(py, bm.instance.bind(py)).to_vm(py)?
                        };
                        bound_instance = Some(instance_val);
                        super_source_type = bm.super_source_type.clone();
                        let mut new_args = Vec::with_capacity(args.len() + 1);
                        new_args.push(instance_val);
                        new_args.extend_from_slice(&args);
                        unwrapped_args = new_args;
                        actual_func = &actual_func_ref;
                    } else {
                        actual_func_ref = py_func_bound.clone();
                        unwrapped_args = args;
                        actual_func = &actual_func_ref;
                    }
                    // Shadow args with potentially prepended self
                    let args = unwrapped_args;
                    let nargs = args.len();

                    // Check if this is a VMFunction (fast Rust cast, then fallback)
                    let vm_func_data: Option<(Arc<CodeObject>, Option<NativeClosureScope>)> =
                        if let Ok(vm_func) = actual_func.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            Some((code, closure))
                        } else if let Ok(vm_code) = actual_func.getattr("vm_code") {
                            Some((convert_code_object(py, &vm_code).to_vm(py)?, None))
                        } else {
                            None
                        };
                    if let Some((new_code, native_closure)) = vm_func_data {
                        let func_id = new_code.func_id();

                        // Register pure function for JIT inlining
                        if new_code.is_pure && self.jit_enabled {
                            let mut jit = self.jit.lock().unwrap();
                            if let Some(ref mut executor) = *jit {
                                crate::jit::executor::register_pure_function(executor, func_id.clone(), &new_code);
                            }
                        }

                        // JIT: Handle recursive calls - check BEFORE recording
                        if self.jit_recorder.is_recording() {
                            let ip = frame.ip - 1; // Call instruction was just executed

                            // Check if this is a recursive call (calling the function being traced)
                            let is_recursive_call = if let Some(ref tracing_func_id) = self.jit_tracing_func_id {
                                &func_id == tracing_func_id
                            } else {
                                false
                            };

                            if is_recursive_call {
                                // Only record the FIRST CallSelf (when depth=0)
                                // Then increment depth to suspend further recording
                                if self.jit_recursive_depth == 0 {
                                    self.jit_recorder.record_call(&func_id, nargs, new_code.is_pure, ip);
                                }
                                self.jit_recursive_depth += 1;
                            } else {
                                // Non-recursive call - record normally
                                self.jit_recorder.record_call(&func_id, nargs, new_code.is_pure, ip);
                            }
                        }

                        // JIT: Check if function is already compiled
                        let mut use_compiled = false;
                        if self.jit_enabled {
                            if self.jit_detector.is_compiled_internal(&func_id) {
                                // Function is compiled - try to use native code
                                let jit = self.jit.lock().unwrap();
                                if let Some(ref executor) = *jit {
                                    if let Some((compiled_fn, max_slot, fn_guards)) =
                                        executor.get_compiled_function(&func_id)
                                    {
                                        // Call compiled native code via locals array
                                        // Setup locals array with enough space for all used slots
                                        let array_size = (max_slot + 1).max(new_code.nlocals);
                                        let mut locals_array: Vec<i64> = vec![0; array_size];

                                        // Copy arguments to first N slots (native i64, not NaN-boxed)
                                        for (i, arg) in args.iter().enumerate() {
                                            if i < array_size {
                                                locals_array[i] = arg.as_int().unwrap_or(0);
                                            }
                                        }

                                        // Populate captured variable slots from name_guards
                                        let fn_guards = fn_guards.to_vec();
                                        let mut guards_passed = true;
                                        for (name, expected_value, slot) in &fn_guards {
                                            // Resolve current value of captured variable
                                            let current_value = resolve_jit_guard_value(
                                                py,
                                                name,
                                                &frame.closure_scope,
                                                host,
                                                &self.globals,
                                            );

                                            match current_value {
                                                Some(val) if val == *expected_value => {
                                                    if *slot < locals_array.len() {
                                                        locals_array[*slot] = val;
                                                    }
                                                }
                                                _ => {
                                                    // Guard failed: fall back to interpreter
                                                    guards_passed = false;
                                                    break;
                                                }
                                            }
                                        }

                                        if !guards_passed {
                                            // Guard failed, skip to interpreter path
                                        } else {
                                            // Call compiled function with locals pointer and depth=0
                                            // Phase 3: Initial call starts at depth 0
                                            // Safety: locals_array has enough elements for all used slots
                                            let result_raw = unsafe { compiled_fn(locals_array.as_mut_ptr(), 0) };

                                            // Check for guard failure (-1 = side exit needed)
                                            if result_raw == -1 {
                                                // Guard failure: fall back to interpreter
                                                if self.trace {
                                                    eprintln!(
                                                        "[JIT] Guard failure in compiled function {}, falling back to interpreter",
                                                        func_id
                                                    );
                                                }
                                                use_compiled = false;
                                            } else {
                                                // Normal return: push result value
                                                if self.trace {
                                                    eprintln!(
                                                        "[JIT] Called compiled function: {}, result_raw = {:#x}",
                                                        func_id, result_raw
                                                    );
                                                }

                                                let result_value = Value::from_raw(result_raw as u64);
                                                frame.push(result_value);
                                                use_compiled = true;
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Check if this function has a pending trace from previous hot detection
                                if let Some(ref pending_func_id) = self.jit_pending_function_trace.clone() {
                                    if pending_func_id == &func_id {
                                        // Check if this is a top-level call (not recursive)
                                        let is_recursive = self.frame_stack.iter().any(|f| {
                                            if let Some(ref code) = f.code {
                                                code.name == new_code.name
                                            } else {
                                                false
                                            }
                                        });

                                        if !is_recursive && !self.jit_tracing {
                                            // Start tracing this top-level call
                                            self.jit_recorder.start_function(
                                                func_id.clone(),
                                                new_code.nlocals,
                                                new_code.nargs,
                                            );
                                            self.jit_tracing = true;
                                            self.jit_tracing_func_id = Some(func_id.clone());
                                            self.jit_tracing_depth = self.frame_stack.len() + 2; // Depth after frame push (current frame not on stack)
                                            self.jit_pending_function_trace = None; // Clear pending

                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Started tracing function '{}' (params: {}) [pending → top-level]",
                                                    new_code.name, new_code.nargs
                                                );
                                            }
                                        }
                                    }
                                }

                                // Track function calls for profiling
                                let became_hot = self.jit_detector.record_call_internal(&func_id);

                                if became_hot {
                                    if self.trace {
                                        eprintln!("[JIT] Function '{}' became hot (id: {})", new_code.name, func_id);
                                    }

                                    // Check if this is a recursive call (function already in call stack)
                                    let is_recursive = self.frame_stack.iter().any(|f| {
                                        if let Some(ref code) = f.code {
                                            code.name == new_code.name
                                        } else {
                                            false
                                        }
                                    });

                                    // If became hot during recursive call, schedule tracing for next top-level call
                                    if is_recursive {
                                        if !self.jit_tracing {
                                            self.jit_pending_function_trace = Some(func_id.clone());
                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Function became hot during recursion, pending trace for next top-level call"
                                                );
                                            }
                                        }
                                    } else {
                                        // Top-level call - start tracing
                                        if !self.jit_tracing {
                                            self.jit_recorder.start_function(
                                                func_id.clone(),
                                                new_code.nlocals,
                                                new_code.nargs,
                                            );
                                            self.jit_tracing = true;
                                            self.jit_tracing_func_id = Some(func_id);
                                            self.jit_tracing_depth = self.frame_stack.len() + 2; // Depth after frame push (current frame not on stack)

                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Started tracing function '{}' (params: {}) [top-level]",
                                                    new_code.name, new_code.nargs
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // If not using compiled code, execute via interpreter
                        if !use_compiled {
                            // Track call for stack traces
                            let call_start_byte = _current_src_byte;
                            let fn_name = new_code.name.clone();
                            let jit_hash = new_code.bytecode_hash();

                            // Create and setup new frame
                            let mut new_frame = Frame::with_code(new_code);
                            new_frame.bind_args(py, &args, None);
                            new_frame.closure_scope = native_closure;

                            // Setup super proxy if this is a bound method call on a struct with parent_methods
                            if let Some(inst_val) = bound_instance {
                                self.setup_super_proxy(py, inst_val, super_source_type, &mut new_frame)?;
                            }

                            self.update_jit_bytecode_hash_value(jit_hash);
                            self.call_stack.push(CallInfo {
                                name: fn_name,
                                call_start_byte,
                            });
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                        }
                        continue;
                    } else {
                        // Regular Python function - call directly

                        // Check if function needs context passed
                        let pass_context = actual_func
                            .getattr("pass_context")
                            .map(|attr| attr.is_truthy().unwrap_or(false))
                            .unwrap_or(false);

                        let mut args_py: Vec<Py<PyAny>> = Vec::with_capacity(args.len() + usize::from(pass_context));

                        if pass_context {
                            if let Some(ref ctx) = host.context() {
                                args_py.push(ctx.clone_ref(py));
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Function requires context but VM has no context available".to_string(),
                                ));
                            }
                        }

                        for arg in args.iter() {
                            args_py.push(arg.to_pyobject(py));
                        }

                        // JIT: record builtin pure calls as native ops
                        if self.jit_tracing && self.jit_recursive_depth == 0 {
                            if let Ok(qualname) =
                                actual_func.getattr("__qualname__").and_then(|n| n.extract::<String>())
                            {
                                let ip = frame.ip - 1;
                                let recorded = match (qualname.as_str(), nargs) {
                                    // Native builtins (Cranelift codegen)
                                    ("abs", 1) => {
                                        self.jit_recorder.record_builtin(TraceOp::AbsInt, ip);
                                        true
                                    }
                                    ("min", 2) => {
                                        self.jit_recorder.record_builtin(TraceOp::MinInt, ip);
                                        true
                                    }
                                    ("max", 2) => {
                                        self.jit_recorder.record_builtin(TraceOp::MaxInt, ip);
                                        true
                                    }
                                    ("round", 1) => {
                                        self.jit_recorder.record_builtin(TraceOp::RoundInt, ip);
                                        true
                                    }
                                    ("int", 1) => {
                                        self.jit_recorder.record_builtin(TraceOp::IntCastInt, ip);
                                        true
                                    }
                                    ("bool", 1) => {
                                        self.jit_recorder.record_builtin(TraceOp::BoolInt, ip);
                                        true
                                    }
                                    // Callback builtins (extern C dispatch)
                                    (name, n) => {
                                        if let Some(bid) = builtin_name_to_id(name) {
                                            self.jit_recorder.record_builtin(
                                                TraceOp::CallBuiltinPure {
                                                    builtin_id: bid,
                                                    num_args: n as u8,
                                                },
                                                ip,
                                            );
                                            true
                                        } else {
                                            false
                                        }
                                    }
                                };
                                if !recorded {
                                    self.jit_recorder.record_fallback(OpCode::Call, ip);
                                }
                            }
                        }

                        let gen_before = GLOBALS_GEN.load(Ordering::Relaxed);
                        let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                        let result = actual_func.call1(args_tuple).to_vm(py)?;
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);

                        // Sync globals back to local slots after Python call.
                        // Skip if no globals were mutated (most builtins don't re-enter VM).
                        // Uses static GLOBALS_GEN to detect re-entrant VMs that share ctx_globals.
                        if GLOBALS_GEN.load(Ordering::Relaxed) != gen_before {
                            if let Some(ref code) = frame.code {
                                let updates: Vec<(String, usize, Value)> = code
                                    .slotmap
                                    .iter()
                                    .filter_map(|(name, &slot_idx)| {
                                        // Skip native-tagged values (would lose tag through Python round-trip)
                                        let current = frame.get_local(slot_idx);
                                        if current.has_native_tag() {
                                            return None;
                                        }
                                        host.lookup_global(py, name.as_str())
                                            .ok()
                                            .flatten()
                                            .map(|v| (name.clone(), slot_idx, v))
                                    })
                                    .collect();
                                for (name, slot_idx, v) in updates {
                                    frame.set_local(slot_idx, v);
                                    // Keep self.globals in sync for subsequent LoadScope
                                    if let Some(existing) = self.globals.get_mut(name.as_str()) {
                                        *existing = v;
                                    }
                                }
                            }
                        }
                    }
                }

                OpCode::CallKw => {
                    // Decode: (nargs << 8) | nkw
                    const NARGS_SHIFT: u32 = 8;
                    const NKW_MASK: u32 = 0xFF;
                    let nargs = (instr.arg >> NARGS_SHIFT) as usize;
                    let nkw = (instr.arg & NKW_MASK) as usize;

                    // Pop kw_names tuple
                    let kw_names = frame.pop().to_pyobject(py);
                    let kw_names_bound = kw_names.bind(py);
                    let kw_names_tuple = kw_names_bound
                        .cast::<PyTuple>()
                        .map_err(|_| VMError::TypeError("expected tuple for kw_names".into()))?;

                    // Read kwargs + args in stack order, then truncate
                    let stack_len = frame.stack.len();
                    let total = nargs + nkw;
                    let start = stack_len - total;
                    let args: Vec<Value> = frame.stack[start..start + nargs].to_vec();
                    let kw_values: Vec<Value> = frame.stack[start + nargs..].to_vec();
                    frame.stack.truncate(start);

                    // Pop function
                    let func = frame.pop();

                    // Native struct instantiation with kwargs (fast path)
                    {
                        let py_func_tmp = func.to_pyobject(py);
                        let ptr = py_func_tmp.bind(py).as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            check_abstract_guard(&self.struct_registry, type_id)?;
                            // Extract type info before mutable borrow
                            let (type_name, field_defaults, field_info, init_func) = {
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                let tn = ty.name.clone();
                                let defs: Vec<(Value, bool)> =
                                    ty.fields.iter().map(|f| (f.default, f.has_default)).collect();
                                let fi: Vec<(String, bool)> =
                                    ty.fields.iter().map(|f| (f.name.clone(), f.has_default)).collect();
                                let init = ty.methods.get("init").map(|f| f.clone_ref(py));
                                (tn, defs, fi, init)
                            };

                            // Start with defaults
                            let mut field_values: Vec<Value> = field_defaults
                                .iter()
                                .map(|(def, has)| if *has { *def } else { Value::NIL })
                                .collect();

                            // Place positional args
                            for (i, val) in args.iter().enumerate() {
                                field_values[i] = *val;
                            }

                            // Place keyword args by name
                            for (i, val) in kw_values.iter().enumerate() {
                                let kw_name: String = kw_names_tuple.get_item(i).to_vm(py)?.extract().to_vm(py)?;
                                match field_info.iter().position(|(n, _)| n == &kw_name) {
                                    Some(idx) => field_values[idx] = *val,
                                    None => {
                                        return Err(VMError::TypeError(format!(
                                            "{}() got an unexpected keyword argument '{}'",
                                            type_name, kw_name
                                        )));
                                    }
                                }
                            }

                            // Validate no missing required fields
                            for (i, (fname, has_default)) in field_info.iter().enumerate() {
                                if !has_default && field_values[i].is_nil() && i >= nargs {
                                    return Err(VMError::TypeError(format!(
                                        "{}() missing required argument: '{}'",
                                        type_name, fname
                                    )));
                                }
                            }

                            let inst_idx = self.struct_registry.create_instance(type_id, field_values);
                            let inst_val = Value::from_struct_instance(inst_idx);
                            if self.push_struct_init_frame(py, inst_val, init_func, frame)? {
                                continue;
                            }
                            frame.push(inst_val);
                            continue;
                        }
                    }

                    let py_func = func.to_pyobject(py);
                    let py_func_bound = py_func.bind(py);

                    // Unwrap BoundCatnipMethod: extract inner func, prepend instance to args
                    let (actual_func_ref_kw, unwrapped_args_kw);
                    let actual_func_kw: &Bound<'_, PyAny>;
                    let mut bound_instance_kw: Option<Value> = None;
                    let mut super_source_type_kw: Option<String> = None;
                    if let Ok(bound_method) = py_func_bound.cast::<crate::core::BoundCatnipMethod>() {
                        let bm = bound_method.borrow();
                        actual_func_ref_kw = bm.func.bind(py).clone();
                        let instance_val = if let Some(idx) = bm.native_instance_idx {
                            self.struct_registry.incref(idx);
                            Value::from_struct_instance(idx)
                        } else {
                            Value::from_pyobject(py, bm.instance.bind(py)).to_vm(py)?
                        };
                        bound_instance_kw = Some(instance_val);
                        super_source_type_kw = bm.super_source_type.clone();
                        let mut new_args = Vec::with_capacity(args.len() + 1);
                        new_args.push(instance_val);
                        new_args.extend_from_slice(&args);
                        unwrapped_args_kw = new_args;
                        actual_func_kw = &actual_func_ref_kw;
                    } else {
                        actual_func_ref_kw = py_func_bound.clone();
                        unwrapped_args_kw = args;
                        actual_func_kw = &actual_func_ref_kw;
                    }
                    let args = unwrapped_args_kw;

                    // Build kwargs dict
                    let kwargs_dict = PyDict::new(py);
                    for (i, val) in kw_values.iter().enumerate() {
                        let name = kw_names_tuple.get_item(i).to_vm(py)?;
                        kwargs_dict.set_item(name, val.to_pyobject(py)).to_vm(py)?;
                    }

                    // Check if VMFunction (fast Rust cast, then fallback)
                    let vm_func_data_kw: Option<(Arc<CodeObject>, Option<NativeClosureScope>)> =
                        if let Ok(vm_func) = actual_func_kw.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            Some((code, closure))
                        } else if let Ok(vm_code) = actual_func_kw.getattr("vm_code") {
                            Some((convert_code_object(py, &vm_code).to_vm(py)?, None))
                        } else {
                            None
                        };
                    if let Some((new_code, native_closure)) = vm_func_data_kw {
                        let fn_name = new_code.name.clone();
                        let call_start_byte = _current_src_byte;
                        let mut new_frame = Frame::with_code(new_code);
                        new_frame.bind_args(py, &args, Some(&kwargs_dict));
                        new_frame.closure_scope = native_closure;

                        // Setup super proxy for bound method calls
                        if let Some(inst_val) = bound_instance_kw {
                            self.setup_super_proxy(py, inst_val, super_source_type_kw, &mut new_frame)?;
                        }

                        self.call_stack.push(CallInfo {
                            name: fn_name,
                            call_start_byte,
                        });
                        {
                            let old = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old);
                        }
                        continue;
                    } else {
                        // Python function - call with kwargs

                        // Check if function needs context passed
                        let pass_context = actual_func_kw
                            .getattr("pass_context")
                            .map(|attr| attr.is_truthy().unwrap_or(false))
                            .unwrap_or(false);

                        let mut args_py: Vec<Py<PyAny>> = Vec::with_capacity(args.len() + usize::from(pass_context));

                        if pass_context {
                            if let Some(ref ctx) = host.context() {
                                args_py.push(ctx.clone_ref(py));
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Function requires context but VM has no context available".to_string(),
                                ));
                            }
                        }

                        for arg in args.iter() {
                            args_py.push(arg.to_pyobject(py));
                        }

                        let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                        let result = actual_func_kw.call(args_tuple, Some(&kwargs_dict)).to_vm(py)?;
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);
                    }
                }

                OpCode::TailCall => {
                    // TCO: reuse current frame instead of creating a new one
                    let nargs = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let args_start = stack_len - nargs;
                    let args: Vec<Value> = frame.stack[args_start..].to_vec();
                    frame.stack.truncate(args_start);
                    let func = frame.pop();

                    // Native struct instantiation (fast path) - same as Call
                    {
                        let py_func_tmp = func.to_pyobject(py);
                        let ptr = py_func_tmp.bind(py).as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            check_abstract_guard(&self.struct_registry, type_id)?;
                            let (num_fields, min_args, type_name, defaults, init_func) = {
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                let nf = ty.fields.len();
                                let ma = ty.fields.iter().filter(|f| !f.has_default).count();
                                let tn = ty.name.clone();
                                let defs: Vec<Value> = ty.fields.iter().map(|f| f.default).collect();
                                let init = ty.methods.get("init").map(|f| f.clone_ref(py));
                                (nf, ma, tn, defs, init)
                            };
                            if nargs < min_args {
                                return Err(VMError::TypeError(format!(
                                    "{}() missing {} required argument(s)",
                                    type_name,
                                    min_args - nargs
                                )));
                            }
                            if nargs > num_fields {
                                return Err(VMError::TypeError(format!(
                                    "{}() takes {} argument(s) but {} were given",
                                    type_name, num_fields, nargs
                                )));
                            }
                            let mut field_values = args;
                            field_values.extend(defaults.iter().take(num_fields).skip(nargs).copied());
                            let idx = self.struct_registry.create_instance(type_id, field_values);
                            let inst_val = Value::from_struct_instance(idx);

                            if self.push_struct_init_frame(py, inst_val, init_func, frame)? {
                                continue;
                            }
                            frame.push(inst_val);
                            continue;
                        }
                    }

                    let py_func = func.to_pyobject(py);
                    let py_func_bound = py_func.bind(py);

                    // Unwrap BoundCatnipMethod: extract inner func, prepend instance to args
                    let (actual_func_ref, unwrapped_args);
                    let actual_func: &Bound<'_, PyAny>;
                    let mut bound_instance: Option<Value> = None;
                    let mut super_source_type: Option<String> = None;
                    if let Ok(bound_method) = py_func_bound.cast::<crate::core::BoundCatnipMethod>() {
                        let bm = bound_method.borrow();
                        actual_func_ref = bm.func.bind(py).clone();
                        let instance_val = if let Some(idx) = bm.native_instance_idx {
                            self.struct_registry.incref(idx);
                            Value::from_struct_instance(idx)
                        } else {
                            Value::from_pyobject(py, bm.instance.bind(py)).to_vm(py)?
                        };
                        bound_instance = Some(instance_val);
                        super_source_type = bm.super_source_type.clone();
                        let mut new_args = Vec::with_capacity(args.len() + 1);
                        new_args.push(instance_val);
                        new_args.extend_from_slice(&args);
                        unwrapped_args = new_args;
                        actual_func = &actual_func_ref;
                    } else {
                        actual_func_ref = py_func_bound.clone();
                        unwrapped_args = args;
                        actual_func = &actual_func_ref;
                    }
                    let args = unwrapped_args;
                    let nargs = args.len();

                    // VMFunction detection (fast Rust cast, then fallback)
                    let tco_data: Option<(Arc<CodeObject>, Option<NativeClosureScope>)> =
                        if let Ok(vm_func) = actual_func.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            Some((code, closure))
                        } else if let Ok(vm_code) = actual_func.getattr("vm_code") {
                            Some((convert_code_object(py, &vm_code).to_vm(py)?, None))
                        } else {
                            None
                        };
                    if let Some((new_code, tco_closure)) = tco_data {
                        // VMFunction - reuse frame for TCO

                        // 1. Resize locals if needed
                        let nlocals = new_code.nlocals;
                        frame.locals.resize(nlocals, Value::NIL);

                        // 2. Clear all slots
                        for i in 0..nlocals {
                            frame.locals[i] = Value::NIL;
                        }

                        // 3. Rebind args with varargs handling
                        let vararg_idx = new_code.vararg_idx;
                        if vararg_idx >= 0 {
                            let vararg_idx_usize = vararg_idx as usize;
                            // Args before vararg
                            frame.locals[..args.len().min(vararg_idx_usize)]
                                .copy_from_slice(&args[..args.len().min(vararg_idx_usize)]);
                            // Collect excess into vararg slot (store PyList directly, skip type detection)
                            if args.len() > vararg_idx_usize {
                                let excess: Vec<Py<PyAny>> = args[vararg_idx_usize..]
                                    .iter()
                                    .map(|v: &Value| v.to_pyobject(py))
                                    .collect();
                                let list = PyList::new(py, excess).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                frame.locals[vararg_idx_usize] = Value::from_owned_pyobject(list.unbind().into_any());
                            } else {
                                let empty = PyList::empty(py);
                                frame.locals[vararg_idx_usize] = Value::from_owned_pyobject(empty.unbind().into_any());
                            }
                        } else {
                            // No varargs - direct rebind
                            for (i, arg) in args.into_iter().enumerate() {
                                if i < nlocals {
                                    frame.locals[i] = arg;
                                }
                            }
                        }

                        // 4. Fill defaults for remaining params
                        let nparams = new_code.nargs;
                        let ndefaults = new_code.defaults.len();
                        if ndefaults > 0 {
                            let default_start = nparams.saturating_sub(ndefaults);
                            for i in nargs.max(default_start)..nparams {
                                let default_idx = i - default_start;
                                if default_idx < ndefaults {
                                    let val = new_code.defaults[default_idx];
                                    val.clone_refcount();
                                    frame.locals[i] = val;
                                }
                            }
                        }

                        // 5. Reset frame state
                        frame.ip = 0;
                        frame.stack.clear();
                        frame.closure_scope = tco_closure;

                        // Setup super proxy for bound method calls (inlined MRO-based)
                        if let Some(inst_val) = bound_instance {
                            let real_type_name = if let Some(idx) = inst_val.as_struct_instance_idx() {
                                self.struct_registry.get_instance(idx).and_then(|inst| {
                                    self.struct_registry.get_type(inst.type_id).map(|ty| ty.name.clone())
                                })
                            } else {
                                let inst_py = inst_val.to_pyobject(py);
                                inst_py
                                    .bind(py)
                                    .cast::<super::structs::CatnipStructProxy>()
                                    .ok()
                                    .map(|proxy| proxy.borrow().type_name.clone())
                            };

                            let mut did_set = false;
                            if let Some(ref real_name) = real_type_name {
                                if let Some(real_type) = self.struct_registry.find_type_by_name(real_name) {
                                    if !real_type.parent_names.is_empty() {
                                        let mro = &real_type.mro;
                                        let start_pos = if let Some(ref source) = super_source_type {
                                            mro.iter().position(|n| n == source).map(|p| p + 1).unwrap_or(1)
                                        } else {
                                            1
                                        };
                                        if start_pos < mro.len() {
                                            let mut methods: IndexMap<String, Py<PyAny>> = IndexMap::new();
                                            let mut method_sources: HashMap<String, String> = HashMap::new();
                                            for mro_type_name in &mro[start_pos..] {
                                                if let Some(ty) = self.struct_registry.find_type_by_name(mro_type_name)
                                                {
                                                    for (k, v) in &ty.methods {
                                                        if !methods.contains_key(k) {
                                                            methods.insert(k.clone(), v.clone_ref(py));
                                                            method_sources.insert(k.clone(), mro_type_name.clone());
                                                        }
                                                    }
                                                }
                                            }
                                            if !methods.is_empty() {
                                                let inst_py = inst_val.to_pyobject(py);
                                                let native_idx = inst_val.as_struct_instance_idx();
                                                let proxy = Py::new(
                                                    py,
                                                    super::structs::SuperProxy {
                                                        methods,
                                                        instance: inst_py,
                                                        method_sources,
                                                        native_instance_idx: native_idx,
                                                    },
                                                )
                                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                                frame.super_proxy = Some(proxy.into_any());
                                                did_set = true;
                                            }
                                        }
                                    }
                                }
                            }
                            if !did_set {
                                frame.super_proxy = None;
                            }
                        } else {
                            frame.super_proxy = None;
                        }

                        // Replace code object
                        frame.code = Some(new_code);
                        // Continue to restart dispatch with new code
                        continue;
                    } else {
                        // Python callable - call directly
                        let pass_context = actual_func
                            .getattr("pass_context")
                            .map(|attr| attr.is_truthy().unwrap_or(false))
                            .unwrap_or(false);

                        let mut args_py: Vec<Py<PyAny>> = Vec::with_capacity(args.len() + usize::from(pass_context));

                        if pass_context {
                            if let Some(ref ctx) = host.context() {
                                args_py.push(ctx.clone_ref(py));
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Function requires context but VM has no context available".to_string(),
                                ));
                            }
                        }

                        for arg in args.iter() {
                            args_py.push(arg.to_pyobject(py));
                        }

                        let args_tuple = PyTuple::new(py, args_py).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let result = actual_func
                            .call1(args_tuple)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);
                    }
                }

                // Fused GetAttr + Call: resolve method on obj, call directly.
                // Eliminates BoundCatnipMethod allocation for struct method calls.
                OpCode::CallMethod => {
                    let nargs = (instr.arg & 0xFFFF) as usize;
                    let name_idx = (instr.arg >> 16) as usize;
                    let method_name = &code.names[name_idx];

                    // Stack: [obj, arg1, arg2, ...argN]
                    let stack_len = frame.stack.len();
                    let args_start = stack_len - nargs;
                    let args: Vec<Value> = frame.stack[args_start..].to_vec();
                    frame.stack.truncate(args_start);
                    let obj = frame.pop();

                    if let Some(idx) = obj.as_struct_instance_idx() {
                        let inst = self
                            .struct_registry
                            .get_instance(idx)
                            .ok_or_else(|| VMError::RuntimeError(format!("invalid struct instance index {idx}")))?;
                        let type_id = inst.type_id;
                        let ty = self
                            .struct_registry
                            .get_type(type_id)
                            .ok_or_else(|| VMError::RuntimeError(format!("invalid struct type index {type_id}")))?;

                        // Check field first (callable field, no self binding)
                        if let Some(field_idx) = ty.field_index(method_name) {
                            let field_val = inst.fields[field_idx];
                            // Call as regular function (no self)
                            let py_func = field_val.to_pyobject(py);
                            let py_func_bound = py_func.bind(py);
                            if let Ok(vm_func) = py_func_bound.cast::<VMFunction>() {
                                let vm_ref = vm_func.borrow();
                                let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                let closure = vm_ref.native_closure.clone();
                                drop(vm_ref);
                                let mut new_frame = Frame::with_code(new_code);
                                new_frame.bind_args(py, &args, None);
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            } else {
                                // Python callable field
                                let args_py: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
                                let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                let result = py_func_bound.call1(args_tuple).to_vm(py)?;
                                let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                frame.push(value);
                                continue;
                            }
                        }

                        // Method lookup (with self binding)
                        if let Some(func) = ty.methods.get(method_name.as_str()) {
                            let func_bound = func.bind(py);
                            // Prepend self to args
                            let mut all_args = Vec::with_capacity(nargs + 1);
                            all_args.push(obj);
                            all_args.extend_from_slice(&args);

                            if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                                let vm_ref = vm_func.borrow();
                                let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                let closure = vm_ref.native_closure.clone();
                                drop(vm_ref);
                                let mut new_frame = Frame::with_code(new_code);
                                new_frame.bind_args(py, &all_args, None);
                                new_frame.closure_scope = closure;
                                // Setup super proxy
                                self.setup_super_proxy(py, obj, None, &mut new_frame)?;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            } else {
                                // Python method
                                let args_py: Vec<Py<PyAny>> = all_args.iter().map(|v| v.to_pyobject(py)).collect();
                                let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                let result = func_bound.call1(args_tuple).to_vm(py)?;
                                let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                frame.push(value);
                                continue;
                            }
                        }

                        // Static method (no self binding)
                        if let Some(func) = ty.static_methods.get(method_name.as_str()) {
                            let func_bound = func.bind(py);
                            if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                                let vm_ref = vm_func.borrow();
                                let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                let closure = vm_ref.native_closure.clone();
                                drop(vm_ref);
                                let mut new_frame = Frame::with_code(new_code);
                                new_frame.bind_args(py, &args, None);
                                new_frame.closure_scope = closure;
                                {
                                    let old = std::mem::replace(frame, new_frame);
                                    self.frame_stack.push(old);
                                }
                                continue;
                            } else {
                                let args_py: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
                                let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                let result = func_bound.call1(args_tuple).to_vm(py)?;
                                let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                frame.push(value);
                                continue;
                            }
                        }

                        return Err(VMError::RuntimeError(attr_error_msg(ty, method_name)));
                    } else {
                        // PyObject fallback: getattr + call
                        let py_obj = obj.to_pyobject(py);
                        let py_bound = py_obj.bind(py);

                        // Check struct marker type (static methods)
                        let ptr = py_bound.as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            let ty = self.struct_registry.get_type(type_id).unwrap();
                            if let Some(func) = ty.static_methods.get(method_name.as_str()) {
                                let func_bound = func.bind(py);
                                if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                                    let vm_ref = vm_func.borrow();
                                    let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                    let closure = vm_ref.native_closure.clone();
                                    drop(vm_ref);
                                    let mut new_frame = Frame::with_code(new_code);
                                    new_frame.bind_args(py, &args, None);
                                    new_frame.closure_scope = closure;
                                    {
                                        let old = std::mem::replace(frame, new_frame);
                                        self.frame_stack.push(old);
                                    }
                                    continue;
                                } else {
                                    // Python static method
                                    let args_py: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
                                    let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                    let result = func_bound.call1(args_tuple).to_vm(py)?;
                                    let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                    frame.push(value);
                                    continue;
                                }
                            }
                            return Err(VMError::RuntimeError(attr_error_msg(ty, method_name)));
                        }

                        // Check SuperProxy: resolve method and call with self
                        if let Ok(sp) = py_bound.cast::<super::structs::SuperProxy>() {
                            let sp_ref = sp.borrow();
                            if let Some(func) = sp_ref.methods.get(method_name.as_str()) {
                                let func_clone = func.clone_ref(py);
                                let inst_py = sp_ref.instance.clone_ref(py);
                                let native_idx = sp_ref.native_instance_idx;
                                let source_type = sp_ref
                                    .method_sources
                                    .get(method_name.as_str())
                                    .cloned()
                                    .unwrap_or_default();
                                drop(sp_ref);
                                let func_bound = func_clone.bind(py);
                                // Build args with self prepended
                                let inst_val = if let Some(idx) = native_idx {
                                    self.struct_registry.incref(idx);
                                    Value::from_struct_instance(idx)
                                } else {
                                    Value::from_pyobject(py, inst_py.bind(py)).to_vm(py)?
                                };
                                let mut all_args = Vec::with_capacity(nargs + 1);
                                all_args.push(inst_val);
                                all_args.extend_from_slice(&args);
                                if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
                                    let vm_ref = vm_func.borrow();
                                    let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                                    let closure = vm_ref.native_closure.clone();
                                    drop(vm_ref);
                                    let mut new_frame = Frame::with_code(new_code);
                                    new_frame.bind_args(py, &all_args, None);
                                    new_frame.closure_scope = closure;
                                    // Setup super chain for parent of parent
                                    self.setup_super_proxy(py, inst_val, Some(source_type), &mut new_frame)?;
                                    {
                                        let old = std::mem::replace(frame, new_frame);
                                        self.frame_stack.push(old);
                                    }
                                    continue;
                                } else {
                                    let args_py: Vec<Py<PyAny>> = all_args.iter().map(|v| v.to_pyobject(py)).collect();
                                    let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                                    let result = func_bound.call1(args_tuple).to_vm(py)?;
                                    let value = Value::from_pyobject(py, &result).to_vm(py)?;
                                    frame.push(value);
                                    continue;
                                }
                            }
                            return Err(VMError::RuntimeError(format!("super has no method '{}'", method_name)));
                        }

                        // General Python getattr + call
                        let method = py_bound.getattr(method_name.as_str()).map_err(|e| {
                            let msg = e.to_string();
                            VMError::RuntimeError(py_attr_error_msg(py_bound, method_name, &msg))
                        })?;
                        // Inline VMFunction calls (avoid VMFunction.__call__ which
                        // creates a fresh VM without the parent's enum/symbol tables).
                        if let Ok(vm_func) = method.cast::<VMFunction>() {
                            let vm_ref = vm_func.borrow();
                            let new_code = Arc::clone(&vm_ref.vm_code.borrow(py).inner);
                            let closure = vm_ref.native_closure.clone();
                            drop(vm_ref);
                            let mut new_frame = Frame::with_code(new_code);
                            new_frame.bind_args(py, &args, None);
                            new_frame.closure_scope = closure;
                            {
                                let old = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old);
                            }
                            continue;
                        }
                        let args_py: Vec<Py<PyAny>> = args.iter().map(|v| v.to_pyobject(py)).collect();
                        let args_tuple = PyTuple::new(py, args_py).to_vm(py)?;
                        let result = method.call1(args_tuple).to_vm(py)?;
                        let value = Value::from_pyobject(py, &result).to_vm(py)?;
                        frame.push(value);
                    }
                }

                OpCode::Return => {
                    // If handler_stack has Finally, handle inline (don't exit dispatch_inner)
                    if !frame.handler_stack.is_empty() {
                        let val = frame.pop();
                        let err = VMError::Return(val);
                        if self.try_unwind_to_handler(frame, &err) {
                            continue;
                        }
                        // No Finally handler, fall through to normal return
                        // Recover the value from the error
                        if let VMError::Return(v) = err {
                            frame.push(v);
                        }
                    }

                    // Decrement recursive depth BEFORE processing return
                    // This ensures we resume tracing at the correct point
                    if self.jit_recursive_depth > 0 {
                        self.jit_recursive_depth -= 1;
                    }

                    // Store the return value and discard flag before releasing frame borrow
                    last_result = frame.pop();
                    let discard = frame.discard_return;

                    // Pop call stack entry (if any)
                    if !self.call_stack.is_empty() {
                        self.call_stack.pop();
                    }

                    // Check if we should finalize function trace
                    // We finalize when returning to the depth where tracing started
                    // current frame is NOT on frame_stack, so depth = frame_stack.len() + 1
                    let current_depth = self.frame_stack.len() + 1;
                    let should_finalize_trace = self.jit_tracing
                        && self.jit_tracing_func_id.is_some()
                        && current_depth == self.jit_tracing_depth;

                    // Pop caller from frame_stack and replace current frame
                    if let Some(caller) = self.frame_stack.pop() {
                        let old = std::mem::replace(frame, caller);
                        // Don't call frame_pool.free: opcodes balance refcounts
                        drop(old);
                        self.handle_nd_frame_pop(py, last_result);

                        // Push result to caller (unless init whose return is discarded)
                        if !discard {
                            frame.push(last_result);
                        }

                        // Restore caller's bytecode hash for JIT
                        if self.jit_enabled {
                            if let Some(ref caller_code) = frame.code {
                                self.update_jit_bytecode_hash_value(caller_code.bytecode_hash());
                            }
                        }
                    } else {
                        // No caller: return from top-level, let outer dispatch handle it
                        return Err(VMError::Return(last_result));
                    }

                    // Finalize trace if needed (after frame is popped)
                    if should_finalize_trace {
                        if let Some(mut trace) = self.jit_recorder.stop() {
                            if trace.is_compilable() {
                                let func_id = self.jit_tracing_func_id.take().unwrap();

                                // Phase 4.1: Optimize tail calls before compilation
                                trace.optimize_tail_calls();

                                if self.trace {
                                    eprintln!(
                                        "[JIT] Function trace complete: {} ops, params: {}",
                                        trace.ops.len(),
                                        trace.num_params
                                    );
                                }

                                // Compile the function trace to native code
                                let mut jit = self.jit.lock().unwrap();
                                if let Some(ref mut executor) = *jit {
                                    match executor.compile_function_trace(&trace) {
                                        Ok((compiled_fn, max_slot, name_guards)) => {
                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Function compiled successfully: {} (max_slot: {}, guards: {})",
                                                    func_id,
                                                    max_slot,
                                                    name_guards.len()
                                                );
                                            }
                                            // Store compiled function with max slot info and guards
                                            executor.store_compiled_function(
                                                func_id.clone(),
                                                compiled_fn,
                                                max_slot,
                                                name_guards,
                                            );
                                            // Mark as compiled in detector
                                            self.jit_detector.mark_compiled_internal(&func_id);
                                        }
                                        Err(e) => {
                                            if self.trace {
                                                eprintln!("[JIT] Function compilation failed: {}", e);
                                            }
                                        }
                                    }
                                }
                            } else if self.trace {
                                eprintln!("[JIT] Function trace not compilable");
                            }
                        }

                        self.jit_tracing = false;
                        self.jit_tracing_func_id = None;
                    }

                    // Sync globals back to caller's local slots ONLY for module frame.
                    // Function frames use LoadScope (resolves from closure chain),
                    // so they don't need sync. Syncing to function frames would
                    // overwrite locals with stale ctx_globals values.
                    if self.frame_stack.is_empty() {
                        let updates: Vec<(usize, Value)> = if let Some(ref code) = frame.code {
                            code.slotmap
                                .iter()
                                .filter_map(|(name, &slot_idx)| self.globals.get(name.as_str()).map(|&v| (slot_idx, v)))
                                .collect()
                        } else {
                            Vec::new()
                        };
                        for (slot_idx, v) in updates {
                            frame.set_local(slot_idx, v);
                        }
                    }
                    continue;
                }

                OpCode::MakeFunction => {
                    // Pop code object and create VMFunction
                    let code_obj = frame.pop().to_pyobject(py);

                    // Build native captured HashMap (no Python boundary crossing)
                    let mut captured: IndexMap<String, Value> = IndexMap::new();
                    if let Some(ref code) = frame.code {
                        for (name, &slot_idx) in &code.slotmap {
                            // Skip module-level vars (accessed via parent chain)
                            if self.globals.contains_key(name.as_str()) {
                                continue;
                            }
                            let val = frame.get_local(slot_idx);
                            if !val.is_nil() && !val.is_invalid() {
                                captured.insert(name.clone(), val);
                            }
                        }
                    }
                    portabilize_struct_values(py, &mut captured, &mut self.struct_registry);

                    // Build parent: native chain or PyGlobals terminal
                    let parent = host.build_closure_parent(py, frame.closure_scope.as_ref());

                    let native_scope = NativeClosureScope::new(captured, parent);
                    let context_for_func = host.context().as_ref().map(|c| c.clone_ref(py));

                    let code_py: Py<PyCodeObject> = code_obj
                        .bind(py)
                        .cast::<PyCodeObject>()
                        .map_err(|e| VMError::TypeError(format!("Expected CodeObject: {e}")))?
                        .clone()
                        .unbind();

                    let code = Arc::clone(&code_py.borrow(py).inner);
                    let idx = self.func_table.insert(FuncSlot {
                        code,
                        closure: Some(native_scope),
                        code_py,
                        context: context_for_func,
                    });
                    frame.push(Value::from_vmfunc(idx));
                }

                // --- Collection literals ---
                OpCode::BuildList => {
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;
                    let items: Vec<Py<PyAny>> = frame.stack[start..].iter().map(|v| v.to_pyobject(py)).collect();
                    frame.stack.truncate(start);
                    let list = PyList::new(py, items).unwrap();
                    frame.push(Value::from_owned_pyobject(list.unbind().into_any()));
                }

                OpCode::BuildTuple => {
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;
                    let items: Vec<Py<PyAny>> = frame.stack[start..].iter().map(|v| v.to_pyobject(py)).collect();
                    frame.stack.truncate(start);
                    let tuple = PyTuple::new(py, items).unwrap();
                    frame.push(Value::from_owned_pyobject(tuple.unbind().into_any()));
                }

                OpCode::BuildSet => {
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;
                    let items: Vec<Py<PyAny>> = frame.stack[start..].iter().map(|v| v.to_pyobject(py)).collect();
                    frame.stack.truncate(start);
                    let set_type = match &self.cached_set_type {
                        Some(cached) => cached.bind(py).clone(),
                        None => {
                            let st = py.import("builtins").to_vm(py)?.getattr("set").to_vm(py)?;
                            self.cached_set_type = Some(st.unbind());
                            self.cached_set_type.as_ref().unwrap().bind(py).clone()
                        }
                    };
                    let py_list = PyList::new(py, items).to_vm(py)?;
                    let py_set = set_type.call1((py_list,)).to_vm(py)?;
                    frame.push(Value::from_owned_pyobject(py_set.unbind()));
                }

                OpCode::BuildDict => {
                    let n = instr.arg as usize;
                    let dict = PyDict::new(py);
                    for _ in 0..n {
                        let value = frame.pop().to_pyobject(py);
                        let key = frame.pop().to_pyobject(py);
                        dict.set_item(key, value).ok();
                    }
                    frame.push(Value::from_owned_pyobject(dict.unbind().into_any()));
                }

                OpCode::BuildSlice => {
                    // Build slice(start, stop[, step])
                    const SLICE_ARGS_MIN: usize = 2;
                    const SLICE_ARGS_MAX: usize = 3;
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;
                    let items: Vec<Py<PyAny>> = frame.stack[start..].iter().map(|v| v.to_pyobject(py)).collect();
                    frame.stack.truncate(start);

                    // Create slice object
                    let slice_type = py.get_type::<pyo3::types::PySlice>();
                    let slice = if n == SLICE_ARGS_MIN {
                        slice_type.call1((&items[0], &items[1])).to_vm(py)?
                    } else if n == SLICE_ARGS_MAX {
                        slice_type.call1((&items[0], &items[1], &items[2])).to_vm(py)?
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "BUILD_SLICE expects 2 or 3 args, got {}",
                            n
                        )));
                    };
                    frame.push(Value::from_owned_pyobject(slice.unbind()));
                }

                // --- Attribute/item access ---
                OpCode::GetAttr => {
                    let attr_name = get_name(code, instr.arg)?;
                    let obj = frame.pop();

                    if let Some(idx) = obj.as_struct_instance_idx() {
                        let inst = self.struct_registry.get_instance(idx).unwrap();
                        let type_id = inst.type_id;
                        let ty = self.struct_registry.get_type(type_id).unwrap();
                        match ty.field_index(attr_name) {
                            Some(field_idx) => {
                                let val = inst.fields[field_idx];
                                // inst/ty borrows end here (NLL: val and type_id are Copy)
                                val.clone_refcount_bigint();
                                if val.is_struct_instance() {
                                    self.struct_registry.incref(val.as_struct_instance_idx().unwrap());
                                }
                                frame.push(val);
                                decref_discard(&mut self.struct_registry, obj);
                            }
                            None => {
                                // Look up method in StructType
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                if let Some(func) = ty.methods.get(attr_name.as_str()) {
                                    let func_clone = func.clone_ref(py);
                                    // ty borrow ends (NLL: func_clone is owned)
                                    let proxy = obj.to_pyobject(py);
                                    let bound = Py::new(
                                        py,
                                        crate::core::BoundCatnipMethod {
                                            func: func_clone,
                                            instance: proxy,
                                            super_source_type: None,
                                            native_instance_idx: Some(idx),
                                        },
                                    )
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                    frame.push(Value::from_owned_pyobject(bound.into_any()));
                                    decref_discard(&mut self.struct_registry, obj);
                                } else if let Some(func) = ty.static_methods.get(attr_name.as_str()) {
                                    let func_clone = func.clone_ref(py);
                                    frame.push(Value::from_owned_pyobject(func_clone));
                                    decref_discard(&mut self.struct_registry, obj);
                                } else {
                                    let msg = attr_error_msg(ty, attr_name);
                                    decref_discard(&mut self.struct_registry, obj);
                                    return Err(VMError::RuntimeError(msg));
                                }
                            }
                        }
                    } else {
                        let py_obj = obj.to_pyobject(py);
                        let py_bound = py_obj.bind(py);
                        // Check if this is a struct marker type (for static methods)
                        let ptr = py_bound.as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            let ty = self.struct_registry.get_type(type_id).unwrap();
                            if let Some(func) = ty.static_methods.get(attr_name.as_str()) {
                                let value = Value::from_pyobject(py, func.bind(py)).to_vm(py)?;
                                frame.push(value);
                            } else {
                                return Err(VMError::RuntimeError(attr_error_msg(ty, attr_name)));
                            }
                        } else if let Some(&enum_type_id) = self.enum_type_map.get(&ptr) {
                            let ety = self.enum_registry.get_type(enum_type_id).unwrap();
                            if let Some(sym_id) = ety.variant_symbol(attr_name) {
                                frame.push(Value::from_symbol(sym_id));
                            } else {
                                return Err(VMError::RuntimeError(format!(
                                    "enum '{}' has no variant '{}'",
                                    ety.name, attr_name
                                )));
                            }
                        } else {
                            // Check if this is a CatnipEnumType from an imported module
                            // that isn't yet registered in our enum_type_map
                            if let Ok(etype) = py_bound.cast::<CatnipEnumType>() {
                                let et = etype.borrow();
                                // Lazily register the imported enum type
                                let type_id =
                                    self.enum_registry
                                        .register(&et.name, &et.variant_names, &mut self.symbol_table);
                                self.enum_type_map.insert(ptr, type_id);
                                // Now resolve the variant
                                let ety = self.enum_registry.get_type(type_id).unwrap();
                                if let Some(sym_id) = ety.variant_symbol(attr_name) {
                                    frame.push(Value::from_symbol(sym_id));
                                } else {
                                    return Err(VMError::RuntimeError(format!(
                                        "enum '{}' has no variant '{}'",
                                        ety.name, attr_name
                                    )));
                                }
                            } else {
                                let value = host.obj_getattr(py, obj, attr_name)?;
                                frame.push(value);
                            }
                        }
                    }
                }

                OpCode::SetAttr => {
                    let attr_name = get_name(code, instr.arg)?;
                    let value = frame.pop();
                    let obj = frame.pop();

                    if let Some(idx) = obj.as_struct_instance_idx() {
                        let type_id = self.struct_registry.get_instance(idx).unwrap().type_id;
                        let ty = self.struct_registry.get_type(type_id).unwrap();
                        match ty.field_index(attr_name) {
                            Some(field_idx) => {
                                // ty borrow ends (NLL: field_idx is Copy)
                                let old = {
                                    let inst = self.struct_registry.get_instance_mut(idx).unwrap();
                                    let old = inst.fields[field_idx];
                                    inst.fields[field_idx] = value;
                                    old
                                };
                                decref_discard(&mut self.struct_registry, old);
                                decref_discard(&mut self.struct_registry, obj);
                            }
                            None => {
                                let msg = attr_error_msg(ty, attr_name);
                                decref_discard(&mut self.struct_registry, obj);
                                return Err(VMError::RuntimeError(msg));
                            }
                        }
                    } else {
                        host.obj_setattr(py, obj, attr_name, value)?;
                    }
                }

                OpCode::GetItem => {
                    if instr.arg == 1 {
                        // Fused slice mode: stack has [obj, start, stop, step]
                        let step = frame.pop();
                        let stop = frame.pop();
                        let start = frame.pop();
                        let obj = frame.pop();
                        let slice_type = py.get_type::<pyo3::types::PySlice>();
                        let py_start = start.to_pyobject(py);
                        let py_stop = stop.to_pyobject(py);
                        let py_step = step.to_pyobject(py);
                        let slice = slice_type.call1((&py_start, &py_stop, &py_step)).to_vm(py)?;
                        let index = Value::from_owned_pyobject(slice.unbind());
                        let value = host.obj_getitem(py, obj, index)?;
                        frame.push(value);
                    } else {
                        let index = frame.pop();
                        let obj = frame.pop();
                        let value = host.obj_getitem(py, obj, index)?;
                        frame.push(value);
                    }
                }

                OpCode::SetItem => {
                    let value = frame.pop();
                    let index = frame.pop();
                    let obj = frame.pop();
                    host.obj_setitem(py, obj, index, value)?;
                }

                // --- Block/scope ---
                OpCode::PushBlock => {
                    let is_module_block = instr.arg & 0x8000_0000 != 0;
                    let slot_start = (instr.arg & 0x7FFF_FFFF) as usize;
                    frame.push_block(slot_start);
                    // Snapshot pre-existing global names for cleanup at PopBlock
                    if is_module_block {
                        if let Some(ref code) = frame.code {
                            let existing: Vec<String> = code.varnames[slot_start..]
                                .iter()
                                .filter(|n| self.globals.contains_key(n.as_str()) || host.has_global(py, n.as_str()))
                                .cloned()
                                .collect();
                            self.block_globals_snapshot.push(existing);
                        }
                    }
                }

                OpCode::PopBlock => {
                    // arg=1: module-level block, clean block-local names from globals
                    if instr.arg == 1 {
                        if let Some(&(slot_start, _)) = frame.block_stack.last() {
                            if let Some(ref code) = frame.code {
                                // Pop the globals snapshot saved at PushBlock
                                let pre_existing = self.block_globals_snapshot.pop();
                                for slot in slot_start..code.varnames.len() {
                                    let name = &code.varnames[slot];
                                    // Only clean names not in globals before the block
                                    let existed_before =
                                        pre_existing.as_ref().is_some_and(|names| names.contains(name));
                                    if !existed_before {
                                        self.globals.swap_remove(name);
                                        host.delete_global(py, name.as_str())?;
                                    }
                                }
                            }
                        }
                    }
                    frame.pop_block();
                }

                // --- Control signals ---
                OpCode::Break => {
                    let err = VMError::Break;
                    if self.try_unwind_to_handler(frame, &err) {
                        continue;
                    }
                    return Err(err);
                }

                OpCode::Continue => {
                    let err = VMError::Continue;
                    if self.try_unwind_to_handler(frame, &err) {
                        continue;
                    }
                    return Err(err);
                }

                // --- Broadcasting ---
                OpCode::Broadcast => {
                    // Decode flags: bit 0 = is_filter, bit 1 = has_operand, bits 2-3 = ND type
                    const FLAG_FILTER: u32 = 1;
                    const FLAG_OPERAND: u32 = 2;
                    const FLAG_ND_RECURSION: u32 = 4;
                    const FLAG_ND_MAP: u32 = 8;
                    let flags = instr.arg;
                    let is_filter = (flags & FLAG_FILTER) != 0;
                    let has_operand = (flags & FLAG_OPERAND) != 0;
                    let is_nd_recursion = (flags & FLAG_ND_RECURSION) != 0;
                    let is_nd_map = (flags & FLAG_ND_MAP) != 0;

                    // Handle ND operations specially (delegated to host for parallelism)
                    if is_nd_recursion || is_nd_map {
                        let lambda_val = frame.pop();
                        let target_val = frame.pop();
                        let lambda_py = lambda_val.to_pyobject(py);
                        let target_py = target_val.to_pyobject(py);
                        let target_bound = target_py.bind(py);
                        let lambda_bound = lambda_py.bind(py);

                        let result_py = if is_nd_recursion {
                            host.broadcast_nd_recursion(py, target_bound, lambda_bound)?
                        } else {
                            host.broadcast_nd_map(py, target_bound, lambda_bound)?
                        };

                        let value = Value::from_pyobject(py, result_py.bind(py)).to_vm(py)?;
                        frame.push(value);
                    } else {
                        // Regular broadcast: pop operand if present
                        let operand = if has_operand {
                            Some(frame.pop().to_pyobject(py))
                        } else {
                            None
                        };

                        // Pop operator and target
                        let operator = frame.pop().to_pyobject(py);
                        let target = frame.pop().to_pyobject(py);

                        // Delegate broadcast to host
                        let target_bound = target.bind(py);
                        let operator_bound = operator.bind(py);
                        let result = host.apply_broadcast(
                            py,
                            target_bound,
                            operator_bound,
                            operand.as_ref().map(|o| o.bind(py)),
                            is_filter,
                        )?;

                        let value = Value::from_pyobject(py, result.bind(py)).to_vm(py)?;
                        frame.push(value);
                    }
                }

                // --- Pattern matching ---
                OpCode::MatchPattern => {
                    return Err(VMError::RuntimeError(errors::ERR_LEGACY_MATCH.into()));
                }

                OpCode::MatchPatternVM => {
                    // Native path: pre-compiled VMPattern, no Python boundary crossing
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    let pattern = frame.code.as_ref().and_then(|c| c.patterns.get(pat_idx)).cloned();
                    match pattern {
                        Some(ref pat) => match vm_match_pattern(py, pat, value, &self.struct_registry).to_vm(py)? {
                            Some(bindings) => {
                                frame.match_bindings = Some(bindings);
                                frame.push(Value::TRUE);
                            }
                            None => {
                                frame.match_bindings = None;
                                frame.push(Value::NIL);
                            }
                        },
                        None => {
                            frame.match_bindings = None;
                            frame.push(Value::NIL);
                        }
                    }
                }

                OpCode::MatchAssignPatternVM => {
                    // Strict assignment-pattern matching:
                    // on mismatch, raise unpacking error (type/runtime) with details.
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    let pattern = frame.code.as_ref().and_then(|c| c.patterns.get(pat_idx)).cloned();
                    match pattern {
                        Some(ref pat) => {
                            let bindings = vm_match_assign_pattern(py, pat, value, &self.struct_registry)?;
                            frame.match_bindings = Some(bindings);
                            frame.push(Value::TRUE);
                        }
                        None => {
                            return Err(VMError::RuntimeError("Invalid assignment pattern index".to_string()));
                        }
                    }
                }

                OpCode::BindMatch => {
                    if let Some(bindings) = frame.match_bindings.clone() {
                        frame.pop(); // pop the sentinel TRUE
                        for (slot, val) in bindings {
                            frame.set_local(slot, val);
                        }
                    }
                }

                OpCode::JumpIfNone => {
                    let value = frame.pop();
                    if value.is_nil() {
                        frame.ip = instr.arg as usize;
                    }
                }

                OpCode::JumpIfNotNoneOrPop => {
                    let cond = frame.peek();
                    if !cond.is_nil() {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                    }
                }

                OpCode::ToBool => {
                    let value = frame.pop();
                    frame.push(Value::from_bool(value.is_truthy()));
                }

                // --- Process ---
                OpCode::Exit => {
                    // arg encodes: 0 = no argument (default 0), 1 = pop code from stack
                    let code = if instr.arg == 1 {
                        let v = frame.pop();
                        v.as_int().map(|n| n as i32).unwrap_or(1)
                    } else {
                        0
                    };
                    return Err(VMError::Exit(code));
                }

                // --- Unpacking ---
                OpCode::UnpackSequence => {
                    let n = instr.arg as usize;
                    let seq = frame.pop();
                    let py_seq = seq.to_pyobject(py);
                    let py_seq_bound = py_seq.bind(py);

                    // Convert to list to get items
                    let items: Vec<Py<PyAny>> = match py_seq_bound.try_iter() {
                        Ok(iter) => iter
                            .map(|item| item.map(|i| i.unbind()))
                            .collect::<PyResult<Vec<_>>>()
                            .to_vm(py)?,
                        Err(_) => {
                            let ty = py_seq_bound
                                .get_type()
                                .name()
                                .map(|n| n.to_string())
                                .unwrap_or_else(|_| "value".to_string());
                            return Err(VMError::TypeError(format!("Cannot unpack non-iterable {}", ty)));
                        }
                    };

                    if items.len() != n {
                        return Err(VMError::RuntimeError(format!(
                            "Cannot unpack {} values into {} variables",
                            items.len(),
                            n
                        )));
                    }

                    // Push items in reverse order (so first item ends on top)
                    for item in items.into_iter().rev() {
                        let val = Value::from_pyobject(py, item.bind(py)).to_vm(py)?;
                        frame.push(val);
                    }
                }

                OpCode::UnpackEx => {
                    // Extended unpacking: *rest syntax
                    // arg encodes (before << 8) | after
                    let before = ((instr.arg >> 8) & 0xFF) as usize;
                    let after = (instr.arg & 0xFF) as usize;

                    let seq = frame.pop();
                    let py_seq = seq.to_pyobject(py);
                    let py_seq_bound = py_seq.bind(py);

                    // Convert to list
                    let items: Vec<Py<PyAny>> = match py_seq_bound.try_iter() {
                        Ok(iter) => iter
                            .map(|item| item.map(|i| i.unbind()))
                            .collect::<PyResult<Vec<_>>>()
                            .to_vm(py)?,
                        Err(_) => {
                            let ty = py_seq_bound
                                .get_type()
                                .name()
                                .map(|n| n.to_string())
                                .unwrap_or_else(|_| "value".to_string());
                            return Err(VMError::TypeError(format!("Cannot unpack non-iterable {}", ty)));
                        }
                    };

                    let total_fixed = before + after;
                    if items.len() < total_fixed {
                        return Err(VMError::RuntimeError(format!(
                            "Not enough values to unpack (expected at least {}, got {})",
                            total_fixed,
                            items.len()
                        )));
                    }

                    // Split: before items, middle (rest), after items
                    let rest_len = items.len() - total_fixed;
                    let before_items = &items[..before];
                    let rest_items = &items[before..before + rest_len];
                    let after_items = &items[before + rest_len..];

                    // Push in reverse order: after, rest (as list), before
                    for item in after_items.iter().rev() {
                        let val = Value::from_pyobject(py, item.bind(py)).to_vm(py)?;
                        frame.push(val);
                    }

                    // Create list for rest
                    let rest_py: Vec<Py<PyAny>> = rest_items.iter().map(|item| item.clone_ref(py)).collect();
                    let rest_list = PyList::new(py, rest_py).to_vm(py)?;
                    frame.push(Value::from_owned_pyobject(rest_list.unbind().into_any()));

                    for item in before_items.iter().rev() {
                        let val = Value::from_pyobject(py, item.bind(py)).to_vm(py)?;
                        frame.push(val);
                    }
                }

                // --- Optimized iteration ---
                OpCode::ForRangeInt => {
                    // Optimized range loop condition check
                    // Replaces: LoadLocal + LoadLocal + GE/LE + JumpIfTrue (4 opcodes -> 1)
                    use super::{
                        FOR_RANGE_JUMP_MASK, FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_SLOT_MASK, FOR_RANGE_SLOT_STOP_SHIFT,
                        FOR_RANGE_STEP_SIGN_SHIFT,
                    };

                    let slot_i = (instr.arg >> FOR_RANGE_SLOT_I_SHIFT) as usize;
                    let slot_stop = ((instr.arg >> FOR_RANGE_SLOT_STOP_SHIFT) & FOR_RANGE_SLOT_MASK) as usize;
                    let step_positive = ((instr.arg >> FOR_RANGE_STEP_SIGN_SHIFT) & 1) == 0;
                    let jump_offset = (instr.arg & FOR_RANGE_JUMP_MASK) as usize;

                    let i = frame.get_local(slot_i);
                    let stop = frame.get_local(slot_stop);

                    // Fast path: both are ints
                    let done = match (i.as_int(), stop.as_int()) {
                        (Some(i_val), Some(stop_val)) => {
                            if step_positive {
                                i_val >= stop_val
                            } else {
                                i_val <= stop_val
                            }
                        }
                        _ => true, // Fallback: treat as done
                    };

                    if done {
                        // If we were tracing this loop, finish tracing
                        if self.jit_tracing && self.jit_tracing_offset == frame.ip - 1 {
                            let ip = frame.ip - 1;
                            self.jit_recorder
                                .record_opcode(OpCode::ForRangeInt, instr.arg, true, ip);
                            let trace = self.jit_recorder.stop();
                            self.jit_tracing = false;

                            if let Some(t) = trace {
                                if t.is_compilable() {
                                    if self.trace {
                                        eprintln!(
                                            "[JIT] Trace recorded: {} ops, {} iterations",
                                            t.ops.len(),
                                            t.iterations
                                        );
                                    }
                                    // Compile via executor
                                    let mut jit = self.jit.lock().unwrap();
                                    if let Some(ref mut executor) = *jit {
                                        match executor.compile_trace(t) {
                                            Ok(true) => {
                                                if self.trace {
                                                    eprintln!("[JIT] Trace compiled successfully");
                                                }
                                            }
                                            Ok(false) => {
                                                if self.trace {
                                                    eprintln!("[JIT] Trace not compilable");
                                                }
                                            }
                                            Err(e) => {
                                                if self.trace {
                                                    eprintln!("[JIT] Compilation failed: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        frame.ip += jump_offset;
                    } else if self.jit_enabled {
                        let loop_offset = frame.ip - 1;

                        // Check if we have compiled code for this loop
                        if !self.jit_tracing {
                            // Skip JIT if guard just failed for this loop
                            if self.jit_guard_failed == Some(loop_offset) {
                                self.jit_guard_failed = None;
                                // Fall through to interpreter
                            } else {
                                let has_compiled = {
                                    let jit = self.jit.lock().unwrap();
                                    jit.as_ref().map(|e| e.has_compiled(loop_offset)).unwrap_or(false)
                                };

                                if has_compiled {
                                    // Validate guards before executing JIT code
                                    let guards = {
                                        let jit = self.jit.lock().unwrap();
                                        jit.as_ref().and_then(|e| e.get_guards(loop_offset)).cloned()
                                    };

                                    let mut guards_pass = true;
                                    let mut guard_locals: Vec<(usize, i64)> = Vec::new();

                                    if let Some(ref guards) = guards {
                                        for (name, expected_value, slot) in guards {
                                            // Resolve current value of name
                                            let current_value = resolve_jit_guard_value(
                                                py,
                                                name,
                                                &frame.closure_scope,
                                                host,
                                                &self.globals,
                                            );

                                            match current_value {
                                                Some(val) if val == *expected_value => {
                                                    guard_locals.push((*slot, val));
                                                }
                                                _ => {
                                                    guards_pass = false;
                                                    break;
                                                }
                                            }
                                        }
                                    }

                                    // Skip JIT if any local holds a heap type (BigInt, PyObj).
                                    // The JIT operates on raw i64 and can't handle them.
                                    if guards_pass {
                                        for v in frame.locals.iter() {
                                            if v.is_bigint() || v.is_pyobj() || v.is_struct_instance() {
                                                guards_pass = false;
                                                break;
                                            }
                                        }
                                    }

                                    if guards_pass {
                                        // Execute compiled code
                                        // Pass NaN-boxed bits to JIT (codegen handles unboxing)
                                        let mut locals_raw: Vec<i64> =
                                            frame.locals.iter().map(|v| v.bits() as i64).collect();

                                        // Extend locals array for LoadScope slots
                                        let max_slot = guard_locals.iter().map(|(s, _)| s).max().copied();
                                        if let Some(max_slot) = max_slot {
                                            if max_slot >= locals_raw.len() {
                                                locals_raw.resize(max_slot + 1, 0);
                                            }
                                        }

                                        // Copy guard values into locals array
                                        for (slot, value) in guard_locals {
                                            locals_raw[slot] = value;
                                        }

                                        // Snapshot pre-JIT values to detect which slots changed
                                        let snapshot: Vec<i64> = locals_raw.clone();

                                        // Call JIT code
                                        let result = {
                                            let jit = self.jit.lock().unwrap();
                                            if let Some(ref executor) = *jit {
                                                unsafe { executor.execute(loop_offset, &mut locals_raw) }
                                            } else {
                                                None
                                            }
                                        };

                                        if let Some(ret) = result {
                                            // ret = 0: loop completed normally
                                            // ret = -1: guard failure (side exit)
                                            let guard_failed = ret == -1;

                                            if self.trace {
                                                eprintln!(
                                                    "[JIT] Executed compiled trace for loop at {} (guard_failed={})",
                                                    loop_offset, guard_failed
                                                );
                                            }
                                            // Restore only slots actually modified by JIT.
                                            // Values are already NaN-boxed by the codegen.
                                            for (i, &val) in locals_raw.iter().enumerate() {
                                                if i < frame.locals.len() && val != snapshot[i] {
                                                    let new_val = Value::from_raw(val as u64);
                                                    let old = frame.locals[i];
                                                    decref_discard(&mut self.struct_registry, old);
                                                    frame.locals[i] = new_val;
                                                    if i < code.varnames.len() {
                                                        host.store_global(py, &code.varnames[i], new_val)?;
                                                    }
                                                }
                                            }
                                            if guard_failed {
                                                // Guard failed: reset IP to ForRangeInt to re-check condition
                                                // Also set flag to skip JIT on next iteration
                                                frame.ip = loop_offset;
                                                self.jit_guard_failed = Some(loop_offset);
                                            } else {
                                                // Loop completed normally, skip to end of loop
                                                frame.ip += jump_offset;
                                            }
                                            continue;
                                        }
                                    }
                                    // If guards didn't pass, fall through to interpreter
                                }
                            } // end of else block for jit_guard_failed check
                        }

                        // If we're tracing and back at loop header, record loop back
                        if self.jit_tracing && self.jit_tracing_offset == loop_offset {
                            let ip = frame.ip - 1;
                            self.jit_recorder.record_loop_back(ip);
                            // Stop after 1 iteration (trace represents single loop body)
                            const TRACE_SINGLE_ITERATIONS: u32 = 1;
                            if self.jit_recorder.iterations() >= TRACE_SINGLE_ITERATIONS {
                                // Finish tracing and compile
                                let trace = self.jit_recorder.stop();
                                self.jit_tracing = false;

                                if let Some(t) = trace {
                                    if self.trace {
                                        eprintln!(
                                            "[JIT] Trace recorded: {} ops, {} iterations, int_only={}",
                                            t.ops.len(),
                                            t.iterations,
                                            t.is_int_only
                                        );
                                        for (i, op) in t.ops.iter().enumerate() {
                                            eprintln!("[JIT]   op[{}]: {:?}", i, op);
                                        }
                                    }
                                    if t.is_compilable() {
                                        // Compile via executor
                                        let mut jit = self.jit.lock().unwrap();
                                        if let Some(ref mut executor) = *jit {
                                            match executor.compile_trace(t) {
                                                Ok(true) => {
                                                    if self.trace {
                                                        eprintln!("[JIT] Trace compiled!");
                                                    }
                                                }
                                                Ok(false) => {
                                                    if self.trace {
                                                        eprintln!("[JIT] Trace not compilable");
                                                    }
                                                }
                                                Err(e) => {
                                                    if self.trace {
                                                        eprintln!("[JIT] Compilation failed: {}", e);
                                                    }
                                                }
                                            }
                                        }
                                    } else if self.trace {
                                        eprintln!("[JIT] Trace has fallbacks, not compilable");
                                    }
                                }
                            }
                        } else if self.jit_tracing && self.jit_tracing_offset != loop_offset {
                            // Tracing a different loop - nested loop encountered
                            // Record the nested ForRangeInt as part of outer trace
                            let ip = frame.ip - 1;
                            self.jit_recorder
                                .record_opcode(OpCode::ForRangeInt, instr.arg, true, ip);
                        } else if !self.jit_tracing {
                            // Check if we have a pending trace for this loop
                            if self.jit_pending_trace == Some(loop_offset) {
                                // Start tracing now (beginning of a full iteration)
                                self.jit_pending_trace = None;

                                let num_locals = frame.locals.len();
                                self.jit_recorder.start(loop_offset, num_locals);

                                // Extract loop bounds from ForRangeInt arg for Jump classification
                                let jump_offset = (instr.arg & 0x7FFF) as usize;
                                let loop_start = frame.ip - 1;
                                let loop_end = loop_start + jump_offset;
                                self.jit_recorder.set_loop_bounds(loop_start, loop_end);

                                self.jit_tracing = true;
                                self.jit_tracing_offset = loop_offset;

                                // Record ForRangeInt as FIRST op
                                let ip = frame.ip - 1;
                                self.jit_recorder
                                    .record_opcode(OpCode::ForRangeInt, instr.arg, true, ip);

                                if self.trace {
                                    eprintln!(
                                        "[JIT] Starting trace at {} (bounds: {} - {})",
                                        loop_offset, loop_start, loop_end
                                    );
                                }
                            } else {
                                // Warm-start: check trace cache on first encounter
                                if !self.jit_cache_checked.contains(&loop_offset) {
                                    self.jit_cache_checked.insert(loop_offset);
                                    let hit = {
                                        let mut jit = self.jit.lock().unwrap();
                                        jit.as_mut()
                                            .map(|e| e.try_compile_from_cache(loop_offset))
                                            .unwrap_or(false)
                                    };
                                    if hit && self.trace {
                                        eprintln!(
                                            "[JIT] Warm-start: for-range loop at {} loaded from cache",
                                            loop_offset
                                        );
                                    }
                                }

                                if self.jit_detector.record_loop_header(loop_offset) {
                                    // Loop just became hot - try cache first
                                    let compiled_from_cache = {
                                        let mut jit = self.jit.lock().unwrap();
                                        jit.as_mut()
                                            .map(|e| e.try_compile_from_cache(loop_offset))
                                            .unwrap_or(false)
                                    };
                                    if compiled_from_cache {
                                        if self.trace {
                                            eprintln!("[JIT] ForRange loop at {} compiled from cache", loop_offset);
                                        }
                                        // Don't schedule tracing, compiled code will be picked up next iteration
                                    } else {
                                        // Cache miss - schedule tracing for next iteration
                                        self.jit_pending_trace = Some(loop_offset);
                                    }

                                    if self.trace && !compiled_from_cache {
                                        eprintln!(
                                            "[JIT] Hot loop detected at {}, will trace next iteration",
                                            loop_offset
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                OpCode::ForRangeStep => {
                    // Fused increment + backward jump for range loops
                    // arg = (slot_i << 24) | (step_i8 << 16) | jump_target
                    let slot_i = (instr.arg >> 24) as usize;
                    let step = ((instr.arg >> 16) & 0xFF) as i8 as i64;
                    let jump_target = (instr.arg & 0xFFFF) as usize;

                    let i_val = frame.get_local(slot_i).as_int().unwrap_or(0);
                    frame.set_local(slot_i, Value::from_int(i_val + step));

                    // JIT: this replaces the backward Jump, so handle loop-back tracing
                    if self.jit_tracing && self.jit_tracing_offset == jump_target {
                        let ip = frame.ip - 1;
                        self.jit_recorder
                            .record_opcode(OpCode::ForRangeStep, instr.arg, true, ip);
                        self.jit_recorder.record_loop_back(ip);
                        const TRACE_SINGLE_ITERATIONS: u32 = 1;
                        if self.jit_recorder.iterations() >= TRACE_SINGLE_ITERATIONS {
                            let trace = self.jit_recorder.stop();
                            self.jit_tracing = false;

                            if let Some(t) = trace {
                                if t.is_compilable() {
                                    if self.trace {
                                        eprintln!(
                                            "[JIT] Trace recorded: {} ops, {} iterations",
                                            t.ops.len(),
                                            t.iterations
                                        );
                                    }
                                    let mut jit = self.jit.lock().unwrap();
                                    if let Some(ref mut executor) = *jit {
                                        match executor.compile_trace(t) {
                                            Ok(true) => {
                                                if self.trace {
                                                    eprintln!("[JIT] Trace compiled successfully");
                                                }
                                            }
                                            Ok(false) => {
                                                if self.trace {
                                                    eprintln!("[JIT] Trace not compilable");
                                                }
                                            }
                                            Err(e) => {
                                                if self.trace {
                                                    eprintln!("[JIT] Compilation failed: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if self.jit_tracing {
                        // Tracing a different loop - record as part of outer trace
                        let ip = frame.ip - 1;
                        self.jit_recorder
                            .record_opcode(OpCode::ForRangeStep, instr.arg, true, ip);
                    }

                    frame.ip = jump_target;
                }

                // --- Special ---
                OpCode::Nop => {}
                OpCode::Breakpoint => {} // handled by debug hook above

                OpCode::TypeOf => {
                    let val = frame.pop();
                    let type_str: &str = if val.is_bool() {
                        "bool"
                    } else if val.is_int() {
                        "int"
                    } else if val.is_float() {
                        "float"
                    } else if val.is_nil() {
                        "nil"
                    } else if val.is_symbol() {
                        // Enum variant: resolve to the enum type name
                        let sym_idx = val.as_symbol().unwrap();
                        if let Some((type_id, _)) = self.enum_registry.lookup_symbol(sym_idx) {
                            &self.enum_registry.get_type(type_id).unwrap().name
                        } else {
                            "symbol"
                        }
                    } else if val.is_bigint() {
                        "int"
                    } else if val.is_vmfunc() {
                        "function"
                    } else if val.is_struct_instance() {
                        let idx = val.as_struct_instance_idx().unwrap();
                        let name = self
                            .struct_registry
                            .get_instance(idx)
                            .and_then(|inst| self.struct_registry.get_type(inst.type_id).map(|ty| ty.name.clone()))
                            .unwrap_or_else(|| "object".to_string());
                        let py_str = PyString::intern(py, &name);
                        frame.push(Value::from_pyobject(py, py_str.as_any()).to_vm(py)?);
                        continue;
                    } else if val.is_pyobj() {
                        let obj = val.to_pyobject(py);
                        let obj_bound = obj.bind(py);
                        if obj_bound.is_instance_of::<pyo3::types::PyBool>() {
                            "bool"
                        } else if obj_bound.is_instance_of::<pyo3::types::PyInt>() {
                            "int"
                        } else if obj_bound.is_instance_of::<pyo3::types::PyFloat>() {
                            "float"
                        } else if obj_bound.is_instance_of::<PyString>() {
                            "string"
                        } else if obj_bound.is_instance_of::<PyList>() {
                            "list"
                        } else if obj_bound.is_instance_of::<PyTuple>() {
                            "tuple"
                        } else if obj_bound.is_instance_of::<PyDict>() {
                            "dict"
                        } else if obj_bound.is_instance_of::<pyo3::types::PySet>()
                            || obj_bound.is_instance_of::<pyo3::types::PyFrozenSet>()
                        {
                            "set"
                        } else if obj_bound.is_none() {
                            "nil"
                        } else if let Ok(proxy) = obj_bound.cast::<super::structs::CatnipStructProxy>() {
                            let name = proxy.borrow().type_name.clone();
                            let py_str = PyString::intern(py, &name);
                            frame.push(Value::from_pyobject(py, py_str.as_any()).to_vm(py)?);
                            continue;
                        } else if obj_bound.is_callable() {
                            "function"
                        } else {
                            let class_name: String = obj_bound
                                .get_type()
                                .qualname()
                                .and_then(|n| n.extract())
                                .unwrap_or_else(|_| "object".to_string());
                            // Catnip convention: lowercase type names
                            let catnip_name = class_name.to_ascii_lowercase();
                            let py_str = PyString::new(py, &catnip_name);
                            frame.push(Value::from_pyobject(py, py_str.as_any()).to_vm(py)?);
                            continue;
                        }
                    } else {
                        "object"
                    };
                    let py_str = PyString::intern(py, type_str);
                    frame.push(Value::from_pyobject(py, py_str.as_any()).to_vm(py)?);
                }

                OpCode::Globals => {
                    let dict = PyDict::new(py);
                    for (k, v) in self.globals.iter() {
                        dict.set_item(k, v.to_pyobject(py))
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    }
                    host.collect_globals(py, &dict)?;
                    let result =
                        Value::from_pyobject(py, dict.as_any()).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    frame.push(result);
                }

                OpCode::Locals => {
                    let dict = PyDict::new(py);
                    // Inside function: frame.locals + code.varnames + closure captures.
                    // At module level (code.name == "<module>"), fall through to globals.
                    let is_module = code.name == "<module>" || code.name.is_empty();
                    if is_module {
                        // Module level: locals() == globals()
                        for (k, v) in self.globals.iter() {
                            dict.set_item(k, v.to_pyobject(py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        }
                        host.collect_globals(py, &dict)?;
                    } else {
                        for (i, name) in code.varnames.iter().enumerate() {
                            if i < frame.locals.len() {
                                let val = frame.locals[i];
                                if !val.is_nil() && val.bits() != Value::INVALID.bits() {
                                    dict.set_item(name, val.to_pyobject(py))
                                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                }
                            }
                        }
                        if let Some(ref closure) = frame.closure_scope {
                            closure
                                .dump_into_dict(py, &dict)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        }
                    }
                    let result =
                        Value::from_pyobject(py, dict.as_any()).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    frame.push(result);
                }

                OpCode::MakeStruct => {
                    let const_idx = instr.arg as usize;
                    let struct_info_val = code.constants[const_idx];
                    let struct_info_py = struct_info_val.to_pyobject(py);
                    let info_tuple = struct_info_py
                        .bind(py)
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(format!("MakeStruct: bad constant: {e}")))?;

                    let name: String = tuple_extract(info_tuple, 0)?;
                    let fields_info = tuple_get(info_tuple, 1)?;
                    let num_defaults: usize = tuple_extract(info_tuple, 2)?;
                    // Detect format: new format has implements tuple at index 3
                    // New: (name, fields, num_defaults, implements, bases_tuple_or_None, [methods])
                    // Legacy: (name, fields, num_defaults, [methods_list])
                    let mut implements_list: Vec<String> = Vec::new();
                    let mut base_names: Vec<String> = Vec::new();
                    let mut methods_idx: Option<usize> = None;

                    if info_tuple.len() > 3 {
                        let item3 = tuple_get(info_tuple, 3)?;
                        // New format: item3 is a tuple (implements list)
                        if item3.is_instance_of::<PyTuple>() {
                            for imp in item3.try_iter().map_err(|e| VMError::RuntimeError(e.to_string()))? {
                                let imp = imp.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                implements_list
                                    .push(imp.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?);
                            }
                            // item4 = bases tuple or None
                            if info_tuple.len() > 4 {
                                let item4 = tuple_get(info_tuple, 4)?;
                                if !item4.is_none() {
                                    if item4.is_instance_of::<PyTuple>() {
                                        for b in item4.try_iter().map_err(|e| VMError::RuntimeError(e.to_string()))? {
                                            let b = b.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                            base_names.push(
                                                b.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?,
                                            );
                                        }
                                    } else if let Ok(base) = item4.extract::<String>() {
                                        // Legacy single base string
                                        base_names.push(base);
                                    }
                                }
                            }
                            // item5 = methods
                            if info_tuple.len() > 5 {
                                methods_idx = Some(5);
                            }
                        } else if let Ok(base) = item3.extract::<String>() {
                            // Legacy: item3 is base name string
                            base_names.push(base);
                            if info_tuple.len() > 4 {
                                methods_idx = Some(4);
                            }
                        } else {
                            // Legacy: item3 is methods list
                            methods_idx = Some(3);
                        }
                    }

                    // Read default values in stack order, then truncate
                    let stack_len = frame.stack.len();
                    let dstart = stack_len - num_defaults;
                    let default_values: Vec<Value> = frame.stack[dstart..].to_vec();
                    frame.stack.truncate(dstart);

                    // Parse fields
                    let fields_tuple = cast_tuple(&fields_info)?;
                    let mut native_fields = Vec::new();
                    let mut default_idx = 0usize;
                    for fi in fields_tuple.iter() {
                        let pair = cast_tuple(&fi)?;
                        let fname: String = tuple_extract(pair, 0)?;
                        let has_default: bool = tuple_extract(pair, 1)?;
                        let default_val = if has_default {
                            let v = default_values[default_idx];
                            default_idx += 1;
                            v
                        } else {
                            Value::NIL
                        };
                        native_fields.push(StructField {
                            name: fname,
                            has_default,
                            default: default_val,
                        });
                    }

                    // Build methods map if present
                    let mut methods_map: IndexMap<String, Py<PyAny>> = IndexMap::new();
                    let mut static_methods_map: IndexMap<String, Py<PyAny>> = IndexMap::new();
                    let mut own_abstract: HashSet<MethodKey> = HashSet::new();
                    if let Some(midx) = methods_idx {
                        let methods = tuple_get(info_tuple, midx)?;
                        for method_result in methods.try_iter().map_err(|e| VMError::RuntimeError(e.to_string()))? {
                            let method_pair = method_result.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let pair = cast_tuple(&method_pair)?;
                            let method_name: String = tuple_extract(pair, 0)?;

                            // Read is_static flag (3rd element, defaults to false)
                            let is_static: bool = if pair.len() > 2 {
                                tuple_extract(pair, 2).unwrap_or(false)
                            } else {
                                false
                            };

                            // Get CodeObject and create VMFunction
                            let code_obj = tuple_get(pair, 1)?;

                            // Abstract method: code_obj is None
                            if code_obj.is_none() {
                                own_abstract.insert(MethodKey {
                                    name: method_name,
                                    kind: if is_static {
                                        super::structs::MethodKind::Static
                                    } else {
                                        super::structs::MethodKind::Instance
                                    },
                                });
                                continue;
                            }

                            let captured = {
                                let mut cap: IndexMap<String, Value> = IndexMap::new();
                                if let Some(ref code) = frame.code {
                                    for (lname, &slot_idx) in &code.slotmap {
                                        if host.has_global(py, lname) {
                                            continue;
                                        }
                                        let val = frame.get_local(slot_idx);
                                        if !val.is_nil() && !val.is_invalid() {
                                            cap.insert(lname.clone(), val);
                                        }
                                    }
                                }
                                portabilize_struct_values(py, &mut cap, &mut self.struct_registry);
                                cap
                            };

                            let parent = host.build_closure_parent(py, frame.closure_scope.as_ref());
                            let native_scope = NativeClosureScope::new(captured, parent);
                            let context_for_func = host.context().as_ref().map(|c| c.clone_ref(py));
                            let code_py: Py<PyCodeObject> = code_obj
                                .cast::<PyCodeObject>()
                                .map_err(|e| VMError::TypeError(format!("Expected CodeObject: {e}")))?
                                .clone()
                                .unbind();
                            let func = Py::new(
                                py,
                                VMFunction::create_native(py, code_py, Some(native_scope), context_for_func),
                            )
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                            if is_static {
                                static_methods_map.insert(method_name, func.into_any());
                            } else {
                                methods_map.insert(method_name, func.into_any());
                            }
                        }
                    }

                    // Phase 1: extends(B, C, ...) merges parent fields+methods via C3 MRO.
                    let (mut merged_fields, mut merged_methods, mut merged_static, struct_mro) =
                        if !base_names.is_empty() {
                            // Compute C3 MRO (fallback to built-in exception hierarchy)
                            let struct_mro = super::mro::c3_linearize(&name, &base_names, |n| {
                                self.struct_registry
                                    .find_type_by_name(n)
                                    .map(|ty| ty.mro.clone())
                                    .or_else(|| catnip_core::exception::ExceptionKind::from_name(n).map(|k| k.mro()))
                            })
                            .map_err(VMError::RuntimeError)?;

                            // Merge fields following MRO (first-seen wins, skip self)
                            let mut seen_fields: HashSet<String> = HashSet::new();
                            let mut mro_fields: Vec<StructField> = Vec::new();
                            for mro_type_name in struct_mro.iter().skip(1) {
                                if let Some(ty) = self.struct_registry.find_type_by_name(mro_type_name) {
                                    for f in &ty.fields {
                                        if seen_fields.insert(f.name.clone()) {
                                            mro_fields.push(f.clone());
                                        }
                                    }
                                }
                            }

                            // Check child doesn't redefine inherited fields
                            for child_field in &native_fields {
                                if seen_fields.contains(&child_field.name) {
                                    return Err(VMError::RuntimeError(format!(
                                        "Struct '{}' redefines inherited field '{}'",
                                        name, child_field.name
                                    )));
                                }
                            }
                            mro_fields.extend(native_fields);

                            // Merge methods following MRO (first-seen wins, skip self)
                            let mut inherited_methods: IndexMap<String, Py<PyAny>> = IndexMap::new();
                            let mut inherited_static: IndexMap<String, Py<PyAny>> = IndexMap::new();
                            for mro_type_name in struct_mro.iter().skip(1) {
                                if let Some(ty) = self.struct_registry.find_type_by_name(mro_type_name) {
                                    for (k, v) in &ty.methods {
                                        if !inherited_methods.contains_key(k) {
                                            inherited_methods.insert(k.clone(), v.clone_ref(py));
                                        }
                                    }
                                    for (k, v) in &ty.static_methods {
                                        if !inherited_static.contains_key(k) {
                                            inherited_static.insert(k.clone(), v.clone_ref(py));
                                        }
                                    }
                                }
                            }

                            // Child overrides win
                            for (mname, mfunc) in methods_map {
                                inherited_methods.insert(mname, mfunc);
                            }
                            for (mname, mfunc) in static_methods_map {
                                inherited_static.insert(mname, mfunc);
                            }

                            (mro_fields, inherited_methods, inherited_static, struct_mro)
                        } else {
                            let mro = vec![name.clone()];
                            (native_fields, methods_map, static_methods_map, mro)
                        };

                    // Phase 2: implements(T1, T2, ...) resolves trait composition.
                    let mut trait_mro = Vec::new();
                    let mut trait_abstract: HashSet<MethodKey> = HashSet::new();
                    if !implements_list.is_empty() {
                        let struct_method_names: HashSet<String> = merged_methods.keys().cloned().collect();
                        let resolved = self
                            .trait_registry
                            .resolve_for_struct(py, &implements_list, &struct_method_names)
                            .map_err(VMError::RuntimeError)?;

                        trait_mro = resolved.linearization;
                        trait_abstract = resolved.abstract_methods;

                        // Prepend trait fields (before struct fields)
                        let struct_field_names: HashSet<String> =
                            merged_fields.iter().map(|f| f.name.clone()).collect();
                        let mut trait_fields_to_prepend = Vec::new();
                        for tf in resolved.fields {
                            if !struct_field_names.contains(&tf.name) {
                                trait_fields_to_prepend.push(StructField {
                                    name: tf.name,
                                    has_default: tf.has_default,
                                    default: tf.default,
                                });
                            }
                        }
                        if !trait_fields_to_prepend.is_empty() {
                            trait_fields_to_prepend.extend(merged_fields);
                            merged_fields = trait_fields_to_prepend;
                        }

                        // Merge trait methods (struct override > trait)
                        for (mname, mcallable) in resolved.methods {
                            if !merged_methods.contains_key(&mname) {
                                merged_methods.insert(mname, mcallable);
                            }
                        }

                        // Merge trait static methods (struct override > trait)
                        for (mname, mcallable) in resolved.static_methods {
                            if !merged_static.contains_key(&mname) {
                                merged_static.insert(mname, mcallable);
                            }
                        }
                    }

                    // Collect all abstract methods (own + inherited)
                    let mut final_abstract = own_abstract.clone();

                    // From parents (extends) - collect from all parents in MRO
                    for parent_name in &base_names {
                        if let Some(parent_type) = self.struct_registry.find_type_by_name(parent_name) {
                            for key in &parent_type.abstract_methods {
                                final_abstract.insert(key.clone());
                            }
                        }
                    }

                    // From traits (implements)
                    for key in trait_abstract {
                        final_abstract.insert(key);
                    }

                    // Remove methods that have concrete implementations
                    final_abstract.retain(|key| match key.kind {
                        super::structs::MethodKind::Instance => !merged_methods.contains_key(&key.name),
                        super::structs::MethodKind::Static => !merged_static.contains_key(&key.name),
                    });

                    // Concrete struct with unresolved abstracts => error
                    if own_abstract.is_empty() && !final_abstract.is_empty() {
                        let mut names: Vec<&str> = final_abstract.iter().map(|k| k.name.as_str()).collect();
                        names.sort();
                        return Err(VMError::RuntimeError(format!(
                            "struct '{}' must implement abstract method(s): {}",
                            name,
                            names.iter().map(|n| format!("'{}'", n)).collect::<Vec<_>>().join(", ")
                        )));
                    }

                    // Build full MRO: struct_mro (from C3) + trait_mro
                    let mut mro = struct_mro;
                    mro.extend(trait_mro);

                    let type_id = self.struct_registry.register_type_with_parents(
                        name.clone(),
                        merged_fields,
                        StructMethods {
                            instance: merged_methods,
                            statics: merged_static,
                            abstract_methods: final_abstract,
                        },
                        StructParents {
                            implements: implements_list,
                            mro,
                            parent_names: base_names,
                        },
                    );

                    // Build a callable CatnipStructType for Python-side access
                    let ty = self.struct_registry.get_type(type_id).unwrap();
                    let field_names: Vec<String> = ty.fields.iter().map(|f| f.name.clone()).collect();
                    let field_defaults: Vec<Option<Py<PyAny>>> = ty
                        .fields
                        .iter()
                        .map(|f| {
                            if f.has_default {
                                Some(f.default.to_pyobject(py))
                            } else {
                                None
                            }
                        })
                        .collect();
                    let methods_py: IndexMap<String, Py<PyAny>> =
                        ty.methods.iter().map(|(k, v)| (k.clone(), v.clone_ref(py))).collect();
                    let static_py: IndexMap<String, Py<PyAny>> = ty
                        .static_methods
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                        .collect();
                    let init_fn = ty.methods.get("init").map(|f| f.clone_ref(py));
                    let parent_names_py = ty.parent_names.clone();
                    let mro_py = ty.mro.clone();
                    let abstract_py = ty.abstract_methods.clone();

                    let struct_type_obj = Py::new(
                        py,
                        CatnipStructType {
                            name: name.clone(),
                            field_names,
                            field_defaults,
                            methods: methods_py,
                            static_methods: static_py,
                            init_fn,
                            parent_names: parent_names_py,
                            mro: mro_py,
                            abstract_methods: abstract_py,
                        },
                    )
                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                    let marker_ptr = struct_type_obj.as_ptr();
                    self.struct_type_map.insert(marker_ptr as usize, type_id);

                    // Insert once into ObjectTable, then clone_refcount for second use
                    let val = Value::from_owned_pyobject(struct_type_obj.into_any());
                    val.clone_refcount(); // two owners: host globals + vm globals

                    // Store in context.globals for Python-side access
                    host.store_global(py, &name, val)?;
                    // Also store in VM globals for scope resolution
                    self.globals.insert(name, val);
                }

                OpCode::MakeTrait => {
                    let const_idx = instr.arg as usize;
                    let trait_info_val = code.constants[const_idx];
                    let trait_info_py = trait_info_val.to_pyobject(py);
                    let info_tuple = trait_info_py
                        .bind(py)
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(format!("MakeTrait: bad constant: {e}")))?;

                    // (name, extends_tuple, fields_info, num_defaults, [methods])
                    let name: String = tuple_extract(info_tuple, 0)?;

                    let extends_obj = tuple_get(info_tuple, 1)?;
                    let mut extends: Vec<String> = Vec::new();
                    for e in extends_obj
                        .try_iter()
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?
                    {
                        let e = e.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        extends.push(e.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?);
                    }

                    let fields_info = tuple_get(info_tuple, 2)?;
                    let num_defaults: usize = tuple_extract(info_tuple, 3)?;

                    let has_methods = info_tuple.len() > 4;

                    // Read default values in stack order, then truncate
                    let stack_len = frame.stack.len();
                    let dstart = stack_len - num_defaults;
                    let default_values: Vec<Value> = frame.stack[dstart..].to_vec();
                    frame.stack.truncate(dstart);

                    // Parse fields
                    let fields_tuple = cast_tuple(&fields_info)?;
                    let mut trait_fields = Vec::new();
                    let mut default_idx = 0usize;
                    for fi in fields_tuple.iter() {
                        let pair = cast_tuple(&fi)?;
                        let fname: String = tuple_extract(pair, 0)?;
                        let has_default: bool = tuple_extract(pair, 1)?;
                        let default_val = if has_default {
                            let v = default_values[default_idx];
                            default_idx += 1;
                            v
                        } else {
                            Value::NIL
                        };
                        trait_fields.push(TraitField {
                            name: fname,
                            has_default,
                            default: default_val,
                        });
                    }

                    // Build method callables (same pattern as MakeStruct)
                    let mut method_bodies: IndexMap<String, Py<PyAny>> = IndexMap::new();
                    let mut trait_static_methods: IndexMap<String, Py<PyAny>> = IndexMap::new();
                    let mut abstract_methods: HashSet<MethodKey> = HashSet::new();
                    if has_methods {
                        let methods = tuple_get(info_tuple, 4)?;
                        for method_result in methods.try_iter().map_err(|e| VMError::RuntimeError(e.to_string()))? {
                            let method_pair = method_result.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let pair = cast_tuple(&method_pair)?;
                            let method_name: String = tuple_extract(pair, 0)?;

                            // Read is_static flag (3rd element, defaults to false)
                            let is_static: bool = if pair.len() > 2 {
                                tuple_extract(pair, 2).unwrap_or(false)
                            } else {
                                false
                            };

                            let code_obj = tuple_get(pair, 1)?;

                            // Abstract method: code_obj is None
                            if code_obj.is_none() {
                                abstract_methods.insert(MethodKey {
                                    name: method_name,
                                    kind: if is_static {
                                        super::structs::MethodKind::Static
                                    } else {
                                        super::structs::MethodKind::Instance
                                    },
                                });
                                continue;
                            }

                            // Build native captured HashMap
                            let mut captured: IndexMap<String, Value> = IndexMap::new();
                            if let Some(ref code) = frame.code {
                                for (lname, &slot_idx) in &code.slotmap {
                                    if host.has_global(py, lname) {
                                        continue;
                                    }
                                    let val = frame.get_local(slot_idx);
                                    if !val.is_nil() && !val.is_invalid() {
                                        captured.insert(lname.clone(), val);
                                    }
                                }
                            }
                            portabilize_struct_values(py, &mut captured, &mut self.struct_registry);

                            let parent = host.build_closure_parent(py, frame.closure_scope.as_ref());
                            let native_scope = NativeClosureScope::new(captured, parent);
                            let context_for_func = host.context().as_ref().map(|c| c.clone_ref(py));
                            let code_py: Py<PyCodeObject> = code_obj
                                .cast::<PyCodeObject>()
                                .map_err(|e| VMError::TypeError(format!("Expected CodeObject: {e}")))?
                                .clone()
                                .unbind();
                            let func = Py::new(
                                py,
                                VMFunction::create_native(py, code_py, Some(native_scope), context_for_func),
                            )
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                            if is_static {
                                trait_static_methods.insert(method_name, func.into_any());
                            } else {
                                method_bodies.insert(method_name, func.into_any());
                            }
                        }
                    }

                    // Register trait
                    let trait_def = TraitDef::new(
                        name,
                        extends,
                        trait_fields,
                        method_bodies,
                        abstract_methods,
                        trait_static_methods,
                    );
                    self.trait_registry.register_trait(trait_def);
                }

                OpCode::MakeEnum => {
                    let const_idx = instr.arg as usize;
                    let enum_info_val = code.constants[const_idx];
                    let enum_info_py = enum_info_val.to_pyobject(py);
                    let info_tuple = enum_info_py
                        .bind(py)
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(format!("MakeEnum: bad constant: {e}")))?;

                    let name: String = tuple_extract(info_tuple, 0)?;
                    let variants_obj = tuple_get(info_tuple, 1)?;
                    let variants_tuple = cast_tuple(&variants_obj)?;

                    let mut variant_names: Vec<String> = Vec::new();
                    for v in variants_tuple.iter() {
                        let vname: String = v.extract().map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                        variant_names.push(vname);
                    }

                    let type_id = self
                        .enum_registry
                        .register(&name, &variant_names, &mut self.symbol_table);

                    // Create a Python marker object for the enum type and store as global
                    let enum_type_obj = Py::new(py, CatnipEnumType::new(name.clone(), type_id, &variant_names))
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let marker_ptr = enum_type_obj.as_ptr() as usize;
                    self.enum_type_map.insert(marker_ptr, type_id);

                    let val = Value::from_owned_pyobject(enum_type_obj.into_any());
                    val.clone_refcount();
                    host.store_global(py, &name, val)?;
                    self.globals.insert(name, val);
                }

                // --- Exception handling ---
                OpCode::SetupExcept => {
                    frame.handler_stack.push(catnip_core::exception::Handler {
                        handler_type: catnip_core::exception::HandlerType::Except,
                        target_addr: instr.arg as usize,
                        stack_depth: frame.stack.len(),
                        block_depth: frame.block_stack.len(),
                    });
                }
                OpCode::SetupFinally => {
                    frame.handler_stack.push(catnip_core::exception::Handler {
                        handler_type: catnip_core::exception::HandlerType::Finally,
                        target_addr: instr.arg as usize,
                        stack_depth: frame.stack.len(),
                        block_depth: frame.block_stack.len(),
                    });
                }
                OpCode::PopHandler => {
                    frame.handler_stack.pop();
                }
                OpCode::CheckExcMatch => {
                    let const_val = code.constants[instr.arg as usize];
                    let py_obj = const_val.to_pyobject(py);
                    let type_name_to_match: String = py_obj.bind(py).str().map(|s| s.to_string()).unwrap_or_default();
                    let matches = if let Some(exc_info) = frame.active_exception_stack.last() {
                        exc_info.matches(&type_name_to_match)
                    } else {
                        false
                    };
                    frame.push(Value::from_bool(matches));
                }
                OpCode::LoadException => {
                    if instr.arg == 1 {
                        // ExcInfo mode: push (exc_type_class, exc_instance, None) tuple
                        if let Some(exc_info) = frame.active_exception_stack.last() {
                            let builtins = py.import("builtins").unwrap();
                            let exc_type = builtins
                                .getattr(exc_info.type_name.as_str())
                                .unwrap_or_else(|_| builtins.getattr("RuntimeError").unwrap());
                            let exc_val = exc_type
                                .call1((&exc_info.message,))
                                .unwrap_or_else(|_| py.None().into_bound(py));
                            let tuple = pyo3::types::PyTuple::new(
                                py,
                                &[exc_type.unbind().into_any(), exc_val.unbind().into_any(), py.None()],
                            )
                            .unwrap();
                            frame.push(Value::from_owned_pyobject(tuple.unbind().into_any()));
                        } else {
                            let tuple = pyo3::types::PyTuple::new(py, &[py.None(), py.None(), py.None()]).unwrap();
                            frame.push(Value::from_owned_pyobject(tuple.unbind().into_any()));
                        }
                    } else if let Some(exc_info) = frame.active_exception_stack.last() {
                        let py_str = PyString::new(py, &exc_info.message);
                        frame.push(Value::from_owned_pyobject(py_str.unbind().into_any()));
                    } else {
                        frame.push(Value::NIL);
                    }
                }
                OpCode::Raise => {
                    if instr.arg == 1 {
                        // Bare raise: re-raise preserving full MRO
                        if let Some(exc_info) = frame.active_exception_stack.last().cloned() {
                            return Err(VMError::UserException(exc_info));
                        } else {
                            return Err(VMError::RuntimeError(errors::ERR_NO_ACTIVE_EXCEPTION.into()));
                        }
                    } else {
                        // raise expr: pop value, detect exception type
                        let val = frame.pop();
                        let err = if let Some(inst_idx) = val.as_struct_instance_idx() {
                            // Struct instance: get real type name from registry
                            // (to_pyobject wraps in CatnipStruct proxy, losing the name)
                            let type_name = self
                                .struct_registry
                                .get_instance(inst_idx)
                                .and_then(|inst| self.struct_registry.get_type(inst.type_id))
                                .map(|ty| ty.name.clone())
                                .unwrap_or_else(|| "RuntimeError".to_string());
                            let msg = self
                                .struct_registry
                                .get_instance(inst_idx)
                                .and_then(|inst| inst.fields.first().copied())
                                .map(|v| {
                                    let obj = v.to_pyobject(py);
                                    obj.bind(py).str().map(|s| s.to_string()).unwrap_or_default()
                                })
                                .unwrap_or_default();
                            let mro = self
                                .struct_registry
                                .find_type_by_name(&type_name)
                                .map(|ty| ty.mro.clone())
                                .unwrap_or_else(|| vec![type_name.clone(), "Exception".to_string()]);
                            VMError::UserException(catnip_core::exception::ExceptionInfo::new(type_name, msg, mro))
                        } else {
                            // Python object: detect type from Python introspection
                            let py_obj = val.to_pyobject(py);
                            let bound = py_obj.bind(py);
                            let msg = bound.str().map(|s| s.to_string()).unwrap_or_default();
                            let type_name = bound.get_type().name().map(|n| n.to_string()).unwrap_or_default();
                            if let Some(kind) = catnip_core::exception::ExceptionKind::from_name(&type_name) {
                                // Known exception: dedicated variant when available,
                                // UserException for group types that would lose identity
                                let test_err = VMError::from_exception_info(&type_name, &msg);
                                if matches!(&test_err, VMError::RuntimeError(_)) && type_name != "RuntimeError" {
                                    VMError::UserException(catnip_core::exception::ExceptionInfo::from_kind(kind, msg))
                                } else {
                                    test_err
                                }
                            } else {
                                VMError::RuntimeError(msg)
                            }
                        };
                        decref_discard(&mut self.struct_registry, val);
                        return Err(err);
                    }
                }
                OpCode::ResumeUnwind => {
                    if let Some(pending) = frame.pending_unwind.take() {
                        match pending {
                            catnip_core::exception::PendingUnwind::Exception(info) => {
                                return Err(VMError::UserException(info));
                            }
                            catnip_core::exception::PendingUnwind::Return => {
                                let val = frame.pop();
                                return Err(VMError::Return(val));
                            }
                            catnip_core::exception::PendingUnwind::Break => {
                                return Err(VMError::Break);
                            }
                            catnip_core::exception::PendingUnwind::Continue => {
                                return Err(VMError::Continue);
                            }
                        }
                    } else if let Some(exc_info) = frame.active_exception_stack.last().cloned() {
                        // Fallback: re-raise from active exception (except no-match -> finally case)
                        return Err(VMError::UserException(exc_info));
                    }
                    // No pending unwind: finally on happy path, just continue
                }
                OpCode::ClearException => {
                    frame.active_exception_stack.pop();
                }

                // --- ND Operations ---
                OpCode::NdEmptyTopos => {
                    // Get cached NDTopos singleton or create it
                    if self.cached_nd_topos.is_none() {
                        let nd_module = py.import(PY_MOD_ND).map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let nd_topos_class = nd_module
                            .getattr("NDTopos")
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let instance = nd_topos_class
                            .call_method0("instance")
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        self.cached_nd_topos = Some(instance.unbind());
                    }

                    let instance = self.cached_nd_topos.as_ref().unwrap();
                    let value = Value::from_pyobject(py, instance.bind(py))
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    frame.push(value);
                }

                OpCode::NdRecursion => {
                    let form = instr.arg;

                    if form == 0 {
                        // Combinator: pop lambda, pop seed
                        let lambda_val = frame.pop();
                        let seed_val = frame.pop();
                        let lambda_py = lambda_val.to_pyobject(py);
                        let seed_py = seed_val.to_pyobject(py);

                        let result = host.execute_nd_recursion(py, seed_py.bind(py), lambda_py.bind(py))?;
                        let value = Value::from_pyobject(py, result.bind(py))
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        frame.push(value);
                    } else {
                        // Declaration: pop lambda, wrap in NDDeclaration
                        let lambda_val = frame.pop();
                        let lambda_py = lambda_val.to_pyobject(py);
                        if let Some(ctx) = host.context() {
                            let decl = Py::new(py, crate::nd::NDDeclaration::new(lambda_py, ctx.clone_ref(py)))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let value = Value::from_pyobject(py, decl.into_any().bind(py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            frame.push(value);
                        } else {
                            // Standalone mode: wrap in NDVmDecl so f(seed) calls lambda(seed, f)
                            let decl = Py::new(py, crate::nd::NDVmDecl::new(lambda_py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let value = Value::from_pyobject(py, decl.into_any().bind(py))
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            frame.push(value);
                        }
                    }
                }

                OpCode::NdMap => {
                    let form = instr.arg;

                    if form == 0 {
                        // Applicative: pop func, pop data
                        let func_val = frame.pop();
                        let data_val = frame.pop();
                        let func_py = func_val.to_pyobject(py);
                        let data_py = data_val.to_pyobject(py);

                        let result = host.execute_nd_map(py, data_py.bind(py), func_py.bind(py))?;
                        let value = Value::from_pyobject(py, result.bind(py))
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        frame.push(value);
                    } else {
                        // Lift: pop func, push back
                        let func_val = frame.pop();
                        frame.push(func_val);
                    }
                }

                OpCode::MatchFail => {
                    let msg_idx = instr.arg as usize;
                    let msg = code.constants[msg_idx].to_pyobject(py);
                    let msg_str: String = msg.bind(py).extract().unwrap_or_default();
                    return Err(VMError::RuntimeError(msg_str));
                }

                // --- String formatting ---
                OpCode::FormatValue => {
                    // flags = (conv << 1) | has_spec
                    let flags = instr.arg;
                    let has_spec = (flags & 1) != 0;
                    let conv = (flags >> 1) & 3;

                    let spec_obj = if has_spec {
                        frame.pop().to_pyobject(py)
                    } else {
                        "".into_pyobject(py).unwrap().into_any().unbind()
                    };
                    let value = frame.pop().to_pyobject(py);

                    let builtins = py.import("builtins").to_vm(py)?;

                    // Apply conversion: 0=none, 1=str, 2=repr, 3=ascii
                    let converted = match conv {
                        1 => builtins
                            .getattr("str")
                            .to_vm(py)?
                            .call1((value.bind(py),))
                            .to_vm(py)?
                            .unbind(),
                        2 => builtins
                            .getattr("repr")
                            .to_vm(py)?
                            .call1((value.bind(py),))
                            .to_vm(py)?
                            .unbind(),
                        3 => builtins
                            .getattr("ascii")
                            .to_vm(py)?
                            .call1((value.bind(py),))
                            .to_vm(py)?
                            .unbind(),
                        _ => value,
                    };

                    let result = builtins
                        .getattr("format")
                        .to_vm(py)?
                        .call1((converted.bind(py), spec_obj.bind(py)))
                        .to_vm(py)?;
                    frame.push(Value::from_owned_pyobject(result.unbind()));
                }

                OpCode::BuildString => {
                    let n = instr.arg as usize;
                    let stack_len = frame.stack.len();
                    let start = stack_len - n;

                    let mut buf = String::with_capacity(n * 16);
                    for i in start..stack_len {
                        let py_obj = frame.stack[i].to_pyobject(py);
                        let s: String = py_obj.bind(py).extract().unwrap_or_default();
                        buf.push_str(&s);
                    }
                    frame.stack.truncate(start);

                    let py_str = PyString::new(py, &buf);
                    frame.push(Value::from_owned_pyobject(py_str.unbind().into_any()));
                }

                OpCode::Halt => {
                    last_result = if frame.stack.is_empty() {
                        Value::NIL
                    } else {
                        frame.pop()
                    };
                    // Sync locals to globals for module-level code
                    // Only sync variables that were stored via STORE_NAME (already in globals)
                    // Loop variables (stored via STORE_LOCAL only) are not synced
                    let sync_data: Vec<(String, Value)> = if let Some(code) = &frame.code {
                        code.slotmap
                            .iter()
                            .filter_map(|(name, &slot)| {
                                // Only sync if this name was already stored via STORE_NAME
                                if !self.globals.contains_key(name) {
                                    return None;
                                }
                                if slot < frame.locals.len() {
                                    let val = frame.locals[slot];
                                    if !val.is_nil() && !val.is_invalid() {
                                        Some((name.clone(), val))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    // Check if main frame (current frame is not on stack)
                    let is_main_frame = self.frame_stack.is_empty();
                    if is_main_frame {
                        for (name, val) in sync_data {
                            self.globals.insert(name, val);
                        }
                    }
                    return Ok(last_result);
                }
            }

            // Post-dispatch debug pause: instruction was already executed
            if debug_should_pause {
                self.debug_last_paused_byte = Some(_current_src_byte);
                let locals_data: Vec<(String, Value)> = if let Some(ref code) = frame.code {
                    code.slotmap
                        .iter()
                        .filter_map(|(name, &slot)| {
                            if slot < frame.locals.len() {
                                Some((name.clone(), frame.locals[slot]))
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let call_stack_snapshot: Vec<(String, u32)> = self
                    .call_stack
                    .iter()
                    .map(|ci| (ci.name.clone(), ci.call_start_byte))
                    .collect();
                let depth = self.call_stack.len();
                // frame no longer used past this point
                self.debug_step_mode = DebugStepMode::Disabled;
                let action = self.invoke_debug_callback(py, _current_src_byte, &locals_data, &call_stack_snapshot)?;
                self.debug_step_mode = action;
                if action == DebugStepMode::StepOver || action == DebugStepMode::StepOut {
                    self.debug_step_depth = depth;
                }
                continue;
            }
        }
    }

    // --- Exception unwinding ---

    /// Try to unwind to a handler in the current frame, or walk up the call stack.
    fn unwind_exception(&mut self, frame: &mut Frame, err: &VMError) -> bool {
        // Try current frame
        if self.try_unwind_to_handler(frame, err) {
            return true;
        }
        // For catchable exceptions, walk up the call stack
        if err.is_catchable() {
            while let Some(caller) = self.frame_stack.pop() {
                let old = std::mem::replace(frame, caller);
                self.frame_pool.free(old, &mut self.struct_registry);
                if self.try_unwind_to_handler(frame, err) {
                    return true;
                }
            }
        }
        false
    }

    /// Try to find and activate a handler in the current frame.
    fn try_unwind_to_handler(&mut self, frame: &mut Frame, err: &VMError) -> bool {
        while let Some(handler) = frame.handler_stack.last() {
            match handler.handler_type {
                catnip_core::exception::HandlerType::Except => {
                    if err.is_catchable() {
                        let handler = frame.handler_stack.pop().unwrap();
                        if let Some(info) = err.exception_info() {
                            frame.active_exception_stack.push(info);
                        }
                        frame.stack.truncate(handler.stack_depth);
                        frame.block_stack.truncate(handler.block_depth);
                        frame.ip = handler.target_addr;
                        return true;
                    }
                    // Control flow signal: skip Except handler
                    frame.handler_stack.pop();
                }
                catnip_core::exception::HandlerType::Finally => {
                    let handler = frame.handler_stack.pop().unwrap();
                    frame.pending_unwind = Some(err.to_pending_unwind());
                    frame.stack.truncate(handler.stack_depth);
                    frame.block_stack.truncate(handler.block_depth);
                    // For Return, save the value on the stack
                    if let VMError::Return(val) = err {
                        frame.push(*val);
                    }
                    frame.ip = handler.target_addr;
                    return true;
                }
            }
        }
        false
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

// --- Binary operations on NaN-boxed values ---

// Floor div/mod helpers -- shared from catnip_vm (type-independent, no Value dependency).
use catnip_vm::ops::arith::{i64_div_floor, i64_mod_floor};
use catnip_vm::ops::errors;

// NOTE: binary_*, compare_*, eq_without_python, to_f64, to_bigint, bigint_binop, bigint_cmp
// remain local because catnip_rs::vm::value::Value is a distinct type from catnip_vm::Value.
// Type unification is Phase 5 (pipeline integration).

/// Promote a Value (SmallInt or BigInt) to owned Integer for mixed arithmetic.
#[inline]
fn to_bigint(v: Value) -> Option<Integer> {
    if let Some(i) = v.as_int() {
        Some(Integer::from(i))
    } else if v.is_bigint() {
        Some(unsafe { v.as_bigint_ref().unwrap().clone() })
    } else {
        None
    }
}

/// Apply a binary BigInt operation using references (zero-clone).
#[inline]
fn bigint_binop<F>(a: Value, b: Value, op: F) -> Option<Value>
where
    F: FnOnce(&Integer, &Integer) -> Integer,
{
    if a.is_bigint() && b.is_bigint() {
        let (ra, rb) = unsafe { (a.as_bigint_ref().unwrap(), b.as_bigint_ref().unwrap()) };
        return Some(Value::from_bigint_or_demote(op(ra, rb)));
    }
    if a.is_bigint() {
        if let Some(bi) = b.as_int() {
            let ra = unsafe { a.as_bigint_ref().unwrap() };
            let tmp = Integer::from(bi);
            return Some(Value::from_bigint_or_demote(op(ra, &tmp)));
        }
    }
    if b.is_bigint() {
        if let Some(ai) = a.as_int() {
            let rb = unsafe { b.as_bigint_ref().unwrap() };
            let tmp = Integer::from(ai);
            return Some(Value::from_bigint_or_demote(op(&tmp, rb)));
        }
    }
    None
}

/// Apply a BigInt comparison using references (zero-clone).
#[inline]
fn bigint_cmp<F>(a: Value, b: Value, cmp: F) -> Option<bool>
where
    F: FnOnce(&Integer, &Integer) -> bool,
{
    if a.is_bigint() && b.is_bigint() {
        let (ra, rb) = unsafe { (a.as_bigint_ref().unwrap(), b.as_bigint_ref().unwrap()) };
        return Some(cmp(ra, rb));
    }
    if a.is_bigint() {
        if let Some(bi) = b.as_int() {
            let ra = unsafe { a.as_bigint_ref().unwrap() };
            let tmp = Integer::from(bi);
            return Some(cmp(ra, &tmp));
        }
    }
    if b.is_bigint() {
        if let Some(ai) = a.as_int() {
            let rb = unsafe { b.as_bigint_ref().unwrap() };
            let tmp = Integer::from(ai);
            return Some(cmp(&tmp, rb));
        }
    }
    None
}

/// Convert a Value to f64 for float promotion.
#[inline]
fn to_f64(v: Value) -> Option<f64> {
    v.as_float().or_else(|| v.as_int().map(|i| i as f64)).or_else(|| {
        if v.is_bigint() {
            Some(unsafe { v.as_bigint_ref().unwrap() }.to_f64())
        } else {
            None
        }
    })
}

#[inline]
fn to_complex(v: Value) -> Option<(f64, f64)> {
    if v.is_complex() {
        return unsafe { v.as_complex_parts() };
    }
    to_f64(v).map(|f| (f, 0.0))
}

fn complex_pow(ar: f64, ai: f64, br: f64, bi: f64) -> VMResult<Value> {
    if br == 0.0 && bi == 0.0 {
        return Ok(Value::from_complex(1.0, 0.0));
    }
    if ar == 0.0 && ai == 0.0 {
        if bi == 0.0 && br > 0.0 {
            return Ok(Value::from_complex(0.0, 0.0));
        }
        return Err(VMError::ZeroDivisionError("0.0 to a negative or complex power".into()));
    }
    let r = (ar * ar + ai * ai).sqrt();
    let theta = ai.atan2(ar);
    let ln_r = r.ln();
    let exp_r = br * ln_r - bi * theta;
    let exp_i = br * theta + bi * ln_r;
    let mag = exp_r.exp();
    Ok(Value::from_complex(mag * exp_i.cos(), mag * exp_i.sin()))
}

/// Compare two Values in Rust when possible; returns None if Python fallback needed.
#[inline]
fn eq_without_python(a: Value, b: Value) -> Option<bool> {
    if a.bits() == b.bits() && !a.is_pyobj() && !a.is_float() {
        return Some(true);
    }
    if a.is_nil() || b.is_nil() {
        return Some(a.is_nil() && b.is_nil());
    }
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Some(ai == bi);
    }
    if let (Some(ab), Some(bb)) = (a.as_bool(), b.as_bool()) {
        return Some(ab == bb);
    }
    if a.is_complex() || b.is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return Some(ar == br && ai == bi);
        }
    }
    if a.is_bigint() || b.is_bigint() {
        return bigint_cmp(a, b, |x, y| x == y);
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Some(af == bf);
    }
    None
}

#[inline]
fn binary_add(a: Value, b: Value) -> VMResult<Value> {
    if a.is_complex() || b.is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return Ok(Value::from_complex(ar + br, ai + bi));
        }
    }
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(sum) = ai.checked_add(bi) {
            if let Some(v) = Value::try_from_int(sum) {
                return Ok(v);
            }
        }
        return Ok(Value::from_bigint_or_demote(Integer::from(ai) + Integer::from(bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x + y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(Value::from_float(af + bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.as_float(), b.as_float()) {
        return Ok(Value::from_float(af + bf));
    }
    if let (Some(ai), Some(bf)) = (a.as_int(), b.as_float()) {
        return Ok(Value::from_float(ai as f64 + bf));
    }
    if let (Some(af), Some(bi)) = (a.as_float(), b.as_int()) {
        return Ok(Value::from_float(af + bi as f64));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_ADD.into()))
}

#[inline]
fn binary_sub(a: Value, b: Value) -> VMResult<Value> {
    if a.is_complex() || b.is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return Ok(Value::from_complex(ar - br, ai - bi));
        }
    }
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(diff) = ai.checked_sub(bi) {
            if let Some(v) = Value::try_from_int(diff) {
                return Ok(v);
            }
        }
        return Ok(Value::from_bigint_or_demote(Integer::from(ai) - Integer::from(bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x - y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(Value::from_float(af - bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.as_float(), b.as_float()) {
        return Ok(Value::from_float(af - bf));
    }
    if let (Some(ai), Some(bf)) = (a.as_int(), b.as_float()) {
        return Ok(Value::from_float(ai as f64 - bf));
    }
    if let (Some(af), Some(bi)) = (a.as_float(), b.as_int()) {
        return Ok(Value::from_float(af - bi as f64));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_SUB.into()))
}

#[inline]
fn binary_mul(a: Value, b: Value) -> VMResult<Value> {
    if a.is_complex() || b.is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return Ok(Value::from_complex(ar * br - ai * bi, ar * bi + ai * br));
        }
    }
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(prod) = ai.checked_mul(bi) {
            if let Some(v) = Value::try_from_int(prod) {
                return Ok(v);
            }
        }
        return Ok(Value::from_bigint_or_demote(Integer::from(ai) * Integer::from(bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x * y)) {
            return Ok(v);
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(Value::from_float(af * bf));
        }
    }
    if let (Some(af), Some(bf)) = (a.as_float(), b.as_float()) {
        return Ok(Value::from_float(af * bf));
    }
    if let (Some(ai), Some(bf)) = (a.as_int(), b.as_float()) {
        return Ok(Value::from_float(ai as f64 * bf));
    }
    if let (Some(af), Some(bi)) = (a.as_float(), b.as_int()) {
        return Ok(Value::from_float(af * bi as f64));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_MUL.into()))
}

#[inline]
fn binary_div(a: Value, b: Value) -> VMResult<Value> {
    if a.is_complex() || b.is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            let denom = br * br + bi * bi;
            if denom == 0.0 {
                return Err(VMError::ZeroDivisionError(errors::ERR_FLOAT_DIV_ZERO.into()));
            }
            return Ok(Value::from_complex(
                (ar * br + ai * bi) / denom,
                (ai * br - ar * bi) / denom,
            ));
        }
    }
    if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError(errors::ERR_FLOAT_DIV_ZERO.into()));
        }
        return Ok(Value::from_float(af / bf));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_DIV.into()))
}

#[inline]
fn binary_floordiv(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi == 0 {
            return Err(VMError::ZeroDivisionError(errors::ERR_INT_DIV_ZERO.into()));
        }
        return Ok(Value::from_int(i64_div_floor(ai, bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if b.is_bigint() {
            if unsafe { b.as_bigint_ref().unwrap().cmp0() == std::cmp::Ordering::Equal } {
                return Err(VMError::ZeroDivisionError(errors::ERR_INT_DIV_ZERO.into()));
            }
        } else if b.as_int() == Some(0) {
            return Err(VMError::ZeroDivisionError(errors::ERR_INT_DIV_ZERO.into()));
        }
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x).div_floor(y)) {
            return Ok(v);
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError(errors::ERR_FLOAT_FLOORDIV_ZERO.into()));
        }
        return Ok(Value::from_float((af / bf).floor()));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_FLOORDIV.into()))
}

#[inline]
fn binary_mod(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi == 0 {
            return Err(VMError::ZeroDivisionError(errors::ERR_INT_DIV_ZERO.into()));
        }
        return Ok(Value::from_int(i64_mod_floor(ai, bi)));
    }
    if a.is_bigint() || b.is_bigint() {
        if b.is_bigint() {
            if unsafe { b.as_bigint_ref().unwrap().cmp0() == std::cmp::Ordering::Equal } {
                return Err(VMError::ZeroDivisionError(errors::ERR_INT_DIV_ZERO.into()));
            }
        } else if b.as_int() == Some(0) {
            return Err(VMError::ZeroDivisionError(errors::ERR_INT_DIV_ZERO.into()));
        }
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x).rem_floor(y)) {
            return Ok(v);
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError(errors::ERR_FLOAT_MOD_ZERO.into()));
        }
        return Ok(Value::from_float(af - bf * (af / bf).floor()));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_MOD.into()))
}

#[inline]
fn binary_pow(a: Value, b: Value) -> VMResult<Value> {
    if a.is_complex() || b.is_complex() {
        if let (Some((ar, ai)), Some((br, bi))) = (to_complex(a), to_complex(b)) {
            return complex_pow(ar, ai, br, bi);
        }
    }
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi >= 0 {
            if bi <= 64 {
                if let Some(result) = ai.checked_pow(bi as u32) {
                    if let Some(v) = Value::try_from_int(result) {
                        return Ok(v);
                    }
                }
            }
            let base = Integer::from(ai);
            if let Ok(exp) = u32::try_from(bi) {
                return Ok(Value::from_bigint_or_demote(base.pow(exp)));
            }
            return Ok(Value::from_float((ai as f64).powf(bi as f64)));
        }
        return Ok(Value::from_float((ai as f64).powf(bi as f64)));
    }
    if a.is_bigint() || b.is_bigint() {
        if let (Some(base), Some(bi)) = (to_bigint(a), b.as_int()) {
            if bi >= 0 {
                if let Ok(exp) = u32::try_from(bi) {
                    return Ok(Value::from_bigint_or_demote(base.pow(exp)));
                }
            }
            if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
                return Ok(Value::from_float(af.powf(bf)));
            }
        }
        if let (Some(af), Some(bf)) = (to_f64(a), to_f64(b)) {
            return Ok(Value::from_float(af.powf(bf)));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_float(af.powf(bf)));
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_POW.into()))
}

#[inline]
fn unary_neg(a: Value) -> VMResult<Value> {
    if a.is_complex() {
        let (r, i) = unsafe { a.as_complex_parts().unwrap() };
        return Ok(Value::from_complex(-r, -i));
    }
    if let Some(i) = a.as_int() {
        if let Some(v) = Value::try_from_int(-i) {
            return Ok(v);
        }
        return Ok(Value::from_bigint_or_demote(-Integer::from(i)));
    }
    if a.is_bigint() {
        let n = unsafe { a.as_bigint_ref().unwrap() };
        return Ok(Value::from_bigint_or_demote(Integer::from(-n)));
    }
    if let Some(f) = a.as_float() {
        return Ok(Value::from_float(-f));
    }
    Err(VMError::TypeError(errors::ERR_BAD_UNARY_NEG.into()))
}

// --- Comparison operations ---

#[inline]
fn compare_lt(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai < bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x < y) {
            return Ok(Value::from_bool(r));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af < bf));
    }
    Err(VMError::TypeError(errors::ERR_CMP_LT.into()))
}

#[inline]
fn compare_le(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai <= bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x <= y) {
            return Ok(Value::from_bool(r));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af <= bf));
    }
    Err(VMError::TypeError(errors::ERR_CMP_LE.into()))
}

#[inline]
fn compare_gt(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai > bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x > y) {
            return Ok(Value::from_bool(r));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af > bf));
    }
    Err(VMError::TypeError(errors::ERR_CMP_GT.into()))
}

#[inline]
fn compare_ge(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai >= bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(r) = bigint_cmp(a, b, |x, y| x >= y) {
            return Ok(Value::from_bool(r));
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af >= bf));
    }
    Err(VMError::TypeError(errors::ERR_CMP_GE.into()))
}

// --- Struct operator dispatch ---

/// Try to dispatch a binary operator on a struct instance.
/// Returns Some((code, closure, args)) if the struct has the method, None otherwise.
fn try_struct_binop(
    registry: &StructRegistry,
    py: Python<'_>,
    a: Value,
    b: Value,
    method_name: &str,
) -> Option<(Arc<CodeObject>, Option<NativeClosureScope>, Vec<Value>)> {
    let idx = a.as_struct_instance_idx()?;
    let inst = registry.get_instance(idx)?;
    let ty = registry.get_type(inst.type_id)?;
    let func = ty.methods.get(method_name)?;
    let func_bound = func.bind(py);
    if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
        let r = vm_func.borrow();
        let code = Arc::clone(&r.vm_code.borrow(py).inner);
        let closure = r.native_closure.clone();
        drop(r);
        Some((code, closure, vec![a, b]))
    } else {
        None
    }
}

/// Try reverse dispatch: look up method on `b` (right operand) when `a` lacks it.
/// Args are passed as (b, a) - the struct stays as self.
fn try_struct_rbinop(
    registry: &StructRegistry,
    py: Python<'_>,
    a: Value,
    b: Value,
    method_name: &str,
) -> Option<(Arc<CodeObject>, Option<NativeClosureScope>, Vec<Value>)> {
    let idx = b.as_struct_instance_idx()?;
    let inst = registry.get_instance(idx)?;
    let ty = registry.get_type(inst.type_id)?;
    let func = ty.methods.get(method_name)?;
    let func_bound = func.bind(py);
    if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
        let r = vm_func.borrow();
        let code = Arc::clone(&r.vm_code.borrow(py).inner);
        let closure = r.native_closure.clone();
        drop(r);
        Some((code, closure, vec![b, a]))
    } else {
        None
    }
}

/// Try to dispatch a unary operator on a struct instance.
fn try_struct_unaryop(
    registry: &StructRegistry,
    py: Python<'_>,
    a: Value,
    method_name: &str,
) -> Option<(Arc<CodeObject>, Option<NativeClosureScope>, Vec<Value>)> {
    let idx = a.as_struct_instance_idx()?;
    let inst = registry.get_instance(idx)?;
    let ty = registry.get_type(inst.type_id)?;
    let func = ty.methods.get(method_name)?;
    let func_bound = func.bind(py);
    if let Ok(vm_func) = func_bound.cast::<VMFunction>() {
        let r = vm_func.borrow();
        let code = Arc::clone(&r.vm_code.borrow(py).inner);
        let closure = r.native_closure.clone();
        drop(r);
        Some((code, closure, vec![a]))
    } else {
        None
    }
}

// --- Bitwise operations ---

#[inline]
fn bitwise_or(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai | bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x | y)) {
            return Ok(v);
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITOR.into()))
}

#[inline]
fn bitwise_xor(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai ^ bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x ^ y)) {
            return Ok(v);
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITXOR.into()))
}

#[inline]
fn bitwise_and(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai & bi));
    }
    if a.is_bigint() || b.is_bigint() {
        if let Some(v) = bigint_binop(a, b, |x, y| Integer::from(x & y)) {
            return Ok(v);
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITAND.into()))
}

#[inline]
fn bitwise_not(a: Value) -> VMResult<Value> {
    if let Some(i) = a.as_int() {
        return Ok(Value::from_int(!i));
    }
    if a.is_bigint() {
        let n = unsafe { a.as_bigint_ref().unwrap() };
        return Ok(Value::from_bigint_or_demote(Integer::from(!n)));
    }
    Err(VMError::TypeError(errors::ERR_BAD_UNARY_NOT.into()))
}

#[inline]
fn bitwise_lshift(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi >= 0 {
            if bi < 64 {
                if let Some(v) = Value::try_from_int(ai << bi) {
                    return Ok(v);
                }
            }
            // Overflow or large shift: promote to BigInt
            if let Ok(shift) = u32::try_from(bi) {
                return Ok(Value::from_bigint_or_demote(Integer::from(ai) << shift));
            }
        }
    }
    if a.is_bigint() || b.is_bigint() {
        if let (Some(ba), Some(bi)) = (to_bigint(a), b.as_int()) {
            if bi >= 0 {
                if let Ok(shift) = u32::try_from(bi) {
                    return Ok(Value::from_bigint_or_demote(ba << shift));
                }
            }
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_LSHIFT.into()))
}

#[inline]
fn bitwise_rshift(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if (0..64).contains(&bi) {
            return Ok(Value::from_int(ai >> bi));
        }
    }
    if a.is_bigint() || b.is_bigint() {
        if let (Some(ba), Some(bi)) = (to_bigint(a), b.as_int()) {
            if bi >= 0 {
                if let Ok(shift) = u32::try_from(bi) {
                    return Ok(Value::from_bigint_or_demote(ba >> shift));
                }
            }
        }
    }
    Err(VMError::TypeError(errors::ERR_UNSUPPORTED_RSHIFT.into()))
}

#[inline]
fn compare_eq(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    if let Some(r) = eq_without_python(a, b) {
        return Ok(Value::from_bool(r));
    }
    // For PyObjects (lists, strings, etc.), delegate to Python's ==
    inc(&PY_COMPARE_EQ_FALLBACKS);
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let result = py_a.bind(py).eq(&py_b).to_vm(py)?;
    Ok(Value::from_bool(result))
}

#[inline]
fn compare_ne(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    if let Some(r) = eq_without_python(a, b) {
        return Ok(Value::from_bool(!r));
    }
    // For PyObjects, delegate to Python's !=
    inc(&PY_COMPARE_NE_FALLBACKS);
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let result = py_a.bind(py).ne(&py_b).to_vm(py)?;
    Ok(Value::from_bool(result))
}

/// Match a VMPattern against a Value entirely in Rust (no Python boundary crossing).
/// Returns Some(bindings) with (slot, value) pairs on match, None on mismatch.
fn vm_match_pattern(
    py: Python<'_>,
    pattern: &VMPattern,
    value: Value,
    registry: &StructRegistry,
) -> PyResult<Option<Vec<(usize, Value)>>> {
    match pattern {
        VMPattern::Wildcard => Ok(Some(Vec::new())),
        VMPattern::Var(slot) => Ok(Some(vec![(*slot, value)])),
        VMPattern::Literal(expected) => {
            // Fast path: compare in Rust for primitive and BigInt values.
            if let Some(eq) = eq_without_python(value, *expected) {
                return if eq { Ok(Some(Vec::new())) } else { Ok(None) };
            }
            // Pointer/bits equality still short-circuits for reference-like payloads.
            if value.bits() == expected.bits() {
                return Ok(Some(Vec::new()));
            }
            // Fallback: Python equality for strings, PyObj, etc.
            inc(&PY_PATTERN_LITERAL_EQ_FALLBACKS);
            let py_val = value.to_pyobject(py);
            let py_exp = expected.to_pyobject(py);
            if py_val.bind(py).eq(py_exp.bind(py))? {
                Ok(Some(Vec::new()))
            } else {
                Ok(None)
            }
        }
        VMPattern::Or(sub_patterns) => {
            for sub in sub_patterns {
                if let Some(bindings) = vm_match_pattern(py, sub, value, registry)? {
                    return Ok(Some(bindings));
                }
            }
            Ok(None)
        }
        VMPattern::Tuple(elements) => {
            // Convert value to Python iterable then to items
            let py_val = value.to_pyobject(py);
            let py_bound = py_val.bind(py);
            let items: Vec<Value> = match py_bound.try_iter() {
                Ok(iter) => {
                    let mut v = Vec::new();
                    for item in iter {
                        v.push(Value::from_pyobject(py, &item?)?);
                    }
                    v
                }
                Err(_) => return Ok(None),
            };

            // Find star element if present
            let mut star_idx: Option<usize> = None;
            let mut non_star_count = 0;
            for (i, elem) in elements.iter().enumerate() {
                match elem {
                    VMPatternElement::Star(_) => {
                        if star_idx.is_some() {
                            return Ok(None); // Multiple stars
                        }
                        star_idx = Some(i);
                    }
                    VMPatternElement::Pattern(_) => non_star_count += 1,
                }
            }

            let mut bindings = Vec::new();

            if star_idx.is_none() {
                // No star: exact length required
                if items.len() != non_star_count {
                    return Ok(None);
                }
                for (i, elem) in elements.iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        match vm_match_pattern(py, sub, items[i], registry)? {
                            Some(sub_bindings) => bindings.extend(sub_bindings),
                            None => return Ok(None),
                        }
                    }
                }
            } else if let Some(star_pos) = star_idx {
                if items.len() < non_star_count {
                    return Ok(None);
                }

                let n_before = star_pos;
                let n_after = elements.len() - star_pos - 1;

                // Match before star
                let mut item_idx = 0;
                for elem in &elements[..star_pos] {
                    if let VMPatternElement::Pattern(sub) = elem {
                        match vm_match_pattern(py, sub, items[item_idx], registry)? {
                            Some(sub_bindings) => bindings.extend(sub_bindings),
                            None => return Ok(None),
                        }
                        item_idx += 1;
                    }
                }

                // Match after star (from end)
                let after_start = items.len() - n_after;
                for (i, elem) in elements[(star_pos + 1)..].iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        match vm_match_pattern(py, sub, items[after_start + i], registry)? {
                            Some(sub_bindings) => bindings.extend(sub_bindings),
                            None => return Ok(None),
                        }
                    }
                }

                // Bind star variable
                if let VMPatternElement::Star(slot) = &elements[star_pos] {
                    if *slot != usize::MAX {
                        let star_items: Vec<Py<PyAny>> =
                            items[n_before..after_start].iter().map(|v| v.to_pyobject(py)).collect();
                        let star_list = PyList::new(py, &star_items)?;
                        let star_val = Value::from_pyobject(py, &star_list.into_any())?;
                        bindings.push((*slot, star_val));
                    }
                }
            }

            Ok(Some(bindings))
        }
        VMPattern::Struct { name, field_slots } => {
            // Native struct path: direct field access via registry
            if let Some(idx) = value.as_struct_instance_idx() {
                let inst = registry.get_instance(idx).unwrap();
                let ty = registry.get_type(inst.type_id).unwrap();
                if ty.name != *name {
                    return Ok(None);
                }
                let mut bindings = Vec::new();
                for (field_name, slot) in field_slots {
                    match ty.field_index(field_name) {
                        Some(field_idx) => {
                            bindings.push((*slot, inst.fields[field_idx]));
                        }
                        None => return Ok(None),
                    }
                }
                return Ok(Some(bindings));
            }

            // Python path (PyObject structs)
            let py_val = value.to_pyobject(py);
            let py_bound = py_val.bind(py);

            // CatnipStructProxy: use type_name field, not Python class name
            if let Ok(proxy) = py_bound.cast::<crate::vm::structs::CatnipStructProxy>() {
                let p = proxy.borrow();
                if p.type_name != *name {
                    return Ok(None);
                }
                let mut bindings = Vec::new();
                for (field_name, slot) in field_slots {
                    match p.field_names.iter().position(|n| n == field_name.as_str()) {
                        Some(i) => {
                            let val = Value::from_pyobject(py, p.field_values[i].bind(py))?;
                            bindings.push((*slot, val));
                        }
                        None => return Ok(None),
                    }
                }
                return Ok(Some(bindings));
            }

            // Generic Python object path
            let value_type_name: String = py_bound.get_type().name()?.extract()?;
            if value_type_name != *name {
                return Ok(None);
            }

            let mut bindings = Vec::new();
            for (field_name, slot) in field_slots {
                let field_value = match py_bound.getattr(field_name.as_str()) {
                    Ok(v) => v,
                    Err(_) => return Ok(None),
                };
                let val = Value::from_pyobject(py, &field_value)?;
                bindings.push((*slot, val));
            }
            Ok(Some(bindings))
        }
        VMPattern::Enum {
            enum_name,
            variant_name,
        } => {
            // Resolve the expected symbol by looking up "EnumName.variant" in the SymbolTable
            let qname = qualified_name(enum_name, variant_name);
            if let Some(expected_sym) = resolve_symbol_by_name(&qname) {
                let expected = Value::from_symbol(expected_sym);
                if value.to_raw() == expected.to_raw() {
                    Ok(Some(Vec::new()))
                } else {
                    Ok(None)
                }
            } else {
                // Fallback: compare via Python
                let py_value = value.to_pyobject(py);
                let expected_str = qname.into_pyobject(py).unwrap().into_any();
                if py_value.bind(py).eq(&expected_str)? {
                    Ok(Some(Vec::new()))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

/// Strict variant for assignment unpacking patterns.
/// Unlike `vm_match_pattern`, mismatches are reported as concrete unpacking errors.
fn vm_match_assign_pattern(
    py: Python<'_>,
    pattern: &VMPattern,
    value: Value,
    registry: &StructRegistry,
) -> VMResult<Vec<(usize, Value)>> {
    match pattern {
        VMPattern::Var(slot) => Ok(vec![(*slot, value)]),
        VMPattern::Wildcard => Ok(Vec::new()),
        VMPattern::Tuple(elements) => {
            let py_val = value.to_pyobject(py);
            let py_bound = py_val.bind(py);
            let items: Vec<Value> = match py_bound.try_iter() {
                Ok(iter) => {
                    let mut v = Vec::new();
                    for item in iter {
                        v.push(Value::from_pyobject(py, &item.to_vm(py)?).to_vm(py)?);
                    }
                    v
                }
                Err(_) => {
                    let ty = py_bound
                        .get_type()
                        .name()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|_| "value".to_string());
                    return Err(VMError::TypeError(format!("Cannot unpack non-iterable {}", ty)));
                }
            };

            let mut star_idx: Option<usize> = None;
            let mut non_star_count = 0usize;
            for (i, elem) in elements.iter().enumerate() {
                match elem {
                    VMPatternElement::Star(_) => {
                        if star_idx.is_some() {
                            return Err(VMError::RuntimeError("Cannot unpack assignment pattern".to_string()));
                        }
                        star_idx = Some(i);
                    }
                    VMPatternElement::Pattern(_) => non_star_count += 1,
                }
            }

            let mut bindings = Vec::new();
            if let Some(star_pos) = star_idx {
                if items.len() < non_star_count {
                    return Err(VMError::RuntimeError(format!(
                        "Not enough values to unpack: expected at least {}, got {}",
                        non_star_count,
                        items.len()
                    )));
                }
                let n_before = star_pos;
                let n_after = elements.len() - star_pos - 1;
                let after_start = items.len() - n_after;

                for (i, elem) in elements[..star_pos].iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        let sub_bindings = vm_match_assign_pattern(py, sub, items[i], registry)?;
                        bindings.extend(sub_bindings);
                    }
                }

                for (i, elem) in elements[(star_pos + 1)..].iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        let sub_bindings = vm_match_assign_pattern(py, sub, items[after_start + i], registry)?;
                        bindings.extend(sub_bindings);
                    }
                }

                if let VMPatternElement::Star(slot) = elements[star_pos] {
                    if slot != usize::MAX {
                        let star_items: Vec<Py<PyAny>> =
                            items[n_before..after_start].iter().map(|v| v.to_pyobject(py)).collect();
                        let star_list = PyList::new(py, &star_items).to_vm(py)?;
                        let star_val = Value::from_pyobject(py, &star_list.into_any()).to_vm(py)?;
                        bindings.push((slot, star_val));
                    }
                }
                Ok(bindings)
            } else {
                if items.len() != non_star_count {
                    return Err(VMError::RuntimeError(format!(
                        "Cannot unpack {} values into {} variables",
                        items.len(),
                        non_star_count
                    )));
                }
                for (i, elem) in elements.iter().enumerate() {
                    if let VMPatternElement::Pattern(sub) = elem {
                        let sub_bindings = vm_match_assign_pattern(py, sub, items[i], registry)?;
                        bindings.extend(sub_bindings);
                    }
                }
                Ok(bindings)
            }
        }
        VMPattern::Literal(_) | VMPattern::Or(_) | VMPattern::Struct { .. } | VMPattern::Enum { .. } => {
            // Assignment patterns compiled by VM should not produce these nodes.
            // Fallback to generic runtime mismatch.
            let _ = registry;
            Err(VMError::RuntimeError("Cannot unpack assignment pattern".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::opcode::Instruction;

    #[test]
    fn test_simple_add() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test");
            code.constants = vec![Value::from_int(2), Value::from_int(3)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::Add),
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(5));
        });
    }

    #[test]
    fn test_comparison() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test");
            code.constants = vec![Value::from_int(5), Value::from_int(3)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::Gt),
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_bool(), Some(true));
        });
    }

    #[test]
    fn test_jump() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test");
            code.constants = vec![Value::from_int(10), Value::from_int(20)];
            code.instructions = vec![
                Instruction::new(OpCode::Jump, 2),      // Jump to LoadConst 20
                Instruction::new(OpCode::LoadConst, 0), // Skip this
                Instruction::new(OpCode::LoadConst, 1), // Load 20
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(20));
        });
    }

    #[test]
    fn test_arithmetic_sub() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_sub");
            code.constants = vec![Value::from_int(10), Value::from_int(3)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0), // 10
                Instruction::new(OpCode::LoadConst, 1), // 3
                Instruction::simple(OpCode::Sub),       // 10 - 3
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(7));
        });
    }

    #[test]
    fn test_arithmetic_mul() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_mul");
            code.constants = vec![Value::from_int(4), Value::from_int(5)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::Mul),
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(20));
        });
    }

    #[test]
    fn test_arithmetic_floordiv() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_floordiv");
            code.constants = vec![Value::from_int(23), Value::from_int(5)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::FloorDiv),
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(4));
        });
    }

    #[test]
    fn test_arithmetic_mod() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_mod");
            code.constants = vec![Value::from_int(23), Value::from_int(5)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::Mod),
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(3));
        });
    }

    #[test]
    fn test_stack_dup_top() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_dup");
            code.constants = vec![Value::from_int(42)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0), // Push 42
                Instruction::simple(OpCode::DupTop),    // Duplicate
                Instruction::simple(OpCode::Add),       // 42 + 42
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(84));
        });
    }

    #[test]
    fn test_stack_pop_top() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_pop");
            code.constants = vec![Value::from_int(10), Value::from_int(20)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0), // Push 10
                Instruction::new(OpCode::LoadConst, 1), // Push 20
                Instruction::simple(OpCode::PopTop),    // Pop 20
                Instruction::simple(OpCode::Halt),      // Return 10
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(10));
        });
    }

    #[test]
    fn test_locals_store_load() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_locals");
            code.nlocals = 2; // 2 local slots
            code.constants = vec![Value::from_int(100), Value::from_int(200)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),  // 100
                Instruction::new(OpCode::StoreLocal, 0), // local[0] = 100
                Instruction::new(OpCode::LoadConst, 1),  // 200
                Instruction::new(OpCode::StoreLocal, 1), // local[1] = 200
                Instruction::new(OpCode::LoadLocal, 0),  // Load local[0]
                Instruction::new(OpCode::LoadLocal, 1),  // Load local[1]
                Instruction::simple(OpCode::Add),        // 100 + 200
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(300));
        });
    }

    #[test]
    fn test_conditional_jump_taken() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_jump");
            code.constants = vec![
                Value::from_int(5),
                Value::from_int(3),
                Value::from_int(100), // Value if jump taken
                Value::from_int(999), // Value if jump not taken
            ];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),   // 5
                Instruction::new(OpCode::LoadConst, 1),   // 3
                Instruction::simple(OpCode::Gt),          // 5 > 3 = True
                Instruction::new(OpCode::JumpIfFalse, 6), // Skip if False
                Instruction::new(OpCode::LoadConst, 2),   // 100 (taken)
                Instruction::new(OpCode::Jump, 7),        // Skip else
                Instruction::new(OpCode::LoadConst, 3),   // 999 (not taken)
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(100)); // Jump was taken
        });
    }

    #[test]
    fn test_conditional_jump_not_taken() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_jump");
            code.constants = vec![
                Value::from_int(3),
                Value::from_int(5),
                Value::from_int(100),
                Value::from_int(999),
            ];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),   // 3
                Instruction::new(OpCode::LoadConst, 1),   // 5
                Instruction::simple(OpCode::Gt),          // 3 > 5 = False
                Instruction::new(OpCode::JumpIfFalse, 6), // Jump to else
                Instruction::new(OpCode::LoadConst, 2),   // 100 (not taken)
                Instruction::new(OpCode::Jump, 7),
                Instruction::new(OpCode::LoadConst, 3), // 999 (taken)
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(999)); // Else branch taken
        });
    }

    #[test]
    fn test_bitwise_and() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_band");
            code.constants = vec![Value::from_int(12), Value::from_int(10)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::BAnd), // 12 & 10 = 8
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(8));
        });
    }

    #[test]
    fn test_bitwise_or() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_bor");
            code.constants = vec![Value::from_int(12), Value::from_int(10)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::BOr), // 12 | 10 = 14
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(14));
        });
    }

    #[test]
    fn test_bitwise_xor() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_bxor");
            code.constants = vec![Value::from_int(12), Value::from_int(10)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::BXor), // 12 ^ 10 = 6
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(6));
        });
    }

    #[test]
    fn test_unary_neg() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_neg");
            code.constants = vec![Value::from_int(42)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::simple(OpCode::Neg),
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(-42));
        });
    }

    #[test]
    fn test_loop_simple() {
        // Simple counting loop: sum = 0; for i in 0..5 { sum += i }
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_loop");
            code.nlocals = 2; // i, sum
            code.constants = vec![
                Value::from_int(0), // Initial i
                Value::from_int(5), // Limit
                Value::from_int(1), // Increment
            ];
            code.instructions = vec![
                // sum = 0
                Instruction::new(OpCode::LoadConst, 0),  // 0
                Instruction::new(OpCode::StoreLocal, 1), // 1
                // i = 0
                Instruction::new(OpCode::LoadConst, 0),  // 2
                Instruction::new(OpCode::StoreLocal, 0), // 3
                // loop start (ip=4)
                Instruction::new(OpCode::LoadLocal, 0),    // 4: i
                Instruction::new(OpCode::LoadConst, 1),    // 5: limit
                Instruction::simple(OpCode::Lt),           // 6: i < 5
                Instruction::new(OpCode::JumpIfFalse, 17), // 7: Exit to ip=17 if false
                // sum = sum + i
                Instruction::new(OpCode::LoadLocal, 1),  // 8: sum
                Instruction::new(OpCode::LoadLocal, 0),  // 9: i
                Instruction::simple(OpCode::Add),        // 10: sum + i
                Instruction::new(OpCode::StoreLocal, 1), // 11: Store sum
                // i = i + 1
                Instruction::new(OpCode::LoadLocal, 0),  // 12: i
                Instruction::new(OpCode::LoadConst, 2),  // 13: 1
                Instruction::simple(OpCode::Add),        // 14: i + 1
                Instruction::new(OpCode::StoreLocal, 0), // 15: Store i
                Instruction::new(OpCode::Jump, 4),       // 16: Loop back to ip=4
                // exit (ip=17)
                Instruction::new(OpCode::LoadLocal, 1), // 17: Return sum
                Instruction::simple(OpCode::Halt),      // 18: Halt
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(10)); // 0+1+2+3+4
        });
    }

    #[test]
    fn test_float_arithmetic() {
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_float");
            code.constants = vec![Value::from_float(1.5), Value::from_float(2.5)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::Add), // 1.5 + 2.5
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_float(), Some(4.0));
        });
    }

    #[test]
    fn test_comparison_chains() {
        // Test multiple comparisons
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_cmp");
            code.constants = vec![Value::from_int(5), Value::from_int(5)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::Eq), // 5 == 5
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_bool(), Some(true));
        });
    }

    #[test]
    fn test_nested_arithmetic() {
        // (2 + 3) * 4 = 20
        Python::initialize();
        Python::attach(|py| {
            let mut code = CodeObject::new("test_nested");
            code.constants = vec![Value::from_int(2), Value::from_int(3), Value::from_int(4)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0), // 2
                Instruction::new(OpCode::LoadConst, 1), // 3
                Instruction::simple(OpCode::Add),       // 5
                Instruction::new(OpCode::LoadConst, 2), // 4
                Instruction::simple(OpCode::Mul),       // 20
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(20));
        });
    }

    // --- Native struct tests ---

    /// Helper: register a struct type and map a Python object's pointer to it.
    /// Returns (py_obj_value, type_id) where py_obj_value can be used as the callable.
    /// Install struct registry thread-local for a test VM.
    fn install_test_tables(vm: &mut VM) {
        crate::vm::value::set_struct_registry(&vm.struct_registry as *const _);
        crate::vm::value::set_func_table(&vm.func_table as *const _);
    }

    fn register_test_struct(
        py: Python<'_>,
        vm: &mut VM,
        name: &str,
        fields: Vec<StructField>,
    ) -> (Value, StructTypeId) {
        install_test_tables(vm);
        let type_id = vm.struct_registry.register_type(
            name.into(),
            fields,
            IndexMap::new(),
            vec![],                 // implements
            vec![name.to_string()], // mro
        );
        // Use a simple Python object as stand-in for the dataclass
        let marker = py.eval(c"type('_Marker', (), {})", None, None).unwrap();
        let ptr = marker.as_ptr() as usize;
        vm.struct_type_map.insert(ptr, type_id);
        let val = Value::from_pyobject(py, &marker).unwrap();
        (val, type_id)
    }

    #[test]
    fn test_native_struct_call() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let (struct_val, type_id) = register_test_struct(
                py,
                &mut vm,
                "Point",
                vec![
                    StructField {
                        name: "x".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "y".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
            );

            // Bytecode: load struct type, load args, call
            let mut code = CodeObject::new("test_struct_call");
            code.constants = vec![struct_val, Value::from_int(10), Value::from_int(20)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0), // struct type
                Instruction::new(OpCode::LoadConst, 1), // x=10
                Instruction::new(OpCode::LoadConst, 2), // y=20
                Instruction::new(OpCode::Call, 2),      // Point(10, 20)
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert!(result.is_struct_instance());
            let idx = result.as_struct_instance_idx().unwrap();
            let inst = vm.struct_registry.get_instance(idx).unwrap();
            assert_eq!(inst.type_id, type_id);
            assert_eq!(inst.fields[0].as_int(), Some(10));
            assert_eq!(inst.fields[1].as_int(), Some(20));
        });
    }

    #[test]
    fn test_native_struct_call_with_defaults() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let (struct_val, _type_id) = register_test_struct(
                py,
                &mut vm,
                "Config",
                vec![
                    StructField {
                        name: "name".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "debug".into(),
                        has_default: true,
                        default: Value::FALSE,
                    },
                    StructField {
                        name: "level".into(),
                        has_default: true,
                        default: Value::from_int(1),
                    },
                ],
            );

            // Call with only required arg: Config("test")
            let mut code = CodeObject::new("test_struct_defaults");
            code.constants = vec![
                struct_val,
                Value::from_pyobject(py, &"test".into_pyobject(py).unwrap()).unwrap(),
            ];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0), // struct type
                Instruction::new(OpCode::LoadConst, 1), // name="test"
                Instruction::new(OpCode::Call, 1),      // Config("test")
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert!(result.is_struct_instance());
            let idx = result.as_struct_instance_idx().unwrap();
            let inst = vm.struct_registry.get_instance(idx).unwrap();
            assert_eq!(inst.fields[1].as_bool(), Some(false)); // debug default
            assert_eq!(inst.fields[2].as_int(), Some(1)); // level default
        });
    }

    #[test]
    fn test_native_struct_call_too_few_args() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let (struct_val, _) = register_test_struct(
                py,
                &mut vm,
                "Point",
                vec![
                    StructField {
                        name: "x".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "y".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
            );

            let mut code = CodeObject::new("test_too_few");
            code.constants = vec![struct_val, Value::from_int(10)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::new(OpCode::Call, 1), // Only 1 arg for 2 required
                Instruction::simple(OpCode::Halt),
            ];

            let err = vm.execute(py, Arc::new(code), &[]).unwrap_err();
            match err {
                VMError::TypeError(msg) => assert!(msg.contains("missing"), "got: {msg}"),
                other => panic!("expected TypeError, got {other:?}"),
            }
        });
    }

    #[test]
    fn test_native_struct_call_too_many_args() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let (struct_val, _) = register_test_struct(
                py,
                &mut vm,
                "Pair",
                vec![
                    StructField {
                        name: "a".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "b".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
            );

            let mut code = CodeObject::new("test_too_many");
            code.constants = vec![struct_val, Value::from_int(1), Value::from_int(2), Value::from_int(3)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::new(OpCode::LoadConst, 2),
                Instruction::new(OpCode::LoadConst, 3),
                Instruction::new(OpCode::Call, 3), // 3 args for 2 fields
                Instruction::simple(OpCode::Halt),
            ];

            let err = vm.execute(py, Arc::new(code), &[]).unwrap_err();
            match err {
                VMError::TypeError(msg) => assert!(msg.contains("takes"), "got: {msg}"),
                other => panic!("expected TypeError, got {other:?}"),
            }
        });
    }

    #[test]
    fn test_native_struct_callkw() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let (struct_val, type_id) = register_test_struct(
                py,
                &mut vm,
                "Point",
                vec![
                    StructField {
                        name: "x".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "y".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
            );

            // CallKw encoding: (nargs << 8) | nkw
            // Point(10, y=20) -> nargs=1, nkw=1
            let kw_names = PyTuple::new(py, ["y"]).unwrap();
            let kw_names_val = Value::from_pyobject(py, kw_names.as_any()).unwrap();

            let mut code = CodeObject::new("test_struct_callkw");
            code.constants = vec![struct_val, Value::from_int(10), Value::from_int(20), kw_names_val];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),         // struct type
                Instruction::new(OpCode::LoadConst, 1),         // x=10 (positional)
                Instruction::new(OpCode::LoadConst, 2),         // y=20 (kw value)
                Instruction::new(OpCode::LoadConst, 3),         // kw_names ("y",)
                Instruction::new(OpCode::CallKw, (1 << 8) | 1), // nargs=1, nkw=1
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert!(result.is_struct_instance());
            let idx = result.as_struct_instance_idx().unwrap();
            let inst = vm.struct_registry.get_instance(idx).unwrap();
            assert_eq!(inst.type_id, type_id);
            assert_eq!(inst.fields[0].as_int(), Some(10));
            assert_eq!(inst.fields[1].as_int(), Some(20));
        });
    }

    /// Helper: create a Point(x, y) instance and return (vm, instance_value).
    fn make_point_instance(py: Python<'_>, x: i64, y: i64) -> (VM, Value) {
        let mut vm = VM::new();

        let (struct_val, _) = register_test_struct(
            py,
            &mut vm,
            "Point",
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
        );

        let mut code = CodeObject::new("make_point");
        code.constants = vec![struct_val, Value::from_int(x), Value::from_int(y)];
        code.instructions = vec![
            Instruction::new(OpCode::LoadConst, 0),
            Instruction::new(OpCode::LoadConst, 1),
            Instruction::new(OpCode::LoadConst, 2),
            Instruction::new(OpCode::Call, 2),
            Instruction::simple(OpCode::Halt),
        ];

        let result = vm.execute(py, Arc::new(code), &[]).unwrap();
        (vm, result)
    }

    #[test]
    fn test_native_struct_getattr() {
        Python::initialize();
        Python::attach(|py| {
            let (mut vm, instance) = make_point_instance(py, 10, 20);

            // GetAttr for "x" (names[0]) then "y" (names[1])
            let mut code = CodeObject::new("test_getattr");
            code.constants = vec![instance];
            code.names = vec!["x".into(), "y".into()];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::GetAttr, 0), // .x
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(10));

            let mut code2 = CodeObject::new("test_getattr_y");
            code2.constants = vec![instance];
            code2.names = vec!["y".into()];
            code2.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::GetAttr, 0), // .y
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code2), &[]).unwrap();
            assert_eq!(result.as_int(), Some(20));
        });
    }

    #[test]
    fn test_native_struct_getattr_unknown_field() {
        Python::initialize();
        Python::attach(|py| {
            let (mut vm, instance) = make_point_instance(py, 10, 20);

            let mut code = CodeObject::new("test_getattr_bad");
            code.constants = vec![instance];
            code.names = vec!["z".into()];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::GetAttr, 0), // .z (doesn't exist)
                Instruction::simple(OpCode::Halt),
            ];

            let err = vm.execute(py, Arc::new(code), &[]).unwrap_err();
            match err {
                VMError::RuntimeError(msg) => {
                    assert!(msg.contains("no attribute"), "got: {msg}");
                    assert!(msg.contains("'z'"), "got: {msg}");
                }
                other => panic!("expected RuntimeError, got {other:?}"),
            }
        });
    }

    #[test]
    fn test_native_struct_setattr() {
        Python::initialize();
        Python::attach(|py| {
            let (mut vm, instance) = make_point_instance(py, 10, 20);

            // SetAttr: set x = 42, then GetAttr to verify
            let mut code = CodeObject::new("test_setattr");
            code.constants = vec![instance, Value::from_int(42)];
            code.names = vec!["x".into()];
            code.instructions = vec![
                // SetAttr: pop value, pop obj -> obj.x = 42
                Instruction::new(OpCode::LoadConst, 0), // obj
                Instruction::new(OpCode::LoadConst, 1), // 42
                Instruction::new(OpCode::SetAttr, 0),   // obj.x = 42
                // GetAttr: verify
                Instruction::new(OpCode::LoadConst, 0), // obj
                Instruction::new(OpCode::GetAttr, 0),   // obj.x
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(42));
        });
    }

    #[test]
    fn test_native_struct_eq_same() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let (struct_val, _) = register_test_struct(
                py,
                &mut vm,
                "Point",
                vec![
                    StructField {
                        name: "x".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "y".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
            );

            // Create two identical instances and compare
            let mut code = CodeObject::new("test_eq_same");
            code.constants = vec![struct_val, Value::from_int(10), Value::from_int(20)];
            code.instructions = vec![
                // Point(10, 20)
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::new(OpCode::LoadConst, 2),
                Instruction::new(OpCode::Call, 2),
                // Point(10, 20)
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::new(OpCode::LoadConst, 2),
                Instruction::new(OpCode::Call, 2),
                // ==
                Instruction::simple(OpCode::Eq),
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_bool(), Some(true));
        });
    }

    #[test]
    fn test_native_struct_eq_different_values() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let (struct_val, _) = register_test_struct(
                py,
                &mut vm,
                "Point",
                vec![
                    StructField {
                        name: "x".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "y".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
            );

            // Point(10, 20) == Point(10, 99) -> false
            let mut code = CodeObject::new("test_eq_diff_vals");
            code.constants = vec![
                struct_val,
                Value::from_int(10),
                Value::from_int(20),
                Value::from_int(99),
            ];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::new(OpCode::LoadConst, 2),
                Instruction::new(OpCode::Call, 2),
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::new(OpCode::LoadConst, 3), // y=99
                Instruction::new(OpCode::Call, 2),
                Instruction::simple(OpCode::Eq),
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_bool(), Some(false));
        });
    }

    #[test]
    fn test_native_struct_eq_different_types() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let (point_val, _) = register_test_struct(
                py,
                &mut vm,
                "Point",
                vec![
                    StructField {
                        name: "x".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "y".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
            );

            let (vec_val, _) = register_test_struct(
                py,
                &mut vm,
                "Vec2",
                vec![
                    StructField {
                        name: "x".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "y".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
            );

            // Point(10, 20) == Vec2(10, 20) -> false (different types)
            let mut code = CodeObject::new("test_eq_diff_types");
            code.constants = vec![point_val, vec_val, Value::from_int(10), Value::from_int(20)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0), // Point
                Instruction::new(OpCode::LoadConst, 2),
                Instruction::new(OpCode::LoadConst, 3),
                Instruction::new(OpCode::Call, 2),
                Instruction::new(OpCode::LoadConst, 1), // Vec2
                Instruction::new(OpCode::LoadConst, 2),
                Instruction::new(OpCode::LoadConst, 3),
                Instruction::new(OpCode::Call, 2),
                Instruction::simple(OpCode::Eq),
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_bool(), Some(false));
        });
    }

    #[test]
    fn test_native_struct_pattern_match() {
        // Point(10, 20) matches Point{x, y} -> bindings x=10, y=20
        Python::initialize();
        Python::attach(|py| {
            let (mut vm, instance) = make_point_instance(py, 10, 20);

            let mut code = CodeObject::new("test_pattern_match");
            code.constants = vec![instance];
            // slots 0=x, 1=y
            code.patterns = vec![VMPattern::Struct {
                name: "Point".into(),
                field_slots: vec![("x".into(), 0), ("y".into(), 1)],
            }];
            code.nlocals = 2;
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),      // push instance
                Instruction::new(OpCode::MatchPatternVM, 0), // match against pattern 0
                Instruction::simple(OpCode::BindMatch),      // bind x=slot0, y=slot1
                Instruction::new(OpCode::LoadLocal, 0),      // push x
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_int(), Some(10));

            // Also check y
            let mut code2 = CodeObject::new("test_pattern_match_y");
            code2.constants = vec![instance];
            code2.patterns = vec![VMPattern::Struct {
                name: "Point".into(),
                field_slots: vec![("x".into(), 0), ("y".into(), 1)],
            }];
            code2.nlocals = 2;
            code2.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::MatchPatternVM, 0),
                Instruction::simple(OpCode::BindMatch),
                Instruction::new(OpCode::LoadLocal, 1), // push y
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code2), &[]).unwrap();
            assert_eq!(result.as_int(), Some(20));
        });
    }

    #[test]
    fn test_native_struct_pattern_mismatch_type() {
        // Point(10, 20) does NOT match Vec2{x, y}
        Python::initialize();
        Python::attach(|py| {
            let (mut vm, instance) = make_point_instance(py, 10, 20);

            let mut code = CodeObject::new("test_pattern_mismatch");
            code.constants = vec![instance];
            code.patterns = vec![VMPattern::Struct {
                name: "Vec2".into(),
                field_slots: vec![("x".into(), 0), ("y".into(), 1)],
            }];
            code.nlocals = 2;
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::MatchPatternVM, 0),
                Instruction::simple(OpCode::Halt), // result is on stack: TRUE or NIL
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert!(result.is_nil(), "expected NIL for type mismatch, got {:?}", result);
        });
    }

    #[test]
    fn test_native_struct_pattern_unknown_field() {
        // Point(10, 20) with pattern Point{x, z} -> no match (z doesn't exist)
        Python::initialize();
        Python::attach(|py| {
            let (mut vm, instance) = make_point_instance(py, 10, 20);

            let mut code = CodeObject::new("test_pattern_unknown_field");
            code.constants = vec![instance];
            code.patterns = vec![VMPattern::Struct {
                name: "Point".into(),
                field_slots: vec![("x".into(), 0), ("z".into(), 1)],
            }];
            code.nlocals = 2;
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::MatchPatternVM, 0),
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert!(result.is_nil(), "expected NIL for unknown field, got {:?}", result);
        });
    }

    #[test]
    fn test_bigint_eq_no_python_fallback() {
        Python::initialize();
        Python::attach(|py| {
            reset_vm_fallback_stats();

            let n = Integer::from(i64::MAX) * Integer::from(1000_u32);
            let mut code = CodeObject::new("test_bigint_eq_no_fallback");
            code.constants = vec![Value::from_bigint(n.clone()), Value::from_bigint(n)];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::simple(OpCode::Eq),
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_bool(), Some(true));

            let stats = get_vm_fallback_stats();
            assert_eq!(stats.py_compare_eq, 0);
        });
    }

    #[test]
    fn test_match_pattern_literal_bigint_no_python_fallback() {
        Python::initialize();
        Python::attach(|py| {
            reset_vm_fallback_stats();

            let n = Integer::from(i64::MAX) * Integer::from(2000_u32);
            let mut code = CodeObject::new("test_match_bigint_literal");
            code.constants = vec![Value::from_bigint(n.clone())];
            code.patterns = vec![VMPattern::Literal(Value::from_bigint(n))];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::MatchPatternVM, 0),
                Instruction::simple(OpCode::Halt),
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, Arc::new(code), &[]).unwrap();
            assert_eq!(result.as_bool(), Some(true));

            let stats = get_vm_fallback_stats();
            assert_eq!(stats.py_pattern_literal_eq, 0);
        });
    }

    #[test]
    fn test_struct_instance_to_pyobject() {
        Python::initialize();
        Python::attach(|py| {
            let mut vm = VM::new();

            let type_id = vm.struct_registry.register_type(
                "Point".into(),
                vec![
                    StructField {
                        name: "x".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                    StructField {
                        name: "y".into(),
                        has_default: false,
                        default: Value::NIL,
                    },
                ],
                IndexMap::new(),
                vec![],               // implements
                vec!["Point".into()], // mro
            );

            // Create a native instance
            let idx = vm
                .struct_registry
                .create_instance(type_id, vec![Value::from_int(10), Value::from_int(20)]);
            let struct_val = Value::from_struct_instance(idx);

            // Install registries for to_pyobject
            crate::vm::value::set_struct_registry(&vm.struct_registry as *const _);
            crate::vm::value::set_func_table(&vm.func_table as *const _);

            let py_obj = struct_val.to_pyobject(py);
            let py_obj_bound = py_obj.bind(py);

            // Check it's a CatnipStructProxy with correct fields
            let x: i64 = py_obj_bound.getattr("x").unwrap().extract().unwrap();
            let y: i64 = py_obj_bound.getattr("y").unwrap().extract().unwrap();
            assert_eq!(x, 10);
            assert_eq!(y, 20);

            // Check repr
            let repr: String = py_obj_bound.repr().unwrap().extract().unwrap();
            assert!(repr.contains("Point"));
            assert!(repr.contains("x=10"));
            assert!(repr.contains("y=20"));

            crate::vm::value::clear_struct_registry();
            crate::vm::value::clear_symbol_table();
        });
    }

    // --- SmallInt overflow tests (regression: wrapping_add masked i64 overflow) ---

    use catnip_core::nanbox::{SMALLINT_MAX as SMAX, SMALLINT_MIN as SMIN};

    #[test]
    fn test_add_smallint_overflow_to_bigint() {
        let result = binary_add(Value::from_int(SMAX), Value::from_int(1)).unwrap();
        assert!(result.is_bigint(), "SMAX + 1 must promote to BigInt");
        let expected = Integer::from(SMAX) + Integer::from(1);
        assert_eq!(unsafe { result.as_bigint_ref().unwrap() }, &expected);
        result.decref();
    }

    #[test]
    fn test_add_i64_overflow_to_bigint() {
        // Sum overflows i64 -- checked_add returns None
        let result = binary_add(Value::from_int(SMAX), Value::from_int(SMAX)).unwrap();
        assert!(result.is_bigint(), "SMAX + SMAX must promote to BigInt");
        let expected = Integer::from(SMAX) + Integer::from(SMAX);
        assert_eq!(unsafe { result.as_bigint_ref().unwrap() }, &expected);
        result.decref();
    }

    #[test]
    fn test_sub_smallint_overflow_to_bigint() {
        let result = binary_sub(Value::from_int(SMIN), Value::from_int(1)).unwrap();
        assert!(result.is_bigint(), "SMIN - 1 must promote to BigInt");
        result.decref();
    }

    #[test]
    fn test_mul_smallint_overflow_to_bigint() {
        let result = binary_mul(Value::from_int(SMAX), Value::from_int(2)).unwrap();
        assert!(result.is_bigint(), "SMAX * 2 must promote to BigInt");
        let expected = Integer::from(SMAX) * Integer::from(2);
        assert_eq!(unsafe { result.as_bigint_ref().unwrap() }, &expected);
        result.decref();
    }

    #[test]
    fn test_mul_i64_overflow_to_bigint() {
        // checked_mul returns None
        let result = binary_mul(Value::from_int(SMAX), Value::from_int(SMAX)).unwrap();
        assert!(result.is_bigint(), "SMAX^2 must promote to BigInt");
        let expected = Integer::from((1_i64 << 46) - 1) * Integer::from((1_i64 << 46) - 1);
        assert_eq!(unsafe { result.as_bigint_ref().unwrap() }, &expected);
        result.decref();
    }
}
