// FILE: catnip_rs/src/jit/inliner.rs
//! Pure function inlining pass for JIT traces.

use std::collections::HashMap;

use super::registry::PureFunctionRegistry;
use super::trace::{Trace, TraceOp};
use crate::vm::frame::CodeObject;
use crate::vm::opcode::VMOpCode;

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
            max_inline_ops: 20,
            max_inline_depth: 2,
        }
    }
}

/// Pure function inliner for JIT traces.
pub struct PureInliner<'a> {
    config: InliningConfig,
    registry: &'a PureFunctionRegistry,
    depth: usize,
}

impl<'a> PureInliner<'a> {
    /// Create a new inliner with given configuration.
    pub fn new(config: InliningConfig, registry: &'a PureFunctionRegistry) -> Self {
        Self {
            config,
            registry,
            depth: 0,
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
        Ok(())
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
        if let Some(code) = self
            .registry
            .get_inlineable(func_id, self.config.max_inline_ops)
        {
            // Check argument count matches
            code.nargs == num_args
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
        let code = self
            .registry
            .get_inlineable(func_id, self.config.max_inline_ops)
            .ok_or_else(|| format!("Function {} not in registry", func_id))?;

        // Convert bytecode to TraceOps
        let body_ops = self.bytecode_to_trace_ops(code, scope_slots)?;

        // Bind arguments: pop args from stack to local slots
        // Stack: [..., arg0, arg1, ..., argN]
        // → Store each arg in corresponding local slot
        for i in 0..num_args {
            new_ops.push(TraceOp::StoreLocal(i));
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
        code: &CodeObject,
        scope_slots: &HashMap<String, usize>,
    ) -> Result<Vec<TraceOp>, String> {
        let mut ops = Vec::new();

        for instr in &code.instructions {
            let trace_op = match instr.op {
                VMOpCode::LoadConst => {
                    let val = code
                        .constants
                        .get(instr.arg as usize)
                        .ok_or("LoadConst: constant index out of bounds")?;
                    if let Some(i) = val.as_int() {
                        TraceOp::LoadConstInt(i)
                    } else if let Some(f) = val.as_float() {
                        TraceOp::LoadConstFloat(f)
                    } else {
                        return Err(format!(
                            "LoadConst: unsupported constant type for inlining: {:?}",
                            val
                        ));
                    }
                }
                VMOpCode::LoadLocal => TraceOp::LoadLocal(instr.arg as usize),
                VMOpCode::StoreLocal => TraceOp::StoreLocal(instr.arg as usize),
                VMOpCode::LoadScope => {
                    let name = code
                        .names
                        .get(instr.arg as usize)
                        .ok_or("LoadScope: name index out of bounds")?;
                    let slot = scope_slots.get(name.as_str()).ok_or_else(|| {
                        format!("LoadScope: captured var '{}' not in trace scope", name)
                    })?;
                    TraceOp::LoadLocal(*slot)
                }
                VMOpCode::Add => TraceOp::AddInt,
                VMOpCode::Sub => TraceOp::SubInt,
                VMOpCode::Mul => TraceOp::MulInt,
                VMOpCode::Div => TraceOp::DivInt,
                VMOpCode::Mod => TraceOp::ModInt,
                VMOpCode::Lt => TraceOp::LtInt,
                VMOpCode::Le => TraceOp::LeInt,
                VMOpCode::Gt => TraceOp::GtInt,
                VMOpCode::Ge => TraceOp::GeInt,
                VMOpCode::Eq => TraceOp::EqInt,
                VMOpCode::Ne => TraceOp::NeInt,
                VMOpCode::Return => TraceOp::Exit,
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
    use crate::vm::opcode::Instruction;
    use crate::vm::value::Value;

    #[test]
    fn test_inline_simple_pure() {
        let mut registry = PureFunctionRegistry::new();

        // Create simple pure function: f(x) = x + 1
        let mut code = CodeObject::new("add_one");
        code.is_pure = true;
        code.nargs = 1;
        code.instructions = vec![
            Instruction::new(VMOpCode::LoadLocal, 0), // x
            Instruction::new(VMOpCode::LoadConst, 0), // 1
            Instruction::new(VMOpCode::Add, 0),       // x + 1
            Instruction::new(VMOpCode::Return, 0),
        ];
        code.constants = vec![Value::from_int(1)];
        code.complexity = 4;

        registry.register("add_one".to_string(), code);

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
        let mut code = CodeObject::new("big_fn");
        code.is_pure = true;
        code.nargs = 1;
        code.complexity = 25;

        for _ in 0..25 {
            code.instructions.push(Instruction::new(VMOpCode::Nop, 0));
        }

        registry.register("big_fn".to_string(), code);

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
        let mut f = CodeObject::new("f");
        f.is_pure = true;
        f.nargs = 1;
        f.instructions = vec![
            Instruction::new(VMOpCode::LoadLocal, 0),
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::Add, 0),
            Instruction::new(VMOpCode::Return, 0),
        ];
        f.constants = vec![Value::from_int(1)];
        f.complexity = 4;

        registry.register("f".to_string(), f);

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

        let mut code = CodeObject::new("test");
        code.instructions = vec![
            Instruction::new(VMOpCode::LoadLocal, 0),
            Instruction::new(VMOpCode::LoadConst, 0),
            Instruction::new(VMOpCode::Add, 0),
        ];
        code.constants = vec![Value::from_int(10)];

        let empty_scope = HashMap::new();
        let ops = inliner.bytecode_to_trace_ops(&code, &empty_scope).unwrap();

        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[0], TraceOp::LoadLocal(0)));
        assert!(matches!(ops[1], TraceOp::LoadConstInt(10)));
        assert!(matches!(ops[2], TraceOp::AddInt));
    }

    #[test]
    fn test_unsupported_opcode() {
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);

        let mut code = CodeObject::new("test");
        code.instructions = vec![Instruction::new(VMOpCode::GetAttr, 0)];

        let empty_scope = HashMap::new();
        let result = inliner.bytecode_to_trace_ops(&code, &empty_scope);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_scope_with_known_var() {
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);

        // closure: (x) => { x + outer }
        let mut code = CodeObject::new("closure");
        code.instructions = vec![
            Instruction::new(VMOpCode::LoadLocal, 0), // x
            Instruction::new(VMOpCode::LoadScope, 0), // outer (names[0])
            Instruction::new(VMOpCode::Add, 0),
            Instruction::new(VMOpCode::Return, 0),
        ];
        code.names = vec!["outer".to_string()];

        let mut scope_slots = HashMap::new();
        scope_slots.insert("outer".to_string(), 5);

        let ops = inliner.bytecode_to_trace_ops(&code, &scope_slots).unwrap();

        assert_eq!(ops.len(), 4);
        assert!(matches!(ops[0], TraceOp::LoadLocal(0)));
        assert!(matches!(ops[1], TraceOp::LoadLocal(5))); // outer → slot 5
        assert!(matches!(ops[2], TraceOp::AddInt));
        assert!(matches!(ops[3], TraceOp::Exit));
    }

    #[test]
    fn test_load_scope_unknown_var_rejected() {
        let registry = PureFunctionRegistry::new();
        let inliner = PureInliner::new(InliningConfig::default(), &registry);

        let mut code = CodeObject::new("closure");
        code.instructions = vec![Instruction::new(VMOpCode::LoadScope, 0)];
        code.names = vec!["unknown_var".to_string()];

        let empty_scope = HashMap::new();
        let result = inliner.bytecode_to_trace_ops(&code, &empty_scope);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown_var"));
    }

    #[test]
    fn test_inline_closure_with_scope() {
        let mut registry = PureFunctionRegistry::new();

        // closure: (x) => { x + outer }
        let mut code = CodeObject::new("use_outer");
        code.is_pure = true;
        code.nargs = 1;
        code.instructions = vec![
            Instruction::new(VMOpCode::LoadLocal, 0), // x
            Instruction::new(VMOpCode::LoadScope, 0), // outer
            Instruction::new(VMOpCode::Add, 0),
            Instruction::new(VMOpCode::Return, 0),
        ];
        code.names = vec!["outer".to_string()];
        code.complexity = 4;

        registry.register("use_outer".to_string(), code);

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

        // Expected: LoadConstInt(42), StoreLocal(0), LoadLocal(0), LoadLocal(3), AddInt, Exit
        assert_eq!(trace.ops.len(), 6);
        assert!(matches!(trace.ops[0], TraceOp::LoadConstInt(42)));
        assert!(matches!(trace.ops[1], TraceOp::StoreLocal(0)));
        assert!(matches!(trace.ops[2], TraceOp::LoadLocal(0)));
        assert!(matches!(trace.ops[3], TraceOp::LoadLocal(3))); // outer → slot 3
        assert!(matches!(trace.ops[4], TraceOp::AddInt));
        assert!(matches!(trace.ops[5], TraceOp::Exit));
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
}
