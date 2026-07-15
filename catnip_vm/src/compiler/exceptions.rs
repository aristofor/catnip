// FILE: catnip_vm/src/compiler/exceptions.rs
use super::*;

impl PureCompiler {
    // ========== Exception handling ==========

    /// Emit inline finally cleanup for each active finally level.
    pub(crate) fn emit_finally_unwind(&mut self) -> CompileResult<()> {
        let bodies: Vec<FinallyInfo> = self.finally_stack.iter().rev().cloned().collect();
        for info in &bodies {
            if info.needs_clear_exception {
                self.emit(VMOpCode::ClearException, 0);
            }
            if info.has_except {
                self.emit(VMOpCode::PopHandler, 0);
            }
            self.emit(VMOpCode::PopHandler, 0);
            self.core.finally_depth -= 1;
            self.compile_node(&info.body)?;
            self.emit(VMOpCode::PopTop, 0);
        }
        Ok(())
    }

    /// Emit break with finally bodies inlined (if inside try/finally).
    pub(crate) fn compile_break_with_finally(&mut self) -> CompileResult<()> {
        if self.core.finally_depth == 0 {
            return Ok(self.core.compile_break()?);
        }
        let n = self.finally_stack.len();
        self.emit_finally_unwind()?;
        let result = self.core.compile_break();
        self.core.finally_depth += n;
        Ok(result?)
    }

    /// Emit continue with finally bodies inlined.
    pub(crate) fn compile_continue_with_finally(&mut self) -> CompileResult<()> {
        if self.core.finally_depth == 0 {
            return Ok(self.core.compile_continue()?);
        }
        let n = self.finally_stack.len();
        self.emit_finally_unwind()?;
        let result = self.core.compile_continue();
        self.core.finally_depth += n;
        Ok(result?)
    }

