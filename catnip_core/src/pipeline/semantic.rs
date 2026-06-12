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
    /// Tagged union (ADT) -- name of the union type
    Union(String),
    Bool,
}

/// Semantic analyzer standalone
pub struct SemanticAnalyzer {
    /// Valid opcodes (static table)
    valid_opcodes: Vec<IROpCode>,
    /// Optimization passes enabled (host baseline; file pragmas can flip it)
    optimize_enabled: bool,
    /// Tail-call optimization enabled (host baseline; file pragmas can flip it)
    tco_enabled: bool,
    /// Host override for optimization (CLI/env). Wins over file pragmas.
    optimize_override: Option<bool>,
    /// Host override for TCO (CLI/env). Wins over file pragmas.
    tco_override: Option<bool>,
    /// Known enum definitions: name -> variant names
    enum_defs: HashMap<String, HashSet<String>>,
    /// Known union (ADT) definitions: name -> { variant_name -> has_payload }.
    /// `has_payload = false` means the variant matches as a `pattern_enum`
    /// (`Option.None`); `true` means it requires `pattern_struct`
    /// (`Option.Some{...}`).
    union_defs: HashMap<String, HashMap<String, bool>>,
    /// Variable type bindings
    var_types: HashMap<String, InferredType>,
    /// Non-fatal diagnostics collected during analysis
    diagnostics: Vec<SemanticDiagnostic>,
}

