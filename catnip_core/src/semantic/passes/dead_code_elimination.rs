// FILE: catnip_core/src/semantic/passes/dead_code_elimination.rs
//! Dead code elimination pass (pure Rust).
//!
//! Removes code that will never execute:
//! - if True { a } else { b } → a
//! - if False { a } else { b } → b
//! - while False { block } → None
//! - block() → None (empty block)
//! - match: dead cases removed, single catchall → body

use super::{PurePass, is_truthy_constant, map_children};
use crate::ir::{IR, IROpCode};

pub struct DeadCodeEliminationPass;

impl PurePass for DeadCodeEliminationPass {
    fn name(&self) -> &str {
        "dead_code_elimination"
    }

    fn optimize(&mut self, ir: IR) -> IR {
        let visited = map_children(ir, &mut |child| self.optimize(child));
        eliminate(visited)
    }
}

fn eliminate(ir: IR) -> IR {
    match &ir {
        IR::Op { opcode, .. } => match opcode {
            IROpCode::OpIf => eliminate_if(ir),
            IROpCode::OpWhile => eliminate_while(ir),
            IROpCode::OpBlock => eliminate_block(ir),
            IROpCode::OpMatch => eliminate_match(ir),
            _ => ir,
        },
        _ => ir,
    }
}

fn eliminate_if(ir: IR) -> IR {
    let IR::Op {
        opcode: IROpCode::OpIf,
        ref args,
        ..
    } = ir
    else {
        return ir;
    };
    if args.is_empty() {
        return ir;
    }

    let branch_items = match &args[0] {
        IR::List(items) | IR::Tuple(items) => items.as_slice(),
        _ => return ir,
    };

    // Check first branch for constant condition
    if let Some(IR::Tuple(parts)) = branch_items.first() {
        if parts.len() >= 2 && is_truthy_constant(&parts[0]) == Some(true) {
            return parts[1].clone();
        }
    }

    // Check if ALL conditions are false
    let all_false = branch_items.iter().all(|b| {
        if let IR::Tuple(parts) = b {
            if !parts.is_empty() {
                return is_truthy_constant(&parts[0]) == Some(false);
            }
        }
        false
    });

    if all_false {
        return args.get(1).cloned().unwrap_or(IR::None);
    }

    ir
}

fn eliminate_while(ir: IR) -> IR {
    let IR::Op {
        opcode: IROpCode::OpWhile,
        ref args,
        ..
    } = ir
    else {
        return ir;
    };
    if !args.is_empty() && is_truthy_constant(&args[0]) == Some(false) {
        return IR::None;
    }
    ir
}

fn eliminate_block(ir: IR) -> IR {
    let IR::Op {
        opcode: IROpCode::OpBlock,
        ref args,
        ..
    } = ir
    else {
        return ir;
    };
    if args.is_empty() {
        return IR::None;
    }
    ir
}

fn eliminate_match(ir: IR) -> IR {
    // Early check
    if !matches!(&ir, IR::Op { opcode: IROpCode::OpMatch, args, .. } if args.len() == 2) {
        return ir;
    }

    // Destructure to take ownership
    let IR::Op {
        opcode,
        args,
        kwargs,
        tail,
        start_byte,
        end_byte,
    } = ir
    else {
        unreachable!()
    };

    let mut args_iter = args.into_iter();
    let value_expr = args_iter.next().unwrap();
    let cases_node = args_iter.next().unwrap();

    let cases: Vec<IR> = match cases_node {
        IR::List(items) | IR::Tuple(items) => items,
        other => {
            return IR::Op {
                opcode,
                args: vec![value_expr, other],
                kwargs,
                tail,
                start_byte,
                end_byte,
            };
        }
    };

    let mut live_cases = Vec::new();

    for case in &cases {
        let parts = match case {
            IR::Tuple(p) if p.len() >= 3 => p,
            _ => {
                live_cases.push(case.clone());
                continue;
            }
        };

        let pattern = &parts[0];
        let guard = &parts[1];
        let body = &parts[2];

        // Guard is constant False: skip case
        if !matches!(guard, IR::None) && is_truthy_constant(guard) == Some(false) {
            continue;
        }

        let is_catchall = is_catchall_pattern(pattern);

        // Guard is constant True: simplify to no guard
        let effective_no_guard = if !matches!(guard, IR::None) && is_truthy_constant(guard) == Some(true) {
            live_cases.push(IR::Tuple(vec![pattern.clone(), IR::None, body.clone()]));
            true
        } else {
            let no_guard = matches!(guard, IR::None);
            live_cases.push(case.clone());
            no_guard
        };

        // Catchall without guard: remaining cases unreachable
        if effective_no_guard && is_catchall {
            break;
        }
    }

    // Single wildcard catchall without guard: replace match with body.
    // Only for PatternWildcard - PatternVar needs the binding.
    if live_cases.len() == 1 {
        if let IR::Tuple(parts) = &live_cases[0] {
            if parts.len() >= 3 && matches!(&parts[1], IR::None) && is_eliminable_catchall(&parts[0]) {
                return keep_scrutinee(value_expr, parts[2].clone());
            }
        }
    }

    if live_cases.is_empty() {
        // Every case was eliminated: at runtime this match raises (no case
        // matched). Keep it unchanged so the error is preserved.
        return IR::Op {
            opcode,
            args: vec![value_expr, IR::Tuple(cases)],
            kwargs,
            tail,
            start_byte,
            end_byte,
        };
    }

    IR::Op {
        opcode,
        args: vec![value_expr, IR::Tuple(live_cases)],
        kwargs,
        tail,
        start_byte,
        end_byte,
    }
}

