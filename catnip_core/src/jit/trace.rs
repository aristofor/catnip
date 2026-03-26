// FILE: catnip_core/src/jit/trace.rs
//! Trace recording for JIT compilation.
//!
//! Records a sequence of operations during loop execution to build
//! a linear trace that can be compiled to native code.

use crate::vm::VMOpCode as OpCode;
use std::collections::{HashMap, HashSet};

/// Type of trace being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TraceType {
    /// Loop trace (records iterations)
    Loop,
    /// Function trace (records single execution from entry to return)
    Function,
}

/// A single operation in a trace.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum TraceOp {
    /// Load integer constant
    LoadConstInt(i64),
    /// Load float constant
    LoadConstFloat(f64),
    /// Load from local slot
    LoadLocal(usize),
    /// Store to local slot
    StoreLocal(usize),
    /// Store to name (scope dict) - optimized as local in hot loops
    StoreScope(usize),
    /// Duplicate top of stack
    DupTop,
    /// Pop top of stack
    PopTop,
    /// Integer add
    AddInt,
    /// Integer subtract
    SubInt,
    /// Integer multiply
    MulInt,
    /// Integer divide
    DivInt,
    /// Integer modulo
    ModInt,
    /// Integer less than
    LtInt,
    /// Integer less or equal
    LeInt,
    /// Integer greater than
    GtInt,
    /// Integer greater or equal
    GeInt,
    /// Integer equal
    EqInt,
    /// Integer not equal
    NeInt,
    /// Float add
    AddFloat,
    /// Float subtract
    SubFloat,
    /// Float multiply
    MulFloat,
    /// Float divide
    DivFloat,
    /// Float less than
    LtFloat,
    /// Float less or equal
    LeFloat,
    /// Float greater than
    GtFloat,
    /// Float greater or equal
    GeFloat,
    /// Float equal
    EqFloat,
    /// Float not equal
    NeFloat,
    /// Jump (unconditional)
    Jump(usize),
    /// Break: unconditional jump out of loop
    Break,
    /// Continue: jump to loop increment (rare, usually implicit in LoopBack)
    Continue,
    /// Conditional jump if top of stack is false
    JumpIfFalse(usize),
    /// Conditional jump if top of stack is true
    JumpIfTrue(usize),
    /// Guard: assert value is integer, bail if not
    GuardInt(usize), // slot to check
    /// Guard: assert value is float, bail if not
    GuardFloat(usize), // slot to check
    /// Guard: assert condition is true, bail if false
    GuardTrue,
    /// Guard: assert condition is false, bail if true
    GuardFalse,
    /// Guard: assert LoadScope variable has expected value
    GuardNameValue(String, i64),
    /// Loop back to start
    LoopBack,
    /// Exit trace (normal completion)
    Exit,
    /// Fallback to interpreter (unsupported op)
    Fallback(OpCode),
    /// Recursive call to self (function being compiled)
    CallSelf { num_args: usize },
    /// Tail-recursive call to self (can be optimized to jump)
    TailCallSelf { num_args: usize },
    /// Call to pure function (candidate for inline)
    CallPure { func_id: String, num_args: usize },
    /// Builtin abs(int) - unary
    AbsInt,
    /// Builtin min(int, int) - binary
    MinInt,
    /// Builtin max(int, int) - binary
    MaxInt,
    /// Builtin round(int) - identity (int already rounded)
    RoundInt,
    /// Builtin int(int) - identity
    IntCastInt,
    /// Builtin bool(int) - x != 0
    BoolInt,
    /// Builtin via extern C callback (float, etc.)
    CallBuiltinPure { builtin_id: u8, num_args: u8 },
}

/// A recorded trace ready for compilation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Trace {
    /// Type of trace
    pub trace_type: TraceType,
    /// Loop header offset in bytecode (for Loop traces)
    pub loop_offset: usize,
    /// Function ID (for Function traces)
    pub func_id: Option<String>,
    /// Number of function parameters (for Function traces)
    pub num_params: usize,
    /// Recorded operations
    pub ops: Vec<TraceOp>,
    /// Bytecode offsets for each recorded operation
    pub op_offsets: Vec<usize>,
    /// Local slots used (for register allocation)
    pub locals_used: Vec<usize>,
    /// Whether trace contains only integer operations
    pub is_int_only: bool,
    /// Number of iterations recorded (for Loop traces)
    pub iterations: u32,
    /// Guards for LoadScope variables: (name, expected_value, slot)
    pub name_guards: Vec<(String, i64, usize)>,
}

