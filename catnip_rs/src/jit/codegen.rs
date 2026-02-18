// FILE: catnip_rs/src/jit/codegen.rs
//! Cranelift-based code generation for JIT compilation.
//!
//! Compiles traces to native machine code using Cranelift.

use cranelift_codegen::ir::{
    condcodes::IntCC, types, AbiParam, BlockArg, Function, InstBuilder, MemFlags, Signature,
    StackSlotData, StackSlotKind, Type, UserFuncName,
};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::{BTreeSet, HashMap, VecDeque};

use super::trace::{Trace, TraceOp, TraceType};
use crate::constants::JIT_MAX_RECURSION_DEPTH;

/// Compute slot types from guard operations in the trace.
/// Returns a map of slot -> Cranelift type (I64 or F64).
fn compute_slot_types(trace: &Trace) -> HashMap<usize, Type> {
    let mut slot_types = HashMap::new();
    for op in &trace.ops {
        match op {
            TraceOp::GuardInt(slot) => {
                slot_types.insert(*slot, types::I64);
            }
            TraceOp::GuardFloat(slot) => {
                slot_types.insert(*slot, types::F64);
            }
            _ => {}
        }
    }
    // Default unguarded slots to I64
    for &slot in &trace.locals_used {
        slot_types.entry(slot).or_insert(types::I64);
    }
    slot_types
}

#[derive(Debug, Clone, Copy)]
struct CfgBlock {
    start: usize,
    end: usize,
}

/// Helper to convert Values to BlockArgs for jump/brif instructions
fn to_block_args(values: &[cranelift_codegen::ir::Value]) -> Vec<BlockArg> {
    values.iter().map(|&v| BlockArg::Value(v)).collect()
}

fn is_terminator(op: &TraceOp) -> bool {
    matches!(
        op,
        TraceOp::Jump(_)
            | TraceOp::JumpIfFalse(_)
            | TraceOp::JumpIfTrue(_)
            | TraceOp::GuardTrue
            | TraceOp::GuardFalse
            | TraceOp::LoopBack
            | TraceOp::Exit
            | TraceOp::Break
            | TraceOp::Continue
            | TraceOp::TailCallSelf { .. }
    )
}

fn map_ip_to_op(trace: &Trace) -> HashMap<usize, usize> {
    let mut map = HashMap::new();
    for (idx, ip) in trace.op_offsets.iter().enumerate() {
        map.entry(*ip).or_insert(idx);
    }
    map
}

fn build_cfg_blocks(
    trace: &Trace,
    ip_to_op: &HashMap<usize, usize>,
) -> Result<Vec<CfgBlock>, String> {
    if trace.ops.is_empty() {
        return Err("Trace has no operations".into());
    }

    let mut leaders: BTreeSet<usize> = BTreeSet::new();
    leaders.insert(0);

    for (i, op) in trace.ops.iter().enumerate() {
        match op {
            TraceOp::Jump(target) | TraceOp::JumpIfFalse(target) | TraceOp::JumpIfTrue(target) => {
                if let Some(&target_idx) = ip_to_op.get(target) {
                    leaders.insert(target_idx);
                }
                if i + 1 < trace.ops.len() {
                    leaders.insert(i + 1);
                }
            }
            TraceOp::GuardTrue | TraceOp::GuardFalse => {
                if i + 1 < trace.ops.len() {
                    leaders.insert(i + 1);
                }
            }
            TraceOp::LoopBack | TraceOp::Exit | TraceOp::Break | TraceOp::Continue => {
                if i + 1 < trace.ops.len() {
                    leaders.insert(i + 1);
                }
            }
            _ => {}
        }
    }

    let leaders: Vec<usize> = leaders.into_iter().collect();
    let mut blocks = Vec::with_capacity(leaders.len());
    for (idx, start) in leaders.iter().enumerate() {
        let end = if idx + 1 < leaders.len() {
            leaders[idx + 1] - 1
        } else {
            trace.ops.len() - 1
        };
        blocks.push(CfgBlock { start: *start, end });
    }

    Ok(blocks)
}

fn build_op_to_block(blocks: &[CfgBlock], op_count: usize) -> Vec<usize> {
    let mut op_to_block = vec![0; op_count];
    for (block_idx, block) in blocks.iter().enumerate() {
        for i in block.start..=block.end {
            op_to_block[i] = block_idx;
        }
    }
    op_to_block
}

