// FILE: catnip_rs/src/vm/core.rs
//! Catnip Virtual Machine with O(1) dispatch via Rust match.
//!
//! Stack-based VM that executes bytecode without growing the Python stack.

use super::frame::{CodeObject, Frame, FramePool, PyCodeObject, RustClosureScope, RustVMFunction};
use super::iter::SeqIter;
use super::pattern::{VMPattern, VMPatternElement};
use super::py_interop::convert_code_object;
use super::structs::{StructField, StructRegistry, StructTypeId};
use super::traits::{TraitDef, TraitField, TraitRegistry};
use super::value::Value;
use super::OpCode;
use crate::constants::{JIT_PURE_BUILTINS, JIT_THRESHOLD_DEFAULT};
use crate::jit::builtin_dispatch::builtin_name_to_id;
use crate::jit::{HotLoopDetector, JITExecutor, TraceOp, TraceRecorder};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// VM execution error
#[derive(Debug)]
pub enum VMError {
    StackUnderflow,
    FrameOverflow,
    NameError(String),
    TypeError(String),
    RuntimeError(String),
    ZeroDivisionError(String),
    Return(Value),
    Break,
    Continue,
}

impl std::fmt::Display for VMError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VMError::StackUnderflow => write!(f, "stack underflow"),
            VMError::FrameOverflow => write!(f, "frame stack overflow"),
            VMError::NameError(s) => write!(f, "name '{}' is not defined", s),
            VMError::TypeError(s) => write!(f, "type error: {}", s),
            VMError::RuntimeError(s) => write!(f, "runtime error: {}", s),
            VMError::ZeroDivisionError(s) => write!(f, "{}", s),
            VMError::Return(_) => write!(f, "return signal"),
            VMError::Break => write!(f, "break signal"),
            VMError::Continue => write!(f, "continue signal"),
        }
    }
}

impl std::error::Error for VMError {}

impl From<VMError> for PyErr {
    fn from(err: VMError) -> PyErr {
        match err {
            VMError::NameError(s) => pyo3::exceptions::PyNameError::new_err(s),
            VMError::TypeError(s) => pyo3::exceptions::PyTypeError::new_err(s),
            VMError::ZeroDivisionError(s) => pyo3::exceptions::PyZeroDivisionError::new_err(s),
            _ => pyo3::exceptions::PyRuntimeError::new_err(err.to_string()),
        }
    }
}

impl From<PyErr> for VMError {
    fn from(err: PyErr) -> VMError {
        Python::attach(|py| {
            let py_err = &err;
            if py_err.is_instance_of::<pyo3::exceptions::PyTypeError>(py) {
                VMError::TypeError(err.to_string())
            } else if py_err.is_instance_of::<pyo3::exceptions::PyNameError>(py) {
                VMError::NameError(err.to_string())
            } else if py_err.is_instance_of::<pyo3::exceptions::PyZeroDivisionError>(py) {
                VMError::ZeroDivisionError(err.to_string())
            } else {
                VMError::RuntimeError(err.to_string())
            }
        })
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

/// Stack-based virtual machine for Catnip bytecode.
pub struct VM {
    /// Frame stack
    frame_stack: Vec<Frame>,
    /// Frame pool for reuse
    frame_pool: FramePool,
    /// Global variables (VM-owned)
    globals: HashMap<String, Value>,
    /// Python context for name resolution fallback
    py_context: Option<Py<PyAny>>,
    /// Cached iter() builtin for GetIter
    cached_iter_fn: Option<Py<PyAny>>,
    /// Cached next() builtin for ForIter
    cached_next_fn: Option<Py<PyAny>>,
    /// Cached operator module for binary ops fallback
    cached_operator: Option<Py<PyAny>>,
    /// Cached NDTopos singleton for NdEmptyTopos
    cached_nd_topos: Option<Py<PyAny>>,
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
    /// Native struct type and instance registry
    pub struct_registry: StructRegistry,
    /// PyObject ptr -> StructTypeId, populated by MakeStruct
    struct_type_map: HashMap<usize, StructTypeId>,
    /// Trait registry for trait composition
    pub trait_registry: TraitRegistry,
}

impl VM {
    /// Create a new VM.
    pub fn new() -> Self {
        const FRAME_STACK_CAPACITY: usize = 64;
        Self {
            frame_stack: Vec::with_capacity(FRAME_STACK_CAPACITY),
            frame_pool: FramePool::default(),
            globals: HashMap::new(),
            py_context: None,
            cached_iter_fn: None,
            cached_next_fn: None,
            cached_operator: None,
            cached_nd_topos: None,
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
            struct_registry: StructRegistry::new(),
            struct_type_map: HashMap::new(),
            trait_registry: TraitRegistry::new(),
        }
    }

    /// Enable JIT compilation with custom threshold.
    pub fn enable_jit_with_threshold(&mut self, threshold: u32) {
        self.jit_enabled = true;
        // Reset detector with new threshold
        self.jit_detector = HotLoopDetector::new(threshold);
        // Lazy init the JIT executor
        if self.jit.lock().unwrap().is_none() {
            *self.jit.lock().unwrap() = Some(JITExecutor::new(threshold));
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

    /// Set the Python context for name resolution.
    pub fn set_context(&mut self, context: Py<PyAny>) {
        self.py_context = Some(context);
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
        .map_err(VMError::from)?;

        let result = cb
            .call1(py, (start_byte, locals_dict, call_stack))
            .map_err(VMError::from)?;
        let action_int: i32 = result.extract(py).unwrap_or(1);
        Ok(DebugStepMode::from_i32(action_int))
    }

    /// Capture error context from current VM state.
    fn capture_error_context(&mut self, error: &VMError) {
        let (error_type, message) = match error {
            VMError::NameError(s) => ("NameError".to_string(), s.clone()),
            VMError::TypeError(s) => ("TypeError".to_string(), s.clone()),
            VMError::ZeroDivisionError(s) => ("ZeroDivisionError".to_string(), s.clone()),
            VMError::RuntimeError(s) => ("RuntimeError".to_string(), s.clone()),
            VMError::StackUnderflow => ("RuntimeError".to_string(), "stack underflow".to_string()),
            VMError::FrameOverflow => (
                "RuntimeError".to_string(),
                "frame stack overflow".to_string(),
            ),
            // Control flow signals - no error context needed
            VMError::Return(_) | VMError::Break | VMError::Continue => return,
        };

        // Get start_byte from current frame's line_table
        let start_byte = if let Some(frame) = self.frame_stack.last() {
            if let Some(ref code) = frame.code {
                let ip = if frame.ip > 0 { frame.ip - 1 } else { 0 };
                code.line_table.get(ip).copied().unwrap_or(0)
            } else {
                0
            }
        } else {
            0
        };

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
            self.cached_next_fn = Some(builtins.getattr("next")?.unbind());
            self.cached_operator = Some(py.import("operator")?.unbind().into());
        }
        Ok(())
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
        // Determine which type to look up parent_methods from
        let type_info = if let Some(ref source) = super_source_type {
            // Super chain: look up the source type and use ITS parent_methods
            self.struct_registry
                .find_type_by_name(source)
                .and_then(|ty| {
                    if ty.parent_methods.is_empty() {
                        None
                    } else {
                        Some((&ty.parent_methods, ty.parent_type_name.as_deref()))
                    }
                })
        } else {
            // Normal method call: use instance's type's parent_methods
            if let Some(idx) = inst_val.as_struct_instance_idx() {
                self.struct_registry.get_instance(idx).and_then(|inst| {
                    self.struct_registry.get_type(inst.type_id).and_then(|ty| {
                        if ty.parent_methods.is_empty() {
                            None
                        } else {
                            Some((&ty.parent_methods, ty.parent_type_name.as_deref()))
                        }
                    })
                })
            } else {
                // Fallback: CatnipStructProxy PyObject
                None
            }
        };

        // Fallback for non-native struct instances (CatnipStructProxy)
        let type_info = type_info.or_else(|| {
            if super_source_type.is_some() {
                return None; // Already checked via source type
            }
            let inst_py = inst_val.to_pyobject(py);
            let inst_bound = inst_py.bind(py);
            inst_bound
                .cast::<super::structs::CatnipStructProxy>()
                .ok()
                .and_then(|proxy| {
                    let proxy_ref = proxy.borrow();
                    self.struct_registry
                        .find_type_by_name(&proxy_ref.type_name)
                        .and_then(|ty| {
                            if ty.parent_methods.is_empty() {
                                None
                            } else {
                                Some((&ty.parent_methods, ty.parent_type_name.as_deref()))
                            }
                        })
                })
        });

        if let Some((pm, parent_name)) = type_info {
            let methods: HashMap<String, Py<PyAny>> = pm
                .iter()
                .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                .collect();
            let inst_py = inst_val.to_pyobject(py);
            let native_idx = inst_val.as_struct_instance_idx();
            let source_name = parent_name.unwrap_or("").to_string();
            let proxy = Py::new(
                py,
                super::structs::SuperProxy {
                    methods,
                    instance: inst_py,
                    source_type_name: source_name,
                    native_instance_idx: native_idx,
                },
            )
            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
            frame.super_proxy = Some(proxy.into_any());
        }
        Ok(())
    }

    /// Execute a code object and return the result.
    pub fn execute(&mut self, py: Python<'_>, code: CodeObject, args: &[Value]) -> VMResult<Value> {
        self.execute_with_closure(py, code, args, None)
    }

    /// Execute a code object with an optional closure scope.
    pub fn execute_with_closure(
        &mut self,
        py: Python<'_>,
        code: CodeObject,
        args: &[Value],
        closure_scope: Option<Py<PyAny>>,
    ) -> VMResult<Value> {
        // Set bytecode hash for JIT trace cache (instructions + constants + names)
        if self.jit_enabled {
            let mut bytes: Vec<u8> =
                Vec::with_capacity(code.instructions.len() * 5 + code.constants.len() * 8);
            for i in &code.instructions {
                bytes.push(i.op as u8);
                bytes.extend_from_slice(&i.arg.to_le_bytes());
            }
            for c in &code.constants {
                bytes.extend_from_slice(&c.to_raw().to_le_bytes());
            }
            for n in &code.names {
                bytes.extend_from_slice(n.as_bytes());
                bytes.push(0); // separator
            }
            let hash = crate::jit::hash_bytecode(&bytes);
            if let Ok(mut jit) = self.jit.lock() {
                if let Some(ref mut executor) = *jit {
                    executor.set_bytecode_hash(hash);
                }
            }
        }

        // Create initial frame
        let mut frame = Frame::with_code(code);
        frame.bind_args(py, args, None);
        frame.closure_scope = closure_scope;

        self.frame_stack.push(frame);

        // Clear previous error context
        self.last_error_context = None;
        self.call_stack.clear();

        // Install struct registry pointer for Value::to_pyobject
        super::value::set_struct_registry(&self.struct_registry as *const _);

        // Run dispatch loop
        let result = match self.run(py) {
            Ok(v) => v,
            Err(e) => {
                self.capture_error_context(&e);
                // Clean up frame stack
                while let Some(frame) = self.frame_stack.pop() {
                    self.frame_pool.free(frame);
                }
                return Err(e);
            }
        };

        // Clean up
        while let Some(frame) = self.frame_stack.pop() {
            self.frame_pool.free(frame);
        }

        Ok(result)
    }

    /// Get globals as a HashMap reference for syncing back to Python.
    pub fn get_globals(&self) -> &HashMap<String, Value> {
        &self.globals
    }

    /// Main dispatch loop.
    fn run(&mut self, py: Python<'_>) -> VMResult<Value> {
        // Cache builtins once at start of execution
        self.ensure_builtins_cached(py)?;

        // Clone refs for use inside the loop (avoids borrow issues)
        let iter_fn = self
            .cached_iter_fn
            .as_ref()
            .expect("iter_fn should be cached")
            .clone_ref(py);
        let next_fn = self
            .cached_next_fn
            .as_ref()
            .expect("next_fn should be cached")
            .clone_ref(py);
        let operator = self
            .cached_operator
            .as_ref()
            .expect("operator should be cached")
            .clone_ref(py);
        let ctx_globals: Option<Py<PyDict>> = self.py_context.as_ref().and_then(|ctx| {
            ctx.bind(py)
                .getattr("globals")
                .ok()
                .and_then(|g| match g.cast::<PyDict>() {
                    Ok(d) => Some(d.clone().unbind()),
                    Err(_) => None,
                })
        });

        let mut last_result = Value::NIL;

        while let Some(frame) = self.frame_stack.last_mut() {
            let code = match &frame.code {
                Some(c) => c,
                None => {
                    self.frame_stack.pop();
                    continue;
                }
            };

            // Check if we've reached the end of bytecode
            if frame.ip >= code.instructions.len() {
                last_result = frame.pop();
                let discard = frame.discard_return;
                self.frame_stack.pop();
                if !discard {
                    if let Some(caller) = self.frame_stack.last_mut() {
                        caller.push(last_result);
                    }
                }
                continue;
            }

            // Fetch instruction
            let instr = code.instructions[frame.ip];
            // Capture source position for error reporting (copy, no borrow)
            let _current_src_byte = code.line_table.get(frame.ip).copied().unwrap_or(0);
            frame.ip += 1;

            if self.trace {
                eprintln!("[TRACE] {:?} arg={}", instr.op, instr.arg);
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
                self.jit_recorder
                    .record_opcode(instr.op, instr.arg, is_int_value, ip);
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
                            self.jit_recorder
                                .record_const_int(if b { 1 } else { 0 }, ip);
                        } else {
                            // Other constants (None, strings, etc.) - record as 0 to balance stack
                            // These will likely prevent compilation (fallback to interpreter)
                            self.jit_recorder.record_const_int(0, ip);
                        }
                    }
                    frame.push(value);
                }

                OpCode::LoadLocal => {
                    let value = frame.get_local(instr.arg as usize);
                    frame.push(value);
                }

                OpCode::StoreLocal => {
                    let value = frame.pop();
                    frame.set_local(instr.arg as usize, value);
                }

                OpCode::LoadScope => {
                    let name = code.names[instr.arg as usize].clone();
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

                    // 1. Check closure_scope first (captured variables)
                    if let Some(ref closure) = frame.closure_scope {
                        let closure_bound = closure.bind(py);
                        if let Ok(val) = closure_bound.call_method1("_resolve", (name.as_str(),)) {
                            let value = Value::from_pyobject(py, &val)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            resolved_value = value;
                            frame.push(resolved_value);

                            // Record LoadScope during tracing
                            if self.jit_tracing {
                                if let Some(int_val) = resolved_value.as_int() {
                                    let ip = frame.ip - 1;
                                    self.jit_recorder.record_load_scope(&name, int_val, ip);
                                }
                                // Non-int LoadScope will abort trace (fallback to interpreter)
                            }
                            continue;
                        }
                        // _resolve raised NameError, fall through to globals
                    }
                    // 2. Check context.globals first (source of truth, can be mutated by closures)
                    if let Some(ref py_globals) = ctx_globals {
                        if let Ok(Some(val)) = py_globals.bind(py).get_item(&name) {
                            let value = Value::from_pyobject(py, &val)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            resolved_value = value;
                            frame.push(resolved_value);

                            // Record LoadScope during tracing
                            if self.jit_tracing {
                                if let Some(int_val) = resolved_value.as_int() {
                                    let ip = frame.ip - 1;
                                    self.jit_recorder.record_load_scope(&name, int_val, ip);
                                }
                            }
                            continue;
                        }
                    }
                    // 3. Check VM globals (fallback when no context)
                    if let Some(&value) = self.globals.get(&name) {
                        resolved_value = value;
                        frame.push(resolved_value);

                        // Record LoadScope during tracing
                        if self.jit_tracing {
                            if let Some(int_val) = resolved_value.as_int() {
                                let ip = frame.ip - 1;
                                self.jit_recorder.record_load_scope(&name, int_val, ip);
                            }
                        }
                    } else {
                        return Err(VMError::NameError(name.clone()));
                    }
                }

                OpCode::StoreScope => {
                    let name = code.names[instr.arg as usize].clone();

                    // Check slotmap before recording
                    let slot_idx = code.slotmap.get(&name).copied();

                    // Record StoreScope during tracing (BEFORE pop, while value is on stack)
                    // Pass the existing slot from slotmap if available
                    let trace_slot = if self.jit_tracing {
                        let ip = frame.ip - 1;
                        self.jit_recorder.record_store_scope(&name, ip, slot_idx)
                    } else {
                        None
                    };

                    let value = frame.pop();

                    // During tracing, also store to the trace slot to keep frame.locals synchronized
                    if let Some(slot) = trace_slot {
                        // Extend locals array if necessary
                        if slot >= frame.locals.len() {
                            frame.locals.resize(slot + 1, Value::NIL);
                        }
                        frame.set_local(slot, value);
                    } else if self.jit_enabled {
                        // When JIT is enabled (but not currently tracing), still sync frame.locals
                        // using the slotmap so that JIT code can read correct values
                        if let Some(slot) = slot_idx {
                            if slot >= frame.locals.len() {
                                frame.locals.resize(slot + 1, Value::NIL);
                            }
                            frame.set_local(slot, value);
                        }
                    }

                    // 1. Try to update closure_scope first (for mutable closures)
                    let mut stored_in_closure = false;
                    if let Some(ref closure) = frame.closure_scope {
                        let closure_bound = closure.bind(py);
                        // Check if variable exists in closure (via _resolve)
                        if closure_bound
                            .call_method1("_resolve", (name.as_str(),))
                            .is_ok()
                        {
                            // Variable exists in closure, update it via _set
                            let py_value = value.to_pyobject(py);
                            let _ = closure_bound.call_method1("_set", (name.as_str(), py_value));
                            stored_in_closure = true;
                        }
                    }

                    // 2. Store to local slot if name is in slotmap
                    if let Some(idx) = slot_idx {
                        frame.set_local(idx, value);
                    }

                    // 3. Store to globals for name resolution (if not in closure)
                    if !stored_in_closure {
                        self.globals.insert(name.clone(), value);
                        // Also sync to Python context.globals immediately
                        // so closures created later can access these values
                        if let Some(ref py_globals) = ctx_globals {
                            let py_value = value.to_pyobject(py);
                            let _ = py_globals.bind(py).set_item(name.as_str(), py_value);
                        }
                    }
                }

                OpCode::LoadGlobal => {
                    let name = code.names[instr.arg as usize].clone();
                    if let Some(&value) = self.globals.get(&name) {
                        frame.push(value);
                    } else if let Some(ref py_globals) = ctx_globals {
                        // Try to get from Python context.globals
                        if let Ok(Some(val)) = py_globals.bind(py).get_item(&*name) {
                            let value = Value::from_pyobject(py, &val)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            frame.push(value);
                        } else {
                            return Err(VMError::NameError(name));
                        }
                    } else {
                        return Err(VMError::NameError(name));
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
                    frame.pop();
                }

                OpCode::DupTop => {
                    let value = frame.peek();
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
                            // Fallback to Python for strings, lists, etc.
                            let py_a = a.to_pyobject(py);
                            let py_b = b.to_pyobject(py);
                            let py_result =
                                operator.bind(py).call_method1("add", (&py_a, &py_b))?;
                            Value::from_pyobject(py, &py_result)?
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
                            let py_a = a.to_pyobject(py);
                            let py_b = b.to_pyobject(py);
                            let py_result =
                                operator.bind(py).call_method1("sub", (&py_a, &py_b))?;
                            Value::from_pyobject(py, &py_result)?
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
                            // Fallback to Python for string * int, etc.
                            let py_a = a.to_pyobject(py);
                            let py_b = b.to_pyobject(py);
                            let py_result =
                                operator.bind(py).call_method1("mul", (&py_a, &py_b))?;
                            Value::from_pyobject(py, &py_result)?
                        }
                        Err(e) => return Err(e),
                    };
                    frame.push(result);
                }

                OpCode::Div => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = binary_div(py, a, b)?;
                    frame.push(result);
                }

                OpCode::FloorDiv => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = binary_floordiv(py, a, b)?;
                    frame.push(result);
                }

