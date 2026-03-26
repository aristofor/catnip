// FILE: catnip_core/src/pipeline/semantic.rs
//! Standalone semantic analyzer - IR → OpPure validation
//!
//! Port of semantic/analyzer.rs with no PyO3 dependencies.
//! Simple validation without optimizations.

use crate::constants::*;
use crate::ir::{IR, IROpCode};
use crate::semantic::PureOptimizer;

/// Semantic analyzer standalone
pub struct SemanticAnalyzer {
    /// Valid opcodes (static table)
    valid_opcodes: Vec<IROpCode>,
    /// Pure optimization passes
    optimizer: Option<PureOptimizer>,
    /// Tail-call optimization enabled
    tco_enabled: bool,
}

impl SemanticAnalyzer {
    /// Create a new analyzer (no optimization passes)
    pub fn new() -> Self {
        Self {
            valid_opcodes: Self::all_opcodes(),
            optimizer: None,
            tco_enabled: true,
        }
    }

    /// Create a new analyzer with optimization passes enabled
    pub fn with_optimizer() -> Self {
        Self {
            valid_opcodes: Self::all_opcodes(),
            optimizer: Some(PureOptimizer::new()),
            tco_enabled: true,
        }
    }

    /// Enable or disable tail-call optimization marking.
    pub fn set_tco_enabled(&mut self, enabled: bool) {
        self.tco_enabled = enabled;
    }

    /// Analyze, transform, optimize and validate the IR
    pub fn analyze(&mut self, ir: &IR) -> Result<IR, String> {
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
        Ok(optimized)
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
        ]
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
}
