// FILE: catnip_vm/src/compiler/collections.rs
use super::*;

impl PureCompiler {
    // ========== Collections ==========

    pub(crate) fn compile_collection(&mut self, vm_op: VMOpCode, args: &[IR]) -> CompileResult<()> {
        for arg in args {
            self.compile_node(arg)?;
        }
        self.emit(vm_op, args.len() as u32);
        Ok(())
    }

    pub(crate) fn compile_dict_op(&mut self, args: &[IR]) -> CompileResult<()> {
        for arg in args {
            let items = match arg {
                IR::Tuple(items) | IR::List(items) => items,
                _ => return Err(CompileError::TypeError("dict entry must be pair".to_string())),
            };
            if items.len() < 2 {
                return Err(CompileError::TypeError("dict entry must have 2 elements".to_string()));
            }
            self.compile_node(&items[0])?;
            self.compile_node(&items[1])?;
        }
        self.emit(VMOpCode::BuildDict, args.len() as u32);
        Ok(())
    }

    // ========== Broadcast ==========

    pub(crate) fn compile_broadcast_op(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.len() < 4 {
            return Err(CompileError::TypeError("Broadcast requires 4 arguments".to_string()));
        }
        self.compile_node(&args[0])?;
        self.compile_node(&args[1])?;

        let has_operand = !is_none_ir(&args[2]);
        if has_operand {
            self.compile_node(&args[2])?;
        }

        let is_filter = matches!(&args[3], IR::Bool(true));
        let mut flags = 0u32;
        if is_filter {
            flags |= 1;
        }
        if has_operand {
            flags |= 2;
        }
        self.emit(VMOpCode::Broadcast, flags);
        Ok(())
    }

    // ========== Match ==========

    pub(crate) fn compile_match(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "match")?;
        let value_expr = &args[0];
        let cases_ir = &args[1];

        // Pre-allocate slots for pattern variables
        if let IR::Tuple(items) | IR::List(items) = cases_ir {
            for case in items {
                if let IR::Tuple(case_parts) = case {
                    if !case_parts.is_empty() {
                        self.collect_pattern_vars_ir(&case_parts[0]);
                    }
                }
            }
        }

        self.compile_node(value_expr)?;

        let cases = match cases_ir {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => return Err(CompileError::TypeError("match cases must be a sequence".to_string())),
        };

        let mut end_jumps = Vec::new();

        for case in cases {
            let case_parts = match case {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            if case_parts.len() < 3 {
                continue;
            }
            let pattern = &case_parts[0];
            let guard = &case_parts[1];
            let body = &case_parts[2];

            self.emit(VMOpCode::DupTop, 0);

            let vm_pattern = self
                .try_compile_pattern_ir(pattern)?
                .ok_or_else(|| CompileError::NotImplemented(format!("unsupported match pattern: {:?}", pattern)))?;
            let pat_idx = self.patterns.len();
            self.patterns.push(vm_pattern);
            self.emit(VMOpCode::MatchPatternVM, pat_idx as u32);

            self.emit(VMOpCode::DupTop, 0);
            let skip_jump = self.emit(VMOpCode::JumpIfNone, 0);

            let guard_fail = if !is_none_ir(guard) {
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::PushBlock, 0);
                self.emit(VMOpCode::BindMatch, 0);
                self.compile_node(guard)?;
                self.emit(VMOpCode::PopBlock, 0);
                Some(self.emit(VMOpCode::JumpIfFalse, 0))
            } else {
                None
            };

            self.emit(VMOpCode::BindMatch, 0);
            self.emit(VMOpCode::PopTop, 0);
            self.compile_node(body)?;
            end_jumps.push(self.emit(VMOpCode::Jump, 0));

            let next_case = self.instructions.len();
            if let Some(guard_fail_addr) = guard_fail {
                self.patch(guard_fail_addr, next_case as u32);
                self.emit(VMOpCode::PopTop, 0);
                let guard_cleanup_done = self.emit(VMOpCode::Jump, 0);
                let skip_cleanup = self.instructions.len();
                self.patch(skip_jump, skip_cleanup as u32);
                self.emit(VMOpCode::PopTop, 0);
                let next_case_start = self.instructions.len();
                self.patch(guard_cleanup_done, next_case_start as u32);
            } else {
                let pos = self.instructions.len() as u32;
                self.patch(skip_jump, pos);
                self.emit(VMOpCode::PopTop, 0);
            }
        }

        // No match: pop value, raise error
        self.emit(VMOpCode::PopTop, 0);
        let msg_val = Value::from_string("No matching pattern".to_string());
        let msg_idx = self.core.add_const(msg_val);
        self.emit(VMOpCode::MatchFail, msg_idx as u32);

        let end_addr = self.instructions.len() as u32;
        for addr in end_jumps {
            self.patch(addr, end_addr);
        }
        Ok(())
    }

    // ========== ND operations ==========

    pub(crate) fn compile_nd_recursion(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.len() < 2 {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        } else if is_none_ir(&args[1]) {
            self.compile_node(&args[0])?;
            self.emit(VMOpCode::NdRecursion, 1);
        } else {
            self.compile_node(&args[0])?;
            self.compile_node(&args[1])?;
            self.emit(VMOpCode::NdRecursion, 0);
        }
        Ok(())
    }

    pub(crate) fn compile_nd_map(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.len() < 2 {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        } else if is_none_ir(&args[1]) {
            self.compile_node(&args[0])?;
            self.emit(VMOpCode::NdMap, 1);
        } else {
            self.compile_node(&args[0])?;
            self.compile_node(&args[1])?;
            self.emit(VMOpCode::NdMap, 0);
        }
        Ok(())
    }

    // ========== F-strings ==========

    pub(crate) fn compile_fstring(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.is_empty() {
            let idx = self.core.add_const(Value::from_string(String::new()));
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut n_parts: u32 = 0;

        for part in args {
            if let IR::String(text) = part {
                let idx = self.core.add_const(Value::from_string(text.clone()));
                self.emit(VMOpCode::LoadConst, idx as u32);
                n_parts += 1;
            } else if let IR::Tuple(items) = part {
                // Interpolation: (expr, conv, spec)
                let expr = &items[0];
                let conv = if let IR::Int(n) = &items[1] { *n as u32 } else { 0 };
                let has_spec = items.len() > 2 && !is_none_ir(&items[2]);

                self.compile_node(expr)?;

                if has_spec {
                    if let IR::String(spec_str) = &items[2] {
                        let idx = self.core.add_const(Value::from_string(spec_str.clone()));
                        self.emit(VMOpCode::LoadConst, idx as u32);
                    } else {
                        return Err(CompileError::SyntaxError(
                            "f-string format spec must be a string literal".to_string(),
                        ));
                    }
                }

                let flags = (conv << 1) | (has_spec as u32);
                self.emit(VMOpCode::FormatValue, flags);
                n_parts += 1;
            }
        }

        if n_parts == 0 {
            let idx = self.core.add_const(Value::from_string(String::new()));
            self.emit(VMOpCode::LoadConst, idx as u32);
        } else if n_parts > 1 {
            self.emit(VMOpCode::BuildString, n_parts);
        }
        Ok(())
    }
}