impl Trace {
    /// Create a new loop trace.
    pub fn new(loop_offset: usize) -> Self {
        Self {
            trace_type: TraceType::Loop,
            loop_offset,
            func_id: None,
            num_params: 0,
            ops: Vec::new(),
            op_offsets: Vec::new(),
            locals_used: Vec::new(),
            is_int_only: true,
            iterations: 0,
            name_guards: Vec::new(),
        }
    }

    /// Create a new function trace.
    pub fn new_function(func_id: String, num_params: usize) -> Self {
        Self {
            trace_type: TraceType::Function,
            loop_offset: 0, // Not used for functions
            func_id: Some(func_id),
            num_params,
            ops: Vec::new(),
            op_offsets: Vec::new(),
            locals_used: Vec::new(),
            is_int_only: true,
            iterations: 1, // Functions = single execution
            name_guards: Vec::new(),
        }
    }

    /// Check if trace is suitable for compilation.
    pub fn is_compilable(&self) -> bool {
        // Need at least one iteration
        if self.iterations == 0 {
            return false;
        }

        // Phase 2 ✅: CallSelf now supported via external storage + side-exit
        // Compiled code stores recursive args in thread-local, VM creates new frame

        // Float traces are now supported with proper type handling
        // Check we don't have too many fallbacks
        let fallback_count = self.ops.iter().filter(|op| matches!(op, TraceOp::Fallback(_))).count();
        fallback_count == 0
    }

    /// Phase 4.1: Optimize CallSelf to TailCallSelf when in tail position.
    /// A CallSelf is in tail position if it's immediately followed by Exit/Return.
    pub fn optimize_tail_calls(&mut self) {
        for i in 0..self.ops.len() {
            if let TraceOp::CallSelf { num_args } = self.ops[i] {
                // Check if next operation is Exit (return)
                // Skip intermediate PopTop (result handling)
                let mut next_idx = i + 1;
                while next_idx < self.ops.len() {
                    match &self.ops[next_idx] {
                        TraceOp::Exit => {
                            // This CallSelf is in tail position!
                            self.ops[i] = TraceOp::TailCallSelf { num_args };
                            break;
                        }
                        // Skip stack cleanup operations
                        TraceOp::PopTop | TraceOp::DupTop => {
                            next_idx += 1;
                        }
                        // Any other operation means not tail position
                        _ => break,
                    }
                }
            }
        }
    }
}

/// Records operations during trace execution.
pub struct TraceRecorder {
    /// Current trace being recorded
    trace: Option<Trace>,
    /// Whether we're currently recording
    recording: bool,
    /// Maximum ops before aborting trace
    max_ops: usize,
    /// Slots seen with integer values
    int_slots: Vec<bool>,
    /// Loop start bytecode offset (for Jump classification)
    loop_start_offset: Option<usize>,
    /// Loop end bytecode offset (for Jump classification)
    loop_end_offset: Option<usize>,
    /// Map LoadScope variables to allocated slots
    name_to_slot: HashMap<String, usize>,
    /// Next available slot for LoadScope captures (starts after regular locals)
    next_name_slot: usize,
    /// Set of variable names that are modified (StoreScope) in the loop
    modified_names: HashSet<String>,
}

impl TraceRecorder {
    pub fn new() -> Self {
        Self {
            trace: None,
            recording: false,
            max_ops: 10000,
            int_slots: Vec::new(),
            loop_start_offset: None,
            loop_end_offset: None,
            name_to_slot: HashMap::new(),
            next_name_slot: 0,
            modified_names: HashSet::new(),
        }
    }

    /// Start recording a trace for a loop.
    pub fn start(&mut self, loop_offset: usize, num_locals: usize) {
        self.trace = Some(Trace::new(loop_offset));
        self.recording = true;
        self.int_slots = vec![false; num_locals];
        self.name_to_slot.clear();
        self.modified_names.clear();
        // Name slots start after regular locals
        self.next_name_slot = num_locals;
    }