    pub(crate) fn compile_try(&mut self, args: &[IR]) -> CompileResult<()> {
        let body = &args[0];
        let handlers_ir = &args[1];
        let finally_ir = &args[2];
        let has_finally = !matches!(finally_ir, IR::None);

        let handlers = match handlers_ir {
            IR::List(items) => items.as_slice(),
            _ => &[],
        };
        let has_except = !handlers.is_empty();

        let mut finally_setup_addr = None;
        let mut except_setup_addr = None;

        // Install handlers (Finally first, Except on top)
        if has_finally {
            finally_setup_addr = Some(self.emit(VMOpCode::SetupFinally, 0));
            self.core.finally_depth += 1;
            self.finally_stack.push(FinallyInfo {
                body: finally_ir.clone(),
                has_except,
                needs_clear_exception: false,
            });
        }
        if has_except {
            except_setup_addr = Some(self.emit(VMOpCode::SetupExcept, 0));
        }

        // Try body
        self.compile_node(body)?;

        // Happy path: pop handlers
        if has_except {
            self.emit(VMOpCode::PopHandler, 0);
        }
        if has_finally {
            self.emit(VMOpCode::PopHandler, 0);
            self.core.finally_depth -= 1;
            self.finally_stack.pop();
        }

        // Inline finally on happy path
        if has_finally {
            self.compile_node(finally_ir)?;
            self.emit(VMOpCode::PopTop, 0);
        }
        let end_jump = self.emit(VMOpCode::Jump, 0);
        let mut handler_end_jumps: Vec<usize> = Vec::new();

        // Except dispatch
        if has_except {
            let except_addr = self.instructions.len();
            if let Some(addr) = except_setup_addr {
                self.core.patch(addr, except_addr as u32);
            }

            // While compiling handler bodies, restore finally context so that
            // break/continue/return inside a handler inline ClearException + finally.
            if has_finally {
                self.core.finally_depth += 1;
                self.finally_stack.push(FinallyInfo {
                    body: finally_ir.clone(),
                    has_except: false,           // except handler already popped by VM
                    needs_clear_exception: true, // inside handler: clear exception stack on exit
                });
            }

            for handler_ir in handlers {
                let (types, binding, handler_body) = match handler_ir {
                    IR::Tuple(items) if items.len() >= 3 => (&items[0], &items[1], &items[2]),
                    _ => continue,
                };

                let type_list = match types {
                    IR::List(t) => t.as_slice(),
                    _ => &[],
                };
                let is_wildcard = type_list.is_empty();

                if !is_wildcard {
                    // Typed handler: check each exception type
                    let mut type_match_jumps = Vec::new();
                    for type_ir in type_list {
                        let type_name = match type_ir {
                            IR::String(s) => s.clone(),
                            _ => continue,
                        };
                        let const_idx = self.core.add_const(Value::from_string(type_name));
                        self.emit(VMOpCode::CheckExcMatch, const_idx as u32);
                        type_match_jumps.push(self.emit(VMOpCode::JumpIfTrue, 0));
                    }
                    let skip_jump = self.emit(VMOpCode::Jump, 0);

                    // Handler body start
                    let handler_start = self.instructions.len() as u32;
                    for addr in type_match_jumps {
                        self.core.patch(addr, handler_start);
                    }

                    // Bind exception message if binding present
                    if !matches!(binding, IR::None) {
                        if let IR::String(name) = binding {
                            self.emit(VMOpCode::LoadException, 0);
                            let slot = self.add_local(name);
                            self.emit(VMOpCode::StoreLocal, slot as u32);
                        }
                    }

                    self.compile_node(handler_body)?;

                    // Pop exception stack + inline finally (normal exit path)
                    self.emit(VMOpCode::ClearException, 0);
                    if has_finally {
                        self.emit(VMOpCode::PopHandler, 0);
                        self.compile_node(finally_ir)?;
                        self.emit(VMOpCode::PopTop, 0);
                    }
                    handler_end_jumps.push(self.emit(VMOpCode::Jump, 0));

                    let next = self.instructions.len() as u32;
                    self.core.patch(skip_jump, next);
                } else {
                    // Wildcard handler
                    if !matches!(binding, IR::None) {
                        if let IR::String(name) = binding {
                            self.emit(VMOpCode::LoadException, 0);
                            let slot = self.add_local(name);
                            self.emit(VMOpCode::StoreLocal, slot as u32);
                        }
                    }

                    self.compile_node(handler_body)?;

                    // Pop exception stack + inline finally (normal exit path)
                    self.emit(VMOpCode::ClearException, 0);
                    if has_finally {
                        self.emit(VMOpCode::PopHandler, 0);
                        self.compile_node(finally_ir)?;
                        self.emit(VMOpCode::PopTop, 0);
                    }
                    handler_end_jumps.push(self.emit(VMOpCode::Jump, 0));
                    break; // Wildcard is always last
                }
            }

            // Restore finally context after handler body compilation
            if has_finally {
                self.core.finally_depth -= 1;
                self.finally_stack.pop();
            }

            // No handler matched: bare re-raise (goes through handler stack for finally)
            self.emit(VMOpCode::Raise, 1);
        }

        // Finally landing pad (reached by VM when it pops a Finally handler)
        if has_finally {
            let finally_landing = self.instructions.len() as u32;
            if let Some(addr) = finally_setup_addr {
                self.core.patch(addr, finally_landing);
            }
            self.compile_node(finally_ir)?;
            self.emit(VMOpCode::PopTop, 0);
            self.emit(VMOpCode::ResumeUnwind, 0);
        }

        // End label: all paths (happy, handler, finally-only) converge here
        let end_addr = self.instructions.len() as u32;
        self.core.patch(end_jump, end_addr);
        for addr in handler_end_jumps {
            self.core.patch(addr, end_addr);
        }
        Ok(())
    }

    pub(crate) fn compile_raise(&mut self, args: &[IR]) -> CompileResult<()> {
        if args.is_empty() {
            // Bare raise
            self.emit(VMOpCode::Raise, 1);
        } else {
            // raise expr
            self.compile_node(&args[0])?;
            self.emit(VMOpCode::Raise, 0);
        }
        Ok(())
    }
}
