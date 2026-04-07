// FILE: catnip_core/src/pipeline/semantic.rs
//! Standalone semantic analyzer - IR → OpPure validation
//!
//! Port of semantic/analyzer.rs with no PyO3 dependencies.
//! Simple validation without optimizations.

use std::collections::{HashMap, HashSet};

use crate::constants::*;
use crate::ir::{IR, IROpCode};
use crate::semantic::PureOptimizer;

use super::diagnostic::{AnalysisResult, SemanticDiagnostic, SemanticSeverity};

/// Inferred type for a variable (minimal)
#[derive(Debug, Clone, PartialEq)]
enum InferredType {
    Enum(String),
    Bool,
}

/// Semantic analyzer standalone
pub struct SemanticAnalyzer {
    /// Valid opcodes (static table)
    valid_opcodes: Vec<IROpCode>,
    /// Pure optimization passes
    optimizer: Option<PureOptimizer>,
    /// Tail-call optimization enabled
    tco_enabled: bool,
    /// Known enum definitions: name -> variant names
    enum_defs: HashMap<String, HashSet<String>>,
    /// Variable type bindings
    var_types: HashMap<String, InferredType>,
    /// Non-fatal diagnostics collected during analysis
    diagnostics: Vec<SemanticDiagnostic>,
}

