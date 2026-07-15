// FILE: catnip_core/src/jit/inliner.rs
//! Pure function inlining pass for JIT traces.

use std::collections::HashMap;

use super::function_info::{JitConstant, JitFunctionInfo};
use super::registry::PureFunctionRegistry;
use super::trace::{Trace, TraceOp};
use crate::vm::VMOpCode;

/// Configuration for inlining optimization.
#[derive(Debug, Clone)]
pub struct InliningConfig {
    /// Maximum operations to inline (default: 20)
    pub max_inline_ops: usize,
    /// Maximum inlining depth (default: 2)
    pub max_inline_depth: usize,
}

impl Default for InliningConfig {
    fn default() -> Self {
        Self {
            max_inline_ops: crate::constants::JIT_MAX_INLINE_OPS,
            max_inline_depth: crate::constants::JIT_MAX_INLINE_DEPTH,
        }
    }
}

/// Pure function inliner for JIT traces.
pub struct PureInliner<'a> {
    config: InliningConfig,
    registry: &'a PureFunctionRegistry,
    depth: usize,
    /// Next free scratch slot for inlined callee locals (monotonic across the pass).
    next_slot: usize,
    /// Scratch slots allocated for inlined locals, to register in locals_used.
    inlined_slots: Vec<usize>,
}

impl<'a> PureInliner<'a> {
    /// Create a new inliner with given configuration.
    pub fn new(config: InliningConfig, registry: &'a PureFunctionRegistry) -> Self {
        Self {
            config,
            registry,
            depth: 0,
            next_slot: 0,
            inlined_slots: Vec::new(),
        }
    }

    /// Execute inlining pass on a trace.
    /// Returns Ok(()) if successful, Err(msg) on failure.
    pub fn optimize(&mut self, trace: &mut Trace) -> Result<(), String> {
        self.depth = 0;
        let mut new_ops = Vec::new();

        // Build scope name → slot mapping from trace name_guards
        let scope_slots: HashMap<String, usize> = trace
            .name_guards
            .iter()
            .map(|(name, _value, slot)| (name.clone(), *slot))
            .collect();

        // Inlined callee locals get fresh scratch slots above every slot the
        // host frame can hold: num_locals (frame.locals.len()) and any slot the
        // trace already references. Without this, arg-binding StoreLocal(0)
        // clobbers the host's slot 0, and callee temps alias host locals.
        self.next_slot = trace.num_locals;
        for &s in &trace.locals_used {
            self.next_slot = self.next_slot.max(s + 1);
        }
        for &s in scope_slots.values() {
            self.next_slot = self.next_slot.max(s + 1);
        }
        self.inlined_slots.clear();

        for op in &trace.ops {
            match op {
                TraceOp::CallPure { func_id, num_args } => {
                    // Try builtin expansion first (abs, min, max)
                    match Self::expand_builtin(func_id, *num_args) {
                        Some(ops) => new_ops.extend(ops),
                        None => {
                            if self.should_inline(func_id, *num_args) {
                                self.inline_call(&mut new_ops, func_id, *num_args, &scope_slots)?;
                            } else {
                                new_ops.push(op.clone());
                            }
                        }
                    }
                }
                _ => new_ops.push(op.clone()),
            }
        }

        trace.ops = new_ops;

        // Register the scratch slots so codegen and the locals-array sizing
        // (loop_max_slot / function max_slot) account for them.
        for slot in self.inlined_slots.drain(..) {
            if !trace.locals_used.contains(&slot) {
                trace.locals_used.push(slot);
            }
        }

        Ok(())
    }

    /// Number of local slots a callee body addresses (params + temporaries).
    fn callee_local_count(info: &JitFunctionInfo) -> usize {
        let mut max_local = None;
        for instr in &info.instructions {
            if matches!(instr.op, VMOpCode::LoadLocal | VMOpCode::StoreLocal) {
                max_local = Some(max_local.map_or(instr.arg as usize, |m: usize| m.max(instr.arg as usize)));
            }
        }
        max_local.map_or(0, |m| m + 1).max(info.nargs)
    }