    /// Start recording a trace for a function.
    pub fn start_function(&mut self, func_id: String, num_locals: usize, num_params: usize) {
        self.trace = Some(Trace::new_function(func_id, num_params));
        self.recording = true;
        self.int_slots = vec![false; num_locals];
        self.name_to_slot.clear();
        self.modified_names.clear();
        // Name slots start after regular locals
        self.next_name_slot = num_locals;
        // No loop bounds for function traces
        self.loop_start_offset = None;
        self.loop_end_offset = None;
    }

    /// Stop recording and return the trace.
    pub fn stop(&mut self) -> Option<Trace> {
        self.recording = false;
        self.trace.take()
    }

    /// Abort recording without returning trace.
    pub fn abort(&mut self) {
        self.recording = false;
        self.trace = None;
    }

    /// Check if currently recording.
    #[inline]
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Set loop boundaries for Jump classification.
    /// Called by VM when starting trace recording for a loop.
    pub fn set_loop_bounds(&mut self, start: usize, end: usize) {
        self.loop_start_offset = Some(start);
        self.loop_end_offset = Some(end);
    }

    /// Get number of iterations recorded so far.
    #[inline]
    pub fn iterations(&self) -> u32 {
        self.trace.as_ref().map(|t| t.iterations).unwrap_or(0)
    }