/// Replace a match with `result` without dropping the scrutinee: a non-literal
/// scrutinee may carry side effects (calls) or raise (unbound Ref), so its
/// evaluation must be preserved.
fn keep_scrutinee(value_expr: IR, result: IR) -> IR {
    if value_expr.is_literal() {
        result
    } else {
        IR::op(IROpCode::OpBlock, vec![value_expr, result])
    }
}

fn is_catchall_pattern(pattern: &IR) -> bool {
    matches!(pattern, IR::PatternWildcard | IR::PatternVar(_))
}

/// Can we safely eliminate this match case entirely?
/// PatternVar captures a value, so eliminating the match would lose the binding.
fn is_eliminable_catchall(pattern: &IR) -> bool {
    matches!(pattern, IR::PatternWildcard)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(ir: IR) -> IR {
        DeadCodeEliminationPass.optimize(ir)
    }

    #[test]
    fn test_if_true() {
        // if True { 42 } else { 0 }
        let branches = IR::List(vec![IR::Tuple(vec![IR::Bool(true), IR::Int(42)])]);
        let ir = IR::op(IROpCode::OpIf, vec![branches, IR::Int(0)]);
        assert_eq!(opt(ir), IR::Int(42));
    }

    #[test]
    fn test_if_false_with_else() {
        // if False { 42 } else { 0 }
        let branches = IR::List(vec![IR::Tuple(vec![IR::Bool(false), IR::Int(42)])]);
        let ir = IR::op(IROpCode::OpIf, vec![branches, IR::Int(0)]);
        assert_eq!(opt(ir), IR::Int(0));
    }

    #[test]
    fn test_if_false_no_else() {
        // if False { 42 }
        let branches = IR::List(vec![IR::Tuple(vec![IR::Bool(false), IR::Int(42)])]);
        let ir = IR::op(IROpCode::OpIf, vec![branches]);
        assert_eq!(opt(ir), IR::None);
    }

    #[test]
    fn test_while_false() {
        let ir = IR::op(IROpCode::OpWhile, vec![IR::Bool(false), IR::Int(1)]);
        assert_eq!(opt(ir), IR::None);
    }

    #[test]
    fn test_empty_block() {
        let ir = IR::op(IROpCode::OpBlock, vec![]);
        assert_eq!(opt(ir), IR::None);
    }

    #[test]
    fn test_non_empty_block_unchanged() {
        let ir = IR::op(IROpCode::OpBlock, vec![IR::Int(1)]);
        let result = opt(ir.clone());
        assert_eq!(result, ir);
    }

    #[test]
    fn test_match_single_catchall_literal_scrutinee() {
        // match 1 { _ => 42 } → 42 (literal scrutinee can be dropped)
        let cases = IR::Tuple(vec![IR::Tuple(vec![IR::PatternWildcard, IR::None, IR::Int(42)])]);
        let ir = IR::op(IROpCode::OpMatch, vec![IR::Int(1), cases]);
        assert_eq!(opt(ir), IR::Int(42));
    }

    #[test]
    fn test_match_all_cases_dead_preserved() {
        // match x { _ if False => 1 }: no case can match, the runtime raises.
        // The match must stay so the error is not replaced by None.
        let cases = IR::Tuple(vec![IR::Tuple(vec![IR::PatternWildcard, IR::Bool(false), IR::Int(1)])]);
        let ir = IR::op(IROpCode::OpMatch, vec![IR::Ref("x".into(), 0, 1), cases]);
        let result = opt(ir.clone());
        assert_eq!(result.opcode(), Some(IROpCode::OpMatch));
        assert_eq!(result.args().unwrap().len(), 2);
    }

    #[test]
    fn test_match_single_catchall_keeps_scrutinee() {
        // match x { _ => 42 } → block(x, 42): evaluating x may raise or
        // have effects (calls), so it must be preserved
        let cases = IR::Tuple(vec![IR::Tuple(vec![IR::PatternWildcard, IR::None, IR::Int(42)])]);
        let scrutinee = IR::Ref("x".into(), 0, 1);
        let ir = IR::op(IROpCode::OpMatch, vec![scrutinee.clone(), cases]);
        let result = opt(ir);
        assert_eq!(result.opcode(), Some(IROpCode::OpBlock));
        assert_eq!(result.args().unwrap(), &[scrutinee, IR::Int(42)]);
    }
}
