// FILE: catnip_vm/src/compiler/expr.rs
use super::*;

/// True when `value` is the IR shape produced by `@pure` on a function:
/// `pure(lambda)` — a Call to the `pure` decorator with a single lambda arg.
/// (Stacked decorators like `@pure @jit` wrap the lambda in another Call and
/// are not matched; only a directly-`@pure`-decorated lambda is.)
fn is_pure_decorated_lambda(value: &IR) -> bool {
    if let IR::Call { func, args, .. } = value {
        if let IR::Ref(name, ..) = func.as_ref() {
            return name == "pure"
                && args.len() == 1
                && matches!(
                    args[0],
                    IR::Op {
                        opcode: IROpCode::OpLambda,
                        ..
                    }
                );
        }
    }
    false
}

impl PureCompiler {
    // ========== compile_node ==========

    pub(crate) fn compile_node(&mut self, ir: &IR) -> CompileResult<()> {
        match ir {
            // Literals
            IR::Int(n) => {
                let idx = self.core.add_const(Value::from_i64(*n));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Float(f) => {
                let idx = self.core.add_const(Value::from_float(*f));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Bool(b) => {
                let idx = self.core.add_const(Value::from_bool(*b));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::None => {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::String(s) => {
                let idx = self.core.add_const(Value::from_string(s.clone()));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Bytes(v) => {
                let idx = self.core.add_const(Value::from_bytes(v.clone()));
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
            IR::Decimal(s) => Err(CompileError::UnsupportedLiteral(format!(
                "Decimal literals not supported in standalone mode: {}",
                s
            ))),
            IR::Imaginary(s) => {
                let imag: f64 = s
                    .parse()
                    .map_err(|_| CompileError::SyntaxError(format!("invalid imaginary literal: {}j", s)))?;
                let val = Value::from_complex(0.0, imag);
                let idx = self.add_const(val);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }

            // Variables
            IR::Ref(name, start_byte, _end_byte) => {
                if *start_byte >= 0 {
                    self.core.current_start_byte = *start_byte as u32;
                }
                self.core.compile_name_load(name);
                Ok(())
            }
            IR::Identifier(name) => {
                self.core.compile_name_load(name);
                Ok(())
            }

            // Sequences
            IR::Program(items) => self.compile_statement_list(items),
            IR::List(items) => {
                for item in items {
                    self.compile_node(item)?;
                }
                self.emit(VMOpCode::BuildList, items.len() as u32);
                Ok(())
            }
            IR::Tuple(items) => {
                for item in items {
                    self.compile_node(item)?;
                }
                self.emit(VMOpCode::BuildTuple, items.len() as u32);
                Ok(())
            }
            IR::Set(items) => {
                for item in items {
                    self.compile_node(item)?;
                }
                self.emit(VMOpCode::BuildSet, items.len() as u32);
                Ok(())
            }
            IR::Dict(pairs) => {
                for (key, value) in pairs {
                    self.compile_node(key)?;
                    self.compile_node(value)?;
                }
                self.emit(VMOpCode::BuildDict, pairs.len() as u32);
                Ok(())
            }

            // Function call
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                tail,
                ..
            } => {
                if *start_byte > 0 {
                    self.core.current_start_byte = *start_byte as u32;
                }
                if *tail {
                    let mut all_args = vec![func.as_ref()];
                    all_args.extend(args.iter());
                    self.compile_call_from_args(&all_args, kwargs, true)
                } else {
                    self.compile_call_dispatch(func, args, kwargs)
                }
            }

            // Operations
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                ..
            } => {
                if *start_byte > 0 {
                    self.core.current_start_byte = *start_byte as u32;
                }
                self.compile_op_dispatch(*opcode, args, kwargs, *tail)
            }

            // Broadcasting
            IR::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => {
                if let Some(t) = target.as_deref() {
                    self.compile_node(t)?;
                }
                let nd_flag = match operator.as_ref() {
                    IR::Op { opcode, .. } if *opcode == IROpCode::NdRecursion => Some(4u32),
                    IR::Op { opcode, .. } if *opcode == IROpCode::NdMap => Some(8u32),
                    _ => None,
                };
                if let Some(nd_flag) = nd_flag {
                    if let IR::Op { args, .. } = operator.as_ref() {
                        if !args.is_empty() {
                            self.compile_node(&args[0])?;
                        }
                    }
                    let is_filter = matches!(broadcast_type, BroadcastType::If);
                    let mut flags = nd_flag;
                    if is_filter {
                        flags |= 1;
                    }
                    self.emit(VMOpCode::Broadcast, flags);
                } else {
                    self.compile_node(operator)?;
                    let has_operand = operand.is_some();
                    if let Some(o) = operand.as_deref() {
                        self.compile_node(o)?;
                    }
                    let is_filter = matches!(broadcast_type, BroadcastType::If);
                    let mut flags = 0u32;
                    if is_filter {
                        flags |= 1;
                    }
                    if has_operand {
                        flags |= 2;
                    }
                    self.emit(VMOpCode::Broadcast, flags);
                }
                Ok(())
            }

            // Slice
            IR::Slice { start, stop, step } => {
                self.compile_node(start)?;
                self.compile_node(stop)?;
                self.compile_node(step)?;
                self.emit(VMOpCode::BuildSlice, 3);
                Ok(())
            }

            // Patterns only appear inside match cases
            IR::PatternLiteral(_)
            | IR::PatternVar(_)
            | IR::PatternWildcard
            | IR::PatternOr(_)
            | IR::PatternTuple(_)
            | IR::PatternStruct { .. }
            | IR::PatternEnum { .. } => {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }
        }
    }

    // ========== Statement list ==========

    fn compile_statement_list(&mut self, stmts: &[IR]) -> CompileResult<()> {
        if stmts.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }
        let len = stmts.len();
        for (i, stmt) in stmts.iter().enumerate() {
            let is_void = is_op_ir(stmt, IROpCode::SetItem) || is_op_ir(stmt, IROpCode::SetAttr);
            self.compile_node(stmt)?;
            if i < len - 1 {
                if !is_void {
                    self.emit(VMOpCode::PopTop, 0);
                }
            } else if is_void {
                // Void op as last stmt: push NIL
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
            }
        }
        Ok(())
    }

    // ========== Op dispatch ==========

    fn compile_op_dispatch(
        &mut self,
        opcode: IROpCode,
        args: &[IR],
        kwargs: &IndexMap<String, IR>,
        tail: bool,
    ) -> CompileResult<()> {
        match opcode {
            // Arithmetic
            IROpCode::Add => self.compile_binary(VMOpCode::Add, args),
            // Typed arithmetic (TH4 canal A): analyzer-rewritten Add on proven types.
            IROpCode::AddInt => self.compile_binary(VMOpCode::AddInt, args),
            IROpCode::AddFloat => self.compile_binary(VMOpCode::AddFloat, args),
            IROpCode::SubInt => self.compile_binary(VMOpCode::SubInt, args),
            IROpCode::SubFloat => self.compile_binary(VMOpCode::SubFloat, args),
            IROpCode::MulInt => self.compile_binary(VMOpCode::MulInt, args),
            IROpCode::MulFloat => self.compile_binary(VMOpCode::MulFloat, args),
            IROpCode::DivFloat => self.compile_binary(VMOpCode::DivFloat, args),

            // FT2-A: enforce a declared-callback return on the caller side.
            // args[0] = the wrapped call, args[1] = the return annotation text;
            // lowered to the matching boundary opcode on the call's result
            // (same classification as a param prologue, no dedicated opcode).
            IROpCode::CheckReturn => {
                use catnip_core::vm::opcode::ParamCheck;
                let call = args
                    .first()
                    .ok_or_else(|| CompileError::SyntaxError("CheckReturn without a call".into()))?;
                self.compile_node(call)?;
                let annotation = match args.get(1) {
                    Some(IR::String(t)) => t.as_str(),
                    _ => return Err(CompileError::SyntaxError("CheckReturn without an annotation".into())),
                };
                self.emit_check_opcode(&ParamCheck::from_annotation(annotation));
                Ok(())
            }
            IROpCode::Sub => self.compile_binary(VMOpCode::Sub, args),
            IROpCode::Mul => self.compile_binary(VMOpCode::Mul, args),
            IROpCode::Div | IROpCode::TrueDiv => self.compile_binary(VMOpCode::Div, args),
            IROpCode::FloorDiv => self.compile_binary(VMOpCode::FloorDiv, args),
            IROpCode::Mod => self.compile_binary(VMOpCode::Mod, args),
            IROpCode::Pow => self.compile_binary(VMOpCode::Pow, args),
            IROpCode::Neg => self.compile_unary(VMOpCode::Neg, args),
            IROpCode::Pos => self.compile_unary(VMOpCode::Pos, args),

            // Comparison
            IROpCode::Lt => self.compile_binary(VMOpCode::Lt, args),
            IROpCode::Le => self.compile_binary(VMOpCode::Le, args),
            IROpCode::Gt => self.compile_binary(VMOpCode::Gt, args),
            IROpCode::Ge => self.compile_binary(VMOpCode::Ge, args),
            IROpCode::Eq => self.compile_binary(VMOpCode::Eq, args),
            IROpCode::Ne => self.compile_binary(VMOpCode::Ne, args),

            // Membership
            IROpCode::In => self.compile_binary(VMOpCode::In, args),
            IROpCode::NotIn => self.compile_binary(VMOpCode::NotIn, args),

            // Identity
            IROpCode::Is => self.compile_binary(VMOpCode::Is, args),
            IROpCode::IsNot => self.compile_binary(VMOpCode::IsNot, args),

            // Logical
            IROpCode::Not => self.compile_unary(VMOpCode::Not, args),
            IROpCode::And => self.compile_and(args),
            IROpCode::Or => self.compile_or(args),
            IROpCode::NullCoalesce => self.compile_null_coalesce(args),

            // Bitwise
            IROpCode::BAnd => self.compile_binary(VMOpCode::BAnd, args),
            IROpCode::BOr => self.compile_binary(VMOpCode::BOr, args),
            IROpCode::BXor => self.compile_binary(VMOpCode::BXor, args),
            IROpCode::BNot => self.compile_unary(VMOpCode::BNot, args),
            IROpCode::LShift => self.compile_binary(VMOpCode::LShift, args),
            IROpCode::RShift => self.compile_binary(VMOpCode::RShift, args),

            // Variables
            IROpCode::SetLocals => self.compile_set_locals(args, kwargs),
            IROpCode::GetAttr => self.compile_getattr(args),
            IROpCode::SetAttr => self.compile_setattr(args),
            IROpCode::GetItem => self.compile_getitem(args),
            IROpCode::SetItem => self.compile_setitem(args),
            IROpCode::Slice => self.compile_slice(args),

            // Control flow
            IROpCode::OpIf => self.compile_if(args),
            IROpCode::OpWhile => self.compile_while(args),
            IROpCode::OpFor => self.compile_for(args),
            IROpCode::OpBlock => self.compile_block(args),
            IROpCode::OpReturn => self.compile_return(args),
            IROpCode::OpBreak => self.compile_break_with_finally(),
            IROpCode::OpContinue => self.compile_continue_with_finally(),

            // Functions
            IROpCode::Call => self.compile_call_op(args, kwargs, tail),
            IROpCode::OpLambda => self.compile_lambda(args),
            IROpCode::FnDef => self.compile_fn_def(args),

            // Collections
            IROpCode::ListLiteral => self.compile_collection(VMOpCode::BuildList, args),
            IROpCode::TupleLiteral => self.compile_collection(VMOpCode::BuildTuple, args),
            IROpCode::SetLiteral => self.compile_collection(VMOpCode::BuildSet, args),
            IROpCode::DictLiteral => self.compile_dict_op(args),

            // String
            IROpCode::Fstring => self.compile_fstring(args),

            // Match
            IROpCode::OpMatch => self.compile_match(args),

            // Broadcasting
            IROpCode::Broadcast => self.compile_broadcast_op(args),

            // ND operations
            IROpCode::NdEmptyTopos => {
                self.emit(VMOpCode::NdEmptyTopos, 0);
                Ok(())
            }
            IROpCode::NdRecursion => self.compile_nd_recursion(args),
            IROpCode::NdMap => self.compile_nd_map(args),

            // Stack ops
            IROpCode::Push => {
                if !args.is_empty() {
                    self.compile_node(&args[0])
                } else {
                    Ok(())
                }
            }
            IROpCode::Pop => {
                self.emit(VMOpCode::PopTop, 0);
                Ok(())
            }
            IROpCode::Nop => {
                self.emit(VMOpCode::Nop, 0);
                Ok(())
            }

            IROpCode::Pragma => {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }

            IROpCode::Breakpoint => {
                self.emit(VMOpCode::Breakpoint, 0);
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                Ok(())
            }

            IROpCode::TypeOf => {
                if !args.is_empty() {
                    self.compile_node(&args[0])?;
                }
                self.emit(VMOpCode::TypeOf, 0);
                Ok(())
            }

            IROpCode::Globals => {
                self.emit(VMOpCode::Globals, 0);
                Ok(())
            }

            IROpCode::Locals => {
                self.emit(VMOpCode::Locals, 0);
                Ok(())
            }

            IROpCode::OpStruct => self.compile_struct(args),
            IROpCode::TraitDef => self.compile_trait(args),
            IROpCode::EnumDef => self.compile_enum(args),
            IROpCode::UnionDef => self.compile_union(args),

            // Error handling
            IROpCode::OpTry => self.compile_try(args),
            IROpCode::OpRaise => self.compile_raise(args),

            // Exception info (for with desugaring)
            IROpCode::ExcInfo => {
                self.emit(VMOpCode::LoadException, 1);
                Ok(())
            }

            _ => Err(CompileError::NotImplemented(format!(
                "PureCompiler: cannot compile IR opcode: {}",
                opcode
            ))),
        }
    }

    // ========== Binary/Unary operations ==========

    fn compile_binary(&mut self, vm_op: VMOpCode, args: &[IR]) -> CompileResult<()> {
        let (left, right) = if args.len() == 1 {
            match &args[0] {
                IR::List(items) | IR::Tuple(items) if items.len() >= 2 => (&items[0], &items[1]),
                _ => return Err(CompileError::ValueError("Invalid binary args".to_string())),
            }
        } else if args.len() >= 2 {
            (&args[0], &args[1])
        } else {
            return Err(CompileError::ValueError("Binary op requires 2 args".to_string()));
        };
        self.compile_node(left)?;
        self.compile_node(right)?;
        self.emit(vm_op, 0);
        Ok(())
    }

    fn compile_unary(&mut self, vm_op: VMOpCode, args: &[IR]) -> CompileResult<()> {
        if args.is_empty() {
            return Err(CompileError::ValueError("Unary op requires 1 arg".to_string()));
        }
        self.compile_node(&args[0])?;
        self.emit(vm_op, 0);
        Ok(())
    }

    // ========== Short-circuit logic ==========

    fn unwrap_binary_args<'a>(&self, args: &'a [IR]) -> CompileResult<(&'a IR, &'a IR)> {
        // Exactly two operands: reject extras instead of silently dropping them.
        if args.len() == 1 {
            match &args[0] {
                IR::List(items) | IR::Tuple(items) if items.len() == 2 => Ok((&items[0], &items[1])),
                _ => Err(CompileError::ValueError("requires 2 operands".to_string())),
            }
        } else if args.len() == 2 {
            Ok((&args[0], &args[1]))
        } else {
            Err(CompileError::ValueError("requires 2 operands".to_string()))
        }
    }

    fn compile_and(&mut self, args: &[IR]) -> CompileResult<()> {
        let (left, right) = self.unwrap_binary_args(args)?;
        self.compile_node(left)?;
        self.emit(VMOpCode::ToBool, 0);
        let jump_idx = self.emit(VMOpCode::JumpIfFalseOrPop, 0);
        self.compile_node(right)?;
        self.emit(VMOpCode::ToBool, 0);
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    fn compile_or(&mut self, args: &[IR]) -> CompileResult<()> {
        let (left, right) = self.unwrap_binary_args(args)?;
        self.compile_node(left)?;
        self.emit(VMOpCode::ToBool, 0);
        let jump_idx = self.emit(VMOpCode::JumpIfTrueOrPop, 0);
        self.compile_node(right)?;
        self.emit(VMOpCode::ToBool, 0);
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    fn compile_null_coalesce(&mut self, args: &[IR]) -> CompileResult<()> {
        let (left, right) = self.unwrap_binary_args(args)?;
        self.compile_node(left)?;
        let jump_idx = self.emit(VMOpCode::JumpIfNotNoneOrPop, 0);
        self.compile_node(right)?;
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    // ========== Variables ==========

    fn compile_set_locals(&mut self, args: &[IR], kwargs: &IndexMap<String, IR>) -> CompileResult<()> {
        let mut effective_args: Vec<&IR> = args.iter().collect();
        let mut explicit_unpack = false;
        if effective_args.len() >= 3 {
            if let Some(IR::Bool(b)) = effective_args.last() {
                explicit_unpack = *b;
                effective_args.pop();
            }
        }

        let names_pattern: Option<&IR>;
        let values: Vec<&IR>;

        if let Some(names_ir) = kwargs.get("names") {
            names_pattern = Some(names_ir);
            values = effective_args;
        } else if effective_args.len() >= 2 {
            if matches!(effective_args[0], IR::Tuple(_)) {
                names_pattern = Some(effective_args[0]);
                values = effective_args.into_iter().skip(1).collect();
            } else {
                names_pattern = None;
                values = Vec::new();
            }
        } else {
            names_pattern = None;
            values = Vec::new();
        }

        let is_void = self.void_context;
        self.void_context = false;

        // Complex patterns (star, nested) -> VM pattern matching path
        if let Some(pattern) = names_pattern {
            if has_complex_pattern_ir(pattern) && values.len() == 1 {
                let unwrapped = unwrap_single_tuple(pattern);

                let vm_pattern = self
                    .try_compile_assign_pattern_ir(unwrapped)?
                    .ok_or_else(|| CompileError::SyntaxError("Unsupported complex assignment pattern".to_string()))?;

                let pat_idx = self.patterns.len();
                self.patterns.push(vm_pattern);

                self.compile_node(values[0])?;
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::MatchAssignPatternVM, pat_idx as u32);
                self.emit(VMOpCode::BindMatch, 0);

                let names_to_sync = extract_names_ir(unwrapped);
                for name in names_to_sync {
                    let Some(slot) = self.locals.iter().position(|n| n == &name) else {
                        continue;
                    };
                    let needs_scope_sync = if self.nesting_depth == 0 {
                        true
                    } else {
                        self.outer_names.contains(&name)
                    };
                    if needs_scope_sync {
                        self.emit(VMOpCode::LoadLocal, slot as u32);
                        let name_idx = self.add_name(&name);
                        self.emit(VMOpCode::StoreScope, name_idx as u32);
                    }
                }

                if is_void {
                    self.emit(VMOpCode::PopTop, 0);
                }
                return Ok(());
            }
        }

        let names: Vec<String> = if let Some(pattern) = names_pattern {
            extract_names_ir(pattern)
        } else {
            Vec::new()
        };

        if names.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        // Single name, single value: simple assignment
        if names.len() == 1 && values.len() == 1 && !explicit_unpack {
            // Named lambda: let compile_lambda bind the name as a
            // self-reference in the closure (let-rec)
            let is_lambda_def = matches!(
                values[0],
                IR::Op {
                    opcode: IROpCode::OpLambda,
                    ..
                }
            );
            if is_lambda_def {
                self.core.pending_self_name = Some(names[0].clone());
            }
            // `@pure` compiles to `name = pure(lambda)`: mark the lambda's
            // CodeObject pure statically so the JIT records calls to it as
            // CallPure (inlining candidate). compile_lambda consumes the flag.
            if is_pure_decorated_lambda(values[0]) {
                self.core.pending_pure = true;
            }
            self.compile_node(values[0])?;
            self.core.pending_self_name = None;
            self.core.pending_pure = false;
            if !is_void {
                self.emit(VMOpCode::DupTop, 0);
            }
            self.emit_store(&names[0]);
            if is_lambda_def {
                self.register_letrec_def(&names[0]);
            }
            return Ok(());
        }

        // Multiple names OR explicit unpack, single value: unpacking
        if values.len() == 1 && (names.len() > 1 || explicit_unpack) {
            self.compile_node(values[0])?;
            self.emit(VMOpCode::UnpackSequence, names.len() as u32);
            for (i, name) in names.iter().enumerate() {
                let is_last = i == names.len() - 1;
                if is_last && !is_void {
                    self.emit(VMOpCode::DupTop, 0);
                }
                self.emit_store(name);
            }
            return Ok(());
        }

        // Multiple names, multiple values: parallel assignment
        for (i, name) in names.iter().enumerate() {
            if i < values.len() {
                self.compile_node(values[i])?;
            } else if !values.is_empty() {
                self.compile_node(values.last().unwrap())?;
            } else {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
            }
            let is_last = i == names.len() - 1;
            if is_last && !is_void {
                self.emit(VMOpCode::DupTop, 0);
            }
            self.emit_store(name);
        }
        Ok(())
    }

    fn compile_getattr(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "getattr")?;
        self.compile_node(&args[0])?;
        let attr = ir_to_name(&args[1]).ok_or_else(|| CompileError::TypeError("expected string".to_string()))?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::GetAttr, idx as u32);
        Ok(())
    }

    fn compile_setattr(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 3, "setattr")?;
        self.compile_node(&args[0])?;
        self.compile_node(&args[2])?;
        let attr = ir_to_name(&args[1]).ok_or_else(|| CompileError::TypeError("expected string".to_string()))?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::SetAttr, idx as u32);
        Ok(())
    }

    fn compile_getitem(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "getitem")?;
        self.compile_node(&args[0])?;
        // Fuse Slice + GetItem: push start/stop/step directly, emit GetItem(1)
        if let IR::Slice { start, stop, step } = &args[1] {
            self.compile_node(start)?;
            self.compile_node(stop)?;
            self.compile_node(step)?;
            self.emit(VMOpCode::GetItem, 1);
        } else {
            self.compile_node(&args[1])?;
            self.emit(VMOpCode::GetItem, 0);
        }
        Ok(())
    }

    fn compile_setitem(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 3, "setitem")?;
        self.compile_node(&args[0])?;
        self.compile_node(&args[1])?;
        self.compile_node(&args[2])?;
        self.emit(VMOpCode::SetItem, 0);
        Ok(())
    }

    fn compile_slice(&mut self, args: &[IR]) -> CompileResult<()> {
        for arg in args {
            self.compile_node(arg)?;
        }
        self.emit(VMOpCode::BuildSlice, args.len() as u32);
        Ok(())
    }
}