    /// Record a VM opcode execution.
    #[inline]
    pub fn record_opcode(&mut self, op: OpCode, arg: u32, is_int_value: bool, ip: usize) -> bool {
        let trace = match &mut self.trace {
            Some(t) => t,
            None => return false,
        };

        // Check max ops limit
        if trace.ops.len() >= self.max_ops {
            self.abort();
            return false;
        }

        let trace_op = match op {
            OpCode::LoadConst => {
                // Will be specialized by caller
                return true;
            }
            OpCode::LoadLocal => {
                let slot = arg as usize;
                if !trace.locals_used.contains(&slot) {
                    trace.locals_used.push(slot);
                }
                // Insert type guard if first access
                if slot < self.int_slots.len() && !self.int_slots[slot] {
                    self.int_slots[slot] = true;
                    if is_int_value {
                        trace.ops.push(TraceOp::GuardInt(slot));
                    } else {
                        trace.is_int_only = false;
                        trace.ops.push(TraceOp::GuardFloat(slot));
                    }
                    trace.op_offsets.push(ip);
                }
                TraceOp::LoadLocal(slot)
            }
            OpCode::StoreLocal => {
                let slot = arg as usize;
                if !trace.locals_used.contains(&slot) {
                    trace.locals_used.push(slot);
                }
                TraceOp::StoreLocal(slot)
            }
            OpCode::Add => {
                if is_int_value {
                    TraceOp::AddInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::AddFloat
                }
            }
            OpCode::Sub => {
                if is_int_value {
                    TraceOp::SubInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::SubFloat
                }
            }
            OpCode::Mul => {
                if is_int_value {
                    TraceOp::MulInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::MulFloat
                }
            }
            OpCode::Div => {
                if is_int_value {
                    TraceOp::DivInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::DivFloat
                }
            }
            OpCode::Mod => TraceOp::ModInt, // Mod is int-only
            OpCode::Lt => {
                if is_int_value {
                    TraceOp::LtInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::LtFloat
                }
            }
            OpCode::Le => {
                if is_int_value {
                    TraceOp::LeInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::LeFloat
                }
            }
            OpCode::Gt => {
                if is_int_value {
                    TraceOp::GtInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::GtFloat
                }
            }
            OpCode::Ge => {
                if is_int_value {
                    TraceOp::GeInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::GeFloat
                }
            }
            OpCode::Eq => {
                if is_int_value {
                    TraceOp::EqInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::EqFloat
                }
            }
            OpCode::Ne => {
                if is_int_value {
                    TraceOp::NeInt
                } else {
                    trace.is_int_only = false;
                    TraceOp::NeFloat
                }
            }
            OpCode::Jump => {
                // Only classify Jumps for loops with known bounds (for range loops)
                // For other loops (while), use old behavior
                if let (Some(start), Some(_end)) = (self.loop_start_offset, self.loop_end_offset) {
                    let target = arg as usize;
                    if target <= start {
                        // Jump backward to loop start = implicit LoopBack
                        // This is handled explicitly via record_loop_back()
                        // Skip this Jump instruction (it's the back edge)
                        return true;
                    }
                    TraceOp::Jump(target)
                } else {
                    // No loop bounds set - use old behavior (while loops, etc.)
                    // Don't classify, just record the Jump
                    let target = arg as usize;
                    if target <= ip {
                        // Likely loop back edge, handled separately
                        return true;
                    }
                    TraceOp::Jump(target)
                }
            }
            OpCode::JumpIfFalse | OpCode::JumpIfTrue => {
                // Conditional jumps are handled specially via record_conditional_jump
                // Skip here - the VM will call record_conditional_jump after execution
                return true;
            }
            OpCode::ForRangeInt => {
                use crate::vm::{
                    FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_SLOT_MASK, FOR_RANGE_SLOT_STOP_SHIFT, FOR_RANGE_STEP_SIGN_SHIFT,
                };
                let slot_i = (arg >> FOR_RANGE_SLOT_I_SHIFT) as usize;
                let slot_stop = ((arg >> FOR_RANGE_SLOT_STOP_SHIFT) & FOR_RANGE_SLOT_MASK) as usize;
                let step_positive = ((arg >> FOR_RANGE_STEP_SIGN_SHIFT) & 1) == 0;

                // Load i and stop, compare (always int for range loops)
                if !trace.locals_used.contains(&slot_i) {
                    trace.locals_used.push(slot_i);
                    trace.ops.push(TraceOp::GuardInt(slot_i));
                    trace.op_offsets.push(ip);
                }
                if !trace.locals_used.contains(&slot_stop) {
                    trace.locals_used.push(slot_stop);
                    trace.ops.push(TraceOp::GuardInt(slot_stop));
                    trace.op_offsets.push(ip);
                }

                trace.ops.push(TraceOp::LoadLocal(slot_i));
                trace.op_offsets.push(ip);
                trace.ops.push(TraceOp::LoadLocal(slot_stop));
                trace.op_offsets.push(ip);
                if step_positive {
                    // Continue while i < stop
                    trace.ops.push(TraceOp::LtInt);
                    trace.op_offsets.push(ip);
                } else {
                    // Continue while i > stop
                    trace.ops.push(TraceOp::GtInt);
                    trace.op_offsets.push(ip);
                }
                // Guard: continue if condition true, exit otherwise
                trace.ops.push(TraceOp::GuardTrue);
                trace.op_offsets.push(ip);
                // GuardTrue peeks (doesn't consume), so we need to pop the comparison result
                trace.ops.push(TraceOp::PopTop);
                trace.op_offsets.push(ip);
                return true;
            }
            // ForRangeStep: decompose into existing TraceOps for JIT compatibility
            OpCode::ForRangeStep => {
                use crate::vm::{FOR_RANGE_SLOT_I_SHIFT, FOR_RANGE_STEP_BYTE_MASK, FOR_RANGE_STEP_SHIFT};
                let slot_i = (arg >> FOR_RANGE_SLOT_I_SHIFT) as usize;
                let step = ((arg >> FOR_RANGE_STEP_SHIFT) & FOR_RANGE_STEP_BYTE_MASK) as i8 as i64;
                if !trace.locals_used.contains(&slot_i) {
                    trace.locals_used.push(slot_i);
                }
                trace.ops.push(TraceOp::LoadLocal(slot_i));
                trace.op_offsets.push(ip);
                trace.ops.push(TraceOp::LoadConstInt(step));
                trace.op_offsets.push(ip);
                trace.ops.push(TraceOp::AddInt);
                trace.op_offsets.push(ip);
                trace.ops.push(TraceOp::StoreLocal(slot_i));
                trace.op_offsets.push(ip);
                trace.ops.push(TraceOp::LoopBack);
                trace.op_offsets.push(ip);
                trace.iterations += 1;
                return true;
            }
            // Ops that force interpreter fallback
            OpCode::CallKw
            | OpCode::TailCall
            | OpCode::GetAttr
            | OpCode::SetAttr
            | OpCode::GetItem
            | OpCode::SetItem
            | OpCode::BuildList
            | OpCode::BuildDict
            | OpCode::BuildTuple
            | OpCode::GetIter
            | OpCode::ForIter
            | OpCode::Broadcast => {
                trace.is_int_only = false;
                TraceOp::Fallback(op)
            }
            // Call is handled separately via record_call() in VM
            OpCode::Call => {
                return true; // Skip recording here, VM will call record_call()
            }
            // Stack ops - record DupTop/PopTop for stack balance
            OpCode::DupTop => TraceOp::DupTop,
            OpCode::PopTop => TraceOp::PopTop,
            // Other stack ops can be skipped
            OpCode::RotTwo | OpCode::PushBlock | OpCode::PopBlock | OpCode::Nop => {
                return true; // Skip, handled implicitly
            }
            // Control flow that aborts trace
            OpCode::Return | OpCode::Break | OpCode::Continue | OpCode::Halt => {
                trace.ops.push(TraceOp::Exit);
                trace.op_offsets.push(ip);
                return true;
            }
            // Name operations
            OpCode::StoreScope => {
                // StoreScope is handled via record_store_scope() called from VM
                // Skip recording here - the VM will call record_store_scope
                return true;
            }
            OpCode::LoadScope => {
                // LoadScope is handled via record_load_scope() called from VM after resolution
                // Skip recording here - the VM will call record_load_scope with the actual value
                return true;
            }
            OpCode::LoadGlobal => {
                // LoadGlobal handled in VM (builtin detection + record_fallback/record_builtin)
                return true;
            }
            _ => {
                // Unknown op - fallback, log for debugging
                #[cfg(debug_assertions)]
                eprintln!("[JIT] Unknown opcode in trace: {:?}", op);
                TraceOp::Fallback(op)
            }
        };

        trace.ops.push(trace_op);
        trace.op_offsets.push(ip);
        true
    }