impl SemanticAnalyzer {
    /// Create a new analyzer (no optimization passes by default)
    pub fn new() -> Self {
        Self {
            valid_opcodes: Self::all_opcodes(),
            optimize_enabled: false,
            tco_enabled: true,
            optimize_override: None,
            tco_override: None,
            enum_defs: HashMap::new(),
            union_defs: HashMap::new(),
            var_types: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Create a new analyzer with optimization passes enabled by default
    pub fn with_optimizer() -> Self {
        Self {
            optimize_enabled: true,
            ..Self::new()
        }
    }

    /// Enable or disable tail-call optimization marking (baseline).
    pub fn set_tco_enabled(&mut self, enabled: bool) {
        self.tco_enabled = enabled;
    }

    /// Force TCO on/off regardless of file pragmas (CLI/env override).
    pub fn set_tco_override(&mut self, forced: Option<bool>) {
        self.tco_override = forced;
    }

    /// Force optimization on/off regardless of file pragmas (CLI/env override).
    pub fn set_optimize_override(&mut self, forced: Option<bool>) {
        self.optimize_override = forced;
    }

    /// Analyze, transform, optimize and validate the IR
    pub fn analyze(&mut self, ir: &IR) -> Result<IR, String> {
        Ok(self.analyze_full(ir)?.ir)
    }

    /// Full analysis returning IR + non-fatal diagnostics
    pub fn analyze_full(&mut self, ir: &IR) -> Result<AnalysisResult, String> {
        self.enum_defs.clear();
        self.union_defs.clear();
        self.var_types.clear();
        self.diagnostics.clear();

        let transformed = self.transform(ir);
        // Precedence: host override (CLI/env) > file pragma > host baseline
        let (file_tco, file_optimize) = Self::scan_file_pragmas(&transformed);
        let tco = self.tco_override.or(file_tco).unwrap_or(self.tco_enabled);
        let optimize = self
            .optimize_override
            .or(file_optimize)
            .unwrap_or(self.optimize_enabled);

        let tail_marked = if tco {
            Self::mark_tail_calls(&transformed)
        } else {
            transformed
        };
        let optimized = if optimize {
            PureOptimizer::new().optimize(tail_marked)
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

    /// Mark tail calls for TCO (proper tail calls).
    ///
    /// Inside every lambda body, any call whose target is a plain name
    /// (Ref/Identifier) in tail position gets `tail: true`: self-recursion,
    /// mutual recursion and terminal calls alike. All three runtimes execute
    /// a marked call in O(1) stack: frame reuse in the VMs, trampoline with
    /// scope swap in the AST interpreter (non-Catnip targets are called
    /// directly).
    fn mark_tail_calls(ir: &IR) -> IR {
        Self::mark_tails(ir, false)
    }

    /// Single traversal. `in_tail` is true when `ir` sits in tail position
    /// of the enclosing lambda body. `in_tail` only becomes true at lambda
    /// bodies, so `tail: true` never escapes to top level (a TailCall
    /// leaking outside a trampoline would surface as a value).
    fn mark_tails(ir: &IR, in_tail: bool) -> IR {
        match ir {
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
                ..
            } => {
                let nominal = matches!(func.as_ref(), IR::Ref(..) | IR::Identifier(_));
                IR::Call {
                    func: Box::new(Self::mark_tails(func, false)),
                    args: args.iter().map(|a| Self::mark_tails(a, false)).collect(),
                    kwargs: kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), Self::mark_tails(v, false)))
                        .collect(),
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                    tail: in_tail && nominal,
                }
            }

            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => {
                let new_args: Vec<IR> = match opcode {
                    // Lambda body: the only place where tail positions begin.
                    // args: [params, body, ...] -- parameter defaults are not tail.
                    IROpCode::OpLambda | IROpCode::FnDef if args.len() >= 2 => args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| Self::mark_tails(a, i == 1))
                        .collect(),

                    // Block: only the last statement inherits tail position
                    IROpCode::OpBlock => {
                        let last = args.len().saturating_sub(1);
                        args.iter()
                            .enumerate()
                            .map(|(i, a)| Self::mark_tails(a, in_tail && i == last))
                            .collect()
                    }

                    // If: args = [branches, else_block] where branches is a
                    // list of (condition, block) pairs. Blocks inherit tail
                    // position, conditions do not.
                    IROpCode::OpIf => args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| match (i, a) {
                            (0, IR::List(branches)) | (0, IR::Tuple(branches)) => {
                                let marked: Vec<IR> = branches
                                    .iter()
                                    .map(|branch| match branch {
                                        IR::Tuple(pair) if pair.len() == 2 => IR::Tuple(vec![
                                            Self::mark_tails(&pair[0], false),
                                            Self::mark_tails(&pair[1], in_tail),
                                        ]),
                                        _ => Self::mark_tails(branch, false),
                                    })
                                    .collect();
                                if matches!(a, IR::Tuple(_)) {
                                    IR::Tuple(marked)
                                } else {
                                    IR::List(marked)
                                }
                            }
                            (1, _) => Self::mark_tails(a, in_tail),
                            _ => Self::mark_tails(a, false),
                        })
                        .collect(),

                    // Match: args = [scrutinee, cases] with cases of shape
                    // Tuple([pattern, guard, body]). Bodies inherit tail
                    // position; scrutinee, patterns and guards do not.
                    IROpCode::OpMatch => args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| match (i, a) {
                            (1, IR::Tuple(cases)) | (1, IR::List(cases)) => {
                                let marked: Vec<IR> = cases
                                    .iter()
                                    .map(|case| match case {
                                        IR::Tuple(parts) if parts.len() >= 3 => {
                                            let new_parts: Vec<IR> = parts
                                                .iter()
                                                .enumerate()
                                                .map(|(j, p)| Self::mark_tails(p, in_tail && j == 2))
                                                .collect();
                                            IR::Tuple(new_parts)
                                        }
                                        _ => Self::mark_tails(case, false),
                                    })
                                    .collect();
                                if matches!(a, IR::List(_)) {
                                    IR::List(marked)
                                } else {
                                    IR::Tuple(marked)
                                }
                            }
                            _ => Self::mark_tails(a, false),
                        })
                        .collect(),

                    // Return: inherits in_tail rather than forcing it.
                    // op_return propagates a TailCall as a value; marking
                    // `return f()` inside a loop body would leak the TailCall
                    // object into op_while/op_for which would treat it as a
                    // plain statement value.
                    IROpCode::OpReturn => args.iter().map(|a| Self::mark_tails(a, in_tail)).collect(),

                    // Everything else is not a tail position. Notably:
                    // - OpTry: the handler must stay on the stack
                    // - OpWhile/OpFor: loop bodies re-enter
                    // - And/Or/NullCoalesce: operands feed is_truthy(), a
                    //   TailCall object would be consumed as truthy
                    // We still recurse to reach nested lambda bodies.
                    _ => args.iter().map(|a| Self::mark_tails(a, false)).collect(),
                };
                IR::Op {
                    opcode: *opcode,
                    args: new_args,
                    kwargs: kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), Self::mark_tails(v, false)))
                        .collect(),
                    tail: *tail,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                }
            }

            // Structural containers: no tail position inside, but nested
            // lambdas must still be reached
            IR::Program(items) => IR::Program(items.iter().map(|i| Self::mark_tails(i, false)).collect()),
            IR::List(items) => IR::List(items.iter().map(|i| Self::mark_tails(i, false)).collect()),
            IR::Tuple(items) => IR::Tuple(items.iter().map(|i| Self::mark_tails(i, false)).collect()),
            IR::Set(items) => IR::Set(items.iter().map(|i| Self::mark_tails(i, false)).collect()),
            IR::Dict(entries) => IR::Dict(
                entries
                    .iter()
                    .map(|(k, v)| (Self::mark_tails(k, false), Self::mark_tails(v, false)))
                    .collect(),
            ),
            IR::Slice { start, stop, step } => IR::Slice {
                start: Box::new(Self::mark_tails(start, false)),
                stop: Box::new(Self::mark_tails(stop, false)),
                step: Box::new(Self::mark_tails(step, false)),
            },
            IR::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => IR::Broadcast {
                target: target.as_ref().map(|t| Box::new(Self::mark_tails(t, false))),
                operator: Box::new(Self::mark_tails(operator, false)),
                operand: operand.as_ref().map(|o| Box::new(Self::mark_tails(o, false))),
                broadcast_type: broadcast_type.clone(),
            },

            // Leaves (literals, identifiers, patterns)
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

    /// Scan top-level pragma directives that affect this analysis pass.
    /// Returns (tco, optimize); `None` when the file does not set the pragma.
    /// Pragmas are file-scoped and sequential: the last one wins. Invalid
    /// values are ignored here -- `validate_pragma` reports them later with
    /// source positions.
    fn scan_file_pragmas(ir: &IR) -> (Option<bool>, Option<bool>) {
        let stmts = match ir {
            IR::Program(stmts) => stmts.as_slice(),
            other => std::slice::from_ref(other),
        };
        let mut tco = None;
        let mut optimize = None;
        for stmt in stmts {
            if let IR::Op {
                opcode: IROpCode::Pragma,
                args,
                ..
            } = stmt
            {
                let (Some(IR::String(directive)), Some(value)) = (args.first(), args.get(1)) else {
                    continue;
                };
                match (directive.to_lowercase().as_str(), value) {
                    ("tco", IR::Bool(b)) => tco = Some(*b),
                    (PRAGMA_OPTIMIZE, IR::Int(n)) if (0..=OPTIMIZE_MAX).contains(n) => optimize = Some(*n > 0),
                    _ => {}
                }
            }
        }
        (tco, optimize)
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
            IROpCode::UnionDef,
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

            // Register union definitions
            // IR layout: UnionDef(name, type_params_list, variants_list)
            // where each variant is Tuple([variant_name, fields_list]).
            // A variant has a payload iff its fields_list is non-empty.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::UnionDef => {
                if let (Some(IR::String(name)), Some(IR::List(variants))) = (args.first(), args.get(2)) {
                    let variants_map: HashMap<String, bool> = variants
                        .iter()
                        .filter_map(|v| match v {
                            IR::Tuple(parts) if parts.len() >= 2 => {
                                let vname = match parts.first() {
                                    Some(IR::String(s)) => s.clone(),
                                    _ => return None,
                                };
                                let has_payload = match &parts[1] {
                                    IR::List(fields) => !fields.is_empty(),
                                    _ => false,
                                };
                                Some((vname, has_payload))
                            }
                            _ => None,
                        })
                        .collect();
                    self.union_defs.insert(name.clone(), variants_map);
                }
            }

            // Track variable types from assignments
            IR::Op { opcode, args, .. } if *opcode == IROpCode::SetLocals => {
                if args.len() >= 2 {
                    if let Some(name) = Self::extract_single_assign_name(&args[0]) {
                        // Shadow: invalidate any enum/union def with the same name
                        self.enum_defs.remove(&name);
                        self.union_defs.remove(&name);
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

            // Scope isolation for lambdas/functions: save/restore var_types + enum/union defs
            // and clear parameter names that shadow outer bindings
            IR::Op {
                opcode, args, kwargs, ..
            } if *opcode == IROpCode::OpLambda || *opcode == IROpCode::FnDef => {
                let saved_vars = self.var_types.clone();
                let saved_enums = self.enum_defs.clone();
                let saved_unions = self.union_defs.clone();
                // Clear parameter names from var_types so outer bindings don't leak.
                // Parameters also shadow any enum/union namespace of the same name,
                // so we drop those defs while analyzing the body -- otherwise
                // `infer_type` would still treat e.g. `(Option) => match Option.None`
                // as a union scrutinee.
                if let Some(IR::Tuple(params) | IR::List(params)) = args.first() {
                    for param in params {
                        if let IR::Tuple(pair) = param {
                            if let Some(IR::String(name)) = pair.first() {
                                self.var_types.remove(name);
                                self.enum_defs.remove(name);
                                self.union_defs.remove(name);
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
                self.union_defs = saved_unions;
            }

            // Scope isolation for control flow: restore var_types before each
            // branch so assignments from one branch don't leak into siblings
            IR::Op {
                opcode, args, kwargs, ..
            } if *opcode == IROpCode::OpIf || *opcode == IROpCode::OpWhile || *opcode == IROpCode::OpFor => {
                let saved_vars = self.var_types.clone();
                let saved_enums = self.enum_defs.clone();
                let saved_unions = self.union_defs.clone();
                for arg in args {
                    self.var_types = saved_vars.clone();
                    self.enum_defs = saved_enums.clone();
                    self.union_defs = saved_unions.clone();
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
                self.var_types = saved_vars;
                self.enum_defs = saved_enums;
                self.union_defs = saved_unions;
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

    /// Infer the type of an expression (minimal: enum/union access, bool
    /// literal, var ref, call on union variant).
    fn infer_type(&self, expr: &IR) -> Option<InferredType> {
        match expr {
            // `Color.red`, `Option.None` -- attribute access on a known type
            IR::Op { opcode, args, .. } if *opcode == IROpCode::GetAttr => {
                if let Some(IR::Ref(name, _, _)) = args.first() {
                    if self.enum_defs.contains_key(name) {
                        return Some(InferredType::Enum(name.clone()));
                    }
                    if self.union_defs.contains_key(name) {
                        return Some(InferredType::Union(name.clone()));
                    }
                }
                None
            }
            // `Option.Some(42)` -- call on a union variant constructor
            IR::Call { func, .. } => {
                if let IR::Op { opcode, args, .. } = func.as_ref() {
                    if *opcode == IROpCode::GetAttr {
                        if let Some(IR::Ref(name, _, _)) = args.first() {
                            if self.union_defs.contains_key(name) {
                                return Some(InferredType::Union(name.clone()));
                            }
                        }
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
            Some(InferredType::Union(union_name)) => self.check_union_exhaustive(&patterns, union_name),
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
                Some(InferredType::Union(name)) => {
                    let covered = self.collect_union_variants(&patterns, name);
                    if let Some(all) = self.union_defs.get(name) {
                        let mut missing: Vec<String> = all.keys().filter(|v| !covered.contains(*v)).cloned().collect();
                        missing.sort();
                        format!(
                            "Non-exhaustive match on union '{}'; missing: {}",
                            name,
                            missing.join(", ")
                        )
                    } else {
                        format!("Non-exhaustive match on union '{}'", name)
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

    fn check_union_exhaustive(&self, patterns: &[IR], union_name: &str) -> bool {
        let all_variants = match self.union_defs.get(union_name) {
            Some(vs) => vs,
            None => return false,
        };
        let covered = self.collect_union_variants(patterns, union_name);
        all_variants.keys().all(|v| covered.contains(v))
    }

    /// Collect variant names covered by a set of patterns on a union scrutinee.
    ///
    /// Two pattern shapes can target a union variant:
    /// - Nullary variants use `pattern_enum` (`Option.None`). A `pattern_enum`
    ///   pointing at a payload-bearing variant never matches at runtime, so we
    ///   refuse to count it as coverage -- otherwise `Option.Some` (no braces)
    ///   would suppress exhaustiveness warnings for `Option.Some(1)`.
    /// - Payload-bearing variants use the qualified `pattern_struct`
    ///   (`Option.Some{value}`), distinguished by its `variant` field.
    fn collect_union_variants(&self, patterns: &[IR], union_name: &str) -> HashSet<String> {
        let variants = self.union_defs.get(union_name);
        let mut covered = HashSet::new();
        for pat in patterns {
            match pat {
                IR::PatternEnum {
                    enum_name: en,
                    variant_name: vn,
                } if en == union_name => {
                    // Only count if the variant is actually nullary; a
                    // bare `Option.Some` against a payload variant is dead.
                    if let Some(v) = variants {
                        if v.get(vn) == Some(&false) {
                            covered.insert(vn.clone());
                        }
                    }
                }
                IR::PatternStruct {
                    name,
                    variant: Some(vn),
                    ..
                } if name == union_name => {
                    // PatternStruct with payload-fields covers any variant
                    // that exists on the union (nullary or not). The
                    // matcher uses the qualified name lookup so even a
                    // zero-field `Option.None{}` resolves cleanly.
                    if let Some(v) = variants {
                        if v.contains_key(vn) {
                            covered.insert(vn.clone());
                        }
                    }
                }
                _ => {}
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

    fn pragma_stmt(directive: &str, value: IR) -> IR {
        IR::op(IROpCode::Pragma, vec![IR::String(directive.into()), value])
    }

    fn foldable_program(pragmas: Vec<IR>) -> IR {
        let mut stmts = pragmas;
        stmts.push(IR::op(IROpCode::Add, vec![IR::Int(1), IR::Int(2)]));
        IR::Program(stmts)
    }

    fn last_stmt(ir: &IR) -> &IR {
        match ir {
            IR::Program(stmts) => stmts.last().unwrap(),
            other => other,
        }
    }

    #[test]
    fn test_scan_file_pragmas_last_wins() {
        let program = IR::Program(vec![
            pragma_stmt("optimize", IR::Int(0)),
            pragma_stmt("optimize", IR::Int(3)),
            pragma_stmt("tco", IR::Bool(false)),
        ]);
        assert_eq!(SemanticAnalyzer::scan_file_pragmas(&program), (Some(false), Some(true)));
    }

    #[test]
    fn test_file_pragma_optimize_off_disables_passes() {
        let mut a = SemanticAnalyzer::with_optimizer();
        let result = a
            .analyze(&foldable_program(vec![pragma_stmt("optimize", IR::Int(0))]))
            .unwrap();
        assert!(
            matches!(
                last_stmt(&result),
                IR::Op {
                    opcode: IROpCode::Add,
                    ..
                }
            ),
            "pragma optimize 0 must disable constant folding, got {:?}",
            result
        );
    }

    #[test]
    fn test_file_pragma_optimize_on_enables_passes() {
        let mut a = SemanticAnalyzer::new(); // baseline: no optimization
        let result = a
            .analyze(&foldable_program(vec![pragma_stmt("optimize", IR::Int(2))]))
            .unwrap();
        assert_eq!(last_stmt(&result), &IR::Int(3), "pragma optimize 2 must enable passes");
    }

    #[test]
    fn test_host_override_wins_over_file_pragma() {
        let mut a = SemanticAnalyzer::with_optimizer();
        a.set_optimize_override(Some(true));
        let result = a
            .analyze(&foldable_program(vec![pragma_stmt("optimize", IR::Int(0))]))
            .unwrap();
        assert_eq!(
            last_stmt(&result),
            &IR::Int(3),
            "host override (CLI/env) must win over in-file pragma"
        );

        let mut a = SemanticAnalyzer::new();
        a.set_optimize_override(Some(false));
        let result = a
            .analyze(&foldable_program(vec![pragma_stmt("optimize", IR::Int(3))]))
            .unwrap();
        assert!(
            matches!(
                last_stmt(&result),
                IR::Op {
                    opcode: IROpCode::Add,
                    ..
                }
            ),
            "host override off must win over in-file pragma on"
        );
    }

    // -----------------------------------------------------------------------
    // Tail-call marking (proper tail calls)
    // -----------------------------------------------------------------------

    fn parse_program(source: &str) -> IR {
        use tree_sitter::Parser;
        let language = catnip_grammar::get_language();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        crate::parser::pure_transforms::transform(tree.root_node(), source).unwrap()
    }

    /// Collect (callee_name, tail) for every Call with a nominal target.
    fn collect_calls(ir: &IR, out: &mut Vec<(String, bool)>) {
        match ir {
            IR::Call {
                func,
                args,
                kwargs,
                tail,
                ..
            } => {
                if let IR::Ref(n, _, _) | IR::Identifier(n) = func.as_ref() {
                    out.push((n.clone(), *tail));
                }
                collect_calls(func, out);
                for a in args {
                    collect_calls(a, out);
                }
                for v in kwargs.values() {
                    collect_calls(v, out);
                }
            }
            IR::Op { args, kwargs, .. } => {
                for a in args {
                    collect_calls(a, out);
                }
                for v in kwargs.values() {
                    collect_calls(v, out);
                }
            }
            IR::Program(items) | IR::List(items) | IR::Tuple(items) | IR::Set(items) | IR::PatternOr(items) => {
                for i in items {
                    collect_calls(i, out);
                }
            }
            IR::Dict(entries) => {
                for (k, v) in entries {
                    collect_calls(k, out);
                    collect_calls(v, out);
                }
            }
            IR::Slice { start, stop, step } => {
                collect_calls(start, out);
                collect_calls(stop, out);
                collect_calls(step, out);
            }
            IR::Broadcast {
                target,
                operator,
                operand,
                ..
            } => {
                if let Some(t) = target {
                    collect_calls(t, out);
                }
                collect_calls(operator, out);
                if let Some(o) = operand {
                    collect_calls(o, out);
                }
            }
            IR::PatternLiteral(inner) => collect_calls(inner, out),
            _ => {}
        }
    }

    fn marked_calls(source: &str) -> Vec<(String, bool)> {
        let marked = SemanticAnalyzer::mark_tail_calls(&parse_program(source));
        let mut out = Vec::new();
        collect_calls(&marked, &mut out);
        out
    }

    fn tail_of(calls: &[(String, bool)], name: &str) -> bool {
        calls
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no call to {name} found"))
            .1
    }

    #[test]
    fn test_mark_tail_self_recursion() {
        let calls = marked_calls("count = (n) => { if n == 0 { 0 } else { count(n - 1) } }");
        assert!(tail_of(&calls, "count"), "self-call in tail position must be marked");
    }

    #[test]
    fn test_mark_tail_mutual_recursion() {
        let calls = marked_calls(
            "is_even = (n) => { if n == 0 { True } else { is_odd(n - 1) } }\n\
             is_odd = (n) => { if n == 0 { False } else { is_even(n - 1) } }",
        );
        assert!(tail_of(&calls, "is_odd"), "mutual call in tail position must be marked");
        assert!(
            tail_of(&calls, "is_even"),
            "mutual call in tail position must be marked"
        );
    }

    #[test]
    fn test_mark_tail_nested_def() {
        // inner is defined in a non-final block statement: both its self-call
        // and the final call to it must be marked
        let calls = marked_calls(
            "outer = (n) => {\n\
               inner = (k) => { if k == 0 { 0 } else { inner(k - 1) } }\n\
               inner(n)\n\
             }",
        );
        assert!(
            calls.iter().filter(|(n, _)| n == "inner").all(|(_, t)| *t),
            "nested def: self-call and final call must both be tail, got {calls:?}"
        );
    }

    #[test]
    fn test_mark_tail_lambda_in_call_arg() {
        // lambda passed as argument: its body has its own tail position
        let calls = marked_calls("r = apply((x) => { helper(x) })");
        assert!(tail_of(&calls, "helper"), "lambda argument body must get tail marking");
        assert!(!tail_of(&calls, "apply"), "top-level call must not be tail");
    }

    #[test]
    fn test_mark_tail_match_case_bodies() {
        let calls = marked_calls("f = (n) => { match n { 0 => { g(n) }\n_ if h(n) => { f(n - 1) }\n_ => { 0 } } }");
        assert!(tail_of(&calls, "g"), "match case body is a tail position");
        assert!(tail_of(&calls, "f"), "match case body is a tail position");
        assert!(!tail_of(&calls, "h"), "match guard is not a tail position");
    }

    #[test]
    fn test_mark_tail_negative_positions() {
        // try body, loop body, and/or operands, call args: never tail
        let calls = marked_calls("f = (n) => { try { g(n) } except { _ => { h(n) } } }");
        assert!(!tail_of(&calls, "g"), "call under try is not tail (handler on stack)");
        assert!(!tail_of(&calls, "h"), "except handler body is not tail");

        let calls = marked_calls("f = (n) => { while True { g(n) } }");
        assert!(!tail_of(&calls, "g"), "loop body is not a tail position");

        let calls = marked_calls("f = (n) => { g(n) or h(n) }");
        assert!(!tail_of(&calls, "g"), "or lhs is not tail (is_truthy consumes it)");
        assert!(!tail_of(&calls, "h"), "or rhs is not tail (is_truthy consumes it)");

        let calls = marked_calls("f = (n) => { g(h(n)) }");
        assert!(tail_of(&calls, "g"), "outer call is tail");
        assert!(!tail_of(&calls, "h"), "call argument is not tail");
    }

    #[test]
    fn test_mark_tail_never_at_top_level() {
        // a TailCall escaping outside any trampoline would leak as a value
        let calls = marked_calls("g(1)\nx = h(2)\nf(3)");
        assert!(
            calls.iter().all(|(_, t)| !t),
            "top-level calls must never be marked, got {calls:?}"
        );
    }

    #[test]
    fn test_mark_tail_survives_optimizer() {
        let marked = SemanticAnalyzer::mark_tail_calls(&parse_program(
            "is_even = (n) => { if n == 0 { True } else { is_odd(n - 1) } }\n\
             is_odd = (n) => { if n == 0 { False } else { is_even(n - 1) } }",
        ));
        let optimized = PureOptimizer::new().optimize(marked);
        let mut calls = Vec::new();
        collect_calls(&optimized, &mut calls);
        assert!(tail_of(&calls, "is_odd"), "tail flag must survive optimization passes");
        assert!(tail_of(&calls, "is_even"), "tail flag must survive optimization passes");
    }
}