fn analyze_stack_heights(
    trace: &Trace,
    blocks: &[CfgBlock],
    op_to_block: &[usize],
    ip_to_op: &HashMap<usize, usize>,
    loop_header_block: usize,
) -> Result<(Vec<Option<usize>>, Option<usize>), String> {
    let mut block_heights: Vec<Option<usize>> = vec![None; blocks.len()];
    let mut exit_height: Option<usize> = None;
    let mut worklist: VecDeque<usize> = VecDeque::new();

    block_heights[loop_header_block] = Some(0);
    worklist.push_back(loop_header_block);

    while let Some(block_idx) = worklist.pop_front() {
        let height_in = block_heights[block_idx].unwrap();
        let block = blocks[block_idx];
        let mut height = height_in;

        for i in block.start..=block.end {
            let op = &trace.ops[i];

            match op {
                TraceOp::LoadConstInt(_) | TraceOp::LoadConstFloat(_) | TraceOp::LoadLocal(_) => {
                    height += 1;
                }
                TraceOp::StoreLocal(_) | TraceOp::StoreScope(_) | TraceOp::PopTop => {
                    if height == 0 {
                        return Err("Stack underflow during analysis".into());
                    }
                    height -= 1;
                }
                TraceOp::DupTop => {
                    if height == 0 {
                        return Err("Stack underflow on DupTop during analysis".into());
                    }
                    height += 1;
                }
                TraceOp::AddInt
                | TraceOp::SubInt
                | TraceOp::MulInt
                | TraceOp::DivInt
                | TraceOp::ModInt
                | TraceOp::LtInt
                | TraceOp::LeInt
                | TraceOp::GtInt
                | TraceOp::GeInt
                | TraceOp::EqInt
                | TraceOp::NeInt
                | TraceOp::AddFloat
                | TraceOp::SubFloat
                | TraceOp::MulFloat
                | TraceOp::DivFloat
                | TraceOp::LtFloat
                | TraceOp::LeFloat
                | TraceOp::GtFloat
                | TraceOp::GeFloat
                | TraceOp::EqFloat
                | TraceOp::NeFloat => {
                    if height < 2 {
                        return Err("Stack underflow during binary op analysis".into());
                    }
                    height -= 1; // pop 2, push 1
                }
                TraceOp::AbsInt | TraceOp::RoundInt | TraceOp::IntCastInt | TraceOp::BoolInt => {
                    // Unary: height unchanged (pop 1, push 1)
                    if height == 0 {
                        return Err("Stack underflow during unary builtin analysis".into());
                    }
                }
                TraceOp::MinInt | TraceOp::MaxInt => {
                    // Binary: pop 2, push 1
                    if height < 2 {
                        return Err("Stack underflow during MinInt/MaxInt analysis".into());
                    }
                    height -= 1;
                }
                TraceOp::CallBuiltinPure { num_args, .. } => {
                    // Pop num_args, push 1
                    let na = *num_args as usize;
                    if height < na {
                        return Err("Stack underflow during CallBuiltinPure analysis".into());
                    }
                    height = height - na + 1;
                }
                TraceOp::GuardInt(_) | TraceOp::GuardFloat(_) | TraceOp::GuardNameValue(_, _) => {}
                TraceOp::JumpIfFalse(_) | TraceOp::JumpIfTrue(_) => {
                    // Conditional jumps consume the condition
                    if height == 0 {
                        return Err("Stack underflow during conditional branch analysis".into());
                    }
                    height -= 1;
                    let fallthrough = if i + 1 < trace.ops.len() {
                        Some(op_to_block[i + 1])
                    } else {
                        None
                    };
                    let target_block = match op {
                        TraceOp::JumpIfFalse(target) | TraceOp::JumpIfTrue(target) => {
                            ip_to_op.get(target).map(|idx| op_to_block[*idx])
                        }
                        _ => None,
                    };

                    for succ in [fallthrough, target_block] {
                        if let Some(succ_idx) = succ {
                            match block_heights[succ_idx] {
                                Some(existing) => {
                                    if existing != height {
                                        return Err("Stack height mismatch at block entry".into());
                                    }
                                }
                                None => {
                                    block_heights[succ_idx] = Some(height);
                                    worklist.push_back(succ_idx);
                                }
                            }
                        } else if let Some(existing) = exit_height {
                            if existing != height {
                                return Err("Stack height mismatch at exit".into());
                            }
                        } else {
                            exit_height = Some(height);
                        }
                    }
                    break;
                }
                TraceOp::GuardTrue | TraceOp::GuardFalse => {
                    // Guards PEEK the stack (don't consume) - value consumed by subsequent PopTop
                    if height == 0 {
                        return Err("Stack underflow during guard analysis".into());
                    }
                    // height unchanged - we peek, not pop
                    let fallthrough = if i + 1 < trace.ops.len() {
                        Some(op_to_block[i + 1])
                    } else {
                        None
                    };
                    let target_block: Option<usize> = None; // guards exit on failure

                    for succ in [fallthrough, target_block] {
                        if let Some(succ_idx) = succ {
                            match block_heights[succ_idx] {
                                Some(existing) => {
                                    if existing != height {
                                        return Err("Stack height mismatch at block entry".into());
                                    }
                                }
                                None => {
                                    block_heights[succ_idx] = Some(height);
                                    worklist.push_back(succ_idx);
                                }
                            }
                        } else if let Some(existing) = exit_height {
                            if existing != height {
                                return Err("Stack height mismatch at exit".into());
                            }
                        } else {
                            exit_height = Some(height);
                        }
                    }
                    break;
                }
                TraceOp::Jump(target) => {
                    let target_block = ip_to_op.get(target).map(|idx| op_to_block[*idx]);
                    if let Some(succ_idx) = target_block {
                        match block_heights[succ_idx] {
                            Some(existing) => {
                                if existing != height {
                                    return Err("Stack height mismatch at jump target".into());
                                }
                            }
                            None => {
                                block_heights[succ_idx] = Some(height);
                                worklist.push_back(succ_idx);
                            }
                        }
                    } else if let Some(existing) = exit_height {
                        if existing != height {
                            return Err("Stack height mismatch at exit".into());
                        }
                    } else {
                        exit_height = Some(height);
                    }
                    break;
                }
                TraceOp::LoopBack | TraceOp::Continue => {
                    let succ_idx = loop_header_block;
                    match block_heights[succ_idx] {
                        Some(existing) => {
                            if existing != height {
                                return Err("Stack height mismatch at loop header".into());
                            }
                        }
                        None => {
                            block_heights[succ_idx] = Some(height);
                            worklist.push_back(succ_idx);
                        }
                    }
                    break;
                }
                TraceOp::Exit | TraceOp::Break => {
                    if let Some(existing) = exit_height {
                        if existing != height {
                            return Err("Stack height mismatch at exit".into());
                        }
                    } else {
                        exit_height = Some(height);
                    }
                    break;
                }
                TraceOp::Fallback(_) => {
                    return Err("Trace contains fallback operations".into());
                }
                TraceOp::CallSelf { num_args } => {
                    // CallSelf: pops num_args arguments, pushes 1 result
                    if height < *num_args {
                        return Err("Stack underflow during CallSelf analysis".into());
                    }
                    height = height - num_args + 1;
                }
                TraceOp::TailCallSelf { num_args } => {
                    // TailCallSelf: pops num_args arguments, then jumps (terminates block)
                    if height < *num_args {
                        return Err("Stack underflow during TailCallSelf analysis".into());
                    }
                    height = height - num_args;
                    // This is a terminator, will break from loop
                }
                TraceOp::CallPure { num_args, .. } => {
                    // CallPure: pops num_args arguments, pushes 1 result
                    if height < *num_args {
                        return Err("Stack underflow during CallPure analysis".into());
                    }
                    height = height - num_args + 1;
                }
            }

            if is_terminator(op) {
                break;
            }
        }

        if !trace.ops[block.start..=block.end].iter().any(is_terminator) {
            let next_block = blocks.get(block_idx + 1).map(|_| block_idx + 1);
            if let Some(succ_idx) = next_block {
                match block_heights[succ_idx] {
                    Some(existing) => {
                        if existing != height {
                            return Err("Stack height mismatch at fallthrough".into());
                        }
                    }
                    None => {
                        block_heights[succ_idx] = Some(height);
                        worklist.push_back(succ_idx);
                    }
                }
            } else if let Some(existing) = exit_height {
                if existing != height {
                    return Err("Stack height mismatch at exit".into());
                }
            } else {
                exit_height = Some(height);
            }
        }
    }

    Ok((block_heights, exit_height))
}

/// JIT code generator using Cranelift.
pub struct JITCodegen {
    /// Cranelift JIT module
    module: JITModule,
    /// Function builder context (reusable)
    builder_ctx: FunctionBuilderContext,
    /// Codegen context (reusable)
    ctx: Context,
    /// External unbox_int function (lazy-initialized)
    unbox_int_fn: Option<FuncId>,
    /// External unbox_float function (lazy-initialized)
    unbox_float_fn: Option<FuncId>,
    /// External memo_lookup function (Phase 4.3 - memoization)
    memo_lookup_fn: Option<FuncId>,
    /// External memo_store function (Phase 4.3 - memoization)
    memo_store_fn: Option<FuncId>,
}

/// Result of JIT compilation - a callable function pointer.
// Phase 3: Added depth parameter for recursion overflow protection
pub type CompiledFn = unsafe extern "C" fn(*mut i64, i32) -> i64;