    /// Try to expand a builtin function call to native TraceOps.
    fn expand_builtin(name: &str, num_args: usize) -> Option<Vec<TraceOp>> {
        match (name, num_args) {
            ("abs", 1) => Some(vec![TraceOp::AbsInt]),
            ("min", 2) => Some(vec![TraceOp::MinInt]),
            ("max", 2) => Some(vec![TraceOp::MaxInt]),
            ("round", 1) => Some(vec![TraceOp::RoundInt]),
            ("int", 1) => Some(vec![TraceOp::IntCastInt]),
            ("bool", 1) => Some(vec![TraceOp::BoolInt]),
            _ => None,
        }
    }

    /// Check if function should be inlined.
    fn should_inline(&self, func_id: &str, num_args: usize) -> bool {
        // Check depth limit
        if self.depth >= self.config.max_inline_depth {
            return false;
        }

        // Check if function is available and small enough
        if let Some(info) = self.registry.get_inlineable(func_id, self.config.max_inline_ops) {
            // Check argument count matches
            info.nargs == num_args
        } else {
            false
        }
    }

    /// Inline a function call into the trace.
    fn inline_call(
        &mut self,
        new_ops: &mut Vec<TraceOp>,
        func_id: &str,
        num_args: usize,
        scope_slots: &HashMap<String, usize>,
    ) -> Result<(), String> {
        let info = self
            .registry
            .get_inlineable(func_id, self.config.max_inline_ops)
            .ok_or_else(|| format!("Function {} not in registry", func_id))?;

        let local_count = Self::callee_local_count(info);
        let local_offset = self.next_slot;

        // Translate the body FIRST. This can fail (e.g. a float op the int-only
        // inliner cannot lower), and it must do so before any state is mutated:
        // a half-allocated scratch block left behind on the error path would
        // desync slot accounting for the rest of the trace.
        let body_ops = self.bytecode_to_trace_ops(info, scope_slots, local_offset)?;

        // Translation succeeded: commit the disjoint scratch-slot block.
        self.next_slot += local_count;
        for slot in local_offset..local_offset + local_count {
            self.inlined_slots.push(slot);
        }

        // Bind arguments: pop args from stack to the callee's (remapped) slots.
        // Stack: [..., arg0, arg1, ..., argN-1] with argN-1 on top. StoreLocal
        // pops the top, so bind from the last slot down to the first -- otherwise
        // argN-1 would land in slot 0 (reversed binding for num_args >= 2).
        for i in (0..num_args).rev() {
            new_ops.push(TraceOp::StoreLocal(local_offset + i));
        }

        // Inline body (recursively if needed)
        self.depth += 1;
        for body_op in body_ops {
            match &body_op {
                TraceOp::CallPure {
                    func_id: nested_id,
                    num_args: nested_args,
                } => {
                    // Recursive inline if within depth limit
                    if self.should_inline(nested_id, *nested_args) {
                        self.inline_call(new_ops, nested_id, *nested_args, scope_slots)?;
                    } else {
                        new_ops.push(body_op);
                    }
                }
                _ => new_ops.push(body_op),
            }
        }
        self.depth -= 1;

        Ok(())
    }