    /// Record an integer constant load.
    pub fn record_const_int(&mut self, value: i64, ip: usize) {
        if let Some(trace) = &mut self.trace {
            trace.ops.push(TraceOp::LoadConstInt(value));
            trace.op_offsets.push(ip);
        }
    }

    /// Record a float constant load.
    pub fn record_const_float(&mut self, value: f64, ip: usize) {
        if let Some(trace) = &mut self.trace {
            trace.ops.push(TraceOp::LoadConstFloat(value));
            trace.op_offsets.push(ip);
            trace.is_int_only = false;
        }
    }

    /// Record a LoadScope operation.
    /// Transforms LoadScope into LoadLocal.
    /// Only creates guards for read-only variables (not modified in the loop).
    pub fn record_load_scope(&mut self, name: &str, value: i64, ip: usize) -> bool {
        let trace = match &mut self.trace {
            Some(t) => t,
            None => return false,
        };

        // Check if this is a read-only variable (not modified in loop)
        let is_read_only = !self.modified_names.contains(name);

        // Allocate or reuse slot for this name
        let slot = if let Some(&existing_slot) = self.name_to_slot.get(name) {
            existing_slot
        } else {
            let new_slot = self.next_name_slot;
            self.next_name_slot += 1;
            self.name_to_slot.insert(name.to_string(), new_slot);

            // Add to locals_used
            if !trace.locals_used.contains(&new_slot) {
                trace.locals_used.push(new_slot);
            }

            // Only create guard for read-only variables (constants, external vars)
            // Variables modified in the loop don't need guards
            if is_read_only {
                trace.name_guards.push((name.to_string(), value, new_slot));
            }

            new_slot
        };

        // Record LoadLocal operation
        trace.ops.push(TraceOp::LoadLocal(slot));
        trace.op_offsets.push(ip);

        // Assume LoadScope values might not be ints
        trace.is_int_only = false;

        true
    }

