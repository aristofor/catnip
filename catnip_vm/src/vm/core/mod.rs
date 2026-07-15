// FILE: catnip_vm/src/vm/core.rs
//! PureVM dispatch loop -- pure Rust, no PyO3.
//!
//! Executes bytecode (CodeObject compiled by PureCompiler) using VmHost
//! for external operations. All values are native catnip_vm::Value.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use catnip_core::constants::MEMORY_CHECK_INTERVAL;
use catnip_core::vm::opcode::VMOpCode;
use catnip_core::vm::{
    CALL_ARGS_MASK, CALL_ARGS_SHIFT, FOR_RANGE_JUMP_MASK, FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_SLOT_MASK,
    FOR_RANGE_SLOT_STOP_SHIFT, FOR_RANGE_STEP_BYTE_MASK, FOR_RANGE_STEP_JUMP_MASK, FOR_RANGE_STEP_SHIFT,
    FOR_RANGE_STEP_SIGN_SHIFT,
};
use indexmap::IndexMap;

use crate::collections::{KeyCtx, ValueKey};
use crate::compiler::code_object::CodeObject;
use crate::compiler::pattern::{VMPattern, VMPatternElement};
use crate::error::{HofCall, HofKind, VMError, VMResult};
use crate::host::VmHost;
use crate::ops::arith;
use crate::ops::errors;
use crate::value::Value;

use super::closure::{PureClosureParent, PureClosureScope};
use super::debug::{DebugHook, DebugState};
use super::enums::PureEnumRegistry;
use super::frame::PureFramePool;
use super::func_table::{PureFuncSlot, PureFunctionTable};
use super::structs::{PureStructField, PureStructRegistry, PureStructType, PureTraitRegistry, StructCell};
use catnip_core::symbols::{SymbolTable, qualified_name};

/// Maximum call depth to prevent infinite recursion.
const MAX_FRAME_DEPTH: usize = 512;

/// Best-effort runtime type name of a primitive, for boundary-check errors.
fn primitive_type_name(v: Value) -> &'static str {
    if v.is_bool() {
        "bool"
    } else if v.is_int() || v.is_bigint() {
        "int"
    } else if v.is_float() {
        "float"
    } else if v.is_native_str() {
        "str"
    } else if v.is_nil() {
        "None"
    } else {
        "value"
    }
}

/// TH2-B step 0b boundary check + numeric-tower coercion for `CheckType`.
///
/// `code` is a `type_code::*` naming the declared param type. A value already of
/// that type passes through; a numeric-tower widening (`int`/`bool` → `float`,
/// `bool` → `int`, `bigint` → `float`) is coerced to the declared type; anything
/// else is a `TypeError`. Enforces `int`/`float`/`str`/`bool`/`None`; `str` has
/// no widening (only a `str` passes). Kept symmetric with the PyO3 VM.
fn boundary_coerce(val: Value, code: u8) -> VMResult<Value> {
    use catnip_core::vm::opcode::type_code;
    let mismatch = || {
        VMError::TypeError(format!(
            "typed parameter expects '{}' but got '{}'",
            type_code::name(code),
            primitive_type_name(val)
        ))
    };
    // Scalar arms (int/float/bool/None, numeric-tower widening) are shared
    // with the twin VM: catnip_core::arith::coerce_scalar. Only the mismatch
    // message (per-crate primitive_type_name) and the heap codes stay here.
    use catnip_core::arith::ScalarCoerce;
    match catnip_core::arith::coerce_scalar(val, code) {
        ScalarCoerce::Ok(v) => return Ok(v),
        ScalarCoerce::HugeInt => {
            return Err(VMError::TypeError("int too large to convert to float".to_string()));
        }
        ScalarCoerce::Mismatch => return Err(mismatch()),
        ScalarCoerce::Unhandled => {}
    }
    match code {
        type_code::STR => {
            if val.is_native_str() {
                Ok(val)
            } else {
                Err(mismatch())
            }
        }
        // Composites are enforced at the constructor level (params ignored), no
        // coercion: only a list satisfies `list`, only a dict satisfies `dict`.
        type_code::LIST => {
            if val.is_native_list() {
                Ok(val)
            } else {
                Err(mismatch())
            }
        }
        type_code::DICT => {
            if val.is_native_dict() {
                Ok(val)
            } else {
                Err(mismatch())
            }
        }
        _ => Ok(val),
    }
}

/// Classify a NaN-box `Value` into its [`PrimitiveClass`] for the shared union
/// membership test ([`catnip_core::vm::opcode::primitive_membership`]). The
/// numeric tower lives in core; this only maps the value's tags. No coercion.
fn value_primitive_class(val: Value) -> catnip_core::vm::opcode::PrimitiveClass {
    catnip_core::vm::opcode::PrimitiveClass {
        int_like: val.is_int() || val.is_bigint(),
        float_like: val.is_float(),
        str_like: val.is_native_str(),
        bool_like: val.as_bool().is_some(),
        nil_like: val.is_nil(),
        list_like: val.is_native_list(),
        set_like: val.is_native_set(),
        dict_like: val.is_native_dict(),
        tuple_like: val.is_native_tuple(),
    }
}

// ---------------------------------------------------------------------------
// PureVM
// ---------------------------------------------------------------------------

/// Pure Rust VM that executes bytecode without Python dependency.
pub struct PureVM {
    /// Stack of execution frames (call stack).
    pub(crate) frame_stack: Vec<PureFrame>,
    /// Frame pool for allocation reuse.
    pub(crate) frame_pool: PureFramePool,
    /// Function table (grow-only, VMFunc index → code+closure).
    pub(crate) func_table: PureFunctionTable,
    /// Total instructions executed (for interrupt checks).
    instruction_count: u64,
    /// External interrupt flag (Ctrl-C).
    interrupt_flag: Arc<AtomicBool>,
    /// Debug state (hook, breakpoints, stepping).
    debug: DebugState,
    /// Struct type and instance registry.
    pub(crate) struct_registry: PureStructRegistry,
    /// Trait definition registry.
    pub(crate) trait_registry: PureTraitRegistry,
    /// Enum type registry.
    pub(crate) enum_registry: PureEnumRegistry,
    /// Symbol interning table (used by enums).
    pub(crate) symbol_table: SymbolTable,
    /// Enum type name → type_id mapping for GetAttr dispatch.
    pub(crate) enum_type_names: HashMap<String, u32>,
    /// Union methods for nullary variants, keyed by the variant's symbol id.
    /// Payload variants carry the same methods on their struct type instead.
    pub(crate) union_nullary_methods: HashMap<u32, std::rc::Rc<IndexMap<String, u32>>>,
    /// ND lambda stack for recursive ND operations (~~). Stores the active ND
    /// lambda *values* (a runtime closure or a template VMFunc), each holding a
    /// strong ref while on the stack so a closure lambda survives the re-entrant
    /// `recur` boundary. Push clone_refcounts, pop decrefs.
    pub(crate) nd_lambda_stack: Vec<Value>,
    /// Optional import loader for .cat file imports.
    pub(crate) import_loader: Option<crate::loader::PureImportLoader>,
    /// Weak handles to runtime closures that received a strong `PatchClosure`
    /// capture (letrec mutual recursion) -- the only site a runtime closure can
    /// enter an Arc cycle (self-references are weak, forward captures are
    /// acyclic). `Drop for PureVM` upgrades and clears their captured maps so the
    /// cycle breaks and the slots are reclaimed at reset rather than leaked past
    /// the VM's life. Weak so this registry itself pins nothing.
    pub(crate) runtime_closures: Vec<std::sync::Weak<PureFuncSlot>>,
    /// Whether `Drop` runs the letrec cycle-breaking drain. True for the main
    /// pipeline VM (reset tears its closures down). False for a child module VM:
    /// its closures escape into the parent (module namespace / `module_globals`
    /// held after the child drops), so clearing their captured maps would break
    /// an exported letrec closure. A child's cyclic closures are retained by
    /// `module_globals` regardless, so skipping the drain leaks nothing extra.
    pub(crate) drain_closures_on_drop: bool,
}

// Re-use PureFrame directly to avoid circular import
use super::frame::PureFrame;

impl Drop for PureVM {
    /// Break letrec mutual-recursion cycles before the VM's runtime closures are
    /// released, so cyclic slots are reclaimed rather than leaked past the VM's
    /// life (parity with the old whole-table drop at reset). Self-recursion uses a
    /// weak self-ref (no cycle); only strong `PatchClosure` captures can cycle,
    /// and those closures were registered in `runtime_closures`. Clearing their
    /// captured maps drops the mutual strong refs; whatever still holds a closure
    /// (globals, cleared just after this drop) then brings it to zero.
    fn drop(&mut self) {
        // A child module VM's closures escape into the parent; draining would
        // break an exported letrec closure (see `drain_closures_on_drop`).
        if !self.drain_closures_on_drop {
            return;
        }
        for weak in self.runtime_closures.drain(..) {
            if let Some(slot) = weak.upgrade() {
                // Never clear a bound method's closure -- it is shared with the
                // method template (`Rc` clone); only MakeFunction closures are
                // unique to their runtime slot.
                if slot.bound_self.is_none() {
                    if let Some(cs) = &slot.closure {
                        cs.clear_captured();
                    }
                }
            }
        }
    }
}

impl PureVM {
    /// Create a new PureVM.
    pub fn new() -> Self {
        Self {
            frame_stack: Vec::with_capacity(32),
            frame_pool: PureFramePool::default(),
            func_table: PureFunctionTable::new(),
            instruction_count: 0,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            debug: DebugState::new(),
            struct_registry: PureStructRegistry::new(),
            trait_registry: PureTraitRegistry::new(),
            enum_registry: PureEnumRegistry::new(),
            symbol_table: SymbolTable::new(),
            enum_type_names: HashMap::new(),
            union_nullary_methods: HashMap::new(),
            nd_lambda_stack: Vec::new(),
            import_loader: None,
            runtime_closures: Vec::new(),
            drain_closures_on_drop: true,
        }
    }

    /// Create a PureVM with a shared interrupt flag.
    pub fn with_interrupt(interrupt: Arc<AtomicBool>) -> Self {
        let mut vm = Self::new();
        vm.interrupt_flag = interrupt;
        vm
    }

    /// Resolve a callable `Value` to its slot data, accepting both a runtime
    /// closure (`TAG_CLOSURE`, Arc-backed) and a template function (`TAG_VMFUNC`,
    /// index into the grow-only table). Returns `(code, closure, bound_self)` --
    /// all independent refs the caller owns (code/closure cloned; `bound_self` is
    /// a refcount-neutral copy the caller must `clone_refcount` before binding, as
    /// `with_bound_receiver` does). `None` for a non-callable value.
    #[inline]
    pub(crate) fn callable_slot_data(
        &self,
        func: Value,
    ) -> Option<(Arc<CodeObject>, Option<PureClosureScope>, Option<Value>)> {
        if func.is_closure() {
            // SAFETY: is_closure() proves a live Arc<PureFuncSlot>; we clone out of
            // the borrow before it ends, and `func` still owns its ref here.
            let s = unsafe { func.as_closure_ref()? };
            Some((Arc::clone(&s.code), s.closure.clone(), s.bound_self))
        } else if func.is_vmfunc() && !func.is_invalid() {
            let s = self.func_table.get(func.as_vmfunc_idx())?;
            Some((Arc::clone(&s.code), s.closure.clone(), s.bound_self))
        } else {
            None
        }
    }

    /// Handle an import() call. Takes the loader temporarily to avoid borrow conflict.
    /// kwargs supports `protocol` (str) and `wild` (bool).
    fn handle_import(&mut self, args: &[Value], kwargs: &[(String, Value)], host: &dyn VmHost) -> VMResult<Value> {
        let spec_val = args
            .first()
            .ok_or_else(|| VMError::TypeError("import() takes at least 1 argument (0 given)".into()))?;
        if !spec_val.is_native_str() {
            return Err(VMError::TypeError("import() argument must be a string".into()));
        }
        // SAFETY: the is_native_str() check above returns early otherwise, so spec_val holds a live Arc<NativeString> owned on the stack; the borrow does not outlive it.
        let spec = unsafe { spec_val.as_native_str_ref().unwrap() };

        // Parse kwargs
        let mut protocol: Option<String> = None;
        let mut wild = false;
        for (name, val) in kwargs {
            match name.as_str() {
                "protocol" => {
                    if !val.is_native_str() {
                        return Err(VMError::TypeError("protocol must be a string".into()));
                    }
                    // SAFETY: the is_native_str() check just above returns early otherwise, so val holds a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
                    protocol = Some(unsafe { val.as_native_str_ref().unwrap() }.to_string());
                }
                "wild" => {
                    wild = val.is_truthy();
                }
                _ => {
                    return Err(VMError::TypeError(format!(
                        "import() got unexpected keyword argument '{}'",
                        name
                    )));
                }
            }
        }

        // Parse selective import names from positional args beyond the first
        let mut names = Vec::new();
        for arg in &args[1..] {
            if !arg.is_native_str() {
                return Err(VMError::TypeError("import() selective names must be strings".into()));
            }
            // SAFETY: the is_native_str() check just above returns early otherwise, so arg holds a live Arc<NativeString> owned by the caller; the borrow does not outlive it.
            let raw = unsafe { arg.as_native_str_ref().unwrap() }.to_string();
            names.push(crate::loader::parse_import_name(&raw)?);
        }

        let params = crate::loader::ImportParams {
            spec,
            names,
            wild,
            protocol: protocol.as_deref(),
        };

        // Take the loader out to avoid &mut self aliasing
        let loader = self.import_loader.take().unwrap();
        let result = loader.load_with_params(params, self);
        self.import_loader = Some(loader);

        match result? {
            crate::loader::ImportResult::Namespace(val) => Ok(val),
            crate::loader::ImportResult::Injected(pairs) => {
                for (name, val) in pairs {
                    host.store_global(&name, val);
                }
                Ok(Value::NIL)
            }
        }
    }

    /// Set a debug hook. The VM will call it on breakpoints and steps.
    pub fn set_debug_hook(&mut self, hook: Box<dyn DebugHook>) {
        self.debug.hook = Some(hook);
    }

    /// Add a breakpoint at a source line (1-indexed).
    pub fn add_breakpoint(&mut self, line: usize) {
        self.debug.breakpoint_lines.lock().unwrap().insert(line);
    }

    /// Remove a breakpoint at a source line (1-indexed).
    pub fn remove_breakpoint(&mut self, line: usize) {
        self.debug.breakpoint_lines.lock().unwrap().remove(&line);
    }

    /// Get a shared handle to the breakpoint set for external modification.
    pub fn breakpoint_lines_handle(&self) -> std::sync::Arc<std::sync::Mutex<std::collections::HashSet<usize>>> {
        self.debug.breakpoint_lines.clone()
    }

    /// Set source bytes for debug line resolution.
    pub fn set_source(&mut self, source: &[u8]) {
        self.debug.source_bytes = Some(source.to_vec());
    }

    /// Execute a CodeObject with the given arguments.
    pub fn execute(&mut self, code: Arc<CodeObject>, args: &[Value], host: &dyn VmHost) -> VMResult<Value> {
        let mut frame = self.frame_pool.alloc_with_code(Arc::clone(&code));
        frame.bind_args(args);
        self.dispatch(frame, host)
    }

    /// Execute a CompileOutput (main code + sub-functions).
    ///
    /// Pre-registers all sub-functions in the function table so that
    /// VMFunc indices in constants (emitted by PureCompiler) are valid.
    pub fn execute_output(
        &mut self,
        output: &crate::compiler::CompileOutput,
        args: &[Value],
        host: &dyn VmHost,
    ) -> VMResult<Value> {
        // Pre-register sub-functions. PureCompiler stores them in output.functions
        // and references them via Value::from_vmfunc(idx) in the constant pool,
        // where idx is the position in output.functions (0, 1, 2...).
        // When func_table already has entries, we must remap these indices.
        let base = self.func_table.len() as u32;
        for func in &output.functions {
            self.func_table
                .insert(PureFuncSlot::template(Arc::new(func.clone()), None));
        }

        // Remap VMFunc indices in the constant pool if needed
        let mut code = output.code.clone();
        if base > 0 && !output.functions.is_empty() {
            for c in &mut code.constants {
                if c.is_vmfunc() && !c.is_invalid() {
                    let old_idx = c.as_vmfunc_idx();
                    if (old_idx as usize) < output.functions.len() {
                        *c = Value::from_vmfunc(old_idx + base);
                    }
                }
            }
        }
        let code = Arc::new(code);
        self.execute(code, args, host)
    }