    /// Translate VM bytecode to TraceOps.
    /// Returns sequence of operations representing the function body.
    fn bytecode_to_trace_ops(
        &self,
        info: &JitFunctionInfo,
        scope_slots: &HashMap<String, usize>,
        local_offset: usize,
    ) -> Result<Vec<TraceOp>, String> {
        let mut ops = Vec::new();
        let last_idx = info.instructions.len().wrapping_sub(1);

        for (idx, instr) in info.instructions.iter().enumerate() {
            // An inlined callee's terminal Return must NOT become a trace
            // terminator (Exit): the result is already on the stack and the host
            // trace (the loop, or an outer function) continues after the call.
            // Drop a terminal Return. A non-terminal Return is an early return --
            // branching control flow the linear inliner cannot model -- so refuse
            // to inline (the call stays an interpreted CallPure).
            if instr.op == VMOpCode::Return {
                if idx == last_idx {
                    continue;
                }
                return Err("callee not inlinable: non-terminal Return".into());
            }
            // CheckType is the typed-param boundary check (TH2-B) emitted in an
            // annotated function's prologue. The int-only inliner can honor only
            // an INT check: on a value this trace already proves to be int it is a
            // no-op, so skip it (the surrounding LoadLocal/StoreLocal round-trip
            // stays, harmless). Any other declared type constrains the param to a
            // type this inliner cannot lower -- refuse, leaving the call as a
            // CallPure (interpreted), same policy as a float constant / *Float op.
            if instr.op == VMOpCode::CheckType {
                if instr.arg as u8 == crate::vm::opcode::type_code::INT {
                    continue;
                }
                return Err(format!(
                    "callee not int-only-inlinable: CheckType({})",
                    crate::vm::opcode::type_code::name(instr.arg as u8)
                ));
            }
            // Block-scope bookkeeping. A lambda body is always a block, so every
            // user callee carries PushBlock/PopBlock. They only reset block-local
            // slots on block exit; the inliner remaps callee locals to scratch
            // slots that are re-bound (StoreLocal) on every call, and only linear
            // bodies are inlinable (branches -> unsupported opcode -> refused), so
            // every local is assigned before use. The reset is therefore a no-op
            // for the inlined result -- skip both.
            if matches!(instr.op, VMOpCode::PushBlock | VMOpCode::PopBlock) {
                continue;
            }
            let trace_op = match instr.op {
                VMOpCode::LoadConst => {
                    let constant = info
                        .constants
                        .get(instr.arg as usize)
                        .ok_or("LoadConst: constant index out of bounds")?;
                    match constant {
                        JitConstant::Int(i) => TraceOp::LoadConstInt(*i),
                        // This inliner is int-only: it lowers every arithmetic op
                        // to its Int form. A float constant means the callee does
                        // float work that cannot be lowered safely, so refuse to
                        // inline -- the call stays a CallPure (interpreted) rather
                        // than feeding a float value to an int opcode.
                        JitConstant::Float(_) => {
                            return Err("callee not int-only-inlinable: float constant".into());
                        }
                    }
                }
                // Callee locals live in the callee's own slot namespace; shift
                // them into the scratch block. Scope vars (below) resolve to
                // host slots and must NOT be shifted.
                VMOpCode::LoadLocal => TraceOp::LoadLocal(instr.arg as usize + local_offset),
                VMOpCode::StoreLocal => TraceOp::StoreLocal(instr.arg as usize + local_offset),
                VMOpCode::LoadScope => {
                    let name = info
                        .names
                        .get(instr.arg as usize)
                        .ok_or("LoadScope: name index out of bounds")?;
                    let slot = scope_slots
                        .get(name.as_str())
                        .ok_or_else(|| format!("LoadScope: captured var '{}' not in trace scope", name))?;
                    TraceOp::LoadLocal(*slot)
                }
                VMOpCode::Add => TraceOp::AddInt,
                // TH4 canal A: the int-specialized ops inline like their
                // polymorphic form. The *Float variants are refused (they fall
                // through to the error arm below, like a float constant), so a
                // float callee stays a CallPure rather than being miscompiled as
                // int. This int-only inliner has no float lowering.
                VMOpCode::AddInt => TraceOp::AddInt,
                VMOpCode::SubInt => TraceOp::SubInt,
                VMOpCode::MulInt => TraceOp::MulInt,
                VMOpCode::Sub => TraceOp::SubInt,
                VMOpCode::Mul => TraceOp::MulInt,
                // True division (`/`) yields a float and raises on /0; an int-only
                // sdiv would truncate and SIGFPE. Refuse to inline it (falls to the
                // error arm below), so the call stays interpreted.
                VMOpCode::Mod => TraceOp::ModInt,
                VMOpCode::Lt => TraceOp::LtInt,
                VMOpCode::Le => TraceOp::LeInt,
                VMOpCode::Gt => TraceOp::GtInt,
                VMOpCode::Ge => TraceOp::GeInt,
                VMOpCode::Eq => TraceOp::EqInt,
                VMOpCode::Ne => TraceOp::NeInt,
                VMOpCode::DupTop => TraceOp::DupTop,
                VMOpCode::PopTop => TraceOp::PopTop,
                _ => return Err(format!("Unsupported opcode for inlining: {:?}", instr.op)),
            };
            ops.push(trace_op);
        }

        Ok(ops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::opcode::{Instruction, type_code};

    #[test]
    fn test_inline_simple_pure() {
        let mut registry = PureFunctionRegistry::new();

        // Create simple pure function: f(x) = x + 1
        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0), // x
                Instruction::new(VMOpCode::LoadConst, 0), // 1
                Instruction::new(VMOpCode::Add, 0),       // x + 1
                Instruction::new(VMOpCode::Return, 0),
            ],
            constants: vec![JitConstant::Int(1)],
            names: vec![],
            nargs: 1,
            complexity: 4,
            is_pure: true,
        };

