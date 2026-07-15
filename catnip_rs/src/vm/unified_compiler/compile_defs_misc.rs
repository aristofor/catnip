//! UnifiedCompiler: enum/union defs, ND ops, f-strings, exceptions, helper methods.

use super::*;
use catnip_core::vm::opcode::ParamCheck;

/// Result of `extract_params`: (param_names, defaults, vararg_idx, param_checks).
/// `param_checks` holds the prologue boundary check per param (TH2-B primitive
/// `CheckType` + enforcement nominal `CheckNominal`), or `None` when unannotated
/// or not enforceable.
type ExtractedParams = (Vec<String>, Vec<Value>, i32, Vec<ParamCheck>);

impl UnifiedCompiler {
    // ========== 15b. Enum definition ==========

    pub(crate) fn compile_enum<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let name = args[0].as_name(py).unwrap_or_default();

        // args[1] = tuple of variant name strings
        let variant_nodes = args[1].children(py)?;
        let mut variant_names: Vec<Bound<'py, PyAny>> = Vec::new();
        for v in &variant_nodes {
            let vname = v.as_name(py).unwrap_or_default();
            variant_names.push(vname.into_pyobject(py)?.into_any());
        }
        let variants_tuple = PyTuple::new(py, &variant_names)?;

        let enum_info = PyTuple::new(
            py,
            &[
                name.as_str().into_pyobject(py)?.into_any().as_any().clone(),
                variants_tuple.into_any(),
            ],
        )?;

