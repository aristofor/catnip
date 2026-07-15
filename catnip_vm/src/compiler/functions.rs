// FILE: catnip_vm/src/compiler/functions.rs
use super::*;

impl PureCompiler {
    // ========== Functions ==========

    /// Dispatch-form call (no tail position): func passed separately from args.
    pub(crate) fn compile_call_dispatch(
        &mut self,
        func: &IR,
        args: &[IR],
        kwargs: &IndexMap<String, IR>,
    ) -> CompileResult<()> {
        let mut all: Vec<&IR> = Vec::with_capacity(args.len() + 1);
        all.push(func);
        all.extend(args.iter());
        self.compile_call_from_args(&all, kwargs, false)
    }

    pub(crate) fn compile_call_op(
        &mut self,
        args: &[IR],
        kwargs: &IndexMap<String, IR>,
        is_tail: bool,
    ) -> CompileResult<()> {
        self.compile_call_from_args(&args.iter().collect::<Vec<_>>(), kwargs, is_tail)
    }

    /// Shared call compilation: detects method calls, emits Call/CallKw/
    /// CallMethod/TailCall. `args[0]` is the callee.
    pub(crate) fn compile_call_from_args(
        &mut self,
        args: &[&IR],
        kwargs: &IndexMap<String, IR>,
        is_tail: bool,
    ) -> CompileResult<()> {
        let func = args[0];
        let call_args = &args[1..];
        let is_empty_kwargs = kwargs.is_empty();

        let method_call_info = if is_empty_kwargs && !is_tail {
            as_getattr_parts_ir(func)
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            self.compile_node(obj)?;
            for arg in call_args {
                self.compile_node(arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << catnip_core::vm::CALL_ARGS_SHIFT) | (call_args.len() as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            self.compile_node(func)?;
            for arg in call_args {
                self.compile_node(arg)?;
            }
            if !is_empty_kwargs {
                let mut kw_names = Vec::new();
                for (name, value) in kwargs {
                    kw_names.push(name.clone());
                    self.compile_node(value)?;
                }
                let kw_tuple_val = Value::from_tuple(kw_names.iter().map(|n| Value::from_string(n.clone())).collect());
                let kw_idx = self.core.add_const(kw_tuple_val);
                self.emit(VMOpCode::LoadConst, kw_idx as u32);
                let encoding = ((call_args.len() as u32) << 8) | (kwargs.len() as u32);
                self.emit(VMOpCode::CallKw, encoding);
            } else if is_tail {
                self.emit(VMOpCode::TailCall, call_args.len() as u32);
            } else {
                self.emit(VMOpCode::Call, call_args.len() as u32);
            }
        }
        Ok(())
    }

    /// Record a syntactic function definition (`name = lambda`) and patch the
    /// closures of sibling definitions of the same block (letrec*): each
    /// earlier sibling gets the new function injected under its name, so
    /// mutual recursion works even after the closures escape the scope.
    pub(crate) fn register_letrec_def(&mut self, name: &str) {
        // Rebinding a captured outer name is a mutation, not a definition
        if self.nesting_depth > 0 && self.core.outer_names.contains(name) {
            return;
        }
        let Some(slot) = self.core.locals.iter().position(|n| n == name) else {
            return;
        };
        let peers: Vec<(String, usize)> = self.core.block_fn_defs.last().cloned().unwrap_or_default();
        let name_idx = self.add_name(name);
        for (peer_name, peer_slot) in peers {
            if peer_name == name {
                continue;
            }
            self.emit(VMOpCode::LoadLocal, peer_slot as u32);
            self.emit(VMOpCode::LoadLocal, slot as u32);
            self.emit(VMOpCode::PatchClosure, name_idx as u32);
        }
        if let Some(defs) = self.core.block_fn_defs.last_mut() {
            defs.retain(|(n, _)| n != name);
            defs.push((name.to_string(), slot));
        }
    }

    pub(crate) fn compile_lambda(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 2, "lambda")?;
        let self_name = self.core.pending_self_name.take();
        // Consume the `@pure` flag before compiling the body, so a nested lambda
        // in the body does not inherit it.
        let is_pure = std::mem::take(&mut self.core.pending_pure);
        let raw_params = &args[0];
        let body = &args[1];

        let (param_names, defaults, vararg_idx, param_types) = self.extract_params(raw_params)?;

        let mut code = self.compile_function_inner(FunctionCompileSpec {
            params: param_names,
            param_types,
            body,
            name: self_name.as_deref().unwrap_or("<lambda>"),
            defaults,
            vararg_idx,
            parent_nesting_depth: self.nesting_depth,
        })?;

        code.is_pure = is_pure;
        code.encoded_ir = Self::freeze_ir_body(body, raw_params);

        let func_idx = self.functions.len() as u32;
        self.functions.push(code);
        let val = Value::from_vmfunc(func_idx);
        let idx = self.core.add_const(val);
        self.emit(VMOpCode::LoadConst, idx as u32);
        // arg = name_idx + 1 binds the name to the function itself in its
        // closure (let-rec); 0 = anonymous
        let mf_arg = match self_name {
            Some(name) => self.add_name(&name) as u32 + 1,
            None => 0,
        };
        self.emit(VMOpCode::MakeFunction, mf_arg);
        Ok(())
    }

    pub(crate) fn compile_fn_def(&mut self, args: &[IR]) -> CompileResult<()> {
        Self::require_args(args, 3, "fn_def")?;
        let name = ir_to_name(&args[0]).unwrap_or_else(|| "<fn>".to_string());
        let raw_params = &args[1];
        let body = &args[2];

        let (param_names, defaults, vararg_idx, param_types) = self.extract_params(raw_params)?;

        let mut code = self.compile_function_inner(FunctionCompileSpec {
            params: param_names,
            param_types,
            body,
            name: &name,
            defaults,
            vararg_idx,
            parent_nesting_depth: self.nesting_depth,
        })?;

        code.encoded_ir = Self::freeze_ir_body(body, raw_params);

        let func_idx = self.functions.len() as u32;
        self.functions.push(code);
        let val = Value::from_vmfunc(func_idx);
        let idx = self.core.add_const(val);
        self.emit(VMOpCode::LoadConst, idx as u32);
        let mf_arg = self.add_name(&name) as u32 + 1;
        self.emit(VMOpCode::MakeFunction, mf_arg);

        self.core.emit_store(&name);
        Ok(())
    }
}