/// Compile a simple trace operation (no control flow).
fn compile_simple_op(
    builder: &mut FunctionBuilder,
    op: &TraceOp,
    stack: &mut Vec<cranelift_codegen::ir::Value>,
    slot_vars: &HashMap<usize, Variable>,
    _slot_types: &HashMap<usize, Type>,
) -> Result<(), String> {
    use cranelift_codegen::ir::condcodes::IntCC;

    match op {
        TraceOp::LoadConstInt(val) => {
            let v = builder.ins().iconst(types::I64, *val);
            stack.push(v);
        }
        TraceOp::LoadConstFloat(val) => {
            let v = builder.ins().f64const(*val);
            stack.push(v);
        }
        TraceOp::LoadLocal(slot) => {
            if let Some(&var) = slot_vars.get(slot) {
                // Variable is already the correct type (I64 or F64)
                let val = builder.use_var(var);
                stack.push(val);
            } else {
                return Err(format!("Unknown local slot: {}", slot));
            }
        }
        TraceOp::StoreLocal(slot) => {
            if let Some(&var) = slot_vars.get(slot) {
                let val = stack.pop().ok_or("Stack underflow on StoreLocal")?;
                // Variable is already the correct type, just store
                builder.def_var(var, val);
            } else {
                return Err(format!("Unknown local slot: {}", slot));
            }
        }
        TraceOp::StoreScope(_) => {
            stack.pop().ok_or("Stack underflow on StoreScope")?;
        }
        TraceOp::DupTop => {
            let val = *stack.last().ok_or("Stack underflow on DupTop")?;
            stack.push(val);
        }
        TraceOp::PopTop => {
            stack.pop().ok_or("Stack underflow on PopTop")?;
        }
        TraceOp::AddInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().iadd(a, b));
        }
        TraceOp::SubInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().isub(a, b));
        }
        TraceOp::MulInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().imul(a, b));
        }
        TraceOp::DivInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().sdiv(a, b));
        }
        TraceOp::ModInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().srem(a, b));
        }
        TraceOp::LtInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder.ins().icmp(IntCC::SignedLessThan, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::LeInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder.ins().icmp(IntCC::SignedLessThanOrEqual, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::GtInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::GeInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::EqInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder.ins().icmp(IntCC::Equal, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::NeInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder.ins().icmp(IntCC::NotEqual, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::AbsInt => {
            let a = stack.pop().ok_or("Stack underflow")?;
            let zero = builder.ins().iconst(types::I64, 0);
            let is_neg = builder.ins().icmp(IntCC::SignedLessThan, a, zero);
            let negated = builder.ins().ineg(a);
            stack.push(builder.ins().select(is_neg, negated, a));
        }
        TraceOp::MinInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cond = builder.ins().icmp(IntCC::SignedLessThan, a, b);
            stack.push(builder.ins().select(cond, a, b));
        }
        TraceOp::MaxInt => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cond = builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
            stack.push(builder.ins().select(cond, a, b));
        }
        TraceOp::RoundInt | TraceOp::IntCastInt => {
            // Identity on ints: value stays on stack unchanged
            if stack.is_empty() {
                return Err("Stack underflow on RoundInt/IntCastInt".into());
            }
        }
        TraceOp::BoolInt => {
            let a = stack.pop().ok_or("Stack underflow")?;
            let zero = builder.ins().iconst(types::I64, 0);
            let cmp = builder.ins().icmp(IntCC::NotEqual, a, zero);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::GuardInt(_) | TraceOp::GuardFloat(_) | TraceOp::GuardNameValue(_, _) => {
            // Guards - checked before JIT entry, assumed valid during execution
        }
        TraceOp::AddFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().fadd(a, b));
        }
        TraceOp::SubFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().fsub(a, b));
        }
        TraceOp::MulFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().fmul(a, b));
        }
        TraceOp::DivFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            stack.push(builder.ins().fdiv(a, b));
        }
        TraceOp::LtFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder
                .ins()
                .fcmp(cranelift_codegen::ir::condcodes::FloatCC::LessThan, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::LeFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder.ins().fcmp(
                cranelift_codegen::ir::condcodes::FloatCC::LessThanOrEqual,
                a,
                b,
            );
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::GtFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp =
                builder
                    .ins()
                    .fcmp(cranelift_codegen::ir::condcodes::FloatCC::GreaterThan, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::GeFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder.ins().fcmp(
                cranelift_codegen::ir::condcodes::FloatCC::GreaterThanOrEqual,
                a,
                b,
            );
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::EqFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder
                .ins()
                .fcmp(cranelift_codegen::ir::condcodes::FloatCC::Equal, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        TraceOp::NeFloat => {
            let b = stack.pop().ok_or("Stack underflow")?;
            let a = stack.pop().ok_or("Stack underflow")?;
            let cmp = builder
                .ins()
                .fcmp(cranelift_codegen::ir::condcodes::FloatCC::NotEqual, a, b);
            stack.push(builder.ins().uextend(types::I64, cmp));
        }
        _ => {
            return Err(format!("compile_simple_op: unexpected op {:?}", op));
        }
    }
    Ok(())
}

impl JITCodegen {
    /// Create a new JIT code generator.
    pub fn new() -> Result<Self, String> {
        let mut flag_builder = settings::builder();
        flag_builder
            .set("opt_level", "speed")
            .map_err(|e| e.to_string())?;
        flag_builder
            .set("is_pic", "false")
            .map_err(|e| e.to_string())?;

        let isa_builder = cranelift_native::builder()
            .map_err(|e| format!("Failed to create ISA builder: {}", e))?;

        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| format!("Failed to build ISA: {}", e))?;

        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let module = JITModule::new(builder);

        Ok(Self {
            module,
            builder_ctx: FunctionBuilderContext::new(),
            ctx: Context::new(),
            unbox_int_fn: None,
            unbox_float_fn: None,
            memo_lookup_fn: None,
            memo_store_fn: None,
        })
    }

    /// Declare external unbox_int function (lazy initialization).
    fn ensure_unbox_int_fn(&mut self) -> Result<FuncId, String> {
        if let Some(func_id) = self.unbox_int_fn {
            return Ok(func_id);
        }

        // extern "C" fn catnip_unbox_int(boxed: i64) -> i64
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // boxed value
        sig.returns.push(AbiParam::new(types::I64)); // unboxed int

        let func_id = self
            .module
            .declare_function("catnip_unbox_int", Linkage::Import, &sig)
            .map_err(|e| format!("Failed to declare external function: {}", e))?;

        self.unbox_int_fn = Some(func_id);
        Ok(func_id)
    }

    /// Declare external unbox_float function (lazy initialization).
    fn ensure_unbox_float_fn(&mut self) -> Result<FuncId, String> {
        if let Some(func_id) = self.unbox_float_fn {
            return Ok(func_id);
        }

        // extern "C" fn catnip_unbox_float(boxed: i64) -> f64
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // boxed value
        sig.returns.push(AbiParam::new(types::F64)); // unboxed float

        let func_id = self
            .module
            .declare_function("catnip_unbox_float", Linkage::Import, &sig)
            .map_err(|e| format!("Failed to declare external function: {}", e))?;

        self.unbox_float_fn = Some(func_id);
        Ok(func_id)
    }

    /// Declare external memo_lookup function (Phase 4.3 - lazy initialization).
    fn ensure_memo_lookup_fn(&mut self) -> Result<FuncId, String> {
        if let Some(func_id) = self.memo_lookup_fn {
            return Ok(func_id);
        }

        // extern "C" fn memo_lookup(func_id: u64, arg: i64) -> i64
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // func_id hash
        sig.params.push(AbiParam::new(types::I64)); // arg (unboxed)
        sig.returns.push(AbiParam::new(types::I64)); // cached result (NaN-boxed) or -1

        let func_id = self
            .module
            .declare_function("memo_lookup", Linkage::Import, &sig)
            .map_err(|e| format!("Failed to declare external function: {}", e))?;

        self.memo_lookup_fn = Some(func_id);
        Ok(func_id)
    }

    /// Declare external memo_store function (Phase 4.3 - lazy initialization).
    fn ensure_memo_store_fn(&mut self) -> Result<FuncId, String> {
        if let Some(func_id) = self.memo_store_fn {
            return Ok(func_id);
        }

        // extern "C" fn memo_store(func_id: u64, arg: i64, result: i64)
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // func_id hash
        sig.params.push(AbiParam::new(types::I64)); // arg (unboxed)
        sig.params.push(AbiParam::new(types::I64)); // result (NaN-boxed)

        let func_id = self
            .module
            .declare_function("memo_store", Linkage::Import, &sig)
            .map_err(|e| format!("Failed to declare external function: {}", e))?;

        self.memo_store_fn = Some(func_id);
        Ok(func_id)
    }

    /// Compile a trace to native code.
    pub fn compile(&mut self, trace: &Trace) -> Result<CompiledFn, String> {
        if !trace.is_compilable() {
            return Err("Trace is not compilable".into());
        }

        // Create function signature: fn(*mut i64, i32) -> i64
        // Phase 3: Added depth parameter for recursion overflow protection
        // Takes pointer to locals array and depth counter, returns last value
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // locals pointer
        sig.params.push(AbiParam::new(types::I32)); // depth counter (Phase 3)
        sig.returns.push(AbiParam::new(types::I64)); // return value

        // Generate function name based on trace type
        let func_name = match trace.trace_type {
            TraceType::Loop => format!("trace_{}", trace.loop_offset),
            TraceType::Function => {
                if let Some(ref func_id) = trace.func_id {
                    format!("trace_{}", func_id)
                } else {
                    return Err("Function trace missing func_id".into());
                }
            }
        };

        let func_id = self
            .module
            .declare_function(&func_name, Linkage::Local, &sig)
            .map_err(|e| format!("Failed to declare function: {}", e))?;

        self.ctx.func = Function::with_name_signature(UserFuncName::user(0, func_id.as_u32()), sig);

        // Build function body - pass func_id for recursive calls if it's a function trace
        let self_func_id = match trace.trace_type {
            TraceType::Function => Some(func_id),
            TraceType::Loop => None,
        };
        self.build_function_body(trace, self_func_id)?;

        // Compile
        self.module
            .define_function(func_id, &mut self.ctx)
            .map_err(|e| format!("Failed to define function: {}", e))?;

        self.module.clear_context(&mut self.ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize: {}", e))?;

        // Get function pointer
        let code_ptr = self.module.get_finalized_function(func_id);

        Ok(unsafe { std::mem::transmute(code_ptr) })
    }

    fn build_function_body(
        &mut self,
        trace: &Trace,
        self_func_id: Option<FuncId>,
    ) -> Result<(), String> {
        let ip_to_op = map_ip_to_op(trace);
        let blocks = build_cfg_blocks(trace, &ip_to_op)?;
        let op_to_block = build_op_to_block(&blocks, trace.ops.len());
        let loop_header_block = op_to_block[0];
        let (block_heights_opt, exit_height_opt) =
            analyze_stack_heights(trace, &blocks, &op_to_block, &ip_to_op, loop_header_block)?;

        let exit_stack_height = exit_height_opt.unwrap_or(0);

        // Compute slot types from guards
        let slot_types = compute_slot_types(trace);

        let block_stack_heights: Vec<usize> =
            block_heights_opt.iter().map(|h| h.unwrap_or(0)).collect();

        // Declare unbox functions if trace contains CallSelf
        let has_call_self = trace
            .ops
            .iter()
            .any(|op| matches!(op, TraceOp::CallSelf { .. }));

        // Choose unbox function based on trace type (int-only vs float)
        let (unbox_int_id, unbox_float_id) = if has_call_self {
            if trace.is_int_only {
                // Int-only trace: use unbox_int
                (Some(self.ensure_unbox_int_fn()?), None)
            } else {
                // Trace has floats: use unbox_float for return values
                (None, Some(self.ensure_unbox_float_fn()?))
            }
        } else {
            (None, None)
        };

        // Phase 4.3: Memoization support for recursive functions with 1 parameter
        // Enable memoization if: function trace + has CallSelf + exactly 1 parameter
        let enable_memoization =
            trace.trace_type == TraceType::Function && has_call_self && trace.num_params == 1;
        let memo_lookup_id = if enable_memoization {
            Some(self.ensure_memo_lookup_fn()?)
        } else {
            None
        };
        let memo_store_id = if enable_memoization {
            Some(self.ensure_memo_store_fn()?)
        } else {
            None
        };

        // Compute func_id hash for memoization cache key
        let func_id_hash = if let Some(ref func_id_str) = trace.func_id {
            // Simple hash: sum of bytes (good enough for cache isolation)
            func_id_str.bytes().map(|b| b as u64).sum::<u64>()
        } else {
            0
        };

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_ctx);

        // Declare self function for recursive calls if needed
        let self_callee = if let Some(func_id) = self_func_id {
            Some(self.module.declare_func_in_func(func_id, builder.func))
        } else {
            None
        };

        // Declare unbox functions in current function context
        let unbox_int_callee = if let Some(unbox_id) = unbox_int_id {
            Some(self.module.declare_func_in_func(unbox_id, builder.func))
        } else {
            None
        };
        let unbox_float_callee = if let Some(unbox_id) = unbox_float_id {
            Some(self.module.declare_func_in_func(unbox_id, builder.func))
        } else {
            None
        };

        // Declare memo functions in current function context (Phase 4.3)
        let memo_lookup_callee = if let Some(memo_id) = memo_lookup_id {
            Some(self.module.declare_func_in_func(memo_id, builder.func))
        } else {
            None
        };
        let memo_store_callee = if let Some(memo_id) = memo_store_id {
            Some(self.module.declare_func_in_func(memo_id, builder.func))
        } else {
            None
        };

        let entry_block = builder.create_block();
        let exit_block = builder.create_block();
        let guard_fail_block = builder.create_block(); // Side exit when guard fails
        let mut cl_blocks = Vec::with_capacity(blocks.len());
        for _ in 0..blocks.len() {
            cl_blocks.push(builder.create_block());
        }

        let locals_order: Vec<usize> = trace.locals_used.clone();
        let mut slot_vars: HashMap<usize, Variable> = HashMap::new();
        for &slot in &locals_order {
            // Declare variable with correct type (I64 or F64)
            let ty = slot_types.get(&slot).copied().unwrap_or(types::I64);
            let var = builder.declare_var(ty);
            slot_vars.insert(slot, var);
        }

        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        // Copy block params to Vec to avoid borrow issues
        let block_params: Vec<_> = builder.block_params(entry_block).to_vec();
        let mut initial_locals = Vec::with_capacity(locals_order.len());

        // Get locals pointer (first parameter) from VM
        let incoming_locals_ptr = block_params[0];

        // Get depth counter (second parameter) - Phase 3
        let depth = block_params[1];

        // Phase 3: Check recursion depth overflow protection
        // Check if depth > MAX_RECURSION_DEPTH
        let max_depth = builder
            .ins()
            .iconst(types::I32, JIT_MAX_RECURSION_DEPTH as i64);
        let too_deep = builder
            .ins()
            .icmp(IntCC::SignedGreaterThan, depth, max_depth);

        // Create overflow block (returns -1 sentinel for guard failure)
        let overflow_block = builder.create_block();

        // Create normal_path block for when depth is ok
        let normal_path = builder.create_block();

        // Branch: if too_deep, go to overflow_block, else continue to normal_path
        builder
            .ins()
            .brif(too_deep, overflow_block, &[], normal_path, &[]);

        // Setup overflow block: return -1 (guard failure sentinel)
        builder.switch_to_block(overflow_block);
        builder.seal_block(overflow_block);
        let minus_one = builder.ins().iconst(types::I64, -1);
        builder.ins().return_(&[minus_one]);

        // Continue with normal_path
        builder.switch_to_block(normal_path);
        builder.seal_block(normal_path);

        // Store locals pointer in stack slot so it's accessible from all blocks
        let locals_ptr_slot =
            builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                8,
                0,
            ));
        let slot_addr = builder.ins().stack_addr(types::I64, locals_ptr_slot, 0);
        builder.ins().store(
            cranelift_codegen::ir::MemFlags::new(),
            incoming_locals_ptr,
            slot_addr,
            0,
        );

        // Load initial locals from incoming_locals_ptr and unbox them
        // NaN-boxing constants from value.rs
        const PAYLOAD_MASK: i64 = 0x0000_FFFF_FFFF_FFFF_i64;
        const SMALLINT_SIGN_BIT: i64 = 0x0000_8000_0000_0000_i64;
        const SMALLINT_SIGN_EXT: i64 = -0x0001_0000_0000_0000_i64; // 0xFFFF_0000_0000_0000

        for &slot in &locals_order {
            let offset = (slot * 8) as i32;
            let addr = builder.ins().iadd_imm(incoming_locals_ptr, offset as i64);
            // Always load as I64 from memory (NaN-boxed representation)
            let val_i64 =
                builder
                    .ins()
                    .load(types::I64, cranelift_codegen::ir::MemFlags::new(), addr, 0);

            let ty = slot_types.get(&slot).copied().unwrap_or(types::I64);
            let val = if ty == types::F64 {
                // For floats: bitcast to F64 (they're stored as raw f64 bits in NaN-box)
                let flags = cranelift_codegen::ir::MemFlags::new();
                builder.ins().bitcast(types::F64, flags, val_i64)
            } else {
                // For ints: unbox by extracting payload (48 bits) and sign-extending
                // payload = val_i64 & PAYLOAD_MASK
                let payload_mask = builder.ins().iconst(types::I64, PAYLOAD_MASK);
                let payload = builder.ins().band(val_i64, payload_mask);

                // Check sign bit: (payload & SMALLINT_SIGN_BIT) != 0
                let sign_bit_mask = builder.ins().iconst(types::I64, SMALLINT_SIGN_BIT);
                let sign_bit = builder.ins().band(payload, sign_bit_mask);
                let zero = builder.ins().iconst(types::I64, 0);
                let is_negative = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                    sign_bit,
                    zero,
                );

                // Sign extend if negative: payload | SMALLINT_SIGN_EXT
                let sign_ext_mask = builder.ins().iconst(types::I64, SMALLINT_SIGN_EXT);
                let extended = builder.ins().bor(payload, sign_ext_mask);

                // Select between payload (positive) and extended (negative)
                builder.ins().select(is_negative, extended, payload)
            };
            initial_locals.push(val);
        }

        // Block params: locals have their correct types, stack values are I64
        // (comparison results are always I64)
        for (block_id, block) in cl_blocks.iter().enumerate() {
            for &slot in &locals_order {
                let ty = slot_types.get(&slot).copied().unwrap_or(types::I64);
                builder.append_block_param(*block, ty);
            }
            for _ in 0..block_stack_heights[block_id] {
                // Stack values: could be I64 or F64 depending on ops
                // For simplicity, use I64 for now (comparison results)
                builder.append_block_param(*block, types::I64);
            }
        }

        for &slot in &locals_order {
            let ty = slot_types.get(&slot).copied().unwrap_or(types::I64);
            builder.append_block_param(exit_block, ty);
            builder.append_block_param(guard_fail_block, ty);
        }
        for _ in 0..exit_stack_height {
            builder.append_block_param(exit_block, types::I64);
            builder.append_block_param(guard_fail_block, types::I64);
        }

        let entry_target = cl_blocks[loop_header_block];
        let block_args = to_block_args(&initial_locals);
        builder.ins().jump(entry_target, &block_args);

        for (block_id, block) in blocks.iter().enumerate() {
            if block_heights_opt[block_id].is_none() {
                continue;
            }

            builder.switch_to_block(cl_blocks[block_id]);

            let params: Vec<_> = builder.block_params(cl_blocks[block_id]).to_vec();
            let mut param_idx = 0;
            for &slot in &locals_order {
                let var = slot_vars[&slot];
                let val = params[param_idx];
                builder.def_var(var, val);
                param_idx += 1;
            }

            let mut stack: Vec<cranelift_codegen::ir::Value> = params[param_idx..].to_vec();

            let mut terminated = false;

            for i in block.start..=block.end {
                let op = &trace.ops[i];
                match op {
                    TraceOp::Jump(target) => {
                        let args = {
                            let mut args = Vec::with_capacity(locals_order.len() + stack.len());
                            for &slot in &locals_order {
                                args.push(builder.use_var(slot_vars[&slot]));
                            }
                            args.extend_from_slice(&stack);
                            args
                        };

                        let block_args = to_block_args(&args);
                        if let Some(target_idx) = ip_to_op.get(target) {
                            let succ = op_to_block[*target_idx];
                            if stack.len() != block_stack_heights[succ] {
                                return Err("Stack height mismatch at jump".into());
                            }
                            builder.ins().jump(cl_blocks[succ], &block_args);
                        } else {
                            if stack.len() != exit_stack_height {
                                return Err("Stack height mismatch at exit".into());
                            }
                            builder.ins().jump(exit_block, &block_args);
                        }
                        terminated = true;
                        break;
                    }
                    TraceOp::JumpIfFalse(target) | TraceOp::JumpIfTrue(target) => {
                        let cond = stack.pop().ok_or("Stack underflow on conditional jump")?;
                        let zero = builder.ins().iconst(types::I64, 0);
                        let is_true = builder.ins().icmp(
                            cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                            cond,
                            zero,
                        );

                        let fallthrough = if i + 1 < trace.ops.len() {
                            Some(op_to_block[i + 1])
                        } else {
                            None
                        };
                        let target_block = ip_to_op.get(target).map(|idx| op_to_block[*idx]);

                        let (true_block, false_block) = match op {
                            TraceOp::JumpIfFalse(_) => (fallthrough, target_block),
                            TraceOp::JumpIfTrue(_) => (target_block, fallthrough),
                            _ => (fallthrough, target_block),
                        };

                        let args = {
                            let mut args = Vec::with_capacity(locals_order.len() + stack.len());
                            for &slot in &locals_order {
                                args.push(builder.use_var(slot_vars[&slot]));
                            }
                            args.extend_from_slice(&stack);
                            args
                        };

                        let block_args = to_block_args(&args);
                        let true_args = block_args.clone();
                        let false_args = block_args;

                        if let Some(b) = true_block {
                            if stack.len() != block_stack_heights[b] {
                                return Err("Stack height mismatch at branch target".into());
                            }
                        }
                        if let Some(b) = false_block {
                            if stack.len() != block_stack_heights[b] {
                                return Err("Stack height mismatch at branch target".into());
                            }
                        }

                        let true_block = true_block.map(|b| cl_blocks[b]).unwrap_or(exit_block);
                        let false_block = false_block.map(|b| cl_blocks[b]).unwrap_or(exit_block);

                        if true_block == exit_block && stack.len() != exit_stack_height {
                            return Err("Stack height mismatch at exit".into());
                        }
                        if false_block == exit_block && stack.len() != exit_stack_height {
                            return Err("Stack height mismatch at exit".into());
                        }

                        builder.ins().brif(
                            is_true,
                            true_block,
                            &true_args,
                            false_block,
                            &false_args,
                        );
                        terminated = true;
                        break;
                    }
                    TraceOp::GuardTrue | TraceOp::GuardFalse => {
                        // Guards check top of stack WITHOUT consuming it
                        // The value will be consumed by a subsequent PopTop
                        let cond = *stack.last().ok_or("Stack underflow on guard")?;
                        let zero = builder.ins().iconst(types::I64, 0);
                        let is_true = builder.ins().icmp(
                            cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                            cond,
                            zero,
                        );
                        let fallthrough = if i + 1 < trace.ops.len() {
                            Some(op_to_block[i + 1])
                        } else {
                            None
                        };

                        let (true_block, false_block) = match op {
                            TraceOp::GuardTrue => (fallthrough, None),
                            TraceOp::GuardFalse => (None, fallthrough),
                            _ => (fallthrough, None),
                        };

                        let args = {
                            let mut args = Vec::with_capacity(locals_order.len() + stack.len());
                            for &slot in &locals_order {
                                args.push(builder.use_var(slot_vars[&slot]));
                            }
                            args.extend_from_slice(&stack);
                            args
                        };

                        let block_args = to_block_args(&args);

                        if let Some(b) = true_block {
                            if stack.len() != block_stack_heights[b] {
                                return Err("Stack height mismatch at branch target".into());
                            }
                        }
                        if let Some(b) = false_block {
                            if stack.len() != block_stack_heights[b] {
                                return Err("Stack height mismatch at branch target".into());
                            }
                        }

                        // Guards use guard_fail_block for side exits
                        let true_block =
                            true_block.map(|b| cl_blocks[b]).unwrap_or(guard_fail_block);
                        let false_block = false_block
                            .map(|b| cl_blocks[b])
                            .unwrap_or(guard_fail_block);

                        if true_block == guard_fail_block && stack.len() != exit_stack_height {
                            return Err("Stack height mismatch at guard exit".into());
                        }
                        if false_block == guard_fail_block && stack.len() != exit_stack_height {
                            return Err("Stack height mismatch at guard exit".into());
                        }

                        builder.ins().brif(
                            is_true,
                            true_block,
                            &block_args,
                            false_block,
                            &block_args,
                        );
                        terminated = true;
                        break;
                    }
                    TraceOp::LoopBack | TraceOp::Continue => {
                        let args = {
                            let mut args = Vec::with_capacity(locals_order.len() + stack.len());
                            for &slot in &locals_order {
                                args.push(builder.use_var(slot_vars[&slot]));
                            }
                            args.extend_from_slice(&stack);
                            args
                        };

                        let block_args = to_block_args(&args);

                        if stack.len() != block_stack_heights[loop_header_block] {
                            return Err("Stack height mismatch at loop header".into());
                        }
                        builder
                            .ins()
                            .jump(cl_blocks[loop_header_block], &block_args);
                        terminated = true;
                        break;
                    }
                    TraceOp::Exit | TraceOp::Break => {
                        let args = {
                            let mut args = Vec::with_capacity(locals_order.len() + stack.len());
                            for &slot in &locals_order {
                                args.push(builder.use_var(slot_vars[&slot]));
                            }
                            args.extend_from_slice(&stack);
                            args
                        };

                        let block_args = to_block_args(&args);

                        if stack.len() != exit_stack_height {
                            return Err("Stack height mismatch at exit".into());
                        }
                        builder.ins().jump(exit_block, &block_args);
                        terminated = true;
                        break;
                    }
                    TraceOp::Fallback(_) => {
                        return Err("Trace contains fallback operations".into());
                    }
                    TraceOp::CallPure { .. } => {
                        // CallPure should have been inlined before codegen
                        // If still present, fall back to interpreter
                        return Err("CallPure not inlined before codegen".into());
                    }
                    TraceOp::CallBuiltinPure {
                        builtin_id,
                        num_args,
                    } => {
                        // Call extern C builtin dispatch function
                        // Declare catnip_call_builtin if not already done
                        let mut call_builtin_sig = Signature::new(CallConv::SystemV);
                        call_builtin_sig.params.push(AbiParam::new(types::I64)); // builtin_id
                        call_builtin_sig.params.push(AbiParam::new(types::I64)); // arg0
                        call_builtin_sig.params.push(AbiParam::new(types::I64)); // arg1
                        call_builtin_sig.params.push(AbiParam::new(types::I64)); // num_args
                        call_builtin_sig.returns.push(AbiParam::new(types::I64)); // result

                        let call_builtin_fn = self
                            .module
                            .declare_function(
                                "catnip_call_builtin",
                                Linkage::Import,
                                &call_builtin_sig,
                            )
                            .map_err(|e| format!("Failed to declare catnip_call_builtin: {}", e))?;
                        let call_builtin_callee = self
                            .module
                            .declare_func_in_func(call_builtin_fn, builder.func);

                        let na = *num_args as usize;

                        // Re-box arguments to NaN-boxed format for the callback
                        const QNAN_BASE_CB: i64 = 0x7FF8_0000_0000_0000_u64 as i64;
                        const PAYLOAD_MASK_CB: i64 = 0x0000_FFFF_FFFF_FFFF_i64;

                        // Collect args from stack (reverse order)
                        let mut args = Vec::with_capacity(na);
                        for _ in 0..na {
                            args.push(stack.pop().ok_or("Stack underflow in CallBuiltinPure")?);
                        }
                        args.reverse();

                        // Re-box each arg
                        let mut boxed_args = Vec::with_capacity(2);
                        for j in 0..2 {
                            if j < na {
                                let payload_mask =
                                    builder.ins().iconst(types::I64, PAYLOAD_MASK_CB);
                                let payload = builder.ins().band(args[j], payload_mask);
                                let qnan = builder.ins().iconst(types::I64, QNAN_BASE_CB);
                                boxed_args.push(builder.ins().bor(qnan, payload));
                            } else {
                                boxed_args.push(builder.ins().iconst(types::I64, 0));
                            }
                        }

                        let bid = builder.ins().iconst(types::I64, *builtin_id as i64);
                        let nargs_val = builder.ins().iconst(types::I64, na as i64);

                        let call_inst = builder.ins().call(
                            call_builtin_callee,
                            &[bid, boxed_args[0], boxed_args[1], nargs_val],
                        );
                        let results = builder.inst_results(call_inst);
                        let boxed_result = results[0];

                        // Check sentinel -1 (guard failure)
                        let minus_one = builder.ins().iconst(types::I64, -1);
                        let is_fail = builder.ins().icmp(IntCC::Equal, boxed_result, minus_one);

                        let cb_fail_block = builder.create_block();
                        let cb_ok_block = builder.create_block();
                        builder
                            .ins()
                            .brif(is_fail, cb_fail_block, &[], cb_ok_block, &[]);

                        // Fail path: jump to guard_fail_block
                        builder.switch_to_block(cb_fail_block);
                        builder.seal_block(cb_fail_block);
                        let fail_args = {
                            let mut a = Vec::with_capacity(locals_order.len());
                            for &slot in &locals_order {
                                a.push(builder.use_var(slot_vars[&slot]));
                            }
                            a
                        };
                        builder
                            .ins()
                            .jump(guard_fail_block, &to_block_args(&fail_args));

                        // OK path: unbox result
                        builder.switch_to_block(cb_ok_block);
                        builder.seal_block(cb_ok_block);

                        // The result is NaN-boxed. For float results, it's raw f64 bits.
                        // For now, treat as float (the only callback builtin is float()).
                        // Unbox: bitcast i64 -> f64 if trace is not int-only,
                        // otherwise extract payload as int.
                        if trace.is_int_only {
                            // Unbox as int
                            let pm = builder.ins().iconst(types::I64, PAYLOAD_MASK_CB);
                            let payload = builder.ins().band(boxed_result, pm);
                            stack.push(payload);
                        } else {
                            // float() returns a float -> bitcast
                            let flags = MemFlags::new();
                            let float_val = builder.ins().bitcast(types::F64, flags, boxed_result);
                            // Convert back to I64 for stack (comparison-compatible)
                            let int_val = builder.ins().bitcast(types::I64, flags, float_val);
                            stack.push(int_val);
                        }
                    }
                    TraceOp::CallSelf { num_args } => {
                        // Phase 2 Complete: Generate actual recursive call in native code
                        // Phase 4.3: Memoization wrapper for single-argument recursive functions
                        if let Some(callee) = self_callee {
                            // Phase 4.3: Declare after_memo_block for scope
                            let after_memo_block = if memo_lookup_callee.is_some() && *num_args == 1
                            {
                                // Get first argument (unboxed) for memoization key
                                let first_arg =
                                    *stack.last().ok_or("Stack underflow in CallSelf")?;

                                // Call memo_lookup(func_id_hash, first_arg)
                                let func_id_hash_const =
                                    builder.ins().iconst(types::I64, func_id_hash as i64);
                                let lookup_inst = builder.ins().call(
                                    memo_lookup_callee.unwrap(),
                                    &[func_id_hash_const, first_arg],
                                );
                                let lookup_results = builder.inst_results(lookup_inst);
                                let cached_result_boxed = lookup_results[0];

                                // Check if cache hit (result != -1)
                                let minus_one = builder.ins().iconst(types::I64, -1);
                                let is_cache_hit = builder.ins().icmp(
                                    IntCC::NotEqual,
                                    cached_result_boxed,
                                    minus_one,
                                );

                                // Create blocks for cache hit/miss paths
                                let cache_hit_block = builder.create_block();
                                let cache_miss_block = builder.create_block();
                                let after_block = builder.create_block();

                                builder.ins().brif(
                                    is_cache_hit,
                                    cache_hit_block,
                                    &[],
                                    cache_miss_block,
                                    &[],
                                );

                                // === CACHE HIT PATH ===
                                builder.switch_to_block(cache_hit_block);
                                builder.seal_block(cache_hit_block);

                                // Unbox cached result (int or float)
                                let cached_result = if let Some(unbox_fn) = unbox_float_callee {
                                    // Float trace: unbox as f64
                                    let unbox_inst =
                                        builder.ins().call(unbox_fn, &[cached_result_boxed]);
                                    let unbox_results = builder.inst_results(unbox_inst);
                                    unbox_results[0]
                                } else if let Some(unbox_fn) = unbox_int_callee {
                                    // Int trace: unbox as i64
                                    let unbox_inst =
                                        builder.ins().call(unbox_fn, &[cached_result_boxed]);
                                    let unbox_results = builder.inst_results(unbox_inst);
                                    unbox_results[0]
                                } else {
                                    return Err(
                                        "Unbox function not available for memoization".into()
                                    );
                                };

                                // Pop argument from stack and push cached result
                                stack.pop().ok_or("Stack underflow in CallSelf cache hit")?;
                                stack.push(cached_result);

                                builder.ins().jump(after_block, &[]);

                                // === CACHE MISS PATH ===
                                builder.switch_to_block(cache_miss_block);
                                builder.seal_block(cache_miss_block);

                                Some(after_block)
                            } else {
                                None
                            };

                            // Create locals array for recursive call
                            // Size = max(num_args, nlocals) to match function signature
                            let array_size = (*num_args).max(trace.locals_used.len());
                            let locals_array_size = (array_size * 8) as u32;
                            let locals_array_slot =
                                builder.create_sized_stack_slot(StackSlotData::new(
                                    StackSlotKind::ExplicitSlot,
                                    locals_array_size,
                                    3, // align to 8 bytes
                                ));

                            // Copy arguments from stack to locals array
                            // Args are at stack[len - num_args .. len]
                            // CRITICAL: Must re-box values to NaN-boxing format for called function
                            const QNAN_BASE_CALL: i64 = 0x7FF8_0000_0000_0000_u64 as i64;
                            const TAG_SMALLINT_CALL: i64 = 0;
                            const PAYLOAD_MASK_CALL: i64 = 0x0000_FFFF_FFFF_FFFF_i64;

                            for i in 0..*num_args {
                                let idx = stack.len() - num_args + i;
                                let arg = stack[idx];
                                let arg_ty = builder.func.dfg.value_type(arg);

                                // Re-box to NaN-boxing format
                                let boxed_arg = if arg_ty == types::F64 {
                                    // For floats: bitcast to I64 (raw bits are correct for NaN-box)
                                    builder.ins().bitcast(types::I64, MemFlags::new(), arg)
                                } else {
                                    // For ints: re-box by combining QNAN_BASE | TAG_SMALLINT | (val & PAYLOAD_MASK)
                                    let payload_mask =
                                        builder.ins().iconst(types::I64, PAYLOAD_MASK_CALL);
                                    let payload = builder.ins().band(arg, payload_mask);

                                    let qnan_base =
                                        builder.ins().iconst(types::I64, QNAN_BASE_CALL);
                                    let tag_smallint =
                                        builder.ins().iconst(types::I64, TAG_SMALLINT_CALL);

                                    let base_with_tag = builder.ins().bor(qnan_base, tag_smallint);
                                    builder.ins().bor(base_with_tag, payload)
                                };

                                let offset = (i * 8) as i32;
                                let slot_addr =
                                    builder
                                        .ins()
                                        .stack_addr(types::I64, locals_array_slot, offset);
                                builder
                                    .ins()
                                    .store(MemFlags::new(), boxed_arg, slot_addr, 0);
                            }

                            // Get pointer to locals array
                            let locals_ptr =
                                builder.ins().stack_addr(types::I64, locals_array_slot, 0);

                            // Phase 3: Increment depth counter for recursive call
                            let one = builder.ins().iconst(types::I32, 1);
                            let depth_incremented = builder.ins().iadd(depth, one);

                            // Call the function recursively with depth + 1
                            let call_inst =
                                builder.ins().call(callee, &[locals_ptr, depth_incremented]);
                            let results = builder.inst_results(call_inst);
                            let boxed_result = results[0];

                            // Phase 4.3: Store result in memoization cache (before unboxing)
                            if memo_store_callee.is_some() && *num_args == 1 {
                                let first_arg = stack[stack.len() - num_args];
                                let func_id_hash_const =
                                    builder.ins().iconst(types::I64, func_id_hash as i64);
                                builder.ins().call(
                                    memo_store_callee.unwrap(),
                                    &[func_id_hash_const, first_arg, boxed_result],
                                );
                            }

                            // Unbox the result (NaN-boxed -> unboxed)
                            // Use unbox_float for float traces, unbox_int for int-only traces
                            let result = if let Some(unbox_fn) = unbox_float_callee {
                                // Float trace: unbox as f64
                                let unbox_inst = builder.ins().call(unbox_fn, &[boxed_result]);
                                let unbox_results = builder.inst_results(unbox_inst);
                                unbox_results[0]
                            } else if let Some(unbox_fn) = unbox_int_callee {
                                // Int trace: unbox as i64
                                let unbox_inst = builder.ins().call(unbox_fn, &[boxed_result]);
                                let unbox_results = builder.inst_results(unbox_inst);
                                unbox_results[0]
                            } else {
                                return Err("Unbox function not available for CallSelf".into());
                            };

                            // Check for guard failure (-1 from nested call)
                            // If the recursive call failed its guards, propagate the failure
                            let minus_one = builder.ins().iconst(types::I64, -1);
                            let is_guard_fail = builder.ins().icmp(IntCC::Equal, result, minus_one);

                            // Create a block for guard failure and a block for normal path
                            let callself_guard_fail = builder.create_block();
                            let callself_continue = builder.create_block();

                            builder.ins().brif(
                                is_guard_fail,
                                callself_guard_fail,
                                &[],
                                callself_continue,
                                &[],
                            );

                            // Guard fail path: jump to guard_fail_block to propagate error
                            builder.switch_to_block(callself_guard_fail);
                            builder.seal_block(callself_guard_fail);
                            let guard_args = {
                                let mut args = Vec::with_capacity(locals_order.len());
                                for &slot in &locals_order {
                                    args.push(builder.use_var(slot_vars[&slot]));
                                }
                                args
                            };
                            builder
                                .ins()
                                .jump(guard_fail_block, &to_block_args(&guard_args));

                            // Continue normal path
                            builder.switch_to_block(callself_continue);
                            builder.seal_block(callself_continue);

                            // Pop arguments from stack
                            for _ in 0..*num_args {
                                stack.pop().ok_or("Stack underflow in CallSelf")?;
                            }

                            // Push result onto stack
                            stack.push(result);

                            // Phase 4.3: Close memoization after_memo_block if used
                            if let Some(after_block) = after_memo_block {
                                // Jump to after_memo_block (both cache hit and miss paths converge here)
                                builder.ins().jump(after_block, &[]);

                                // Switch to after_memo_block to continue execution
                                builder.switch_to_block(after_block);
                                builder.seal_block(after_block);
                            }
                        } else {
                            return Err("CallSelf in non-function trace (loop traces don't support recursion)".into());
                        }
                    }
                    TraceOp::TailCallSelf { num_args } => {
                        // Phase 4.1: Tail-call optimization - jump instead of call
                        // No depth increment, no call overhead, reuse same frame

                        // Pop arguments from stack (they're already unboxed)
                        let mut args_values = Vec::with_capacity(*num_args);
                        for _ in 0..*num_args {
                            let arg = stack.pop().ok_or("Stack underflow in TailCallSelf")?;
                            args_values.push(arg);
                        }
                        args_values.reverse(); // Args were pushed in reverse order

                        // Build jump args: use new argument values for params, existing vars for others
                        // This avoids type mismatch errors when def_var would assign incompatible types
                        let jump_args = {
                            let mut args = Vec::with_capacity(locals_order.len());
                            for &slot in &locals_order {
                                if slot < trace.num_params && slot < args_values.len() {
                                    // Use new argument value for parameter slots
                                    args.push(args_values[slot]);
                                } else {
                                    // Use existing variable value for other slots
                                    args.push(builder.use_var(slot_vars[&slot]));
                                }
                            }
                            args
                        };

                        let target_block = cl_blocks[loop_header_block];
                        builder.ins().jump(target_block, &to_block_args(&jump_args));
                        terminated = true;
                        break;
                    }
                    _ => {
                        compile_simple_op(&mut builder, op, &mut stack, &slot_vars, &slot_types)?;
                    }
                }

                if is_terminator(op) {
                    break;
                }
            }

            if !terminated {
                let args = {
                    let mut args = Vec::with_capacity(locals_order.len() + stack.len());
                    for &slot in &locals_order {
                        args.push(builder.use_var(slot_vars[&slot]));
                    }
                    args.extend_from_slice(&stack);
                    args
                };

                let block_args = to_block_args(&args);

                if block_id + 1 < blocks.len() {
                    let succ = block_id + 1;
                    if stack.len() != block_stack_heights[succ] {
                        return Err("Stack height mismatch at fallthrough".into());
                    }
                    builder.ins().jump(cl_blocks[succ], &block_args);
                } else {
                    if stack.len() != exit_stack_height {
                        return Err("Stack height mismatch at exit".into());
                    }
                    builder.ins().jump(exit_block, &block_args);
                }
            }
        }

        builder.switch_to_block(exit_block);
        let params: Vec<_> = builder.block_params(exit_block).to_vec();

        let mut param_idx = 0;
        for &slot in &locals_order {
            let var = slot_vars[&slot];
            let val = params[param_idx];
            builder.def_var(var, val);
            param_idx += 1;
        }

        // Stack values from params
        let stack_values: Vec<cranelift_codegen::ir::Value> = params[param_idx..].to_vec();

        // Load locals pointer from stack slot
        let slot_addr = builder.ins().stack_addr(types::I64, locals_ptr_slot, 0);
        let locals_ptr = builder.ins().load(
            types::I64,
            cranelift_codegen::ir::MemFlags::new(),
            slot_addr,
            0,
        );

        // Store locals back to memory, re-boxing them to NaN-boxed format
        // NaN-boxing constants from value.rs
        const QNAN_BASE: i64 = 0x7FF8_0000_0000_0000_u64 as i64;
        const TAG_SMALLINT: i64 = 0;
        const PAYLOAD_MASK_STORE: i64 = 0x0000_FFFF_FFFF_FFFF_i64;

        for (&slot, &var) in &slot_vars {
            let val = builder.use_var(var);
            let ty = slot_types.get(&slot).copied().unwrap_or(types::I64);

            let val_i64 = if ty == types::F64 {
                // For floats: bitcast to I64 (raw bits are already correct for NaN-box)
                let flags = cranelift_codegen::ir::MemFlags::new();
                builder.ins().bitcast(types::I64, flags, val)
            } else {
                // For ints: re-box by combining QNAN_BASE | TAG_SMALLINT | (val & PAYLOAD_MASK)
                let payload_mask = builder.ins().iconst(types::I64, PAYLOAD_MASK_STORE);
                let payload = builder.ins().band(val, payload_mask);

                let qnan_base = builder.ins().iconst(types::I64, QNAN_BASE);
                let tag_smallint = builder.ins().iconst(types::I64, TAG_SMALLINT);

                let base_with_tag = builder.ins().bor(qnan_base, tag_smallint);
                builder.ins().bor(base_with_tag, payload)
            };

            let offset = (slot * 8) as i32;
            let addr = builder.ins().iadd_imm(locals_ptr, offset as i64);
            builder
                .ins()
                .store(cranelift_codegen::ir::MemFlags::new(), val_i64, addr, 0);
        }

        // Return value depends on trace type
        let ret_val = match trace.trace_type {
            TraceType::Loop => {
                // Return 0 = loop completed normally
                builder.ins().iconst(types::I64, 0)
            }
            TraceType::Function => {
                // Return the top of the stack (function return value), re-boxed
                if !stack_values.is_empty() {
                    let raw_value = *stack_values.last().unwrap();

                    // Re-box the value to NaN-boxed format
                    // QNAN_BASE | TAG_SMALLINT | (value & PAYLOAD_MASK)
                    const QNAN_BASE_RET: i64 = 0x7FF8_0000_0000_0000_u64 as i64;
                    const TAG_SMALLINT_RET: i64 = 0;
                    const PAYLOAD_MASK_RET: i64 = 0x0000_FFFF_FFFF_FFFF_i64;

                    let payload_mask = builder.ins().iconst(types::I64, PAYLOAD_MASK_RET);
                    let payload = builder.ins().band(raw_value, payload_mask);

                    let qnan_base = builder.ins().iconst(types::I64, QNAN_BASE_RET);
                    let tag_smallint = builder.ins().iconst(types::I64, TAG_SMALLINT_RET);

                    let base_with_tag = builder.ins().bor(qnan_base, tag_smallint);
                    builder.ins().bor(base_with_tag, payload)
                } else {
                    // No stack value - return NaN-boxed 0
                    const QNAN_BASE_ZERO: i64 = 0x7FF8_0000_0000_0000_u64 as i64;
                    builder.ins().iconst(types::I64, QNAN_BASE_ZERO)
                }
            }
        };
        builder.ins().return_(&[ret_val]);

        // guard_fail_block: handle guard failure based on trace type
        builder.switch_to_block(guard_fail_block);
        let guard_params: Vec<_> = builder.block_params(guard_fail_block).to_vec();
        let mut guard_param_idx = 0;
        for &slot in &locals_order {
            let var = slot_vars[&slot];
            let val = guard_params[guard_param_idx];
            builder.def_var(var, val);
            guard_param_idx += 1;
        }

        // Load locals pointer from stack slot
        let slot_addr = builder.ins().stack_addr(types::I64, locals_ptr_slot, 0);
        let locals_ptr = builder.ins().load(
            types::I64,
            cranelift_codegen::ir::MemFlags::new(),
            slot_addr,
            0,
        );

        // Store locals back to memory
        for (&slot, &var) in &slot_vars {
            let val = builder.use_var(var);
            let ty = slot_types.get(&slot).copied().unwrap_or(types::I64);
            let val_i64 = if ty == types::F64 {
                let flags = cranelift_codegen::ir::MemFlags::new();
                builder.ins().bitcast(types::I64, flags, val)
            } else {
                val
            };
            let offset = (slot * 8) as i32;
            let addr = builder.ins().iadd_imm(locals_ptr, offset as i64);
            builder
                .ins()
                .store(cranelift_codegen::ir::MemFlags::new(), val_i64, addr, 0);
        }

        // Return -1 = guard failure (side exit)
        let guard_fail_ret = builder.ins().iconst(types::I64, -1);
        builder.ins().return_(&[guard_fail_ret]);

        for block in cl_blocks {
            builder.seal_block(block);
        }
        builder.seal_block(exit_block);
        builder.seal_block(guard_fail_block);

        builder.finalize();
        Ok(())
    }
}