    /// Record a StoreScope operation.
    /// Transforms StoreScope into StoreLocal.
    /// `existing_slot`: If the variable already has a slot in the slotmap, use it instead of allocating a new one.
    /// Returns Some(slot) if recording succeeded, None otherwise.
    pub fn record_store_scope(&mut self, name: &str, ip: usize, existing_slot: Option<usize>) -> Option<usize> {
        let trace = match &mut self.trace {
            Some(t) => t,
            None => return None,
        };

        // Mark this name as modified (written to) in the loop
        self.modified_names.insert(name.to_string());

        // Use existing slot from slotmap if provided, otherwise allocate/reuse slot
        let slot = if let Some(slot_from_map) = existing_slot {
            // Use the slot from the bytecode slotmap (for function local variables)
            if !self.name_to_slot.contains_key(name) {
                self.name_to_slot.insert(name.to_string(), slot_from_map);
            }
            // Add to locals_used if not already there
            if !trace.locals_used.contains(&slot_from_map) {
                trace.locals_used.push(slot_from_map);
            }
            slot_from_map
        } else if let Some(&existing_slot) = self.name_to_slot.get(name) {
            existing_slot
        } else {
            // New name - allocate slot
            let new_slot = self.next_name_slot;
            self.next_name_slot += 1;
            self.name_to_slot.insert(name.to_string(), new_slot);

            // Add to locals_used
            if !trace.locals_used.contains(&new_slot) {
                trace.locals_used.push(new_slot);
            }

            new_slot
        };

        // Record StoreLocal operation
        trace.ops.push(TraceOp::StoreLocal(slot));
        trace.op_offsets.push(ip);

        Some(slot)
    }

    /// Record a function call operation.
    /// Detects self-calls and records them as CallSelf.
    /// Pure functions are recorded as CallPure (candidates for inlining).
    /// Other calls are recorded as Fallback.
    /// Returns true if recording succeeded (call was recorded), false otherwise.
    pub fn record_call(&mut self, called_func_id: &str, num_args: usize, is_pure: bool, ip: usize) -> bool {
        let trace = match &mut self.trace {
            Some(t) => t,
            None => return false,
        };

        // Check if this is a self-call (recursive call to the function being compiled)
        let is_self_call = if let Some(ref current_func_id) = trace.func_id {
            current_func_id == called_func_id
        } else {
            false
        };

        if is_self_call {
            // Record as CallSelf (takes priority over CallPure)
            trace.ops.push(TraceOp::CallSelf { num_args });
            trace.op_offsets.push(ip);
            // Recursive calls may involve non-int values
            trace.is_int_only = false;
        } else if is_pure {
            // Record as CallPure (candidate for inlining)
            trace.ops.push(TraceOp::CallPure {
                func_id: called_func_id.to_string(),
                num_args,
            });
            trace.op_offsets.push(ip);
            // Pure function calls may involve non-int values
            trace.is_int_only = false;
        } else {
            // Other calls fall back to interpreter
            trace.ops.push(TraceOp::Fallback(OpCode::Call));
            trace.op_offsets.push(ip);
            trace.is_int_only = false;
        }

        true
    }

    /// Record a builtin pure call as a native TraceOp.
    pub fn record_builtin(&mut self, op: TraceOp, ip: usize) {
        if let Some(ref mut trace) = self.trace {
            trace.ops.push(op);
            trace.op_offsets.push(ip);
        }
    }

    /// Record a fallback (non-compilable op).
    pub fn record_fallback(&mut self, op: OpCode, ip: usize) {
        if let Some(ref mut trace) = self.trace {
            trace.is_int_only = false;
            trace.ops.push(TraceOp::Fallback(op));
            trace.op_offsets.push(ip);
        }
    }

