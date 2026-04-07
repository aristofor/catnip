// FILE: catnip_vm/src/vm/core.rs
//! PureVM dispatch loop -- pure Rust, no PyO3.
//!
//! Executes bytecode (CodeObject compiled by PureCompiler) using VmHost
//! for external operations. All values are native catnip_vm::Value.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use catnip_core::vm::opcode::VMOpCode;
use catnip_core::vm::{
    FOR_RANGE_JUMP_MASK, FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_SLOT_MASK, FOR_RANGE_SLOT_STOP_SHIFT,
    FOR_RANGE_STEP_BYTE_MASK, FOR_RANGE_STEP_JUMP_MASK, FOR_RANGE_STEP_SHIFT, FOR_RANGE_STEP_SIGN_SHIFT,
};
use indexmap::IndexMap;

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
use super::structs::{PureStructField, PureStructRegistry, PureStructType, PureTraitRegistry};
use catnip_core::symbols::{SymbolTable, qualified_name};

/// Interrupt check frequency (every ~64k instructions).
const INTERRUPT_CHECK_INTERVAL: u64 = 1 << 16;

/// Maximum call depth to prevent infinite recursion.
const MAX_FRAME_DEPTH: usize = 512;

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
    /// ND lambda stack for recursive ND operations (~~).
    /// Stores VMFunc indices of active ND lambdas.
    pub(crate) nd_lambda_stack: Vec<u32>,
    /// Optional import loader for .cat file imports.
    pub(crate) import_loader: Option<crate::loader::PureImportLoader>,
}