                OpCode::Mod => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = binary_mod(py, a, b)?;
                    frame.push(result);
                }

                OpCode::Pow => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = binary_pow(a, b)?;
                    frame.push(result);
                }

                // --- Arithmetic (unary) ---
                OpCode::Neg => {
                    let a = frame.pop();
                    let result = unary_neg(a)?;
                    frame.push(result);
                }

                OpCode::Pos => {
                    // Unary plus is essentially a no-op for numbers
                    // but we keep the value on stack
                }

                // --- Bitwise ---
                OpCode::BOr => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = bitwise_or(a, b)?;
                    frame.push(result);
                }

                OpCode::BXor => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = bitwise_xor(a, b)?;
                    frame.push(result);
                }

                OpCode::BAnd => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = bitwise_and(a, b)?;
                    frame.push(result);
                }

                OpCode::BNot => {
                    let a = frame.pop();
                    let result = bitwise_not(a)?;
                    frame.push(result);
                }

                OpCode::LShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = bitwise_lshift(a, b)?;
                    frame.push(result);
                }

                OpCode::RShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = bitwise_rshift(a, b)?;
                    frame.push(result);
                }

                // --- Comparison ---
                OpCode::Lt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = compare_lt(a, b)?;
                    frame.push(result);
                }

                OpCode::Le => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = compare_le(a, b)?;
                    frame.push(result);
                }

                OpCode::Gt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = compare_gt(a, b)?;
                    frame.push(result);
                }

                OpCode::Ge => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = compare_ge(a, b)?;
                    frame.push(result);
                }

                OpCode::Eq => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = if let (Some(idx_a), Some(idx_b)) =
                        (a.as_struct_instance_idx(), b.as_struct_instance_idx())
                    {
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
                    let result = if let (Some(idx_a), Some(idx_b)) =
                        (a.as_struct_instance_idx(), b.as_struct_instance_idx())
                    {
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
                                    jit.as_ref()
                                        .map(|e| e.has_compiled(loop_offset))
                                        .unwrap_or(false)
                                };

                                if has_compiled {
                                    // Validate guards before executing JIT code
                                    let guards = {
                                        let jit = self.jit.lock().unwrap();
                                        jit.as_ref()
                                            .and_then(|e| e.get_guards(loop_offset))
                                            .cloned()
                                    };

                                    let mut guards_pass = true;
                                    let mut guard_locals: Vec<(usize, i64)> = Vec::new();

                                    if let Some(ref guards) = guards {
                                        for (name, expected_value, slot) in guards {
                                            // Resolve current value of name
                                            let current_value: Option<i64> = {
                                                // 1. Check closure_scope
                                                if let Some(ref closure) = frame.closure_scope {
                                                    let closure_bound = closure.bind(py);
                                                    if let Ok(val) = closure_bound
                                                        .call_method1("_resolve", (name.as_str(),))
                                                    {
                                                        if let Ok(value) =
                                                            Value::from_pyobject(py, &val)
                                                        {
                                                            value.as_int()
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        // Fall through to globals
                                                        None
                                                    }
                                                } else {
                                                    None
                                                }
                                                .or_else(|| {
                                                    // 2. Check context.globals
                                                    if let Some(ref py_globals) = ctx_globals {
                                                        if let Ok(Some(val)) =
                                                            py_globals.bind(py).get_item(name)
                                                        {
                                                            if let Ok(value) =
                                                                Value::from_pyobject(py, &val)
                                                            {
                                                                value.as_int()
                                                            } else {
                                                                None
                                                            }
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .or_else(|| {
                                                    // 3. Check VM globals
                                                    self.globals.get(name).and_then(|v| v.as_int())
                                                })
                                            };

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

                                    if guards_pass {
                                        // Execute compiled code
                                        let mut locals_i64: Vec<i64> = frame
                                            .locals
                                            .iter()
                                            .map(|v| v.as_int().unwrap_or(0))
                                            .collect();

                                        // Extend locals array for LoadScope slots
                                        let max_slot =
                                            guard_locals.iter().map(|(s, _)| s).max().copied();
                                        if let Some(max_slot) = max_slot {
                                            if max_slot >= locals_i64.len() {
                                                locals_i64.resize(max_slot + 1, 0);
                                            }
                                        }

                                        // Copy guard values into locals array
                                        for (slot, value) in guard_locals {
                                            locals_i64[slot] = value;
                                        }

                                        let result = {
                                            let jit = self.jit.lock().unwrap();
                                            if let Some(ref executor) = *jit {
                                                unsafe {
                                                    executor.execute(loop_offset, &mut locals_i64)
                                                }
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
                                            // Restore locals from i64 array
                                            for (i, &val) in locals_i64.iter().enumerate() {
                                                if i < frame.locals.len() {
                                                    frame.locals[i] = Value::from_int(val);
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
                                                            eprintln!(
                                                                "[JIT] While loop trace compiled!"
                                                            );
                                                        }
                                                    }
                                                    Ok(false) => {
                                                        if self.trace {
                                                            eprintln!(
                                                                "[JIT] While loop trace not compilable"
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        if self.trace {
                                                            eprintln!(
                                                                "[JIT] While loop compilation failed: {}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if !self.jit_tracing {
                                // Not tracing, check if loop becomes hot
                                if self.jit_detector.record_loop_header(loop_offset) {
                                    // Try cache first — skip recording if trace already cached
                                    let compiled_from_cache = {
                                        let mut jit = self.jit.lock().unwrap();
                                        jit.as_mut()
                                            .map(|e| e.try_compile_from_cache(loop_offset))
                                            .unwrap_or(false)
                                    };
                                    if compiled_from_cache {
                                        if self.trace {
                                            eprintln!(
                                                "[JIT] While loop at offset {} compiled from cache",
                                                loop_offset
                                            );
                                        }
                                    } else {
                                        // Cache miss — start tracing
                                        let num_locals = frame.locals.len();
                                        self.jit_recorder.start(loop_offset, num_locals);
                                        self.jit_tracing = true;
                                        self.jit_tracing_offset = loop_offset;

                                        if self.trace {
                                            eprintln!(
                                                "[JIT] Started tracing while loop at offset {}",
                                                loop_offset
                                            );
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
                        self.jit_recorder
                            .record_conditional_jump(took_jump, true, ip);
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
                        self.jit_recorder
                            .record_conditional_jump(took_jump, false, ip);
                    }
                }

                OpCode::JumpIfFalseOrPop => {
                    let cond = frame.peek();
                    if !cond.is_truthy() {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                    }
                }

                OpCode::JumpIfTrueOrPop => {
                    let cond = frame.peek();
                    if cond.is_truthy() {
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
                        let iter = Py::new(py, SeqIter::from_list(list)?)?;
                        let value = Value::from_pyobject(py, iter.bind(py).as_any())?;
                        frame.push(value);
                        continue;
                    }

                    if let Ok(tuple) = py_obj_bound.cast::<PyTuple>() {
                        let iter = Py::new(py, SeqIter::from_tuple(tuple)?)?;
                        let value = Value::from_pyobject(py, iter.bind(py).as_any())?;
                        frame.push(value);
                        continue;
                    }

                    // Fallback: use cached iter() builtin
                    let iterator = iter_fn.bind(py).call1((py_obj,))?;
                    let value = Value::from_pyobject(py, &iterator)?;
                    frame.push(value);
                }

                OpCode::ForIter => {
                    // TOS is the iterator. Try to get next item.
                    // If exhausted, jump to end of loop (arg is jump target).
                    let iter_val = frame.peek();
                    let py_iter = iter_val.to_pyobject(py);
                    let py_iter_bound = py_iter.bind(py);

                    if let Ok(iter_ref) = py_iter_bound.cast::<SeqIter>() {
                        let mut iter = iter_ref.borrow_mut();
                        match iter.next_value(py)? {
                            Some(value) => frame.push(value),
                            None => {
                                frame.pop();
                                frame.ip = instr.arg as usize;
                            }
                        }
                        continue;
                    }

                    // Fallback: use cached next() builtin with sentinel
                    let sentinel = py.None();
                    match next_fn
                        .bind(py)
                        .call1((py_iter.clone_ref(py), sentinel.clone_ref(py)))
                    {
                        Ok(result) => {
                            if result.is(&sentinel) {
                                // Iterator exhausted - pop iterator and jump
                                frame.pop();
                                frame.ip = instr.arg as usize;
                            } else {
                                // Got a value - push it
                                let value = Value::from_pyobject(py, &result)?;
                                frame.push(value);
                            }
                        }
                        Err(_) => {
                            // StopIteration or other error - pop and jump
                            frame.pop();
                            frame.ip = instr.arg as usize;
                        }
                    }
                }

                // --- Function calls ---
                OpCode::Call => {
                    let nargs = instr.arg as usize;
                    // Pop args as Values (not PyObjects yet)
                    let mut args = Vec::with_capacity(nargs);
                    for _ in 0..nargs {
                        args.push(frame.pop());
                    }
                    args.reverse();
                    // Pop function
                    let func = frame.pop();

                    // Native struct instantiation (fast path)
                    {
                        let py_func_tmp = func.to_pyobject(py);
                        let ptr = py_func_tmp.bind(py).as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            // Extract type info before mutable borrow
                            let (num_fields, min_args, type_name, defaults, init_func) = {
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                let nf = ty.fields.len();
                                let ma = ty.fields.iter().filter(|f| !f.has_default).count();
                                let tn = ty.name.clone();
                                let defs: Vec<Value> =
                                    ty.fields.iter().map(|f| f.default).collect();
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
                            for i in nargs..num_fields {
                                field_values.push(defaults[i]);
                            }
                            let idx = self.struct_registry.create_instance(type_id, field_values);
                            let inst_val = Value::from_struct_instance(idx);

                            // Check for init method (post-constructor)
                            if let Some(init_fn) = init_func {
                                let init_bound = init_fn.bind(py);
                                if let Ok(vm_code) = init_bound.getattr("vm_code") {
                                    let new_code = convert_code_object(py, &vm_code)?;
                                    let closure_scope = init_bound
                                        .getattr("closure_scope")
                                        .ok()
                                        .map(|c| c.unbind());
                                    // Push instance on caller's stack (survives init call)
                                    let frame = self.frame_stack.last_mut().unwrap();
                                    frame.push(inst_val);
                                    // Create init frame with self=instance (keep native Value tag)
                                    let mut new_frame = Frame::with_code(new_code);
                                    new_frame.set_local(0, inst_val);
                                    new_frame.closure_scope = closure_scope;
                                    new_frame.discard_return = true;
                                    // Setup super proxy for init (so super.init() works)
                                    self.setup_super_proxy(py, inst_val, None, &mut new_frame)?;
                                    self.frame_stack.push(new_frame);
                                    continue;
                                }
                            }

                            let frame = self.frame_stack.last_mut().unwrap();
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
                    if let Ok(bound_method) = py_func_bound.cast::<crate::core::BoundCatnipMethod>()
                    {
                        let bm = bound_method.borrow();
                        actual_func_ref = bm.func.bind(py).clone();
                        // Use native struct index if available (avoids CatnipStructProxy round-trip)
                        let instance_val = if let Some(idx) = bm.native_instance_idx {
                            Value::from_struct_instance(idx)
                        } else {
                            Value::from_pyobject(py, bm.instance.bind(py))?
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

                    // Check if this is a VMFunction (has vm_code attribute)
                    if let Ok(vm_code) = actual_func.getattr("vm_code") {
                        // VMFunction - check if compiled, otherwise create new frame
                        let new_code = convert_code_object(py, &vm_code)?;
                        let closure_scope = actual_func
                            .getattr("closure_scope")
                            .ok()
                            .map(|c| c.unbind());

                        let func_id = new_code.func_id();

                        // Register pure function for JIT inlining
                        if new_code.is_pure && self.jit_enabled {
                            let mut jit = self.jit.lock().unwrap();
                            if let Some(ref mut executor) = *jit {
                                executor.register_pure_function(
                                    func_id.clone(),
                                    new_code.clone_with_py(py),
                                );
                            }
                        }

                        // JIT: Handle recursive calls - check BEFORE recording
                        if self.jit_recorder.is_recording() {
                            let ip = frame.ip - 1; // Call instruction was just executed

                            // Check if this is a recursive call (calling the function being traced)
                            let is_recursive_call =
                                if let Some(ref tracing_func_id) = self.jit_tracing_func_id {
                                    &func_id == tracing_func_id
                                } else {
                                    false
                                };

                            if is_recursive_call {
                                // Only record the FIRST CallSelf (when depth=0)
                                // Then increment depth to suspend further recording
                                if self.jit_recursive_depth == 0 {
                                    self.jit_recorder.record_call(
                                        &func_id,
                                        nargs,
                                        new_code.is_pure,
                                        ip,
                                    );
                                }
                                self.jit_recursive_depth += 1;
                            } else {
                                // Non-recursive call - record normally
                                self.jit_recorder.record_call(
                                    &func_id,
                                    nargs,
                                    new_code.is_pure,
                                    ip,
                                );
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

                                        // Copy arguments to first N slots
                                        for (i, arg) in args.iter().enumerate() {
                                            if i < array_size {
                                                locals_array[i] = arg.to_raw() as i64;
                                            }
                                        }

                                        // Populate captured variable slots from name_guards
                                        let fn_guards = fn_guards.to_vec();
                                        for (name, expected_value, slot) in &fn_guards {
                                            // Resolve current value of captured variable
                                            let current_value: Option<i64> = {
                                                if let Some(ref closure) = frame.closure_scope {
                                                    let closure_bound = closure.bind(py);
                                                    if let Ok(val) = closure_bound
                                                        .call_method1("_resolve", (name.as_str(),))
                                                    {
                                                        Value::from_pyobject(py, &val)
                                                            .ok()
                                                            .and_then(|v| v.as_int())
                                                    } else {
                                                        None
                                                    }
                                                } else {
                                                    None
                                                }
                                                .or_else(|| {
                                                    if let Some(ref py_globals) = ctx_globals {
                                                        py_globals
                                                            .bind(py)
                                                            .get_item(name)
                                                            .ok()
                                                            .flatten()
                                                            .and_then(|val| {
                                                                Value::from_pyobject(py, &val)
                                                                    .ok()
                                                                    .and_then(|v| v.as_int())
                                                            })
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .or_else(|| {
                                                    self.globals.get(name).and_then(|v| v.as_int())
                                                })
                                            };

                                            match current_value {
                                                Some(val) if val == *expected_value => {
                                                    if *slot < locals_array.len() {
                                                        locals_array[*slot] = val;
                                                    }
                                                }
                                                _ => {
                                                    // Guard failed: fall back to interpreter
                                                    use_compiled = false;
                                                    break;
                                                }
                                            }
                                        }

                                        if !use_compiled {
                                            // Guard failed, skip to interpreter path
                                        } else {
                                            // Call compiled function with locals pointer and depth=0
                                            // Phase 3: Initial call starts at depth 0
                                            // Safety: locals_array has enough elements for all used slots
                                            let result_raw = unsafe {
                                                compiled_fn(locals_array.as_mut_ptr(), 0)
                                            };

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

                                                let result_value =
                                                    Value::from_raw(result_raw as u64);
                                                frame.push(result_value);
                                                use_compiled = true;
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Check if this function has a pending trace from previous hot detection
                                if let Some(ref pending_func_id) =
                                    self.jit_pending_function_trace.clone()
                                {
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
                                            self.jit_tracing_depth = self.frame_stack.len() + 1; // Depth after frame push
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
                                        eprintln!(
                                            "[JIT] Function '{}' became hot (id: {})",
                                            new_code.name, func_id
                                        );
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
                                            self.jit_tracing_depth = self.frame_stack.len() + 1; // Depth after frame push

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

                            // Create and setup new frame
                            let mut new_frame = Frame::with_code(new_code);
                            new_frame.bind_args(py, &args, None);
                            new_frame.closure_scope = closure_scope;

                            // Setup super proxy if this is a bound method call on a struct with parent_methods
                            if let Some(inst_val) = bound_instance {
                                self.setup_super_proxy(
                                    py,
                                    inst_val,
                                    super_source_type,
                                    &mut new_frame,
                                )?;
                            }

                            // Drop the mutable borrow on frame_stack before pushing
                            // (frame ref becomes invalid after push)
                            self.call_stack.push(CallInfo {
                                name: fn_name,
                                call_start_byte,
                            });
                            self.frame_stack.push(new_frame);
                        }
                        continue;
                    } else {
                        // Regular Python function - call directly

                        // Check if function needs context passed
                        let pass_context = actual_func
                            .getattr("pass_context")
                            .map(|attr| attr.is_truthy().unwrap_or(false))
                            .unwrap_or(false);

                        let mut args_py: Vec<Py<PyAny>> =
                            Vec::with_capacity(args.len() + usize::from(pass_context));

                        if pass_context {
                            if let Some(ref ctx) = self.py_context {
                                args_py.push(ctx.clone_ref(py));
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Function requires context but VM has no context available"
                                        .to_string(),
                                ));
                            }
                        }

                        for arg in args.iter() {
                            args_py.push(arg.to_pyobject(py));
                        }

                        // JIT: record builtin pure calls as native ops
                        if self.jit_tracing && self.jit_recursive_depth == 0 {
                            if let Ok(qualname) = actual_func
                                .getattr("__qualname__")
                                .and_then(|n| n.extract::<String>())
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

                        let args_tuple = PyTuple::new(py, args_py)?;
                        let result = actual_func.call1(args_tuple)?;
                        let value = Value::from_pyobject(py, &result)?;
                        frame.push(value);

                        // Sync globals back to local slots after Python call
                        // This handles cases where Python code called VMFunction which mutated globals
                        let updates: Vec<(usize, Value)> = if let Some(ref code) = frame.code {
                            if let Some(ref py_globals) = ctx_globals {
                                code.slotmap
                                    .iter()
                                    .filter_map(|(name, &slot_idx)| {
                                        match py_globals.bind(py).get_item(name.as_str()) {
                                            Ok(Some(val)) => Value::from_pyobject(py, &val)
                                                .ok()
                                                .map(|v| (slot_idx, v)),
                                            _ => None,
                                        }
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        };
                        for (slot_idx, value) in updates {
                            frame.set_local(slot_idx, value);
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

                    // Pop kwargs values (reverse order)
                    let mut kw_values = Vec::with_capacity(nkw);
                    for _ in 0..nkw {
                        kw_values.push(frame.pop());
                    }
                    kw_values.reverse();

                    // Pop positional args (reverse order)
                    let mut args = Vec::with_capacity(nargs);
                    for _ in 0..nargs {
                        args.push(frame.pop());
                    }
                    args.reverse();

                    // Pop function
                    let func = frame.pop();

                    // Native struct instantiation with kwargs (fast path)
                    {
                        let py_func_tmp = func.to_pyobject(py);
                        let ptr = py_func_tmp.bind(py).as_ptr() as usize;
                        if let Some(&type_id) = self.struct_type_map.get(&ptr) {
                            // Extract type info before mutable borrow
                            let (type_name, field_defaults, field_info, init_func) = {
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                let tn = ty.name.clone();
                                let defs: Vec<(Value, bool)> = ty
                                    .fields
                                    .iter()
                                    .map(|f| (f.default, f.has_default))
                                    .collect();
                                let fi: Vec<(String, bool)> = ty
                                    .fields
                                    .iter()
                                    .map(|f| (f.name.clone(), f.has_default))
                                    .collect();
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
                                let kw_name: String = kw_names_tuple.get_item(i)?.extract()?;
                                match field_info.iter().position(|(n, _)| n == &kw_name) {
                                    Some(idx) => field_values[idx] = *val,
                                    None => {
                                        return Err(VMError::TypeError(format!(
                                            "{}() got an unexpected keyword argument '{}'",
                                            type_name, kw_name
                                        )))
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

                            let inst_idx =
                                self.struct_registry.create_instance(type_id, field_values);
                            let inst_val = Value::from_struct_instance(inst_idx);
                            if let Some(init_fn) = init_func {
                                let init_bound = init_fn.bind(py);
                                if let Ok(vm_code) = init_bound.getattr("vm_code") {
                                    let new_code = convert_code_object(py, &vm_code)?;
                                    let closure_scope = init_bound
                                        .getattr("closure_scope")
                                        .ok()
                                        .map(|c| c.unbind());
                                    let frame = self.frame_stack.last_mut().unwrap();
                                    frame.push(inst_val);
                                    let mut new_frame = Frame::with_code(new_code);
                                    new_frame.set_local(0, inst_val);
                                    new_frame.closure_scope = closure_scope;
                                    new_frame.discard_return = true;
                                    self.setup_super_proxy(py, inst_val, None, &mut new_frame)?;
                                    self.frame_stack.push(new_frame);
                                    continue;
                                }
                            }

                            let frame = self.frame_stack.last_mut().unwrap();
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
                    if let Ok(bound_method) = py_func_bound.cast::<crate::core::BoundCatnipMethod>()
                    {
                        let bm = bound_method.borrow();
                        actual_func_ref_kw = bm.func.bind(py).clone();
                        let instance_val = if let Some(idx) = bm.native_instance_idx {
                            Value::from_struct_instance(idx)
                        } else {
                            Value::from_pyobject(py, bm.instance.bind(py))?
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
                        let name = kw_names_tuple.get_item(i)?;
                        kwargs_dict.set_item(name, val.to_pyobject(py))?;
                    }

                    // Check if VMFunction
                    if let Ok(vm_code) = actual_func_kw.getattr("vm_code") {
                        // VMFunction - create new frame with kwargs
                        let new_code = convert_code_object(py, &vm_code)?;
                        let closure_scope = actual_func_kw
                            .getattr("closure_scope")
                            .ok()
                            .map(|c| c.unbind());

                        let mut new_frame = Frame::with_code(new_code);
                        new_frame.bind_args(py, &args, Some(&kwargs_dict));
                        new_frame.closure_scope = closure_scope;

                        // Setup super proxy for bound method calls
                        if let Some(inst_val) = bound_instance_kw {
                            self.setup_super_proxy(
                                py,
                                inst_val,
                                super_source_type_kw,
                                &mut new_frame,
                            )?;
                        }

                        self.frame_stack.push(new_frame);
                        continue;
                    } else {
                        // Python function - call with kwargs

                        // Check if function needs context passed
                        let pass_context = actual_func_kw
                            .getattr("pass_context")
                            .map(|attr| attr.is_truthy().unwrap_or(false))
                            .unwrap_or(false);

                        let mut args_py: Vec<Py<PyAny>> =
                            Vec::with_capacity(args.len() + usize::from(pass_context));

                        if pass_context {
                            if let Some(ref ctx) = self.py_context {
                                args_py.push(ctx.clone_ref(py));
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Function requires context but VM has no context available"
                                        .to_string(),
                                ));
                            }
                        }

                        for arg in args.iter() {
                            args_py.push(arg.to_pyobject(py));
                        }

                        let args_tuple = PyTuple::new(py, args_py)?;
                        let result = actual_func_kw.call(args_tuple, Some(&kwargs_dict))?;
                        let value = Value::from_pyobject(py, &result)?;
                        frame.push(value);
                    }
                }

                OpCode::TailCall => {
                    // TCO: reuse current frame instead of creating a new one
                    let nargs = instr.arg as usize;
                    let mut args = Vec::with_capacity(nargs);
                    for _ in 0..nargs {
                        args.push(frame.pop());
                    }
                    args.reverse();
                    let func = frame.pop();

                    let py_func = func.to_pyobject(py);
                    let py_func_bound = py_func.bind(py);

                    if let Ok(vm_code) = py_func_bound.getattr("vm_code") {
                        // VMFunction - reuse frame
                        let new_code = convert_code_object(py, &vm_code)?;

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
                            for i in 0..args.len().min(vararg_idx_usize) {
                                frame.locals[i] = args[i];
                            }
                            // Collect excess into vararg slot
                            if args.len() > vararg_idx_usize {
                                let excess: Vec<Py<PyAny>> = args[vararg_idx_usize..]
                                    .iter()
                                    .map(|v: &Value| v.to_pyobject(py))
                                    .collect();
                                let list = PyList::new(py, excess)
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                frame.locals[vararg_idx_usize] =
                                    Value::from_pyobject(py, &list.into_any())?;
                            } else {
                                let empty = PyList::empty(py);
                                frame.locals[vararg_idx_usize] =
                                    Value::from_pyobject(py, &empty.into_any())?;
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
                                    let default_obj = new_code.defaults[default_idx].bind(py);
                                    frame.locals[i] = Value::from_pyobject(py, default_obj)?;
                                }
                            }
                        }

                        // 5. Reset frame state
                        frame.ip = 0;
                        frame.stack.clear();
                        if let Ok(closure) = py_func_bound.getattr("closure_scope") {
                            frame.closure_scope = Some(closure.unbind());
                        }
                        // Replace code object
                        frame.code = Some(new_code);
                        // Continue to restart dispatch with new code
                        continue;
                    } else {
                        // Python callable - call directly
                        let pass_context = py_func_bound
                            .getattr("pass_context")
                            .map(|attr| attr.is_truthy().unwrap_or(false))
                            .unwrap_or(false);

                        let mut args_py: Vec<Py<PyAny>> =
                            Vec::with_capacity(args.len() + usize::from(pass_context));

                        if pass_context {
                            if let Some(ref ctx) = self.py_context {
                                args_py.push(ctx.clone_ref(py));
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Function requires context but VM has no context available"
                                        .to_string(),
                                ));
                            }
                        }

                        for arg in args.iter() {
                            args_py.push(arg.to_pyobject(py));
                        }

                        let args_tuple = PyTuple::new(py, args_py)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let result = py_func_bound
                            .call1(args_tuple)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let value = Value::from_pyobject(py, &result)?;
                        frame.push(value);
                    }
                }

                OpCode::Return => {
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
                    // Note: frame_stack still contains the current frame at this point
                    let current_depth = self.frame_stack.len();
                    let should_finalize_trace = self.jit_tracing
                        && self.jit_tracing_func_id.is_some()
                        && current_depth == self.jit_tracing_depth;

                    // Pop the frame
                    self.frame_stack.pop();

                    // Push result to caller if any (unless init whose return is discarded)
                    if !discard {
                        if let Some(caller) = self.frame_stack.last_mut() {
                            caller.push(last_result);
                        }
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
                                                    func_id, max_slot, name_guards.len()
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
                                                eprintln!(
                                                    "[JIT] Function compilation failed: {}",
                                                    e
                                                );
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
                    if self.frame_stack.len() == 1 {
                        if let Some(caller) = self.frame_stack.last_mut() {
                            let updates: Vec<(usize, Value)> = if let Some(ref code) = caller.code {
                                if let Some(ref py_globals) = ctx_globals {
                                    code.slotmap
                                        .iter()
                                        .filter_map(|(name, &slot_idx)| {
                                            match py_globals.bind(py).get_item(name.as_str()) {
                                                Ok(Some(val)) => Value::from_pyobject(py, &val)
                                                    .ok()
                                                    .map(|v| (slot_idx, v)),
                                                _ => None,
                                            }
                                        })
                                        .collect()
                                } else {
                                    Vec::new()
                                }
                            } else {
                                Vec::new()
                            };
                            for (slot_idx, value) in updates {
                                caller.set_local(slot_idx, value);
                            }
                        }
                    }
                    continue;
                }

                OpCode::MakeFunction => {
                    // Pop code object and create VMFunction
                    let code_obj = frame.pop().to_pyobject(py);

                    // Create closure scope from current frame's locals
                    // IMPORTANT: Don't copy variables that are in context.globals -
                    // they should be accessed via the parent chain so mutations
                    // update the original, not a copy.
                    let closure_dict = PyDict::new(py);
                    if let Some(ref code) = frame.code {
                        // Get context.globals to check which names are module-level
                        let ctx_globals_bound: Option<Bound<'_, PyDict>> =
                            ctx_globals.as_ref().map(|g| g.bind(py).clone());

                        for (name, &slot_idx) in &code.slotmap {
                            // Skip variables that exist in context.globals (module-level)
                            if let Some(ref globals) = ctx_globals_bound {
                                if globals
                                    .contains(name)
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?
                                {
                                    continue;
                                }
                            }
                            let val = frame.get_local(slot_idx);
                            if !val.is_nil() {
                                closure_dict.set_item(name, val.to_pyobject(py))?;
                            }
                        }
                    }

                    // Create parent scope - use frame's closure_scope for proper nesting
                    let parent_scope: Option<Py<PyAny>> =
                        if let Some(ref parent_closure) = frame.closure_scope {
                            // Nested closure: chain to parent's closure scope
                            Some(parent_closure.clone_ref(py))
                        } else if let Some(ref ctx) = self.py_context {
                            // Top-level function: use context.globals as fallback
                            let ctx_bound = ctx.bind(py);
                            if let Ok(py_globals) = ctx_bound.getattr("globals") {
                                if let Ok(dict) = py_globals.cast::<PyDict>() {
                                    let globals_scope =
                                        RustClosureScope::create(dict.clone().unbind(), None);
                                    Some(Py::new(py, globals_scope)?.into_any())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                    // Create closure scope with captured variables
                    let closure_scope = Py::new(
                        py,
                        RustClosureScope::create(closure_dict.unbind(), parent_scope),
                    )?;

                    // Create VMFunction(code, closure_scope, context)
                    let context_for_func = self.py_context.as_ref().map(|c| c.clone_ref(py));

                    // code_obj is a PyCodeObject (RustCodeObject) - extract as Py<PyCodeObject>
                    let code_py: Py<PyCodeObject> = code_obj
                        .bind(py)
                        .cast::<PyCodeObject>()
                        .map_err(|e| VMError::TypeError(format!("Expected CodeObject: {e}")))?
                        .clone()
                        .unbind();

                    let func = Py::new(
                        py,
                        RustVMFunction::create(
                            py,
                            code_py,
                            Some(closure_scope.into_any()),
                            context_for_func,
                        ),
                    )?;
                    let value = Value::from_pyobject(py, func.bind(py))?;
                    frame.push(value);
                }

                // --- Collection literals ---
                OpCode::BuildList => {
                    let n = instr.arg as usize;
                    let mut items: Vec<Py<PyAny>> = Vec::with_capacity(n);
                    for _ in 0..n {
                        items.push(frame.pop().to_pyobject(py));
                    }
                    items.reverse();
                    let list = PyList::new(py, items).unwrap();
                    let value = Value::from_pyobject(py, &list.into_any()).unwrap_or(Value::NIL);
                    frame.push(value);
                }

                OpCode::BuildTuple => {
                    let n = instr.arg as usize;
                    let mut items: Vec<Py<PyAny>> = Vec::with_capacity(n);
                    for _ in 0..n {
                        items.push(frame.pop().to_pyobject(py));
                    }
                    items.reverse();
                    let tuple = PyTuple::new(py, items).unwrap();
                    let value = Value::from_pyobject(py, &tuple.into_any()).unwrap_or(Value::NIL);
                    frame.push(value);
                }

                OpCode::BuildSet => {
                    let n = instr.arg as usize;
                    let mut items = Vec::with_capacity(n);
                    for _ in 0..n {
                        items.push(frame.pop().to_pyobject(py));
                    }
                    items.reverse();
                    // Create a Python set from the items
                    let set_type = py.import("builtins")?.getattr("set")?;
                    let py_list = PyList::new(py, items)?;
                    let py_set = set_type.call1((py_list,))?;
                    let value = Value::from_pyobject(py, &py_set)?;
                    frame.push(value);
                }

                OpCode::BuildDict => {
                    let n = instr.arg as usize;
                    let dict = PyDict::new(py);
                    for _ in 0..n {
                        let value = frame.pop().to_pyobject(py);
                        let key = frame.pop().to_pyobject(py);
                        dict.set_item(key, value).ok();
                    }
                    let value = Value::from_pyobject(py, &dict.into_any()).unwrap_or(Value::NIL);
                    frame.push(value);
                }

                OpCode::BuildSlice => {
                    // Build slice(start, stop[, step])
                    const SLICE_ARGS_MIN: usize = 2;
                    const SLICE_ARGS_MAX: usize = 3;
                    let n = instr.arg as usize;
                    let mut items = Vec::with_capacity(n);
                    for _ in 0..n {
                        items.push(frame.pop().to_pyobject(py));
                    }
                    items.reverse();

                    // Create slice object
                    let slice_type = py.get_type::<pyo3::types::PySlice>();
                    let slice = if n == SLICE_ARGS_MIN {
                        slice_type.call1((&items[0], &items[1]))?
                    } else if n == SLICE_ARGS_MAX {
                        slice_type.call1((&items[0], &items[1], &items[2]))?
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "BUILD_SLICE expects 2 or 3 args, got {}",
                            n
                        )));
                    };
                    let value = Value::from_pyobject(py, &slice)?;
                    frame.push(value);
                }

                // --- Attribute/item access ---
                OpCode::GetAttr => {
                    let attr_name = code.names[instr.arg as usize].clone();
                    let obj = frame.pop();

                    if let Some(idx) = obj.as_struct_instance_idx() {
                        let inst = self.struct_registry.get_instance(idx).unwrap();
                        let type_id = inst.type_id;
                        let ty = self.struct_registry.get_type(type_id).unwrap();
                        match ty.field_index(&attr_name) {
                            Some(field_idx) => {
                                let val = inst.fields[field_idx];
                                frame.push(val);
                            }
                            None => {
                                // Look up method in StructType
                                let ty = self.struct_registry.get_type(type_id).unwrap();
                                if let Some(func) = ty.methods.get(&attr_name) {
                                    let proxy = obj.to_pyobject(py);
                                    let bound = Py::new(
                                        py,
                                        crate::core::BoundCatnipMethod {
                                            func: func.clone_ref(py),
                                            instance: proxy,
                                            super_source_type: None,
                                            native_instance_idx: None,
                                        },
                                    )
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                    let value = Value::from_pyobject(py, bound.bind(py))?;
                                    frame.push(value);
                                } else {
                                    return Err(VMError::RuntimeError(format!(
                                        "'{}' has no attribute '{}'",
                                        ty.name, attr_name
                                    )));
                                }
                            }
                        }
                    } else {
                        let py_obj = obj.to_pyobject(py);
                        let result = py_obj
                            .bind(py)
                            .getattr(attr_name.as_str())
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let value = Value::from_pyobject(py, &result)?;
                        frame.push(value);
                    }
                }

                OpCode::SetAttr => {
                    let attr_name = code.names[instr.arg as usize].clone();
                    let value = frame.pop();
                    let obj = frame.pop();

                    if let Some(idx) = obj.as_struct_instance_idx() {
                        let type_id = self.struct_registry.get_instance(idx).unwrap().type_id;
                        let ty = self.struct_registry.get_type(type_id).unwrap();
                        match ty.field_index(&attr_name) {
                            Some(field_idx) => {
                                self.struct_registry.get_instance_mut(idx).unwrap().fields
                                    [field_idx] = value;
                            }
                            None => {
                                return Err(VMError::RuntimeError(format!(
                                    "'{}' has no attribute '{}'",
                                    ty.name, attr_name
                                )));
                            }
                        }
                    } else {
                        let py_obj = obj.to_pyobject(py);
                        let py_value = value.to_pyobject(py);
                        py_obj
                            .bind(py)
                            .setattr(attr_name.as_str(), py_value)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    }
                }

                OpCode::GetItem => {
                    let index = frame.pop();
                    let obj = frame.pop();
                    let py_obj = obj.to_pyobject(py);
                    let py_index = index.to_pyobject(py);
                    let result = py_obj
                        .bind(py)
                        .get_item(py_index)
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let value = Value::from_pyobject(py, &result)?;
                    frame.push(value);
                }

                OpCode::SetItem => {
                    let value = frame.pop();
                    let index = frame.pop();
                    let obj = frame.pop();
                    let py_obj = obj.to_pyobject(py);
                    let py_index = index.to_pyobject(py);
                    let py_value = value.to_pyobject(py);
                    py_obj
                        .bind(py)
                        .set_item(py_index, py_value)
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                }

                // --- Block/scope ---
                OpCode::PushBlock => {
                    frame.push_block(instr.arg as usize);
                }

                OpCode::PopBlock => {
                    frame.pop_block();
                }

                // --- Control signals ---
                OpCode::Break => {
                    return Err(VMError::Break);
                }

                OpCode::Continue => {
                    return Err(VMError::Continue);
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

                    // Handle ND operations specially
                    if is_nd_recursion || is_nd_map {
                        // For ND ops: pop lambda/func, then target
                        let lambda_val = frame.pop();
                        let target_val = frame.pop();
                        let lambda_py = lambda_val.to_pyobject(py);
                        let target_py = target_val.to_pyobject(py);

                        // Get registry
                        let registry = if let Some(ref ctx) = self.py_context {
                            let ctx_bound = ctx.bind(py);
                            ctx_bound
                                .getattr("_registry")
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?
                        } else {
                            return Err(VMError::RuntimeError(
                                "Context not available for broadcast".to_string(),
                            ));
                        };

                        // Iterate over target and apply ND operation to each element
                        let target_bound = target_py.bind(py);
                        let is_tuple = target_bound.is_instance_of::<pyo3::types::PyTuple>();
                        let result_list = pyo3::types::PyList::empty(py);

                        for elem_result in target_bound.try_iter()? {
                            let elem = elem_result?;
                            let elem_py = elem.unbind();

                            // Call appropriate ND operation
                            let nd_result = if is_nd_recursion {
                                registry
                                    .call_method(
                                        "execute_nd_recursion_py",
                                        (elem_py, lambda_py.clone_ref(py)),
                                        None,
                                    )
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?
                            } else {
                                registry
                                    .call_method(
                                        "execute_nd_map_py",
                                        (elem_py, lambda_py.clone_ref(py)),
                                        None,
                                    )
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?
                            };

                            result_list
                                .append(nd_result)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        }

                        // Convert back to tuple if needed
                        let result_bound = if is_tuple {
                            pyo3::types::PyTuple::new(py, result_list)?.into_any()
                        } else {
                            result_list.into_any()
                        };

                        let value = Value::from_pyobject(py, &result_bound)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
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

                        // Delegate to Rust Registry's _apply_broadcast
                        let result = if let Some(ref ctx) = self.py_context {
                            let ctx_bound = ctx.bind(py);
                            if let Ok(registry) = ctx_bound.getattr("_registry") {
                                let target_bound = target.bind(py);
                                let operator_bound = operator.bind(py);
                                let operand_bound = operand.as_ref().map(|o| o.bind(py));

                                registry.call_method(
                                    "_apply_broadcast",
                                    (&target_bound, &operator_bound, operand_bound, is_filter),
                                    None,
                                )?
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Registry not available in context".to_string(),
                                ));
                            }
                        } else {
                            return Err(VMError::RuntimeError(
                                "Context not available for broadcast".to_string(),
                            ));
                        };

                        let value = Value::from_pyobject(py, &result)?;
                        frame.push(value);
                    }
                }

                // --- Pattern matching ---
                OpCode::MatchPattern => {
                    // Legacy path: pattern stored as constant (PyObject)
                    let pattern_idx = instr.arg as usize;
                    let pattern = code
                        .constants
                        .get(pattern_idx)
                        .copied()
                        .unwrap_or(Value::NIL)
                        .to_pyobject(py);

                    let value = frame.pop().to_pyobject(py);

                    // Delegate to Python's _match_pattern via context
                    let bindings = if let Some(ref ctx) = self.py_context {
                        let ctx_bound = ctx.bind(py);
                        if let Ok(registry) = ctx_bound.getattr("_registry") {
                            let result =
                                registry.call_method1("_match_pattern", (&pattern, &value))?;
                            Value::from_pyobject(py, &result)?
                        } else {
                            Value::NIL
                        }
                    } else {
                        Value::NIL
                    };
                    frame.push(bindings);
                }

                OpCode::MatchPatternVM => {
                    // Native path: pre-compiled VMPattern, no Python boundary crossing
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    let pattern = frame
                        .code
                        .as_ref()
                        .and_then(|c| c.patterns.get(pat_idx))
                        .cloned();
                    match pattern {
                        Some(ref pat) => {
                            match vm_match_pattern(py, pat, value, &self.struct_registry)? {
                                Some(bindings) => {
                                    frame.match_bindings = Some(bindings);
                                    frame.push(Value::TRUE);
                                }
                                None => {
                                    frame.match_bindings = None;
                                    frame.push(Value::NIL);
                                }
                            }
                        }
                        None => {
                            frame.match_bindings = None;
                            frame.push(Value::NIL);
                        }
                    }
                }

                OpCode::BindMatch => {
                    // New path: native bindings from MatchPatternVM
                    if let Some(bindings) = frame.match_bindings.take() {
                        frame.pop(); // pop the sentinel TRUE
                        for (slot, val) in bindings {
                            frame.set_local(slot, val);
                        }
                    } else {
                        // Legacy path: bindings as PyDict from MatchPattern
                        let slotmap: Vec<(String, usize)> = frame
                            .code
                            .as_ref()
                            .map(|c| c.slotmap.iter().map(|(k, &v)| (k.clone(), v)).collect())
                            .unwrap_or_default();

                        let bindings = frame.pop().to_pyobject(py);
                        let bindings_bound = bindings.bind(py);

                        if let Ok(dict) = bindings_bound.cast::<PyDict>() {
                            for (key, val) in dict.iter() {
                                if let Ok(name) = key.extract::<String>() {
                                    for (slot_name, slot_idx) in &slotmap {
                                        if slot_name == &name {
                                            let value = Value::from_pyobject(py, &val)?;
                                            frame.set_local(*slot_idx, value);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                OpCode::JumpIfNone => {
                    let value = frame.pop();
                    if value.is_nil() {
                        frame.ip = instr.arg as usize;
                    }
                }

                // --- Unpacking ---
                OpCode::UnpackSequence => {
                    let n = instr.arg as usize;
                    let seq = frame.pop();
                    let py_seq = seq.to_pyobject(py);
                    let py_seq_bound = py_seq.bind(py);

                    // Convert to list to get items
                    let items: Vec<Py<PyAny>> = py_seq_bound
                        .try_iter()?
                        .map(|item| item.map(|i| i.unbind()))
                        .collect::<PyResult<Vec<_>>>()?;

                    if items.len() != n {
                        return Err(VMError::RuntimeError(format!(
                            "Cannot unpack {} values into {} variables",
                            items.len(),
                            n
                        )));
                    }

                    // Push items in reverse order (so first item ends on top)
                    for item in items.into_iter().rev() {
                        let val = Value::from_pyobject(py, item.bind(py))?;
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
                    let items: Vec<Py<PyAny>> = py_seq_bound
                        .try_iter()?
                        .map(|item| item.map(|i| i.unbind()))
                        .collect::<PyResult<Vec<_>>>()?;

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
                        let val = Value::from_pyobject(py, item.bind(py))?;
                        frame.push(val);
                    }

                    // Create list for rest
                    let rest_py: Vec<Py<PyAny>> =
                        rest_items.iter().map(|item| item.clone_ref(py)).collect();
                    let rest_list = PyList::new(py, rest_py)?;
                    let rest_val = Value::from_pyobject(py, &rest_list.into_any())?;
                    frame.push(rest_val);

                    for item in before_items.iter().rev() {
                        let val = Value::from_pyobject(py, item.bind(py))?;
                        frame.push(val);
                    }
                }

                // --- Optimized iteration ---
                OpCode::ForRangeInt => {
                    // Optimized range loop condition check
                    // Replaces: LoadLocal + LoadLocal + GE/LE + JumpIfTrue (4 opcodes -> 1)
                    // arg = (slot_i << 24) | (slot_stop << 16) | (step_sign << 15) | jump_offset
                    // step_sign: 0 = positive step, 1 = negative step
                    // jump_offset: relative offset to jump when done (max 32767)
                    const SLOT_I_SHIFT: u32 = 24;
                    const SLOT_STOP_SHIFT: u32 = 16;
                    const STEP_SIGN_SHIFT: u32 = 15;
                    const SLOT_MASK: u32 = 0xFF;
                    const JUMP_OFFSET_MASK: u32 = 0x7FFF;

                    let slot_i = (instr.arg >> SLOT_I_SHIFT) as usize;
                    let slot_stop = ((instr.arg >> SLOT_STOP_SHIFT) & SLOT_MASK) as usize;
                    let step_positive = ((instr.arg >> STEP_SIGN_SHIFT) & 1) == 0;
                    let jump_offset = (instr.arg & JUMP_OFFSET_MASK) as usize;

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
                            self.jit_recorder.record_opcode(
                                OpCode::ForRangeInt,
                                instr.arg,
                                true,
                                ip,
                            );
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
                                    jit.as_ref()
                                        .map(|e| e.has_compiled(loop_offset))
                                        .unwrap_or(false)
                                };

                                if has_compiled {
                                    // Validate guards before executing JIT code
                                    let guards = {
                                        let jit = self.jit.lock().unwrap();
                                        jit.as_ref()
                                            .and_then(|e| e.get_guards(loop_offset))
                                            .cloned()
                                    };

                                    let mut guards_pass = true;
                                    let mut guard_locals: Vec<(usize, i64)> = Vec::new();

                                    if let Some(ref guards) = guards {
                                        for (name, expected_value, slot) in guards {
                                            // Resolve current value of name
                                            let current_value: Option<i64> = {
                                                // 1. Check closure_scope
                                                if let Some(ref closure) = frame.closure_scope {
                                                    let closure_bound = closure.bind(py);
                                                    if let Ok(val) = closure_bound
                                                        .call_method1("_resolve", (name.as_str(),))
                                                    {
                                                        if let Ok(value) =
                                                            Value::from_pyobject(py, &val)
                                                        {
                                                            value.as_int()
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                } else {
                                                    None
                                                }
                                                .or_else(|| {
                                                    // 2. Check context.globals
                                                    if let Some(ref py_globals) = ctx_globals {
                                                        if let Ok(Some(val)) =
                                                            py_globals.bind(py).get_item(name)
                                                        {
                                                            if let Ok(value) =
                                                                Value::from_pyobject(py, &val)
                                                            {
                                                                value.as_int()
                                                            } else {
                                                                None
                                                            }
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .or_else(|| {
                                                    // 3. Check VM globals
                                                    self.globals.get(name).and_then(|v| v.as_int())
                                                })
                                            };

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

                                    if guards_pass {
                                        // Execute compiled code
                                        // Convert locals to raw i64: ints/bools as i64, floats as f64 bits
                                        // JIT operates on "native" values, not NaN-boxed
                                        let mut locals_raw: Vec<i64> = frame
                                            .locals
                                            .iter()
                                            .map(|v| {
                                                if let Some(i) = v.as_int() {
                                                    i // Real int value
                                                } else if let Some(b) = v.as_bool() {
                                                    if b {
                                                        1
                                                    } else {
                                                        0
                                                    } // Bool as int
                                                } else if let Some(f) = v.as_float() {
                                                    f.to_bits() as i64 // Float bits as i64
                                                } else {
                                                    0 // Fallback
                                                }
                                            })
                                            .collect();

                                        // Remember slot types for restoration (0=int, 1=bool, 2=float)
                                        let slot_types: Vec<u8> = frame
                                            .locals
                                            .iter()
                                            .map(|v| {
                                                if v.is_float() {
                                                    2
                                                } else if v.is_bool() {
                                                    1
                                                } else {
                                                    0
                                                }
                                            })
                                            .collect();

                                        // Extend locals array for LoadScope slots
                                        let max_slot =
                                            guard_locals.iter().map(|(s, _)| s).max().copied();
                                        if let Some(max_slot) = max_slot {
                                            if max_slot >= locals_raw.len() {
                                                locals_raw.resize(max_slot + 1, 0);
                                            }
                                        }

                                        // Copy guard values into locals array
                                        for (slot, value) in guard_locals {
                                            locals_raw[slot] = value;
                                        }

                                        // Call JIT code
                                        let result = {
                                            let jit = self.jit.lock().unwrap();
                                            if let Some(ref executor) = *jit {
                                                unsafe {
                                                    executor.execute(loop_offset, &mut locals_raw)
                                                }
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
                                            // Restore locals: reconstruct Values from raw i64
                                            // Use original type info to decide int/bool/float
                                            for (i, &val) in locals_raw.iter().enumerate() {
                                                if i < frame.locals.len() {
                                                    frame.locals[i] = match slot_types[i] {
                                                        2 => Value::from_float(f64::from_bits(
                                                            val as u64,
                                                        )),
                                                        1 => Value::from_bool(val != 0),
                                                        _ => Value::from_int(val),
                                                    };
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
                                                        eprintln!(
                                                            "[JIT] Compilation failed: {}",
                                                            e
                                                        );
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
                            self.jit_recorder.record_opcode(
                                OpCode::ForRangeInt,
                                instr.arg,
                                true,
                                ip,
                            );
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
                                self.jit_recorder.record_opcode(
                                    OpCode::ForRangeInt,
                                    instr.arg,
                                    true,
                                    ip,
                                );

                                if self.trace {
                                    eprintln!(
                                        "[JIT] Starting trace at {} (bounds: {} - {})",
                                        loop_offset, loop_start, loop_end
                                    );
                                }
                            } else if self.jit_detector.record_loop_header(loop_offset) {
                                // Loop just became hot — try cache first
                                let compiled_from_cache = {
                                    let mut jit = self.jit.lock().unwrap();
                                    jit.as_mut()
                                        .map(|e| e.try_compile_from_cache(loop_offset))
                                        .unwrap_or(false)
                                };
                                if compiled_from_cache {
                                    if self.trace {
                                        eprintln!(
                                            "[JIT] ForRange loop at {} compiled from cache",
                                            loop_offset
                                        );
                                    }
                                    // Don't schedule tracing, compiled code will be picked up next iteration
                                } else {
                                    // Cache miss — schedule tracing for next iteration
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

                OpCode::MakeStruct => {
                    let const_idx = instr.arg as usize;
                    let struct_info_val = code.constants[const_idx];
                    let struct_info_py = struct_info_val.to_pyobject(py);
                    let info_tuple = struct_info_py.bind(py).cast::<PyTuple>().map_err(|e| {
                        VMError::RuntimeError(format!("MakeStruct: bad constant: {e}"))
                    })?;

                    let name: String = info_tuple
                        .get_item(0)
                        .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                        .extract()
                        .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                    let fields_info = info_tuple
                        .get_item(1)
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let num_defaults: usize = info_tuple
                        .get_item(2)
                        .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                        .extract()
                        .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                    // Detect format: new format has implements tuple at index 3
                    // New: (name, fields, num_defaults, implements, base_or_None, [methods])
                    // Legacy: (name, fields, num_defaults, [base_string_or_methods])
                    let mut implements_list: Vec<String> = Vec::new();
                    let mut base_name: Option<String> = None;
                    let mut methods_idx: Option<usize> = None;

                    if info_tuple.len() > 3 {
                        let item3 = info_tuple
                            .get_item(3)
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                        // New format: item3 is a tuple (implements list)
                        if item3.is_instance_of::<PyTuple>() {
                            for imp in item3
                                .try_iter()
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?
                            {
                                let imp = imp.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                                implements_list
                                    .push(imp.extract().map_err(|e: PyErr| {
                                        VMError::RuntimeError(e.to_string())
                                    })?);
                            }
                            // item4 = base_or_None
                            if info_tuple.len() > 4 {
                                let item4 = info_tuple
                                    .get_item(4)
                                    .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                                if !item4.is_none() {
                                    base_name = item4.extract::<String>().ok();
                                }
                            }
                            // item5 = methods
                            if info_tuple.len() > 5 {
                                methods_idx = Some(5);
                            }
                        } else if let Ok(base) = item3.extract::<String>() {
                            // Legacy: item3 is base name string
                            base_name = Some(base);
                            if info_tuple.len() > 4 {
                                methods_idx = Some(4);
                            }
                        } else {
                            // Legacy: item3 is methods list
                            methods_idx = Some(3);
                        }
                    }

                    // Pop default values from stack (LIFO order)
                    let mut default_values: Vec<Value> = Vec::with_capacity(num_defaults);
                    for _ in 0..num_defaults {
                        default_values.push(frame.pop());
                    }
                    default_values.reverse();

                    // Parse fields
                    let fields_tuple = fields_info
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let mut native_fields = Vec::new();
                    let mut default_idx = 0usize;
                    for fi in fields_tuple.iter() {
                        let pair = fi
                            .cast::<PyTuple>()
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let fname: String = pair
                            .get_item(0)
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                            .extract()
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                        let has_default: bool = pair
                            .get_item(1)
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                            .extract()
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
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
                    let mut methods_map: HashMap<String, Py<PyAny>> = HashMap::new();
                    if let Some(midx) = methods_idx {
                        let methods = info_tuple
                            .get_item(midx)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        for method_result in methods
                            .try_iter()
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?
                        {
                            let method_pair =
                                method_result.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let pair = method_pair
                                .cast::<PyTuple>()
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let method_name: String = pair
                                .get_item(0)
                                .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                                .extract()
                                .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;

                            // Get CodeObject and create VMFunction
                            let code_obj = pair
                                .get_item(1)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                            // Create closure scope from current frame's locals
                            let closure_dict = PyDict::new(py);
                            if let Some(ref code) = frame.code {
                                let ctx_globals_bound: Option<Bound<'_, PyDict>> =
                                    ctx_globals.as_ref().map(|g| g.bind(py).clone());

                                for (lname, &slot_idx) in &code.slotmap {
                                    if let Some(ref globals) = ctx_globals_bound {
                                        if globals
                                            .contains(lname)
                                            .map_err(|e| VMError::RuntimeError(e.to_string()))?
                                        {
                                            continue;
                                        }
                                    }
                                    let val = frame.get_local(slot_idx);
                                    if !val.is_nil() {
                                        closure_dict.set_item(lname, val.to_pyobject(py))?;
                                    }
                                }
                            }

                            let parent_scope: Option<Py<PyAny>> = if let Some(ref parent_closure) =
                                frame.closure_scope
                            {
                                Some(parent_closure.clone_ref(py))
                            } else if let Some(ref ctx) = self.py_context {
                                let ctx_bound = ctx.bind(py);
                                if let Ok(py_globals) = ctx_bound.getattr("globals") {
                                    if let Ok(dict) = py_globals.cast::<PyDict>() {
                                        let globals_scope =
                                            RustClosureScope::create(dict.clone().unbind(), None);
                                        Some(
                                            Py::new(py, globals_scope)
                                                .map_err(|e| VMError::RuntimeError(e.to_string()))?
                                                .into_any(),
                                        )
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            let closure_scope = Py::new(
                                py,
                                RustClosureScope::create(closure_dict.unbind(), parent_scope),
                            )
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let context_for_func =
                                self.py_context.as_ref().map(|c| c.clone_ref(py));
                            let code_py: Py<PyCodeObject> = code_obj
                                .cast::<PyCodeObject>()
                                .map_err(|e| {
                                    VMError::TypeError(format!("Expected CodeObject: {e}"))
                                })?
                                .clone()
                                .unbind();
                            let func = Py::new(
                                py,
                                RustVMFunction::create(
                                    py,
                                    code_py,
                                    Some(closure_scope.into_any()),
                                    context_for_func,
                                ),
                            )
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                            methods_map.insert(method_name, func.into_any());
                        }
                    }

                    // Phase 1: extends(base) merges parent fields+methods.
                    let mut parent_methods_map: HashMap<String, Py<PyAny>> = HashMap::new();
                    let parent_type_name = base_name.clone();
                    let (mut merged_fields, mut merged_methods) = if let Some(base) = base_name {
                        let parent_type = self
                            .struct_registry
                            .find_type_by_name(&base)
                            .ok_or_else(|| {
                                VMError::RuntimeError(format!(
                                    "Unknown base struct '{}' for '{}'",
                                    base, name
                                ))
                            })?;

                        let mut inherited_fields = parent_type.fields.clone();
                        for child_field in &native_fields {
                            if parent_type.field_index(&child_field.name).is_some() {
                                return Err(VMError::RuntimeError(format!(
                                    "Struct '{}' redefines inherited field '{}'",
                                    name, child_field.name
                                )));
                            }
                        }
                        inherited_fields.extend(native_fields);

                        // Collect parent methods for super (includes grandparent chain)
                        for (k, v) in &parent_type.parent_methods {
                            parent_methods_map.insert(k.clone(), v.clone_ref(py));
                        }
                        for (k, v) in &parent_type.methods {
                            parent_methods_map.insert(k.clone(), v.clone_ref(py));
                        }

                        let mut inherited_methods: HashMap<String, Py<PyAny>> = parent_type
                            .methods
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                            .collect();
                        for (mname, mfunc) in methods_map {
                            inherited_methods.insert(mname, mfunc);
                        }

                        (inherited_fields, inherited_methods)
                    } else {
                        (native_fields, methods_map)
                    };

                    // Phase 2: implements(T1, T2, ...) resolves trait composition.
                    let mut trait_mro = Vec::new();
                    if !implements_list.is_empty() {
                        let struct_method_names: HashSet<String> =
                            merged_methods.keys().cloned().collect();
                        let resolved = self
                            .trait_registry
                            .resolve_for_struct(py, &implements_list, &struct_method_names)
                            .map_err(VMError::RuntimeError)?;

                        trait_mro = resolved.linearization;

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
                    }

                    // Build MRO: struct + traits
                    let mut mro = vec![name.clone()];
                    mro.extend(trait_mro);

                    let type_id = self.struct_registry.register_type_with_parents(
                        name.clone(),
                        merged_fields,
                        merged_methods,
                        implements_list,
                        mro,
                        parent_methods_map,
                        parent_type_name,
                    );

                    // Create marker type for struct_type_map
                    let marker = py
                        .eval(
                            &std::ffi::CString::new(format!("type('{}', (), {{}})", name)).unwrap(),
                            None,
                            None,
                        )
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let ptr = marker.as_ptr() as usize;
                    self.struct_type_map.insert(ptr, type_id);

                    // Store marker in context.globals
                    if let Some(ref ctx) = self.py_context {
                        let ctx_bound = ctx.bind(py);
                        let globals = ctx_bound
                            .getattr("globals")
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        globals
                            .call_method1("__setitem__", (&name, &marker))
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    }
                    // Also store in VM globals for scope resolution
                    let val = Value::from_pyobject(py, &marker)
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    self.globals.insert(name, val);
                }

                OpCode::MakeTrait => {
                    let const_idx = instr.arg as usize;
                    let trait_info_val = code.constants[const_idx];
                    let trait_info_py = trait_info_val.to_pyobject(py);
                    let info_tuple = trait_info_py.bind(py).cast::<PyTuple>().map_err(|e| {
                        VMError::RuntimeError(format!("MakeTrait: bad constant: {e}"))
                    })?;

                    // (name, extends_tuple, fields_info, num_defaults, [methods])
                    let name: String = info_tuple
                        .get_item(0)
                        .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                        .extract()
                        .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;

                    let extends_obj = info_tuple
                        .get_item(1)
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let mut extends: Vec<String> = Vec::new();
                    for e in extends_obj
                        .try_iter()
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?
                    {
                        let e = e.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        extends.push(
                            e.extract()
                                .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?,
                        );
                    }

                    let fields_info = info_tuple
                        .get_item(2)
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let num_defaults: usize = info_tuple
                        .get_item(3)
                        .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                        .extract()
                        .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;

                    let has_methods = info_tuple.len() > 4;

                    // Pop default values from stack
                    let mut default_values: Vec<Value> = Vec::with_capacity(num_defaults);
                    for _ in 0..num_defaults {
                        default_values.push(frame.pop());
                    }
                    default_values.reverse();

                    // Parse fields
                    let fields_tuple = fields_info
                        .cast::<PyTuple>()
                        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                    let mut trait_fields = Vec::new();
                    let mut default_idx = 0usize;
                    for fi in fields_tuple.iter() {
                        let pair = fi
                            .cast::<PyTuple>()
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        let fname: String = pair
                            .get_item(0)
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                            .extract()
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                        let has_default: bool = pair
                            .get_item(1)
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                            .extract()
                            .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
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
                    let mut method_bodies: HashMap<String, Py<PyAny>> = HashMap::new();
                    if has_methods {
                        let methods = info_tuple
                            .get_item(4)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        for method_result in methods
                            .try_iter()
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?
                        {
                            let method_pair =
                                method_result.map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let pair = method_pair
                                .cast::<PyTuple>()
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let method_name: String = pair
                                .get_item(0)
                                .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?
                                .extract()
                                .map_err(|e: PyErr| VMError::RuntimeError(e.to_string()))?;
                            let code_obj = pair
                                .get_item(1)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                            // Create closure scope from current frame
                            let closure_dict = PyDict::new(py);
                            if let Some(ref code) = frame.code {
                                let ctx_globals_bound: Option<Bound<'_, PyDict>> =
                                    ctx_globals.as_ref().map(|g| g.bind(py).clone());
                                for (lname, &slot_idx) in &code.slotmap {
                                    if let Some(ref globals) = ctx_globals_bound {
                                        if globals
                                            .contains(lname)
                                            .map_err(|e| VMError::RuntimeError(e.to_string()))?
                                        {
                                            continue;
                                        }
                                    }
                                    let val = frame.get_local(slot_idx);
                                    if !val.is_nil() {
                                        closure_dict.set_item(lname, val.to_pyobject(py))?;
                                    }
                                }
                            }

                            let parent_scope: Option<Py<PyAny>> = if let Some(ref parent_closure) =
                                frame.closure_scope
                            {
                                Some(parent_closure.clone_ref(py))
                            } else if let Some(ref ctx) = self.py_context {
                                let ctx_bound = ctx.bind(py);
                                if let Ok(py_globals) = ctx_bound.getattr("globals") {
                                    if let Ok(dict) = py_globals.cast::<PyDict>() {
                                        let globals_scope =
                                            RustClosureScope::create(dict.clone().unbind(), None);
                                        Some(
                                            Py::new(py, globals_scope)
                                                .map_err(|e| VMError::RuntimeError(e.to_string()))?
                                                .into_any(),
                                        )
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            let closure_scope = Py::new(
                                py,
                                RustClosureScope::create(closure_dict.unbind(), parent_scope),
                            )
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                            let context_for_func =
                                self.py_context.as_ref().map(|c| c.clone_ref(py));
                            let code_py: Py<PyCodeObject> = code_obj
                                .cast::<PyCodeObject>()
                                .map_err(|e| {
                                    VMError::TypeError(format!("Expected CodeObject: {e}"))
                                })?
                                .clone()
                                .unbind();
                            let func = Py::new(
                                py,
                                RustVMFunction::create(
                                    py,
                                    code_py,
                                    Some(closure_scope.into_any()),
                                    context_for_func,
                                ),
                            )
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;

                            method_bodies.insert(method_name, func.into_any());
                        }
                    }

                    // Register trait
                    let trait_def = TraitDef::new(name, extends, trait_fields, method_bodies);
                    self.trait_registry.register_trait(trait_def);
                }

                // --- ND Operations ---
                OpCode::NdEmptyTopos => {
                    // Get cached NDTopos singleton or create it
                    if self.cached_nd_topos.is_none() {
                        let nd_module = py
                            .import("catnip.nd")
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
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

                        let result = if let Some(ref ctx) = self.py_context {
                            let ctx_bound = ctx.bind(py);
                            if let Ok(registry) = ctx_bound.getattr("_registry") {
                                registry
                                    .call_method(
                                        "execute_nd_recursion_py",
                                        (seed_py, lambda_py),
                                        None,
                                    )
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Registry not found in context".to_string(),
                                ));
                            }
                        } else {
                            return Err(VMError::RuntimeError("No context available".to_string()));
                        };
                        let value = Value::from_pyobject(py, &result)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        frame.push(value);
                    } else {
                        // Declaration: pop lambda, push back
                        let lambda_val = frame.pop();
                        frame.push(lambda_val);
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

                        let result = if let Some(ref ctx) = self.py_context {
                            let ctx_bound = ctx.bind(py);
                            if let Ok(registry) = ctx_bound.getattr("_registry") {
                                registry
                                    .call_method("execute_nd_map_py", (data_py, func_py), None)
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?
                            } else {
                                return Err(VMError::RuntimeError(
                                    "Registry not found in context".to_string(),
                                ));
                            }
                        } else {
                            return Err(VMError::RuntimeError("No context available".to_string()));
                        };
                        let value = Value::from_pyobject(py, &result)
                            .map_err(|e| VMError::RuntimeError(e.to_string()))?;
                        frame.push(value);
                    } else {
                        // Lift: pop func, push back
                        let func_val = frame.pop();
                        frame.push(func_val);
                    }
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
                                    if !val.is_nil() {
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
                    // Check if main frame before dropping borrow
                    let is_main_frame = self.frame_stack.len() == 1;
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
                let action = self.invoke_debug_callback(
                    py,
                    _current_src_byte,
                    &locals_data,
                    &call_stack_snapshot,
                )?;
                self.debug_step_mode = action;
                if action == DebugStepMode::StepOver || action == DebugStepMode::StepOut {
                    self.debug_step_depth = depth;
                }
                continue;
            }
        }

        Ok(last_result)
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

// --- Binary operations on NaN-boxed values ---

#[inline]
fn binary_add(a: Value, b: Value) -> VMResult<Value> {
    // Fast path: both small ints
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(v) = Value::try_from_int(ai.wrapping_add(bi)) {
            return Ok(v);
        }
    }
    // Float path
    if let (Some(af), Some(bf)) = (a.as_float(), b.as_float()) {
        return Ok(Value::from_float(af + bf));
    }
    // Int + float
    if let (Some(ai), Some(bf)) = (a.as_int(), b.as_float()) {
        return Ok(Value::from_float(ai as f64 + bf));
    }
    if let (Some(af), Some(bi)) = (a.as_float(), b.as_int()) {
        return Ok(Value::from_float(af + bi as f64));
    }
    Err(VMError::TypeError("unsupported operand types for +".into()))
}

#[inline]
fn binary_sub(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(v) = Value::try_from_int(ai.wrapping_sub(bi)) {
            return Ok(v);
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
    Err(VMError::TypeError("unsupported operand types for -".into()))
}

#[inline]
fn binary_mul(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if let Some(v) = Value::try_from_int(ai.wrapping_mul(bi)) {
            return Ok(v);
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
    Err(VMError::TypeError("unsupported operand types for *".into()))
}

#[inline]
fn binary_div(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    // Division always returns float in Catnip (like Python 3)
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError("division by zero".into()));
        }
        return Ok(Value::from_float(af / bf));
    }
    // Fallback to Python
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let operator = py.import("operator")?;
    let py_result = operator.call_method1("truediv", (&py_a, &py_b))?;
    Ok(Value::from_pyobject(py, &py_result)?)
}

#[inline]
fn binary_floordiv(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi == 0 {
            return Err(VMError::ZeroDivisionError(
                "integer division or modulo by zero".into(),
            ));
        }
        return Ok(Value::from_int(ai / bi));
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError(
                "float floor division by zero".into(),
            ));
        }
        return Ok(Value::from_float((af / bf).floor()));
    }
    // Fallback to Python
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let operator = py.import("operator")?;
    let py_result = operator.call_method1("floordiv", (&py_a, &py_b))?;
    Ok(Value::from_pyobject(py, &py_result)?)
}

#[inline]
fn binary_mod(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if bi == 0 {
            return Err(VMError::ZeroDivisionError(
                "integer division or modulo by zero".into(),
            ));
        }
        return Ok(Value::from_int(ai % bi));
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        if bf == 0.0 {
            return Err(VMError::ZeroDivisionError("float modulo by zero".into()));
        }
        return Ok(Value::from_float(af % bf));
    }
    // Fallback to Python (for string formatting, etc.)
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let operator = py.import("operator")?;
    let py_result = operator.call_method1("mod", (&py_a, &py_b))?;
    Ok(Value::from_pyobject(py, &py_result)?)
}

#[inline]
fn binary_pow(a: Value, b: Value) -> VMResult<Value> {
    const MAX_INT_SHIFT: i64 = 64;
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if (0..MAX_INT_SHIFT).contains(&bi) {
            if let Some(result) = ai.checked_pow(bi as u32) {
                if let Some(v) = Value::try_from_int(result) {
                    return Ok(v);
                }
            }
        }
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_float(af.powf(bf)));
    }
    Err(VMError::TypeError(
        "unsupported operand types for **".into(),
    ))
}

#[inline]
fn unary_neg(a: Value) -> VMResult<Value> {
    if let Some(i) = a.as_int() {
        if let Some(v) = Value::try_from_int(-i) {
            return Ok(v);
        }
    }
    if let Some(f) = a.as_float() {
        return Ok(Value::from_float(-f));
    }
    Err(VMError::TypeError("bad operand type for unary -".into()))
}

// --- Bitwise operations ---

#[inline]
fn bitwise_or(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai | bi));
    }
    Err(VMError::TypeError("unsupported operand types for |".into()))
}

#[inline]
fn bitwise_xor(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai ^ bi));
    }
    Err(VMError::TypeError("unsupported operand types for ^".into()))
}

#[inline]
fn bitwise_and(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_int(ai & bi));
    }
    Err(VMError::TypeError("unsupported operand types for &".into()))
}

#[inline]
fn bitwise_not(a: Value) -> VMResult<Value> {
    if let Some(i) = a.as_int() {
        return Ok(Value::from_int(!i));
    }
    Err(VMError::TypeError("bad operand type for unary ~".into()))
}

#[inline]
fn bitwise_lshift(a: Value, b: Value) -> VMResult<Value> {
    const MAX_INT_SHIFT: i64 = 64;
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if (0..MAX_INT_SHIFT).contains(&bi) {
            return Ok(Value::from_int(ai << bi));
        }
    }
    Err(VMError::TypeError(
        "unsupported operand types for <<".into(),
    ))
}

#[inline]
fn bitwise_rshift(a: Value, b: Value) -> VMResult<Value> {
    const MAX_INT_SHIFT: i64 = 64;
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        if (0..MAX_INT_SHIFT).contains(&bi) {
            return Ok(Value::from_int(ai >> bi));
        }
    }
    Err(VMError::TypeError(
        "unsupported operand types for >>".into(),
    ))
}

// --- Comparison operations ---

#[inline]
fn compare_lt(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai < bi));
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af < bf));
    }
    Err(VMError::TypeError("'<' not supported".into()))
}

#[inline]
fn compare_le(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai <= bi));
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af <= bf));
    }
    Err(VMError::TypeError("'<=' not supported".into()))
}

#[inline]
fn compare_gt(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai > bi));
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af > bf));
    }
    Err(VMError::TypeError("'>' not supported".into()))
}

#[inline]
fn compare_ge(a: Value, b: Value) -> VMResult<Value> {
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai >= bi));
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af >= bf));
    }
    Err(VMError::TypeError("'>=' not supported".into()))
}

#[inline]
fn compare_eq(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    // For primitive types (int, float, bool, nil), compare bits
    if a.is_nil() || b.is_nil() {
        return Ok(Value::from_bool(a.is_nil() && b.is_nil()));
    }
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai == bi));
    }
    if let (Some(ab), Some(bb)) = (a.as_bool(), b.as_bool()) {
        return Ok(Value::from_bool(ab == bb));
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af == bf));
    }
    // For PyObjects (lists, strings, etc.), delegate to Python's ==
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let result = py_a.bind(py).eq(&py_b)?;
    Ok(Value::from_bool(result))
}

#[inline]
fn compare_ne(py: Python<'_>, a: Value, b: Value) -> VMResult<Value> {
    // For primitive types (int, float, bool, nil), compare bits
    if a.is_nil() || b.is_nil() {
        return Ok(Value::from_bool(!(a.is_nil() && b.is_nil())));
    }
    if let (Some(ai), Some(bi)) = (a.as_int(), b.as_int()) {
        return Ok(Value::from_bool(ai != bi));
    }
    if let (Some(ab), Some(bb)) = (a.as_bool(), b.as_bool()) {
        return Ok(Value::from_bool(ab != bb));
    }
    let af = a.as_float().or_else(|| a.as_int().map(|i| i as f64));
    let bf = b.as_float().or_else(|| b.as_int().map(|i| i as f64));
    if let (Some(af), Some(bf)) = (af, bf) {
        return Ok(Value::from_bool(af != bf));
    }
    // For PyObjects, delegate to Python's !=
    let py_a = a.to_pyobject(py);
    let py_b = b.to_pyobject(py);
    let result = py_a.bind(py).ne(&py_b)?;
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
            // Fast path: NaN-boxed comparison for int/float/bool/nil
            if value.bits() == expected.bits() {
                return Ok(Some(Vec::new()));
            }
            // Fast path for ints with different bits (shouldn't happen but safety)
            if let (Some(a), Some(b)) = (value.as_int(), expected.as_int()) {
                return if a == b {
                    Ok(Some(Vec::new()))
                } else {
                    Ok(None)
                };
            }
            // Fallback: Python equality for strings, PyObj, etc.
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
            } else {
                let star_pos = star_idx.unwrap();
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
                        let star_items: Vec<Py<PyAny>> = items[n_before..after_start]
                            .iter()
                            .map(|v| v.to_pyobject(py))
                            .collect();
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

            // Check type name matches
            let value_type_name: String = py_bound.get_type().name()?.extract()?;
            if value_type_name != *name {
                return Ok(None);
            }

            // Extract field values as bindings (missing field = no match)
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
                Instruction::new(OpCode::LoadLocal, 0), // 4: i
                Instruction::new(OpCode::LoadConst, 1), // 5: limit
                Instruction::simple(OpCode::Lt),        // 6: i < 5
                Instruction::new(OpCode::JumpIfFalse, 17), // 7: Exit to ip=17 if false
                // sum = sum + i
                Instruction::new(OpCode::LoadLocal, 1), // 8: sum
                Instruction::new(OpCode::LoadLocal, 0), // 9: i
                Instruction::simple(OpCode::Add),       // 10: sum + i
                Instruction::new(OpCode::StoreLocal, 1), // 11: Store sum
                // i = i + 1
                Instruction::new(OpCode::LoadLocal, 0), // 12: i
                Instruction::new(OpCode::LoadConst, 2), // 13: 1
                Instruction::simple(OpCode::Add),       // 14: i + 1
                Instruction::new(OpCode::StoreLocal, 0), // 15: Store i
                Instruction::new(OpCode::Jump, 4),      // 16: Loop back to ip=4
                // exit (ip=17)
                Instruction::new(OpCode::LoadLocal, 1), // 17: Return sum
                Instruction::simple(OpCode::Halt),      // 18: Halt
            ];

            let mut vm = VM::new();
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
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
            let result = vm.execute(py, code, &[]).unwrap();
            assert_eq!(result.as_int(), Some(20));
        });
    }

    // --- Native struct tests ---

    /// Helper: register a struct type and map a Python object's pointer to it.
    /// Returns (py_obj_value, type_id) where py_obj_value can be used as the callable.
    fn register_test_struct(
        py: Python<'_>,
        vm: &mut VM,
        name: &str,
        fields: Vec<StructField>,
    ) -> (Value, StructTypeId) {
        let type_id = vm.struct_registry.register_type(
            name.into(),
            fields,
            HashMap::new(),
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

            let result = vm.execute(py, code, &[]).unwrap();
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

            let result = vm.execute(py, code, &[]).unwrap();
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

            let err = vm.execute(py, code, &[]).unwrap_err();
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
            code.constants = vec![
                struct_val,
                Value::from_int(1),
                Value::from_int(2),
                Value::from_int(3),
            ];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::LoadConst, 1),
                Instruction::new(OpCode::LoadConst, 2),
                Instruction::new(OpCode::LoadConst, 3),
                Instruction::new(OpCode::Call, 3), // 3 args for 2 fields
                Instruction::simple(OpCode::Halt),
            ];

            let err = vm.execute(py, code, &[]).unwrap_err();
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
            let kw_names = PyTuple::new(py, &["y"]).unwrap();
            let kw_names_val = Value::from_pyobject(py, kw_names.as_any()).unwrap();

            let mut code = CodeObject::new("test_struct_callkw");
            code.constants = vec![
                struct_val,
                Value::from_int(10),
                Value::from_int(20),
                kw_names_val,
            ];
            code.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),         // struct type
                Instruction::new(OpCode::LoadConst, 1),         // x=10 (positional)
                Instruction::new(OpCode::LoadConst, 2),         // y=20 (kw value)
                Instruction::new(OpCode::LoadConst, 3),         // kw_names ("y",)
                Instruction::new(OpCode::CallKw, (1 << 8) | 1), // nargs=1, nkw=1
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, code, &[]).unwrap();
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

        let result = vm.execute(py, code, &[]).unwrap();
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

            let result = vm.execute(py, code, &[]).unwrap();
            assert_eq!(result.as_int(), Some(10));

            let mut code2 = CodeObject::new("test_getattr_y");
            code2.constants = vec![instance];
            code2.names = vec!["y".into()];
            code2.instructions = vec![
                Instruction::new(OpCode::LoadConst, 0),
                Instruction::new(OpCode::GetAttr, 0), // .y
                Instruction::simple(OpCode::Halt),
            ];

            let result = vm.execute(py, code2, &[]).unwrap();
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

            let err = vm.execute(py, code, &[]).unwrap_err();
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

            let result = vm.execute(py, code, &[]).unwrap();
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

            let result = vm.execute(py, code, &[]).unwrap();
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

            let result = vm.execute(py, code, &[]).unwrap();
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

            let result = vm.execute(py, code, &[]).unwrap();
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

            let result = vm.execute(py, code, &[]).unwrap();
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

            let result = vm.execute(py, code2, &[]).unwrap();
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

            let result = vm.execute(py, code, &[]).unwrap();
            assert!(
                result.is_nil(),
                "expected NIL for type mismatch, got {:?}",
                result
            );
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

            let result = vm.execute(py, code, &[]).unwrap();
            assert!(
                result.is_nil(),
                "expected NIL for unknown field, got {:?}",
                result
            );
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
                HashMap::new(),
                vec![],               // implements
                vec!["Point".into()], // mro
            );

            // Create a native instance
            let idx = vm
                .struct_registry
                .create_instance(type_id, vec![Value::from_int(10), Value::from_int(20)]);
            let struct_val = Value::from_struct_instance(idx);

            // Install registry for to_pyobject
            crate::vm::value::set_struct_registry(&vm.struct_registry as *const _);

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
        });
    }
}
