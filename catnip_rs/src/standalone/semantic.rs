// FILE: catnip_rs/src/standalone/semantic.rs
//! Standalone semantic analyzer - IRPure → OpPure validation
//!
//! Port of semantic/analyzer.rs with no PyO3 dependencies.
//! Simple validation without optimizations.

use crate::ir::{IROpCode, IRPure};

/// Semantic analyzer standalone
pub struct SemanticAnalyzer {
    /// Valid opcodes (static table)
    valid_opcodes: Vec<IROpCode>,
}

impl SemanticAnalyzer {
    /// Create a new analyzer
    pub fn new() -> Self {
        Self {
            valid_opcodes: Self::all_opcodes(),
        }
    }

    /// Analyze and validate the IR
    pub fn analyze(&mut self, ir: &IRPure) -> Result<IRPure, String> {
        self.validate(ir)?;
        // Pour MVP Phase 1: pas d'optimisations, retourner l'IR tel quel
        Ok(ir.clone())
    }

    /// Recursively validate the IR
    fn validate(&self, ir: &IRPure) -> Result<(), String> {
        match ir {
            // Literals sont toujours valides
            IRPure::Int(_)
            | IRPure::Float(_)
            | IRPure::String(_)
            | IRPure::Bytes(_)
            | IRPure::Bool(_)
            | IRPure::None
            | IRPure::Decimal(_)
            | IRPure::Imaginary(_) => Ok(()),

            // Identifiers et Refs
            IRPure::Identifier(_) | IRPure::Ref(..) => Ok(()),

            // Collections + Program
            IRPure::Program(items)
            | IRPure::List(items)
            | IRPure::Tuple(items)
            | IRPure::Set(items) => {
                for item in items {
                    self.validate(item)?;
                }
                Ok(())
            }

            IRPure::Dict(pairs) => {
                for (key, value) in pairs {
                    self.validate(key)?;
                    self.validate(value)?;
                }
                Ok(())
            }

            // Function calls
            IRPure::Call {
                func, args, kwargs, ..
            } => {
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
            IRPure::Op {
                opcode,
                args,
                kwargs,
                ..
            } => {
                // Vérifier que l'opcode existe
                if !self.valid_opcodes.contains(opcode) {
                    return Err(format!("Unknown opcode: {:?}", opcode));
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
            IRPure::PatternLiteral(value) => self.validate(value),
            IRPure::PatternVar(_) => Ok(()),
            IRPure::PatternWildcard => Ok(()),
            IRPure::PatternOr(patterns) | IRPure::PatternTuple(patterns) => {
                for pattern in patterns {
                    self.validate(pattern)?;
                }
                Ok(())
            }
            IRPure::PatternStruct { .. } => Ok(()),

            // Slice
            IRPure::Slice { start, stop, step } => {
                self.validate(start)?;
                self.validate(stop)?;
                self.validate(step)
            }

            // Broadcast
            IRPure::Broadcast {
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
        ]
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_literal() {
        let analyzer = SemanticAnalyzer::new();
        assert!(analyzer.validate(&IRPure::Int(42)).is_ok());
        assert!(analyzer.validate(&IRPure::Float(3.14)).is_ok());
        assert!(analyzer.validate(&IRPure::String("hello".into())).is_ok());
        assert!(analyzer.validate(&IRPure::Bool(true)).is_ok());
        assert!(analyzer.validate(&IRPure::None).is_ok());
    }

    #[test]
    fn test_validate_operation() {
        let analyzer = SemanticAnalyzer::new();
        let op = IRPure::op(IROpCode::Add, vec![IRPure::Int(1), IRPure::Int(2)]);
        assert!(analyzer.validate(&op).is_ok());
    }

    #[test]
    fn test_validate_nested() {
        let analyzer = SemanticAnalyzer::new();
        let inner = IRPure::op(IROpCode::Mul, vec![IRPure::Int(2), IRPure::Int(3)]);
        let outer = IRPure::op(IROpCode::Add, vec![IRPure::Int(1), inner]);
        assert!(analyzer.validate(&outer).is_ok());
    }

    #[test]
    fn test_analyze() {
        let mut analyzer = SemanticAnalyzer::new();
        let ir = IRPure::op(IROpCode::Add, vec![IRPure::Int(1), IRPure::Int(2)]);
        let result = analyzer.analyze(&ir);
        assert!(result.is_ok());
    }
}
