// FILE: catnip_vm/src/compiler/structs.rs
use super::*;

impl PureCompiler {
    // ========== Struct/Trait ==========

    /// Compile a struct/trait method list into `(name, func_or_NIL, is_static)`
    /// entries; abstract methods (no lambda body) keep a NIL body slot.
    fn compile_method_list(&mut self, methods_node: &IR) -> CompileResult<Value> {
        let methods_items = match methods_node {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &[],
        };
        let mut compiled: Vec<Value> = Vec::new();
        for m in methods_items {
            let m_items = match m {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            let method_name = ir_to_name(&m_items[0]).unwrap_or_default();
            let is_static = if m_items.len() > 2 {
                matches!(&m_items[2], IR::Bool(true))
            } else {
                false
            };

            let lambda_node = &m_items[1];
            if is_none_ir(lambda_node) {
                let entry = Value::from_tuple(vec![
                    Value::from_string(method_name),
                    Value::NIL,
                    Value::from_bool(is_static),
                ]);
                compiled.push(entry);
                continue;
            }

            // Compile method body
            let lambda_items = match lambda_node {
                IR::Op { args, .. } => args.as_slice(),
                IR::Tuple(items) | IR::List(items) => items.as_slice(),
                _ => &[],
            };
            if lambda_items.len() >= 2 {
                let lambda_params = &lambda_items[0];
                let lambda_body = &lambda_items[1];
                let (param_names, defaults, vararg_idx, param_types) = self.extract_params(lambda_params)?;

                let mut code = self.compile_function_inner(FunctionCompileSpec {
                    params: param_names,
                    param_types,
                    body: lambda_body,
                    name: &method_name,
                    defaults,
                    vararg_idx,
                    parent_nesting_depth: self.nesting_depth,
                })?;
                code.encoded_ir = Self::freeze_ir_body(lambda_body, lambda_params);
                let func_idx = self.functions.len() as u32;
                self.functions.push(code);
                let entry = Value::from_tuple(vec![
                    Value::from_string(method_name),
                    Value::from_vmfunc(func_idx),
                    Value::from_bool(is_static),
                ]);
                compiled.push(entry);
            }
        }
        Ok(Value::from_list(compiled))
    }

    pub(crate) fn compile_struct(&mut self, args: &[IR]) -> CompileResult<()> {
        let name = ir_to_name(&args[0]).unwrap_or_else(|| "<struct>".to_string());

        let fields_items = match &args[1] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &[],
        };
        let args_len = args.len();

        let mut implements_list: Vec<String> = Vec::new();
        let mut base_names: Vec<String> = Vec::new();
        let mut methods_index: Option<usize> = None;

        if args_len > 3 {
            if let IR::Tuple(impl_items) | IR::List(impl_items) = &args[2] {
                for imp in impl_items {
                    if let Some(s) = ir_to_name(imp) {
                        implements_list.push(s);
                    }
                }
            }
            if !is_none_ir(&args[3]) {
                if let IR::Tuple(base_items) | IR::List(base_items) = &args[3] {
                    for b in base_items {
                        if let Some(s) = ir_to_name(b) {
                            base_names.push(s);
                        }
                    }
                } else if let Some(s) = ir_to_name(&args[3]) {
                    base_names.push(s);
                }
            }
            if args_len > 4 {
                methods_index = Some(4);
            }
        } else if args_len > 2 {
            if let Some(s) = ir_to_name(&args[2]) {
                base_names.push(s);
                if args_len > 3 {
                    methods_index = Some(3);
                }
            } else if let IR::Tuple(impl_items) | IR::List(impl_items) = &args[2] {
                let mut is_impl_list = true;
                for imp in impl_items {
                    if let Some(s) = ir_to_name(imp) {
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

        // Build fields info as NativeTuple: ((name, has_default), ...)
        let mut fields_info: Vec<Value> = Vec::new();
        let mut num_defaults: u32 = 0;

        for field in fields_items {
            let items = match field {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            if items.len() >= 2 {
                let fname = ir_to_name(&items[0]).unwrap_or_default();
                let has_default = matches!(&items[1], IR::Bool(true));
                if has_default && items.len() >= 3 {
                    self.compile_node(&items[2])?;
                    num_defaults += 1;
                }
                // Field IR is (name, has_default, default, type_or_none); carry the
                // raw annotation text so MakeStruct can classify a runtime check.
                let entry = match items.get(3) {
                    Some(IR::String(t)) => Value::from_tuple(vec![
                        Value::from_string(fname),
                        Value::from_bool(has_default),
                        Value::from_string(t.clone()),
                    ]),
                    _ => Value::from_tuple(vec![Value::from_string(fname), Value::from_bool(has_default)]),
                };
                fields_info.push(entry);
            }
        }
        let fields_tuple = Value::from_tuple(fields_info);

        // Compile methods
        let methods_val = match methods_index {
            Some(idx) => Some(self.compile_method_list(&args[idx])?),
            None => None,
        };

        // Build struct info constant as NativeTuple
        let has_implements = !implements_list.is_empty();
        let has_bases = !base_names.is_empty();

        let struct_info = if has_implements || has_bases {
            let impl_tuple = Value::from_tuple(implements_list.iter().map(|s| Value::from_string(s.clone())).collect());
            let bases_val = if has_bases {
                Value::from_tuple(base_names.iter().map(|s| Value::from_string(s.clone())).collect())
            } else {
                Value::NIL
            };
            let mut items = vec![
                Value::from_string(name),
                fields_tuple,
                Value::from_i64(num_defaults as i64),
                impl_tuple,
                bases_val,
            ];
            if let Some(methods) = methods_val {
                items.push(methods);
            }
            Value::from_tuple(items)
        } else {
            match methods_val {
                Some(methods) => Value::from_tuple(vec![
                    Value::from_string(name),
                    fields_tuple,
                    Value::from_i64(num_defaults as i64),
                    methods,
                ]),
                None => Value::from_tuple(vec![
                    Value::from_string(name),
                    fields_tuple,
                    Value::from_i64(num_defaults as i64),
                ]),
            }
        };

        let idx = self.core.add_const(struct_info);
        self.emit(VMOpCode::MakeStruct, idx as u32);
        Ok(())
    }

    pub(crate) fn compile_trait(&mut self, args: &[IR]) -> CompileResult<()> {
        let name = ir_to_name(&args[0]).unwrap_or_default();

        let extends_items = match &args[1] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &[],
        };
        let fields_items = match &args[2] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &[],
        };

        let extends_tuple = Value::from_tuple(
            extends_items
                .iter()
                .filter_map(ir_to_name)
                .map(Value::from_string)
                .collect(),
        );

        let mut fields_info: Vec<Value> = Vec::new();
        let mut num_defaults: u32 = 0;
        for f in fields_items {
            let f_items = match f {
                IR::Tuple(items) | IR::List(items) => items,
                _ => continue,
            };
            if f_items.len() >= 2 {
                let fname = ir_to_name(&f_items[0]).unwrap_or_default();
                let has_default = !is_none_ir(&f_items[1]);
                if has_default {
                    self.compile_node(&f_items[1])?;
                    num_defaults += 1;
                }
                let entry = Value::from_tuple(vec![Value::from_string(fname), Value::from_bool(has_default)]);
                fields_info.push(entry);
            }
        }
        let fields_tuple = Value::from_tuple(fields_info);

        let methods_val = if args.len() > 3 {
            Some(self.compile_method_list(&args[3])?)
        } else {
            None
        };

        let trait_info = if let Some(methods) = methods_val {
            Value::from_tuple(vec![
                Value::from_string(name),
                extends_tuple,
                fields_tuple,
                Value::from_i64(num_defaults as i64),
                methods,
            ])
        } else {
            Value::from_tuple(vec![
                Value::from_string(name),
                extends_tuple,
                fields_tuple,
                Value::from_i64(num_defaults as i64),
            ])
        };

        let idx = self.core.add_const(trait_info);
        self.emit(VMOpCode::MakeTrait, idx as u32);
        Ok(())
    }

    pub(crate) fn compile_enum(&mut self, args: &[IR]) -> CompileResult<()> {
        let name = ir_to_name(&args[0]).unwrap_or_default();

        let variant_items = match &args[1] {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => &[],
        };

        let variants_tuple = Value::from_tuple(
            variant_items
                .iter()
                .filter_map(ir_to_name)
                .map(Value::from_string)
                .collect(),
        );

        let enum_info = Value::from_tuple(vec![Value::from_string(name), variants_tuple]);
        let idx = self.core.add_const(enum_info);
        self.emit(VMOpCode::MakeEnum, idx as u32);
        Ok(())
    }

    /// Compile a union (ADT) definition.
    ///
    /// IR layout: `UnionDef(name, type_params, variants[, methods])` where
    /// each variant is `(variant_name, fields_list)`, each field is
    /// `(field_name, type_text_or_none)`, and each method is
    /// `(method_name, lambda)`.
    ///
    /// Emitted bytecode: one constant tuple `(name, type_params_tuple,
    /// variants_tuple[, methods_list])` referenced by a single `MakeUnion`
    /// opcode. Both VM handlers (PyO3 `OpCode::MakeUnion` and PureVM
    /// `handle_make_union`) decode the same layout.
    pub(crate) fn compile_union(&mut self, args: &[IR]) -> CompileResult<()> {
        let name = ir_to_name(&args[0]).unwrap_or_default();

        // Type parameters: list/tuple of identifier strings.
        let type_param_items = match args.get(1) {
            Some(IR::List(items) | IR::Tuple(items)) => items.as_slice(),
            _ => &[],
        };
        let type_params_tuple = Value::from_tuple(
            type_param_items
                .iter()
                .filter_map(ir_to_name)
                .map(Value::from_string)
                .collect(),
        );

        // Variants: each is emitted as (variant_name, field_names, field_types),
        // where field_types carries the raw annotation text (empty string when a
        // field is unannotated), parallel to field_names. The type text drives the
        // generic-nominal boundary (`Option[int]`): `handle_make_union` classifies
        // each field into a `FieldTemplate` against the union's type parameters.
        let variant_items = match args.get(2) {
            Some(IR::List(items) | IR::Tuple(items)) => items.as_slice(),
            _ => &[],
        };
        let mut variant_tuples = Vec::with_capacity(variant_items.len());
        for variant in variant_items {
            let parts = match variant {
                IR::Tuple(parts) | IR::List(parts) if parts.len() >= 2 => parts,
                _ => continue,
            };
            let variant_name = ir_to_name(&parts[0]).unwrap_or_default();
            let fields_slice = match &parts[1] {
                IR::List(items) | IR::Tuple(items) => items.as_slice(),
                _ => &[],
            };
            let mut field_names: Vec<Value> = Vec::with_capacity(fields_slice.len());
            let mut field_types: Vec<Value> = Vec::with_capacity(fields_slice.len());
            for field in fields_slice {
                let (fname, ftype) = match field {
                    IR::Tuple(pair) | IR::List(pair) if !pair.is_empty() => {
                        let ftype = match pair.get(1) {
                            Some(IR::String(s)) => s.clone(),
                            _ => String::new(),
                        };
                        (ir_to_name(&pair[0]), ftype)
                    }
                    other => (ir_to_name(other), String::new()),
                };
                if let Some(fname) = fname {
                    field_names.push(Value::from_string(fname));
                    field_types.push(Value::from_string(ftype));
                }
            }
            variant_tuples.push(Value::from_tuple(vec![
                Value::from_string(variant_name),
                Value::from_tuple(field_names),
                Value::from_tuple(field_types),
            ]));
        }
        let variants_tuple = Value::from_tuple(variant_tuples);

        // Methods: each is (method_name, lambda). Compiled like struct
        // methods -- one function slot per method, no static/abstract forms.
        let methods_val = match args.get(3) {
            Some(IR::List(items) | IR::Tuple(items)) if !items.is_empty() => {
                let mut compiled: Vec<Value> = Vec::new();
                for m in items.as_slice() {
                    let m_items = match m {
                        IR::Tuple(items) | IR::List(items) => items,
                        _ => continue,
                    };
                    let method_name = ir_to_name(&m_items[0]).unwrap_or_default();
                    let lambda_items = match &m_items[1] {
                        IR::Op { args, .. } => args.as_slice(),
                        IR::Tuple(items) | IR::List(items) => items.as_slice(),
                        _ => &[],
                    };
                    if lambda_items.len() >= 2 {
                        let (param_names, defaults, vararg_idx, param_types) = self.extract_params(&lambda_items[0])?;
                        let mut code = self.compile_function_inner(FunctionCompileSpec {
                            params: param_names,
                            param_types,
                            body: &lambda_items[1],
                            name: &method_name,
                            defaults,
                            vararg_idx,
                            parent_nesting_depth: self.nesting_depth,
                        })?;
                        code.encoded_ir = Self::freeze_ir_body(&lambda_items[1], &lambda_items[0]);
                        let func_idx = self.functions.len() as u32;
                        self.functions.push(code);
                        compiled.push(Value::from_tuple(vec![
                            Value::from_string(method_name),
                            Value::from_vmfunc(func_idx),
                        ]));
                    }
                }
                Some(Value::from_list(compiled))
            }
            _ => None,
        };

        let mut info_items = vec![Value::from_string(name), type_params_tuple, variants_tuple];
        if let Some(methods) = methods_val {
            info_items.push(methods);
        }
        let union_info = Value::from_tuple(info_items);
        let idx = self.core.add_const(union_info);
        self.emit(VMOpCode::MakeUnion, idx as u32);
        Ok(())
    }
}
