//! UnifiedCompiler: expression compilation (binary/unary, short-circuit, variables, control flow).

use super::*;

impl UnifiedCompiler {
    // ========== 7. Binary/Unary operations ==========

    pub(crate) fn compile_binary<'py>(
        &mut self,
        py: Python<'py>,
        vm_op: VMOpCode,
        args: &[CompilerNode<'py>],
    ) -> PyResult<()> {
        let (left, right) = if args.len() == 1 {
            let inner = &args[0];
            if inner.is_list_or_tuple() {
                (inner.child(py, 0)?, inner.child(py, 1)?)
            } else {
                return Err(pyo3::exceptions::PyValueError::new_err("Invalid binary args"));
            }
        } else if args.len() >= 2 {
            (args[0].clone(), args[1].clone())
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err("Binary op requires 2 args"));
        };
        self.compile_node(py, &left)?;
        self.compile_node(py, &right)?;
        self.emit(vm_op, 0);
        Ok(())
    }

    pub(crate) fn compile_unary<'py>(
        &mut self,
        py: Python<'py>,
        vm_op: VMOpCode,
        args: &[CompilerNode<'py>],
    ) -> PyResult<()> {
        if args.is_empty() {
            return Err(pyo3::exceptions::PyValueError::new_err("Unary op requires 1 arg"));
        }
        self.compile_node(py, &args[0])?;
        self.emit(vm_op, 0);
        Ok(())
    }

    // ========== 8. Short-circuit logic ==========

    pub(crate) fn compile_and<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let (left, right) = if args.len() == 1 && args[0].is_list_or_tuple() {
            (args[0].child(py, 0)?, args[0].child(py, 1)?)
        } else if args.len() >= 2 {
            (args[0].clone(), args[1].clone())
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err("And requires 2 operands"));
        };
        self.compile_node(py, &left)?;
        self.emit(VMOpCode::ToBool, 0);
        let jump_idx = self.emit(VMOpCode::JumpIfFalseOrPop, 0);
        self.compile_node(py, &right)?;
        self.emit(VMOpCode::ToBool, 0);
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    pub(crate) fn compile_or<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let (left, right) = if args.len() == 1 && args[0].is_list_or_tuple() {
            (args[0].child(py, 0)?, args[0].child(py, 1)?)
        } else if args.len() >= 2 {
            (args[0].clone(), args[1].clone())
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err("Or requires 2 operands"));
        };
        self.compile_node(py, &left)?;
        self.emit(VMOpCode::ToBool, 0);
        let jump_idx = self.emit(VMOpCode::JumpIfTrueOrPop, 0);
        self.compile_node(py, &right)?;
        self.emit(VMOpCode::ToBool, 0);
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    pub(crate) fn compile_null_coalesce<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let (left, right) = if args.len() == 1 && args[0].is_list_or_tuple() {
            (args[0].child(py, 0)?, args[0].child(py, 1)?)
        } else if args.len() >= 2 {
            (args[0].clone(), args[1].clone())
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "NullCoalesce requires 2 operands",
            ));
        };
        self.compile_node(py, &left)?;
        let jump_idx = self.emit(VMOpCode::JumpIfNotNoneOrPop, 0);
        self.compile_node(py, &right)?;
        let pos = self.instructions.len() as u32;
        self.patch(jump_idx, pos);
        Ok(())
    }

    // ========== 9. Variables (set_locals, getattr, etc.) ==========

    pub(crate) fn compile_set_locals<'py>(
        &mut self,
        py: Python<'py>,
        args: &[CompilerNode<'py>],
        kwargs: &CompilerKwargs<'py>,
    ) -> PyResult<()> {
        // Check if last arg is a boolean explicit_unpack flag
        let mut effective_args: Vec<CompilerNode<'py>> = args.to_vec();
        let mut explicit_unpack = false;
        if effective_args.len() >= 3 {
            if let Some(last) = effective_args.last() {
                if last.is_bool() {
                    explicit_unpack = last.as_bool().unwrap_or(false);
                    effective_args.pop();
                }
            }
        }

        // Detect format: kwargs['names'] or args[0] is tuple of names
        let names_pattern: Option<CompilerNode<'py>>;
        let values: Vec<CompilerNode<'py>>;

        if let Some(names_obj) = kwargs.get(py, "names")? {
            names_pattern = Some(names_obj);
            values = effective_args;
        } else if effective_args.len() >= 2 {
            if effective_args[0].is_tuple() {
                names_pattern = Some(effective_args[0].clone());
                values = effective_args.into_iter().skip(1).collect();
            } else {
                names_pattern = None;
                values = Vec::new();
            }
        } else {
            names_pattern = None;
            values = Vec::new();
        }

        // Capture void_context, then disable for sub-expressions
        let is_void = self.void_context;
        self.void_context = false;

        // Check for complex patterns (star, nested) -> VM pattern matching path
        if let Some(ref pattern) = names_pattern {
            if pattern.has_complex_pattern(py) && values.len() == 1 {
                let unwrapped = pattern.unwrap_single_tuple(py)?;

                let vm_pattern = self.try_compile_assign_pattern(py, &unwrapped)?.ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                        "Unsupported complex assignment pattern in VM compiler",
                    )
                })?;

                let pat_idx = self.patterns.len();
                self.patterns.push(vm_pattern);

                self.compile_node(py, &values[0])?;
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::MatchAssignPatternVM, pat_idx as u32);
                self.emit(VMOpCode::BindMatch, 0);

                // Sync bound names to scope where needed
                let names_to_sync = unwrapped.extract_names(py);
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

        // Extract flat names
        let names: Vec<String> = if let Some(ref pattern) = names_pattern {
            pattern.extract_names(py)
        } else {
            Vec::new()
        };

        if names.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        // Single name, single value: simple assignment (unless explicit_unpack)
        if names.len() == 1 && values.len() == 1 && !explicit_unpack {
            // Named lambda: let compile_lambda bind the name as a
            // self-reference in the closure (let-rec)
            let is_lambda_def = values[0].is_op(py, IROpCode::OpLambda);
            if is_lambda_def {
                self.pending_self_name = Some(names[0].clone());
            }
            // `@pure` compiles to `name = pure(lambda)`: mark the lambda's
            // CodeObject pure statically so the JIT records calls to it as
            // CallPure (inlining candidate). compile_lambda consumes the flag.
            if values[0].is_pure_decorated_lambda(py) {
                self.pending_pure = true;
            }
            self.compile_node(py, &values[0])?;
            self.pending_self_name = None;
            self.pending_pure = false;
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
            self.compile_node(py, &values[0])?;
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
                self.compile_node(py, &values[i])?;
            } else if !values.is_empty() {
                self.compile_node(py, values.last().unwrap())?;
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

    pub(crate) fn compile_getattr<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        self.compile_node(py, &args[0])?;
        let attr = args[1].as_string()?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::GetAttr, idx as u32);
        Ok(())
    }

    pub(crate) fn compile_setattr<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        self.compile_node(py, &args[0])?;
        self.compile_node(py, &args[2])?;
        let attr = args[1].as_string()?;
        let idx = self.add_name(&attr);
        self.emit(VMOpCode::SetAttr, idx as u32);
        Ok(())
    }

    pub(crate) fn compile_getitem<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        self.compile_node(py, &args[0])?;
        self.compile_node(py, &args[1])?;
        self.emit(VMOpCode::GetItem, 0);
        Ok(())
    }

    pub(crate) fn compile_setitem<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        self.compile_node(py, &args[0])?;
        self.compile_node(py, &args[1])?;
        self.compile_node(py, &args[2])?;
        self.emit(VMOpCode::SetItem, 0);
        Ok(())
    }

    pub(crate) fn compile_slice<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            self.compile_node(py, arg)?;
        }
        self.emit(VMOpCode::BuildSlice, args.len() as u32);
        Ok(())
    }

    // ========== 10. Control flow (if, while, for, block, body, return) ==========

    pub(crate) fn compile_if<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let branches_node = &args[0];
        let else_branch = if args.len() > 1 { Some(&args[1]) } else { None };

        let branches = branches_node.children(py)?;
        if branches.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut end_jumps = Vec::new();

        for branch in &branches {
            let len = branch.children_len(py)?;
            if len != 2 {
                continue;
            }
            let cond = branch.child(py, 0)?;
            let then_body = branch.child(py, 1)?;

            self.compile_node(py, &cond)?;
            let jump_to_next = self.emit(VMOpCode::JumpIfFalse, 0);
            self.compile_body(py, &then_body)?;
            end_jumps.push(self.emit(VMOpCode::Jump, 0));
            let pos = self.instructions.len() as u32;
            self.patch(jump_to_next, pos);
        }

        if let Some(else_body) = else_branch {
            self.compile_body(py, else_body)?;
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }

        let end_addr = self.instructions.len() as u32;
        for addr in end_jumps {
            self.patch(addr, end_addr);
        }
        Ok(())
    }

    pub(crate) fn compile_while<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let cond = &args[0];
        let body = &args[1];

        let loop_start = self.instructions.len();
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: Some(loop_start),
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        self.compile_node(py, cond)?;
        let jump_to_end = self.emit(VMOpCode::JumpIfFalse, 0);
        self.compile_body_void(py, body)?;

        if can_optimize {
            self.core.emit_loop_sync();
        }

        self.emit(VMOpCode::Jump, loop_start as u32);
        let ctx = self.loop_stack.pop().unwrap();

        let loop_end = self.instructions.len() as u32;
        if can_optimize {
            self.core.emit_loop_sync();
        }

        let loadconst_pos = self.instructions.len() as u32;
        let idx = self.core.add_const(Value::NIL);
        self.emit(VMOpCode::LoadConst, idx as u32);

        self.patch(jump_to_end, loop_end);
        let break_target = if can_optimize { loadconst_pos } else { loop_end };
        for addr in ctx.break_targets {
            self.patch(addr, break_target);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    pub(crate) fn compile_for<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let var_pattern = &args[0];
        let iterable = &args[1];
        let body = &args[2];

        let var_name = var_pattern.as_name(py);

        // Range optimization
        if let Some(var_name) = var_name.as_ref().filter(|_| iterable.is_range_call(py)) {
            return self.compile_for_range(py, var_name, iterable, body);
        }

        // Save/restore for existing loop variable
        let save_restore = if let Some(ref name) = var_name {
            if let Some(existing) = self.get_local_slot(name) {
                let temp = self.add_local(&format!("_for_save_{}", existing));
                self.emit(VMOpCode::LoadLocal, existing as u32);
                self.emit(VMOpCode::StoreLocal, temp as u32);
                Some((existing, temp))
            } else {
                None
            }
        } else {
            None
        };

        let slot_start = self.locals.len();
        self.emit(VMOpCode::PushBlock, slot_start as u32);
        self.compile_node(py, iterable)?;
        self.emit(VMOpCode::GetIter, 0);

        let loop_start = self.instructions.len();
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: Some(loop_start),
            continue_patches: Vec::new(),
            is_for_loop: true,
        });

        let for_iter_idx = self.emit(VMOpCode::ForIter, 0);

        // Store loop variable
        if let Some(ref name) = var_name {
            let slot = self.add_local(name);
            self.emit(VMOpCode::StoreLocal, slot as u32);
        } else {
            // Pattern unpacking
            self.compile_unpack_pattern(py, var_pattern, false)?;
        }

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        self.compile_body_void(py, body)?;

        if can_optimize {
            self.core.emit_loop_sync();
        }

        self.emit(VMOpCode::Jump, loop_start as u32);
        let ctx = self.loop_stack.pop().unwrap();

        let loop_end = self.instructions.len();
        if can_optimize {
            self.core.emit_loop_sync();
        }
        self.emit(VMOpCode::PopBlock, 0);

        if let Some((orig_slot, temp_slot)) = save_restore {
            self.emit(VMOpCode::LoadLocal, temp_slot as u32);
            self.emit(VMOpCode::StoreLocal, orig_slot as u32);
        }

        let idx = self.core.add_const(Value::NIL);
        self.emit(VMOpCode::LoadConst, idx as u32);

        self.patch(for_iter_idx, loop_end as u32);
        // Always use loop_end so break hits PopBlock + save_restore
        for addr in ctx.break_targets {
            self.patch(addr, loop_end as u32);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    pub(crate) fn compile_for_range<'py>(
        &mut self,
        py: Python<'py>,
        var_name: &str,
        range_call: &CompilerNode<'py>,
        body: &CompilerNode<'py>,
    ) -> PyResult<()> {
        let range_args = range_call.range_call_args(py)?;

        let (start, stop, step): (CompilerNode<'py>, CompilerNode<'py>, i64) = match range_args.len() {
            1 => {
                let zero = CompilerNode::Pure(&IR::Int(0));
                (zero, range_args[0].clone(), 1)
            }
            2 => (range_args[0].clone(), range_args[1].clone(), 1),
            _ => {
                let step = range_args[2]
                    .as_int()
                    .or_else(|_| {
                        range_args[2]
                            .try_extract_neg_literal(py)
                            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("non-literal step"))
                    })
                    .unwrap_or(1);
                (range_args[0].clone(), range_args[1].clone(), step)
            }
        };

        let step_is_positive = step > 0;

        let save_restore = if let Some(existing) = self.get_local_slot(var_name) {
            let temp = self.add_local(&format!("_for_save_{}", existing));
            self.emit(VMOpCode::LoadLocal, existing as u32);
            self.emit(VMOpCode::StoreLocal, temp as u32);
            Some((existing, temp))
        } else {
            None
        };

        let slot_start = self.locals.len();
        self.emit(VMOpCode::PushBlock, slot_start as u32);

        let slot_i = self.add_local(var_name);
        let nlocals = self.locals.len();
        let slot_stop = self.add_local(&format!("_range_stop_{}", nlocals));

        self.compile_node(py, &start)?;
        self.emit(VMOpCode::StoreLocal, slot_i as u32);
        self.compile_node(py, &stop)?;
        self.emit(VMOpCode::StoreLocal, slot_stop as u32);

        let loop_start = self.instructions.len();
        self.loop_stack.push(LoopContext {
            break_targets: Vec::new(),
            continue_target: None,
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(py, body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        let arg = CompilerCore::encode_for_range_args(slot_i, slot_stop, step_is_positive, 0);
        let for_range_idx = self.emit(VMOpCode::ForRangeInt, arg);

        self.compile_body_void(py, body)?;

        if can_optimize {
            self.core.emit_loop_sync();
        }

        let increment_addr = self.instructions.len();
        self.loop_stack.last_mut().unwrap().continue_target = Some(increment_addr);

        // Fused encoding fits when step is an i8 and the jump target fits the mask
        if i8::try_from(step).is_ok() && loop_start <= crate::vm::FOR_RANGE_STEP_JUMP_MASK as usize {
            let arg = CompilerCore::encode_for_range_step(slot_i, step, loop_start);
            self.emit(VMOpCode::ForRangeStep, arg);
        } else {
            self.emit(VMOpCode::LoadLocal, slot_i as u32);
            let step_idx = self.core.add_const(Value::from_i64(step));
            self.emit(VMOpCode::LoadConst, step_idx as u32);
            self.emit(VMOpCode::Add, 0);
            self.emit(VMOpCode::StoreLocal, slot_i as u32);
            self.emit(VMOpCode::Jump, loop_start as u32);
        }

        let ctx = self.loop_stack.pop().unwrap();

        for addr in &ctx.continue_patches {
            self.patch(*addr, increment_addr as u32);
        }

        let loop_end = self.instructions.len() as u32;
        self.emit(VMOpCode::PopBlock, 0);

        if let Some((orig_slot, temp_slot)) = save_restore {
            self.emit(VMOpCode::LoadLocal, temp_slot as u32);
            self.emit(VMOpCode::StoreLocal, orig_slot as u32);
        }

        if can_optimize {
            self.core.emit_loop_sync();
        }

        let idx = self.core.add_const(Value::NIL);
        self.emit(VMOpCode::LoadConst, idx as u32);

        let jump_offset = (loop_end as usize) - for_range_idx;
        let arg = CompilerCore::encode_for_range_args(slot_i, slot_stop, step_is_positive, jump_offset);
        self.patch(for_range_idx, arg);

        // Always use loop_end so break hits PopBlock + save_restore
        for addr in ctx.break_targets {
            self.patch(addr, loop_end);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    pub(crate) fn compile_block<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let slot_start = self.locals.len();
        let is_module_block = self.nesting_depth == 0;
        let push_arg = if is_module_block {
            slot_start as u32 | 0x8000_0000
        } else {
            slot_start as u32
        };
        self.emit(VMOpCode::PushBlock, push_arg);

        self.core.block_fn_defs.push(Vec::new());
        let len = args.len();
        for (i, item) in args.iter().enumerate() {
            self.compile_node(py, item)?;
            if i < len - 1 {
                self.emit(VMOpCode::PopTop, 0);
            }
        }
        self.core.block_fn_defs.pop();

        let pop_arg = if is_module_block { 1u32 } else { 0u32 };
        self.emit(VMOpCode::PopBlock, pop_arg);
        Ok(())
    }

    /// Compile body without PushBlock/PopBlock (for control structures).
    /// If body is an OpBlock, compile its contents inline.
    pub(crate) fn compile_body<'py>(&mut self, py: Python<'py>, body: &CompilerNode<'py>) -> PyResult<()> {
        if let Some(contents) = body.as_block_contents(py) {
            if contents.is_empty() {
                let idx = self.core.add_const(Value::NIL);
                self.emit(VMOpCode::LoadConst, idx as u32);
                return Ok(());
            }
            let len = contents.len();
            for (i, item) in contents.iter().enumerate() {
                let is_last = i == len - 1;
                // SetItem/SetAttr are truly void (push nothing).
                // SetLocals is NOT void here: it emits DupTop when void_context=false.
                let is_void = item.is_void_op(py);
                self.compile_node(py, item)?;
                if !is_last {
                    if !is_void {
                        self.emit(VMOpCode::PopTop, 0);
                    }
                } else if is_void {
                    // void op as last stmt: push NIL so compile_body
                    // always leaves exactly 1 value on the stack
                    let idx = self.core.add_const(Value::NIL);
                    self.emit(VMOpCode::LoadConst, idx as u32);
                }
            }
            return Ok(());
        }
        // Single node: check if void
        if body.is_void_op(py) {
            self.compile_node(py, body)?;
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }
        self.compile_node(py, body)
    }

    /// Compile body in void context: statements don't leave values on the stack.
    pub(crate) fn compile_body_void<'py>(&mut self, py: Python<'py>, body: &CompilerNode<'py>) -> PyResult<()> {
        if let Some(contents) = body.as_block_contents(py) {
            for stmt in &contents {
                let is_set_locals = stmt.is_set_locals(py);
                let is_void_op = stmt.is_void_op(py);

                if is_set_locals {
                    self.void_context = true;
                    self.compile_node(py, stmt)?;
                    self.void_context = false;
                } else if is_void_op {
                    self.compile_node(py, stmt)?;
                } else {
                    self.compile_node(py, stmt)?;
                    self.emit(VMOpCode::PopTop, 0);
                }
            }
            return Ok(());
        }
        // Not a block: compile single node
        let is_set_locals = body.is_set_locals(py);
        let is_void_op = body.is_void_op(py);
        if is_set_locals {
            self.void_context = true;
            self.compile_node(py, body)?;
            self.void_context = false;
        } else if is_void_op {
            self.compile_node(py, body)?;
        } else {
            self.compile_node(py, body)?;
            self.emit(VMOpCode::PopTop, 0);
        }
        Ok(())
    }

    pub(crate) fn compile_return<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if !args.is_empty() {
            self.compile_node(py, &args[0])?;
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        if self.core.finally_depth > 0 {
            let n = self.finally_stack.len();
            self.emit_finally_unwind(py)?;
            self.core.finally_depth += n;
        }
        self.emit(VMOpCode::Return, 0);
        Ok(())
    }
}