    /// Main dispatch loop with exception unwinding.
    ///
    /// Records `base_depth` so that re-entrant calls (from HOF builtins)
    /// never pop frames belonging to an outer dispatch invocation.
    pub(crate) fn dispatch(&mut self, mut frame: PureFrame, host: &dyn VmHost) -> VMResult<Value> {
        let base_depth = self.frame_stack.len();
        let mut iterators: Vec<Option<Box<dyn crate::host::ValueIter>>> = Vec::new();
        'outer: loop {
            match self.dispatch_inner(&mut frame, host, &mut iterators, base_depth) {
                Ok(val) => {
                    self.frame_pool.free(frame);
                    return Ok(val);
                }
                Err(err) => {
                    // HOF builtin signal: execute synchronously, then continue
                    let err = match err {
                        VMError::HofBuiltin(hof) => match self.execute_hof(hof, host) {
                            Ok(result) => {
                                frame.push(result);
                                continue 'outer;
                            }
                            Err(e) => e,
                        },
                        other => other,
                    };
                    // Try to unwind to an exception handler
                    if self.unwind_exception(&mut frame, &err, base_depth) {
                        continue 'outer;
                    }
                    // No handler found. Handle Return specially (frame pop).
                    if let VMError::Return(val) = err {
                        if self.frame_stack.len() > base_depth {
                            let caller = self.frame_stack.pop().unwrap();
                            let discard = frame.discard_return;
                            let old = std::mem::replace(&mut frame, caller);
                            self.frame_pool.free(old);
                            if discard {
                                val.decref();
                            } else {
                                frame.push(val);
                            }
                            continue 'outer;
                        }
                        self.frame_pool.free(frame);
                        return Ok(val);
                    }
                    // Clean up frames above base_depth
                    while self.frame_stack.len() > base_depth {
                        self.frame_pool.free(self.frame_stack.pop().unwrap());
                    }
                    self.frame_pool.free(frame);
                    return Err(err);
                }
            }
        }
    }

    /// Inner dispatch loop. Returns Ok on clean exit, Err on any signal/exception.
    fn dispatch_inner(
        &mut self,
        frame: &mut PureFrame,
        host: &dyn VmHost,
        iterators: &mut Vec<Option<Box<dyn crate::host::ValueIter>>>,
        base_depth: usize,
    ) -> VMResult<Value> {
        loop {
            let code = match &frame.code {
                Some(c) => Arc::clone(c),
                None => return Err(VMError::RuntimeError("frame has no code".into())),
            };
            let instructions = &code.instructions;

            if frame.ip >= instructions.len() {
                // End of code: return top of stack or NIL
                let result = if !frame.stack.is_empty() {
                    frame.pop()
                } else {
                    Value::NIL
                };
                // Pop frame from call stack (only above base_depth)
                if self.frame_stack.len() > base_depth {
                    let caller = self.frame_stack.pop().unwrap();
                    let discard = frame.discard_return;
                    let old = std::mem::replace(frame, caller);
                    self.frame_pool.free(old);
                    if discard {
                        result.decref();
                    } else {
                        frame.push(result);
                    }
                    continue;
                }
                return Ok(result);
            }

            let instr = instructions[frame.ip];
            let instr_idx = frame.ip;
            frame.ip += 1;

            // Debug: check breakpoints and stepping
            if self.debug.is_active() {
                let depth = self.frame_stack.len() + 1;
                if self.debug.stepping {
                    self.debug.check_step(frame, &code, instr_idx, depth);
                } else if self.debug.is_breakpoint(&code, instr_idx) {
                    let src_byte = code.line_table.get(instr_idx).copied().unwrap_or(u32::MAX);
                    if src_byte != self.debug.last_pause_byte {
                        self.debug.pause(frame, &code, instr_idx, depth, true);
                    }
                }
            }

            // Periodic interrupt check
            self.instruction_count += 1;
            if self.instruction_count & MEMORY_CHECK_INTERVAL == 0 && self.interrupt_flag.load(Ordering::Relaxed) {
                return Err(VMError::Interrupted);
            }

            match instr.op {
                // =============================================================
                // Tier 1: Stack operations
                // =============================================================
                VMOpCode::LoadConst => {
                    let val = code.constants[instr.arg as usize];
                    val.clone_refcount();
                    frame.push(val);
                }
                VMOpCode::LoadLocal => {
                    let val = frame.get_local(instr.arg as usize);
                    val.clone_refcount();
                    frame.push(val);
                }
                VMOpCode::StoreLocal => {
                    let val = frame.pop();
                    let slot = instr.arg as usize;
                    let old = frame.get_local(slot);
                    old.decref();
                    frame.set_local(slot, val);
                }
                VMOpCode::PopTop => {
                    let val = frame.pop();
                    val.decref();
                }
                VMOpCode::DupTop => {
                    let val = frame.peek();
                    val.clone_refcount();
                    frame.push(val);
                }
                VMOpCode::RotTwo => {
                    let a = frame.pop();
                    let b = frame.pop();
                    frame.push(a);
                    frame.push(b);
                }

                // =============================================================
                // Tier 1: Jumps
                // =============================================================
                VMOpCode::Jump => {
                    frame.ip = instr.arg as usize;
                }
                VMOpCode::JumpIfFalse => {
                    let val = frame.pop();
                    if !val.is_truthy() {
                        frame.ip = instr.arg as usize;
                    }
                }
                VMOpCode::JumpIfTrue => {
                    let val = frame.pop();
                    if val.is_truthy() {
                        frame.ip = instr.arg as usize;
                    }
                }
                VMOpCode::JumpIfFalseOrPop => {
                    let val = frame.peek();
                    if !val.is_truthy() {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                    }
                }
                VMOpCode::JumpIfTrueOrPop => {
                    let val = frame.peek();
                    if val.is_truthy() {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                    }
                }
                VMOpCode::JumpIfNone => {
                    let val = frame.pop();
                    if val.is_nil() {
                        frame.ip = instr.arg as usize;
                    }
                }
                VMOpCode::JumpIfNotNoneOrPop => {
                    let val = frame.peek();
                    if !val.is_nil() {
                        frame.ip = instr.arg as usize;
                    } else {
                        frame.pop();
                    }
                }

                // =============================================================
                // Tier 1: Blocks (scope isolation)
                // =============================================================
                VMOpCode::PushBlock => {
                    // Bit 31 = module-level block (no scope isolation needed)
                    let is_module = (instr.arg & 0x8000_0000) != 0;
                    if !is_module {
                        let slot_start = (instr.arg & 0x7FFF_FFFF) as usize;
                        frame.push_block(slot_start);
                    }
                }
                VMOpCode::PopBlock => {
                    frame.pop_block();
                }

                // =============================================================
                // Tier 1: Control flow signals
                // =============================================================
                VMOpCode::Break => {
                    let err = VMError::Break;
                    if self.try_unwind_to_handler(frame, &err) {
                        continue;
                    }
                    return Err(err);
                }
                VMOpCode::Continue => {
                    let err = VMError::Continue;
                    if self.try_unwind_to_handler(frame, &err) {
                        continue;
                    }
                    return Err(err);
                }
                VMOpCode::Return => {
                    let val = if !frame.stack.is_empty() {
                        frame.pop()
                    } else {
                        Value::NIL
                    };

                    // If handler stack has Finally, handle inline
                    if !frame.handler_stack.is_empty() {
                        let err = VMError::Return(val);
                        if self.try_unwind_to_handler(frame, &err) {
                            continue;
                        }
                        // No Finally handler, recover value and fall through
                        if let VMError::Return(v) = err {
                            frame.push(v);
                        }
                    }

                    // Fast path: no handlers, direct frame switching
                    if self.frame_stack.len() > base_depth {
                        let caller = self.frame_stack.pop().unwrap();
                        let discard = frame.discard_return;
                        let old = std::mem::replace(frame, caller);
                        self.frame_pool.free(old);
                        // Back at the module top-level: re-sync the module's local slots
                        // from the globals so a global mutated by the callee (e.g. a
                        // closure bumping a module counter) is visible when the module
                        // reads the name back via LoadLocal. Module frame only (empty call
                        // stack); syncing a function frame would clobber its locals with
                        // stale globals. Mirrors catnip_rs.
                        if self.frame_stack.is_empty() {
                            self.sync_module_slots_from_globals(frame, host);
                        }
                        if discard {
                            val.decref();
                        } else {
                            frame.push(val);
                        }
                        continue;
                    }
                    return Ok(val);
                }
                VMOpCode::Halt => {
                    let val = if !frame.stack.is_empty() {
                        frame.pop()
                    } else {
                        Value::NIL
                    };
                    return Ok(val);
                }
                VMOpCode::Exit => {
                    let code = if !frame.stack.is_empty() {
                        frame.pop().as_int().unwrap_or(0) as i32
                    } else {
                        0
                    };
                    return Err(VMError::Exit(code));
                }
                VMOpCode::Nop => {}

                // =============================================================
                // Tier 1: Boolean
                // =============================================================
                VMOpCode::ToBool => {
                    let val = frame.pop();
                    frame.push(Value::from_bool(val.is_truthy()));
                }
                VMOpCode::Not => {
                    let val = frame.pop();
                    frame.push(Value::from_bool(!val.is_truthy()));
                }

                // Typed-parameter boundary (TH2-B step 0b): check + numeric-tower
                // coercion so an annotated param IS its declared type before any
                // specialized opcode reads it. On a type error the popped operand
                // is dropped, so release its refcount before unwinding.
                VMOpCode::CheckType => {
                    let val = frame.pop();
                    match boundary_coerce(val, instr.arg as u8) {
                        Ok(coerced) => frame.push(coerced),
                        Err(e) => {
                            val.decref();
                            return Err(e);
                        }
                    }
                }

                // Nominal-type boundary (enforcement nominal): an annotated param
                // must be an instance of its declared struct/enum/union type, with
                // subtyping (MRO + traits). No coercion; an unknown type name is
                // inert. On a type error the popped value is dropped, so release
                // its refcount before unwinding.
                VMOpCode::CheckNominal => {
                    let name = &code.names[instr.arg as usize];
                    let val = frame.pop();
                    match self.check_nominal(val, name, host) {
                        Ok(()) => frame.push(val),
                        Err(e) => {
                            val.decref();
                            return Err(e);
                        }
                    }
                }

                // Type-union boundary (`int | str`, `Point | None`): accept if the
                // value satisfies any member (no coercion). On a type error the
                // popped value is dropped, so release its refcount before unwinding.
                VMOpCode::CheckUnion => {
                    let members = &code.union_checks[instr.arg as usize];
                    let val = frame.pop();
                    match self.check_union(val, members, host) {
                        Ok(()) => frame.push(val),
                        Err(e) => {
                            val.decref();
                            return Err(e);
                        }
                    }
                }

                // Composite boundary (`list[T]`, `dict[K, V]`): check the container
                // tag (the type parameters are carried but not yet enforced). On a
                // type error the popped value is dropped, so release its refcount.
                VMOpCode::CheckComposite => {
                    let spec = &code.composite_checks[instr.arg as usize];
                    let val = frame.pop();
                    match self.check_composite(val, spec, host) {
                        Ok(()) => frame.push(val),
                        Err(e) => {
                            val.decref();
                            return Err(e);
                        }
                    }
                }

                // Generic-nominal boundary (`Option[int]`): check union membership
                // and the parametric payload substitution. On a type error the
                // popped value is dropped, so release its refcount before unwinding.
                VMOpCode::CheckGeneric => {
                    let spec = &code.generic_checks[instr.arg as usize];
                    let val = frame.pop();
                    match self.check_generic(val, spec, host) {
                        Ok(()) => frame.push(val),
                        Err(e) => {
                            val.decref();
                            return Err(e);
                        }
                    }
                }

                // Function-type boundary (`(int) -> int`, FT3): the value must
                // be callable and, when introspectable, accept the declared
                // arity. `instr.arg` IS the arity (no side table).
                VMOpCode::CheckCallable => {
                    let val = frame.pop();
                    match self.check_callable(val, instr.arg, host) {
                        Ok(()) => frame.push(val),
                        Err(e) => {
                            val.decref();
                            return Err(e);
                        }
                    }
                }

                // =============================================================
                // Tier 1: Optimized range loops
                // =============================================================
                VMOpCode::ForRangeInt => {
                    let slot_i = (instr.arg >> FOR_RANGE_SLOT_I_SHIFT) as usize;
                    let slot_stop = ((instr.arg >> FOR_RANGE_SLOT_STOP_SHIFT) & FOR_RANGE_SLOT_MASK) as usize;
                    let step_positive = ((instr.arg >> FOR_RANGE_STEP_SIGN_SHIFT) & 1) == 0;
                    let jump_offset = (instr.arg & FOR_RANGE_JUMP_MASK) as usize;

                    let i = frame.get_local(slot_i);
                    let stop = frame.get_local(slot_stop);

                    let done = match (i.as_int(), stop.as_int()) {
                        (Some(i_val), Some(stop_val)) => {
                            if step_positive {
                                i_val >= stop_val
                            } else {
                                i_val <= stop_val
                            }
                        }
                        _ => true,
                    };

                    if done {
                        frame.ip += jump_offset;
                    }
                }
                VMOpCode::ForRangeStep => {
                    let slot_i = (instr.arg >> FOR_RANGE_SLOT_I_SHIFT) as usize;
                    let step = ((instr.arg >> FOR_RANGE_STEP_SHIFT) & FOR_RANGE_STEP_BYTE_MASK) as i8 as i64;
                    let jump_target = (instr.arg & FOR_RANGE_STEP_JUMP_MASK) as usize;

                    let i_val = frame.get_local(slot_i).as_int().unwrap_or(0);
                    frame.set_local(slot_i, Value::from_int(i_val + step));
                    frame.ip = jump_target;
                }

                // =============================================================
                // Tier 2: Arithmetic (inline fast path + host fallback)
                // =============================================================
                VMOpCode::Add => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_add", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Add, a, b);
                        a.decref();
                        b.decref();
                        frame.push(result?);
                    }
                }
                // TH4 canal A: typed arithmetic. Operands are a proven int/float
                // runtime fact, so skip the struct-overload lookup and the type
                // dispatch. AddInt reuses the generic integer add (overflow ->
                // bigint); AddFloat adds directly, falling back if off-type.
                VMOpCode::AddInt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = arith::numeric_add(a, b);
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::AddFloat => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match (a.as_float(), b.as_float()) {
                        (Some(x), Some(y)) => Ok(Value::from_float(x + y)),
                        _ => arith::numeric_add(a, b),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::SubInt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = arith::numeric_sub(a, b);
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::SubFloat => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match (a.as_float(), b.as_float()) {
                        (Some(x), Some(y)) => Ok(Value::from_float(x - y)),
                        _ => arith::numeric_sub(a, b),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::MulInt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = arith::numeric_mul(a, b);
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::MulFloat => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match (a.as_float(), b.as_float()) {
                        (Some(x), Some(y)) => Ok(Value::from_float(x * y)),
                        _ => arith::numeric_mul(a, b),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                // True division always yields a float; the fast path divides
                // directly but defers the zero check to numeric_div.
                VMOpCode::DivFloat => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = match (a.as_float(), b.as_float()) {
                        (Some(x), Some(y)) if y != 0.0 => Ok(Value::from_float(x / y)),
                        _ => arith::numeric_div(a, b),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::Sub => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_sub", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Sub, a, b);
                        a.decref();
                        b.decref();
                        frame.push(result?);
                    }
                }
                VMOpCode::Mul => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_mul", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Mul, a, b);
                        a.decref();
                        b.decref();
                        frame.push(result?);
                    }
                }
                VMOpCode::Div => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = host.binary_op(crate::host::BinaryOp::TrueDiv, a, b);
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::FloorDiv => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = host.binary_op(crate::host::BinaryOp::FloorDiv, a, b);
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::Mod => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = host.binary_op(crate::host::BinaryOp::Mod, a, b);
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::Pow => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = host.binary_op(crate::host::BinaryOp::Pow, a, b);
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }

                // =============================================================
                // Tier 2: Unary
                // =============================================================
                VMOpCode::Neg => {
                    let a = frame.pop();
                    let result = arith::numeric_neg(a);
                    a.decref();
                    frame.push(result?);
                }
                VMOpCode::Pos => {
                    // +x is a no-op for numeric types
                    let a = frame.peek();
                    if !a.is_int() && !a.is_float() && !a.is_bigint() {
                        return Err(VMError::TypeError(errors::ERR_BAD_UNARY_POS.into()));
                    }
                }

                // =============================================================
                // Tier 2: Bitwise
                // =============================================================
                VMOpCode::BAnd => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result: VMResult<Value> = match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => Ok(Value::from_int(ai & bi)),
                        _ => arith::bigint_binop(a, b, |x, y| rug::Integer::from(x & y))
                            .ok_or_else(|| VMError::TypeError(errors::ERR_UNSUPPORTED_BITAND.into())),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::BOr => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result: VMResult<Value> = match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => Ok(Value::from_int(ai | bi)),
                        _ => arith::bigint_binop(a, b, |x, y| rug::Integer::from(x | y))
                            .ok_or_else(|| VMError::TypeError(errors::ERR_UNSUPPORTED_BITOR.into())),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::BXor => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result: VMResult<Value> = match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => Ok(Value::from_int(ai ^ bi)),
                        _ => arith::bigint_binop(a, b, |x, y| rug::Integer::from(x ^ y))
                            .ok_or_else(|| VMError::TypeError(errors::ERR_UNSUPPORTED_BITXOR.into())),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::BNot => {
                    let a = frame.pop();
                    let result: VMResult<Value> = if let Some(i) = a.as_int() {
                        Ok(Value::from_int(!i))
                    } else if a.is_bigint() {
                        // SAFETY: the is_bigint() guard above proves a holds a live Arc<Integer> owned on the stack; the borrow does not outlive it.
                        let n = unsafe { a.as_bigint_ref().unwrap() };
                        Ok(Value::from_bigint_or_demote(rug::Integer::from(!n)))
                    } else {
                        Err(VMError::TypeError(errors::ERR_BAD_UNARY_NOT.into()))
                    };
                    a.decref();
                    frame.push(result?);
                }
                VMOpCode::LShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result: VMResult<Value> = match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => {
                            if bi < 0 {
                                Err(VMError::ValueError("negative shift count".into()))
                            } else if bi < 64 {
                                Ok(Value::try_from_int(ai << bi).unwrap_or_else(|| {
                                    Value::from_bigint_or_demote(rug::Integer::from(ai) << bi as u32)
                                }))
                            } else {
                                Ok(Value::from_bigint_or_demote(rug::Integer::from(ai) << bi as u32))
                            }
                        }
                        _ => Err(VMError::TypeError(errors::ERR_UNSUPPORTED_LSHIFT.into())),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }
                VMOpCode::RShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result: VMResult<Value> = match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => {
                            if bi < 0 {
                                Err(VMError::ValueError("negative shift count".into()))
                            } else {
                                Ok(Value::from_int(ai >> bi.min(63)))
                            }
                        }
                        _ => Err(VMError::TypeError(errors::ERR_UNSUPPORTED_RSHIFT.into())),
                    };
                    a.decref();
                    b.decref();
                    frame.push(result?);
                }

                // =============================================================
                // Tier 2: Comparisons
                // =============================================================
                VMOpCode::Lt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_lt", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Lt, a, b);
                        a.decref();
                        b.decref();
                        frame.push(result?);
                    }
                }
                VMOpCode::Le => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_le", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Le, a, b);
                        a.decref();
                        b.decref();
                        frame.push(result?);
                    }
                }
                VMOpCode::Gt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_gt", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Gt, a, b);
                        a.decref();
                        b.decref();
                        frame.push(result?);
                    }
                }
                VMOpCode::Ge => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_ge", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Ge, a, b);
                        a.decref();
                        b.decref();
                        frame.push(result?);
                    }
                }
                VMOpCode::Eq => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_eq", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let r = Value::from_bool(deep_eq(a, b));
                        a.decref();
                        b.decref();
                        frame.push(r);
                    }
                }
                VMOpCode::Ne => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_ne", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?; // consumes a, b
                    } else {
                        let r = Value::from_bool(!deep_eq(a, b));
                        a.decref();
                        b.decref();
                        frame.push(r);
                    }
                }

                // =============================================================
                // Tier 2: Scope resolution
                // =============================================================
                VMOpCode::LoadScope => {
                    let name = &code.names[instr.arg as usize];
                    // Try closure scope first (stored on frame_stack context)
                    let val = self.resolve_scope(name, frame, host)?;
                    val.clone_refcount();
                    frame.push(val);
                }
                VMOpCode::StoreScope => {
                    let val = frame.pop();
                    let name = &code.names[instr.arg as usize];
                    if !self.store_scope(name, val, frame) {
                        // If not in any scope, store as global
                        host.store_global(name, val);
                    }
                }
                VMOpCode::LoadGlobal => {
                    let name = &code.names[instr.arg as usize];
                    match host.lookup_global(name)? {
                        Some(val) => {
                            val.clone_refcount();
                            frame.push(val);
                        }
                        None => {
                            return Err(VMError::NameError(name.clone()));
                        }
                    }
                }

                // =============================================================
                // Tier 2: Function creation
                // =============================================================
                VMOpCode::MakeFunction => {
                    // Stack protocol: LoadConst pushes VMFunc(idx), MakeFunction pops it.
                    let func_code_val = frame.pop();

                    let func_code = if func_code_val.is_vmfunc() && !func_code_val.is_invalid() {
                        let slot_idx = func_code_val.as_vmfunc_idx();
                        match self.func_table.get(slot_idx) {
                            Some(slot) => Arc::clone(&slot.code),
                            None => {
                                return Err(VMError::RuntimeError(format!("invalid function index {slot_idx}")));
                            }
                        }
                    } else {
                        return Err(VMError::RuntimeError("MakeFunction: expected VMFunc on stack".into()));
                    };

                    // Capture every local of the defining frame, not only the names this
                    // child references directly: a more deeply nested closure may read an
                    // outer local that this child never names, and the scope chain only
                    // resolves through what each level captured. Filtering by the child's
                    // own names breaks that transitive case. Mirrors catnip_rs MakeFunction.
                    let mut captured = IndexMap::new();
                    if let Some(ref parent_code) = frame.code {
                        for (name, &slot_idx) in &parent_code.slotmap {
                            // Module-level vars are reached live via the parent chain, not
                            // frozen. But a slot in a nested function frame is a real local
                            // (param/local) that shadows any global homonym, so it must be
                            // captured; only suppress capture at the top-level frame, where
                            // slots coincide with module globals.
                            if frame.closure_scope.is_none() && host.has_global(name) {
                                continue;
                            }
                            let val = frame.get_local(slot_idx);
                            if !val.is_nil() && !val.is_invalid() {
                                val.clone_refcount();
                                captured.insert(name.clone(), val);
                            }
                        }
                    }

                    // Build closure parent from current frame's closure
                    let parent = if let Some(ref cs) = frame.closure_scope {
                        PureClosureParent::Scope(cs.clone())
                    } else if let Some(globals) = self.get_globals_rc(host) {
                        PureClosureParent::Globals(globals)
                    } else {
                        PureClosureParent::None
                    };

                    let closure = PureClosureScope::new(captured, parent);
                    // Runtime closure: an Arc-backed slot, reclaimed when its last
                    // Value dies (no longer a permanent grow-only table entry).
                    let arc = PureFuncSlot::new_runtime(func_code, Some(closure.clone()), None).into_arc();

                    // arg = name_idx + 1: bind the function under its own name in
                    // its closure so recursive references resolve (let-rec). The
                    // self-reference is a *weak* handle -- a strong capture would be
                    // an Arc self-cycle pinning the slot forever. The caller holds a
                    // strong ref while the function runs, so resolve() upgrades it.
                    if instr.arg > 0 {
                        let self_name = frame
                            .code
                            .as_ref()
                            .and_then(|c| c.names.get((instr.arg - 1) as usize))
                            .cloned();
                        if let Some(name) = self_name {
                            closure.set_self_ref(name, Arc::downgrade(&arc));
                        }
                    }

                    frame.push(Value::from_arc_closure(arc));
                }

                VMOpCode::PatchClosure => {
                    // Letrec group patch: closure_of(target)[names[arg]] = value.
                    // No-op when target is not a runtime closure (e.g. a sibling
                    // slot not populated yet because its branch never ran).
                    let value = frame.pop();
                    let target = frame.pop();
                    let mut consumed = false;
                    if target.is_closure() && value.is_closure() {
                        let name = frame
                            .code
                            .as_ref()
                            .and_then(|c| c.names.get(instr.arg as usize))
                            .cloned();
                        if let Some(name) = name {
                            // SAFETY: is_closure() proves a live Arc<PureFuncSlot>;
                            // the borrow ends before target.decref() below.
                            if let Some(slot) = unsafe { target.as_closure_ref() } {
                                if let Some(closure) = slot.closure.clone() {
                                    // Transfers value's ref: a *strong* sibling
                                    // capture -- the only site a runtime closure can
                                    // enter a mutual-recursion Arc cycle. Register
                                    // target so the reset drain breaks it.
                                    closure.insert_captured(&name, value);
                                    consumed = true;
                                    if let Some(weak) = target.closure_weak() {
                                        self.runtime_closures.push(weak);
                                    }
                                }
                            }
                        }
                    }
                    if !consumed {
                        value.decref();
                    }
                    target.decref();
                }

                // =============================================================
                // Tier 2: Function calls
                // =============================================================
                VMOpCode::Call => {
                    let nargs = instr.arg as usize;
                    let mut args = Vec::with_capacity(nargs);
                    for _ in 0..nargs {
                        args.push(frame.pop());
                    }
                    args.reverse();
                    let func = frame.pop();

                    if let Some((callee_code, closure, bound_self)) = self.callable_slot_data(func) {
                        if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                            Self::release_operands(&args);
                            func.decref(); // release the popped callee (no-op for a template)
                            return Err(VMError::FrameOverflow);
                        }

                        let mut new_frame = self.frame_pool.alloc_with_code(Arc::clone(&callee_code));
                        let args = Self::with_bound_receiver(bound_self, args);
                        new_frame.bind_args(&args);
                        new_frame.closure_scope = closure;
                        // The frame owns the popped callee ref for its lifetime: this
                        // keeps a runtime closure's slot alive while its body runs, so
                        // a letrec self-ref (weak) still upgrades even when the callee
                        // had no surviving binding (`mk()(5)`). Released at teardown.
                        new_frame.callee = func;

                        // Save current frame, switch to new
                        let old_frame = std::mem::replace(frame, new_frame);
                        self.frame_stack.push(old_frame);
                    } else {
                        self.call_non_vmfunc(func, args, frame, host)?;
                    }
                }
                VMOpCode::TailCall => {
                    let nargs = instr.arg as usize;
                    let mut args = Vec::with_capacity(nargs);
                    for _ in 0..nargs {
                        args.push(frame.pop());
                    }
                    args.reverse();
                    let func = frame.pop();

                    if let Some((callee_code, closure, bound_self)) = self.callable_slot_data(func) {
                        // Reuse current frame (TCO)
                        let nlocals = callee_code.nlocals;
                        // Decref old locals
                        for &v in &frame.locals {
                            v.decref();
                        }
                        frame.locals.clear();
                        let fill = if cfg!(debug_assertions) {
                            Value::INVALID
                        } else {
                            Value::NIL
                        };
                        frame.locals.resize(nlocals, fill);
                        frame.code = Some(callee_code);
                        frame.ip = 0;
                        frame.stack.clear();
                        for (_slot_start, saved) in frame.block_stack.drain(..) {
                            for val in saved {
                                val.decref();
                            }
                        }
                        if let Some(prev) = frame.match_bindings.take() {
                            for (_s, v) in prev {
                                v.decref();
                            }
                        }
                        frame.closure_scope = closure;
                        let args = Self::with_bound_receiver(bound_self, args);
                        frame.bind_args(&args);
                        // Reused frame: release the previous callee, take the new one.
                        // Keeps the tail-callee's slot alive while its body runs.
                        frame.callee.decref();
                        frame.callee = func;
                    } else {
                        // Non-VMFunc target (struct type, ND sentinel/wrapper,
                        // builtin, host object): plain-call semantics. The
                        // result lands on the current stack and the body's
                        // trailing Return forwards it to the caller.
                        self.call_non_vmfunc(func, args, frame, host)?;
                    }
                }
                VMOpCode::CallMethod => {
                    // arg = (name_idx << 16) | nargs
                    let name_idx = (instr.arg >> CALL_ARGS_SHIFT) as usize;
                    let nargs = (instr.arg & CALL_ARGS_MASK) as usize;

                    let mut args = Vec::with_capacity(nargs);
                    for _ in 0..nargs {
                        args.push(frame.pop());
                    }
                    args.reverse();
                    let obj = frame.pop();
                    let method_name = &code.names[name_idx];

                    // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
                    if let Some(cell) = unsafe { obj.as_struct_ref() } {
                        let type_id = cell.type_id;
                        let ty = self
                            .struct_registry
                            .get_type(type_id)
                            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;
                        if let Some(field_idx) = ty.field_index(method_name) {
                            // Callable field first (mirror struct_getattr and the
                            // catnip_rs VM): `r.h(5)` calls the field's value with
                            // no self binding. field_val is a bit-copy borrowed
                            // from the instance, so obj stays alive until the call
                            // is wired (its decref could cascade into the slot).
                            let field_val = cell.field(field_idx);
                            if let Some((callee_code, closure, bound_self)) = self.callable_slot_data(field_val) {
                                if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                    obj.decref();
                                    Self::release_operands(&args);
                                    return Err(VMError::FrameOverflow);
                                }
                                let args = Self::with_bound_receiver(bound_self, args);
                                let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                                new_frame.bind_args(&args);
                                new_frame.closure_scope = closure;
                                // Keep the field's slot alive while its body runs:
                                // obj may be the last ref of the instance, and its
                                // decref below would cascade into the field (the
                                // weak letrec self-ref would fail to upgrade).
                                field_val.clone_refcount();
                                new_frame.callee = field_val;
                                let old_frame = std::mem::replace(frame, new_frame);
                                self.frame_stack.push(old_frame);
                                obj.decref(); // field call: the instance is not bound as self
                            } else {
                                // Non-slot value: route through the same dispatch
                                // as a plain call (struct type constructor, ND
                                // wrapper, builtin, host object; raises on a
                                // non-callable). call_non_vmfunc consumes func and
                                // args; field_val is borrowed from the instance,
                                // so take an owned ref before releasing obj.
                                field_val.clone_refcount();
                                obj.decref();
                                self.call_non_vmfunc(field_val, args, frame, host)?;
                            }
                        } else if let Some(&func_idx) = ty.methods.get(method_name) {
                            let slot = self
                                .func_table
                                .get(func_idx)
                                .ok_or_else(|| VMError::RuntimeError("invalid method function".into()))?;
                            let callee_code = Arc::clone(&slot.code);
                            let closure = slot.closure.clone();
                            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                obj.decref(); // not yet transferred into full_args
                                Self::release_operands(&args);
                                return Err(VMError::FrameOverflow);
                            }
                            // Prepend self to args (transfers obj's ref into the frame)
                            let mut full_args = Vec::with_capacity(1 + args.len());
                            full_args.push(obj);
                            full_args.extend(args);
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&full_args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                        } else if let Some(&func_idx) = ty.static_methods.get(method_name) {
                            // Static method via an instance receiver: obj is not bound
                            // as self, so release it (covers success and overflow).
                            obj.decref();
                            let slot = self
                                .func_table
                                .get(func_idx)
                                .ok_or_else(|| VMError::RuntimeError("invalid static method".into()))?;
                            let callee_code = Arc::clone(&slot.code);
                            let closure = slot.closure.clone();
                            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                Self::release_operands(&args);
                                return Err(VMError::FrameOverflow);
                            }
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                        } else {
                            let ty_name = ty.name.clone();
                            obj.decref();
                            Self::release_operands(&args);
                            // "attribute", not "method": this arm covers fields,
                            // methods and statics (mirrors catnip_rs).
                            return Err(VMError::RuntimeError(format!(
                                "'{}' has no attribute '{}'",
                                ty_name, method_name
                            )));
                        }
                    } else if let Some(type_id) = obj.as_struct_type_id() {
                        // Static method call on the type itself (e.g. Point.origin())
                        let ty = self
                            .struct_registry
                            .get_type(type_id)
                            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;
                        if let Some(&func_idx) = ty.static_methods.get(method_name) {
                            let slot = self
                                .func_table
                                .get(func_idx)
                                .ok_or_else(|| VMError::RuntimeError("invalid static method".into()))?;
                            let callee_code = Arc::clone(&slot.code);
                            let closure = slot.closure.clone();
                            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                Self::release_operands(&args);
                                return Err(VMError::FrameOverflow);
                            }
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                        } else {
                            let ty_name = ty.name.clone();
                            Self::release_operands(&args);
                            return Err(VMError::RuntimeError(format!(
                                "type '{}' has no static method '{}'",
                                ty_name, method_name
                            )));
                        }
                    } else if obj.is_union_type() {
                        // Variant construction: `Option.Some(42)` compiles to
                        // CallMethod on the union namespace.
                        let (binding, union_name) = {
                            // SAFETY: the is_union_type() guard above proves obj holds a live Arc<UnionNamespace> owned on the stack; the borrow does not outlive it.
                            let ns = unsafe { obj.as_union_ref().unwrap() };
                            (ns.variant(method_name), ns.name.clone())
                        };
                        // The binding is an immediate (struct type id / symbol); obj
                        // (the union namespace) is not needed past this point.
                        obj.decref();
                        match binding {
                            Some(val) if val.is_struct_type() => {
                                let type_id = val.as_struct_type_id().unwrap();
                                // construct_struct consumes args on success, nothing
                                // on its reachable errors -- release them on failure.
                                let result = match self.construct_struct(type_id, &args, &IndexMap::new(), host) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        Self::release_operands(&args);
                                        return Err(e);
                                    }
                                };
                                frame.push(result);
                            }
                            Some(val) if val.is_symbol() => {
                                Self::release_operands(&args);
                                return Err(VMError::TypeError(format!(
                                    "variant '{}.{}' takes no payload",
                                    union_name, method_name
                                )));
                            }
                            _ => {
                                Self::release_operands(&args);
                                return Err(VMError::AttributeError(format!(
                                    "union '{}' has no variant '{}'",
                                    union_name, method_name
                                )));
                            }
                        }
                    } else if obj.is_module() {
                        let func_val_opt = {
                            // SAFETY: the is_module() guard above proves obj holds a live Arc<ModuleNamespace> owned on the stack; the borrow ends with this block.
                            let ns = unsafe { obj.as_module_ref().unwrap() };
                            ns.attrs.get(method_name).copied()
                        };
                        // func_val is a bit-copy borrowed from the module's attrs (not
                        // incref'd), so obj must stay alive until func_val is used;
                        // release obj at each terminal below.
                        let func_val = match func_val_opt {
                            Some(v) => v,
                            None => {
                                let name = {
                                    // SAFETY: obj is still a live Arc<ModuleNamespace> (guarded above); the borrow ends with this block.
                                    let ns = unsafe { obj.as_module_ref().unwrap() };
                                    ns.name.clone()
                                };
                                Self::release_operands(&args);
                                obj.decref();
                                return Err(VMError::AttributeError(format!(
                                    "module '{}' has no attribute '{}'",
                                    name, method_name
                                )));
                            }
                        };
                        // func_val is borrowed from the module's attrs (the module
                        // owns it), so no func_val.decref() here. It may be a runtime
                        // closure or a template function.
                        if let Some((callee_code, closure, bound_self)) = self.callable_slot_data(func_val) {
                            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                Self::release_operands(&args);
                                obj.decref();
                                return Err(VMError::FrameOverflow);
                            }
                            let args = Self::with_bound_receiver(bound_self, args);
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                            obj.decref(); // module namespace no longer needed
                        } else {
                            // Delegate to host for NativeStr builtins etc.
                            let result = host.call_function(func_val, &args);
                            Self::release_operands(&args);
                            obj.decref(); // func_val consumed; release the module
                            frame.push(result?);
                        }
                    } else if let Some(methods) = obj
                        .as_symbol()
                        .and_then(|sym_id| self.union_nullary_methods.get(&sym_id))
                        .cloned()
                    {
                        // Union method on a nullary variant: same shape as a
                        // struct method call, self = the symbol itself.
                        if let Some(&func_idx) = methods.get(method_name) {
                            let slot = self
                                .func_table
                                .get(func_idx)
                                .ok_or_else(|| VMError::RuntimeError("invalid method function".into()))?;
                            let callee_code = Arc::clone(&slot.code);
                            let closure = slot.closure.clone();
                            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                Self::release_operands(&args);
                                return Err(VMError::FrameOverflow);
                            }
                            let mut full_args = Vec::with_capacity(1 + args.len());
                            full_args.push(obj);
                            full_args.extend(args);
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&full_args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                        } else {
                            let qname = self
                                .symbol_table
                                .resolve(obj.as_symbol().unwrap())
                                .unwrap_or("<variant>")
                                .to_string();
                            Self::release_operands(&args);
                            return Err(VMError::RuntimeError(format!(
                                "'{}' has no method '{}'",
                                qname, method_name
                            )));
                        }
                    } else if (obj.is_native_list() && matches!(method_name.as_str(), "index" | "count" | "remove"))
                        || (obj.is_native_tuple() && matches!(method_name.as_str(), "index" | "count"))
                    {
                        // Registry-aware equality: the default collection methods
                        // compare with Value::eq, which has no struct registry and
                        // misses struct payloads compared by value.
                        // These host paths borrow obj/args (they do not transfer
                        // them to a callee frame), so release the stack-owned
                        // operands afterwards.
                        let result = self.collection_eq_method(obj, method_name, &args);
                        obj.decref();
                        for a in &args {
                            a.decref();
                        }
                        frame.push(result?);
                    } else if let Some(result) = self.collection_method_reg(obj, method_name.as_str(), &args, host) {
                        // Registry-aware dict/set methods so struct keys hash and
                        // materialize correctly (the host has no registry).
                        obj.decref();
                        for a in &args {
                            a.decref();
                        }
                        frame.push(result?);
                    } else {
                        let result = host.call_method(obj, method_name, &args);
                        obj.decref();
                        for a in &args {
                            a.decref();
                        }
                        frame.push(result?);
                    }
                }
                VMOpCode::CallKw => {
                    let nargs = ((instr.arg >> 8) & 0xFF) as usize;
                    let nkwargs = (instr.arg & 0xFF) as usize;

                    // Pop kw_names tuple
                    let kw_names_val = frame.pop();
                    let kw_names: Vec<String> = if kw_names_val.is_native_tuple() {
                        // SAFETY: the is_native_tuple() guard above proves kw_names_val holds a live Arc<NativeTuple> owned on the stack; the borrow does not outlive it.
                        let tuple = unsafe { kw_names_val.as_native_tuple_ref().unwrap() };
                        tuple.as_slice().iter().map(|v| v.display_string()).collect()
                    } else {
                        vec![]
                    };

                    // Pop kwarg values (in reverse stack order)
                    let mut kw_values = Vec::with_capacity(nkwargs);
                    for _ in 0..nkwargs {
                        kw_values.push(frame.pop());
                    }
                    kw_values.reverse();

                    // Pop positional args
                    let mut args = Vec::with_capacity(nargs);
                    for _ in 0..nargs {
                        args.push(frame.pop());
                    }
                    args.reverse();

                    // Pop function
                    let mut func = frame.pop();

                    if let Some((callee_code, closure, bound_self)) = self.callable_slot_data(func) {
                        if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                            Self::release_operands(&args);
                            Self::release_operands(&kw_values);
                            kw_names_val.decref();
                            func.decref(); // release the popped callee (closure Arc; no-op for a template)
                            return Err(VMError::FrameOverflow);
                        }

                        let mut new_frame = self.frame_pool.alloc_with_code(Arc::clone(&callee_code));
                        // Clone bound_self's ref before the trailing func.decref() may drop the slot.
                        let args = Self::with_bound_receiver(bound_self, args);
                        new_frame.bind_args(&args);
                        // Bind kwargs by name
                        for (name, val) in kw_names.iter().zip(kw_values.iter()) {
                            if let Some(&slot_idx) = callee_code.slotmap.get(name) {
                                // A kwarg naming an already-bound slot displaces
                                // an owned value (positional arg) -- release it,
                                // set_local overwrites raw. NIL on fresh slots.
                                new_frame.get_local(slot_idx).decref();
                                new_frame.set_local(slot_idx, *val);
                            } else {
                                // Unknown kwarg name: not bound to any slot, so
                                // release the stack-owned token.
                                val.decref();
                            }
                        }
                        new_frame.closure_scope = closure;
                        // The frame owns the callee ref for its lifetime (keeps a
                        // self-recursive closure's slot alive). NIL the local so the
                        // trailing common func.decref() does not double-release it.
                        new_frame.callee = func;
                        func = Value::NIL;
                        let old_frame = std::mem::replace(frame, new_frame);
                        self.frame_stack.push(old_frame);
                    } else if func.is_native_str() {
                        // SAFETY: the is_native_str() guard above proves func holds a live Arc<NativeString> owned on the stack; the borrow does not outlive it.
                        let name = unsafe { func.as_native_str_ref().unwrap() };
                        match name {
                            "import" if self.import_loader.is_some() => {
                                let kw: Vec<(String, Value)> = kw_names.into_iter().zip(kw_values).collect();
                                let result = self.handle_import(&args, &kw, host);
                                // handle_import borrows args/kw; release the tokens.
                                for (_, v) in &kw {
                                    v.decref();
                                }
                                for a in &args {
                                    a.decref();
                                }
                                frame.push(result?);
                            }
                            "dict" => {
                                let dict = Value::from_empty_dict();
                                // SAFETY: dict was just built by from_empty_dict, so it is a live Arc<NativeDict> owned here; the borrow does not outlive it.
                                let d = unsafe { dict.as_native_dict_ref().unwrap() };
                                for (kname, val) in kw_names.iter().zip(kw_values.iter()) {
                                    let key = crate::collections::ValueKey::Str(Arc::new(
                                        crate::value::NativeString::new(kname.clone()),
                                    ));
                                    d.set_item(key, *val);
                                }
                                // set_item borrows each value; release the kwarg
                                // tokens and any unused positional args.
                                for v in &kw_values {
                                    v.decref();
                                }
                                for a in &args {
                                    a.decref();
                                }
                                frame.push(dict);
                            }
                            _ => {
                                // Build the message before releasing func: `name`
                                // borrows the NativeStr behind it.
                                let err = VMError::TypeError(format!(
                                    "{}() does not accept keyword arguments in PureVM",
                                    name
                                ));
                                Self::release_operands(&args);
                                Self::release_operands(&kw_values);
                                kw_names_val.decref();
                                func.decref();
                                return Err(err);
                            }
                        }
                    } else if func.is_struct_type() {
                        // Struct constructor with kwargs: same semantics as the
                        // PyO3 runtime (positionals first, keywords by name;
                        // unknown or doubled names error). construct_struct
                        // consumes args and kwarg values on success, nothing on
                        // its reachable errors.
                        let type_id = func.as_struct_type_id().unwrap();
                        let mut kw: IndexMap<String, Value> = IndexMap::with_capacity(kw_values.len());
                        let mut dup: Option<String> = None;
                        for (name, &val) in kw_names.iter().zip(kw_values.iter()) {
                            if kw.insert(name.clone(), val).is_some() {
                                dup = Some(name.clone());
                                break;
                            }
                        }
                        let result = if let Some(name) = dup {
                            Err(VMError::TypeError(format!(
                                "got multiple values for keyword argument '{}'",
                                name
                            )))
                        } else {
                            self.construct_struct(type_id, &args, &kw, host)
                        };
                        match result {
                            Ok(r) => frame.push(r),
                            Err(e) => {
                                Self::release_operands(&args);
                                Self::release_operands(&kw_values);
                                kw_names_val.decref();
                                func.decref();
                                return Err(e);
                            }
                        }
                    } else {
                        Self::release_operands(&args);
                        Self::release_operands(&kw_values);
                        kw_names_val.decref();
                        func.decref();
                        return Err(VMError::TypeError("CallKw: cannot call non-function".into()));
                    }
                    kw_names_val.decref();
                    // Releases the popped callee: no-op for a template (vmfunc is an
                    // index), a real decref for a runtime closure, and releases the
                    // builtin-name string on the dict/import paths.
                    func.decref();
                }

                // =============================================================
                // Tier 2: Collections
                // =============================================================
                VMOpCode::BuildList => {
                    let n = instr.arg as usize;
                    let start = frame.stack.len() - n;
                    let items: Vec<Value> = frame.stack.drain(start..).collect();
                    frame.push(Value::from_list(items));
                }
                VMOpCode::BuildTuple => {
                    let n = instr.arg as usize;
                    let start = frame.stack.len() - n;
                    let items: Vec<Value> = frame.stack.drain(start..).collect();
                    frame.push(Value::from_tuple(items));
                }
                VMOpCode::BuildSet => {
                    let n = instr.arg as usize;
                    let start = frame.stack.len() - n;
                    let items: Vec<Value> = frame.stack.drain(start..).collect();
                    let mut set = indexmap::IndexSet::new();
                    for v in &items {
                        let key = self.value_to_key(*v, host)?;
                        set.insert(key);
                    }
                    // Decref items (keys copied into set)
                    for v in &items {
                        v.decref();
                    }
                    frame.push(Value::from_set(set));
                }
                VMOpCode::BuildDict => {
                    let n = instr.arg as usize;
                    // Stack: key1, val1, key2, val2, ...
                    let start = frame.stack.len() - n * 2;
                    let pairs: Vec<Value> = frame.stack.drain(start..).collect();
                    let dict = Value::from_empty_dict();
                    for chunk in pairs.chunks(2) {
                        let key = self.value_to_key(chunk[0], host)?;
                        // SAFETY: dict was just built by from_empty_dict, so it is a live Arc<NativeDict> owned here; the borrow does not outlive it.
                        let d = unsafe { dict.as_native_dict_ref().unwrap() };
                        d.set_item(key, chunk[1]);
                        chunk[0].decref(); // key consumed by to_key
                        chunk[1].decref(); // value borrowed by set_item (clone_refcount)
                    }
                    frame.push(dict);
                }
                VMOpCode::BuildSlice => {
                    // In PureVM, slices are represented as tuples (start, stop[, step])
                    let n = instr.arg as usize;
                    let start = frame.stack.len() - n;
                    let items: Vec<Value> = frame.stack.drain(start..).collect();
                    frame.push(Value::from_tuple(items));
                }

                // =============================================================
                // Tier 2: Iteration
                // =============================================================
                VMOpCode::GetIter => {
                    let obj = frame.pop();
                    // Dict/set keys are materialized through the registry so
                    // struct keys iterate as instances; the host has no registry.
                    // The iterator clones its items (independent refs), so the
                    // iterable operand is released once the iterator is built.
                    let iter = if obj.is_native_dict() {
                        // SAFETY: the is_native_dict() guard above proves obj holds a live Arc<NativeDict> owned on the stack; the borrow does not outlive it.
                        let keys = unsafe { obj.as_native_dict_ref().unwrap() }.keys_cloned();
                        let vals: Vec<Value> = keys.iter().map(|k| self.key_to_value(k)).collect();
                        crate::host::vec_value_iter(vals)
                    } else if obj.is_native_set() {
                        // SAFETY: the is_native_set() guard above proves obj holds a live Arc<NativeSet> owned on the stack; the borrow does not outlive it.
                        let keys = unsafe { obj.as_native_set_ref().unwrap() }.keys_cloned();
                        let vals: Vec<Value> = keys.iter().map(|k| self.key_to_value(k)).collect();
                        crate::host::vec_value_iter(vals)
                    } else {
                        match host.get_iter(obj) {
                            Ok(it) => it,
                            Err(e) => {
                                obj.decref();
                                return Err(e);
                            }
                        }
                    };
                    obj.decref();
                    let handle = iterators.len() as i64;
                    iterators.push(Some(iter));
                    // Push handle onto stack (like catnip_rs keeps iterator on TOS)
                    frame.push(Value::from_int(handle));
                }
                VMOpCode::ForIter => {
                    let jump_target = instr.arg as usize;
                    let handle = frame.peek().as_int().unwrap_or(-1);
                    if handle >= 0 {
                        let idx = handle as usize;
                        let next = if let Some(Some(ref mut iter)) = iterators.get_mut(idx) {
                            iter.next_value()?
                        } else {
                            None
                        };
                        match next {
                            Some(val) => {
                                frame.push(val);
                            }
                            None => {
                                // Exhausted: release iterator, pop handle, jump to end
                                if idx < iterators.len() {
                                    iterators[idx] = None;
                                }
                                frame.pop(); // pop handle
                                frame.ip = jump_target;
                            }
                        }
                    } else {
                        frame.pop();
                        frame.ip = jump_target;
                    }
                }

                // =============================================================
                // Tier 2: Attribute/item access
                // =============================================================
                VMOpCode::GetAttr => {
                    let name_idx = instr.arg as usize;
                    let name = &code.names[name_idx];
                    let obj = frame.pop();
                    // Every branch yields an owned value (struct_getattr,
                    // module attrs, and host.obj_getattr incref; enum/union
                    // bindings and static methods are immediates). Funnel through
                    // a single result so `obj` is released on every path.
                    let result: VMResult<Value> = if obj.is_struct_instance() {
                        self.struct_getattr(obj, name)
                    } else if let Some(type_id) = obj.as_struct_type_id() {
                        match self.struct_registry.get_type(type_id) {
                            Some(ty) => match ty.static_methods.get(name) {
                                Some(&func_idx) => Ok(Value::from_vmfunc(func_idx)),
                                None => Err(VMError::RuntimeError(format!(
                                    "type '{}' has no static method '{}'",
                                    ty.name, name
                                ))),
                            },
                            None => Err(VMError::RuntimeError("invalid struct type".into())),
                        }
                    } else if let Some(enum_type_id) = obj.as_enum_type_id() {
                        match self.enum_registry.get_variant_value(enum_type_id, name) {
                            Some(val) => Ok(val),
                            None => {
                                let ety = self.enum_registry.get_type(enum_type_id).unwrap();
                                Err(VMError::RuntimeError(format!(
                                    "enum '{}' has no variant '{}'",
                                    ety.name, name
                                )))
                            }
                        }
                    } else if obj.is_union_type() {
                        // SAFETY: the is_union_type() guard above proves obj holds a live Arc<UnionNamespace> owned on the stack; the borrow does not outlive it.
                        let ns = unsafe { obj.as_union_ref().unwrap() };
                        // Bindings are immediates (struct type id or symbol).
                        match ns.variant(name) {
                            Some(val) => Ok(val),
                            None => Err(VMError::AttributeError(format!(
                                "union '{}' has no variant '{}'",
                                ns.name, name
                            ))),
                        }
                    } else if obj.is_module() {
                        // SAFETY: the is_module() guard above proves obj holds a live Arc<ModuleNamespace> owned on the stack; the borrow does not outlive it.
                        let ns = unsafe { obj.as_module_ref().unwrap() };
                        match ns.attrs.get(name) {
                            Some(val) => {
                                val.clone_refcount();
                                Ok(*val)
                            }
                            None => Err(VMError::AttributeError(format!(
                                "module '{}' has no attribute '{}'",
                                ns.name, name
                            ))),
                        }
                    } else {
                        host.obj_getattr(obj, name)
                    };
                    obj.decref();
                    frame.push(result?);
                }
                VMOpCode::SetAttr => {
                    let name_idx = instr.arg as usize;
                    let name = &code.names[name_idx];
                    let val = frame.pop();
                    let obj = frame.pop();
                    let res = if obj.is_struct_instance() {
                        self.struct_setattr(obj, name, val)
                    } else {
                        host.obj_setattr(obj, name, val)
                    };
                    // struct_setattr / obj_setattr consume `val` on success (stored
                    // without an incref); on error it was not stored, so release
                    // it. `obj` is only read.
                    obj.decref();
                    if res.is_err() {
                        val.decref();
                    }
                    res?;
                }
                VMOpCode::GetItem => {
                    if instr.arg == 1 {
                        // Slice mode: stack has [obj, start, stop, step]
                        let step = frame.pop();
                        let stop = frame.pop();
                        let start = frame.pop();
                        let obj = frame.pop();
                        let result = crate::host::apply_slice(obj, start, stop, step);
                        // apply_slice returns a fresh value; release the operands.
                        obj.decref();
                        start.decref();
                        stop.decref();
                        step.decref();
                        frame.push(result?);
                    } else {
                        let key = frame.pop();
                        let obj = frame.pop();
                        // Dict lookups route through the registry-aware key so
                        // struct keys are found by value; the host has no
                        // registry. Other containers keep the host's semantics.
                        let result = if obj.is_native_dict() {
                            match self.value_to_key(key, host) {
                                Ok(k) => {
                                    // SAFETY: the is_native_dict() guard above proves obj holds a live Arc<NativeDict> owned on the stack; the borrow does not outlive it.
                                    let dict = unsafe { obj.as_native_dict_ref().unwrap() };
                                    dict.get_item(&k)
                                }
                                Err(e) => Err(e),
                            }
                        } else {
                            host.obj_getitem(obj, key)
                        };
                        // get_item / obj_getitem incref the returned value; release
                        // the container and key operands.
                        obj.decref();
                        key.decref();
                        frame.push(result?);
                    }
                }
                VMOpCode::SetItem => {
                    let val = frame.pop();
                    let key = frame.pop();
                    let obj = frame.pop();
                    let res: VMResult<()> = if obj.is_native_dict() {
                        match self.value_to_key(key, host) {
                            Ok(k) => {
                                // SAFETY: the is_native_dict() guard above proves obj holds a live Arc<NativeDict> owned on the stack; the borrow does not outlive it.
                                let dict = unsafe { obj.as_native_dict_ref().unwrap() };
                                dict.set_item(k, val);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        }
                    } else {
                        host.obj_setitem(obj, key, val)
                    };
                    // set_item / list.set borrow the value (clone_refcount) and
                    // to_key takes an independent key ref; release the three
                    // stack-owned operands (the container is only read).
                    obj.decref();
                    key.decref();
                    val.decref();
                    res?;
                }

                // =============================================================
                // Tier 2: Membership + identity
                // =============================================================
                VMOpCode::In => {
                    let container = frame.pop();
                    let item = frame.pop();
                    let result = self.contains_value(item, container, host);
                    // contains_value only reads its operands; release the
                    // stack-owned references.
                    item.decref();
                    container.decref();
                    frame.push(Value::from_bool(result?));
                }
                VMOpCode::NotIn => {
                    let container = frame.pop();
                    let item = frame.pop();
                    let result = self.contains_value(item, container, host);
                    item.decref();
                    container.decref();
                    frame.push(Value::from_bool(!result?));
                }
                VMOpCode::Is => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let r = Value::from_bool(a.bits() == b.bits());
                    a.decref();
                    b.decref();
                    frame.push(r);
                }
                VMOpCode::IsNot => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let r = Value::from_bool(a.bits() != b.bits());
                    a.decref();
                    b.decref();
                    frame.push(r);
                }

                // =============================================================
                // Tier 2: Unpack
                // =============================================================
                VMOpCode::UnpackSequence => {
                    let n = instr.arg as usize;
                    let seq = frame.pop();
                    let items = match self.unpack_to_vec(seq) {
                        Ok(v) => v,
                        Err(e) => {
                            seq.decref();
                            return Err(e);
                        }
                    };
                    seq.decref(); // the source container is consumed
                    if items.len() != n {
                        // unpack_to_vec increfed each item; none get pushed here
                        for it in &items {
                            it.decref();
                        }
                        return Err(VMError::RuntimeError(format!(
                            "cannot unpack {} values into {} variables",
                            items.len(),
                            n
                        )));
                    }
                    // Push in reverse order (first item ends on top). The items' refs
                    // (from unpack_to_vec) transfer to the stack -- no extra clone.
                    for item in items.into_iter().rev() {
                        frame.push(item);
                    }
                }
                VMOpCode::UnpackEx => {
                    let before = ((instr.arg >> 8) & 0xFF) as usize;
                    let after = (instr.arg & 0xFF) as usize;
                    let seq = frame.pop();
                    let items = match self.unpack_to_vec(seq) {
                        Ok(v) => v,
                        Err(e) => {
                            seq.decref();
                            return Err(e);
                        }
                    };
                    seq.decref(); // the source container is consumed

                    let total_fixed = before + after;
                    if items.len() < total_fixed {
                        for it in &items {
                            it.decref();
                        }
                        return Err(VMError::RuntimeError(format!(
                            "not enough values to unpack (expected at least {}, got {})",
                            total_fixed,
                            items.len()
                        )));
                    }

                    let rest_len = items.len() - total_fixed;
                    let before_items = &items[..before];
                    let rest_items = &items[before..before + rest_len];
                    let after_items = &items[before + rest_len..];

                    // Push in reverse order: after, rest (as list), before. Each push
                    // clones, so the pushed values own fresh refs.
                    for item in after_items.iter().rev() {
                        item.clone_refcount();
                        frame.push(*item);
                    }
                    let rest_list: Vec<Value> = rest_items.to_vec();
                    for v in &rest_list {
                        v.clone_refcount();
                    }
                    frame.push(Value::from_list(rest_list));
                    for item in before_items.iter().rev() {
                        item.clone_refcount();
                        frame.push(*item);
                    }
                    // The pushes cloned every element; release the `items` refs that
                    // unpack_to_vec increfed (they are not transferred).
                    for it in &items {
                        it.decref();
                    }
                }

                // =============================================================
                // Tier 2: Pattern matching
                // =============================================================
                VMOpCode::MatchPatternVM => {
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    // Release a previous arm's bindings before overwriting them
                    // (BindMatch clones, so match_bindings keeps its owned refs).
                    if let Some(prev) = frame.match_bindings.take() {
                        for (_s, v) in prev {
                            v.decref();
                        }
                    }
                    let pattern = code.patterns.get(pat_idx).cloned();
                    // Scope predicate for enum patterns: mirror of resolve_scope
                    // (closure chain, then host globals) -- resolve() is
                    // refcount-neutral by contract, has_global reads the map.
                    let in_scope = |name: &str| -> bool {
                        frame
                            .closure_scope
                            .as_ref()
                            .is_some_and(|cs| cs.resolve(name).is_some())
                            || host.has_global(name)
                    };
                    let matched = match pattern.as_ref() {
                        Some(pat) => {
                            match vm_match_pattern(pat, value, &self.struct_registry, &self.symbol_table, &in_scope) {
                                Ok(m) => m,
                                Err(e) => {
                                    // The subject is a DupTop copy owned by this op.
                                    value.decref();
                                    return Err(e);
                                }
                            }
                        }
                        None => None,
                    };
                    match matched {
                        Some(bindings) => {
                            frame.match_bindings = Some(bindings);
                            frame.push(Value::TRUE);
                        }
                        None => {
                            frame.match_bindings = None;
                            frame.push(Value::NIL);
                        }
                    }
                    // The subject is a DupTop copy owned by this op. Every arm binds
                    // clones (Var clone_refcounts, struct/tuple copy their fields),
                    // never the subject itself, so release the copy here. Before the
                    // Arc model this was a no-op for a struct subject -- it leaked.
                    value.decref();
                }
                VMOpCode::MatchAssignPatternVM => {
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    if let Some(prev) = frame.match_bindings.take() {
                        for (_s, v) in prev {
                            v.decref();
                        }
                    }
                    let pattern = code.patterns.get(pat_idx).cloned();
                    let in_scope = |name: &str| -> bool {
                        frame
                            .closure_scope
                            .as_ref()
                            .is_some_and(|cs| cs.resolve(name).is_some())
                            || host.has_global(name)
                    };
                    let result = match pattern {
                        Some(ref pat) => {
                            vm_match_assign_pattern(pat, value, &self.struct_registry, &self.symbol_table, &in_scope)
                        }
                        None => Err(VMError::RuntimeError("invalid assignment pattern index".into())),
                    };
                    // The subject is a DupTop copy owned by this op; the destructured
                    // bindings are clones, so release the copy on every path (the twin
                    // of MatchPatternVM). Before the Arc model this leaked a struct.
                    value.decref();
                    frame.match_bindings = Some(result?);
                    frame.push(Value::TRUE);
                }
                VMOpCode::BindMatch => {
                    // clone (not take): a guarded arm binds twice from the same
                    // match_bindings -- once inside the guard's push_block/pop_block,
                    // once for the body (see compile_match). Taking here left the
                    // body binding empty, so a guarded arm ran with an unbound
                    // capture. Each binding takes its OWN refcount; the owned refs
                    // still held by match_bindings are released when it is overwritten
                    // (next MatchPatternVM) or the frame is torn down.
                    if let Some(bindings) = frame.match_bindings.clone() {
                        frame.pop(); // pop sentinel TRUE
                        for (slot, val) in bindings {
                            let old = frame.get_local(slot);
                            old.decref();
                            val.clone_refcount();
                            frame.set_local(slot, val);
                        }
                    }
                }
                VMOpCode::MatchFail => {
                    let msg_idx = instr.arg as usize;
                    let msg = &code.constants[msg_idx];
                    let msg_str = msg.display_string();
                    return Err(VMError::RuntimeError(msg_str));
                }

                // =============================================================
                // Tier 2: String formatting
                // =============================================================
                VMOpCode::FormatValue => {
                    let flags = instr.arg;
                    let has_spec = (flags & 1) != 0;
                    let conv = (flags >> 1) & 3;

                    let spec = if has_spec { frame.pop() } else { Value::NIL };
                    let value = frame.pop();

                    // Apply conversion: 0=none, 1=str, 2=repr, 3=ascii
                    let base = match conv {
                        2 => self.display_value_repr(&value),
                        _ => self.display_value(&value),
                    };

                    // Apply format spec if present
                    let formatted = if has_spec && spec.is_native_str() {
                        // SAFETY: the is_native_str() guard above proves spec holds a live Arc<NativeString> owned on the stack; the borrow does not outlive it.
                        let spec_str = unsafe { spec.as_native_str_ref().unwrap() };
                        if !spec_str.is_empty() {
                            apply_format_spec(&base, &value, spec_str)
                        } else {
                            base
                        }
                    } else {
                        base
                    };
                    spec.decref();
                    frame.push(Value::from_string(formatted));
                }
                VMOpCode::BuildString => {
                    let n = instr.arg as usize;
                    let start = frame.stack.len() - n;
                    let mut buf = String::with_capacity(n * 16);
                    for i in start..frame.stack.len() {
                        let s = self.display_value(&frame.stack[i]);
                        buf.push_str(&s);
                    }
                    // Decref and truncate
                    for i in start..frame.stack.len() {
                        frame.stack[i].decref();
                    }
                    frame.stack.truncate(start);
                    frame.push(Value::from_string(buf));
                }

                // =============================================================
                // Tier 2: TypeOf
                // =============================================================
                VMOpCode::TypeOf => {
                    let val = frame.pop();
                    // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
                    if let Some(cell) = unsafe { val.as_struct_ref() } {
                        let name = self.struct_registry.type_name(cell.type_id).to_string();
                        frame.push(Value::from_string(name));
                    } else if let Some(sym_id) = val.as_symbol() {
                        // Enum variant: resolve to the declaring enum type name
                        if let Some((type_id, _)) = self.enum_registry.lookup_symbol(sym_id) {
                            let name = &self.enum_registry.get_type(type_id).unwrap().name;
                            frame.push(Value::from_string(name.clone()));
                        } else if let Some((owner, _)) =
                            self.symbol_table.resolve(sym_id).and_then(|s| s.split_once('.'))
                        {
                            // Union nullary variant: qualified symbol outside
                            // the enum registry -- report the declaring union.
                            frame.push(Value::from_string(owner.to_string()));
                        } else {
                            frame.push(Value::from_str("symbol"));
                        }
                    } else {
                        frame.push(Value::from_str(val.type_name()));
                    }
                    val.decref(); // the operand is consumed by TypeOf
                }

                // =============================================================
                // Tier 3: Structs and traits
                // =============================================================
                VMOpCode::MakeStruct => {
                    let const_idx = instr.arg as usize;
                    let info = code.constants[const_idx];
                    self.handle_make_struct(info, frame, host)?;
                }
                VMOpCode::MakeTrait => {
                    let const_idx = instr.arg as usize;
                    let info = code.constants[const_idx];
                    self.handle_make_trait(info, frame)?;
                }
                VMOpCode::Globals => {
                    let mut items = indexmap::IndexMap::new();
                    for (k, v) in host.collect_all_globals() {
                        // `from_dict` owns its entries and decrefs them on drop;
                        // the globals map keeps its own ref, so clone here.
                        v.clone_refcount();
                        let key =
                            crate::collections::ValueKey::Str(std::sync::Arc::new(crate::value::NativeString::new(k)));
                        items.insert(key, v);
                    }
                    frame.push(Value::from_dict(items));
                }

                VMOpCode::Locals => {
                    // Every branch feeds `from_dict`, which owns and decrefs its
                    // entries on drop; the source (globals map, frame locals,
                    // closure captures) keeps its own ref, so clone each value.
                    let mut items: indexmap::IndexMap<crate::collections::ValueKey, Value> = indexmap::IndexMap::new();
                    if self.frame_stack.is_empty() {
                        // Module level: locals() == globals()
                        for (k, v) in host.collect_all_globals() {
                            v.clone_refcount();
                            let key = crate::collections::ValueKey::Str(std::sync::Arc::new(
                                crate::value::NativeString::new(k),
                            ));
                            items.insert(key, v);
                        }
                    } else {
                        // Inside function: frame.locals + code.varnames
                        for (i, name) in code.varnames.iter().enumerate() {
                            if i < frame.locals.len() {
                                let val = frame.locals[i];
                                if !val.is_nil() {
                                    val.clone_refcount();
                                    let key = crate::collections::ValueKey::Str(std::sync::Arc::new(
                                        crate::value::NativeString::new(name.clone()),
                                    ));
                                    items.insert(key, val);
                                }
                            }
                        }
                        // Closure captures
                        if let Some(ref closure) = frame.closure_scope {
                            for (k, v) in closure.captured_entries() {
                                if !v.is_nil() {
                                    v.clone_refcount();
                                    let key = crate::collections::ValueKey::Str(std::sync::Arc::new(
                                        crate::value::NativeString::new(k),
                                    ));
                                    items.insert(key, v);
                                }
                            }
                        }
                    }
                    frame.push(Value::from_dict(items));
                }

                // =============================================================
                // Broadcast & ND operations
                // =============================================================
                VMOpCode::Broadcast => {
                    let flags = instr.arg;
                    let is_filter = flags & 1 != 0;
                    let has_operand = flags & 2 != 0;
                    let is_nd_recursion = flags & 4 != 0;
                    let is_nd_map = flags & 8 != 0;

                    if is_nd_recursion {
                        let lambda = frame.pop();
                        let target = frame.pop();
                        let result = self.broadcast_nd_recursion(target, lambda, host);
                        lambda.decref(); // callee borrows the lambda; release the popped ref (closure)
                        frame.push(result?);
                    } else if is_nd_map {
                        let func = frame.pop();
                        let target = frame.pop();
                        let result = self.nd_map_apply(target, func, host);
                        // callee borrows both; release the popped refs (real decref
                        // for a heap target / closure func, no-op for scalars).
                        target.decref();
                        func.decref();
                        frame.push(result?);
                    } else {
                        let operand = if has_operand { Some(frame.pop()) } else { None };
                        let operator = frame.pop();
                        let target = frame.pop();
                        let result = self.apply_broadcast(target, operator, operand, is_filter, host);
                        // apply_broadcast and its whole subtree BORROW target,
                        // operator, and operand (each dispatched per element, never
                        // consumed -- a callable branch clones before handing an arg
                        // to the move-consuming `run_sync`). So the opcode owns the
                        // three popped refs and releases them on both outcomes: a real
                        // decref for heap values (a struct list target, a runtime
                        // closure operator `xs.[f]`), a no-op for scalars/templates.
                        target.decref();
                        operator.decref();
                        if let Some(operand) = operand {
                            operand.decref();
                        }
                        frame.push(result?);
                    }
                }
                VMOpCode::NdRecursion => {
                    let flag = instr.arg;
                    if flag == 1 {
                        // Declaration form: ~~(lambda) → callable wrapper
                        let lambda = frame.pop();
                        let wrapper = Value::from_tuple(vec![Value::from_str(super::broadcast::ND_DECL_TAG), lambda]);
                        frame.push(wrapper);
                    } else {
                        // Direct invocation: ~~(seed, lambda)
                        // Stack: [seed, lambda] (lambda on top)
                        let lambda = frame.pop();
                        let seed = frame.pop();
                        let result = self.nd_recursion_call(seed, lambda, host);
                        lambda.decref(); // callee borrows the lambda; release the popped ref (closure)
                        frame.push(result?);
                    }
                }
                VMOpCode::NdMap => {
                    let flag = instr.arg;
                    if flag == 1 {
                        // Declaration form: ~>(func) → callable wrapper
                        let func = frame.pop();
                        let wrapper = Value::from_tuple(vec![Value::from_str(super::broadcast::ND_LIFT_TAG), func]);
                        frame.push(wrapper);
                    } else {
                        // Direct invocation: ~>(data, func)
                        let func = frame.pop();
                        let data = frame.pop();
                        let result = self.nd_map_apply(data, func, host);
                        // callee borrows both; release the popped refs (real decref
                        // for a heap data target / closure func, no-op for scalars).
                        data.decref();
                        func.decref();
                        frame.push(result?);
                    }
                }
                VMOpCode::NdEmptyTopos => {
                    frame.push(Value::from_list(vec![]));
                }
                VMOpCode::MatchPattern => {
                    return Err(VMError::RuntimeError(errors::ERR_LEGACY_MATCH.into()));
                }
                VMOpCode::Breakpoint => {
                    let depth = self.frame_stack.len() + 1;
                    self.debug.pause(frame, &code, instr_idx, depth, true);
                }
                VMOpCode::MakeEnum => {
                    let const_idx = instr.arg as usize;
                    let info = code.constants[const_idx];
                    self.handle_make_enum(info, frame, host)?;
                }
                VMOpCode::MakeUnion => {
                    let const_idx = instr.arg as usize;
                    let info = code.constants[const_idx];
                    self.handle_make_union(info, frame, host)?;
                }

                // Exception handling
                VMOpCode::SetupExcept => {
                    frame.handler_stack.push(catnip_core::exception::Handler {
                        handler_type: catnip_core::exception::HandlerType::Except,
                        target_addr: instr.arg as usize,
                        stack_depth: frame.stack.len(),
                        block_depth: frame.block_stack.len(),
                    });
                }
                VMOpCode::SetupFinally => {
                    frame.handler_stack.push(catnip_core::exception::Handler {
                        handler_type: catnip_core::exception::HandlerType::Finally,
                        target_addr: instr.arg as usize,
                        stack_depth: frame.stack.len(),
                        block_depth: frame.block_stack.len(),
                    });
                }
                VMOpCode::PopHandler => {
                    frame.handler_stack.pop();
                }
                VMOpCode::CheckExcMatch => {
                    let const_val = code.constants[instr.arg as usize];
                    let type_name_to_match = const_val.display_string();
                    let matches = if let Some(exc_info) = frame.active_exception_stack.last() {
                        exc_info.matches(&type_name_to_match)
                    } else {
                        false
                    };
                    frame.push(Value::from_bool(matches));
                }
                VMOpCode::LoadException => {
                    if instr.arg == 1 {
                        // ExcInfo mode: push (type_name, message, nil) as a tuple
                        if let Some(exc_info) = frame.active_exception_stack.last() {
                            let tuple = Value::from_tuple(vec![
                                Value::from_string(exc_info.type_name.clone()),
                                Value::from_string(exc_info.message.clone()),
                                Value::NIL,
                            ]);
                            frame.push(tuple);
                        } else {
                            frame.push(Value::from_tuple(vec![Value::NIL, Value::NIL, Value::NIL]));
                        }
                    } else if let Some(exc_info) = frame.active_exception_stack.last() {
                        frame.push(Value::from_string(exc_info.message.clone()));
                    } else {
                        frame.push(Value::NIL);
                    }
                }
                VMOpCode::Raise => {
                    if instr.arg == 1 {
                        // Bare raise: re-raise preserving full MRO
                        if let Some(exc_info) = frame.active_exception_stack.last().cloned() {
                            return Err(VMError::UserException(exc_info));
                        } else {
                            return Err(VMError::RuntimeError(errors::ERR_NO_ACTIVE_EXCEPTION.into()));
                        }
                    } else {
                        // raise expr: detect exception type from struct instances
                        let val = frame.pop();
                        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
                        let err = if let Some(cell) = unsafe { val.as_struct_ref() } {
                            let type_name = self.struct_registry.type_name(cell.type_id).to_string();
                            let msg = cell
                                .fields
                                .first()
                                .map(|c| c.get().display_string())
                                .unwrap_or_default();
                            // Get MRO from struct registry for hierarchical matching
                            let mro = self
                                .struct_registry
                                .find_type_by_name(&type_name)
                                .map(|ty| ty.mro.clone())
                                .unwrap_or_else(|| vec![type_name.clone()]);
                            VMError::UserException(catnip_core::exception::ExceptionInfo::new(type_name, msg, mro))
                        } else {
                            let msg = val.display_string();
                            VMError::RuntimeError(msg)
                        };
                        val.decref();
                        return Err(err);
                    }
                }
                VMOpCode::ResumeUnwind => {
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
                VMOpCode::ClearException => {
                    frame.active_exception_stack.pop();
                }
            }
        }
    }

    // --- Higher-order function builtins (map, filter, fold, reduce) ---

    /// Try to build a HofCall from a builtin name and arguments.
    /// Returns None if the name is not a HOF builtin.
    /// Stores the iterable Value directly; extraction happens in execute_hof
    /// via host.get_iter(), supporting all iterable types (list, tuple,
    /// range, set, dict, str, bytes).
    fn try_build_hof(name: &str, args: &[Value]) -> Option<HofCall> {
        match name {
            "map" if args.len() == 2 => Some(HofCall {
                kind: HofKind::Map,
                func: args[0],
                iterable: args[1],
                init: None,
            }),
            "filter" if args.len() == 2 => Some(HofCall {
                kind: HofKind::Filter,
                func: args[0],
                iterable: args[1],
                init: None,
            }),
            "fold" if args.len() == 3 => Some(HofCall {
                kind: HofKind::Fold,
                func: args[2],
                iterable: args[0],
                init: Some(args[1]),
            }),
            "reduce" if args.len() == 2 => Some(HofCall {
                kind: HofKind::Reduce,
                func: args[1],
                iterable: args[0],
                init: None,
            }),
            _ => None,
        }
    }

    // --- isinstance ---

    /// Check if `obj` is an instance of `type_spec`.
    /// type_spec can be a struct type, a builtin type name (NativeStr), or a
    /// tuple of type specs (OR semantics).
    fn check_isinstance(&self, obj: Value, type_spec: Value) -> VMResult<Value> {
        Ok(Value::from_bool(self.isinstance_check(obj, type_spec)?))
    }

    fn isinstance_check(&self, obj: Value, type_spec: Value) -> VMResult<bool> {
        // Tuple of types: OR semantics
        if type_spec.is_native_tuple() {
            // SAFETY: the is_native_tuple() guard above proves type_spec holds a live Arc<NativeTuple> owned on the stack; the borrow does not outlive it.
            let tuple = unsafe { type_spec.as_native_tuple_ref().unwrap() };
            for i in 0..tuple.len() {
                let t = tuple.as_slice()[i];
                if self.isinstance_check(obj, t)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        // Struct type: check instance type_id against type hierarchy (MRO)
        if type_spec.is_struct_type() {
            let target_id = type_spec.as_struct_type_id().unwrap();
            if !obj.is_struct_instance() {
                return Ok(false);
            }
            // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
            let cell = unsafe { obj.as_struct_ref() }.unwrap();
            // Direct match
            if cell.type_id == target_id {
                return Ok(true);
            }
            // Check MRO by stable type id (inheritance chain)
            if let Some(source) = self.struct_registry.get_type(cell.type_id) {
                return Ok(source.mro_ids.contains(&target_id));
            }
            return Ok(false);
        }

        // Builtin type name as NativeStr: compare with obj.type_name()
        if type_spec.is_native_str() {
            // SAFETY: the is_native_str() guard above proves type_spec holds a live Arc<NativeString> owned on the stack; the borrow does not outlive it.
            let name = unsafe { type_spec.as_native_str_ref().unwrap() };
            return Ok(obj.type_name() == name);
        }

        Err(VMError::TypeError(
            "isinstance() arg 2 must be a type, a string type name, or a tuple of types".into(),
        ))
    }

    /// Collect all items from a host iterator into a Vec.
    /// Each item carries +1 refcount from the iterator; caller must decref.
    fn collect_iterable(iter: &mut dyn crate::host::ValueIter) -> VMResult<Vec<Value>> {
        let mut items = Vec::new();
        while let Some(v) = iter.next_value()? {
            items.push(v);
        }
        Ok(items)
    }

    /// Release stack-owned operands on an early-return error path.
    ///
    /// A call opcode pops its operands (taking ownership) before deciding how to
    /// dispatch. On the frame-transfer success branches `bind_args` consumes
    /// them; on an error branch that never reaches the transfer the operands are
    /// still owned and must be released, or every pointer operand (string, list,
    /// dict, ...) routed through a failing call leaks for the VM's lifetime.
    #[inline]
    fn release_operands(operands: &[Value]) {
        for v in operands {
            v.decref();
        }
    }

    /// Prepend a bound method's curried receiver to owned `args`, cloning it so
    /// the func slot keeps its own ref for repeated calls (`bind_args` consumes
    /// the returned Vec). A no-op returning `args` unchanged when the slot is a
    /// plain function/closure. `self` is the method's first parameter, so every
    /// VMFunc-dispatch site that binds owned args must funnel through here (Call,
    /// TailCall, CallKw) -- the receiver is a curried arg, not a closure capture.
    #[inline]
    fn with_bound_receiver(bound_self: Option<Value>, mut args: Vec<Value>) -> Vec<Value> {
        if let Some(bself) = bound_self {
            bself.clone_refcount();
            args.insert(0, bself);
        }
        args
    }

    /// Call a function value synchronously via re-entrant dispatch.
    /// Enforces the same MAX_FRAME_DEPTH limit as the normal Call handler.
    fn call_func_sync(&mut self, func: Value, args: &[Value], host: &dyn VmHost) -> VMResult<Value> {
        // `func` is borrowed (owned and released by the caller), so no decref here.
        if let Some((callee_code, closure, bound_self)) = self.callable_slot_data(func) {
            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                return Err(VMError::FrameOverflow);
            }
            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
            for a in args {
                a.clone_refcount();
            }
            // A bound method (passed to a HOF as a first-class value) curries its
            // receiver as the first arg. `args` is borrowed and already cloned
            // above, so build an owned list only when a receiver must be prepended.
            if let Some(bself) = bound_self {
                bself.clone_refcount();
                let mut full = Vec::with_capacity(1 + args.len());
                full.push(bself);
                full.extend_from_slice(args);
                new_frame.bind_args(&full);
            } else {
                new_frame.bind_args(args);
            }
            new_frame.closure_scope = closure;
            self.dispatch(new_frame, host)
        } else {
            // Builtin or host-provided function
            host.call_function(func, args)
        }
    }

    /// Decref all items extracted by `extract_iterable`.
    fn decref_items(items: &[Value]) {
        for item in items {
            item.decref();
        }
    }

    /// Execute a higher-order builtin (map/filter/fold/reduce).
    ///
    /// map/filter return eager lists (not lazy iterators) because PureVM
    /// has no suspended-iterator Value tag. The PyO3 pipeline uses Python's
    /// lazy builtins; see docs/dev/VM.md "Divergence pipeline Python".
    ///
    /// Owns refcounts for: callable (`.func`), iterable (`.iterable`),
    /// and fold init (`.init`). Items are collected via `host.get_iter()`
    /// which supports all PureVM iterable types. Every ref is released
    /// exactly once before returning, on both success and error paths.
    fn execute_hof(&mut self, hof: HofCall, host: &dyn VmHost) -> VMResult<Value> {
        let func = hof.func;
        // Collect items from the iterable via the host iterator protocol.
        let items = match host.get_iter(hof.iterable) {
            Ok(mut iter) => {
                hof.iterable.decref();
                match Self::collect_iterable(iter.as_mut()) {
                    Ok(items) => items,
                    Err(e) => {
                        func.decref();
                        if let Some(init) = hof.init {
                            init.decref();
                        }
                        return Err(e);
                    }
                }
            }
            Err(e) => {
                hof.iterable.decref();
                func.decref();
                if let Some(init) = hof.init {
                    init.decref();
                }
                return Err(e);
            }
        };

        let result = match hof.kind {
            HofKind::Map => {
                let mut results = Vec::with_capacity(items.len());
                let mut err = None;
                for item in &items {
                    match self.call_func_sync(func, &[*item], host) {
                        Ok(r) => results.push(r),
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                Self::decref_items(&items);
                if let Some(e) = err {
                    for r in &results {
                        r.decref();
                    }
                    Err(e)
                } else {
                    Ok(Value::from_list(results))
                }
            }
            HofKind::Filter => {
                let mut results = Vec::new();
                let mut err = None;
                for item in &items {
                    match self.call_func_sync(func, &[*item], host) {
                        Ok(keep) => {
                            if keep.is_truthy() {
                                item.clone_refcount();
                                results.push(*item);
                            }
                            keep.decref();
                        }
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                Self::decref_items(&items);
                if let Some(e) = err {
                    for r in &results {
                        r.decref();
                    }
                    Err(e)
                } else {
                    Ok(Value::from_list(results))
                }
            }
            HofKind::Fold => {
                let mut acc = hof.init.unwrap_or(Value::NIL);
                let mut err = None;
                for item in &items {
                    match self.call_func_sync(func, &[acc, *item], host) {
                        Ok(new_acc) => {
                            acc.decref();
                            acc = new_acc;
                        }
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                Self::decref_items(&items);
                if let Some(e) = err {
                    acc.decref();
                    Err(e)
                } else {
                    Ok(acc)
                }
            }
            HofKind::Reduce => {
                if items.is_empty() {
                    func.decref();
                    return Err(VMError::ValueError(
                        "reduce() of empty sequence with no initial value".into(),
                    ));
                }
                // items[0]'s iterator ref becomes the initial accumulator.
                let mut acc = items[0];
                let mut err = None;
                for item in &items[1..] {
                    match self.call_func_sync(func, &[acc, *item], host) {
                        Ok(new_acc) => {
                            acc.decref();
                            acc = new_acc;
                        }
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                Self::decref_items(&items[1..]);
                if let Some(e) = err {
                    acc.decref();
                    Err(e)
                } else {
                    Ok(acc)
                }
            }
        };
        func.decref();
        result
    }

    // --- Exception unwinding ---

    /// Try to unwind to a handler in the current frame, or walk up the call stack.
    /// Only pops frames above `base_depth` (re-entrant dispatch safety).
    fn unwind_exception(&mut self, frame: &mut PureFrame, err: &VMError, base_depth: usize) -> bool {
        // Try current frame
        if self.try_unwind_to_handler(frame, err) {
            return true;
        }
        // For catchable exceptions, walk up the call stack (above base_depth only)
        if err.is_catchable() {
            while self.frame_stack.len() > base_depth {
                let caller = self.frame_stack.pop().unwrap();
                let old = std::mem::replace(frame, caller);
                self.frame_pool.free(old);
                if self.try_unwind_to_handler(frame, err) {
                    return true;
                }
            }
        }
        false
    }

    /// Try to find and activate a handler in the current frame.
    fn try_unwind_to_handler(&mut self, frame: &mut PureFrame, err: &VMError) -> bool {
        while let Some(handler) = frame.handler_stack.last() {
            match handler.handler_type {
                catnip_core::exception::HandlerType::Except => {
                    if err.is_catchable() {
                        let handler = frame.handler_stack.pop().unwrap();
                        if let Some(info) = err.exception_info() {
                            frame.active_exception_stack.push(info);
                        }
                        for i in handler.stack_depth..frame.stack.len() {
                            frame.stack[i].decref();
                        }
                        frame.stack.truncate(handler.stack_depth);
                        for (_slot_start, saved) in frame.block_stack.drain(handler.block_depth..) {
                            for val in saved {
                                val.decref();
                            }
                        }
                        frame.ip = handler.target_addr;
                        return true;
                    }
                    // Control flow signal: skip Except handler
                    frame.handler_stack.pop();
                }
                catnip_core::exception::HandlerType::Finally => {
                    let handler = frame.handler_stack.pop().unwrap();
                    frame.pending_unwind = Some(err.to_pending_unwind());
                    for i in handler.stack_depth..frame.stack.len() {
                        frame.stack[i].decref();
                    }
                    frame.stack.truncate(handler.stack_depth);
                    for (_slot_start, saved) in frame.block_stack.drain(handler.block_depth..) {
                        for val in saved {
                            val.decref();
                        }
                    }
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

    // --- Helpers ---

    /// Resolve a name through the scope chain: frame closure → host globals.
    fn resolve_scope(&self, name: &str, frame: &PureFrame, host: &dyn VmHost) -> VMResult<Value> {
        // Check closure scope first
        if let Some(ref cs) = frame.closure_scope {
            if let Some(val) = cs.resolve(name) {
                return Ok(val);
            }
        }
        // Then globals via host
        if let Some(val) = host.lookup_global(name)? {
            return Ok(val);
        }
        Err(VMError::NameError(name.to_string()))
    }

    /// Store a value in the nearest enclosing scope that already has it.
    fn store_scope(&self, name: &str, value: Value, frame: &PureFrame) -> bool {
        if let Some(ref cs) = frame.closure_scope {
            if cs.set(name, value) {
                return true;
            }
        }
        false
    }

    /// Re-sync the module frame's local slots from the globals, so a global
    /// mutated while a callee ran is reflected in the slot the module reads via
    /// `LoadLocal`. Refcount-correct: the slot drops its previous value and takes
    /// a ref on the global it now aliases. Called only on return to the module
    /// top-level frame (empty call stack), never to a function frame (which would
    /// clobber its locals with stale globals).
    fn sync_module_slots_from_globals(&self, frame: &mut PureFrame, host: &dyn VmHost) {
        let Some(code) = frame.code.clone() else {
            return;
        };
        let updates: Vec<(usize, Value)> = code
            .slotmap
            .iter()
            .filter_map(|(name, &slot)| {
                let g = host.lookup_global(name).ok().flatten()?;
                let old = frame.get_local(slot);
                (old.bits() != g.bits()).then_some((slot, g))
            })
            .collect();
        for (slot, g) in updates {
            let old = frame.get_local(slot);
            g.clone_refcount();
            old.decref();
            frame.set_local(slot, g);
        }
    }

    /// Get a Globals Rc from the host (for building closure parents).
    fn get_globals_rc(&self, host: &dyn VmHost) -> Option<crate::host::Globals> {
        host.globals_rc()
    }

    /// Boundary check for a nominal-typed param (`CheckNominal`). Returns
    /// `Ok(())` when `val` is an instance of the type named `name` -- with
    /// subtyping (MRO + implemented traits) and tagged-union membership -- or
    /// when `name` is not a known nominal type at runtime, in which case the
    /// annotation is inert (a composite like `list`, or an unknown name).
    /// Returns a `TypeError` when `name` is a known nominal but `val` is not a
    /// member. No coercion: a nominal value is never rewritten.
    fn check_nominal(&self, val: Value, name: &str, host: &dyn VmHost) -> VMResult<()> {
        if self.value_is_nominal_member(val, name) {
            return Ok(());
        }
        if self.name_is_known_nominal(name, host) {
            return Err(VMError::TypeError(format!(
                "typed parameter expects '{}' but got '{}'",
                name,
                self.nominal_value_type_name(val)
            )));
        }
        Ok(())
    }

    /// Boundary check for a type-union param (`CheckUnion`). Accepts `val` when it
    /// satisfies any member: a primitive member by the numeric tower (no coercion,
    /// [`primitive_membership`]), a nominal member by the same subtyping rule as
    /// [`Self::value_is_nominal_member`]. A nominal member whose name is unknown at
    /// runtime is inert (it can't match), mirroring `CheckNominal`. Raises a
    /// `TypeError` naming the union when no member matches.
    fn check_union(
        &self,
        val: Value,
        members: &[catnip_core::vm::opcode::ParamCheck],
        host: &dyn VmHost,
    ) -> VMResult<()> {
        use catnip_core::vm::opcode::{ParamCheck, format_union_members, primitive_membership};
        // A nominal member whose name is unknown at runtime can't be proven absent
        // (forward ref, conditionally-defined type), so -- like `CheckNominal` --
        // we stay inert rather than reject a possibly-valid value.
        let class = value_primitive_class(val);
        let mut unknown_nominal = false;
        for m in members {
            match m {
                ParamCheck::Primitive(code) => {
                    if primitive_membership(*code, &class) {
                        return Ok(());
                    }
                }
                ParamCheck::Nominal(name) => {
                    if self.value_is_nominal_member(val, name) {
                        return Ok(());
                    }
                    if !self.name_is_known_nominal(name, host) {
                        unknown_nominal = true;
                    }
                }
                // A composite member is checked in full (container + parameters),
                // mirroring `check_composite`.
                ParamCheck::Composite { .. } => {
                    if self.check_composite(val, m, host).is_ok() {
                        return Ok(());
                    }
                }
                // A generic-nominal member (`Option[int]`): a member of the union
                // with a matching payload accepts; a member with a mismatched
                // payload is not this alternative (keep trying); an unknown union
                // name keeps the whole check inert, like `Nominal`.
                ParamCheck::Generic { name, .. } => {
                    if self.value_is_nominal_member(val, name) {
                        if self.check_generic(val, m, host).is_ok() {
                            return Ok(());
                        }
                    } else if !self.name_is_known_nominal(name, host) {
                        unknown_nominal = true;
                    }
                }
                // A function-type member (`None | (int) -> int`): full
                // callability + arity acceptance, mirroring the prologue check.
                ParamCheck::Callable { arity } => {
                    if self.check_callable(val, *arity, host).is_ok() {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        if unknown_nominal {
            return Ok(());
        }
        Err(VMError::TypeError(format!(
            "typed parameter expects '{}' but got '{}'",
            format_union_members(members),
            self.nominal_value_type_name(val)
        )))
    }

    /// Boundary check for a composite param (`CheckComposite`). Checks the
    /// container tag (`list` for `LIST`, `set` for `SET`, `dict` for `DICT`,
    /// `tuple` for `TUPLE`) and, when the spec carries parameters, that each
    /// element satisfies the corresponding parameter check. No coercion. A bare
    /// composite (no params) or an inert parameter (unenforceable type) skips the
    /// element pass. `tuple` is positional: `params.len()` is the enforced arity
    /// and position `i` is checked against `params[i]`; the others are
    /// homogeneous. Read-only: list/tuple elements and dict values are snapshotted
    /// without touching refcounts (the container is alive on the stack); set
    /// elements and dict keys are checked via `key_satisfies` (which materializes
    /// non-struct keys with a balanced `to_value`/`decref` and resolves struct
    /// keys from their `type_id` without rebuilding).
    fn check_composite(
        &self,
        val: Value,
        spec: &catnip_core::vm::opcode::ParamCheck,
        host: &dyn VmHost,
    ) -> VMResult<()> {
        use catnip_core::vm::opcode::{ParamCheck, format_param_check, primitive_membership, type_code};
        let ParamCheck::Composite { head, params } = spec else {
            return Ok(());
        };
        // Container tag.
        if !primitive_membership(*head, &value_primitive_class(val)) {
            return Err(VMError::TypeError(format!(
                "typed parameter expects '{}' but got '{}'",
                format_param_check(spec),
                self.nominal_value_type_name(val)
            )));
        }
        let enforced = |p: &ParamCheck| !matches!(p, ParamCheck::None);
        match *head {
            type_code::LIST => {
                if let Some(elem) = params.first().filter(|p| enforced(p)) {
                    // SAFETY: val is the boundary-checked param alive on the stack and its list tag was confirmed above; a Some borrow points to a live Arc<NativeList> and does not outlive val.
                    if let Some(list) = unsafe { val.as_native_list_ref() } {
                        for it in list.snapshot_items() {
                            if !self.value_satisfies(it, elem, host) {
                                return Err(VMError::TypeError(format!(
                                    "typed parameter expects '{}' but an element has the wrong type",
                                    format_param_check(spec)
                                )));
                            }
                        }
                    }
                }
            }
            type_code::SET => {
                if let Some(elem) = params.first().filter(|p| enforced(p)) {
                    // SAFETY: val is the boundary-checked param alive on the stack and its set tag was confirmed above; a Some borrow points to a live Arc<NativeSet> and does not outlive val.
                    if let Some(set) = unsafe { val.as_native_set_ref() } {
                        // Set elements are stored as hashable keys (like dict keys);
                        // `key_satisfies` checks each without rebuilding struct keys.
                        for k in set.keys_cloned() {
                            if !self.key_satisfies(&k, elem, host) {
                                return Err(VMError::TypeError(format!(
                                    "typed parameter expects '{}' but an element has the wrong type",
                                    format_param_check(spec)
                                )));
                            }
                        }
                    }
                }
            }
            type_code::TUPLE => {
                // Positional: `params.len()` is the enforced arity, and position
                // `i` is checked against `params[i]`. A bare `tuple` (no params)
                // checks only the container. Elements are plain `Value`s (like a
                // list, not hashable keys), read directly from the live tuple's
                // slice without touching refcounts.
                if !params.is_empty() {
                    // SAFETY: val is the boundary-checked param alive on the stack and its tuple tag was confirmed above; a Some borrow points to a live Arc<NativeTuple> and does not outlive val.
                    if let Some(tuple) = unsafe { val.as_native_tuple_ref() } {
                        let items = tuple.as_slice();
                        if items.len() != params.len() {
                            return Err(VMError::TypeError(format!(
                                "typed parameter expects '{}' but got a tuple of length {}",
                                format_param_check(spec),
                                items.len()
                            )));
                        }
                        for (it, p) in items.iter().zip(params.iter()) {
                            if !self.value_satisfies(*it, p, host) {
                                return Err(VMError::TypeError(format!(
                                    "typed parameter expects '{}' but an element has the wrong type",
                                    format_param_check(spec)
                                )));
                            }
                        }
                    }
                }
            }
            type_code::DICT if params.len() == 2 => {
                let (kc, vc) = (&params[0], &params[1]);
                // SAFETY: val is the boundary-checked param alive on the stack and its dict tag was confirmed above; a Some borrow points to a live Arc<NativeDict> and does not outlive val.
                if let Some(dict) = unsafe { val.as_native_dict_ref() } {
                    if enforced(kc) {
                        for k in dict.keys_cloned() {
                            if !self.key_satisfies(&k, kc, host) {
                                return Err(VMError::TypeError(format!(
                                    "typed parameter expects '{}' but a key has the wrong type",
                                    format_param_check(spec)
                                )));
                            }
                        }
                    }
                    if enforced(vc) {
                        for v in dict.snapshot_values() {
                            if !self.value_satisfies(v, vc, host) {
                                return Err(VMError::TypeError(format!(
                                    "typed parameter expects '{}' but a value has the wrong type",
                                    format_param_check(spec)
                                )));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Boundary check for a generic-nominal param (`CheckGeneric`, `Option[int]`).
    /// First the union membership (same rule as `CheckNominal`: an unknown union
    /// name is inert, a known one with a non-member value is a `TypeError`). Then
    /// the parametric payload: for the value's variant, each payload field carries
    /// a [`FieldTemplate`] -- `Param(k)` requires the k-th use-site type argument,
    /// `Fixed(check)` a concrete check -- validated against the field value with
    /// [`Self::value_satisfies`]. A nullary variant (a symbol, `Option.None`) has
    /// no payload and passes. Read-only: field values are inspected in place
    /// without touching refcounts (the instance is alive on the stack).
    fn check_generic(&self, val: Value, spec: &catnip_core::vm::opcode::ParamCheck, host: &dyn VmHost) -> VMResult<()> {
        use catnip_core::vm::opcode::{FieldTemplate, ParamCheck, format_param_check};
        let ParamCheck::Generic { name, args } = spec else {
            return Ok(());
        };
        // Union membership (mirror `check_nominal`).
        if !self.value_is_nominal_member(val, name) {
            if self.name_is_known_nominal(name, host) {
                return Err(VMError::TypeError(format!(
                    "typed parameter expects '{}' but got '{}'",
                    format_param_check(spec),
                    self.nominal_value_type_name(val)
                )));
            }
            return Ok(()); // unknown union -> inert, like `CheckNominal`
        }
        // Only a struct-instance variant carries a payload; a nullary variant
        // (symbol) or any non-instance member has nothing to substitute.
        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
        let Some(cell) = (unsafe { val.as_struct_ref() }) else {
            return Ok(());
        };
        let Some(templates) = self.struct_registry.variant_templates(cell.type_id) else {
            return Ok(()); // no templates -> membership-only (non-generic union)
        };
        for (i, tmpl) in templates.iter().enumerate() {
            let Some(fval) = cell.fields.get(i).map(|c| c.get()) else {
                break;
            };
            let required: Option<&ParamCheck> = match tmpl {
                // A type parameter left without a use-site argument (defensive:
                // static arity checking rules this out) is inert.
                FieldTemplate::Param(k) => args.get(*k),
                FieldTemplate::Fixed(c) => Some(c),
            };
            if let Some(check) = required {
                if !matches!(check, ParamCheck::None) && !self.value_satisfies(fval, check, host) {
                    return Err(VMError::TypeError(format!(
                        "typed parameter expects '{}' but a payload field has the wrong type",
                        format_param_check(spec)
                    )));
                }
            }
        }
        Ok(())
    }

    /// Whether a single value satisfies a `ParamCheck` (used per element by
    /// `check_composite`). Primitives by the numeric tower, nominals by subtyping
    /// (an unknown nominal name is inert -> satisfied, like `CheckNominal`), unions
    /// by any member, composites recursively. No coercion, read-only.
    /// Read-only validation of a constructor field value against its annotation.
    /// Mirrors [`Self::value_satisfies`] but additionally rejects a bigint that
    /// would overflow `f64` when the slot is `float` -- the one input
    /// [`boundary_coerce`] fails on -- so the constructor can validate first and
    /// coerce second without a fallible step after a refcount mutation.
    fn field_value_ok(&self, val: Value, check: &catnip_core::vm::opcode::ParamCheck, host: &dyn VmHost) -> bool {
        use catnip_core::vm::opcode::{ParamCheck, type_code};
        if let ParamCheck::Primitive(type_code::FLOAT) = check {
            if val.is_bigint() {
                // SAFETY: is_bigint() guards the Arc<Integer> payload.
                return unsafe { val.as_bigint_ref() }.unwrap().to_f64().is_finite();
            }
        }
        self.value_satisfies(val, check, host)
    }

    /// Function-type boundary (FT3). Callable + declared-arity acceptance
    /// (the shared `callable_arity_accepts` rule, so executors cannot drift):
    ///
    /// - a VM function: fixed slots = `nargs` (or `vararg_idx` when variadic),
    ///   a real default is a non-nil entry of the padded defaults table;
    /// - a struct constructor: fixed slots = field count, defaulted fields
    ///   are the defaults;
    /// - a builtin-by-name: a `NativeString` naming a known builtin or a host
    ///   global is callable-only (no structured arity); any other string is
    ///   NOT a callable (parity with the PyO3/AST executors, where a plain
    ///   str is rejected);
    /// - anything else is not callable: `TypeError`.
    ///
    /// Parameter/return types are NOT checked (observable only at calls).
    fn check_callable(&self, val: Value, arity: u32, host: &dyn VmHost) -> Result<(), VMError> {
        use catnip_core::vm::opcode::callable_arity_accepts;
        let arity = arity as usize;
        let code = if val.is_closure() {
            // SAFETY: is_closure() proves a live Arc<PureFuncSlot>; we clone the code Arc out of the borrow.
            Some(unsafe { Arc::clone(&val.as_closure_ref().unwrap().code) })
        } else if val.is_vmfunc() && !val.is_invalid() {
            Some(Arc::clone(
                &self
                    .func_table
                    .get(val.as_vmfunc_idx())
                    .ok_or_else(|| VMError::RuntimeError("invalid function index".into()))?
                    .code,
            ))
        } else {
            None
        };
        if let Some(code) = code {
            let real_defaults = code.defaults.iter().filter(|v| !v.is_nil()).count();
            let has_vararg = code.vararg_idx >= 0;
            let fixed = if has_vararg {
                code.vararg_idx as usize
            } else {
                code.nargs
            };
            let (required, accepts) = callable_arity_accepts(fixed, has_vararg, real_defaults, arity);
            if !accepts {
                return Err(VMError::TypeError(format!(
                    "typed parameter expects a callable taking {arity} argument(s) but the function requires {required}"
                )));
            }
            return Ok(());
        }
        if val.is_struct_type() {
            if let Some(type_id) = val.as_struct_type_id() {
                if let Some(ty) = self.struct_registry.get_type(type_id) {
                    let fixed = ty.fields.len();
                    let defaults = fixed - ty.required_field_count();
                    let (required, accepts) = callable_arity_accepts(fixed, false, defaults, arity);
                    if accepts {
                        return Ok(());
                    }
                    return Err(VMError::TypeError(format!(
                        "typed parameter expects a callable taking {arity} argument(s) but the constructor requires {required}"
                    )));
                }
            }
            return Ok(());
        }
        if val.is_native_str() {
            // SAFETY: is_native_str() guards the payload.
            let name = unsafe { val.as_native_str_ref() }.unwrap_or_default();
            if crate::host::BUILTIN_NAMES.contains(&name) || host.has_global(name) {
                return Ok(());
            }
        }
        Err(VMError::TypeError(format!(
            "typed parameter expects a callable taking {arity} argument(s) but got a non-callable value"
        )))
    }

    fn value_satisfies(&self, val: Value, check: &catnip_core::vm::opcode::ParamCheck, host: &dyn VmHost) -> bool {
        use catnip_core::vm::opcode::{ParamCheck, primitive_membership};
        match check {
            ParamCheck::None => true,
            ParamCheck::Primitive(code) => primitive_membership(*code, &value_primitive_class(val)),
            ParamCheck::Nominal(name) => {
                self.value_is_nominal_member(val, name) || !self.name_is_known_nominal(name, host)
            }
            ParamCheck::Union(members) => members.iter().any(|m| self.value_satisfies(val, m, host)),
            ParamCheck::Composite { .. } => self.check_composite(val, check, host).is_ok(),
            ParamCheck::Generic { .. } => self.check_generic(val, check, host).is_ok(),
            ParamCheck::Callable { arity } => self.check_callable(val, *arity, host).is_ok(),
        }
    }

    /// Whether a dict/set key satisfies `check`. Mirrors `value_satisfies`, but a
    /// key that holds a struct snapshot (directly, or nested in a tuple) cannot be
    /// rebuilt by `to_value` -- materializing a struct needs the registry
    /// (`&mut self`), which a `&self` type check does not have and does not need.
    /// A struct-bearing key is checked from its `type_id`; every other key
    /// materializes as before. (`to_value` panics on a struct key, so the guard
    /// must come before it.)
    fn key_satisfies(&self, key: &ValueKey, check: &catnip_core::vm::opcode::ParamCheck, host: &dyn VmHost) -> bool {
        use catnip_core::vm::opcode::{FieldTemplate, ParamCheck};
        if !Self::key_has_struct(key) {
            let kv = key.to_value();
            let ok = self.value_satisfies(kv, check, host);
            kv.decref();
            return ok;
        }
        // A struct-bearing key matches only a nominal (or a union over nominals);
        // it is never a primitive nor a list/set/dict container.
        match check {
            ParamCheck::None => true,
            ParamCheck::Nominal(name) => match key {
                ValueKey::Struct { type_id, .. } => {
                    self.type_id_is_nominal_member(*type_id, name) || !self.name_is_known_nominal(name, host)
                }
                // A tuple is not a nominal struct; it matches only an unknown name.
                _ => !self.name_is_known_nominal(name, host),
            },
            // A struct-bearing key is never callable (an instance snapshot, not
            // a function); the non-struct key path went through value_satisfies.
            ParamCheck::Callable { .. } => false,
            ParamCheck::Union(members) => members.iter().any(|m| self.key_satisfies(key, m, host)),
            // A struct-bearing key under a generic union: membership by `type_id`,
            // then the parametric payload substitution. Unlike an instance, a key
            // snapshot carries its field *keys* (`ValueKey::Struct.fields`), so the
            // payload is checked recursively -- parity with the value path, where a
            // `set[Option[int]]` element goes through `value_satisfies`.
            ParamCheck::Generic { name, args } => match key {
                ValueKey::Struct { type_id, fields, .. } => {
                    if !self.type_id_is_nominal_member(*type_id, name) {
                        return !self.name_is_known_nominal(name, host);
                    }
                    match self.struct_registry.variant_templates(*type_id) {
                        Some(templates) => templates.iter().enumerate().all(|(i, tmpl)| {
                            let required = match tmpl {
                                FieldTemplate::Param(k) => args.get(*k),
                                FieldTemplate::Fixed(c) => Some(c),
                            };
                            match required {
                                Some(c) if !matches!(c, ParamCheck::None) => {
                                    fields.get(i).is_none_or(|fk| self.key_satisfies(fk, c, host))
                                }
                                _ => true,
                            }
                        }),
                        None => true,
                    }
                }
                _ => !self.name_is_known_nominal(name, host),
            },
            ParamCheck::Primitive(_) | ParamCheck::Composite { .. } => false,
        }
    }

    /// True if `val` is a struct instance whose type name, MRO, or implemented
    /// traits include `name`, a tagged-union payload variant of `name` (struct
    /// named `name.Variant`), or an enum/union symbol whose type is `name`.
    fn value_is_nominal_member(&self, val: Value, name: &str) -> bool {
        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
        if let Some(cell) = unsafe { val.as_struct_ref() } {
            return self.type_id_is_nominal_member(cell.type_id, name);
        }
        if let Some(sym) = val.as_symbol() {
            if let Some(full) = self.symbol_table.resolve(sym) {
                let tyname = full.split_once('.').map(|(t, _)| t).unwrap_or(full);
                return tyname == name;
            }
        }
        false
    }

    /// Whether the struct type `type_id` (its name, MRO, implemented traits, or
    /// the tagged union it is a variant of) includes `name`. The struct-key
    /// analogue of [`value_is_nominal_member`]'s instance arm: a `ValueKey::Struct`
    /// snapshot carries its `type_id`, so a set/dict element check resolves the
    /// nominal relation without rebuilding the instance.
    fn type_id_is_nominal_member(&self, type_id: u32, name: &str) -> bool {
        self.struct_registry.get_type(type_id).is_some_and(|ty| {
            ty.name == name
                || ty.mro.iter().any(|n| n == name)
                || ty.implements.iter().any(|n| n == name)
                || ty.name.split_once('.').map(|(u, _)| u) == Some(name)
        })
    }

    /// True if `name` is a struct, enum, tagged union, or trait defined at
    /// runtime. Decides whether a non-member is a type error (known nominal) or
    /// an inert annotation (unknown name -> no-op). A trait annotation accepts a
    /// struct that implements it (via `value_is_nominal_member`); recognizing the
    /// trait here makes a non-implementer a type error rather than a no-op.
    /// `lookup_global` returns a copied (non-incref'd) Value, so the union marker
    /// is inspected without decref.
    fn name_is_known_nominal(&self, name: &str, host: &dyn VmHost) -> bool {
        self.struct_registry.find_type_id(name).is_some()
            || self.enum_registry.find_by_name(name).is_some()
            || self.trait_registry.get_trait(name).is_some()
            || matches!(host.lookup_global(name), Ok(Some(v)) if v.is_union_type())
    }

    /// Best-effort runtime type name of `val` for a boundary error message: the
    /// nominal type name for a struct/enum/union value, else the generic name.
    fn nominal_value_type_name(&self, val: Value) -> String {
        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
        if let Some(cell) = unsafe { val.as_struct_ref() } {
            return self.struct_registry.type_name(cell.type_id).to_string();
        }
        if let Some(sym) = val.as_symbol() {
            if let Some(full) = self.symbol_table.resolve(sym) {
                return full.to_string();
            }
        }
        val.type_name().to_string()
    }

    /// Unpack a value (list/tuple) into a Vec for UnpackSequence/UnpackEx.
    fn unpack_to_vec(&self, val: Value) -> VMResult<Vec<Value>> {
        if val.is_native_list() {
            // SAFETY: the is_native_list() guard above proves val holds a live Arc<NativeList> owned on the stack; the borrow does not outlive it.
            let list = unsafe { val.as_native_list_ref().unwrap() };
            return Ok(list.as_slice_cloned());
        }
        if val.is_native_tuple() {
            // SAFETY: the is_native_tuple() guard above proves val holds a live Arc<NativeTuple> owned on the stack; the borrow does not outlive it.
            let tuple = unsafe { val.as_native_tuple_ref().unwrap() };
            let items: Vec<Value> = tuple.as_slice().to_vec();
            for v in &items {
                v.clone_refcount();
            }
            return Ok(items);
        }
        Err(VMError::TypeError(format!(
            "cannot unpack non-iterable {}",
            val.type_name()
        )))
    }

    /// Register a function code object and return its VMFunc index.
    /// Used by external callers (e.g. compiler integration).
    pub fn register_function(&mut self, code: Arc<CodeObject>) -> u32 {
        self.func_table.insert(PureFuncSlot::template(code, None))
    }

    // --- Struct helpers ---

    /// Handle MakeStruct opcode: parse metadata, register type, store in globals.
    fn handle_make_struct(&mut self, info: Value, frame: &mut PureFrame, host: &dyn VmHost) -> VMResult<()> {
        // SAFETY: info is a constant held alive by the CodeObject pool; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
        let tuple = unsafe {
            info.as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeStruct: expected tuple constant".into()))?
        };
        let items = tuple.as_slice();
        if items.len() < 3 {
            return Err(VMError::RuntimeError("MakeStruct: malformed metadata".into()));
        }

        // Parse name
        // SAFETY: items[0] borrows the live constant tuple; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
        let name = unsafe {
            items[0]
                .as_native_str_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeStruct: name must be string".into()))?
                .to_string()
        };

        // Parse fields: tuple of (name, has_default) tuples
        // SAFETY: items[1] is the fields tuple in the compiler-emitted layout, a live constant; the tuple tag holds by construction, so the borrow points to a live Arc<NativeTuple>.
        let fields_tuple = unsafe { items[1].as_native_tuple_ref().unwrap() };
        let num_defaults = items[2].as_int().unwrap_or(0) as usize;

        let mut fields = Vec::new();
        let mut default_slot = 0usize;
        for fv in fields_tuple.as_slice() {
            // SAFETY: each field entry fv is a tuple in the emitted layout, a live constant element; the tuple tag holds by construction.
            let ft = unsafe { fv.as_native_tuple_ref().unwrap() };
            let fs = ft.as_slice();
            // SAFETY: fs[0] is the field name string in the emitted layout, a live constant element; the str tag holds by construction.
            let fname = unsafe { fs[0].as_native_str_ref().unwrap().to_string() };
            let has_default = fs[1].as_bool().unwrap_or(false);
            let slot = if has_default {
                let s = default_slot;
                default_slot += 1;
                Some(s)
            } else {
                None
            };
            // Classify the field's annotation text (entry element 2, present only
            // when annotated) into a runtime boundary check, like a param prologue.
            let check = match fs.get(2) {
                // SAFETY: fs[2] is the annotation string in the emitted layout, a live constant element; as_native_str_ref returns None unless it carries the str tag.
                Some(v) => match unsafe { v.as_native_str_ref() } {
                    Some(t) => catnip_core::vm::opcode::ParamCheck::from_annotation(t),
                    None => catnip_core::vm::opcode::ParamCheck::None,
                },
                None => catnip_core::vm::opcode::ParamCheck::None,
            };
            fields.push(PureStructField {
                name: fname,
                has_default,
                default_slot: slot,
                check,
            });
        }

        // Pop default values from stack (pushed by compiler)
        let mut defaults = vec![Value::NIL; num_defaults];
        for i in (0..num_defaults).rev() {
            defaults[i] = frame.pop();
        }

        // Determine tuple layout: 3 items = no methods, 4+ = check for implements/bases/methods
        let mut methods_map = IndexMap::new();
        let mut static_methods_map = IndexMap::new();
        let mut init_fn = None;
        let mut implements = Vec::new();
        let mut parent_names = Vec::new();
        let mut abstract_methods = std::collections::HashSet::new();

        // Layout variants:
        // (name, fields, num_defaults) -- no methods
        // (name, fields, num_defaults, methods_list) -- with methods, no implements/bases
        // (name, fields, num_defaults, impl_tuple, bases_val, [methods_list]) -- with implements/bases
        if items.len() >= 5 {
            // Has implements and/or bases
            // SAFETY: items[3] is a live constant; as_native_tuple_ref returns None unless it carries the tuple tag, so a Some borrow points to a live Arc<NativeTuple>.
            let impl_tuple = unsafe { items[3].as_native_tuple_ref() };
            if let Some(impls) = impl_tuple {
                for iv in impls.as_slice() {
                    // SAFETY: iv is a live constant element; as_native_str_ref returns None unless it carries the str tag, so a Some borrow points to a live Arc<NativeString>.
                    if let Some(s) = unsafe { iv.as_native_str_ref() } {
                        implements.push(s.to_string());
                    }
                }
            }
            if !items[4].is_nil() {
                // SAFETY: items[4] is a live constant; as_native_tuple_ref returns None unless it carries the tuple tag, so a Some borrow points to a live Arc<NativeTuple>.
                if let Some(bases) = unsafe { items[4].as_native_tuple_ref() } {
                    for bv in bases.as_slice() {
                        // SAFETY: bv is a live constant element; as_native_str_ref returns None unless it carries the str tag, so a Some borrow points to a live Arc<NativeString>.
                        if let Some(s) = unsafe { bv.as_native_str_ref() } {
                            parent_names.push(s.to_string());
                        }
                    }
                }
            }
            if items.len() >= 6 {
                self.parse_methods_list(
                    items[5],
                    &mut methods_map,
                    &mut static_methods_map,
                    &mut init_fn,
                    &mut abstract_methods,
                )?;
            }
        } else if items.len() == 4 {
            // Methods list (no implements/bases)
            self.parse_methods_list(
                items[3],
                &mut methods_map,
                &mut static_methods_map,
                &mut init_fn,
                &mut abstract_methods,
            )?;
        }

        // Resolve traits (implements)
        if !implements.is_empty() {
            let (trait_fields, trait_methods, trait_statics, trait_abstract) =
                self.trait_registry.resolve_for_struct(&implements)?;
            // Merge trait fields (prepend, before struct's own fields)
            let existing: std::collections::HashSet<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            let mut prepend_fields = Vec::new();
            let mut prepend_defaults = Vec::new();
            for tf in &trait_fields {
                if !existing.contains(tf.name.as_str()) {
                    let has_default = tf.has_default;
                    let slot = if has_default {
                        // The implementing type owns its own ref on the trait's
                        // default (both registries release at Drop).
                        tf.default.clone_refcount();
                        prepend_defaults.push(tf.default);
                        Some(prepend_defaults.len() - 1)
                    } else {
                        None
                    };
                    prepend_fields.push(PureStructField {
                        name: tf.name.clone(),
                        has_default,
                        default_slot: slot,
                        // Trait fields carry no enforced type annotation in v1.
                        check: catnip_core::vm::opcode::ParamCheck::None,
                    });
                }
            }
            if !prepend_fields.is_empty() {
                let offset = prepend_defaults.len();
                for f in &mut fields {
                    if let Some(ref mut s) = f.default_slot {
                        *s += offset;
                    }
                }
                prepend_defaults.extend(defaults);
                defaults = prepend_defaults;
                prepend_fields.extend(fields);
                fields = prepend_fields;
            }
            // Merge trait methods (trait provides, struct overrides)
            for (k, v) in &trait_methods {
                if !methods_map.contains_key(k) {
                    methods_map.insert(k.clone(), *v);
                }
            }
            for (k, v) in &trait_statics {
                if !static_methods_map.contains_key(k) {
                    static_methods_map.insert(k.clone(), *v);
                }
            }
            // Track abstract methods not implemented by struct
            for a in &trait_abstract {
                if !methods_map.contains_key(a) && !static_methods_map.contains_key(a) {
                    abstract_methods.insert(a.clone());
                }
            }
            // Remove from abstract if struct implements them
            abstract_methods.retain(|a| !methods_map.contains_key(a) && !static_methods_map.contains_key(a));
        }

        // Compute MRO
        let mro = if parent_names.is_empty() {
            vec![name.clone()]
        } else {
            match catnip_core::vm::mro::c3_linearize(&name, &parent_names, |n| {
                self.struct_registry.find_type_by_name(n).map(|ty| ty.mro.clone())
            }) {
                Ok(m) => m,
                Err(e) => return Err(VMError::RuntimeError(e)),
            }
        };

        // Merge parent fields/methods for inheritance
        if !parent_names.is_empty() {
            // Walk MRO skipping self (first entry), most derived parent first
            for ancestor_name in mro.iter().skip(1) {
                if let Some(parent_ty) = self.struct_registry.find_type_by_name(ancestor_name) {
                    // Inherit fields (prepend parent fields not already defined)
                    let existing_names: std::collections::HashSet<&str> =
                        fields.iter().map(|f| f.name.as_str()).collect();
                    let mut inherited_fields = Vec::new();
                    for pf in &parent_ty.fields {
                        if !existing_names.contains(pf.name.as_str()) {
                            inherited_fields.push(pf.clone());
                        }
                    }
                    // Prepend parent defaults for inherited default fields
                    if !inherited_fields.is_empty() {
                        let mut inherited_defaults = Vec::new();
                        for f in &inherited_fields {
                            if let Some(slot) = f.default_slot {
                                let d = parent_ty.defaults[slot];
                                // Each type owns one ref per heap default
                                // (released at registry Drop): the inherited
                                // copy takes its own.
                                d.clone_refcount();
                                inherited_defaults.push(d);
                            }
                        }
                        // Re-number default slots
                        let offset = inherited_defaults.len();
                        for f in &mut fields {
                            if let Some(ref mut s) = f.default_slot {
                                *s += offset;
                            }
                        }
                        inherited_defaults.extend(defaults);
                        defaults = inherited_defaults;
                        // Re-assign default_slot for inherited fields
                        let mut dslot = 0usize;
                        for f in &mut inherited_fields {
                            if f.has_default {
                                f.default_slot = Some(dslot);
                                dslot += 1;
                            }
                        }
                        inherited_fields.extend(fields);
                        fields = inherited_fields;
                    }

                    // Inherit methods (parent methods, child overrides)
                    for (mn, &mi) in &parent_ty.methods {
                        if !methods_map.contains_key(mn) {
                            methods_map.insert(mn.clone(), mi);
                        }
                    }
                    for (mn, &mi) in &parent_ty.static_methods {
                        if !static_methods_map.contains_key(mn) {
                            static_methods_map.insert(mn.clone(), mi);
                        }
                    }
                    if init_fn.is_none() {
                        init_fn = parent_ty.init_fn;
                    }
                }
            }
        }

        // Build mro_ids from mro names (skip self, resolve ancestors)
        let mro_ids: Vec<u32> = mro
            .iter()
            .skip(1) // self not yet registered
            .filter_map(|n| self.struct_registry.find_type_id(n))
            .collect();

        let type_id = self.struct_registry.register_type(PureStructType {
            id: 0,
            name: name.clone(),
            fields,
            defaults,
            methods: methods_map,
            static_methods: static_methods_map,
            init_fn,
            implements,
            mro,
            mro_ids,
            parent_names,
            abstract_methods,
        });

        // Push struct type and store in globals
        let val = Value::from_struct_type(type_id);
        frame.push(val);
        host.store_global(&name, val);
        Ok(())
    }

    /// Parse the methods list from a MakeStruct constant.
    fn parse_methods_list(
        &self,
        methods_val: Value,
        methods_map: &mut IndexMap<String, u32>,
        static_methods_map: &mut IndexMap<String, u32>,
        init_fn: &mut Option<u32>,
        abstract_methods: &mut std::collections::HashSet<String>,
    ) -> VMResult<()> {
        // SAFETY: methods_val is a live constant; the accessor checks the list tag (errors otherwise), so the borrow points to a live Arc<NativeList>.
        let list = unsafe {
            methods_val
                .as_native_list_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeStruct: methods must be a list".into()))?
        };
        let items = list.as_slice_cloned();
        for entry in &items {
            // SAFETY: entry is a live element of the cloned methods list; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
            let t = unsafe {
                entry
                    .as_native_tuple_ref()
                    .ok_or_else(|| VMError::RuntimeError("MakeStruct: method entry must be tuple".into()))?
            };
            let parts = t.as_slice();
            // SAFETY: parts[0] is the method name string in the emitted layout, a live element; the str tag holds by construction.
            let mname = unsafe { parts[0].as_native_str_ref().unwrap().to_string() };
            let is_static = if parts.len() > 2 {
                parts[2].as_bool().unwrap_or(false)
            } else {
                false
            };

            if parts[1].is_nil() {
                // Abstract method
                abstract_methods.insert(mname);
                continue;
            }

            let func_idx = if parts[1].is_vmfunc() {
                parts[1].as_vmfunc_idx()
            } else {
                continue;
            };

            if mname == "init" && !is_static {
                *init_fn = Some(func_idx);
            }

            if is_static {
                static_methods_map.insert(mname, func_idx);
            } else {
                methods_map.insert(mname, func_idx);
            }
        }
        for v in items {
            v.decref();
        }
        Ok(())
    }

    /// Handle MakeTrait opcode: parse metadata, register trait.
    fn handle_make_trait(&mut self, info: Value, frame: &mut PureFrame) -> VMResult<()> {
        use super::structs::{PureTraitDef, PureTraitField};

        // SAFETY: info is a constant held alive by the CodeObject pool; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
        let tuple = unsafe {
            info.as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeTrait: expected tuple constant".into()))?
        };
        let items = tuple.as_slice();
        if items.len() < 4 {
            return Err(VMError::RuntimeError("MakeTrait: malformed metadata".into()));
        }

        // SAFETY: items[0] is the trait name string in the emitted layout, a live constant; the str tag holds by construction.
        let name = unsafe { items[0].as_native_str_ref().unwrap().to_string() };

        // extends tuple
        let mut extends = Vec::new();
        // SAFETY: items[1] is a live constant; as_native_tuple_ref returns None unless it carries the tuple tag, so a Some borrow points to a live Arc<NativeTuple>.
        if let Some(ext) = unsafe { items[1].as_native_tuple_ref() } {
            for ev in ext.as_slice() {
                // SAFETY: ev is a live constant element; as_native_str_ref returns None unless it carries the str tag, so a Some borrow points to a live Arc<NativeString>.
                if let Some(s) = unsafe { ev.as_native_str_ref() } {
                    extends.push(s.to_string());
                }
            }
        }

        // fields
        // SAFETY: items[2] is the fields tuple in the emitted layout, a live constant; the tuple tag holds by construction, so the borrow points to a live Arc<NativeTuple>.
        let fields_tuple = unsafe { items[2].as_native_tuple_ref().unwrap() };
        let num_defaults = items[3].as_int().unwrap_or(0) as usize;

        let mut fields = Vec::new();
        let mut defaults_popped = vec![Value::NIL; num_defaults];
        for i in (0..num_defaults).rev() {
            defaults_popped[i] = frame.pop();
        }

        let mut default_idx = 0usize;
        for fv in fields_tuple.as_slice() {
            // SAFETY: each field entry fv is a tuple in the emitted layout, a live constant element; the tuple tag holds by construction.
            let ft = unsafe { fv.as_native_tuple_ref().unwrap() };
            let fs = ft.as_slice();
            // SAFETY: fs[0] is the field name string in the emitted layout, a live constant element; the str tag holds by construction.
            let fname = unsafe { fs[0].as_native_str_ref().unwrap().to_string() };
            let has_default = fs[1].as_bool().unwrap_or(false);
            let default = if has_default && default_idx < defaults_popped.len() {
                let d = defaults_popped[default_idx];
                default_idx += 1;
                d
            } else {
                Value::NIL
            };
            fields.push(PureTraitField {
                name: fname,
                has_default,
                default,
            });
        }

        // Methods (optional, index 4)
        let mut methods = IndexMap::new();
        let mut static_methods = IndexMap::new();
        let mut abstract_methods = std::collections::HashSet::new();
        if items.len() > 4 {
            let mut init_fn_unused = None;
            self.parse_methods_list(
                items[4],
                &mut methods,
                &mut static_methods,
                &mut init_fn_unused,
                &mut abstract_methods,
            )?;
        }

        self.trait_registry.register_trait(PureTraitDef {
            name,
            extends,
            fields,
            methods,
            static_methods,
            abstract_methods,
        });

        // Traits aren't values; push NIL so the statement has a result to pop
        frame.push(Value::NIL);
        Ok(())
    }

    /// Handle MakeEnum opcode: parse metadata, register enum type.
    fn handle_make_enum(&mut self, info: Value, frame: &mut PureFrame, host: &dyn VmHost) -> VMResult<()> {
        // SAFETY: info is a constant held alive by the CodeObject pool; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
        let tuple = unsafe {
            info.as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeEnum: expected tuple constant".into()))?
        };
        let items = tuple.as_slice();
        if items.len() < 2 {
            return Err(VMError::RuntimeError("MakeEnum: malformed metadata".into()));
        }

        // SAFETY: items[0] is a live constant; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
        let name = unsafe {
            items[0]
                .as_native_str_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeEnum: expected string name".into()))?
                .to_string()
        };

        // Extract variant names
        // SAFETY: items[1] is a live constant; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
        let variants_tuple = unsafe {
            items[1]
                .as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeEnum: expected variants tuple".into()))?
        };
        let mut variant_names = Vec::new();
        for v in variants_tuple.as_slice() {
            // SAFETY: v is a live constant element; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
            let vname = unsafe {
                v.as_native_str_ref()
                    .ok_or_else(|| VMError::RuntimeError("MakeEnum: expected string variant name".into()))?
                    .to_string()
            };
            variant_names.push(vname);
        }

        let type_id = self
            .enum_registry
            .register(&name, &variant_names, &mut self.symbol_table);
        self.enum_type_names.insert(name.clone(), type_id);

        // Store the enum type as a global so `EnumName.variant` works via GetAttr
        let marker = Value::from_enum_type(type_id);
        host.store_global(&name, marker);
        frame.push(Value::NIL);
        Ok(())
    }

    /// Handle MakeUnion opcode: register variant constructors, store the
    /// union namespace in globals.
    ///
    /// Mirrors `build_union_type` in the PyO3 VM so the two runtimes agree:
    /// payload variants become struct types named `"Union.Variant"` (matched
    /// by `Union.Variant{...}` patterns), nullary variants become interned
    /// symbols of the same qualified name (matched like enum variants).
    fn handle_make_union(&mut self, info: Value, frame: &mut PureFrame, host: &dyn VmHost) -> VMResult<()> {
        // SAFETY: info is a constant held alive by the CodeObject pool; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
        let tuple = unsafe {
            info.as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeUnion: expected tuple constant".into()))?
        };
        let items = tuple.as_slice();
        if items.len() < 3 {
            return Err(VMError::RuntimeError("MakeUnion: malformed metadata".into()));
        }

        // SAFETY: items[0] is a live constant; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
        let name = unsafe {
            items[0]
                .as_native_str_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeUnion: name must be string".into()))?
                .to_string()
        };

        // SAFETY: items[1] is a live constant; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
        let type_params_tuple = unsafe {
            items[1]
                .as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeUnion: expected type params tuple".into()))?
        };
        let mut type_params = Vec::with_capacity(type_params_tuple.as_slice().len());
        for tp in type_params_tuple.as_slice() {
            // SAFETY: tp is a live constant element; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
            let s = unsafe {
                tp.as_native_str_ref()
                    .ok_or_else(|| VMError::RuntimeError("MakeUnion: type param must be string".into()))?
            };
            type_params.push(s.to_string());
        }

        // Optional 4th element: methods list of (name, vmfunc_idx) tuples.
        // Shared by every variant -- `self` receives whichever variant the
        // method is called on, the body discriminates with `match`.
        let mut methods_map: IndexMap<String, u32> = IndexMap::new();
        if let Some(methods_val) = items.get(3) {
            // SAFETY: methods_val is a live constant; the accessor checks the list tag (errors otherwise), so the borrow points to a live Arc<NativeList>.
            let methods_list = unsafe {
                methods_val
                    .as_native_list_ref()
                    .ok_or_else(|| VMError::RuntimeError("MakeUnion: bad methods list".into()))?
            };
            for m in methods_list.as_slice_cloned() {
                // SAFETY: m is a live element of the cloned methods list; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
                let pair = unsafe {
                    m.as_native_tuple_ref()
                        .ok_or_else(|| VMError::RuntimeError("MakeUnion: bad method entry".into()))?
                };
                let mp = pair.as_slice();
                if mp.len() >= 2 {
                    // SAFETY: mp[0] is a live constant element; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
                    let mname = unsafe {
                        mp[0]
                            .as_native_str_ref()
                            .ok_or_else(|| VMError::RuntimeError("MakeUnion: method name must be string".into()))?
                            .to_string()
                    };
                    if mp[1].is_vmfunc() && !mp[1].is_invalid() {
                        methods_map.insert(mname, mp[1].as_vmfunc_idx());
                    }
                }
                m.decref();
            }
        }
        let nullary_methods = if methods_map.is_empty() {
            None
        } else {
            Some(std::rc::Rc::new(methods_map.clone()))
        };

        // SAFETY: items[2] is a live constant; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
        let variants_tuple = unsafe {
            items[2]
                .as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeUnion: expected variants tuple".into()))?
        };
        let mut variants: Vec<(String, Value)> = Vec::with_capacity(variants_tuple.as_slice().len());
        for variant in variants_tuple.as_slice() {
            // SAFETY: variant is a live constant element; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
            let pair = unsafe {
                variant
                    .as_native_tuple_ref()
                    .ok_or_else(|| VMError::RuntimeError("MakeUnion: bad variant".into()))?
            };
            let parts = pair.as_slice();
            if parts.len() < 2 {
                return Err(VMError::RuntimeError("MakeUnion: variant tuple too small".into()));
            }
            // SAFETY: parts[0] is a live constant element; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
            let variant_name = unsafe {
                parts[0]
                    .as_native_str_ref()
                    .ok_or_else(|| VMError::RuntimeError("MakeUnion: variant name must be string".into()))?
                    .to_string()
            };
            if variants.iter().any(|(n, _)| *n == variant_name) {
                return Err(VMError::RuntimeError(format!(
                    "union '{}': duplicate variant '{}'",
                    name, variant_name
                )));
            }
            // SAFETY: parts[1] is a live constant element; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
            let fields_tuple = unsafe {
                parts[1]
                    .as_native_tuple_ref()
                    .ok_or_else(|| VMError::RuntimeError("MakeUnion: bad fields".into()))?
            };
            let qualified = qualified_name(&name, &variant_name);
            let binding = if fields_tuple.as_slice().is_empty() {
                let sym_id = self.symbol_table.intern(&qualified);
                if let Some(ref nm) = nullary_methods {
                    self.union_nullary_methods.insert(sym_id, std::rc::Rc::clone(nm));
                }
                Value::from_symbol(sym_id)
            } else {
                // Field type texts (3rd variant element, parallel to field names;
                // empty string = unannotated). Drive the generic-nominal boundary:
                // each is classified into a `FieldTemplate` against `type_params`.
                let field_types: Vec<String> = match parts.get(2) {
                    Some(v) => {
                        // SAFETY: parts[2] is a live constant element; the accessor checks the tuple tag (errors otherwise), so the borrow points to a live Arc<NativeTuple>.
                        let types_tuple = unsafe {
                            v.as_native_tuple_ref()
                                .ok_or_else(|| VMError::RuntimeError("MakeUnion: bad field types".into()))?
                        };
                        let mut out = Vec::with_capacity(types_tuple.as_slice().len());
                        for t in types_tuple.as_slice() {
                            // SAFETY: t is a live constant element; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
                            let s = unsafe {
                                t.as_native_str_ref().ok_or_else(|| {
                                    VMError::RuntimeError("MakeUnion: field type must be string".into())
                                })?
                            };
                            out.push(s.to_string());
                        }
                        out
                    }
                    None => Vec::new(),
                };
                let mut fields = Vec::with_capacity(fields_tuple.as_slice().len());
                let mut templates = Vec::with_capacity(fields_tuple.as_slice().len());
                for (fi, f) in fields_tuple.as_slice().iter().enumerate() {
                    // SAFETY: f is a live constant element; the accessor checks the str tag (errors otherwise), so the borrow points to a live Arc<NativeString>.
                    let fname = unsafe {
                        f.as_native_str_ref()
                            .ok_or_else(|| VMError::RuntimeError("MakeUnion: field name must be string".into()))?
                    };
                    let ftext = field_types.get(fi).map(String::as_str).filter(|s| !s.is_empty());
                    let template = catnip_core::vm::opcode::compute_field_template(&type_params, ftext);
                    // A concrete field (`A(x: int)`) is enforced at construction,
                    // mirroring struct fields; a type-parameter field (`Some(value: T)`)
                    // is inert here (`T` binds at the use-site generic boundary).
                    fields.push(PureStructField {
                        name: fname.to_string(),
                        has_default: false,
                        default_slot: None,
                        check: template.construction_check(),
                    });
                    templates.push(template);
                }
                let type_id = self.struct_registry.register_type(PureStructType {
                    id: 0,
                    name: qualified.clone(),
                    fields,
                    defaults: Vec::new(),
                    methods: methods_map.clone(),
                    static_methods: IndexMap::new(),
                    init_fn: None,
                    implements: Vec::new(),
                    mro: vec![qualified],
                    mro_ids: Vec::new(),
                    parent_names: Vec::new(),
                    abstract_methods: std::collections::HashSet::new(),
                });
                self.struct_registry.set_variant_templates(type_id, templates);
                Value::from_struct_type(type_id)
            };
            variants.push((variant_name, binding));
        }

        let ns = crate::value::UnionNamespace {
            name: name.clone(),
            type_params,
            variants,
        };
        host.store_global(&name, Value::from_union_type(ns));
        frame.push(Value::NIL);
        Ok(())
    }

    /// Construct a struct instance from args and defaults.
    fn construct_struct(
        &mut self,
        type_id: u32,
        args: &[Value],
        kwargs: &IndexMap<String, Value>,
        host: &dyn VmHost,
    ) -> VMResult<Value> {
        let ty = self
            .struct_registry
            .get_type(type_id)
            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;

        // Check abstract methods
        if !ty.abstract_methods.is_empty() {
            let abs: Vec<&str> = ty.abstract_methods.iter().map(|s| s.as_str()).collect();
            return Err(VMError::RuntimeError(format!(
                "cannot instantiate '{}': abstract methods not implemented: {}",
                ty.name,
                abs.join(", ")
            )));
        }

        let num_fields = ty.fields.len();
        let required = ty.required_field_count();
        let ty_name = ty.name.clone();

        if kwargs.is_empty() && args.len() < required {
            return Err(VMError::RuntimeError(format!(
                "{}() takes {} argument(s) ({} required), got {}",
                ty_name,
                num_fields,
                required,
                args.len()
            )));
        }
        if args.len() > num_fields {
            return Err(VMError::RuntimeError(format!(
                "{}() takes {} argument(s), got {}",
                ty_name,
                num_fields,
                args.len()
            )));
        }

        // Map every field to its source (positional, then keyword by name) --
        // bit-copies only, so any error below leaves args/kwargs untouched for
        // the caller to release (construct_struct consumes them on success only).
        // Mirrors the PyO3 runtime's kwargs semantics: unknown names and
        // positional/keyword double-fills are errors.
        let mut sources: Vec<Option<Value>> = vec![None; num_fields];
        for (i, &a) in args.iter().enumerate() {
            sources[i] = Some(a);
        }
        for (name, &val) in kwargs {
            let Some(idx) = ty.fields.iter().position(|f| &f.name == name) else {
                return Err(VMError::TypeError(format!(
                    "{}() got an unexpected keyword argument '{}'",
                    ty_name, name
                )));
            };
            if sources[idx].is_some() {
                return Err(VMError::TypeError(format!(
                    "{}() got multiple values for argument '{}'",
                    ty_name, name
                )));
            }
            sources[idx] = Some(val);
        }

        // Pass 1 -- validate each field's source value against its annotation,
        // read-only (no refcount mutation). `field_value_ok` also rejects up
        // front the single case pass 2 could fail on (bigint -> float overflow),
        // so coercion below never errors after mutating a refcount.
        let ty = self.struct_registry.get_type(type_id).unwrap();
        for (i, f) in ty.fields.iter().enumerate() {
            let src = if let Some(v) = sources[i] {
                v
            } else if let Some(slot) = f.default_slot {
                ty.defaults[slot]
            } else {
                return Err(VMError::RuntimeError(format!(
                    "{}(): missing argument for field '{}'",
                    ty_name, f.name
                )));
            };
            if !self.field_value_ok(src, &f.check, host) {
                return Err(VMError::TypeError(format!(
                    "field '{}' of '{}' expects '{}' but got '{}'",
                    f.name,
                    ty_name,
                    catnip_core::vm::opcode::format_param_check(&f.check),
                    self.nominal_value_type_name(src)
                )));
            }
        }

        // Pass 2 -- materialize values: sources are moved in, defaults fill the
        // rest (cloned), primitives are coerced to the declared type (numeric
        // tower). Validated above, so coercion is infallible here.
        let ty = self.struct_registry.get_type(type_id).unwrap();
        let mut field_values = Vec::with_capacity(num_fields);
        for (i, f) in ty.fields.iter().enumerate() {
            let raw = if let Some(v) = sources[i] {
                v
            } else {
                let default = ty.defaults[f.default_slot.unwrap()];
                default.clone_refcount();
                default
            };
            let val = match &f.check {
                catnip_core::vm::opcode::ParamCheck::Primitive(code) => boundary_coerce(raw, *code).unwrap_or(raw),
                _ => raw,
            };
            field_values.push(val);
        }

        Ok(Value::from_struct_instance(StructCell::new(type_id, field_values)))
    }

    /// Get attribute from a struct instance (field or method).
    fn struct_getattr(&mut self, obj: Value, name: &str) -> VMResult<Value> {
        // SAFETY: `obj` is an owned Value held by the caller across this call; the
        // borrow is transient and obj is not decref'd while `cell` is used.
        let cell =
            unsafe { obj.as_struct_ref() }.ok_or_else(|| VMError::RuntimeError("freed struct instance".into()))?;
        let type_id = cell.type_id;
        let ty = self
            .struct_registry
            .get_type(type_id)
            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;

        // Field access
        if let Some(field_idx) = ty.field_index(name) {
            let val = cell.field(field_idx);
            val.clone_refcount(); // return a fresh owning ref to the field
            return Ok(val);
        }

        // Method access: bind the receiver as a curried first argument. `self`
        // is the method's first parameter (the direct `p.get()` path prepends
        // the receiver to the args), so it must reach the callee's param slot at
        // call time -- not a closure capture, which the body never reads. The
        // slot keeps the method's own closure (globals/sibling methods) as-is.
        if let Some(&func_idx) = ty.methods.get(name) {
            let slot = self
                .func_table
                .get(func_idx)
                .ok_or_else(|| VMError::RuntimeError("invalid method function".into()))?;
            let method_code = Arc::clone(&slot.code);
            let method_closure = slot.closure.clone();

            obj.clone_refcount(); // the bound slot owns a ref to the curried self
            // Runtime bound-method closure: Arc-backed, reclaimed when its last
            // Value dies (frees the curried receiver via PureFuncSlot::Drop). The
            // method_closure Rc is shared with the template, so dropping this slot
            // only decrements it -- the template keeps the captures alive.
            let arc = PureFuncSlot::new_runtime(method_code, method_closure, Some(obj)).into_arc();
            return Ok(Value::from_arc_closure(arc));
        }

        // Static method access
        if let Some(&func_idx) = ty.static_methods.get(name) {
            return Ok(Value::from_vmfunc(func_idx));
        }

        let ty_name = ty.name.clone();
        Err(VMError::RuntimeError(format!(
            "'{}' has no attribute '{}'",
            ty_name, name
        )))
    }

    /// Build a hashable key for `value`, honoring a struct's custom `op_hash`
    /// (and freezing it). Single entry point so every dict/set/`hash()` site
    /// shares the same registry- and op_hash-aware path.
    fn value_to_key(&mut self, value: Value, host: &dyn VmHost) -> VMResult<ValueKey> {
        let mut builder = KeyBuilder { vm: self, host };
        value.to_key_ctx(&mut builder)
    }

    /// `in` with registry-aware equality for native lists and tuples (deep
    /// structural comparison, struct instances included); other containers
    /// (str, dict, set) keep the host's semantics.
    fn contains_value(&mut self, item: Value, container: Value, host: &dyn VmHost) -> VMResult<bool> {
        if container.is_native_list() {
            // SAFETY: the is_native_list() guard above proves container holds a live Arc<NativeList> owned on the stack; the borrow does not outlive it.
            let list = unsafe { container.as_native_list_ref().unwrap() };
            let items = list.as_slice_cloned();
            let found = items.iter().any(|v| deep_eq(item, *v));
            for v in &items {
                v.decref();
            }
            Ok(found)
        } else if container.is_native_tuple() {
            // SAFETY: the is_native_tuple() guard above proves container holds a live Arc<NativeTuple> owned on the stack; the borrow does not outlive it.
            let t = unsafe { container.as_native_tuple_ref().unwrap() };
            Ok(t.as_slice().iter().any(|v| deep_eq(item, *v)))
        } else if container.is_native_dict() {
            // Registry-aware key so struct keys match by value; the host has no
            // registry.
            let key = self.value_to_key(item, host)?;
            // SAFETY: the is_native_dict() guard above proves container holds a live Arc<NativeDict> owned on the stack; the borrow does not outlive it.
            let dict = unsafe { container.as_native_dict_ref().unwrap() };
            Ok(dict.contains_key(&key))
        } else if container.is_native_set() {
            let key = self.value_to_key(item, host)?;
            // SAFETY: the is_native_set() guard above proves container holds a live Arc<NativeSet> owned on the stack; the borrow does not outlive it.
            let set = unsafe { container.as_native_set_ref().unwrap() };
            Ok(set.contains(&key))
        } else {
            host.contains_op(item, container)
        }
    }

    /// Registry-aware `index`/`count` (list, tuple) and `remove` (list). The
    /// default `NativeList/Tuple` methods compare with `Value::eq`, which has no
    /// struct registry and misses struct payloads compared by value; this routes
    /// the search through `deep_eq` instead -- mirror of `contains_value`.
    fn collection_eq_method(&mut self, obj: Value, method: &str, args: &[Value]) -> VMResult<Value> {
        let needle = *args
            .first()
            .ok_or_else(|| VMError::TypeError(format!("{}() takes 1 argument", method)))?;

        if obj.is_native_list() {
            // SAFETY: the is_native_list() guard above proves obj holds a live Arc<NativeList> owned on the stack; the borrow does not outlive it.
            let list = unsafe { obj.as_native_list_ref().unwrap() };
            let items = list.as_slice_cloned();
            let result = match method {
                "count" => Ok(Value::from_int(
                    items.iter().filter(|v| deep_eq(needle, **v)).count() as i64
                )),
                "index" => items
                    .iter()
                    .position(|v| deep_eq(needle, *v))
                    .map(|i| Value::from_int(i as i64))
                    .ok_or_else(|| VMError::ValueError("value not in list".into())),
                "remove" => match items.iter().position(|v| deep_eq(needle, *v)) {
                    Some(i) => {
                        let removed = list.remove_at(i as i64)?;
                        removed.decref();
                        Ok(Value::NIL)
                    }
                    None => Err(VMError::ValueError("list.remove(x): x not in list".into())),
                },
                _ => unreachable!(),
            };
            for v in &items {
                v.decref();
            }
            result
        } else {
            // SAFETY: callers restrict obj to a native list or tuple; the list branch is taken otherwise, so here obj holds a live Arc<NativeTuple> owned on the stack; the borrow does not outlive it.
            let tuple = unsafe { obj.as_native_tuple_ref().unwrap() };
            let items = tuple.as_slice();
            match method {
                "count" => Ok(Value::from_int(
                    items.iter().filter(|v| deep_eq(needle, **v)).count() as i64
                )),
                "index" => items
                    .iter()
                    .position(|v| deep_eq(needle, *v))
                    .map(|i| Value::from_int(i as i64))
                    .ok_or_else(|| VMError::ValueError("value not in tuple".into())),
                _ => unreachable!(),
            }
        }
    }

    /// Registry-aware dict/set methods whose keys may be struct snapshots: the
    /// default `collection::call_method` builds keys via `Value::to_key` (no
    /// registry, rejects structs) and materializes them via `ValueKey::to_value`
    /// (cannot rebuild structs). Returns `None` for methods/types it does not
    /// handle, so the caller falls back to the host. Mirrors the dispatch in
    /// `ops::collection`.
    fn collection_method_reg(
        &mut self,
        obj: Value,
        method: &str,
        args: &[Value],
        host: &dyn VmHost,
    ) -> Option<VMResult<Value>> {
        if obj.is_native_dict() {
            // SAFETY: the is_native_dict() guard above proves obj holds a live Arc<NativeDict> owned on the stack; the borrow does not outlive it.
            let dict = unsafe { obj.as_native_dict_ref().unwrap() };
            match method {
                "get" => {
                    let Some(k0) = args.first() else {
                        return Some(Err(VMError::TypeError("get() takes at least 1 argument".into())));
                    };
                    let key = match self.value_to_key(*k0, host) {
                        Ok(k) => k,
                        Err(e) => return Some(Err(e)),
                    };
                    let default = if args.len() > 1 { args[1] } else { Value::NIL };
                    Some(Ok(dict.get_default(&key, default)))
                }
                "pop" => {
                    let Some(k0) = args.first() else {
                        return Some(Err(VMError::TypeError("pop() takes at least 1 argument".into())));
                    };
                    let key = match self.value_to_key(*k0, host) {
                        Ok(k) => k,
                        Err(e) => return Some(Err(e)),
                    };
                    Some(match dict.pop(&key) {
                        Ok(v) => Ok(v),
                        Err(_) if args.len() > 1 => {
                            args[1].clone_refcount();
                            Ok(args[1])
                        }
                        Err(e) => Err(e),
                    })
                }
                "keys" => {
                    let keys = dict.keys_cloned();
                    let mut vals = Vec::with_capacity(keys.len());
                    for k in &keys {
                        vals.push(self.key_to_value(k));
                    }
                    Some(Ok(Value::from_list(vals)))
                }
                "items" => {
                    let keys = dict.keys_cloned();
                    let mut items = Vec::with_capacity(keys.len());
                    for k in &keys {
                        let kv = self.key_to_value(k);
                        let vv = dict.get_item(k).unwrap_or(Value::NIL);
                        items.push(Value::from_tuple(vec![kv, vv]));
                    }
                    Some(Ok(Value::from_list(items)))
                }
                _ => None,
            }
        } else if obj.is_native_set() {
            // SAFETY: the is_native_set() guard above proves obj holds a live Arc<NativeSet> owned on the stack; the borrow does not outlive it.
            let set = unsafe { obj.as_native_set_ref().unwrap() };
            match method {
                "add" | "remove" | "discard" => {
                    let Some(v) = args.first() else {
                        return Some(Err(VMError::TypeError(format!("{}() takes 1 argument", method))));
                    };
                    let key = match self.value_to_key(*v, host) {
                        Ok(k) => k,
                        Err(e) => return Some(Err(e)),
                    };
                    Some(match method {
                        "add" => {
                            set.add(key);
                            Ok(Value::NIL)
                        }
                        "remove" => set.remove(&key).map(|_| Value::NIL),
                        _ => {
                            set.discard(&key);
                            Ok(Value::NIL)
                        }
                    })
                }
                "pop" => Some(match set.pop() {
                    Ok(k) => Ok(self.key_to_value(&k)),
                    Err(e) => Err(e),
                }),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Display a value, using the struct registry for struct instances
    /// and the symbol table for enum/union variants.
    pub(crate) fn display_value(&self, val: &Value) -> String {
        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
        if let Some(cell) = unsafe { val.as_struct_ref() } {
            self.struct_registry
                .display_instance(cell, |v| self.display_value_repr(v))
        } else if let Some(qname) = val.as_symbol().and_then(|sym_id| self.symbol_table.resolve(sym_id)) {
            qname.to_string()
        } else if let Some(s) = self.display_collection_repr(val) {
            s
        } else {
            val.display_string()
        }
    }

    /// Repr a value, using the struct registry for struct instances
    /// and the symbol table for enum/union variants.
    pub(crate) fn display_value_repr(&self, val: &Value) -> String {
        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
        if let Some(cell) = unsafe { val.as_struct_ref() } {
            self.struct_registry
                .display_instance(cell, |v| self.display_value_repr(v))
        } else if let Some(qname) = val.as_symbol().and_then(|sym_id| self.symbol_table.resolve(sym_id)) {
            qname.to_string()
        } else if let Some(s) = self.display_collection_repr(val) {
            s
        } else {
            val.repr_string()
        }
    }

    /// Materialize a dict/set key back to a Value. A struct key (snapshot) holds
    /// no live registry handle, so it is rebuilt as a fresh instance, re-frozen
    /// to match the source (a hashed struct stays locked). Other keys delegate
    /// to `ValueKey::to_value`. Tuples are rebuilt element-wise so nested struct
    /// keys are handled.
    fn key_to_value(&mut self, k: &ValueKey) -> Value {
        match k {
            ValueKey::Struct { type_id, fields, .. } => {
                let type_id = *type_id;
                let mut field_vals = Vec::with_capacity(fields.len());
                for f in fields.iter() {
                    field_vals.push(self.key_to_value(f));
                }
                // Rebuild a fresh instance, re-frozen to match the source (a
                // hashed struct key stays locked against mutation).
                let cell = StructCell::new(type_id, field_vals);
                cell.frozen.set(true);
                Value::from_struct_instance(cell)
            }
            ValueKey::Tuple(elems) if elems.iter().any(Self::key_has_struct) => {
                let mut vals = Vec::with_capacity(elems.len());
                for e in elems.iter() {
                    vals.push(self.key_to_value(e));
                }
                Value::from_tuple(vals)
            }
            other => other.to_value(),
        }
    }

    /// True if a key is (or transitively contains) a struct snapshot, which
    /// `ValueKey::to_value` cannot rebuild without the registry.
    fn key_has_struct(k: &ValueKey) -> bool {
        match k {
            ValueKey::Struct { .. } => true,
            ValueKey::Tuple(elems) => elems.iter().any(Self::key_has_struct),
            _ => false,
        }
    }

    /// Format a dict/set key for repr, resolving struct keys through the registry
    /// without materializing a fresh instance.
    fn display_key(&self, k: &ValueKey) -> String {
        match k {
            ValueKey::Struct { type_id, fields, .. } => match self.struct_registry.get_type(*type_id) {
                Some(ty) => {
                    let parts: Vec<String> = ty
                        .fields
                        .iter()
                        .zip(fields.iter())
                        .map(|(f, fk)| format!("{}={}", f.name, self.display_key(fk)))
                        .collect();
                    format!("{}({})", ty.name, parts.join(", "))
                }
                None => "<unknown struct>".to_string(),
            },
            ValueKey::Tuple(elems) => {
                let parts: Vec<String> = elems.iter().map(|e| self.display_key(e)).collect();
                if parts.len() == 1 {
                    format!("({},)", parts[0])
                } else {
                    format!("({})", parts.join(", "))
                }
            }
            other => {
                let v = other.to_value();
                let s = self.display_value_repr(&v);
                v.decref();
                s
            }
        }
    }

    /// Format a native collection (list, tuple, dict, set) with registry-aware
    /// element repr; `None` for non-collection values. Mirrors the layout of
    /// `Value::display_string`, but resolves nested struct instances and symbols
    /// through the registry/symbol table recursively -- `Value`'s own formatting
    /// has no registry access and would render them as `<struct #N>` / `???`.
    fn display_collection_repr(&self, val: &Value) -> Option<String> {
        if val.is_native_list() {
            // SAFETY: the is_native_list() guard above proves val holds a live Arc<NativeList> owned on the stack; the borrow does not outlive it.
            let list = unsafe { val.as_native_list_ref().unwrap() };
            let parts: Vec<String> = list
                .as_slice_cloned()
                .iter()
                .map(|v| {
                    let s = self.display_value_repr(v);
                    v.decref();
                    s
                })
                .collect();
            Some(format!("[{}]", parts.join(", ")))
        } else if val.is_native_tuple() {
            // SAFETY: the is_native_tuple() guard above proves val holds a live Arc<NativeTuple> owned on the stack; the borrow does not outlive it.
            let tuple = unsafe { val.as_native_tuple_ref().unwrap() };
            let items = tuple.as_slice();
            let parts: Vec<String> = items.iter().map(|v| self.display_value_repr(v)).collect();
            if items.len() == 1 {
                Some(format!("({},)", parts[0]))
            } else {
                Some(format!("({})", parts.join(", ")))
            }
        } else if val.is_native_dict() {
            // SAFETY: the is_native_dict() guard above proves val holds a live Arc<NativeDict> owned on the stack; the borrow does not outlive it.
            let dict = unsafe { val.as_native_dict_ref().unwrap() };
            let parts: Vec<String> = dict
                .keys_cloned()
                .iter()
                .map(|k| {
                    let vv = dict.get_item(k).unwrap_or(Value::NIL);
                    let s = format!("{}: {}", self.display_key(k), self.display_value_repr(&vv));
                    vv.decref();
                    s
                })
                .collect();
            Some(format!("{{{}}}", parts.join(", ")))
        } else if val.is_native_set() {
            // SAFETY: the is_native_set() guard above proves val holds a live Arc<NativeSet> owned on the stack; the borrow does not outlive it.
            let set = unsafe { val.as_native_set_ref().unwrap() };
            let keys = set.keys_cloned();
            if keys.is_empty() {
                Some("set()".to_string())
            } else {
                let parts: Vec<String> = keys.iter().map(|k| self.display_key(k)).collect();
                Some(format!("{{{}}}", parts.join(", ")))
            }
        } else {
            None
        }
    }

    /// Try to dispatch a binary op on struct instances via operator methods.
    /// Checks left operand first, then right (reverse dispatch).
    fn struct_binary_op(&self, op_name: &str, a: Value, b: Value) -> Option<u32> {
        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
        if let Some(cell) = unsafe { a.as_struct_ref() } {
            let ty = self.struct_registry.get_type(cell.type_id)?;
            if let Some(&func_idx) = ty.methods.get(op_name) {
                return Some(func_idx);
            }
        }
        // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
        if let Some(cell) = unsafe { b.as_struct_ref() } {
            let ty = self.struct_registry.get_type(cell.type_id)?;
            return ty.methods.get(op_name).copied();
        }
        None
    }

    /// Call a struct operator method: pushes a new frame with self=a and other=b.
    /// Execute a call whose target is not a VM function: struct type
    /// (instantiation + optional init frame), ND recursion sentinel, ND
    /// declaration/lift wrapper, builtin name, or host object. Shared by
    /// Call and TailCall -- in both cases the result (or the init frame)
    /// goes through the current frame.
    fn call_non_vmfunc(
        &mut self,
        func: Value,
        args: Vec<Value>,
        frame: &mut PureFrame,
        host: &dyn VmHost,
    ) -> VMResult<()> {
        if func.is_struct_type() {
            let type_id = func.as_struct_type_id().unwrap();
            // construct_struct consumes args on success and nothing on its
            // reachable errors (arg-count / abstract checks run before any arg is
            // moved into a field), so release them only on failure.
            let result = match self.construct_struct(type_id, &args, &IndexMap::new(), host) {
                Ok(r) => r,
                Err(e) => {
                    Self::release_operands(&args);
                    return Err(e);
                }
            };
            // If init_fn, call it
            let init_fn = self.struct_registry.get_type(type_id).and_then(|ty| ty.init_fn);
            if let Some(init_idx) = init_fn {
                let slot = self
                    .func_table
                    .get(init_idx)
                    .ok_or_else(|| VMError::RuntimeError("invalid init function".into()))?;
                let callee_code = Arc::clone(&slot.code);
                let closure = slot.closure.clone();
                if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                    result.decref(); // the freshly-constructed instance would otherwise leak
                    return Err(VMError::FrameOverflow);
                }
                // Push instance onto caller stack first -- init's return will be discarded.
                // The instance now has two owners: the caller stack (bound to the
                // result) and init's `self` local (released when the init frame tears
                // down). `bind_args` moves the ref into the frame, so clone once to
                // cover the second owner.
                frame.push(result);
                result.clone_refcount();
                let init_args = vec![result];
                let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                new_frame.bind_args(&init_args);
                new_frame.closure_scope = closure;
                new_frame.discard_return = true;
                let old_frame = std::mem::replace(frame, new_frame);
                self.frame_stack.push(old_frame);
            } else {
                frame.push(result);
            }
        } else if Self::is_nd_recur_sentinel(func) {
            // ND recursion callback: recur(value)
            let result = self.handle_nd_recur_call(&args, host);
            func.decref();
            for a in &args {
                a.decref();
            }
            frame.push(result?);
        } else if let Some((tag, inner)) = Self::check_nd_wrapper(func) {
            // ND declaration/lift wrapper call
            let result = if tag == super::broadcast::ND_DECL_TAG {
                self.handle_nd_decl_call(inner, &args, host)
            } else {
                self.handle_nd_lift_call(inner, &args, host)
            };
            func.decref();
            for a in &args {
                a.decref();
            }
            frame.push(result?);
        } else if func.is_native_str() {
            // SAFETY: the is_native_str() guard above proves func holds a live Arc<NativeString> owned on the stack; the borrow does not outlive it.
            let name = unsafe { func.as_native_str_ref().unwrap() };
            if name == "isinstance" && args.len() == 2 {
                let result = self.check_isinstance(args[0], args[1]);
                func.decref();
                for a in &args {
                    a.decref();
                }
                frame.push(result?);
            } else if name == "import" && self.import_loader.is_some() {
                let result = self.handle_import(&args, &[], host);
                func.decref();
                for a in &args {
                    a.decref();
                }
                frame.push(result?);
            } else if let Some(hof) = Self::try_build_hof(name, &args) {
                // HofCall owns func, iterable, and init (fold).
                // Only the builtin name string is untracked.
                func.decref();
                return Err(VMError::HofBuiltin(hof));
            } else if name == "print" {
                // Format with the registry (struct fields, symbol names);
                // the host's display_string can resolve neither. I/O stays
                // in the host.
                let mut out = String::new();
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    out.push_str(&self.display_value(a));
                }
                for a in &args {
                    a.decref();
                }
                let line = Value::from_string(out);
                let result = host.call_function(func, &[line])?;
                line.decref();
                func.decref();
                frame.push(result);
            } else if (name == "str" || name == "repr") && args.len() == 1 {
                // Registry-aware conversion; other arities keep the host's
                // error messages.
                let s = if name == "str" {
                    self.display_value(&args[0])
                } else {
                    self.display_value_repr(&args[0])
                };
                args[0].decref();
                func.decref();
                frame.push(Value::from_string(s));
            } else if name == "hash" && args.len() == 1 {
                // Registry-aware key so structs (and union payload variants)
                // hash structurally (the host's `to_key` has no registry) or via
                // a custom op_hash. A top-level op_hash result is returned as-is,
                // matching the Python CLI's `hash(obj) == obj.op_hash()`.
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let key = self.value_to_key(args[0], host)?;
                let h = match &key {
                    ValueKey::Struct {
                        hash_override: Some(h), ..
                    } => *h,
                    _ => {
                        let mut hasher = DefaultHasher::new();
                        key.hash(&mut hasher);
                        hasher.finish() as i64
                    }
                };
                let result =
                    Value::try_from_int(h).unwrap_or_else(|| Value::from_bigint_or_demote(rug::Integer::from(h)));
                args[0].decref();
                func.decref();
                frame.push(result);
            } else {
                let result = host.call_function(func, &args);
                func.decref();
                for a in &args {
                    a.decref();
                }
                frame.push(result?);
            }
        } else {
            // Delegate to host
            let result = host.call_function(func, &args);
            func.decref();
            for a in &args {
                a.decref();
            }
            frame.push(result?);
        }
        Ok(())
    }

    fn call_struct_op(&mut self, func_idx: u32, a: Value, b: Value, frame: &mut PureFrame) -> VMResult<()> {
        let slot = self
            .func_table
            .get(func_idx)
            .ok_or_else(|| VMError::RuntimeError("invalid operator method".into()))?;
        let callee_code = Arc::clone(&slot.code);
        let closure = slot.closure.clone();
        if self.frame_stack.len() >= MAX_FRAME_DEPTH {
            a.decref();
            b.decref();
            return Err(VMError::FrameOverflow);
        }
        let args = vec![a, b];
        let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
        new_frame.bind_args(&args);
        new_frame.closure_scope = closure;
        let old_frame = std::mem::replace(frame, new_frame);
        self.frame_stack.push(old_frame);
        Ok(())
    }

    /// Set attribute on a struct instance.
    fn struct_setattr(&mut self, obj: Value, name: &str, val: Value) -> VMResult<()> {
        // SAFETY: `obj` is an owned Value held by the caller across this call; the
        // borrow is transient and obj is not decref'd while `cell` is used.
        let cell =
            unsafe { obj.as_struct_ref() }.ok_or_else(|| VMError::RuntimeError("freed struct instance".into()))?;
        let ty = self
            .struct_registry
            .get_type(cell.type_id)
            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;

        // Refuse mutation once the instance has been hashed (used as a dict/set
        // key), so its hash stays stable. Mirrors the PyO3 runtime.
        if cell.frozen.get() {
            return Err(VMError::TypeError(format!(
                "cannot mutate '{}' after it has been hashed (used as dict/set key)",
                ty.name
            )));
        }

        if let Some(field_idx) = ty.field_index(name) {
            // The Cell allows in-place mutation through the shared Arc; no
            // `&mut self.struct_registry` needed.
            let old = cell.fields[field_idx].replace(val);
            old.decref(); // release the previous field value
            Ok(())
        } else {
            let ty_name = ty.name.clone();
            Err(VMError::RuntimeError(format!(
                "cannot set attribute '{}' on '{}'",
                name, ty_name
            )))
        }
    }
}

impl Default for PureVM {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PureFrame extension: closure_scope field
// ---------------------------------------------------------------------------

// We need closure_scope on PureFrame but it wasn't in the original struct.
// We'll use a wrapper approach -- but actually, let's check if we should
// add closure_scope to PureFrame instead.

// For now, we'll store it as part of the frame. We need to add it to PureFrame.

// ---------------------------------------------------------------------------
// Pattern matching (pure Rust, no Python)
// ---------------------------------------------------------------------------

/// Match a value against a VMPattern, returning bindings on success.
/// Deep structural equality, with struct instances resolved through the
/// registry.
///
/// Mirrors the PyO3 fallback chain: scalars via `eq_native`, struct
/// instances by type + fields (the `structural_eq` proxy fallback), native
/// collections element-wise. A custom `op_eq` on a *nested* instance is not
/// dispatched here (that would require pushing a frame mid-comparison); the
/// Eq/Ne handlers check it for the top-level operands only.
fn deep_eq(a: Value, b: Value) -> bool {
    if let Some(eq) = arith::eq_native(a, b) {
        return eq;
    }
    // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
    if let (Some(ca), Some(cb)) = (unsafe { a.as_struct_ref() }, unsafe { b.as_struct_ref() }) {
        // Identity: the same Arc (same NaN-box pointer).
        if a.to_raw() == b.to_raw() {
            return true;
        }
        return ca.type_id == cb.type_id
            && ca.fields.len() == cb.fields.len()
            && ca
                .fields
                .iter()
                .zip(cb.fields.iter())
                .all(|(x, y)| deep_eq(x.get(), y.get()));
    }
    if a.is_native_list() && b.is_native_list() {
        // SAFETY: the is_native_list() guards above prove a and b hold live Arc<NativeList> payloads owned by the caller; the borrows do not outlive them.
        let (la, lb) = unsafe { (a.as_native_list_ref().unwrap(), b.as_native_list_ref().unwrap()) };
        let (va, vb) = (la.as_slice_cloned(), lb.as_slice_cloned());
        let eq = va.len() == vb.len() && va.iter().zip(vb.iter()).all(|(x, y)| deep_eq(*x, *y));
        for v in &va {
            v.decref();
        }
        for v in &vb {
            v.decref();
        }
        return eq;
    }
    if a.is_native_tuple() && b.is_native_tuple() {
        // SAFETY: the is_native_tuple() guards above prove a and b hold live Arc<NativeTuple> payloads owned by the caller; the borrows do not outlive them.
        let (ta, tb) = unsafe { (a.as_native_tuple_ref().unwrap(), b.as_native_tuple_ref().unwrap()) };
        let (sa, sb) = (ta.as_slice(), tb.as_slice());
        return sa.len() == sb.len() && sa.iter().zip(sb.iter()).all(|(x, y)| deep_eq(*x, *y));
    }
    if a.is_native_dict() && b.is_native_dict() {
        // SAFETY: the is_native_dict() guards above prove a and b hold live Arc<NativeDict> payloads owned by the caller; the borrows do not outlive them.
        let (da, db) = unsafe { (a.as_native_dict_ref().unwrap(), b.as_native_dict_ref().unwrap()) };
        if da.len() != db.len() {
            return false;
        }
        for k in da.keys_cloned() {
            if !db.contains_key(&k) {
                return false;
            }
            let va = da.get_item(&k).unwrap_or(Value::NIL);
            let vb = db.get_item(&k).unwrap_or(Value::NIL);
            let eq = deep_eq(va, vb);
            va.decref();
            vb.decref();
            if !eq {
                return false;
            }
        }
        return true;
    }
    if a.is_native_set() && b.is_native_set() {
        // SAFETY: the is_native_set() guards above prove a and b hold live Arc<NativeSet> payloads owned by the caller; the borrows do not outlive them.
        let (sa, sb) = unsafe { (a.as_native_set_ref().unwrap(), b.as_native_set_ref().unwrap()) };
        // Set members are ValueKeys (hashables only): key equality suffices.
        return sa.len() == sb.len() && sa.copy().iter().all(|k| sb.contains(k));
    }
    // Bytes, identity, and remaining cross-tag cases.
    a == b
}

/// Release partially-collected owned bindings when a compound pattern bails
/// mid-way (a later item mismatches or errors) -- they were clone_refcount'd
/// and would otherwise drop raw.
fn release_partial_bindings(bindings: &[(usize, Value)]) {
    for (_slot, v) in bindings {
        v.decref();
    }
}

fn vm_match_pattern(
    pattern: &VMPattern,
    value: Value,
    struct_reg: &PureStructRegistry,
    symbol_table: &SymbolTable,
    in_scope: &dyn Fn(&str) -> bool,
) -> VMResult<Option<Vec<(usize, Value)>>> {
    match pattern {
        VMPattern::Wildcard => Ok(Some(vec![])),
        VMPattern::Literal(lit) => {
            if let Some(eq) = arith::eq_native(*lit, value) {
                if eq { Ok(Some(vec![])) } else { Ok(None) }
            } else {
                Ok(None)
            }
        }
        VMPattern::Var(slot) => {
            value.clone_refcount();
            Ok(Some(vec![(*slot, value)]))
        }
        VMPattern::Or(pats) => {
            for pat in pats {
                if let Some(bindings) = vm_match_pattern(pat, value, struct_reg, symbol_table, in_scope)? {
                    return Ok(Some(bindings));
                }
            }
            Ok(None)
        }
        VMPattern::Tuple(elements) => {
            // Destructure tuple/list
            // Snapshot the elements without touching refcounts (bit-copies valid
            // while the subject is alive on the stack); the arms clone_refcount what
            // they bind. Using as_slice_cloned() here would incref every element and
            // then drop the Vec undropped -- a leak per list-subject match.
            let items = if value.is_native_tuple() {
                // SAFETY: the is_native_tuple() guard above proves value holds a live Arc<NativeTuple> owned on the stack; the borrow does not outlive it.
                let tuple = unsafe { value.as_native_tuple_ref().unwrap() };
                tuple.as_slice().to_vec()
            } else if value.is_native_list() {
                // SAFETY: the is_native_list() guard above proves value holds a live Arc<NativeList> owned on the stack; the borrow does not outlive it.
                let list = unsafe { value.as_native_list_ref().unwrap() };
                list.snapshot_items()
            } else {
                return Ok(None);
            };

            // Count fixed elements and find star position
            let mut star_pos = None;
            let mut fixed_count = 0;
            for (i, elem) in elements.iter().enumerate() {
                match elem {
                    VMPatternElement::Pattern(_) => fixed_count += 1,
                    VMPatternElement::Star(_) => star_pos = Some(i),
                }
            }

            // Sub-match helper: on a mismatch or an error the bindings already
            // collected for earlier items are released (owned clones).
            macro_rules! sub_or_bail {
                ($pat:expr, $item:expr, $bindings:expr) => {
                    match vm_match_pattern($pat, $item, struct_reg, symbol_table, in_scope) {
                        Ok(Some(sub)) => sub,
                        Ok(None) => {
                            release_partial_bindings(&$bindings);
                            return Ok(None);
                        }
                        Err(e) => {
                            release_partial_bindings(&$bindings);
                            return Err(e);
                        }
                    }
                };
            }

            if let Some(sp) = star_pos {
                // Star pattern: items >= fixed_count
                if items.len() < fixed_count {
                    return Ok(None);
                }
                let mut bindings = Vec::new();
                let after_count = elements.len() - sp - 1;
                let rest_len = items.len() - fixed_count;

                // Before star
                for i in 0..sp {
                    if let VMPatternElement::Pattern(ref pat) = elements[i] {
                        let sub = sub_or_bail!(pat, items[i], bindings);
                        bindings.extend(sub);
                    }
                }
                // Star itself
                if let VMPatternElement::Star(slot) = &elements[sp] {
                    if *slot != usize::MAX {
                        let rest: Vec<Value> = items[sp..sp + rest_len].to_vec();
                        for v in &rest {
                            v.clone_refcount();
                        }
                        bindings.push((*slot, Value::from_list(rest)));
                    }
                }
                // After star
                for i in 0..after_count {
                    let elem_idx = sp + 1 + i;
                    let item_idx = sp + rest_len + i;
                    if let VMPatternElement::Pattern(ref pat) = elements[elem_idx] {
                        let sub = sub_or_bail!(pat, items[item_idx], bindings);
                        bindings.extend(sub);
                    }
                }
                Ok(Some(bindings))
            } else {
                // No star: exact length match
                if items.len() != elements.len() {
                    return Ok(None);
                }
                let mut bindings = Vec::new();
                for (elem, item) in elements.iter().zip(items.iter()) {
                    if let VMPatternElement::Pattern(ref pat) = elem {
                        let sub = sub_or_bail!(pat, *item, bindings);
                        bindings.extend(sub);
                    }
                }
                Ok(Some(bindings))
            }
        }
        VMPattern::Struct {
            name,
            variant,
            field_slots,
        } => {
            // SAFETY: the owning Value lives on the stack for this transient borrow and is not decref'd while the reference is used.
            let Some(cell) = (unsafe { value.as_struct_ref() }) else {
                return Ok(None);
            };
            let Some(ty) = struct_reg.get_type(cell.type_id) else {
                return Ok(None);
            };
            let expected = match variant {
                Some(v) => qualified_name(name, v),
                None => name.clone(),
            };
            if ty.name != expected {
                return Ok(None);
            }
            let mut bindings = Vec::with_capacity(field_slots.len());
            for (field_name, slot) in field_slots {
                let Some(field_idx) = ty.field_index(field_name) else {
                    // Unknown field: the earlier fields' clones are owned.
                    release_partial_bindings(&bindings);
                    return Ok(None);
                };
                let val = cell.field(field_idx);
                val.clone_refcount();
                bindings.push((*slot, val));
            }
            Ok(Some(bindings))
        }
        VMPattern::Enum {
            enum_name,
            variant_name,
        } => {
            // The enum type must be resolvable in scope, exactly like the PyO3
            // runtime (both executors raise CatnipNameError on a pattern naming
            // a type never brought into scope -- decision 2026-07-06). Matching
            // by interned qualified name alone would silently accept it. Mirror
            // of LoadScope resolution: closure chain, then host globals; lazy
            // (inside the matcher) for OR-pattern parity.
            if !in_scope(enum_name) {
                return Err(VMError::NameError(enum_name.clone()));
            }
            // Resolve the expected symbol by looking up "EnumName.variant" in the symbol table
            let qname = qualified_name(enum_name, variant_name);
            let Some(expected_sym) = symbol_table.lookup(&qname) else {
                return Ok(None);
            };
            let expected = Value::from_symbol(expected_sym);
            if value.to_raw() == expected.to_raw() {
                Ok(Some(Vec::new()))
            } else {
                Ok(None)
            }
        }
    }
}

/// Match for assignment patterns (strict: error on mismatch).
fn vm_match_assign_pattern(
    pattern: &VMPattern,
    value: Value,
    struct_reg: &PureStructRegistry,
    symbol_table: &SymbolTable,
    in_scope: &dyn Fn(&str) -> bool,
) -> VMResult<Vec<(usize, Value)>> {
    match vm_match_pattern(pattern, value, struct_reg, symbol_table, in_scope)? {
        Some(bindings) => Ok(bindings),
        None => Err(VMError::RuntimeError(format!(
            "pattern match failed for value: {}",
            value.display_string()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Format spec
// ---------------------------------------------------------------------------

/// Apply a Python-style format spec to a value.
fn apply_format_spec(base: &str, value: &Value, spec: &str) -> String {
    // Handle .Nf (float precision)
    if let Some(prec_str) = spec.strip_prefix('.') {
        if let Some(prec_str) = prec_str.strip_suffix('f') {
            if let Ok(prec) = prec_str.parse::<usize>() {
                let f = value
                    .as_float()
                    .or_else(|| value.as_int().map(|i| i as f64))
                    .unwrap_or(0.0);
                return format!("{:.prec$}", f, prec = prec);
            }
        }
    }
    // Handle fill+align+width: [fill][<>^]width
    let bytes = spec.as_bytes();
    let (fill, align, width_start) = if bytes.len() >= 2 && matches!(bytes[1], b'<' | b'>' | b'^') {
        (bytes[0] as char, bytes[1] as char, 2)
    } else if !bytes.is_empty() && matches!(bytes[0], b'<' | b'>' | b'^') {
        (' ', bytes[0] as char, 1)
    } else {
        return base.to_string();
    };
    if let Ok(width) = spec[width_start..].parse::<usize>() {
        if base.len() >= width {
            return base.to_string();
        }
        let pad = width - base.len();
        let fill_str = |n| std::iter::repeat_n(fill, n).collect::<String>();
        match align {
            '<' => format!("{}{}", base, fill_str(pad)),
            '>' => format!("{}{}", fill_str(pad), base),
            '^' => {
                let left = pad / 2;
                let right = pad - left;
                format!("{}{}{}", fill_str(left), base, fill_str(right))
            }
            _ => base.to_string(),
        }
    } else {
        base.to_string()
    }
}

// ---------------------------------------------------------------------------
// Key building (registry- and op_hash-aware)
// ---------------------------------------------------------------------------

/// Adapter binding the VM and the host so `Value::to_key_ctx` can query the
/// struct registry and run a custom `op_hash` while building a dict/set key.
/// The host is needed only because `op_hash` runs through `call_vmfunc_sync`.
struct KeyBuilder<'a> {
    vm: &'a mut PureVM,
    host: &'a dyn VmHost,
}

impl KeyCtx for KeyBuilder<'_> {
    fn type_defines_op_eq(&self, type_id: u32) -> bool {
        self.vm.struct_registry.type_defines_op_eq(type_id)
    }

    fn type_op_hash_func(&self, type_id: u32) -> Option<u32> {
        self.vm.struct_registry.type_op_hash_func(type_id)
    }

    fn type_name(&self, type_id: u32) -> String {
        self.vm.struct_registry.type_name(type_id).to_string()
    }

    fn call_op_hash(&mut self, func_idx: u32, instance: Value) -> VMResult<i64> {
        // The synchronous call consumes one ref of its `self` arg (the callee
        // frame decrefs its locals on return); balance it so the live instance
        // keeps its refcount.
        instance.clone_refcount();
        let result = self.vm.call_vmfunc_sync(func_idx, &[instance], self.host)?;
        let h = result.as_int();
        result.decref();
        h.ok_or_else(|| VMError::TypeError("op_hash must return an int".into()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