        let idx = self.add_const_pyobj(py, &enum_info.into_any());
        self.emit(VMOpCode::MakeEnum, idx as u32);
        Ok(())
    }

    // ========== 15c. Union definition ==========

    /// Compile a `UnionDef(name, type_params, variants[, methods])` IR node.
    ///
    /// Builds a Python tuple `(name, type_params_tuple, variants_tuple
    /// [, methods_list])` where each variant in `variants_tuple` is itself
    /// a tuple `(variant_name, field_names_tuple)` and each method is
    /// `(method_name, CodeObject)`. The tuple is stored as a single
    /// constant, referenced by `MakeUnion`'s argument.
    pub(crate) fn compile_union<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.len() < 3 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "UnionDef: expected (name, type_params, variants)",
            ));
        }

        let name = args[0].as_name(py).unwrap_or_default();

        // Type parameters: list of identifier strings.
        let type_param_nodes = args[1].children(py)?;
        let mut type_param_strs: Vec<Bound<'py, PyAny>> = Vec::new();
        for tp in &type_param_nodes {
            let s = tp.as_name(py).unwrap_or_default();
            type_param_strs.push(s.into_pyobject(py)?.into_any());
        }
        let type_params_tuple = PyTuple::new(py, &type_param_strs)?;

        // Variants: each is (variant_name, fields_list).
        let variant_nodes = args[2].children(py)?;
        let mut variant_tuples: Vec<Bound<'py, PyAny>> = Vec::new();
        for variant in &variant_nodes {
            let variant_children = variant.children(py)?;
            if variant_children.len() < 2 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "UnionDef: variant must be (name, fields)",
                ));
            }
            let variant_name = variant_children[0].as_name(py).unwrap_or_default();

            // Fields: each field is (field_name, type_text_or_none). Both the
            // name and the raw type text are emitted (parallel tuples); the type
            // text drives the generic-nominal boundary via `FieldTemplate`s built
            // in `build_union_type` (empty string = unannotated).
            let field_nodes = variant_children[1].children(py)?;
            let mut field_name_strs: Vec<Bound<'py, PyAny>> = Vec::new();
            let mut field_type_strs: Vec<Bound<'py, PyAny>> = Vec::new();
            for field in &field_nodes {
                let field_children = field.children(py)?;
                let (fname, ftype) = if field_children.is_empty() {
                    (field.as_name(py).unwrap_or_default(), String::new())
                } else {
                    let fname = field_children[0].as_name(py).unwrap_or_default();
                    let ftype = field_children.get(1).and_then(|c| c.as_name(py)).unwrap_or_default();
                    (fname, ftype)
                };
                field_name_strs.push(fname.into_pyobject(py)?.into_any());
                field_type_strs.push(ftype.into_pyobject(py)?.into_any());
            }
            let fields_tuple = PyTuple::new(py, &field_name_strs)?;
            let field_types_tuple = PyTuple::new(py, &field_type_strs)?;

            let variant_tuple = PyTuple::new(
                py,
                &[
                    variant_name.as_str().into_pyobject(py)?.into_any().as_any().clone(),
                    fields_tuple.into_any(),
                    field_types_tuple.into_any(),
                ],
            )?;
            variant_tuples.push(variant_tuple.into_any());
        }
        let variants_tuple = PyTuple::new(py, &variant_tuples)?;

        // Methods: each is (method_name, lambda). Compiled like struct
        // methods -- one CodeObject per method, no static/abstract forms.
        let methods_list = if args.len() > 3 {
            let methods_cn = args[3].children(py)?;
            let mut compiled: Vec<Py<PyAny>> = Vec::new();
            for m in &methods_cn {
                let method_name = m.child(py, 0)?.as_name(py).unwrap_or_default();
                let lambda_node = m.child(py, 1)?;
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
                    &[method_name.into_pyobject(py)?.into_any().unbind(), py_code.into_any()],
                )?;
                compiled.push(pair.into_any().unbind());
            }
            Some(PyList::new(py, &compiled)?)
        } else {
            None
        };

        let mut info_items: Vec<Py<PyAny>> = vec![
            name.as_str().into_pyobject(py)?.into_any().unbind(),
            type_params_tuple.into_any().unbind(),
            variants_tuple.into_any().unbind(),
        ];
        if let Some(methods) = methods_list {
            info_items.push(methods.into_any().unbind());
        }
        let union_info = PyTuple::new(py, info_items.as_slice())?;

        let idx = self.add_const_pyobj(py, &union_info.into_any());
        self.emit(VMOpCode::MakeUnion, idx as u32);
        Ok(())
    }

    // ========== 16. ND operations ==========

    pub(crate) fn compile_nd_recursion<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.len() == 1 {
            // Declaration form: ~~(lambda) → wraps lambda in NDVmDecl
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::NdRecursion, 1);
        } else if args.len() >= 2 && args[1].is_none_value() {
            // Declaration form with explicit None seed
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::NdRecursion, 1);
        } else if args.len() >= 2 {
            // Combinator form: ~~(seed, lambda)
            self.compile_node(py, &args[0])?;
            self.compile_node(py, &args[1])?;
            self.emit(VMOpCode::NdRecursion, 0);
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        Ok(())
    }

    pub(crate) fn compile_nd_map<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.len() == 1 {
            // Lift form: ~>(func) → return func as-is
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::NdMap, 1);
        } else if args.len() >= 2 && args[1].is_none_value() {
            // Lift form with explicit None
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::NdMap, 1);
        } else if args.len() >= 2 {
            // Applicative form: ~>(data, func)
            self.compile_node(py, &args[0])?;
            self.compile_node(py, &args[1])?;
            self.emit(VMOpCode::NdMap, 0);
        } else {
            let idx = self.core.add_const(Value::NIL);
            self.emit(VMOpCode::LoadConst, idx as u32);
        }
        Ok(())
    }

    // ========== 17. F-strings ==========

    pub(crate) fn compile_fstring<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.is_empty() {
            let py_str = "".into_pyobject(py)?.into_any();
            let idx = self.add_const_pyobj(py, &py_str);
            self.emit(VMOpCode::LoadConst, idx as u32);
            return Ok(());
        }

        let mut n_parts: u32 = 0;

        for part in args {
            if let Ok(text) = part.as_string() {
                // Text part → LoadConst
                let py_str = text.into_pyobject(py)?.into_any();
                let idx = self.add_const_pyobj(py, &py_str);
                self.emit(VMOpCode::LoadConst, idx as u32);
                n_parts += 1;
            } else if part.is_tuple() {
                // Interpolation: Tuple([expr, Int(conv), spec])
                let expr = part.child(py, 0)?;
                let conv = part.child(py, 1)?.as_int().unwrap_or(0) as u32;
                let spec_node = part.child(py, 2)?;
                let has_spec = !spec_node.is_none_value();

                self.compile_node(py, &expr)?;

                if has_spec {
                    let spec_str = spec_node.as_string()?;
                    let py_str = spec_str.into_pyobject(py)?.into_any();
                    let idx = self.add_const_pyobj(py, &py_str);
                    self.emit(VMOpCode::LoadConst, idx as u32);
                }

                // flags = (conv << 1) | has_spec
                let flags = (conv << 1) | (has_spec as u32);
                self.emit(VMOpCode::FormatValue, flags);
                n_parts += 1;
            }
        }

        if n_parts == 0 {
            let py_str = "".into_pyobject(py)?.into_any();
            let idx = self.add_const_pyobj(py, &py_str);
            self.emit(VMOpCode::LoadConst, idx as u32);
        } else if n_parts > 1 {
            self.emit(VMOpCode::BuildString, n_parts);
        }
        // n_parts == 1: result already on stack
        Ok(())
    }

    // ========== 18. Exception handling ==========

    /// Emit inline finally cleanup for break/continue/return paths.
    pub(crate) fn emit_finally_unwind<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        let bodies: Vec<UCFinallyInfo> = self.finally_stack.iter().rev().cloned().collect();
        for info in &bodies {
            if info.needs_clear_exception {
                self.emit(VMOpCode::ClearException, 0);
            }
            if info.has_except {
                self.emit(VMOpCode::PopHandler, 0);
            }
            self.emit(VMOpCode::PopHandler, 0);
            self.core.finally_depth -= 1;
            match &info.body {
                UCFinallyBody::Pure(ir) => {
                    let cn = CompilerNode::Pure(ir);
                    self.compile_node(py, &cn)?;
                }
                UCFinallyBody::PyObj(obj) => {
                    let cn = CompilerNode::PyObj(obj.bind(py).clone());
                    self.compile_node(py, &cn)?;
                }
            }
            self.emit(VMOpCode::PopTop, 0);
        }
        Ok(())
    }

    pub(crate) fn compile_break_with_finally<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        if self.core.finally_depth == 0 {
            return self.core.compile_break().map_err(syntax_err);
        }
        let n = self.finally_stack.len();
        self.emit_finally_unwind(py)?;
        let result = self.core.compile_break();
        self.core.finally_depth += n;
        result.map_err(syntax_err)
    }

    pub(crate) fn compile_continue_with_finally<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        if self.core.finally_depth == 0 {
            return self.core.compile_continue().map_err(syntax_err);
        }
        let n = self.finally_stack.len();
        self.emit_finally_unwind(py)?;
        let result = self.core.compile_continue();
        self.core.finally_depth += n;
        result.map_err(syntax_err)
    }

    pub(crate) fn compile_try<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        let body = &args[0];
        let handlers_node = &args[1];
        let finally_node = &args[2];
        let has_finally = !finally_node.is_none_value();

        let handlers = if handlers_node.is_list_or_tuple() {
            handlers_node.children(py)?
        } else {
            Vec::new()
        };
        let has_except = !handlers.is_empty();

        let mut finally_setup_addr = None;
        let mut except_setup_addr = None;

        // Install handlers (Finally first, Except on top)
        if has_finally {
            finally_setup_addr = Some(self.emit(VMOpCode::SetupFinally, 0));
            self.core.finally_depth += 1;
            let body = match finally_node {
                CompilerNode::Pure(ir) => UCFinallyBody::Pure((*ir).clone()),
                CompilerNode::PyObj(obj) => UCFinallyBody::PyObj(obj.clone().unbind()),
            };
            self.finally_stack.push(UCFinallyInfo {
                body,
                has_except,
                needs_clear_exception: false,
            });
        }
        if has_except {
            except_setup_addr = Some(self.emit(VMOpCode::SetupExcept, 0));
        }

        // Try body
        self.compile_node(py, body)?;

        // Happy path: pop handlers
        if has_except {
            self.emit(VMOpCode::PopHandler, 0);
        }
        // Save the finally body before popping (needed for handler bodies below)
        let saved_finally_body = if has_finally {
            self.finally_stack.last().map(|info| info.body.clone())
        } else {
            None
        };
        if has_finally {
            self.emit(VMOpCode::PopHandler, 0);
            self.core.finally_depth -= 1;
            self.finally_stack.pop();
        }

        // Inline finally on happy path
        if has_finally {
            self.compile_node(py, finally_node)?;
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

            // Restore finally context for handler bodies so break/continue inline ClearException + finally
            if let Some(ref body) = saved_finally_body {
                self.core.finally_depth += 1;
                self.finally_stack.push(UCFinallyInfo {
                    body: body.clone(),
                    has_except: false,
                    needs_clear_exception: true,
                });
            }

            for handler_node in &handlers {
                let handler_len = handler_node.children_len(py)?;
                if handler_len < 3 {
                    continue;
                }
                let types_node = handler_node.child(py, 0)?;
                let binding_node = handler_node.child(py, 1)?;
                let handler_body = handler_node.child(py, 2)?;

                let type_list = if types_node.is_list_or_tuple() {
                    types_node.children(py)?
                } else {
                    Vec::new()
                };
                let is_wildcard = type_list.is_empty();

                if !is_wildcard {
                    // Typed handler: check each exception type
                    let mut type_match_jumps = Vec::new();
                    for type_ir in &type_list {
                        let type_name = type_ir.as_string()?;
                        let py_str = type_name.into_pyobject(py)?.into_any();
                        let const_idx = self.add_const_pyobj(py, &py_str);
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
                    if !binding_node.is_none_value() {
                        if let Ok(name) = binding_node.as_string() {
                            self.emit(VMOpCode::LoadException, 0);
                            let slot = self.add_local(&name);
                            self.emit(VMOpCode::StoreLocal, slot as u32);
                        }
                    }

                    self.compile_node(py, &handler_body)?;

                    // Pop exception stack + inline finally
                    self.emit(VMOpCode::ClearException, 0);
                    if has_finally {
                        self.emit(VMOpCode::PopHandler, 0);
                        self.compile_node(py, finally_node)?;
                        self.emit(VMOpCode::PopTop, 0);
                    }
                    handler_end_jumps.push(self.emit(VMOpCode::Jump, 0));

                    let next = self.instructions.len() as u32;
                    self.core.patch(skip_jump, next);
                } else {
                    // Wildcard handler
                    if !binding_node.is_none_value() {
                        if let Ok(name) = binding_node.as_string() {
                            self.emit(VMOpCode::LoadException, 0);
                            let slot = self.add_local(&name);
                            self.emit(VMOpCode::StoreLocal, slot as u32);
                        }
                    }

                    self.compile_node(py, &handler_body)?;

                    // Pop exception stack + inline finally
                    self.emit(VMOpCode::ClearException, 0);
                    if has_finally {
                        self.emit(VMOpCode::PopHandler, 0);
                        self.compile_node(py, finally_node)?;
                        self.emit(VMOpCode::PopTop, 0);
                    }
                    handler_end_jumps.push(self.emit(VMOpCode::Jump, 0));
                    break; // Wildcard is always last
                }
            }

            // Restore finally context
            if saved_finally_body.is_some() {
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
            self.compile_node(py, finally_node)?;
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

    pub(crate) fn compile_raise<'py>(&mut self, py: Python<'py>, args: &[CompilerNode<'py>]) -> PyResult<()> {
        if args.is_empty() {
            // Bare raise
            self.emit(VMOpCode::Raise, 1);
        } else {
            // raise expr
            self.compile_node(py, &args[0])?;
            self.emit(VMOpCode::Raise, 0);
        }
        Ok(())
    }

    // ========== 19. Helper methods ==========

    /// Add a Python object constant (fallback to NIL on conversion error).
    pub(crate) fn add_const_pyobj(&mut self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> usize {
        self.core.add_const_py(py, obj).unwrap_or_else(|e| {
            #[cfg(debug_assertions)]
            eprintln!("[compiler] failed to convert PyObject to Value: {e}");
            let _ = e;
            self.core.add_const(Value::NIL)
        })
    }

    /// Check if body contains function calls (recursive scan).
    pub(crate) fn body_has_calls<'py>(&self, py: Python<'py>, node: &CompilerNode<'py>) -> bool {
        match node {
            CompilerNode::Pure(ir) => self.body_has_calls_ir(ir),
            CompilerNode::PyObj(obj) => self.body_has_calls_py(py, obj),
        }
    }

    pub(crate) fn body_has_calls_ir(&self, node: &IR) -> bool {
        match node {
            IR::Op { opcode, args, .. } => {
                if *opcode == IROpCode::Call || *opcode == IROpCode::FnDef || *opcode == IROpCode::OpLambda {
                    return true;
                }
                args.iter().any(|a| self.body_has_calls_ir(a))
            }
            IR::Call { .. } => true,
            IR::List(items) | IR::Tuple(items) | IR::Program(items) => items.iter().any(|i| self.body_has_calls_ir(i)),
            _ => false,
        }
    }

    pub(crate) fn body_has_calls_py(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> bool {
        if let Ok(list) = node.cast::<PyList>() {
            for item in list.iter() {
                if self.body_has_calls_py(py, &item) {
                    return true;
                }
            }
            return false;
        }
        if let Ok(tuple) = node.cast::<PyTuple>() {
            for item in tuple.iter() {
                if self.body_has_calls_py(py, &item) {
                    return true;
                }
            }
            return false;
        }
        if let Ok(op) = node.extract::<PyRef<Op>>() {
            let ident = op.ident;
            if ident == IROpCode::Call as i32 || ident == IROpCode::FnDef as i32 || ident == IROpCode::OpLambda as i32 {
                return true;
            }
            let args = op.args.bind(py);
            if let Ok(len) = args.len() {
                for i in 0..len {
                    if let Ok(arg) = args.get_item(i) {
                        if self.body_has_calls_py(py, &arg) {
                            return true;
                        }
                    }
                }
            }
            return false;
        }
        if let Ok(type_name) = node.get_type().name() {
            if type_name == "Call" {
                return true;
            }
        }
        false
    }

    /// Extract params, defaults, vararg_idx from a CompilerNode params node.
    pub(crate) fn extract_params<'py>(&self, py: Python<'py>, params: &CompilerNode<'py>) -> PyResult<ExtractedParams> {
        let mut param_names = Vec::new();
        let mut defaults = Vec::new();
        let mut vararg_idx: i32 = -1;
        // Prologue boundary check per param, aligned with `param_names`: a
        // primitive `CheckType` code, a nominal type name, or none.
        let mut param_types: Vec<ParamCheck> = Vec::new();

        let children = params.children(py)?;
        for item in &children {
            let item_len = item.children_len(py).unwrap_or(0);
            if item_len < 2 {
                // Simple param name (no tuple wrapper)
                if let Some(name) = item.as_name(py) {
                    param_names.push(name);
                    param_types.push(ParamCheck::None);
                }
                continue;
            }
            let first = item.child(py, 0)?;
            let second = item.child(py, 1)?;
            let name = first.as_name(py).unwrap_or_default();
            // Variadic marker ("*", vararg_name) stays a 2-element tuple.
            if item_len == 2 && name == "*" {
                vararg_idx = param_names.len() as i32;
                param_names.push(second.as_name(py).unwrap_or_default());
                param_types.push(ParamCheck::None);
            } else {
                // Regular param (name, default[, type]); the annotation at index
                // 2 maps to a primitive `CheckType` code or a nominal type name.
                param_names.push(name);
                let val = self.ir_to_value(py, &second)?;
                defaults.push(val);
                let check = if item_len >= 3 {
                    item.child(py, 2)
                        .ok()
                        .and_then(|t| t.as_string().ok())
                        .map(|n| ParamCheck::from_annotation(&n))
                        .unwrap_or(ParamCheck::None)
                } else {
                    ParamCheck::None
                };
                param_types.push(check);
            }
        }
        Ok((param_names, defaults, vararg_idx, param_types))
    }

    /// Per-param type codes from a params IR node (TH2-B 0b). Public so the ND
    /// `process` worker can rebuild typed-param boundary checks when it
    /// recompiles a function from frozen IR.
    pub fn param_type_codes(&self, py: Python<'_>, params: &IR) -> Vec<ParamCheck> {
        let cn = CompilerNode::Pure(params);
        self.extract_params(py, &cn).map(|(_, _, _, t)| t).unwrap_or_default()
    }

    /// Convert a literal CompilerNode to a Value.
    pub(crate) fn ir_to_value<'py>(&self, py: Python<'py>, node: &CompilerNode<'py>) -> PyResult<Value> {
        match node {
            CompilerNode::Pure(ir) => match ir {
                IR::Int(n) => Ok(Value::from_i64(*n)),
                IR::Float(f) => Ok(Value::from_float(*f)),
                IR::Bool(b) => Ok(Value::from_bool(*b)),
                IR::None => Ok(Value::NIL),
                IR::String(s) => {
                    let py_str = s.as_str().into_pyobject(py)?.into_any();
                    Value::from_pyobject(py, &py_str)
                }
                _ => Ok(Value::NIL),
            },
            CompilerNode::PyObj(obj) => Value::from_pyobject(py, obj).or(Ok(Value::NIL)),
        }
    }

    /// Try to compile a pattern into a VMPattern (native VM path).
    /// Returns None if the pattern can't be compiled natively (fallback to legacy).
    pub(crate) fn try_compile_pattern<'py>(
        &mut self,
        py: Python<'py>,
        pattern: &CompilerNode<'py>,
    ) -> PyResult<Option<VMPattern>> {
        match pattern {
            CompilerNode::Pure(ir) => self.try_compile_pattern_ir(py, ir),
            CompilerNode::PyObj(obj) => self.try_compile_pattern_py(py, obj),
        }
    }

    pub(crate) fn try_compile_pattern_ir(&mut self, py: Python<'_>, pattern: &IR) -> PyResult<Option<VMPattern>> {
        match pattern {
            IR::PatternWildcard => Ok(Some(VMPattern::Wildcard)),
            IR::PatternVar(name) => {
                if name == "_" {
                    Ok(Some(VMPattern::Wildcard))
                } else {
                    let slot = self.add_local(name);
                    Ok(Some(VMPattern::Var(slot)))
                }
            }
            IR::PatternLiteral(value) => {
                let val = self.ir_to_value(py, &CompilerNode::Pure(value))?;
                Ok(Some(VMPattern::Literal(val)))
            }
            IR::PatternOr(patterns) => {
                let mut sub_patterns = Vec::new();
                for p in patterns {
                    match self.try_compile_pattern_ir(py, p)? {
                        Some(vp) => sub_patterns.push(vp),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Or(sub_patterns)))
            }
            IR::PatternTuple(patterns) => {
                let mut elements = Vec::new();
                for p in patterns {
                    // Star pattern: encoded as Tuple(["*", name])
                    if let IR::Tuple(items) = p {
                        if items.len() == 2 {
                            if let (IR::String(star), IR::String(name)) = (&items[0], &items[1]) {
                                if star == "*" {
                                    let slot = if name.is_empty() || name == "_" {
                                        usize::MAX
                                    } else {
                                        self.add_local(name)
                                    };
                                    elements.push(VMPatternElement::Star(slot));
                                    continue;
                                }
                            }
                        }
                    }
                    match self.try_compile_pattern_ir(py, p)? {
                        Some(vp) => elements.push(VMPatternElement::Pattern(vp)),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Tuple(elements)))
            }
            IR::PatternStruct { name, variant, fields } => {
                let mut field_slots = Vec::new();
                for field_name in fields {
                    let slot = self.add_local(field_name);
                    field_slots.push((field_name.clone(), slot));
                }
                Ok(Some(VMPattern::Struct {
                    name: name.clone(),
                    variant: variant.clone(),
                    field_slots,
                }))
            }
            IR::PatternEnum {
                enum_name,
                variant_name,
            } => Ok(Some(VMPattern::Enum {
                enum_name: enum_name.clone(),
                variant_name: variant_name.clone(),
            })),
            _ => Ok(None),
        }
    }

    pub(crate) fn try_compile_pattern_py(
        &mut self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
    ) -> PyResult<Option<VMPattern>> {
        let tag = match get_pattern_tag(pattern) {
            Some(t) => t,
            None => return Ok(None),
        };
        match tag {
            TAG_WILDCARD => Ok(Some(VMPattern::Wildcard)),
            TAG_VAR => {
                let pat = pattern.cast::<PatternVar>().unwrap();
                let name = pat.borrow().name.clone();
                if name == "_" {
                    Ok(Some(VMPattern::Wildcard))
                } else {
                    let slot = self.add_local(&name);
                    Ok(Some(VMPattern::Var(slot)))
                }
            }
            TAG_LITERAL => {
                let pat = pattern.cast::<PatternLiteral>().unwrap();
                let value_obj = pat.borrow().value.clone_ref(py);
                let value_bound = value_obj.bind(py);
                if value_bound.cast::<Op>().is_ok() {
                    return Ok(None);
                }
                match Value::from_pyobject(py, value_bound) {
                    Ok(val) => Ok(Some(VMPattern::Literal(val))),
                    Err(_) => Ok(None),
                }
            }
            TAG_OR => {
                let pat = pattern.cast::<PatternOr>().unwrap();
                let patterns_obj = pat.borrow().patterns.clone_ref(py);
                let mut sub_patterns = Vec::new();
                for sub_result in patterns_obj.bind(py).try_iter()? {
                    let sub = sub_result?;
                    match self.try_compile_pattern_py(py, &sub)? {
                        Some(p) => sub_patterns.push(p),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Or(sub_patterns)))
            }
            TAG_TUPLE => {
                let pat = pattern.cast::<PatternTuple>().unwrap();
                let patterns_obj = pat.borrow().patterns.clone_ref(py);
                let mut elements = Vec::new();
                for sub_result in patterns_obj.bind(py).try_iter()? {
                    let sub = sub_result?;
                    // Check for star pattern tuple ("*", name)
                    if sub.is_instance_of::<PyTuple>() && sub.len()? == 2 {
                        let first: String = sub.get_item(0)?.extract().unwrap_or_default();
                        if first == "*" {
                            let name: String = sub.get_item(1)?.extract().unwrap_or_default();
                            let slot = if name.is_empty() || name == "_" {
                                usize::MAX
                            } else {
                                self.add_local(&name)
                            };
                            elements.push(VMPatternElement::Star(slot));
                            continue;
                        }
                    }
                    match self.try_compile_pattern_py(py, &sub)? {
                        Some(p) => elements.push(VMPatternElement::Pattern(p)),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Tuple(elements)))
            }
            TAG_STRUCT => {
                let pat = pattern.cast::<PatternStruct>().unwrap();
                let struct_name = pat.borrow().name.clone();
                let variant = pat.borrow().variant.clone();
                let fields_obj = pat.borrow().fields.clone_ref(py);
                let mut field_slots = Vec::new();
                for field_result in fields_obj.bind(py).try_iter()? {
                    let field_name: String = field_result?.extract()?;
                    let slot = self.add_local(&field_name);
                    field_slots.push((field_name, slot));
                }
                Ok(Some(VMPattern::Struct {
                    name: struct_name,
                    variant,
                    field_slots,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Pre-allocate local slots for all pattern variables in match cases.
    pub(crate) fn collect_pattern_vars<'py>(&mut self, py: Python<'py>, cases: &CompilerNode<'py>) -> PyResult<()> {
        match cases {
            CompilerNode::Pure(ir) => {
                let items = match ir {
                    IR::Tuple(items) | IR::List(items) => items.as_slice(),
                    _ => return Ok(()),
                };
                for case in items {
                    if let IR::Tuple(case_parts) = case {
                        if !case_parts.is_empty() {
                            self.collect_pattern_vars_ir(&case_parts[0]);
                        }
                    }
                }
            }
            CompilerNode::PyObj(obj) => {
                let len = obj.len()?;
                for i in 0..len {
                    let case = obj.get_item(i)?;
                    let pattern = case.get_item(0)?;
                    self.collect_vars_from_pattern_py(py, &pattern)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn collect_pattern_vars_ir(&mut self, pattern: &IR) {
        match pattern {
            IR::PatternVar(name) => {
                if name != "_" && !self.locals.contains(name) {
                    self.add_local(name);
                }
            }
            IR::PatternStruct { fields, .. } => {
                for field in fields {
                    if field != "_" && !self.locals.contains(field) {
                        self.add_local(field);
                    }
                }
            }
            IR::PatternOr(pats) | IR::PatternTuple(pats) => {
                for p in pats {
                    self.collect_pattern_vars_ir(p);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn collect_vars_from_pattern_py(&mut self, _py: Python<'_>, pattern: &Bound<'_, PyAny>) -> PyResult<()> {
        let type_name = pattern.get_type().name()?;

        if type_name == "PatternVar" {
            let name: String = pattern.getattr("name")?.extract()?;
            if name != "_" && !self.locals.contains(&name) {
                self.add_local(&name);
            }
        } else if type_name == "PatternStruct" {
            let fields = pattern.getattr("fields")?;
            for field_result in fields.try_iter()? {
                let name: String = field_result?.extract()?;
                if name != "_" && !self.locals.contains(&name) {
                    self.add_local(&name);
                }
            }
        } else if type_name == "PatternOr" || type_name == "PatternTuple" {
            let patterns = pattern.getattr("patterns")?;
            let len = patterns.len()?;
            for i in 0..len {
                let p = patterns.get_item(i)?;
                if p.is_instance_of::<PyTuple>() && p.len()? == 2 {
                    let first: String = p.get_item(0)?.extract().unwrap_or_default();
                    if first == "*" {
                        let name: String = p.get_item(1)?.extract().unwrap_or_default();
                        if !name.is_empty() && name != "_" && !self.locals.contains(&name) {
                            self.add_local(&name);
                        }
                        continue;
                    }
                }
                self.collect_vars_from_pattern_py(_py, &p)?;
            }
        }
        Ok(())
    }

    /// Try to compile an assignment pattern for set_locals complex patterns.
    pub(crate) fn try_compile_assign_pattern<'py>(
        &mut self,
        py: Python<'py>,
        pattern: &CompilerNode<'py>,
    ) -> PyResult<Option<VMPattern>> {
        match pattern {
            CompilerNode::Pure(ir) => self.try_compile_assign_pattern_ir(ir),
            CompilerNode::PyObj(obj) => self.try_compile_assign_pattern_py(py, obj),
        }
    }

    pub(crate) fn try_compile_assign_pattern_ir(&mut self, pattern: &IR) -> PyResult<Option<VMPattern>> {
        match pattern {
            IR::Tuple(items) => {
                let mut elements = Vec::new();
                for item in items {
                    if let IR::Tuple(pair) = item {
                        if pair.len() == 2 {
                            if let IR::String(s) = &pair[0] {
                                if s == "*" {
                                    let star_name = ir_to_name(&pair[1]).unwrap_or_default();
                                    let star_slot = self.add_local(&star_name);
                                    elements.push(VMPatternElement::Star(star_slot));
                                    continue;
                                }
                            }
                        }
                    }
                    match self.try_compile_assign_pattern_ir(item)? {
                        Some(sub) => elements.push(VMPatternElement::Pattern(sub)),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Tuple(elements)))
            }
            IR::Ref(name, _, _) | IR::Identifier(name) | IR::String(name) => {
                let slot = self.add_local(name);
                Ok(Some(VMPattern::Var(slot)))
            }
            _ => Ok(None),
        }
    }

    pub(crate) fn try_compile_assign_pattern_py(
        &mut self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
    ) -> PyResult<Option<VMPattern>> {
        if let Ok(tuple) = pattern.cast::<PyTuple>() {
            let mut elements = Vec::new();
            for item in tuple.iter() {
                if let Ok(star_tuple) = item.cast::<PyTuple>() {
                    if star_tuple.len() == 2 {
                        let first: String = star_tuple.get_item(0)?.extract().unwrap_or_default();
                        if first == "*" {
                            let star_name = self.extract_single_name_py(py, &star_tuple.get_item(1)?)?;
                            let star_slot = self.add_local(&star_name);
                            elements.push(VMPatternElement::Star(star_slot));
                            continue;
                        }
                    }
                }
                match self.try_compile_assign_pattern_py(py, &item)? {
                    Some(sub) => elements.push(VMPatternElement::Pattern(sub)),
                    None => return Ok(None),
                }
            }
            return Ok(Some(VMPattern::Tuple(elements)));
        }

        if let Ok(list) = pattern.cast::<PyList>() {
            let mut elements = Vec::new();
            for item in list.iter() {
                if let Ok(star_tuple) = item.cast::<PyTuple>() {
                    if star_tuple.len() == 2 {
                        let first: String = star_tuple.get_item(0)?.extract().unwrap_or_default();
                        if first == "*" {
                            let star_name = self.extract_single_name_py(py, &star_tuple.get_item(1)?)?;
                            let star_slot = self.add_local(&star_name);
                            elements.push(VMPatternElement::Star(star_slot));
                            continue;
                        }
                    }
                }
                match self.try_compile_assign_pattern_py(py, &item)? {
                    Some(sub) => elements.push(VMPatternElement::Pattern(sub)),
                    None => return Ok(None),
                }
            }
            return Ok(Some(VMPattern::Tuple(elements)));
        }

        // Ref, Lvalue, or plain string
        if let Ok(name) = self.extract_single_name_py(py, pattern) {
            let slot = self.add_local(&name);
            return Ok(Some(VMPattern::Var(slot)));
        }

        Ok(None)
    }

    /// Compile unpack pattern for for-loop tuple variable patterns.
    pub(crate) fn compile_unpack_pattern<'py>(
        &mut self,
        py: Python<'py>,
        pattern: &CompilerNode<'py>,
        keep_last: bool,
    ) -> PyResult<()> {
        match pattern {
            CompilerNode::Pure(ir) => self.compile_unpack_pattern_ir(ir, keep_last),
            CompilerNode::PyObj(obj) => self.compile_unpack_pattern_py(py, obj, keep_last),
        }
    }

    pub(crate) fn compile_unpack_pattern_ir(&mut self, ir: &IR, keep_last: bool) -> PyResult<()> {
        if let IR::Tuple(items) = ir {
            // Check for star pattern
            let mut star_idx: Option<usize> = None;
            for (i, item) in items.iter().enumerate() {
                if let IR::Tuple(pair) = item {
                    if pair.len() == 2 {
                        if let IR::String(s) = &pair[0] {
                            if s == "*" {
                                star_idx = Some(i);
                                break;
                            }
                        }
                    }
                }
            }

            if let Some(si) = star_idx {
                let before = si as u32;
                let after = (items.len() - si - 1) as u32;
                let arg = (before << 8) | after;
                self.emit(VMOpCode::UnpackEx, arg);
            } else {
                self.emit(VMOpCode::UnpackSequence, items.len() as u32);
            }

            for (idx, item) in items.iter().enumerate() {
                let is_last = idx == items.len() - 1;
                if is_last && keep_last {
                    self.emit(VMOpCode::DupTop, 0);
                }
                // Star pattern: store the rest list
                if let IR::Tuple(pair) = item {
                    if pair.len() == 2 {
                        if let IR::String(s) = &pair[0] {
                            if s == "*" {
                                let star_name = ir_to_name(&pair[1]).unwrap_or_default();
                                let slot = self.add_local(&star_name);
                                self.emit(VMOpCode::StoreLocal, slot as u32);
                                continue;
                            }
                        }
                    }
                }
                // Nested tuple pattern: recursive unpack
                if let IR::Tuple(_) = item {
                    self.compile_unpack_pattern_ir(item, false)?;
                    continue;
                }
                // Simple name
                if let Some(name) = ir_to_name(item) {
                    let slot = self.add_local(&name);
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn compile_unpack_pattern_py(
        &mut self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
        keep_last: bool,
    ) -> PyResult<()> {
        let len = pattern.len()?;

        // Find star pattern index
        let mut star_idx: i32 = -1;
        for i in 0..len {
            let item = pattern.get_item(i)?;
            if item.is_instance_of::<PyTuple>() && item.len()? == 2 {
                let first: String = item.get_item(0)?.extract().unwrap_or_default();
                if first == "*" {
                    star_idx = i as i32;
                    break;
                }
            }
        }

        if star_idx >= 0 {
            let before = star_idx as u32;
            let after = (len as i32 - star_idx - 1) as u32;
            let arg = (before << 8) | after;
            self.emit(VMOpCode::UnpackEx, arg);
        } else {
            self.emit(VMOpCode::UnpackSequence, len as u32);
        }

        let in_block = !self.loop_stack.is_empty();
        for idx in 0..len {
            let item = pattern.get_item(idx)?;
            let is_last = idx == len - 1;

            if is_last && keep_last {
                self.emit(VMOpCode::DupTop, 0);
            }

            if item.is_instance_of::<PyTuple>() && item.len()? == 2 {
                let first: String = item.get_item(0)?.extract().unwrap_or_default();
                if first == "*" {
                    let name = self.extract_single_name_py(py, &item.get_item(1)?)?;
                    let slot = self.add_local(&name);
                    if in_block {
                        self.emit(VMOpCode::StoreLocal, slot as u32);
                    } else {
                        let name_idx = self.add_name(&name);
                        self.emit(VMOpCode::StoreScope, name_idx as u32);
                    }
                    continue;
                }
            }

            if item.is_instance_of::<PyList>() || item.is_instance_of::<PyTuple>() {
                self.compile_unpack_pattern_py(py, &item, false)?;
            } else {
                let name = self.extract_single_name_py(py, &item)?;
                let slot = self.add_local(&name);
                if in_block {
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                } else {
                    let name_idx = self.add_name(&name);
                    self.emit(VMOpCode::StoreScope, name_idx as u32);
                }
            }
        }
        Ok(())
    }

    /// Extract a single variable name from a Python pattern node.
    pub(crate) fn extract_single_name_py(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<String> {
        use crate::types::catnip;

        if let Ok(s) = node.extract::<String>() {
            return Ok(s);
        }
        let node_type = node.get_type();
        let type_name = node_type.name()?;

        if type_name == catnip::LVALUE {
            return node.getattr("value")?.extract();
        }
        if type_name == catnip::REF {
            return node.getattr("ident")?.extract();
        }
        if type_name == "Identifier" {
            if let Ok(name) = node.getattr("name").and_then(|n| n.extract()) {
                return Ok(name);
            }
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "Cannot extract variable name from type: {}",
            type_name
        )))
    }
}