impl Default for JITCodegen {
    fn default() -> Self {
        Self::new().expect("Failed to create JIT codegen")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codegen_creation() {
        let codegen = JITCodegen::new();
        assert!(codegen.is_ok());
    }

    #[test]
    fn test_simple_trace_compilation() {
        let mut codegen = JITCodegen::new().unwrap();

        // Build a simple trace: load 0, add 1, store 0, loop
        let mut trace = Trace::new(100);
        trace.locals_used.push(0);
        trace.ops.push(TraceOp::LoadLocal(0));
        trace.op_offsets.push(100);
        trace.ops.push(TraceOp::LoadConstInt(1));
        trace.op_offsets.push(101);
        trace.ops.push(TraceOp::AddInt);
        trace.op_offsets.push(102);
        trace.ops.push(TraceOp::StoreLocal(0));
        trace.op_offsets.push(103);
        trace.ops.push(TraceOp::LoadLocal(0));
        trace.op_offsets.push(104);
        trace.ops.push(TraceOp::LoadConstInt(10));
        trace.op_offsets.push(105);
        trace.ops.push(TraceOp::LtInt);
        trace.op_offsets.push(106);
        trace.ops.push(TraceOp::GuardTrue);
        trace.op_offsets.push(107);
        trace.ops.push(TraceOp::PopTop);
        trace.op_offsets.push(107); // Same offset since PopTop is implicit after guard
        trace.ops.push(TraceOp::LoopBack);
        trace.op_offsets.push(108);
        trace.iterations = 1;

        let result = codegen.compile(&trace);
        assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
    }

    #[test]
    fn test_builtin_abs_compilation() {
        let mut codegen = JITCodegen::new().unwrap();

        // Trace: slot[0] = abs(slot[0]), loop
        let mut trace = Trace::new(100);
        trace.locals_used.push(0);
        trace.ops.push(TraceOp::LoadLocal(0));
        trace.op_offsets.push(100);
        trace.ops.push(TraceOp::AbsInt);
        trace.op_offsets.push(101);
        trace.ops.push(TraceOp::StoreLocal(0));
        trace.op_offsets.push(102);
        trace.ops.push(TraceOp::LoopBack);
        trace.op_offsets.push(103);
        trace.iterations = 1;

        let result = codegen.compile(&trace);
        assert!(
            result.is_ok(),
            "AbsInt compilation failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_builtin_min_max_compilation() {
        let mut codegen = JITCodegen::new().unwrap();

        // Trace: slot[0] = min(slot[0], slot[1]), loop
        let mut trace = Trace::new(200);
        trace.locals_used.push(0);
        trace.locals_used.push(1);
        trace.ops.push(TraceOp::LoadLocal(0));
        trace.op_offsets.push(200);
        trace.ops.push(TraceOp::LoadLocal(1));
        trace.op_offsets.push(201);
        trace.ops.push(TraceOp::MinInt);
        trace.op_offsets.push(202);
        trace.ops.push(TraceOp::StoreLocal(0));
        trace.op_offsets.push(203);
        trace.ops.push(TraceOp::LoadLocal(0));
        trace.op_offsets.push(204);
        trace.ops.push(TraceOp::LoadLocal(1));
        trace.op_offsets.push(205);
        trace.ops.push(TraceOp::MaxInt);
        trace.op_offsets.push(206);
        trace.ops.push(TraceOp::StoreLocal(1));
        trace.op_offsets.push(207);
        trace.ops.push(TraceOp::LoopBack);
        trace.op_offsets.push(208);
        trace.iterations = 1;

        let result = codegen.compile(&trace);
        assert!(
            result.is_ok(),
            "MinInt/MaxInt compilation failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_builtin_round_int_compilation() {
        let mut codegen = JITCodegen::new().unwrap();

        // Trace: slot[0] = round(slot[0]), loop
        let mut trace = Trace::new(100);
        trace.locals_used.push(0);
        trace.ops.push(TraceOp::LoadLocal(0));
        trace.op_offsets.push(100);
        trace.ops.push(TraceOp::RoundInt);
        trace.op_offsets.push(101);
        trace.ops.push(TraceOp::StoreLocal(0));
        trace.op_offsets.push(102);
        trace.ops.push(TraceOp::LoopBack);
        trace.op_offsets.push(103);
        trace.iterations = 1;

        let result = codegen.compile(&trace);
        assert!(
            result.is_ok(),
            "RoundInt compilation failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_builtin_bool_int_compilation() {
        let mut codegen = JITCodegen::new().unwrap();

        // Trace: slot[0] = bool(slot[0]), loop
        let mut trace = Trace::new(100);
        trace.locals_used.push(0);
        trace.ops.push(TraceOp::LoadLocal(0));
        trace.op_offsets.push(100);
        trace.ops.push(TraceOp::BoolInt);
        trace.op_offsets.push(101);
        trace.ops.push(TraceOp::StoreLocal(0));
        trace.op_offsets.push(102);
        trace.ops.push(TraceOp::LoopBack);
        trace.op_offsets.push(103);
        trace.iterations = 1;

        let result = codegen.compile(&trace);
        assert!(
            result.is_ok(),
            "BoolInt compilation failed: {:?}",
            result.err()
        );
    }
}