// Re-use PureFrame directly to avoid circular import
use super::frame::PureFrame;

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
            nd_lambda_stack: Vec::new(),
            import_loader: None,
        }
    }

    /// Create a PureVM with a shared interrupt flag.
    pub fn with_interrupt(interrupt: Arc<AtomicBool>) -> Self {
        let mut vm = Self::new();
        vm.interrupt_flag = interrupt;
        vm
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
            self.func_table.insert(PureFuncSlot {
                code: Arc::new(func.clone()),
                closure: None,
            });
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
            if self.instruction_count & (INTERRUPT_CHECK_INTERVAL - 1) == 0
                && self.interrupt_flag.load(Ordering::Relaxed)
            {
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
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Add, a, b)?;
                        frame.push(result);
                    }
                }
                VMOpCode::Sub => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_sub", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Sub, a, b)?;
                        frame.push(result);
                    }
                }
                VMOpCode::Mul => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_mul", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Mul, a, b)?;
                        frame.push(result);
                    }
                }
                VMOpCode::Div => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = host.binary_op(crate::host::BinaryOp::TrueDiv, a, b)?;
                    frame.push(result);
                }
                VMOpCode::FloorDiv => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = host.binary_op(crate::host::BinaryOp::FloorDiv, a, b)?;
                    frame.push(result);
                }
                VMOpCode::Mod => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = host.binary_op(crate::host::BinaryOp::Mod, a, b)?;
                    frame.push(result);
                }
                VMOpCode::Pow => {
                    let b = frame.pop();
                    let a = frame.pop();
                    let result = host.binary_op(crate::host::BinaryOp::Pow, a, b)?;
                    frame.push(result);
                }

                // =============================================================
                // Tier 2: Unary
                // =============================================================
                VMOpCode::Neg => {
                    let a = frame.pop();
                    frame.push(arith::numeric_neg(a)?);
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
                    match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => frame.push(Value::from_int(ai & bi)),
                        _ => {
                            if let Some(v) = arith::bigint_binop(a, b, |x, y| rug::Integer::from(x & y)) {
                                frame.push(v);
                            } else {
                                return Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITAND.into()));
                            }
                        }
                    }
                }
                VMOpCode::BOr => {
                    let b = frame.pop();
                    let a = frame.pop();
                    match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => frame.push(Value::from_int(ai | bi)),
                        _ => {
                            if let Some(v) = arith::bigint_binop(a, b, |x, y| rug::Integer::from(x | y)) {
                                frame.push(v);
                            } else {
                                return Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITOR.into()));
                            }
                        }
                    }
                }
                VMOpCode::BXor => {
                    let b = frame.pop();
                    let a = frame.pop();
                    match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => frame.push(Value::from_int(ai ^ bi)),
                        _ => {
                            if let Some(v) = arith::bigint_binop(a, b, |x, y| rug::Integer::from(x ^ y)) {
                                frame.push(v);
                            } else {
                                return Err(VMError::TypeError(errors::ERR_UNSUPPORTED_BITXOR.into()));
                            }
                        }
                    }
                }
                VMOpCode::BNot => {
                    let a = frame.pop();
                    if let Some(i) = a.as_int() {
                        frame.push(Value::from_int(!i));
                    } else if a.is_bigint() {
                        let n = unsafe { a.as_bigint_ref().unwrap() };
                        frame.push(Value::from_bigint_or_demote(rug::Integer::from(!n)));
                    } else {
                        return Err(VMError::TypeError(errors::ERR_BAD_UNARY_NOT.into()));
                    }
                }
                VMOpCode::LShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => {
                            if bi < 0 {
                                return Err(VMError::ValueError("negative shift count".into()));
                            }
                            if bi < 64 {
                                if let Some(v) = Value::try_from_int(ai << bi) {
                                    frame.push(v);
                                } else {
                                    frame.push(Value::from_bigint_or_demote(rug::Integer::from(ai) << bi as u32));
                                }
                            } else {
                                frame.push(Value::from_bigint_or_demote(rug::Integer::from(ai) << bi as u32));
                            }
                        }
                        _ => {
                            return Err(VMError::TypeError(errors::ERR_UNSUPPORTED_LSHIFT.into()));
                        }
                    }
                }
                VMOpCode::RShift => {
                    let b = frame.pop();
                    let a = frame.pop();
                    match (a.as_int(), b.as_int()) {
                        (Some(ai), Some(bi)) => {
                            if bi < 0 {
                                return Err(VMError::ValueError("negative shift count".into()));
                            }
                            frame.push(Value::from_int(ai >> bi.min(63)));
                        }
                        _ => {
                            return Err(VMError::TypeError(errors::ERR_UNSUPPORTED_RSHIFT.into()));
                        }
                    }
                }

                // =============================================================
                // Tier 2: Comparisons
                // =============================================================
                VMOpCode::Lt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_lt", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Lt, a, b)?;
                        frame.push(result);
                    }
                }
                VMOpCode::Le => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_le", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Le, a, b)?;
                        frame.push(result);
                    }
                }
                VMOpCode::Gt => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_gt", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Gt, a, b)?;
                        frame.push(result);
                    }
                }
                VMOpCode::Ge => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_ge", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else {
                        let result = host.binary_op(crate::host::BinaryOp::Ge, a, b)?;
                        frame.push(result);
                    }
                }
                VMOpCode::Eq => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_eq", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else if let Some(eq) = arith::eq_native(a, b) {
                        frame.push(Value::from_bool(eq));
                    } else {
                        frame.push(Value::from_bool(a.bits() == b.bits()));
                    }
                }
                VMOpCode::Ne => {
                    let b = frame.pop();
                    let a = frame.pop();
                    if let Some(func_idx) = self.struct_binary_op("op_ne", a, b) {
                        self.call_struct_op(func_idx, a, b, frame)?;
                    } else if let Some(eq) = arith::eq_native(a, b) {
                        frame.push(Value::from_bool(!eq));
                    } else {
                        frame.push(Value::from_bool(a.bits() != b.bits()));
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

                    // Capture only variables that the child function references via LoadScope/StoreScope.
                    // func_code.names contains all externally-referenced names in the child.
                    let mut captured = IndexMap::new();
                    if let Some(ref parent_code) = frame.code {
                        let child_names: std::collections::HashSet<&str> =
                            func_code.names.iter().map(|s| s.as_str()).collect();
                        for (name, &slot_idx) in &parent_code.slotmap {
                            if !child_names.contains(name.as_str()) {
                                continue;
                            }
                            if host.has_global(name) {
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
                    let new_idx = self.func_table.insert(PureFuncSlot {
                        code: func_code,
                        closure: Some(closure),
                    });
                    frame.push(Value::from_vmfunc(new_idx));
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

                    if func.is_struct_type() {
                        let type_id = func.as_struct_type_id().unwrap();
                        let result = self.construct_struct(type_id, &args, &IndexMap::new())?;
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
                                return Err(VMError::FrameOverflow);
                            }
                            // Push instance onto caller stack first -- init's return will be discarded.
                            // No incref needed: caller owns the ref (refcount=1 from create_instance),
                            // init receives the same index as a read-only self parameter.
                            frame.push(result);
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
                    } else if func.is_vmfunc() && !func.is_invalid() {
                        let idx = func.as_vmfunc_idx();
                        let slot = self
                            .func_table
                            .get(idx)
                            .ok_or_else(|| VMError::RuntimeError("invalid function index".into()))?;
                        let callee_code = Arc::clone(&slot.code);
                        let closure = slot.closure.clone();

                        if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                            return Err(VMError::FrameOverflow);
                        }

                        let mut new_frame = self.frame_pool.alloc_with_code(Arc::clone(&callee_code));
                        new_frame.bind_args(&args);
                        new_frame.closure_scope = closure;

                        // Save current frame, switch to new
                        let old_frame = std::mem::replace(frame, new_frame);
                        self.frame_stack.push(old_frame);
                    } else if Self::is_nd_recur_sentinel(func) {
                        // ND recursion callback: recur(value)
                        let result = self.handle_nd_recur_call(&args, host)?;
                        frame.push(result);
                    } else if let Some((tag, inner)) = Self::check_nd_wrapper(func) {
                        // ND declaration/lift wrapper call
                        let result = if tag == super::broadcast::ND_DECL_TAG {
                            self.handle_nd_decl_call(inner, &args, host)?
                        } else {
                            self.handle_nd_lift_call(inner, &args, host)?
                        };
                        frame.push(result);
                    } else if func.is_native_str() {
                        let name = unsafe { func.as_native_str_ref().unwrap() };
                        if name == "isinstance" && args.len() == 2 {
                            let result = self.check_isinstance(args[0], args[1]);
                            func.decref();
                            for a in &args {
                                a.decref();
                            }
                            frame.push(result?);
                        } else if name == "import" && self.import_loader.is_some() {
                            let result = self.handle_import(&args, &[], host)?;
                            frame.push(result);
                        } else if let Some(hof) = Self::try_build_hof(name, &args) {
                            // HofCall owns func, iterable, and init (fold).
                            // Only the builtin name string is untracked.
                            func.decref();
                            return Err(VMError::HofBuiltin(hof));
                        } else {
                            let result = host.call_function(func, &args)?;
                            frame.push(result);
                        }
                    } else {
                        // Delegate to host
                        let result = host.call_function(func, &args)?;
                        frame.push(result);
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

                    if func.is_vmfunc() && !func.is_invalid() {
                        let idx = func.as_vmfunc_idx();
                        let slot = self
                            .func_table
                            .get(idx)
                            .ok_or_else(|| VMError::RuntimeError("invalid function index".into()))?;
                        let callee_code = Arc::clone(&slot.code);
                        let closure = slot.closure.clone();

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
                        frame.block_stack.clear();
                        frame.match_bindings = None;
                        frame.closure_scope = closure;
                        frame.bind_args(&args);
                    } else if func.is_native_str() {
                        let name = unsafe { func.as_native_str_ref().unwrap() };
                        let result = if name == "isinstance" && args.len() == 2 {
                            let r = self.check_isinstance(args[0], args[1]);
                            func.decref();
                            for a in &args {
                                a.decref();
                            }
                            r?
                        } else if name == "import" && self.import_loader.is_some() {
                            self.handle_import(&args, &[], host)?
                        } else if let Some(hof) = Self::try_build_hof(name, &args) {
                            func.decref();
                            return Err(VMError::HofBuiltin(hof));
                        } else {
                            host.call_function(func, &args)?
                        };
                        // Return the result up the call stack
                        if self.frame_stack.len() > base_depth {
                            let caller = self.frame_stack.pop().unwrap();
                            let old = std::mem::replace(frame, caller);
                            self.frame_pool.free(old);
                            frame.push(result);
                        } else {
                            return Ok(result);
                        }
                    } else {
                        // Non-VMFunc, non-builtin: delegate to host
                        let result = host.call_function(func, &args)?;
                        if self.frame_stack.len() > base_depth {
                            let caller = self.frame_stack.pop().unwrap();
                            let old = std::mem::replace(frame, caller);
                            self.frame_pool.free(old);
                            frame.push(result);
                        } else {
                            return Ok(result);
                        }
                    }
                }
                VMOpCode::CallMethod => {
                    // arg = (name_idx << 16) | nargs
                    let name_idx = (instr.arg >> 16) as usize;
                    let nargs = (instr.arg & 0xFFFF) as usize;

                    let mut args = Vec::with_capacity(nargs);
                    for _ in 0..nargs {
                        args.push(frame.pop());
                    }
                    args.reverse();
                    let obj = frame.pop();
                    let method_name = &code.names[name_idx];

                    if let Some(inst_idx) = obj.as_struct_instance_idx() {
                        let inst = self
                            .struct_registry
                            .get_instance(inst_idx)
                            .ok_or_else(|| VMError::RuntimeError("freed struct instance".into()))?;
                        let type_id = inst.type_id;
                        let ty = self
                            .struct_registry
                            .get_type(type_id)
                            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;
                        if let Some(&func_idx) = ty.methods.get(method_name) {
                            let slot = self
                                .func_table
                                .get(func_idx)
                                .ok_or_else(|| VMError::RuntimeError("invalid method function".into()))?;
                            let callee_code = Arc::clone(&slot.code);
                            let closure = slot.closure.clone();
                            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                return Err(VMError::FrameOverflow);
                            }
                            // Prepend self to args
                            let mut full_args = Vec::with_capacity(1 + args.len());
                            full_args.push(obj);
                            full_args.extend(args);
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&full_args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                        } else if let Some(&func_idx) = ty.static_methods.get(method_name) {
                            let slot = self
                                .func_table
                                .get(func_idx)
                                .ok_or_else(|| VMError::RuntimeError("invalid static method".into()))?;
                            let callee_code = Arc::clone(&slot.code);
                            let closure = slot.closure.clone();
                            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                return Err(VMError::FrameOverflow);
                            }
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                        } else {
                            let ty_name = ty.name.clone();
                            return Err(VMError::RuntimeError(format!(
                                "'{}' has no method '{}'",
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
                                return Err(VMError::FrameOverflow);
                            }
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                        } else {
                            let ty_name = ty.name.clone();
                            return Err(VMError::RuntimeError(format!(
                                "type '{}' has no static method '{}'",
                                ty_name, method_name
                            )));
                        }
                    } else if obj.is_module() {
                        let ns = unsafe { obj.as_module_ref().unwrap() };
                        let func_val = *ns.attrs.get(method_name).ok_or_else(|| {
                            VMError::AttributeError(format!("module '{}' has no attribute '{}'", ns.name, method_name))
                        })?;
                        if func_val.is_vmfunc() && !func_val.is_invalid() {
                            let idx = func_val.as_vmfunc_idx();
                            let slot = self
                                .func_table
                                .get(idx)
                                .ok_or_else(|| VMError::RuntimeError("invalid function index".into()))?;
                            let callee_code = Arc::clone(&slot.code);
                            let closure = slot.closure.clone();
                            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                                return Err(VMError::FrameOverflow);
                            }
                            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
                            new_frame.bind_args(&args);
                            new_frame.closure_scope = closure;
                            let old_frame = std::mem::replace(frame, new_frame);
                            self.frame_stack.push(old_frame);
                        } else {
                            // Delegate to host for NativeStr builtins etc.
                            let result = host.call_function(func_val, &args)?;
                            frame.push(result);
                        }
                    } else {
                        let result = host.call_method(obj, method_name, &args)?;
                        frame.push(result);
                    }
                }
                VMOpCode::CallKw => {
                    let nargs = ((instr.arg >> 8) & 0xFF) as usize;
                    let nkwargs = (instr.arg & 0xFF) as usize;

                    // Pop kw_names tuple
                    let kw_names_val = frame.pop();
                    let kw_names: Vec<String> = if kw_names_val.is_native_tuple() {
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
                    let func = frame.pop();

                    if func.is_vmfunc() && !func.is_invalid() {
                        let idx = func.as_vmfunc_idx();
                        let slot = self
                            .func_table
                            .get(idx)
                            .ok_or_else(|| VMError::RuntimeError("invalid function index".into()))?;
                        let callee_code = Arc::clone(&slot.code);
                        let closure = slot.closure.clone();

                        if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                            return Err(VMError::FrameOverflow);
                        }

                        let mut new_frame = self.frame_pool.alloc_with_code(Arc::clone(&callee_code));
                        new_frame.bind_args(&args);
                        // Bind kwargs by name
                        for (name, val) in kw_names.iter().zip(kw_values.iter()) {
                            if let Some(&slot_idx) = callee_code.slotmap.get(name) {
                                new_frame.set_local(slot_idx, *val);
                            }
                        }
                        new_frame.closure_scope = closure;
                        let old_frame = std::mem::replace(frame, new_frame);
                        self.frame_stack.push(old_frame);
                    } else if func.is_native_str() {
                        let name = unsafe { func.as_native_str_ref().unwrap() };
                        match name {
                            "import" if self.import_loader.is_some() => {
                                let kw: Vec<(String, Value)> = kw_names.into_iter().zip(kw_values).collect();
                                let result = self.handle_import(&args, &kw, host)?;
                                frame.push(result);
                            }
                            "dict" => {
                                let dict = Value::from_empty_dict();
                                let d = unsafe { dict.as_native_dict_ref().unwrap() };
                                for (kname, val) in kw_names.iter().zip(kw_values.iter()) {
                                    let key = crate::collections::ValueKey::Str(Arc::new(
                                        crate::value::NativeString::new(kname.clone()),
                                    ));
                                    d.set_item(key, *val);
                                }
                                frame.push(dict);
                            }
                            _ => {
                                return Err(VMError::TypeError(format!(
                                    "{}() does not accept keyword arguments in PureVM",
                                    name
                                )));
                            }
                        }
                    } else {
                        return Err(VMError::TypeError("CallKw: cannot call non-function".into()));
                    }
                    kw_names_val.decref();
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
                        set.insert(v.to_key()?);
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
                    let d = unsafe { dict.as_native_dict_ref().unwrap() };
                    for chunk in pairs.chunks(2) {
                        let key = chunk[0].to_key()?;
                        d.set_item(key, chunk[1]);
                        chunk[0].decref(); // key consumed by to_key
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
                    let iter = host.get_iter(obj)?;
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
                    if let Some(inst_idx) = obj.as_struct_instance_idx() {
                        let result = self.struct_getattr(inst_idx, name)?;
                        frame.push(result);
                    } else if let Some(type_id) = obj.as_struct_type_id() {
                        // Static method access on the type itself
                        let ty = self
                            .struct_registry
                            .get_type(type_id)
                            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;
                        if let Some(&func_idx) = ty.static_methods.get(name) {
                            frame.push(Value::from_vmfunc(func_idx));
                        } else {
                            return Err(VMError::RuntimeError(format!(
                                "type '{}' has no static method '{}'",
                                ty.name, name
                            )));
                        }
                    } else if let Some(enum_type_id) = obj.as_enum_type_id() {
                        match self.enum_registry.get_variant_value(enum_type_id, name) {
                            Some(val) => frame.push(val),
                            None => {
                                let ety = self.enum_registry.get_type(enum_type_id).unwrap();
                                return Err(VMError::RuntimeError(format!(
                                    "enum '{}' has no variant '{}'",
                                    ety.name, name
                                )));
                            }
                        }
                    } else if obj.is_module() {
                        let ns = unsafe { obj.as_module_ref().unwrap() };
                        let val = ns.attrs.get(name).ok_or_else(|| {
                            VMError::AttributeError(format!("module '{}' has no attribute '{}'", ns.name, name))
                        })?;
                        val.clone_refcount();
                        frame.push(*val);
                    } else {
                        let result = host.obj_getattr(obj, name)?;
                        frame.push(result);
                    }
                }
                VMOpCode::SetAttr => {
                    let name_idx = instr.arg as usize;
                    let name = &code.names[name_idx];
                    let val = frame.pop();
                    let obj = frame.pop();
                    if let Some(inst_idx) = obj.as_struct_instance_idx() {
                        self.struct_setattr(inst_idx, name, val)?;
                    } else {
                        host.obj_setattr(obj, name, val)?;
                    }
                }
                VMOpCode::GetItem => {
                    if instr.arg == 1 {
                        // Slice mode: stack has [obj, start, stop, step]
                        let step = frame.pop();
                        let stop = frame.pop();
                        let start = frame.pop();
                        let obj = frame.pop();
                        let result = crate::host::apply_slice(obj, start, stop, step)?;
                        frame.push(result);
                    } else {
                        let key = frame.pop();
                        let obj = frame.pop();
                        let result = host.obj_getitem(obj, key)?;
                        frame.push(result);
                    }
                }
                VMOpCode::SetItem => {
                    let val = frame.pop();
                    let key = frame.pop();
                    let obj = frame.pop();
                    host.obj_setitem(obj, key, val)?;
                }

                // =============================================================
                // Tier 2: Membership + identity
                // =============================================================
                VMOpCode::In => {
                    let container = frame.pop();
                    let item = frame.pop();
                    let result = host.contains_op(item, container)?;
                    frame.push(Value::from_bool(result));
                }
                VMOpCode::NotIn => {
                    let container = frame.pop();
                    let item = frame.pop();
                    let result = host.contains_op(item, container)?;
                    frame.push(Value::from_bool(!result));
                }
                VMOpCode::Is => {
                    let b = frame.pop();
                    let a = frame.pop();
                    frame.push(Value::from_bool(a.bits() == b.bits()));
                }
                VMOpCode::IsNot => {
                    let b = frame.pop();
                    let a = frame.pop();
                    frame.push(Value::from_bool(a.bits() != b.bits()));
                }

                // =============================================================
                // Tier 2: Unpack
                // =============================================================
                VMOpCode::UnpackSequence => {
                    let n = instr.arg as usize;
                    let seq = frame.pop();
                    let items = self.unpack_to_vec(seq)?;
                    if items.len() != n {
                        return Err(VMError::RuntimeError(format!(
                            "cannot unpack {} values into {} variables",
                            items.len(),
                            n
                        )));
                    }
                    // Push in reverse order (first item ends on top)
                    for item in items.into_iter().rev() {
                        frame.push(item);
                    }
                }
                VMOpCode::UnpackEx => {
                    let before = ((instr.arg >> 8) & 0xFF) as usize;
                    let after = (instr.arg & 0xFF) as usize;
                    let seq = frame.pop();
                    let items = self.unpack_to_vec(seq)?;

                    let total_fixed = before + after;
                    if items.len() < total_fixed {
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

                    // Push in reverse order: after, rest (as list), before
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
                }

                // =============================================================
                // Tier 2: Pattern matching
                // =============================================================
                VMOpCode::MatchPatternVM => {
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    let pattern = code.patterns.get(pat_idx).cloned();
                    match pattern {
                        Some(ref pat) => {
                            match vm_match_pattern(pat, value, &self.struct_registry, &self.symbol_table) {
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
                VMOpCode::MatchAssignPatternVM => {
                    let pat_idx = instr.arg as usize;
                    let value = frame.pop();
                    let pattern = code.patterns.get(pat_idx).cloned();
                    match pattern {
                        Some(ref pat) => {
                            let bindings =
                                vm_match_assign_pattern(pat, value, &self.struct_registry, &self.symbol_table)?;
                            frame.match_bindings = Some(bindings);
                            frame.push(Value::TRUE);
                        }
                        None => {
                            return Err(VMError::RuntimeError("invalid assignment pattern index".into()));
                        }
                    }
                }
                VMOpCode::BindMatch => {
                    if let Some(bindings) = frame.match_bindings.take() {
                        frame.pop(); // pop sentinel TRUE
                        for (slot, val) in bindings {
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
                    if let Some(idx) = val.as_struct_instance_idx() {
                        let name = self.struct_registry.instance_type_name(idx).to_string();
                        frame.push(Value::from_string(name));
                    } else if let Some(sym_id) = val.as_symbol() {
                        // Enum variant: resolve to the declaring enum type name
                        if let Some((type_id, _)) = self.enum_registry.lookup_symbol(sym_id) {
                            let name = &self.enum_registry.get_type(type_id).unwrap().name;
                            frame.push(Value::from_string(name.clone()));
                        } else {
                            frame.push(Value::from_str("symbol"));
                        }
                    } else {
                        frame.push(Value::from_str(val.type_name()));
                    }
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
                        let key =
                            crate::collections::ValueKey::Str(std::sync::Arc::new(crate::value::NativeString::new(k)));
                        items.insert(key, v);
                    }
                    frame.push(Value::from_dict(items));
                }

                VMOpCode::Locals => {
                    let mut items: indexmap::IndexMap<crate::collections::ValueKey, Value> = indexmap::IndexMap::new();
                    if self.frame_stack.is_empty() {
                        // Module level: locals() == globals()
                        for (k, v) in host.collect_all_globals() {
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
                        let result = self.broadcast_nd_recursion(target, lambda, host)?;
                        frame.push(result);
                    } else if is_nd_map {
                        let func = frame.pop();
                        let target = frame.pop();
                        let result = self.nd_map_apply(target, func, host)?;
                        frame.push(result);
                    } else {
                        let operand = if has_operand { Some(frame.pop()) } else { None };
                        let operator = frame.pop();
                        let target = frame.pop();
                        let result = self.apply_broadcast(target, operator, operand, is_filter, host)?;
                        frame.push(result);
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
                        let result = self.nd_recursion_call(seed, lambda, host)?;
                        frame.push(result);
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
                        let result = self.nd_map_apply(data, func, host)?;
                        frame.push(result);
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
                        let err = if let Some(inst_idx) = val.as_struct_instance_idx() {
                            let type_name = self.struct_registry.instance_type_name(inst_idx).to_string();
                            let inst = self.struct_registry.get_instance(inst_idx);
                            let msg = inst
                                .and_then(|i| i.fields.first().copied())
                                .map(|v| v.display_string())
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
            let inst_idx = obj.as_struct_instance_idx().unwrap();
            let inst = self
                .struct_registry
                .get_instance(inst_idx)
                .ok_or_else(|| VMError::RuntimeError("invalid struct instance".into()))?;
            // Direct match
            if inst.type_id == target_id {
                return Ok(true);
            }
            // Check MRO by stable type id (inheritance chain)
            if let Some(source) = self.struct_registry.get_type(inst.type_id) {
                return Ok(source.mro_ids.contains(&target_id));
            }
            return Ok(false);
        }

        // Builtin type name as NativeStr: compare with obj.type_name()
        if type_spec.is_native_str() {
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

    /// Call a function value synchronously via re-entrant dispatch.
    /// Enforces the same MAX_FRAME_DEPTH limit as the normal Call handler.
    fn call_func_sync(&mut self, func: Value, args: &[Value], host: &dyn VmHost) -> VMResult<Value> {
        if func.is_vmfunc() && !func.is_invalid() {
            if self.frame_stack.len() >= MAX_FRAME_DEPTH {
                return Err(VMError::FrameOverflow);
            }
            let idx = func.as_vmfunc_idx();
            let slot = self
                .func_table
                .get(idx)
                .ok_or_else(|| VMError::RuntimeError("invalid function index".into()))?;
            let callee_code = Arc::clone(&slot.code);
            let closure = slot.closure.clone();
            let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
            for a in args {
                a.clone_refcount();
            }
            new_frame.bind_args(args);
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

    /// Get a Globals Rc from the host (for building closure parents).
    fn get_globals_rc(&self, host: &dyn VmHost) -> Option<crate::host::Globals> {
        host.globals_rc()
    }

    /// Unpack a value (list/tuple) into a Vec for UnpackSequence/UnpackEx.
    fn unpack_to_vec(&self, val: Value) -> VMResult<Vec<Value>> {
        if val.is_native_list() {
            let list = unsafe { val.as_native_list_ref().unwrap() };
            return Ok(list.as_slice_cloned());
        }
        if val.is_native_tuple() {
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
        self.func_table.insert(PureFuncSlot { code, closure: None })
    }

    // --- Struct helpers ---

    /// Handle MakeStruct opcode: parse metadata, register type, store in globals.
    fn handle_make_struct(&mut self, info: Value, frame: &mut PureFrame, host: &dyn VmHost) -> VMResult<()> {
        let tuple = unsafe {
            info.as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeStruct: expected tuple constant".into()))?
        };
        let items = tuple.as_slice();
        if items.len() < 3 {
            return Err(VMError::RuntimeError("MakeStruct: malformed metadata".into()));
        }

        // Parse name
        let name = unsafe {
            items[0]
                .as_native_str_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeStruct: name must be string".into()))?
                .to_string()
        };

        // Parse fields: tuple of (name, has_default) tuples
        let fields_tuple = unsafe { items[1].as_native_tuple_ref().unwrap() };
        let num_defaults = items[2].as_int().unwrap_or(0) as usize;

        let mut fields = Vec::new();
        let mut default_slot = 0usize;
        for fv in fields_tuple.as_slice() {
            let ft = unsafe { fv.as_native_tuple_ref().unwrap() };
            let fs = ft.as_slice();
            let fname = unsafe { fs[0].as_native_str_ref().unwrap().to_string() };
            let has_default = fs[1].as_bool().unwrap_or(false);
            let slot = if has_default {
                let s = default_slot;
                default_slot += 1;
                Some(s)
            } else {
                None
            };
            fields.push(PureStructField {
                name: fname,
                has_default,
                default_slot: slot,
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
            let impl_tuple = unsafe { items[3].as_native_tuple_ref() };
            if let Some(impls) = impl_tuple {
                for iv in impls.as_slice() {
                    if let Some(s) = unsafe { iv.as_native_str_ref() } {
                        implements.push(s.to_string());
                    }
                }
            }
            if !items[4].is_nil() {
                if let Some(bases) = unsafe { items[4].as_native_tuple_ref() } {
                    for bv in bases.as_slice() {
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
                        prepend_defaults.push(tf.default);
                        Some(prepend_defaults.len() - 1)
                    } else {
                        None
                    };
                    prepend_fields.push(PureStructField {
                        name: tf.name.clone(),
                        has_default,
                        default_slot: slot,
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
                                inherited_defaults.push(parent_ty.defaults[slot]);
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
        let list = unsafe {
            methods_val
                .as_native_list_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeStruct: methods must be a list".into()))?
        };
        let items = list.as_slice_cloned();
        for entry in &items {
            let t = unsafe {
                entry
                    .as_native_tuple_ref()
                    .ok_or_else(|| VMError::RuntimeError("MakeStruct: method entry must be tuple".into()))?
            };
            let parts = t.as_slice();
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

        let tuple = unsafe {
            info.as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeTrait: expected tuple constant".into()))?
        };
        let items = tuple.as_slice();
        if items.len() < 4 {
            return Err(VMError::RuntimeError("MakeTrait: malformed metadata".into()));
        }

        let name = unsafe { items[0].as_native_str_ref().unwrap().to_string() };

        // extends tuple
        let mut extends = Vec::new();
        if let Some(ext) = unsafe { items[1].as_native_tuple_ref() } {
            for ev in ext.as_slice() {
                if let Some(s) = unsafe { ev.as_native_str_ref() } {
                    extends.push(s.to_string());
                }
            }
        }

        // fields
        let fields_tuple = unsafe { items[2].as_native_tuple_ref().unwrap() };
        let num_defaults = items[3].as_int().unwrap_or(0) as usize;

        let mut fields = Vec::new();
        let mut defaults_popped = vec![Value::NIL; num_defaults];
        for i in (0..num_defaults).rev() {
            defaults_popped[i] = frame.pop();
        }

        let mut default_idx = 0usize;
        for fv in fields_tuple.as_slice() {
            let ft = unsafe { fv.as_native_tuple_ref().unwrap() };
            let fs = ft.as_slice();
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
        let tuple = unsafe {
            info.as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeEnum: expected tuple constant".into()))?
        };
        let items = tuple.as_slice();
        if items.len() < 2 {
            return Err(VMError::RuntimeError("MakeEnum: malformed metadata".into()));
        }

        let name = unsafe {
            items[0]
                .as_native_str_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeEnum: expected string name".into()))?
                .to_string()
        };

        // Extract variant names
        let variants_tuple = unsafe {
            items[1]
                .as_native_tuple_ref()
                .ok_or_else(|| VMError::RuntimeError("MakeEnum: expected variants tuple".into()))?
        };
        let mut variant_names = Vec::new();
        for v in variants_tuple.as_slice() {
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

    /// Construct a struct instance from args and defaults.
    fn construct_struct(&mut self, type_id: u32, args: &[Value], _kwargs: &IndexMap<String, Value>) -> VMResult<Value> {
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

        if args.len() < required {
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

        // Build field values: args fill from left, defaults fill the rest
        let ty = self.struct_registry.get_type(type_id).unwrap();
        let mut field_values = Vec::with_capacity(num_fields);
        for (i, f) in ty.fields.iter().enumerate() {
            if i < args.len() {
                field_values.push(args[i]);
            } else if let Some(slot) = f.default_slot {
                let default = ty.defaults[slot];
                default.clone_refcount();
                field_values.push(default);
            } else {
                return Err(VMError::RuntimeError(format!(
                    "{}(): missing argument for field '{}'",
                    ty_name, f.name
                )));
            }
        }

        let idx = self.struct_registry.create_instance(type_id, field_values);
        Ok(Value::from_struct_instance(idx))
    }

    /// Get attribute from a struct instance (field or method).
    fn struct_getattr(&mut self, inst_idx: u32, name: &str) -> VMResult<Value> {
        let inst = self
            .struct_registry
            .get_instance(inst_idx)
            .ok_or_else(|| VMError::RuntimeError("freed struct instance".into()))?;
        let type_id = inst.type_id;
        let ty = self
            .struct_registry
            .get_type(type_id)
            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;

        // Field access
        if let Some(field_idx) = ty.field_index(name) {
            let val = inst.fields[field_idx];
            val.clone_refcount();
            if let Some(si) = val.as_struct_instance_idx() {
                self.struct_registry.incref(si);
            }
            return Ok(val);
        }

        // Method access: create a bound closure that captures self
        if let Some(&func_idx) = ty.methods.get(name) {
            let slot = self
                .func_table
                .get(func_idx)
                .ok_or_else(|| VMError::RuntimeError("invalid method function".into()))?;
            let method_code = Arc::clone(&slot.code);
            let method_closure = slot.closure.clone();

            // Build a closure scope with self captured
            let self_val = Value::from_struct_instance(inst_idx);
            self.struct_registry.incref(inst_idx);
            let mut captured = IndexMap::new();
            captured.insert("self".to_string(), self_val);
            let parent = match method_closure {
                Some(mc) => PureClosureParent::Scope(mc),
                None => PureClosureParent::None,
            };
            let bound_closure = PureClosureScope::new(captured, parent);

            let bound_idx = self.func_table.insert(PureFuncSlot {
                code: method_code,
                closure: Some(bound_closure),
            });
            return Ok(Value::from_vmfunc(bound_idx));
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

    /// Display a value, using the struct registry for struct instances.
    pub(crate) fn display_value(&self, val: &Value) -> String {
        if let Some(idx) = val.as_struct_instance_idx() {
            self.struct_registry
                .display_instance(idx, |v| self.display_value_repr(v))
        } else {
            val.display_string()
        }
    }

    /// Repr a value, using the struct registry for struct instances.
    pub(crate) fn display_value_repr(&self, val: &Value) -> String {
        if let Some(idx) = val.as_struct_instance_idx() {
            self.struct_registry
                .display_instance(idx, |v| self.display_value_repr(v))
        } else {
            val.repr_string()
        }
    }

    /// Try to dispatch a binary op on struct instances via operator methods.
    /// Checks left operand first, then right (reverse dispatch).
    fn struct_binary_op(&self, op_name: &str, a: Value, b: Value) -> Option<u32> {
        if let Some(idx) = a.as_struct_instance_idx() {
            let inst = self.struct_registry.get_instance(idx)?;
            let ty = self.struct_registry.get_type(inst.type_id)?;
            if let Some(&func_idx) = ty.methods.get(op_name) {
                return Some(func_idx);
            }
        }
        if let Some(idx) = b.as_struct_instance_idx() {
            let inst = self.struct_registry.get_instance(idx)?;
            let ty = self.struct_registry.get_type(inst.type_id)?;
            return ty.methods.get(op_name).copied();
        }
        None
    }

    /// Call a struct operator method: pushes a new frame with self=a and other=b.
    fn call_struct_op(&mut self, func_idx: u32, a: Value, b: Value, frame: &mut PureFrame) -> VMResult<()> {
        let slot = self
            .func_table
            .get(func_idx)
            .ok_or_else(|| VMError::RuntimeError("invalid operator method".into()))?;
        let callee_code = Arc::clone(&slot.code);
        let closure = slot.closure.clone();
        if self.frame_stack.len() >= MAX_FRAME_DEPTH {
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
    fn struct_setattr(&mut self, inst_idx: u32, name: &str, val: Value) -> VMResult<()> {
        let inst = self
            .struct_registry
            .get_instance(inst_idx)
            .ok_or_else(|| VMError::RuntimeError("freed struct instance".into()))?;
        let type_id = inst.type_id;
        let ty = self
            .struct_registry
            .get_type(type_id)
            .ok_or_else(|| VMError::RuntimeError("invalid struct type".into()))?;

        if let Some(field_idx) = ty.field_index(name) {
            let inst = self.struct_registry.get_instance_mut(inst_idx).unwrap();
            let old = inst.fields[field_idx];
            inst.fields[field_idx] = val;
            // Decref old value
            old.decref();
            if let Some(si) = old.as_struct_instance_idx() {
                self.struct_registry.decref(si);
            }
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
fn vm_match_pattern(
    pattern: &VMPattern,
    value: Value,
    struct_reg: &PureStructRegistry,
    symbol_table: &SymbolTable,
) -> Option<Vec<(usize, Value)>> {
    match pattern {
        VMPattern::Wildcard => Some(vec![]),
        VMPattern::Literal(lit) => {
            if let Some(eq) = arith::eq_native(*lit, value) {
                if eq { Some(vec![]) } else { None }
            } else {
                None
            }
        }
        VMPattern::Var(slot) => {
            value.clone_refcount();
            Some(vec![(*slot, value)])
        }
        VMPattern::Or(pats) => {
            for pat in pats {
                if let Some(bindings) = vm_match_pattern(pat, value, struct_reg, symbol_table) {
                    return Some(bindings);
                }
            }
            None
        }
        VMPattern::Tuple(elements) => {
            // Destructure tuple/list
            let items = if value.is_native_tuple() {
                let tuple = unsafe { value.as_native_tuple_ref().unwrap() };
                tuple.as_slice().to_vec()
            } else if value.is_native_list() {
                let list = unsafe { value.as_native_list_ref().unwrap() };
                list.as_slice_cloned()
            } else {
                return None;
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

            if let Some(sp) = star_pos {
                // Star pattern: items >= fixed_count
                if items.len() < fixed_count {
                    return None;
                }
                let mut bindings = Vec::new();
                let after_count = elements.len() - sp - 1;
                let rest_len = items.len() - fixed_count;

                // Before star
                for i in 0..sp {
                    if let VMPatternElement::Pattern(ref pat) = elements[i] {
                        let sub = vm_match_pattern(pat, items[i], struct_reg, symbol_table)?;
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
                        let sub = vm_match_pattern(pat, items[item_idx], struct_reg, symbol_table)?;
                        bindings.extend(sub);
                    }
                }
                Some(bindings)
            } else {
                // No star: exact length match
                if items.len() != elements.len() {
                    return None;
                }
                let mut bindings = Vec::new();
                for (elem, item) in elements.iter().zip(items.iter()) {
                    if let VMPatternElement::Pattern(ref pat) = elem {
                        let sub = vm_match_pattern(pat, *item, struct_reg, symbol_table)?;
                        bindings.extend(sub);
                    }
                }
                Some(bindings)
            }
        }
        VMPattern::Struct { name, field_slots } => {
            let inst_idx = value.as_struct_instance_idx()?;
            let inst = struct_reg.get_instance(inst_idx)?;
            let ty = struct_reg.get_type(inst.type_id)?;
            if ty.name != *name {
                return None;
            }
            let mut bindings = Vec::with_capacity(field_slots.len());
            for (field_name, slot) in field_slots {
                let field_idx = ty.field_index(field_name)?;
                let val = inst.fields[field_idx];
                val.clone_refcount();
                bindings.push((*slot, val));
            }
            Some(bindings)
        }
        VMPattern::Enum {
            enum_name,
            variant_name,
        } => {
            // Resolve the expected symbol by looking up "EnumName.variant" in the symbol table
            let qname = qualified_name(enum_name, variant_name);
            let expected_sym = symbol_table.lookup(&qname)?;
            let expected = Value::from_symbol(expected_sym);
            if value.to_raw() == expected.to_raw() {
                Some(Vec::new())
            } else {
                None
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
) -> VMResult<Vec<(usize, Value)>> {
    match vm_match_pattern(pattern, value, struct_reg, symbol_table) {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::code_object::CodeObject;
    use crate::host::PureHost;
    use catnip_core::vm::opcode::Instruction;

    fn make_code(instructions: Vec<Instruction>, constants: Vec<Value>) -> Arc<CodeObject> {
        Arc::new(CodeObject {
            instructions,
            constants,
            names: vec![],
            nlocals: 0,
            varnames: vec![],
            slotmap: Default::default(),
            nargs: 0,
            defaults: vec![],
            name: "test".into(),
            freevars: vec![],
            vararg_idx: -1,
            is_pure: false,
            complexity: 0,
            line_table: vec![],
            patterns: vec![],
            encoded_ir: None,
        })
    }

    fn make_code_with_locals(
        instructions: Vec<Instruction>,
        constants: Vec<Value>,
        nlocals: usize,
        varnames: Vec<String>,
    ) -> Arc<CodeObject> {
        let slotmap = varnames.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
        Arc::new(CodeObject {
            instructions,
            constants,
            names: vec![],
            nlocals,
            varnames,
            slotmap,
            nargs: 0,
            defaults: vec![],
            name: "test".into(),
            freevars: vec![],
            vararg_idx: -1,
            is_pure: false,
            complexity: 0,
            line_table: vec![],
            patterns: vec![],
            encoded_ir: None,
        })
    }

    #[test]
    fn test_load_const_halt() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(42)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_add_two_ints() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::new(VMOpCode::Add, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(2), Value::from_int(3)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(5));
    }

    #[test]
    fn test_arithmetic_expression() {
        // 2 + 3 * 4 = 14
        // Bytecode: LoadConst(3), LoadConst(4), Mul, LoadConst(2), Add (wrong)
        // Actually: LoadConst(2), LoadConst(3), LoadConst(4), Mul, Add
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0), // 2
                Instruction::new(VMOpCode::LoadConst, 1), // 3
                Instruction::new(VMOpCode::LoadConst, 2), // 4
                Instruction::new(VMOpCode::Mul, 0),       // 3*4=12
                Instruction::new(VMOpCode::Add, 0),       // 2+12=14
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(2), Value::from_int(3), Value::from_int(4)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(14));
    }

    #[test]
    fn test_comparison() {
        // 3 < 5 -> true
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::new(VMOpCode::Lt, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(3), Value::from_int(5)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_jump_if_false() {
        // if false: push 1 else push 2
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),   // false
                Instruction::new(VMOpCode::JumpIfFalse, 4), // jump to 4
                Instruction::new(VMOpCode::LoadConst, 1),   // 1 (skipped)
                Instruction::new(VMOpCode::Jump, 5),        // jump to halt
                Instruction::new(VMOpCode::LoadConst, 2),   // 2 (target)
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::FALSE, Value::from_int(1), Value::from_int(2)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(2));
    }

    #[test]
    fn test_locals() {
        // x = 10; x + 5
        let code = make_code_with_locals(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),  // 10
                Instruction::new(VMOpCode::StoreLocal, 0), // x = 10
                Instruction::new(VMOpCode::LoadLocal, 0),  // x
                Instruction::new(VMOpCode::LoadConst, 1),  // 5
                Instruction::new(VMOpCode::Add, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(10), Value::from_int(5)],
            1,
            vec!["x".into()],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(15));
    }

    #[test]
    fn test_not_and_tobool() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0), // 0
                Instruction::simple(VMOpCode::Not),       // !0 = true
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(0)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_dup_top() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0), // 42
                Instruction::simple(VMOpCode::DupTop),    // 42, 42
                Instruction::new(VMOpCode::Add, 0),       // 84
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(42)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(84));
    }

    #[test]
    fn test_eq_ne() {
        // 5 == 5 -> true
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::Eq, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(5)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_build_list() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::new(VMOpCode::LoadConst, 2),
                Instruction::new(VMOpCode::BuildList, 3),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert!(result.is_native_list());
        let list = unsafe { result.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        assert_eq!(list.get(0).unwrap(), Value::from_int(1));
        assert_eq!(list.get(2).unwrap(), Value::from_int(3));
        result.decref();
    }

    #[test]
    fn test_build_tuple() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::new(VMOpCode::BuildTuple, 2),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(10), Value::from_int(20)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert!(result.is_native_tuple());
        let tuple = unsafe { result.as_native_tuple_ref().unwrap() };
        assert_eq!(tuple.len(), 2);
        result.decref();
    }

    #[test]
    fn test_negation() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::simple(VMOpCode::Neg),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(42)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(-42));
    }

    #[test]
    fn test_bitwise_and() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::new(VMOpCode::BAnd, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(0b1100), Value::from_int(0b1010)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(0b1000));
    }

    #[test]
    fn test_return_from_implicit_end() {
        // Code that just falls off the end returns last stack value
        let code = make_code(
            vec![Instruction::new(VMOpCode::LoadConst, 0)],
            vec![Value::from_int(99)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(99));
    }

    #[test]
    fn test_vm_match_pattern_literal() {
        let reg = PureStructRegistry::new();
        let pat = VMPattern::Literal(Value::from_int(42));
        assert!(vm_match_pattern(&pat, Value::from_int(42), &reg, &SymbolTable::new()).is_some());
        assert!(vm_match_pattern(&pat, Value::from_int(99), &reg, &SymbolTable::new()).is_none());
    }

    #[test]
    fn test_vm_match_pattern_var() {
        let reg = PureStructRegistry::new();
        let pat = VMPattern::Var(0);
        let result = vm_match_pattern(&pat, Value::from_int(42), &reg, &SymbolTable::new());
        assert!(result.is_some());
        let bindings = result.unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0], (0, Value::from_int(42)));
    }

    #[test]
    fn test_vm_match_pattern_wildcard() {
        let reg = PureStructRegistry::new();
        let pat = VMPattern::Wildcard;
        assert!(vm_match_pattern(&pat, Value::from_int(42), &reg, &SymbolTable::new()).is_some());
        assert!(vm_match_pattern(&pat, Value::NIL, &reg, &SymbolTable::new()).is_some());
    }

    #[test]
    fn test_vm_match_pattern_or() {
        let reg = PureStructRegistry::new();
        let pat = VMPattern::Or(vec![
            VMPattern::Literal(Value::from_int(1)),
            VMPattern::Literal(Value::from_int(2)),
        ]);
        assert!(vm_match_pattern(&pat, Value::from_int(1), &reg, &SymbolTable::new()).is_some());
        assert!(vm_match_pattern(&pat, Value::from_int(2), &reg, &SymbolTable::new()).is_some());
        assert!(vm_match_pattern(&pat, Value::from_int(3), &reg, &SymbolTable::new()).is_none());
    }

    #[test]
    fn test_format_value() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),   // 42
                Instruction::new(VMOpCode::FormatValue, 0), // no conv, no spec
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(42)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert!(result.is_native_str());
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("42"));
        result.decref();
    }

    #[test]
    fn test_build_string() {
        let s1 = Value::from_str("hello ");
        let s2 = Value::from_str("world");
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::new(VMOpCode::BuildString, 2),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![s1, s2],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("hello world"));
        s1.decref();
        s2.decref();
        result.decref();
    }

    #[test]
    fn test_function_call() {
        // Create a function that returns its argument + 1
        // func(x) => x + 1
        let func_code = Arc::new(CodeObject {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0), // x
                Instruction::new(VMOpCode::LoadConst, 0), // 1
                Instruction::new(VMOpCode::Add, 0),
                Instruction::simple(VMOpCode::Return),
            ],
            constants: vec![Value::from_int(1)],
            names: vec![],
            nlocals: 1,
            varnames: vec!["x".into()],
            slotmap: [("x".into(), 0)].into(),
            nargs: 1,
            defaults: vec![],
            name: "inc".into(),
            freevars: vec![],
            vararg_idx: -1,
            is_pure: false,
            complexity: 4,
            line_table: vec![],
            patterns: vec![],
            encoded_ir: None,
        });

        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();

        // Register the function
        let func_idx = vm.register_function(func_code);

        // Main code: call func(10)
        let main_code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0), // func ref
                Instruction::new(VMOpCode::LoadConst, 1), // arg: 10
                Instruction::new(VMOpCode::Call, 1),      // call(1 arg)
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_vmfunc(func_idx), Value::from_int(10)],
        );

        let result = vm.execute(main_code, &[], &host).unwrap();
        assert_eq!(result.as_int(), Some(11));
    }

    #[test]
    fn test_for_range_loop() {
        // sum = 0; for i in range(5): sum = sum + i
        // Bytecode:
        //   0: LoadConst 0 (=0)     -> sum initial
        //   1: StoreLocal 0         -> sum = 0
        //   2: LoadConst 0 (=0)     -> i initial
        //   3: StoreLocal 1         -> i = 0
        //   4: LoadConst 1 (=5)     -> stop
        //   5: StoreLocal 2         -> stop = 5
        //   6: ForRangeInt(slot_i=1, slot_stop=2, step_pos=true, jump=5)
        //   7: LoadLocal 0          -> sum
        //   8: LoadLocal 1          -> i
        //   9: Add
        //  10: StoreLocal 0         -> sum = sum + i
        //  11: ForRangeStep(slot_i=1, step=1, jump_target=6)
        //  12: LoadLocal 0          -> result
        //  13: Halt

        // ForRangeInt arg: (1 << 24) | (2 << 16) | (0 << 15) | 5
        let fri_arg = (1u32 << 24) | (2u32 << 16) | (0u32 << 15) | 5;
        // ForRangeStep arg: (1 << 24) | (1 << 16) | 6
        let frs_arg = (1u32 << 24) | (1u32 << 16) | 6;

        let code = make_code_with_locals(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),          // 0
                Instruction::new(VMOpCode::StoreLocal, 0),         // sum = 0
                Instruction::new(VMOpCode::LoadConst, 0),          // 0
                Instruction::new(VMOpCode::StoreLocal, 1),         // i = 0
                Instruction::new(VMOpCode::LoadConst, 1),          // 5
                Instruction::new(VMOpCode::StoreLocal, 2),         // stop = 5
                Instruction::new(VMOpCode::ForRangeInt, fri_arg),  // [6]
                Instruction::new(VMOpCode::LoadLocal, 0),          // [7] sum
                Instruction::new(VMOpCode::LoadLocal, 1),          // [8] i
                Instruction::new(VMOpCode::Add, 0),                // [9]
                Instruction::new(VMOpCode::StoreLocal, 0),         // [10] sum = sum + i
                Instruction::new(VMOpCode::ForRangeStep, frs_arg), // [11] i++, jump 6
                Instruction::new(VMOpCode::LoadLocal, 0),          // [12] sum
                Instruction::simple(VMOpCode::Halt),               // [13]
            ],
            vec![Value::from_int(0), Value::from_int(5)],
            3,
            vec!["sum".into(), "i".into(), "stop".into()],
        );

        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        // sum = 0+1+2+3+4 = 10
        assert_eq!(result.as_int(), Some(10));
    }

    #[test]
    fn test_typeof() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::simple(VMOpCode::TypeOf),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(42)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(unsafe { result.as_native_str_ref() }, Some("int"));
        result.decref();
    }

    #[test]
    fn test_in_operator() {
        // 2 in [1, 2, 3] -> true
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0), // 2
                Instruction::new(VMOpCode::LoadConst, 1), // 1
                Instruction::new(VMOpCode::LoadConst, 0), // 2
                Instruction::new(VMOpCode::LoadConst, 2), // 3
                Instruction::new(VMOpCode::BuildList, 3), // [1, 2, 3]
                Instruction::new(VMOpCode::In, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(2), Value::from_int(1), Value::from_int(3)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_is_operator() {
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0), // None
                Instruction::new(VMOpCode::LoadConst, 0), // None
                Instruction::new(VMOpCode::Is, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::NIL],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_div_and_floordiv() {
        // 7 / 2 = 3.5
        let code = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::new(VMOpCode::Div, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(7), Value::from_int(2)],
        );
        let mut vm = PureVM::new();
        let host = PureHost::with_builtins();
        let result = vm.execute(code, &[], &host).unwrap();
        assert!((result.as_float().unwrap() - 3.5).abs() < 1e-10);

        // -7 // 2 = -4
        let code2 = make_code(
            vec![
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::LoadConst, 1),
                Instruction::new(VMOpCode::FloorDiv, 0),
                Instruction::simple(VMOpCode::Halt),
            ],
            vec![Value::from_int(-7), Value::from_int(2)],
        );
        let mut vm2 = PureVM::new();
        let result2 = vm2.execute(code2, &[], &host).unwrap();
        assert_eq!(result2.as_int(), Some(-4));
    }

    // =================================================================
    // End-to-end: PureCompiler -> PureVM
    // =================================================================

    mod e2e {
        use super::*;
        use crate::compiler::PureCompiler;
        use catnip_core::ir::{IR, IROpCode};

        fn run(ir: IR) -> Value {
            let mut compiler = PureCompiler::new();
            let program = IR::Program(vec![ir]);
            let output = compiler.compile(&program).unwrap();
            let mut vm = PureVM::new();
            let host = PureHost::with_builtins();
            vm.execute_output(&output, &[], &host).unwrap()
        }

        #[test]
        fn test_e2e_arithmetic() {
            // 2 + 3 * 4 = 14
            let expr = IR::op(
                IROpCode::Add,
                vec![IR::Int(2), IR::op(IROpCode::Mul, vec![IR::Int(3), IR::Int(4)])],
            );
            assert_eq!(run(expr).as_int(), Some(14));
        }

        #[test]
        fn test_e2e_comparison() {
            let expr = IR::op(IROpCode::Lt, vec![IR::Int(3), IR::Int(5)]);
            assert_eq!(run(expr).as_bool(), Some(true));
        }

        #[test]
        fn test_e2e_negation() {
            let expr = IR::op(IROpCode::Neg, vec![IR::Int(42)]);
            assert_eq!(run(expr).as_int(), Some(-42));
        }

        #[test]
        fn test_e2e_lambda_call() {
            // ((n) => n * 2)(21)
            let params = IR::List(vec![IR::Identifier("n".into())]);
            let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
            let lambda = IR::op(IROpCode::OpLambda, vec![params, body]);
            let call = IR::op(IROpCode::Call, vec![lambda, IR::Int(21)]);
            assert_eq!(run(call).as_int(), Some(42));
        }

        #[test]
        fn test_e2e_fn_def_and_call() {
            // Use PurePipeline for proper semantic analysis
            let mut p = crate::pipeline::PurePipeline::new().unwrap();
            let result = p.execute("double = (n) => { n * 2 }; double(21)").unwrap();
            assert_eq!(result.as_int(), Some(42));
        }

        #[test]
        fn test_e2e_list_operations() {
            let expr = IR::op(IROpCode::ListLiteral, vec![IR::Int(1), IR::Int(2), IR::Int(3)]);
            let result = run(expr);
            assert!(result.is_native_list());
            let list = unsafe { result.as_native_list_ref().unwrap() };
            assert_eq!(list.len(), 3);
            result.decref();
        }

        #[test]
        fn test_e2e_string_literal() {
            let result = run(IR::String("hello".into()));
            assert!(result.is_native_str());
            assert_eq!(unsafe { result.as_native_str_ref() }, Some("hello"));
            result.decref();
        }

        #[test]
        fn test_e2e_bool_not() {
            let expr = IR::op(IROpCode::Not, vec![IR::Bool(false)]);
            assert_eq!(run(expr).as_bool(), Some(true));
        }
    }
}
