//! UnifiedCompiler: functions, collections, broadcast, match, struct/trait compilation.

use super::*;

impl UnifiedCompiler {
    // ========== 11. Functions (call, lambda, fn_def) ==========

    /// Dispatch-form call (no tail position): func passed separately from args.
    pub(crate) fn compile_call_dispatch<'py>(
        &mut self,
        py: Python<'py>,
        func: &CompilerNode<'py>,
        args: &[CompilerNode<'py>],
        kwargs: &CompilerKwargs<'py>,
    ) -> PyResult<()> {
        self.compile_call_parts(py, func, args, kwargs, false)
    }

    pub(crate) fn compile_call<'py>(
        &mut self,
        py: Python<'py>,
        args: &[CompilerNode<'py>],
        kwargs: &CompilerKwargs<'py>,
        is_tail: bool,
    ) -> PyResult<()> {
        self.compile_call_parts(py, &args[0], &args[1..], kwargs, is_tail)
    }

    /// Shared call compilation: detects method calls, emits Call/CallKw/
    /// CallMethod/TailCall.
    fn compile_call_parts<'py>(
        &mut self,
        py: Python<'py>,
        func: &CompilerNode<'py>,
        call_args: &[CompilerNode<'py>],
        kwargs: &CompilerKwargs<'py>,
        is_tail: bool,
    ) -> PyResult<()> {
        let is_empty_kwargs = kwargs.is_empty()?;

        // Detect method call
        let method_call_info = if is_empty_kwargs && !is_tail {
            func.as_getattr_parts(py)
        } else {
            None
        };

        if let Some((obj, method_name)) = method_call_info {
            self.compile_node(py, &obj)?;
            for arg in call_args {
                self.compile_node(py, arg)?;
            }
            let name_idx = self.add_name(&method_name);
            let encoding = ((name_idx as u32) << crate::vm::CALL_ARGS_SHIFT) | (call_args.len() as u32);
            self.emit(VMOpCode::CallMethod, encoding);
        } else {
            self.compile_node(py, func)?;
            for arg in call_args {
                self.compile_node(py, arg)?;
            }
            if !is_empty_kwargs {
                let kw_pairs = kwargs.iter(py)?;
                let mut kw_names = Vec::new();
                for (name, value) in &kw_pairs {
                    kw_names.push(name.clone());
                    self.compile_node(py, value)?;
                }
                let kw_tuple = PyTuple::new(py, &kw_names)?;
                let kw_idx = self.add_const_pyobj(py, &kw_tuple.into_any());
                self.emit(VMOpCode::LoadConst, kw_idx as u32);
                let encoding = ((call_args.len() as u32) << 8) | (kw_pairs.len() as u32);
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

    pub(crate) fn compile_lambda<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let self_name = self.pending_self_name.take();
        // Consume the `@pure` flag before compiling the body, so a nested lambda
        // in the body does not inherit it.
        let is_pure = std::mem::take(&mut self.pending_pure);
        let raw_params = &args[0];
        let body = &args[1];

        let (param_names, defaults, vararg_idx, param_types) = self.extract_params(py, raw_params)?;

        let mut func_compiler = UnifiedCompiler::new();
        let mut code = func_compiler.compile_function(
            py,
            FunctionCompileSpec {
                params: param_names,
                param_types,
                body,
                name: self_name.as_deref().unwrap_or("<lambda>"),
                defaults,
                vararg_idx,
                parent_nesting_depth: self.nesting_depth,
            },
        )?;

        code.is_pure = is_pure;
        // Freeze IR source for ND process workers
        code.encoded_ir = freeze_ir_body(body, raw_params);

        let py_code = Py::new(py, PyCodeObject::new(code))?;
        let val = Value::from_pyobject(py, py_code.bind(py))?;
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

    pub(crate) fn compile_fn_def<'py>(
        &mut self,
        py: Python<'py>,
        args: &[CompilerNode<'py>],
        _kwargs: &CompilerKwargs<'py>,
    ) -> PyResult<()> {
        let name = args[0].as_name(py).unwrap_or_else(|| "<fn>".to_string());
        let raw_params = &args[1];
        let body = &args[2];

        let (param_names, defaults, vararg_idx, param_types) = self.extract_params(py, raw_params)?;

        let mut func_compiler = UnifiedCompiler::new();
        let mut code = func_compiler.compile_function(
            py,
            FunctionCompileSpec {
                params: param_names,
                param_types,
                body,
                name: &name,
                defaults,
                vararg_idx,
                parent_nesting_depth: self.nesting_depth,
            },
        )?;

        // Freeze IR source for ND process workers
        code.encoded_ir = freeze_ir_body(body, raw_params);

        let py_code = Py::new(py, PyCodeObject::new(code))?;
        let val = Value::from_pyobject(py, py_code.bind(py))?;
        let idx = self.core.add_const(val);
        self.emit(VMOpCode::LoadConst, idx as u32);
        let mf_arg = self.add_name(&name) as u32 + 1;
        self.emit(VMOpCode::MakeFunction, mf_arg);

        self.core.emit_store(&name);
        Ok(())
    }

    // ========== 12. Collections ==========

    pub(crate) fn compile_list<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            self.compile_node(py, arg)?;
        }
        self.emit(VMOpCode::BuildList, args.len() as u32);
        Ok(())
    }

    pub(crate) fn compile_tuple<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            self.compile_node(py, arg)?;
        }
        self.emit(VMOpCode::BuildTuple, args.len() as u32);
        Ok(())
    }

    pub(crate) fn compile_set<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            self.compile_node(py, arg)?;
        }
        self.emit(VMOpCode::BuildSet, args.len() as u32);
        Ok(())
    }

    pub(crate) fn compile_dict<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        for arg in args {
            // Each arg is a 2-child node (key, value)
            let key = arg.child(py, 0)?;
            let value = arg.child(py, 1)?;
            self.compile_node(py, &key)?;
            self.compile_node(py, &value)?;
        }
        self.emit(VMOpCode::BuildDict, args.len() as u32);
        Ok(())
    }

    // ========== 13. Broadcast ==========

    pub(crate) fn compile_broadcast_op<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.len() < 4 {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "Broadcast requires 4 arguments: target, operator, operand, is_filter",
            ));
        }
        let target_expr = &args[0];
        let operator_expr = &args[1];
        let operand_expr = &args[2];
        let is_filter = args[3].as_bool().unwrap_or(false);

        self.compile_node(py, target_expr)?;
        self.compile_node(py, operator_expr)?;

        let has_operand = !operand_expr.is_none_value();
        if has_operand {
            self.compile_node(py, operand_expr)?;
        }

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

    // ========== 14. Match ==========

    pub(crate) fn compile_match<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let value_expr = &args[0];
        let cases_node = &args[1];

        // Pre-allocate slots for pattern variables
        self.collect_pattern_vars(py, cases_node)?;

        // Compile value to match
        self.compile_node(py, value_expr)?;

        let cases = cases_node.children(py)?;
        let mut end_jumps = Vec::new();

        for case in &cases {
            let case_len = case.children_len(py)?;
            if case_len < 3 {
                continue;
            }
            let pattern = case.child(py, 0)?;
            let guard = case.child(py, 1)?;
            let body = case.child(py, 2)?;

            self.emit(VMOpCode::DupTop, 0);

            let vm_pattern = self
                .try_compile_pattern(py, &pattern)?
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("unsupported match pattern"))?;
            let pat_idx = self.patterns.len();
            self.patterns.push(vm_pattern);
            self.emit(VMOpCode::MatchPatternVM, pat_idx as u32);

            self.emit(VMOpCode::DupTop, 0);
            let skip_jump = self.emit(VMOpCode::JumpIfNone, 0);

            let guard_fail = if !guard.is_none_value() {
                self.emit(VMOpCode::DupTop, 0);
                self.emit(VMOpCode::PushBlock, 0);
                self.emit(VMOpCode::BindMatch, 0);
                self.compile_node(py, &guard)?;
                self.emit(VMOpCode::PopBlock, 0);
                Some(self.emit(VMOpCode::JumpIfFalse, 0))
            } else {
                None
            };

            self.emit(VMOpCode::BindMatch, 0);
            self.emit(VMOpCode::PopTop, 0);
            self.compile_node(py, &body)?;
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
        let msg = "No matching pattern";
        let msg_py = msg.into_pyobject(py)?.into_any();
        let msg_idx = self.add_const_pyobj(py, &msg_py);
        self.emit(VMOpCode::MatchFail, msg_idx as u32);

        let end_addr = self.instructions.len() as u32;
        for addr in end_jumps {
            self.patch(addr, end_addr);
        }
        Ok(())
    }

    // ========== 15. Struct/Trait ==========

    /// Compile a struct/trait method list into `(name, code_or_None, is_static)`
    /// entries; abstract methods (no lambda body) keep a None body slot.
    fn compile_method_list<'py>(
        &mut self,
        py: Python<'py>,
        methods_node: &CompilerNode<'py>,
    ) -> PyResult<Bound<'py, PyList>> {
        let methods_cn = methods_node.children(py)?;
        let mut compiled: Vec<Py<PyAny>> = Vec::new();
        for m in &methods_cn {
            let method_name = m.child(py, 0)?.as_name(py).unwrap_or_default();
            let is_static = if m.children_len(py)? > 2 {
                m.child(py, 2)?.as_bool().unwrap_or(false)
            } else {
                false
            };
            let is_static_py = is_static.into_pyobject(py)?.to_owned().into_any().unbind();

            // Abstract method check
            let lambda_node = m.child(py, 1)?;
            if lambda_node.is_none_value() {
                let pair = PyTuple::new(
                    py,
                    &[
                        method_name.into_pyobject(py)?.into_any().unbind(),
                        py.None(),
                        is_static_py,
                    ],
                )?;
                compiled.push(pair.into_any().unbind());
                continue;
            }

            // Compile method body (lambda Op) - extract params and body from the lambda
            let lambda_params = lambda_node.child(py, 0)?;
            let lambda_body = lambda_node.child(py, 1)?;
            let (param_names, defaults, vararg_idx, param_types) = self.extract_params(py, &lambda_params)?;

            let mut func_compiler = UnifiedCompiler::new();
            let mut code = func_compiler.compile_function(
                py,
                FunctionCompileSpec {
                    params: param_names,
                    param_types,
                    body: &lambda_body,
                    name: &method_name,
                    defaults,
                    vararg_idx,
                    parent_nesting_depth: self.nesting_depth,
                },
            )?;
            code.encoded_ir = freeze_ir_body(&lambda_body, &lambda_params);
            let py_code = Py::new(py, PyCodeObject::new(code))?;
            let pair = PyTuple::new(
                py,
                &[
                    method_name.into_pyobject(py)?.into_any().unbind(),
                    py_code.into_any(),
                    is_static_py,
                ],
            )?;
            compiled.push(pair.into_any().unbind());
        }
        PyList::new(py, &compiled)
    }

    pub(crate) fn compile_struct<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let name = args[0].as_name(py).unwrap_or_else(|| "<struct>".to_string());

        let fields_cn = args[1].children(py)?;
        let args_len = args.len();

        let mut implements_list: Vec<String> = Vec::new();
        let mut base_names: Vec<String> = Vec::new();
        let mut methods_index: Option<usize> = None;

        if args_len > 3 {
            // args[2] = implements, args[3] = bases
            let impl_items = args[2].children(py)?;
            for imp in &impl_items {
                if let Some(s) = imp.as_name(py) {
                    implements_list.push(s);
                }
            }
            // bases
            if !args[3].is_none_value() {
                let base_items = args[3].children(py).unwrap_or_default();
                if !base_items.is_empty() {
                    for b in &base_items {
                        if let Some(s) = b.as_name(py) {
                            base_names.push(s);
                        }
                    }
                } else if let Some(s) = args[3].as_name(py) {
                    base_names.push(s);
                }
            }
            if args_len > 4 {
                methods_index = Some(4);
            }
        } else if args_len > 2 {
            if let Some(s) = args[2].as_name(py) {
                base_names.push(s);
                if args_len > 3 {
                    methods_index = Some(3);
                }
            } else {
                let impl_items = args[2].children(py).unwrap_or_default();
                if !impl_items.is_empty() {
                    let mut is_impl_list = true;
                    for imp in &impl_items {
                        if let Some(s) = imp.as_name(py) {
                            implements_list.push(s);
                        } else {
                            is_impl_list = false;
                            break;
                        }
                    }
                    if !is_impl_list {
                        implements_list.clear();
                        methods_index = Some(2);
                    }
                } else {
                    methods_index = Some(2);
                }
            }
        }

        // Compile field defaults and build fields_info
        let mut fields_info: Vec<Py<PyAny>> = Vec::new();
        let mut num_defaults: u32 = 0;

        for field in &fields_cn {
            let field_len = field.children_len(py)?;
            if field_len >= 2 {
                let fname = field.child(py, 0)?.as_name(py).unwrap_or_default();
                let has_default = field.child(py, 1)?.as_bool().unwrap_or(false);
                if has_default && field_len >= 3 {
                    let default_expr = field.child(py, 2)?;
                    self.compile_node(py, &default_expr)?;
                    num_defaults += 1;
                }
                // Field IR is (name, has_default, default, type_or_none); carry the
                // raw annotation text (element 3) so the MakeStruct handler can
                // classify a runtime check, mirroring the PureVM compiler.
                let mut elems = vec![
                    fname.into_pyobject(py)?.into_any().unbind(),
                    has_default.into_pyobject(py)?.to_owned().into_any().unbind(),
                ];
                if field_len >= 4 {
                    if let Some(type_text) = field.child(py, 3).ok().and_then(|t| t.as_string().ok()) {
                        elems.push(type_text.into_pyobject(py)?.into_any().unbind());
                    }
                }
                let entry = PyTuple::new(py, &elems)?;
                fields_info.push(entry.into_any().unbind());
            }
        }
        let fields_tuple = PyTuple::new(py, &fields_info)?;

        // Compile methods
        let methods_list = match methods_index {
            Some(idx) => Some(self.compile_method_list(py, &args[idx])?),
            None => None,
        };

        // Build struct info constant
        let num_defaults_py = num_defaults.into_pyobject(py)?.into_any().as_any().clone();
        let name_py = name.as_str().into_pyobject(py)?.into_any().as_any().clone();

        let has_implements = !implements_list.is_empty();
        let has_bases = !base_names.is_empty();

        let struct_info = if has_implements || has_bases {
            let impl_py = PyTuple::new(
                py,
                implements_list
                    .iter()
                    .map(|s| s.as_str().into_pyobject(py).unwrap().into_any().unbind())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )?;
            let bases_py = if has_bases {
                PyTuple::new(
                    py,
                    base_names
                        .iter()
                        .map(|s| s.as_str().into_pyobject(py).unwrap().into_any().unbind())
                        .collect::<Vec<_>>()
                        .as_slice(),
                )?
                .into_any()
                .unbind()
            } else {
                py.None()
            };
            let mut items: Vec<Py<PyAny>> = vec![
                name_py.unbind(),
                fields_tuple.into_any().unbind(),
                num_defaults_py.unbind(),
                impl_py.into_any().unbind(),
                bases_py,
            ];
            if let Some(methods) = methods_list {
                items.push(methods.into_any().unbind());
            }
            PyTuple::new(py, items.as_slice())?
        } else {
            match methods_list {
                Some(methods) => PyTuple::new(
                    py,
                    &[name_py, fields_tuple.into_any(), num_defaults_py, methods.into_any()],
                )?,
                None => PyTuple::new(py, &[name_py, fields_tuple.into_any(), num_defaults_py])?,
            }
        };

        let idx = self.add_const_pyobj(py, &struct_info.into_any());
        self.emit(VMOpCode::MakeStruct, idx as u32);
        Ok(())
    }

    pub(crate) fn compile_trait<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let name = args[0].as_name(py).unwrap_or_default();

        // args[1] = extends list, args[2] = fields, args[3] = methods (optional)
        let extends_cn = args[1].children(py)?;
        let fields_cn = args[2].children(py)?;

        let mut extends: Vec<Py<PyAny>> = Vec::new();
        for e in &extends_cn {
            if let Some(s) = e.as_name(py) {
                extends.push(s.into_pyobject(py)?.into_any().unbind());
            }
        }
        let extends_tuple = PyTuple::new(py, &extends)?;

        let mut fields_info: Vec<Py<PyAny>> = Vec::new();
        let mut num_defaults: u32 = 0;
        for f in &fields_cn {
            let f_len = f.children_len(py)?;
            if f_len >= 2 {
                let fname = f.child(py, 0)?.as_name(py).unwrap_or_default();
                let default_node = f.child(py, 1)?;
                let has_default = !default_node.is_none_value();
                if has_default {
                    self.compile_node(py, &default_node)?;
                    num_defaults += 1;
                }
                let entry = PyTuple::new(
                    py,
                    &[
                        fname.into_pyobject(py)?.into_any().unbind(),
                        has_default.into_pyobject(py)?.to_owned().into_any().unbind(),
                    ],
                )?;
                fields_info.push(entry.into_any().unbind());
            }
        }
        let fields_tuple = PyTuple::new(py, &fields_info)?;

        // Methods
        let methods_list = if args.len() > 3 {
            Some(self.compile_method_list(py, &args[3])?)
        } else {
            None
        };

        let num_defaults_py = num_defaults.into_pyobject(py)?.into_any().as_any().clone();
        let trait_info = if let Some(methods) = methods_list {
            PyTuple::new(
                py,
                &[
                    name.as_str().into_pyobject(py)?.into_any().as_any().clone(),
                    extends_tuple.into_any(),
                    fields_tuple.into_any(),
                    num_defaults_py,
                    methods.into_any(),
                ],
            )?
        } else {
            PyTuple::new(
                py,
                &[
                    name.as_str().into_pyobject(py)?.into_any().as_any().clone(),
                    extends_tuple.into_any(),
                    fields_tuple.into_any(),
                    num_defaults_py,
                ],
            )?
        };

        let idx = self.add_const_pyobj(py, &trait_info.into_any());
        self.emit(VMOpCode::MakeTrait, idx as u32);
        Ok(())
    }
}
