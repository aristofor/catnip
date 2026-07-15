// FILE: catnip_vm/src/compiler/helpers.rs
use super::*;
use catnip_core::vm::opcode::ParamCheck;

/// Result of `extract_params`: (param_names, defaults, vararg_idx, param_checks).
/// `param_checks` holds the prologue boundary check per param (TH2-B primitive
/// `CheckType` + enforcement nominal `CheckNominal`), or `None` when unannotated
/// or not enforceable.
type ExtractedParams = (Vec<String>, Vec<Value>, i32, Vec<ParamCheck>);

impl PureCompiler {
    // ========== Helpers ==========

    pub(crate) fn body_has_calls(&self, node: &IR) -> bool {
        match node {
            IR::Op { opcode, args, .. } => {
                if *opcode == IROpCode::Call || *opcode == IROpCode::FnDef || *opcode == IROpCode::OpLambda {
                    return true;
                }
                args.iter().any(|a| self.body_has_calls(a))
            }
            IR::Call { .. } => true,
            IR::List(items) | IR::Tuple(items) | IR::Program(items) => items.iter().any(|i| self.body_has_calls(i)),
            _ => false,
        }
    }

    pub(crate) fn extract_params(&self, params: &IR) -> CompileResult<ExtractedParams> {
        let mut param_names = Vec::new();
        let mut defaults = Vec::new();
        let mut vararg_idx: i32 = -1;
        // Prologue boundary check per param, aligned with `param_names`: a
        // primitive `CheckType` code, a nominal type name, or none.
        let mut param_types: Vec<ParamCheck> = Vec::new();

        let children = match params {
            IR::Tuple(items) | IR::List(items) => items.as_slice(),
            _ => return Ok((param_names, defaults, vararg_idx, param_types)),
        };

        for item in children {
            let item_parts = match item {
                IR::Tuple(items) | IR::List(items) => Some(items.as_slice()),
                _ => None,
            };
            if let Some(parts) = item_parts {
                if parts.len() >= 2 {
                    let name = ir_to_name(&parts[0]).unwrap_or_default();
                    // Variadic marker ("*", vararg_name) stays a 2-element tuple.
                    if parts.len() == 2 && name == "*" {
                        vararg_idx = param_names.len() as i32;
                        param_names.push(ir_to_name(&parts[1]).unwrap_or_default());
                        param_types.push(ParamCheck::None);
                    } else {
                        // Regular param (name, default[, type]); index 2 maps to a
                        // primitive `CheckType` code or a nominal type name.
                        param_names.push(name);
                        let val = self.ir_to_value(&parts[1]);
                        defaults.push(val);
                        let check = parts
                            .get(2)
                            .and_then(ir_to_name)
                            .map(|n| ParamCheck::from_annotation(&n))
                            .unwrap_or(ParamCheck::None);
                        param_types.push(check);
                    }
                } else if let Some(name) = ir_to_name(item) {
                    param_names.push(name);
                    param_types.push(ParamCheck::None);
                }
            } else if let Some(name) = ir_to_name(item) {
                param_names.push(name);
                param_types.push(ParamCheck::None);
            }
        }
        Ok((param_names, defaults, vararg_idx, param_types))
    }

    fn ir_to_value(&self, ir: &IR) -> Value {
        match ir {
            IR::Int(n) => Value::from_i64(*n),
            IR::Float(f) => Value::from_float(*f),
            IR::Bool(b) => Value::from_bool(*b),
            IR::None => Value::NIL,
            IR::String(s) => Value::from_string(s.clone()),
            _ => Value::NIL,
        }
    }

    pub(crate) fn try_compile_pattern_ir(&mut self, pattern: &IR) -> CompileResult<Option<VMPattern>> {
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
                let val = self.ir_to_value(value);
                Ok(Some(VMPattern::Literal(val)))
            }
            IR::PatternOr(patterns) => {
                let mut sub_patterns = Vec::new();
                for p in patterns {
                    match self.try_compile_pattern_ir(p)? {
                        Some(vp) => sub_patterns.push(vp),
                        None => return Ok(None),
                    }
                }
                Ok(Some(VMPattern::Or(sub_patterns)))
            }
            IR::PatternTuple(patterns) => {
                let mut elements = Vec::new();
                for p in patterns {
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
                    match self.try_compile_pattern_ir(p)? {
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

    pub(crate) fn try_compile_assign_pattern_ir(&mut self, pattern: &IR) -> CompileResult<Option<VMPattern>> {
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

    pub(crate) fn compile_unpack_pattern_ir(&mut self, ir: &IR, keep_last: bool) -> CompileResult<()> {
        if let IR::Tuple(items) = ir {
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
                if let IR::Tuple(_) = item {
                    self.compile_unpack_pattern_ir(item, false)?;
                    continue;
                }
                if let Some(name) = ir_to_name(item) {
                    let slot = self.add_local(&name);
                    self.emit(VMOpCode::StoreLocal, slot as u32);
                }
            }
        }
        Ok(())
    }

    /// Freeze the IR body of a lambda/function for ND process workers.
    pub fn freeze_ir_body(body: &IR, params: &IR) -> Option<Arc<Vec<u8>>> {
        // Capture the params (with their type annotations) alongside the body so
        // an ND `process` worker, which recompiles from this frozen IR, can
        // rebuild the typed-param boundary checks (TH2-B 0b) instead of dropping
        // them. Element 0 is the body; element 1 is the params node.
        let ir_vec = vec![body.clone(), params.clone()];
        catnip_core::freeze::encode(&ir_vec).ok().map(Arc::new)
    }
}