        registry.register("add_one".to_string(), std::sync::Arc::new(info));

        let mut inliner = PureInliner::new(InliningConfig::default(), &registry);

        // Trace with CallPure
        let mut trace = Trace::new(0);
        trace.ops = vec![
            TraceOp::LoadConstInt(5), // arg
            TraceOp::CallPure {
                func_id: "add_one".to_string(),
                num_args: 1,
            },
        ];

        inliner.optimize(&mut trace).unwrap();

        // Should be inlined to:
        // LoadConstInt(5), StoreLocal(0), LoadLocal(0), LoadConstInt(1), AddInt, Exit
        assert!(trace.ops.len() > 2);
        assert!(matches!(trace.ops[0], TraceOp::LoadConstInt(5)));
        assert!(matches!(trace.ops[1], TraceOp::StoreLocal(0)));
    }

    #[test]
    fn test_inline_size_limit() {
        let mut registry = PureFunctionRegistry::new();

        // Create function with 25 ops (> max 20)
        let mut instructions = Vec::new();
        for _ in 0..25 {
            instructions.push(Instruction::new(VMOpCode::Nop, 0));
        }

        let info = JitFunctionInfo {
            instructions,
            constants: vec![],
            names: vec![],
            nargs: 1,
            complexity: 25,
            is_pure: true,
        };

        registry.register("big_fn".to_string(), std::sync::Arc::new(info));

        let mut inliner = PureInliner::new(InliningConfig::default(), &registry);

        let mut trace = Trace::new(0);
        trace.ops = vec![TraceOp::CallPure {
            func_id: "big_fn".to_string(),
            num_args: 1,
        }];

        inliner.optimize(&mut trace).unwrap();

        // Should NOT be inlined (too big)
        assert_eq!(trace.ops.len(), 1);
        assert!(matches!(
            trace.ops[0],
            TraceOp::CallPure {
                func_id: _,
                num_args: 1
            }
        ));
    }

    #[test]
    fn test_inline_depth_limit() {
        let mut registry = PureFunctionRegistry::new();

        // f(x) = x + 1
        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::Add, 0),
                Instruction::new(VMOpCode::Return, 0),
            ],
            constants: vec![JitConstant::Int(1)],
            names: vec![],
            nargs: 1,
            complexity: 4,
            is_pure: true,
        };

        registry.register("f".to_string(), std::sync::Arc::new(info));

        let mut inliner = PureInliner::new(
            InliningConfig {
                max_inline_ops: 20,
                max_inline_depth: 1,
            },
            &registry,
        );

        let mut trace = Trace::new(0);
        trace.ops = vec![TraceOp::CallPure {
            func_id: "f".to_string(),
            num_args: 1,
        }];

        inliner.optimize(&mut trace).unwrap();

        // Depth 1: should inline first call only
        assert!(trace.ops.len() > 1);
    }

    #[test]
    fn test_bytecode_translation() {
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);

        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::Add, 0),
            ],
            constants: vec![JitConstant::Int(10)],
            names: vec![],
            nargs: 0,
            complexity: 3,
            is_pure: false,
        };

        let empty_scope = HashMap::new();
        let ops = inliner.bytecode_to_trace_ops(&info, &empty_scope, 0).unwrap();

        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[0], TraceOp::LoadLocal(0)));
        assert!(matches!(ops[1], TraceOp::LoadConstInt(10)));
        assert!(matches!(ops[2], TraceOp::AddInt));
    }

    #[test]
    fn test_unsupported_opcode() {
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);

        let info = JitFunctionInfo {
            instructions: vec![Instruction::new(VMOpCode::GetAttr, 0)],
            constants: vec![],
            names: vec![],
            nargs: 0,
            complexity: 1,
            is_pure: false,
        };

        let empty_scope = HashMap::new();
        let result = inliner.bytecode_to_trace_ops(&info, &empty_scope, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_float_callee_refused() {
        // TH4 canal A: the int-only inliner must refuse a float callee (a float
        // constant or an AddFloat) instead of lowering it as int -- otherwise a
        // float value gets fed to an int opcode (silent garbage).
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);
        let empty_scope = HashMap::new();

        let float_const = JitFunctionInfo {
            instructions: vec![Instruction::new(VMOpCode::LoadConst, 0)],
            constants: vec![JitConstant::Float(1.5)],
            names: vec![],
            nargs: 0,
            complexity: 1,
            is_pure: false,
        };
        assert!(inliner.bytecode_to_trace_ops(&float_const, &empty_scope, 0).is_err());

        let add_float = JitFunctionInfo {
            instructions: vec![Instruction::new(VMOpCode::AddFloat, 0)],
            constants: vec![],
            names: vec![],
            nargs: 0,
            complexity: 1,
            is_pure: false,
        };
        assert!(inliner.bytecode_to_trace_ops(&add_float, &empty_scope, 0).is_err());
    }

    #[test]
    fn test_checktype_int_is_noop() {
        // TH2-B: an `(x: int) => x + 1` callee carries a typed-param prologue
        // LoadLocal/CheckType(INT)/StoreLocal. The int-only inliner treats the
        // INT check as a no-op (the value is already a proven int), so the body
        // lowers to int TraceOps with the CheckType dropped.
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);

        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::CheckType, type_code::INT as u32),
                Instruction::new(VMOpCode::StoreLocal, 0),
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::Add, 0),
                Instruction::new(VMOpCode::Return, 0),
            ],
            constants: vec![JitConstant::Int(1)],
            names: vec![],
            nargs: 1,
            complexity: 7,
            is_pure: true,
        };

        let empty_scope = HashMap::new();
        let ops = inliner.bytecode_to_trace_ops(&info, &empty_scope, 0).unwrap();

        // CheckType skipped, terminal Return dropped (no Exit): prologue
        // round-trip kept, body lowered to int ops, result left on the stack.
        assert_eq!(ops.len(), 5);
        assert!(matches!(ops[0], TraceOp::LoadLocal(0)));
        assert!(matches!(ops[1], TraceOp::StoreLocal(0)));
        assert!(matches!(ops[2], TraceOp::LoadLocal(0)));
        assert!(matches!(ops[3], TraceOp::LoadConstInt(1)));
        assert!(matches!(ops[4], TraceOp::AddInt));
        assert!(!ops.iter().any(|op| matches!(op, TraceOp::Exit)));
    }

    #[test]
    fn test_checktype_non_int_refused() {
        // Only an INT boundary check is honored. float/str/bool/None constrain
        // the param to a type the int-only inliner can't lower, so the callee is
        // refused (stays an interpreted CallPure).
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);
        let empty_scope = HashMap::new();

        for code in [type_code::FLOAT, type_code::STR, type_code::BOOL, type_code::NONE] {
            let info = JitFunctionInfo {
                instructions: vec![
                    Instruction::new(VMOpCode::LoadLocal, 0),
                    Instruction::new(VMOpCode::CheckType, code as u32),
                    Instruction::new(VMOpCode::StoreLocal, 0),
                ],
                constants: vec![],
                names: vec![],
                nargs: 1,
                complexity: 3,
                is_pure: true,
            };
            assert!(
                inliner.bytecode_to_trace_ops(&info, &empty_scope, 0).is_err(),
                "CheckType({}) should refuse inlining",
                type_code::name(code)
            );
        }
    }

    #[test]
    fn test_inline_typed_param_fn() {
        // End-to-end through optimize(): a registered `(x: int) => x + 1` with a
        // typed-param prologue is now inlinable (was refused on CheckType before).
        let mut registry = PureFunctionRegistry::new();

        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::CheckType, type_code::INT as u32),
                Instruction::new(VMOpCode::StoreLocal, 0),
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::LoadConst, 0),
                Instruction::new(VMOpCode::Add, 0),
                Instruction::new(VMOpCode::Return, 0),
            ],
            constants: vec![JitConstant::Int(1)],
            names: vec![],
            nargs: 1,
            complexity: 7,
            is_pure: true,
        };

        registry.register("inc".to_string(), std::sync::Arc::new(info));

        let mut inliner = PureInliner::new(InliningConfig::default(), &registry);

        let mut trace = Trace::new(0);
        trace.ops = vec![
            TraceOp::LoadConstInt(5),
            TraceOp::CallPure {
                func_id: "inc".to_string(),
                num_args: 1,
            },
        ];

        inliner.optimize(&mut trace).unwrap();

        // The CallPure is gone (inlined), not left interpreted.
        assert!(!trace.ops.iter().any(|op| matches!(op, TraceOp::CallPure { .. })));
        assert!(matches!(trace.ops[0], TraceOp::LoadConstInt(5)));
        assert!(matches!(trace.ops[1], TraceOp::StoreLocal(0)));
        // CheckType lowered to nothing: no stray opcode survives the prologue.
        assert!(trace.ops.iter().any(|op| matches!(op, TraceOp::AddInt)));
    }

    #[test]
    fn test_load_scope_with_known_var() {
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);

        // closure: (x) => { x + outer }
        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0), // x
                Instruction::new(VMOpCode::LoadScope, 0), // outer (names[0])
                Instruction::new(VMOpCode::Add, 0),
                Instruction::new(VMOpCode::Return, 0),
            ],
            constants: vec![],
            names: vec!["outer".to_string()],
            nargs: 1,
            complexity: 4,
            is_pure: false,
        };

        let mut scope_slots = HashMap::new();
        scope_slots.insert("outer".to_string(), 5);

        let ops = inliner.bytecode_to_trace_ops(&info, &scope_slots, 0).unwrap();

        // Terminal Return dropped (no Exit): result left on the stack.
        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[0], TraceOp::LoadLocal(0)));
        assert!(matches!(ops[1], TraceOp::LoadLocal(5))); // outer → slot 5 (host slot, unshifted)
        assert!(matches!(ops[2], TraceOp::AddInt));
    }

    #[test]
    fn test_load_scope_unknown_var_rejected() {
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);

        let info = JitFunctionInfo {
            instructions: vec![Instruction::new(VMOpCode::LoadScope, 0)],
            constants: vec![],
            names: vec!["unknown_var".to_string()],
            nargs: 0,
            complexity: 1,
            is_pure: false,
        };

        let empty_scope = HashMap::new();
        let result = inliner.bytecode_to_trace_ops(&info, &empty_scope, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown_var"));
    }

    #[test]
    fn test_inline_closure_with_scope() {
        let mut registry = PureFunctionRegistry::new();

        // closure: (x) => { x + outer }
        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0), // x
                Instruction::new(VMOpCode::LoadScope, 0), // outer
                Instruction::new(VMOpCode::Add, 0),
                Instruction::new(VMOpCode::Return, 0),
            ],
            constants: vec![],
            names: vec!["outer".to_string()],
            nargs: 1,
            complexity: 4,
            is_pure: true,
        };

        registry.register("use_outer".to_string(), std::sync::Arc::new(info));

        let mut inliner = PureInliner::new(InliningConfig::default(), &registry);

        let mut trace = Trace::new(0);
        // outer is guarded at slot 3
        trace.name_guards.push(("outer".to_string(), 100, 3));
        trace.ops = vec![
            TraceOp::LoadConstInt(42),
            TraceOp::CallPure {
                func_id: "use_outer".to_string(),
                num_args: 1,
            },
        ];

        inliner.optimize(&mut trace).unwrap();

        // outer is guarded at slot 3, so callee locals are remapped above it
        // (scratch base = 4). The scope slot 3 itself stays unshifted. The
        // terminal Return is dropped (no Exit): the result stays on the stack.
        // Expected: LoadConstInt(42), StoreLocal(4), LoadLocal(4), LoadLocal(3), AddInt
        assert_eq!(trace.ops.len(), 5);
        assert!(matches!(trace.ops[0], TraceOp::LoadConstInt(42)));
        assert!(matches!(trace.ops[1], TraceOp::StoreLocal(4)));
        assert!(matches!(trace.ops[2], TraceOp::LoadLocal(4)));
        assert!(matches!(trace.ops[3], TraceOp::LoadLocal(3))); // outer → slot 3 (unshifted)
        assert!(matches!(trace.ops[4], TraceOp::AddInt));
        assert!(!trace.ops.iter().any(|op| matches!(op, TraceOp::Exit)));
        // Scratch slot registered for sizing
        assert!(trace.locals_used.contains(&4));
    }

    #[test]
    fn test_expand_builtin_abs() {
        let ops = PureInliner::expand_builtin("abs", 1);
        assert!(ops.is_some());
        let ops = ops.unwrap();
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], TraceOp::AbsInt));
    }

    #[test]
    fn test_expand_builtin_min_max() {
        let min_ops = PureInliner::expand_builtin("min", 2).unwrap();
        assert_eq!(min_ops.len(), 1);
        assert!(matches!(min_ops[0], TraceOp::MinInt));

        let max_ops = PureInliner::expand_builtin("max", 2).unwrap();
        assert_eq!(max_ops.len(), 1);
        assert!(matches!(max_ops[0], TraceOp::MaxInt));
    }

    #[test]
    fn test_expand_builtin_wrong_arity() {
        assert!(PureInliner::expand_builtin("abs", 2).is_none());
        assert!(PureInliner::expand_builtin("min", 1).is_none());
        assert!(PureInliner::expand_builtin("unknown", 1).is_none());
    }

    #[test]
    fn test_expand_builtin_round_int_bool() {
        let round_ops = PureInliner::expand_builtin("round", 1).unwrap();
        assert_eq!(round_ops.len(), 1);
        assert!(matches!(round_ops[0], TraceOp::RoundInt));

        let int_ops = PureInliner::expand_builtin("int", 1).unwrap();
        assert_eq!(int_ops.len(), 1);
        assert!(matches!(int_ops[0], TraceOp::IntCastInt));

        let bool_ops = PureInliner::expand_builtin("bool", 1).unwrap();
        assert_eq!(bool_ops.len(), 1);
        assert!(matches!(bool_ops[0], TraceOp::BoolInt));
    }

    #[test]
    fn test_optimize_expands_callpure_builtins() {
        let registry = PureFunctionRegistry::new();
        let mut inliner = PureInliner::new(InliningConfig::default(), &registry);

        let mut trace = Trace::new(0);
        trace.ops = vec![
            TraceOp::LoadConstInt(5),
            TraceOp::CallPure {
                func_id: "abs".to_string(),
                num_args: 1,
            },
            TraceOp::LoadConstInt(3),
            TraceOp::LoadConstInt(7),
            TraceOp::CallPure {
                func_id: "min".to_string(),
                num_args: 2,
            },
        ];

        inliner.optimize(&mut trace).unwrap();

        // abs(5) -> LoadConstInt(5), AbsInt
        // min(3, 7) -> LoadConstInt(3), LoadConstInt(7), MinInt
        assert_eq!(trace.ops.len(), 5);
        assert!(matches!(trace.ops[0], TraceOp::LoadConstInt(5)));
        assert!(matches!(trace.ops[1], TraceOp::AbsInt));
        assert!(matches!(trace.ops[2], TraceOp::LoadConstInt(3)));
        assert!(matches!(trace.ops[3], TraceOp::LoadConstInt(7)));
        assert!(matches!(trace.ops[4], TraceOp::MinInt));
    }

    #[test]
    fn test_inline_arg_order_two_params() {
        // f(a, b) = a - b. The stack is [a, b] with b on top; binding must put a
        // in slot 0 and b in slot 1, not reversed -- else a - b becomes b - a.
        let mut registry = PureFunctionRegistry::new();
        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0), // a
                Instruction::new(VMOpCode::LoadLocal, 1), // b
                Instruction::new(VMOpCode::Sub, 0),       // a - b
                Instruction::new(VMOpCode::Return, 0),
            ],
            constants: vec![],
            names: vec![],
            nargs: 2,
            complexity: 4,
            is_pure: true,
        };
        registry.register("sub".to_string(), std::sync::Arc::new(info));
        let mut inliner = PureInliner::new(InliningConfig::default(), &registry);

        let mut trace = Trace::new(0);
        trace.ops = vec![
            TraceOp::LoadConstInt(10), // a
            TraceOp::LoadConstInt(3),  // b (top of stack)
            TraceOp::CallPure {
                func_id: "sub".to_string(),
                num_args: 2,
            },
        ];
        inliner.optimize(&mut trace).unwrap();

        assert!(!trace.ops.iter().any(|op| matches!(op, TraceOp::CallPure { .. })));
        // Scratch base is 0 (empty host frame). Reversed binding: StoreLocal(1)
        // pops b, then StoreLocal(0) pops a -> a in slot 0, b in slot 1.
        let stores: Vec<usize> = trace
            .ops
            .iter()
            .filter_map(|op| match op {
                TraceOp::StoreLocal(s) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(stores, vec![1, 0]);
        // Body reads slot 0 then slot 1: a - b (not b - a).
        let loads: Vec<usize> = trace
            .ops
            .iter()
            .filter_map(|op| match op {
                TraceOp::LoadLocal(s) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(loads, vec![0, 1]);
    }

    #[test]
    fn test_non_terminal_return_refused() {
        // A Return before the end is an early return; the linear inliner cannot
        // model the branch, so it must refuse (call stays an interpreted CallPure).
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);
        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::Return, 0), // not last
                Instruction::new(VMOpCode::LoadConst, 0),
            ],
            constants: vec![JitConstant::Int(1)],
            names: vec![],
            nargs: 1,
            complexity: 3,
            is_pure: true,
        };
        let empty_scope = HashMap::new();
        let result = inliner.bytecode_to_trace_ops(&info, &empty_scope, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Return"));
    }

    #[test]
    fn test_block_scope_opcodes_skipped() {
        // A lambda body is always a block, so real callee bytecode carries
        // PushBlock/PopBlock. They only reset block-local slots on exit; for a
        // linear inlinable body every local is assigned before use, so they are
        // skipped (the computed result is unaffected).
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);
        let info = JitFunctionInfo {
            instructions: vec![
                Instruction::new(VMOpCode::PushBlock, 0),
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::LoadLocal, 0),
                Instruction::new(VMOpCode::Mul, 0),
                Instruction::new(VMOpCode::PopBlock, 0),
                Instruction::new(VMOpCode::Return, 0),
            ],
            constants: vec![],
            names: vec![],
            nargs: 1,
            complexity: 6,
            is_pure: true,
        };
        let empty_scope = HashMap::new();
        let ops = inliner.bytecode_to_trace_ops(&info, &empty_scope, 0).unwrap();
        // PushBlock/PopBlock skipped, terminal Return dropped: x * x.
        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[0], TraceOp::LoadLocal(0)));
        assert!(matches!(ops[1], TraceOp::LoadLocal(0)));
        assert!(matches!(ops[2], TraceOp::MulInt));
    }
}
