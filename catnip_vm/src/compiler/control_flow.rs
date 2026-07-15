// FILE: catnip_vm/src/compiler/control_flow.rs
use super::*;

impl PureCompiler {
    // ========== Control flow ==========

    pub(crate) fn compile_if(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 1, "if")?;
        let branches = match &args[0] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &args[0..1],
        };
        let else_branch = if args.len() > 1 { Some(&args[1]) } else { None };

        if branches.is_empty() {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut end_jumps = Vec::new();

        for branch in branches {
            let items = match branch {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            if items.len() != 2 {
                continue;
            }
            let cond = &items[0];
            let then_body = &items[1];

            self.compile_node(cond)?;
            let jump_to_next = self.emit(VMOpCode::JumpIfFalse, 0);
            self.compile_body(then_body)?;
            // Always emit the merge jump. Eliding it when the last emitted
            // instruction is a Jump/Return is unsound: that instruction can
            // belong to ONE arm of a nested if (`if c2 { .. } else { break }`)
            // whose other arm falls through -- its own merge jump then lands
            // on the next emitted code, i.e. THIS if's else branch, running
            // both arms (found by the Phase 4 property harness). A truly
            // terminal branch leaves this jump unreachable and the peephole
            // dead-code pass removes it.
            end_jumps.push(self.emit(VMOpCode::Jump, 0));
            let pos = self.instructions.len() as u32;
            self.patch(jump_to_next, pos);
        }

        if let Some(else_body) = else_branch {
            self.compile_body(else_body)?;
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

    pub(crate) fn compile_while(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "while")?;
        let cond = &args[0];
        let body = &args[1];

        let loop_start = self.instructions.len();
        self.loop_stack.push(core::LoopContext {
            break_targets: Vec::new(),
            continue_target: Some(loop_start),
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        self.compile_node(cond)?;
        let jump_to_end = self.emit(VMOpCode::JumpIfFalse, 0);
        self.compile_body_void(body)?;

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

    pub(crate) fn compile_for(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 3, "for")?;
        let var_pattern = &args[0];
        let iterable = &args[1];
        let body = &args[2];

        let var_name = ir_to_name(var_pattern);

        // Range optimization
        if let Some(ref vn) = var_name {
            if is_range_call_ir(iterable) {
                return self.compile_for_range(vn, iterable, body);
            }
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
        self.compile_node(iterable)?;
        self.emit(VMOpCode::GetIter, 0);

        let loop_start = self.instructions.len();
        self.loop_stack.push(core::LoopContext {
            break_targets: Vec::new(),
            continue_target: Some(loop_start),
            continue_patches: Vec::new(),
            is_for_loop: true,
        });

        let for_iter_idx = self.emit(VMOpCode::ForIter, 0);

        if let Some(ref name) = var_name {
            let slot = self.add_local(name);
            self.emit(VMOpCode::StoreLocal, slot as u32);
        } else {
            self.compile_unpack_pattern_ir(var_pattern, false)?;
        }

        let can_optimize = self.nesting_depth == 0 && !self.body_has_calls(body);
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        self.compile_body_void(body)?;

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
        for addr in ctx.break_targets {
            self.patch(addr, loop_end as u32);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    fn compile_for_range(&mut self, var_name: &str, range_call: &IR, body: &IR) -> CompileResult<()> {
        let range_args =
            range_call_args_ir(range_call).ok_or_else(|| CompileError::ValueError("not a range call".to_string()))?;

        let (start, stop, step): (&IR, &IR, i64) = match range_args.len() {
            1 => (&IR::Int(0), &range_args[0], 1),
            2 => (&range_args[0], &range_args[1], 1),
            _ => {
                let step = if let IR::Int(n) = &range_args[2] {
                    *n
                } else {
                    try_extract_neg_literal_ir(&range_args[2]).unwrap_or(1)
                };
                (&range_args[0], &range_args[1], step)
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

        self.compile_node(start)?;
        self.emit(VMOpCode::StoreLocal, slot_i as u32);
        self.compile_node(stop)?;
        self.emit(VMOpCode::StoreLocal, slot_stop as u32);

        let loop_start = self.instructions.len();
        self.loop_stack.push(core::LoopContext {
            break_targets: Vec::new(),
            continue_target: None,
            continue_patches: Vec::new(),
            is_for_loop: false,
        });

        let has_calls = self.body_has_calls(body);
        let can_optimize = self.nesting_depth == 0 && !has_calls;
        let old_optimized = self.in_optimized_loop;
        let old_modified = std::mem::take(&mut self.loop_modified_vars);
        if can_optimize {
            self.in_optimized_loop = true;
        }

        let arg = CompilerCore::encode_for_range_args(slot_i, slot_stop, step_is_positive, 0);
        let for_range_idx = self.emit(VMOpCode::ForRangeInt, arg);

        self.compile_body_void(body)?;

        if can_optimize {
            self.core.emit_loop_sync();
        }

        let increment_addr = self.instructions.len();
        self.loop_stack.last_mut().unwrap().continue_target = Some(increment_addr);

        // Fused encoding fits when step is an i8 and the jump target fits the mask
        if i8::try_from(step).is_ok() && loop_start <= catnip_core::vm::FOR_RANGE_STEP_JUMP_MASK as usize {
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

        // Same convention as catnip_rs UnifiedCompiler:
        // jump_offset = loop_end - for_range_idx
        // (loop_end was computed before PopBlock/sync/LoadConst emissions)
        let jump_offset = (loop_end as usize) - for_range_idx;
        let arg = CompilerCore::encode_for_range_args(slot_i, slot_stop, step_is_positive, jump_offset);
        self.patch(for_range_idx, arg);

        for addr in ctx.break_targets {
            self.patch(addr, loop_end);
        }

        self.in_optimized_loop = old_optimized;
        self.loop_modified_vars = old_modified;
        Ok(())
    }

    pub(crate) fn compile_block(&mut self, args: &[IR]) -> CompileResult<()> {
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
            let is_void = is_op_ir(item, IROpCode::SetItem) || is_op_ir(item, IROpCode::SetAttr);
            self.compile_node(item)?;
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
        self.core.block_fn_defs.pop();

        let pop_arg = if is_module_block { 1u32 } else { 0u32 };
        self.emit(VMOpCode::PopBlock, pop_arg);
        Ok(())
    }

    fn compile_body(&mut self, body: &IR) -> CompileResult<()> {
        if let Some(contents) = as_block_contents_ir(body) {
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
                let is_void = is_op_ir(item, IROpCode::SetItem) || is_op_ir(item, IROpCode::SetAttr);
                self.compile_node(item)?;
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
        // Single node
        let is_void = is_op_ir(body, IROpCode::SetItem) || is_op_ir(body, IROpCode::SetAttr);
        if is_void {
            self.compile_node(body)?;
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }
        self.compile_node(body)
    }

    fn compile_body_void(&mut self, body: &IR) -> CompileResult<()> {
        if let Some(contents) = as_block_contents_ir(body) {
            for stmt in contents {
                let is_set_locals = is_op_ir(stmt, IROpCode::SetLocals);
                let is_void_op = is_op_ir(stmt, IROpCode::SetItem) || is_op_ir(stmt, IROpCode::SetAttr);

                if is_set_locals {
                    self.void_context = true;
                    self.compile_node(stmt)?;
                    self.void_context = false;
                } else if is_void_op {
                    self.compile_node(stmt)?;
                } else {
                    self.compile_node(stmt)?;
                    self.emit(VMOpCode::PopTop, 0);
                }
            }
            return Ok(());
        }
        let is_set_locals = is_op_ir(body, IROpCode::SetLocals);
        let is_void_op = is_op_ir(body, IROpCode::SetItem) || is_op_ir(body, IROpCode::SetAttr);
        if is_set_locals {
            self.void_context = true;
            self.compile_node(body)?;
            self.void_context = false;
        } else if is_void_op {
            self.compile_node(body)?;
        } else {
            self.compile_node(body)?;
            self.emit(VMOpCode::PopTop, 0);
        }
        Ok(())
    }

    pub(crate) fn compile_return(&mut self, args: &[IR]) -> CompileResult<()> {
        if !args.is_empty() {
            self.compile_node(&args[0])?;
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        // Inside try/finally: pop handlers, inline finally bodies, then return
        if self.core.finally_depth > 0 {
            let n = self.finally_stack.len();
            self.emit_finally_unwind()?;
            self.core.finally_depth += n;
        }
        self.emit(VMOpCode::Return, 0);
        Ok(())
    }
}