    /// Record a conditional jump result.
    /// Called AFTER the VM executes JumpIfFalse/JumpIfTrue.
    /// `took_jump`: true if the jump was taken, false if we fell through.
    /// `is_jump_if_false`: true for JumpIfFalse, false for JumpIfTrue.
    pub fn record_conditional_jump(&mut self, took_jump: bool, is_jump_if_false: bool, ip: usize) {
        if let Some(trace) = &mut self.trace {
            // Convert to a guard based on the path taken:
            // - JumpIfFalse, didn't jump (condition was truthy) -> GuardTrue
            // - JumpIfFalse, jumped (condition was falsy) -> GuardFalse
            // - JumpIfTrue, didn't jump (condition was falsy) -> GuardFalse
            // - JumpIfTrue, jumped (condition was truthy) -> GuardTrue
            let guard = if is_jump_if_false {
                if took_jump {
                    TraceOp::GuardFalse // We expect condition to be false
                } else {
                    TraceOp::GuardTrue // We expect condition to be true
                }
            } else {
                // JumpIfTrue
                if took_jump {
                    TraceOp::GuardTrue // We expect condition to be true
                } else {
                    TraceOp::GuardFalse // We expect condition to be false
                }
            };
            trace.ops.push(guard);
            trace.op_offsets.push(ip);
            // Guards peek the stack - need to pop the condition value
            // JumpIfFalse/JumpIfTrue pop the condition in the VM
            trace.ops.push(TraceOp::PopTop);
            trace.op_offsets.push(ip);
        }
    }

    /// Record loop back edge (end of iteration).
    pub fn record_loop_back(&mut self, ip: usize) {
        if let Some(trace) = &mut self.trace {
            trace.ops.push(TraceOp::LoopBack);
            trace.op_offsets.push(ip);
            trace.iterations += 1;
        }
    }
}