impl SemanticAnalyzer {
    /// Create a new analyzer (no optimization passes)
    pub fn new() -> Self {
        Self {
            valid_opcodes: Self::all_opcodes(),
            optimizer: None,
            tco_enabled: true,
            enum_defs: HashMap::new(),
            var_types: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Create a new analyzer with optimization passes enabled
    pub fn with_optimizer() -> Self {
        Self {
            valid_opcodes: Self::all_opcodes(),
            optimizer: Some(PureOptimizer::new()),
            tco_enabled: true,
            enum_defs: HashMap::new(),
            var_types: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Enable or disable tail-call optimization marking.
    pub fn set_tco_enabled(&mut self, enabled: bool) {
        self.tco_enabled = enabled;
    }

    /// Analyze, transform, optimize and validate the IR
    pub fn analyze(&mut self, ir: &IR) -> Result<IR, String> {
        Ok(self.analyze_full(ir)?.ir)
    }

    /// Full analysis returning IR + non-fatal diagnostics
    pub fn analyze_full(&mut self, ir: &IR) -> Result<AnalysisResult, String> {
        self.enum_defs.clear();
        self.var_types.clear();
        self.diagnostics.clear();

        let transformed = self.transform(ir);
        let tail_marked = if self.tco_enabled {
            Self::mark_tail_calls(&transformed)
        } else {
            transformed
        };
        let optimized = if let Some(ref mut optimizer) = self.optimizer {
            optimizer.optimize(tail_marked)
        } else {
            tail_marked
        };
        self.validate(&optimized)?;
        self.check_exhaustiveness(&optimized);

        Ok(AnalysisResult {
            ir: optimized,
            diagnostics: std::mem::take(&mut self.diagnostics),
        })
    }

    /// Transform the IR: intercept intrinsic calls (type, breakpoint)
    fn transform(&self, ir: &IR) -> IR {
        match ir {
            // Intercept Call nodes for intrinsic functions
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
                ..
            } => {
                let func_name = match func.as_ref() {
                    IR::Identifier(n) => Some(n.as_str()),
                    IR::Ref(n, _, _) => Some(n.as_str()),
                    _ => None,
                };
                if let Some(name) = func_name {
                    // typeof(expr) → Op(TypeOf, [expr])
                    if name == "typeof" && args.len() == 1 && kwargs.is_empty() {
                        return IR::Op {
                            opcode: IROpCode::TypeOf,
                            args: vec![self.transform(&args[0])],
                            kwargs: indexmap::IndexMap::new(),
                            tail: false,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                    // breakpoint() → Op(Breakpoint, [])
                    if name == "breakpoint" && args.is_empty() && kwargs.is_empty() {
                        return IR::Op {
                            opcode: IROpCode::Breakpoint,
                            args: vec![],
                            kwargs: indexmap::IndexMap::new(),
                            tail: false,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                    // globals() → Op(Globals, [])
                    if name == "globals" && args.is_empty() && kwargs.is_empty() {
                        return IR::Op {
                            opcode: IROpCode::Globals,
                            args: vec![],
                            kwargs: indexmap::IndexMap::new(),
                            tail: false,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                    // locals() → Op(Locals, [])
                    if name == "locals" && args.is_empty() && kwargs.is_empty() {
                        return IR::Op {
                            opcode: IROpCode::Locals,
                            args: vec![],
                            kwargs: indexmap::IndexMap::new(),
                            tail: false,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                }
                // Not an intrinsic - transform children
                IR::Call {
                    func: Box::new(self.transform(func)),
                    args: args.iter().map(|a| self.transform(a)).collect(),
                    kwargs: kwargs.iter().map(|(k, v)| (k.clone(), self.transform(v))).collect(),
                    tail: false,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                }
            }

            // Recursively transform all node types
            IR::Program(items) => IR::Program(items.iter().map(|i| self.transform(i)).collect()),
            IR::List(items) => IR::List(items.iter().map(|i| self.transform(i)).collect()),
            IR::Tuple(items) => IR::Tuple(items.iter().map(|i| self.transform(i)).collect()),
            IR::Set(items) => IR::Set(items.iter().map(|i| self.transform(i)).collect()),
            IR::Dict(pairs) => IR::Dict(
                pairs
                    .iter()
                    .map(|(k, v)| (self.transform(k), self.transform(v)))
                    .collect(),
            ),
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => IR::Op {
                opcode: *opcode,
                args: args.iter().map(|a| self.transform(a)).collect(),
                kwargs: kwargs.iter().map(|(k, v)| (k.clone(), self.transform(v))).collect(),
                tail: *tail,
                start_byte: *start_byte,
                end_byte: *end_byte,
            },
            IR::PatternLiteral(v) => IR::PatternLiteral(Box::new(self.transform(v))),
            IR::PatternOr(ps) => IR::PatternOr(ps.iter().map(|p| self.transform(p)).collect()),
            IR::PatternTuple(ps) => IR::PatternTuple(ps.iter().map(|p| self.transform(p)).collect()),
            IR::Slice { start, stop, step } => IR::Slice {
                start: Box::new(self.transform(start)),
                stop: Box::new(self.transform(stop)),
                step: Box::new(self.transform(step)),
            },
            IR::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => IR::Broadcast {
                target: target.as_ref().map(|t| Box::new(self.transform(t))),
                operator: Box::new(self.transform(operator)),
                operand: operand.as_ref().map(|o| Box::new(self.transform(o))),
                broadcast_type: broadcast_type.clone(),
            },

            // Leaf nodes - return as-is
            _ => ir.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // Tail-call marking
    // -----------------------------------------------------------------------

    /// Mark tail calls in named lambdas for TCO.
    ///
    /// Traverses the IR looking for `SetLocals([name], OpLambda(...))` patterns.
    /// Inside each lambda body, calls to `name` in tail position get `tail: true`.
    fn mark_tail_calls(ir: &IR) -> IR {
        match ir {
            IR::Program(items) => IR::Program(items.iter().map(Self::mark_tail_calls).collect()),

            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } if *opcode == IROpCode::SetLocals => {
                // SetLocals([names...], value, is_const)
                // Detect: single name + OpLambda value
                if args.len() >= 2 {
                    let func_name = Self::extract_single_name(&args[0]);
                    let is_lambda = matches!(&args[1], IR::Op { opcode, .. } if *opcode == IROpCode::OpLambda);

                    if let (Some(name), true) = (func_name, is_lambda) {
                        let marked_value = Self::mark_tail_in_lambda(&args[1], &name);
                        let mut new_args = vec![args[0].clone(), marked_value];
                        new_args.extend(args[2..].iter().cloned());
                        return IR::Op {
                            opcode: *opcode,
                            args: new_args,
                            kwargs: kwargs.clone(),
                            tail: *tail,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                }
                // Not a named lambda - recurse normally
                IR::Op {
                    opcode: *opcode,
                    args: args.iter().map(Self::mark_tail_calls).collect(),
                    kwargs: kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), Self::mark_tail_calls(v)))
                        .collect(),
                    tail: *tail,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                }
            }

            // Recurse into other Op nodes
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => IR::Op {
                opcode: *opcode,
                args: args.iter().map(Self::mark_tail_calls).collect(),
                kwargs: kwargs
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::mark_tail_calls(v)))
                    .collect(),
                tail: *tail,
                start_byte: *start_byte,
                end_byte: *end_byte,
            },

            // Don't recurse into Call children at this level (handled inside lambdas)
            _ => ir.clone(),
        }
    }

    /// Extract a single variable name from a SetLocals names argument.
    /// Returns None if not a single-name assignment.
    fn extract_single_name(names_ir: &IR) -> Option<String> {
        match names_ir {
            IR::List(items) | IR::Tuple(items) if items.len() == 1 => match &items[0] {
                IR::Ref(name, _, _) | IR::Identifier(name) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Mark tail calls inside a lambda body.
    fn mark_tail_in_lambda(lambda: &IR, func_name: &str) -> IR {
        if let IR::Op {
            opcode,
            args,
            kwargs,
            tail,
            start_byte,
            end_byte,
        } = lambda
        {
            if *opcode == IROpCode::OpLambda && args.len() >= 2 {
                let params = args[0].clone();
                let body = Self::mark_tail_in_body(&args[1], func_name);
                let mut new_args = vec![params, body];
                new_args.extend(args[2..].iter().cloned());
                return IR::Op {
                    opcode: *opcode,
                    args: new_args,
                    kwargs: kwargs.clone(),
                    tail: *tail,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                };
            }
        }
        lambda.clone()
    }

    /// Mark tail calls in a body expression (recursive).
    /// `in_tail` is true when the expression is in tail position.
    fn mark_tail_in_body(ir: &IR, func_name: &str) -> IR {
        match ir {
            // Call: mark as tail if it's a direct call to func_name
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
                ..
            } => {
                let is_self_call = match func.as_ref() {
                    IR::Ref(n, _, _) | IR::Identifier(n) => n == func_name,
                    _ => false,
                };
                IR::Call {
                    func: func.clone(),
                    args: args.clone(), // don't recurse into call args
                    kwargs: kwargs.clone(),
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                    tail: is_self_call,
                }
            }

            // Block: only the last statement is in tail position
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } if *opcode == IROpCode::OpBlock => {
                let new_args: Vec<IR> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        if i == args.len() - 1 {
                            Self::mark_tail_in_body(a, func_name)
                        } else {
                            a.clone()
                        }
                    })
                    .collect();
                IR::Op {
                    opcode: *opcode,
                    args: new_args,
                    kwargs: kwargs.clone(),
                    tail: *tail,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                }
            }

            // If: both branches are in tail position
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } if *opcode == IROpCode::OpIf => {
                // OpIf args: [branches_list, else_block]
                let new_args: Vec<IR> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        match (i, a) {
                            // First arg: list of (condition, block) tuples
                            (0, IR::List(branches)) | (0, IR::Tuple(branches)) => {
                                let marked: Vec<IR> = branches
                                    .iter()
                                    .map(|branch| {
                                        if let IR::Tuple(pair) = branch {
                                            if pair.len() == 2 {
                                                let cond = pair[0].clone();
                                                let block = Self::mark_tail_in_body(&pair[1], func_name);
                                                IR::Tuple(vec![cond, block])
                                            } else {
                                                branch.clone()
                                            }
                                        } else {
                                            branch.clone()
                                        }
                                    })
                                    .collect();
                                if matches!(a, IR::Tuple(_)) {
                                    IR::Tuple(marked)
                                } else {
                                    IR::List(marked)
                                }
                            }
                            // Second arg: else block (tail position)
                            (1, _) => Self::mark_tail_in_body(a, func_name),
                            _ => a.clone(),
                        }
                    })
                    .collect();
                IR::Op {
                    opcode: *opcode,
                    args: new_args,
                    kwargs: kwargs.clone(),
                    tail: *tail,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                }
            }

            // Return: the inner expression is in tail position
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } if *opcode == IROpCode::OpReturn && !args.is_empty() => {
                let new_args = vec![Self::mark_tail_in_body(&args[0], func_name)];
                IR::Op {
                    opcode: *opcode,
                    args: new_args,
                    kwargs: kwargs.clone(),
                    tail: *tail,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                }
            }

            // Everything else: not in tail position, return as-is
            _ => ir.clone(),
        }
    }

    /// Recursively validate the IR
    fn validate(&self, ir: &IR) -> Result<(), String> {
        match ir {
            // Literals sont toujours valides
            IR::Int(_)
            | IR::Float(_)
            | IR::String(_)
            | IR::Bytes(_)
            | IR::Bool(_)
            | IR::None
            | IR::Decimal(_)
            | IR::Imaginary(_) => Ok(()),

            // Identifiers et Refs
            IR::Identifier(_) | IR::Ref(..) => Ok(()),

            // Collections + Program
            IR::Program(items) | IR::List(items) | IR::Tuple(items) | IR::Set(items) => {
                for item in items {
                    self.validate(item)?;
                }
                Ok(())
            }

            IR::Dict(pairs) => {
                for (key, value) in pairs {
                    self.validate(key)?;
                    self.validate(value)?;
                }
                Ok(())
            }

            // Function calls
            IR::Call { func, args, kwargs, .. } => {
                self.validate(func)?;
                for arg in args {
                    self.validate(arg)?;
                }
                for (_, value) in kwargs {
                    self.validate(value)?;
                }
                Ok(())
            }

            // Operations
            IR::Op {
                opcode,
                args,
                kwargs,
                start_byte,
                ..
            } => {
                // Vérifier que l'opcode existe
                if !self.valid_opcodes.contains(opcode) {
                    return Err(format!("Unknown opcode: {:?}", opcode));
                }

                // Validate pragma directives and values
                if *opcode == IROpCode::Pragma && args.len() >= 2 {
                    self.validate_pragma(args, *start_byte)?;
                }

                // Valider les arguments
                for arg in args {
                    self.validate(arg)?;
                }

                // Valider les kwargs
                for (_, value) in kwargs {
                    self.validate(value)?;
                }

                Ok(())
            }

            // Pattern matching
            IR::PatternLiteral(value) => self.validate(value),
            IR::PatternVar(_) => Ok(()),
            IR::PatternWildcard => Ok(()),
            IR::PatternOr(patterns) | IR::PatternTuple(patterns) => {
                for pattern in patterns {
                    self.validate(pattern)?;
                }
                Ok(())
            }
            IR::PatternStruct { .. } => Ok(()),
            IR::PatternEnum { .. } => Ok(()),

            // Slice
            IR::Slice { start, stop, step } => {
                self.validate(start)?;
                self.validate(stop)?;
                self.validate(step)
            }

            // Broadcast
            IR::Broadcast {
                target,
                operator,
                operand,
                ..
            } => {
                if let Some(t) = target {
                    self.validate(t)?;
                }
                self.validate(operator)?;
                if let Some(o) = operand {
                    self.validate(o)?;
                }
                Ok(())
            }
        }
    }

    /// Validate pragma directive name and value.
    /// Errors are prefixed with `@pragma:BYTE ` for downstream position enrichment.
    fn validate_pragma(&self, args: &[IR], start_byte: usize) -> Result<(), String> {
        let directive = match &args[0] {
            IR::String(s) => s.to_lowercase(),
            _ => return Ok(()), // non-string directive caught elsewhere
        };

        let pragma_err = |msg: String| -> Result<(), String> { Err(format!("@pragma:{} {}", start_byte, msg)) };

        if !PRAGMA_DIRECTIVES.contains(&directive.as_str()) {
            let known = PRAGMA_DIRECTIVES.join(", ");
            return pragma_err(format!("Unknown pragma directive: '{}'. Known: {}", directive, known));
        }

        match directive.as_str() {
            d if PRAGMA_BOOL.contains(&d) => match &args[1] {
                IR::Bool(_) => {}
                _ => return pragma_err(format!("Pragma '{}' requires True or False", directive)),
            },
            PRAGMA_OPTIMIZE => match &args[1] {
                IR::Int(n) if *n >= 0 && *n <= OPTIMIZE_MAX => {}
                IR::Int(n) => return pragma_err(format!("Optimization level must be 0-{}, got {}", OPTIMIZE_MAX, n)),
                _ => {
                    return pragma_err(format!(
                        "Pragma '{}' requires an integer 0-{}",
                        PRAGMA_OPTIMIZE, OPTIMIZE_MAX
                    ));
                }
            },
            PRAGMA_JIT => match &args[1] {
                IR::Bool(_) => {}
                IR::String(s) if s == "all" => {}
                _ => return pragma_err(format!("Pragma '{}' requires True, False, or \"all\"", PRAGMA_JIT)),
            },
            PRAGMA_ND_MODE => match &args[1] {
                IR::String(s) if ND_MODE_VALUES.contains(&s.to_lowercase().as_str()) => {}
                IR::String(s) => {
                    return pragma_err(format!(
                        "Unknown ND mode: '{}'. Use ND.sequential, ND.thread, or ND.process",
                        s
                    ));
                }
                _ => {
                    return pragma_err(format!(
                        "Pragma '{}' requires ND.sequential, ND.thread, or ND.process",
                        PRAGMA_ND_MODE
                    ));
                }
            },
            d if PRAGMA_UINT.contains(&d) => match &args[1] {
                IR::Int(n) if *n < 0 => return pragma_err(format!("Pragma '{}' must be non-negative, got {}", d, n)),
                IR::Int(_) => {}
                _ => return pragma_err(format!("Pragma '{}' requires a non-negative integer", d)),
            },
            d if PRAGMA_DEFERRED.contains(&d) => {} // validated elsewhere
            _ => {}                                 // covered by PRAGMA_DIRECTIVES check above
        }
        Ok(())
    }

    /// Return all valid opcodes
    fn all_opcodes() -> Vec<IROpCode> {
        vec![
            IROpCode::Nop,
            IROpCode::OpIf,
            IROpCode::OpWhile,
            IROpCode::OpFor,
            IROpCode::OpMatch,
            IROpCode::OpBlock,
            IROpCode::OpReturn,
            IROpCode::OpBreak,
            IROpCode::OpContinue,
            IROpCode::Call,
            IROpCode::OpLambda,
            IROpCode::FnDef,
            IROpCode::SetLocals,
            IROpCode::GetAttr,
            IROpCode::SetAttr,
            IROpCode::GetItem,
            IROpCode::SetItem,
            IROpCode::Slice,
            IROpCode::Add,
            IROpCode::Sub,
            IROpCode::Mul,
            IROpCode::Div,
            IROpCode::TrueDiv,
            IROpCode::FloorDiv,
            IROpCode::Mod,
            IROpCode::Pow,
            IROpCode::Neg,
            IROpCode::Pos,
            IROpCode::Eq,
            IROpCode::Ne,
            IROpCode::Lt,
            IROpCode::Le,
            IROpCode::Gt,
            IROpCode::Ge,
            IROpCode::And,
            IROpCode::Or,
            IROpCode::Not,
            IROpCode::BAnd,
            IROpCode::BOr,
            IROpCode::BXor,
            IROpCode::BNot,
            IROpCode::LShift,
            IROpCode::RShift,
            IROpCode::Broadcast,
            IROpCode::ListLiteral,
            IROpCode::TupleLiteral,
            IROpCode::SetLiteral,
            IROpCode::DictLiteral,
            IROpCode::Push,
            IROpCode::Pop,
            IROpCode::PushPeek,
            IROpCode::Fstring,
            IROpCode::Pragma,
            IROpCode::NdRecursion,
            IROpCode::NdMap,
            IROpCode::NdEmptyTopos,
            IROpCode::Breakpoint,
            IROpCode::OpStruct,
            IROpCode::TraitDef,
            IROpCode::In,
            IROpCode::NotIn,
            IROpCode::Is,
            IROpCode::IsNot,
            IROpCode::NullCoalesce,
            IROpCode::TypeOf,
            IROpCode::Globals,
            IROpCode::Locals,
            IROpCode::EnumDef,
            IROpCode::OpTry,
            IROpCode::OpRaise,
            IROpCode::ExcInfo,
        ]
    }

    // -----------------------------------------------------------------------
    // Exhaustiveness checking (I103)
    // -----------------------------------------------------------------------

    /// Walk the IR collecting enum defs, tracking variable types,
    /// and checking match exhaustiveness.
    fn check_exhaustiveness(&mut self, ir: &IR) {
        match ir {
            IR::Program(items) => {
                for item in items {
                    self.check_exhaustiveness(item);
                }
            }

            // Register enum definitions
            IR::Op { opcode, args, .. } if *opcode == IROpCode::EnumDef => {
                if let (Some(IR::String(name)), Some(IR::Tuple(variants))) = (args.first(), args.get(1)) {
                    let variant_set: HashSet<String> = variants
                        .iter()
                        .filter_map(|v| if let IR::String(s) = v { Some(s.clone()) } else { None })
                        .collect();
                    self.enum_defs.insert(name.clone(), variant_set);
                }
            }

            // Track variable types from assignments
            IR::Op { opcode, args, .. } if *opcode == IROpCode::SetLocals => {
                if args.len() >= 2 {
                    if let Some(name) = Self::extract_single_assign_name(&args[0]) {
                        // If name shadows an enum, invalidate the enum def
                        self.enum_defs.remove(&name);
                        if let Some(ty) = self.infer_type(&args[1]) {
                            self.var_types.insert(name, ty);
                        } else {
                            self.var_types.remove(&name);
                        }
                    }
                }
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
            }

            // Check match expressions
            IR::Op {
                opcode,
                args,
                start_byte,
                end_byte,
                ..
            } if *opcode == IROpCode::OpMatch => {
                self.check_match_node(args, *start_byte, *end_byte);
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
            }

            // Scope isolation for lambdas/functions: save/restore var_types + enum_defs
            // and clear parameter names that shadow outer bindings
            IR::Op {
                opcode, args, kwargs, ..
            } if *opcode == IROpCode::OpLambda || *opcode == IROpCode::FnDef => {
                let saved_vars = self.var_types.clone();
                let saved_enums = self.enum_defs.clone();
                // Clear parameter names from var_types so outer bindings don't leak
                if let Some(IR::Tuple(params) | IR::List(params)) = args.first() {
                    for param in params {
                        if let IR::Tuple(pair) = param {
                            if let Some(IR::String(name)) = pair.first() {
                                self.var_types.remove(name);
                            }
                        }
                    }
                }
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
                self.var_types = saved_vars;
                self.enum_defs = saved_enums;
            }

            // Scope isolation for control flow: restore var_types before each
            // branch so assignments from one branch don't leak into siblings
            IR::Op {
                opcode, args, kwargs, ..
            } if *opcode == IROpCode::OpIf || *opcode == IROpCode::OpWhile || *opcode == IROpCode::OpFor => {
                let saved_vars = self.var_types.clone();
                let saved_enums = self.enum_defs.clone();
                for arg in args {
                    self.var_types = saved_vars.clone();
                    self.enum_defs = saved_enums.clone();
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
                self.var_types = saved_vars;
                self.enum_defs = saved_enums;
            }

            // Recurse into other node types
            IR::Op { args, kwargs, .. } => {
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
            }
            IR::Call { func, args, kwargs, .. } => {
                self.check_exhaustiveness(func);
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
            }
            IR::List(items) | IR::Tuple(items) | IR::Set(items) => {
                for item in items {
                    self.check_exhaustiveness(item);
                }
            }
            IR::Dict(pairs) => {
                for (k, v) in pairs {
                    self.check_exhaustiveness(k);
                    self.check_exhaustiveness(v);
                }
            }
            _ => {}
        }
    }

    /// Infer the type of an expression (minimal: enum access, bool literal, var ref)
    fn infer_type(&self, expr: &IR) -> Option<InferredType> {
        match expr {
            IR::Op { opcode, args, .. } if *opcode == IROpCode::GetAttr => {
                if let Some(IR::Ref(name, _, _)) = args.first() {
                    if self.enum_defs.contains_key(name) {
                        return Some(InferredType::Enum(name.clone()));
                    }
                }
                None
            }
            IR::Bool(_) => Some(InferredType::Bool),
            IR::Ref(name, _, _) => self.var_types.get(name).cloned(),
            _ => None,
        }
    }

    fn extract_single_assign_name(target: &IR) -> Option<String> {
        match target {
            IR::Tuple(items) | IR::List(items) if items.len() == 1 => match &items[0] {
                IR::Ref(name, _, _) | IR::Identifier(name) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn check_match_node(&mut self, args: &[IR], start_byte: usize, end_byte: usize) {
        if args.len() < 2 {
            return;
        }
        let scrutinee = &args[0];
        let cases = match &args[1] {
            IR::Tuple(cases) => cases,
            _ => return,
        };

        if Self::has_unconditional_catchall(cases) {
            return;
        }

        let patterns = Self::collect_unguarded_patterns(cases);
        let scrutinee_type = self.infer_type(scrutinee);

        let is_exhaustive = match &scrutinee_type {
            Some(InferredType::Enum(enum_name)) => self.check_enum_exhaustive(&patterns, enum_name),
            Some(InferredType::Bool) => Self::check_bool_exhaustive(&patterns),
            None => false, // unknown type: never suppress
        };

        if !is_exhaustive {
            let message = match &scrutinee_type {
                Some(InferredType::Enum(name)) => {
                    let covered = Self::collect_enum_variants(&patterns, name);
                    if let Some(all) = self.enum_defs.get(name) {
                        let mut missing: Vec<_> = all.iter().filter(|v| !covered.contains(*v)).cloned().collect();
                        missing.sort();
                        format!(
                            "Non-exhaustive match on enum '{}'; missing: {}",
                            name,
                            missing.join(", ")
                        )
                    } else {
                        format!("Non-exhaustive match on enum '{}'", name)
                    }
                }
                Some(InferredType::Bool) => {
                    let (has_true, has_false) = Self::bool_coverage(&patterns);
                    let missing = match (has_true, has_false) {
                        (false, false) => "True, False",
                        (true, false) => "False",
                        (false, true) => "True",
                        _ => "",
                    };
                    format!("Non-exhaustive match on boolean; missing: {}", missing)
                }
                None => "Match has no wildcard branch; exhaustiveness depends on runtime values".to_string(),
            };
            self.diagnostics.push(SemanticDiagnostic {
                code: "I103".to_string(),
                message,
                severity: SemanticSeverity::Hint,
                start_byte,
                end_byte,
            });
        }
    }

    fn has_unconditional_catchall(cases: &[IR]) -> bool {
        for case in cases {
            if let IR::Tuple(elems) = case {
                if elems.len() >= 2 && elems[1] == IR::None && Self::is_catchall(&elems[0]) {
                    return true;
                }
            }
        }
        false
    }

    fn is_catchall(pattern: &IR) -> bool {
        match pattern {
            IR::PatternWildcard | IR::PatternVar(_) => true,
            IR::PatternOr(pats) => pats.iter().any(|p| Self::is_catchall(p)),
            _ => false,
        }
    }

    fn collect_unguarded_patterns(cases: &[IR]) -> Vec<IR> {
        let mut patterns = Vec::new();
        for case in cases {
            if let IR::Tuple(elems) = case {
                if elems.len() >= 2 && elems[1] == IR::None {
                    Self::flatten_pattern(&elems[0], &mut patterns);
                }
            }
        }
        patterns
    }

    fn flatten_pattern(pattern: &IR, out: &mut Vec<IR>) {
        match pattern {
            IR::PatternOr(pats) => {
                for p in pats {
                    Self::flatten_pattern(p, out);
                }
            }
            other => out.push(other.clone()),
        }
    }

    fn check_enum_exhaustive(&self, patterns: &[IR], enum_name: &str) -> bool {
        let all_variants = match self.enum_defs.get(enum_name) {
            Some(vs) => vs,
            None => return false,
        };
        let covered = Self::collect_enum_variants(patterns, enum_name);
        covered == *all_variants
    }

    fn collect_enum_variants(patterns: &[IR], enum_name: &str) -> HashSet<String> {
        let mut covered = HashSet::new();
        for pat in patterns {
            if let IR::PatternEnum {
                enum_name: en,
                variant_name: vn,
            } = pat
            {
                if en == enum_name {
                    covered.insert(vn.clone());
                }
            }
        }
        covered
    }

    fn check_bool_exhaustive(patterns: &[IR]) -> bool {
        let (has_true, has_false) = Self::bool_coverage(patterns);
        has_true && has_false
    }

    fn bool_coverage(patterns: &[IR]) -> (bool, bool) {
        let mut has_true = false;
        let mut has_false = false;
        for pat in patterns {
            if let IR::PatternLiteral(inner) = pat {
                match inner.as_ref() {
                    IR::Bool(true) => has_true = true,
                    IR::Bool(false) => has_false = true,
                    _ => {}
                }
            }
        }
        (has_true, has_false)
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export for convenience
pub use crate::semantic::passes::PurePass;

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    #[test]
    fn test_validate_literal() {
        let analyzer = SemanticAnalyzer::new();
        assert!(analyzer.validate(&IR::Int(42)).is_ok());
        assert!(analyzer.validate(&IR::Float(3.14)).is_ok());
        assert!(analyzer.validate(&IR::String("hello".into())).is_ok());
        assert!(analyzer.validate(&IR::Bool(true)).is_ok());
        assert!(analyzer.validate(&IR::None).is_ok());
    }

    #[test]
    fn test_validate_operation() {
        let analyzer = SemanticAnalyzer::new();
        let op = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        assert!(analyzer.validate(&op).is_ok());
    }

    #[test]
    fn test_validate_nested() {
        let analyzer = SemanticAnalyzer::new();
        let inner = IR::op(IROpCode::Mul, vec![IR::Int(2), IR::Int(3)]);
        let outer = IR::op(IROpCode::Add, vec![IR::Int(1), inner]);
        assert!(analyzer.validate(&outer).is_ok());
    }

    #[test]
    fn test_analyze() {
        let mut analyzer = SemanticAnalyzer::new();
        let ir = IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]);
        let result = analyzer.analyze(&ir);
        assert!(result.is_ok());
    }

    #[test]
    fn test_transform_type_call() {
        let analyzer = SemanticAnalyzer::new();
        // typeof(42) → Op(TypeOf, [42]) - parser produces Ref, not Identifier
        let ir = IR::Call {
            func: Box::new(IR::Ref("typeof".into(), 0, 6)),
            args: vec![IR::Int(42)],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        };
        let result = analyzer.transform(&ir);
        match result {
            IR::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::TypeOf);
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], IR::Int(42));
            }
            _ => panic!("Expected Op(TypeOf), got {:?}", result),
        }
    }

    #[test]
    fn test_transform_breakpoint_call() {
        let analyzer = SemanticAnalyzer::new();
        // breakpoint() → Op(Breakpoint, [])
        let ir = IR::Call {
            func: Box::new(IR::Ref("breakpoint".into(), 0, 10)),
            args: vec![],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        };
        let result = analyzer.transform(&ir);
        match result {
            IR::Op { opcode, args, .. } => {
                assert_eq!(opcode, IROpCode::Breakpoint);
                assert!(args.is_empty());
            }
            _ => panic!("Expected Op(Breakpoint), got {:?}", result),
        }
    }

    #[test]
    fn test_transform_preserves_normal_call() {
        let analyzer = SemanticAnalyzer::new();
        // abs(-5) stays as Call
        let ir = IR::Call {
            func: Box::new(IR::Ref("abs".into(), 0, 3)),
            args: vec![IR::Int(-5)],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 0,
            end_byte: 0,
        };
        let result = analyzer.transform(&ir);
        assert!(matches!(result, IR::Call { .. }));
    }

    #[test]
    fn test_try_passes_semantic_analysis() {
        let mut analyzer = SemanticAnalyzer::new();
        // try { 1 } except { _ => { 2 } }
        let body = IR::op(IROpCode::OpBlock, vec![IR::Int(1)]);
        let handler = IR::Tuple(vec![
            IR::List(vec![]), // wildcard: no types
            IR::None,         // no binding
            IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
        ]);
        let ir = IR::op(
            IROpCode::OpTry,
            vec![
                body,
                IR::List(vec![handler]),
                IR::None, // no finally
            ],
        );
        assert!(analyzer.analyze(&ir).is_ok());
    }

    #[test]
    fn test_raise_passes_semantic_analysis() {
        let mut analyzer = SemanticAnalyzer::new();
        // raise (bare)
        let bare = IR::op(IROpCode::OpRaise, vec![]);
        assert!(analyzer.analyze(&bare).is_ok());

        // raise <expr>
        let with_expr = IR::op(IROpCode::OpRaise, vec![IR::Int(42)]);
        assert!(analyzer.analyze(&with_expr).is_ok());
    }

    // --- I103 exhaustiveness tests ---

    /// Build a simple program: enum def + assignment + match
    fn make_enum_match_program(
        enum_name: &str,
        variants: &[&str],
        assign_var: &str,
        assign_variant: &str,
        matched_variants: &[&str],
        has_wildcard: bool,
    ) -> IR {
        let enum_def = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String(enum_name.into()),
                IR::Tuple(variants.iter().map(|v| IR::String((*v).into())).collect()),
            ],
        );
        let assignment = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref(assign_var.into(), 0, 0)]),
                IR::op(
                    IROpCode::GetAttr,
                    vec![IR::Ref(enum_name.into(), 0, 0), IR::String(assign_variant.into())],
                ),
                IR::Bool(false),
            ],
        );
        let mut cases: Vec<IR> = matched_variants
            .iter()
            .map(|v| {
                IR::Tuple(vec![
                    IR::PatternEnum {
                        enum_name: enum_name.into(),
                        variant_name: (*v).into(),
                    },
                    IR::None,
                    IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                ])
            })
            .collect();
        if has_wildcard {
            cases.push(IR::Tuple(vec![
                IR::PatternWildcard,
                IR::None,
                IR::op(IROpCode::OpBlock, vec![IR::Int(0)]),
            ]));
        }
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![IR::Ref(assign_var.into(), 0, 0), IR::Tuple(cases)],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        IR::Program(vec![enum_def, assignment, match_expr])
    }

    #[test]
    fn test_i103_enum_exhaustive_correct_type() {
        let mut a = SemanticAnalyzer::new();
        let ir = make_enum_match_program(
            "Color",
            &["red", "green", "blue"],
            "c",
            "green",
            &["red", "green", "blue"],
            false,
        );
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            result.diagnostics.is_empty(),
            "exhaustive enum match with correct type should not trigger I103"
        );
    }

    #[test]
    fn test_i103_enum_partial_correct_type() {
        let mut a = SemanticAnalyzer::new();
        let ir = make_enum_match_program(
            "Color",
            &["red", "green", "blue"],
            "c",
            "green",
            &["red", "green"],
            false,
        );
        let result = a.analyze_full(&ir).unwrap();
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "I103");
        assert!(
            result.diagnostics[0].message.contains("blue"),
            "should mention missing variant"
        );
    }

    #[test]
    fn test_i103_enum_wrong_type() {
        let mut a = SemanticAnalyzer::new();
        // c = Size.small, match c { Color.red => ... Color.green => ... Color.blue => ... }
        let enum_color = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Color".into()),
                IR::Tuple(vec![
                    IR::String("red".into()),
                    IR::String("green".into()),
                    IR::String("blue".into()),
                ]),
            ],
        );
        let enum_size = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Size".into()),
                IR::Tuple(vec![IR::String("small".into()), IR::String("large".into())]),
            ],
        );
        let assignment = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]),
                IR::op(
                    IROpCode::GetAttr,
                    vec![IR::Ref("Size".into(), 0, 0), IR::String("small".into())],
                ),
                IR::Bool(false),
            ],
        );
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("c".into(), 0, 0),
                IR::Tuple(vec![
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "red".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "green".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "blue".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(3)]),
                    ]),
                ]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let ir = IR::Program(vec![enum_color, enum_size, assignment, match_expr]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(!result.diagnostics.is_empty(), "wrong enum type should trigger I103");
    }

    #[test]
    fn test_i103_enum_unknown_scrutinee() {
        let mut a = SemanticAnalyzer::new();
        // No assignment to c, just match c { Color.* }
        let enum_def = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Color".into()),
                IR::Tuple(vec![
                    IR::String("red".into()),
                    IR::String("green".into()),
                    IR::String("blue".into()),
                ]),
            ],
        );
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("c".into(), 0, 0),
                IR::Tuple(vec![
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "red".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "green".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "blue".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(3)]),
                    ]),
                ]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let ir = IR::Program(vec![enum_def, match_expr]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            !result.diagnostics.is_empty(),
            "unknown scrutinee type should trigger I103"
        );
    }

    #[test]
    fn test_i103_wildcard_suppresses() {
        let mut a = SemanticAnalyzer::new();
        let ir = make_enum_match_program("Color", &["red", "green", "blue"], "c", "green", &["red"], true);
        let result = a.analyze_full(&ir).unwrap();
        assert!(result.diagnostics.is_empty(), "wildcard should suppress I103");
    }

    #[test]
    fn test_i103_boolean_exhaustive() {
        let mut a = SemanticAnalyzer::new();
        let assignment = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("b".into(), 0, 0)]),
                IR::Bool(true),
                IR::Bool(false),
            ],
        );
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("b".into(), 0, 0),
                IR::Tuple(vec![
                    IR::Tuple(vec![
                        IR::PatternLiteral(Box::new(IR::Bool(true))),
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternLiteral(Box::new(IR::Bool(false))),
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(0)]),
                    ]),
                ]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let ir = IR::Program(vec![assignment, match_expr]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            result.diagnostics.is_empty(),
            "exhaustive boolean match should not trigger I103"
        );
    }

    #[test]
    fn test_i103_boolean_partial() {
        let mut a = SemanticAnalyzer::new();
        let assignment = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("b".into(), 0, 0)]),
                IR::Bool(true),
                IR::Bool(false),
            ],
        );
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("b".into(), 0, 0),
                IR::Tuple(vec![IR::Tuple(vec![
                    IR::PatternLiteral(Box::new(IR::Bool(true))),
                    IR::None,
                    IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                ])]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let ir = IR::Program(vec![assignment, match_expr]);
        let result = a.analyze_full(&ir).unwrap();
        assert_eq!(result.diagnostics.len(), 1);
        assert!(result.diagnostics[0].message.contains("False"));
    }

    #[test]
    fn test_i103_guarded_not_counted() {
        let mut a = SemanticAnalyzer::new();
        let enum_def = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Color".into()),
                IR::Tuple(vec![IR::String("red".into()), IR::String("green".into())]),
            ],
        );
        let assignment = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]),
                IR::op(
                    IROpCode::GetAttr,
                    vec![IR::Ref("Color".into(), 0, 0), IR::String("red".into())],
                ),
                IR::Bool(false),
            ],
        );
        // red unguarded, green guarded → not exhaustive
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("c".into(), 0, 0),
                IR::Tuple(vec![
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "red".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "green".into(),
                        },
                        IR::Bool(true), // guard present
                        IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
                    ]),
                ]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let ir = IR::Program(vec![enum_def, assignment, match_expr]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            !result.diagnostics.is_empty(),
            "guarded case should not count for exhaustiveness"
        );
    }

    #[test]
    fn test_i103_pattern_or() {
        let mut a = SemanticAnalyzer::new();
        let enum_def = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Color".into()),
                IR::Tuple(vec![
                    IR::String("red".into()),
                    IR::String("green".into()),
                    IR::String("blue".into()),
                ]),
            ],
        );
        let assignment = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]),
                IR::op(
                    IROpCode::GetAttr,
                    vec![IR::Ref("Color".into(), 0, 0), IR::String("red".into())],
                ),
                IR::Bool(false),
            ],
        );
        // Color.red | Color.green => ..., Color.blue => ...
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("c".into(), 0, 0),
                IR::Tuple(vec![
                    IR::Tuple(vec![
                        IR::PatternOr(vec![
                            IR::PatternEnum {
                                enum_name: "Color".into(),
                                variant_name: "red".into(),
                            },
                            IR::PatternEnum {
                                enum_name: "Color".into(),
                                variant_name: "green".into(),
                            },
                        ]),
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "blue".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
                    ]),
                ]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let ir = IR::Program(vec![enum_def, assignment, match_expr]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            result.diagnostics.is_empty(),
            "pattern_or should flatten and count all variants"
        );
    }

    #[test]
    fn test_i103_branch_local_assignment_not_definite() {
        // if flag { c = Color.red } else { c = 1 }
        // match c { Color.red => ..., Color.green => ..., Color.blue => ... }
        // → should still warn because c might not be Color
        let mut a = SemanticAnalyzer::new();
        let enum_def = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Color".into()),
                IR::Tuple(vec![
                    IR::String("red".into()),
                    IR::String("green".into()),
                    IR::String("blue".into()),
                ]),
            ],
        );
        let if_stmt = IR::Op {
            opcode: IROpCode::OpIf,
            args: vec![
                IR::Ref("flag".into(), 0, 0),
                IR::op(
                    IROpCode::OpBlock,
                    vec![IR::op(
                        IROpCode::SetLocals,
                        vec![IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]), IR::Int(1), IR::Bool(false)],
                    )],
                ),
                IR::op(
                    IROpCode::OpBlock,
                    vec![IR::op(
                        IROpCode::SetLocals,
                        vec![
                            IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]),
                            IR::op(
                                IROpCode::GetAttr,
                                vec![IR::Ref("Color".into(), 0, 0), IR::String("red".into())],
                            ),
                            IR::Bool(false),
                        ],
                    )],
                ),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 50,
            end_byte: 100,
        };
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("c".into(), 0, 0),
                IR::Tuple(vec![
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "red".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "green".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "blue".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(3)]),
                    ]),
                ]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let ir = IR::Program(vec![enum_def, if_stmt, match_expr]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            !result.diagnostics.is_empty(),
            "branch-local assignment should not count as definite type"
        );
    }

    #[test]
    fn test_i103_lambda_param_shadows_outer() {
        // c = Color.red
        // f = (c) => { match c { Color.red => 1, Color.green => 2, Color.blue => 3 } }
        // → should warn: c is a parameter, not necessarily Color
        let mut a = SemanticAnalyzer::new();
        let enum_def = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Color".into()),
                IR::Tuple(vec![
                    IR::String("red".into()),
                    IR::String("green".into()),
                    IR::String("blue".into()),
                ]),
            ],
        );
        let assignment = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]),
                IR::op(
                    IROpCode::GetAttr,
                    vec![IR::Ref("Color".into(), 0, 0), IR::String("red".into())],
                ),
                IR::Bool(false),
            ],
        );
        let match_in_lambda = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("c".into(), 0, 0),
                IR::Tuple(vec![
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "red".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "green".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "blue".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(3)]),
                    ]),
                ]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let lambda_def = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("f".into(), 0, 0)]),
                IR::Op {
                    opcode: IROpCode::OpLambda,
                    args: vec![
                        IR::Tuple(vec![IR::Tuple(vec![IR::String("c".into()), IR::None])]),
                        IR::op(IROpCode::OpBlock, vec![match_in_lambda]),
                    ],
                    kwargs: IndexMap::new(),
                    tail: false,
                    start_byte: 50,
                    end_byte: 210,
                },
                IR::Bool(false),
            ],
        );
        let ir = IR::Program(vec![enum_def, assignment, lambda_def]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            !result.diagnostics.is_empty(),
            "lambda param shadowing outer enum binding should trigger I103"
        );
    }

    #[test]
    fn test_i103_then_branch_does_not_leak_into_else() {
        // if flag { c = Color.red } else { match c { Color.red => 1 } }
        // The else branch should NOT see c as Color (assigned only in then)
        let mut a = SemanticAnalyzer::new();
        let enum_def = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Color".into()),
                IR::Tuple(vec![IR::String("red".into()), IR::String("green".into())]),
            ],
        );
        let if_stmt = IR::Op {
            opcode: IROpCode::OpIf,
            args: vec![
                IR::Ref("flag".into(), 0, 0),
                // then: c = Color.red
                IR::op(
                    IROpCode::OpBlock,
                    vec![IR::op(
                        IROpCode::SetLocals,
                        vec![
                            IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]),
                            IR::op(
                                IROpCode::GetAttr,
                                vec![IR::Ref("Color".into(), 0, 0), IR::String("red".into())],
                            ),
                            IR::Bool(false),
                        ],
                    )],
                ),
                // else: match c { Color.red => 1, Color.green => 2 }
                IR::op(
                    IROpCode::OpBlock,
                    vec![IR::Op {
                        opcode: IROpCode::OpMatch,
                        args: vec![
                            IR::Ref("c".into(), 0, 0),
                            IR::Tuple(vec![
                                IR::Tuple(vec![
                                    IR::PatternEnum {
                                        enum_name: "Color".into(),
                                        variant_name: "red".into(),
                                    },
                                    IR::None,
                                    IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                                ]),
                                IR::Tuple(vec![
                                    IR::PatternEnum {
                                        enum_name: "Color".into(),
                                        variant_name: "green".into(),
                                    },
                                    IR::None,
                                    IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
                                ]),
                            ]),
                        ],
                        kwargs: IndexMap::new(),
                        tail: false,
                        start_byte: 100,
                        end_byte: 200,
                    }],
                ),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 50,
            end_byte: 250,
        };
        let ir = IR::Program(vec![enum_def, if_stmt]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            !result.diagnostics.is_empty(),
            "then-branch assignment should not leak into else"
        );
    }

    #[test]
    fn test_i103_enum_name_shadowed() {
        // enum Color { red; green }
        // Color = something_else
        // c = Color.red   <- this is now attribute access, not enum variant
        // match c { Color.red => 1, Color.green => 2 } <- should warn
        let mut a = SemanticAnalyzer::new();
        let enum_def = IR::op(
            IROpCode::EnumDef,
            vec![
                IR::String("Color".into()),
                IR::Tuple(vec![IR::String("red".into()), IR::String("green".into())]),
            ],
        );
        let shadow = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("Color".into(), 0, 0)]),
                IR::Int(42),
                IR::Bool(false),
            ],
        );
        let assign_c = IR::op(
            IROpCode::SetLocals,
            vec![
                IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]),
                IR::op(
                    IROpCode::GetAttr,
                    vec![IR::Ref("Color".into(), 0, 0), IR::String("red".into())],
                ),
                IR::Bool(false),
            ],
        );
        let match_expr = IR::Op {
            opcode: IROpCode::OpMatch,
            args: vec![
                IR::Ref("c".into(), 0, 0),
                IR::Tuple(vec![
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "red".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
                    ]),
                    IR::Tuple(vec![
                        IR::PatternEnum {
                            enum_name: "Color".into(),
                            variant_name: "green".into(),
                        },
                        IR::None,
                        IR::op(IROpCode::OpBlock, vec![IR::Int(2)]),
                    ]),
                ]),
            ],
            kwargs: IndexMap::new(),
            tail: false,
            start_byte: 100,
            end_byte: 200,
        };
        let ir = IR::Program(vec![enum_def, shadow, assign_c, match_expr]);
        let result = a.analyze_full(&ir).unwrap();
        assert!(
            !result.diagnostics.is_empty(),
            "shadowed enum name should not be treated as enum type"
        );
    }
}