impl Default for TraceRecorder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_recording() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);

        assert!(recorder.is_recording());

        recorder.record_const_int(0, 100);
        recorder.record_opcode(OpCode::StoreLocal, 0, true, 101);
        recorder.record_opcode(OpCode::LoadLocal, 0, true, 102);
        recorder.record_const_int(1, 103);
        recorder.record_opcode(OpCode::Add, 0, true, 102);
        recorder.record_opcode(OpCode::StoreLocal, 0, true, 103);
        recorder.record_loop_back(104);

        let trace = recorder.stop().unwrap();
        assert_eq!(trace.loop_offset, 100);
        assert_eq!(trace.iterations, 1);
        assert!(trace.is_int_only);
    }

    #[test]
    fn test_jump_classification_break() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);
        recorder.set_loop_bounds(100, 120); // Loop at offsets 100-120

        // Jump to 125 (>= loop_end) = break
        recorder.record_opcode(OpCode::Jump, 125, true, 110);

        let trace = recorder.stop().unwrap();

        // Should contain Break + Exit
        assert_eq!(trace.ops.len(), 1);
        assert!(matches!(trace.ops[0], TraceOp::Jump(125)));
    }

    #[test]
    fn test_jump_classification_loop_back() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);
        recorder.set_loop_bounds(100, 120);

        // Jump to 100 (<= loop_start) = loop back (skipped, handled by record_loop_back)
        recorder.record_opcode(OpCode::Jump, 100, true, 115);

        let trace = recorder.stop().unwrap();

        // Jump backwards should be skipped (returns true without adding to trace)
        assert_eq!(trace.ops.len(), 0);
    }

    #[test]
    fn test_jump_classification_within_loop() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);
        recorder.set_loop_bounds(100, 120);

        // Jump to 110 (inside loop) = conditional jump (fallback)
        recorder.record_opcode(OpCode::Jump, 110, true, 112);

        let trace = recorder.stop().unwrap();

        // Jump within loop should be recorded as Jump
        assert_eq!(trace.ops.len(), 1);
        assert!(matches!(trace.ops[0], TraceOp::Jump(110)));
    }

    #[test]
    fn test_set_loop_bounds() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);

        // Initially no bounds
        assert!(recorder.loop_start_offset.is_none());
        assert!(recorder.loop_end_offset.is_none());

        // Set bounds
        recorder.set_loop_bounds(100, 150);

        assert_eq!(recorder.loop_start_offset, Some(100));
        assert_eq!(recorder.loop_end_offset, Some(150));
    }

    #[test]
    fn test_jumpiffalse_loop_exit() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);
        recorder.set_loop_bounds(100, 120);

        // JumpIfFalse to 125 (>= loop_end) = loop exit guard
        // Must call record_conditional_jump AFTER record_opcode
        recorder.record_opcode(OpCode::JumpIfFalse, 125, true, 111);
        recorder.record_conditional_jump(true, true, 111); // took_jump=true for exit condition

        let trace = recorder.stop().unwrap();

        // Should record a guard and pop
        assert_eq!(trace.ops.len(), 2);
        assert!(matches!(trace.ops[0], TraceOp::GuardFalse)); // We expect condition to be false for jump
        assert!(matches!(trace.ops[1], TraceOp::PopTop));
    }

    #[test]
    fn test_jumpiffalse_within_loop() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);
        recorder.set_loop_bounds(100, 120);

        // JumpIfFalse to 110 (inside loop) = conditional within body (fallback)
        // Must call record_conditional_jump AFTER record_opcode
        recorder.record_opcode(OpCode::JumpIfFalse, 110, true, 113);
        recorder.record_conditional_jump(false, true, 113); // took_jump=false for fallthrough

        let trace = recorder.stop().unwrap();

        // Should record a guard and pop
        assert_eq!(trace.ops.len(), 2);
        assert!(matches!(trace.ops[0], TraceOp::GuardTrue)); // We expect condition to be true for fallthrough
        assert!(matches!(trace.ops[1], TraceOp::PopTop));
    }

    #[test]
    fn test_record_builtin() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);

        recorder.record_builtin(TraceOp::AbsInt, 200);
        recorder.record_builtin(TraceOp::MinInt, 201);
        recorder.record_builtin(TraceOp::MaxInt, 202);

        let trace = recorder.stop().unwrap();
        assert_eq!(trace.ops.len(), 3);
        assert!(matches!(trace.ops[0], TraceOp::AbsInt));
        assert!(matches!(trace.ops[1], TraceOp::MinInt));
        assert!(matches!(trace.ops[2], TraceOp::MaxInt));
        // Builtins are int-only, is_int_only should stay true
        assert!(trace.is_int_only);
    }

    #[test]
    fn test_record_new_builtins() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);

        recorder.record_builtin(TraceOp::RoundInt, 200);
        recorder.record_builtin(TraceOp::IntCastInt, 201);
        recorder.record_builtin(TraceOp::BoolInt, 202);

        let trace = recorder.stop().unwrap();
        assert_eq!(trace.ops.len(), 3);
        assert!(matches!(trace.ops[0], TraceOp::RoundInt));
        assert!(matches!(trace.ops[1], TraceOp::IntCastInt));
        assert!(matches!(trace.ops[2], TraceOp::BoolInt));
    }

    #[test]
    fn test_call_builtin_pure_compilable() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);

        recorder.record_builtin(
            TraceOp::CallBuiltinPure {
                builtin_id: 0,
                num_args: 1,
            },
            200,
        );
        recorder.record_loop_back(201);

        let trace = recorder.stop().unwrap();
        // CallBuiltinPure is NOT a Fallback, trace should be compilable
        assert!(trace.is_compilable());
    }

    #[test]
    fn test_record_fallback() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);

        recorder.record_fallback(OpCode::LoadGlobal, 200);

        let trace = recorder.stop().unwrap();
        assert_eq!(trace.ops.len(), 1);
        assert!(matches!(trace.ops[0], TraceOp::Fallback(OpCode::LoadGlobal)));
        assert!(!trace.is_int_only);
        assert!(!trace.is_compilable());
    }

    #[test]
    fn test_loadglobal_skipped_in_record_opcode() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);

        // LoadGlobal should be skipped (returns true without adding to trace)
        let result = recorder.record_opcode(OpCode::LoadGlobal, 0, true, 200);
        assert!(result);

        let trace = recorder.stop().unwrap();
        assert_eq!(trace.ops.len(), 0);
    }

    #[test]
    fn test_builtin_trace_compilable() {
        let mut recorder = TraceRecorder::new();
        recorder.start(100, 4);

        // Trace with builtin ops should be compilable (no fallbacks)
        recorder.record_const_int(42, 100);
        recorder.record_builtin(TraceOp::AbsInt, 101);
        recorder.record_opcode(OpCode::StoreLocal, 0, true, 102);
        recorder.record_loop_back(103);

        let trace = recorder.stop().unwrap();
        assert!(trace.is_compilable());
    }
}
