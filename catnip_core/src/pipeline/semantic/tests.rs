//! Tests for semantic analysis passes.

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
        // Real lowering: OpIf([Tuple([(cond, then)]), else]).
        args: vec![
            IR::Tuple(vec![IR::Tuple(vec![
                IR::Ref("flag".into(), 0, 0),
                IR::op(
                    IROpCode::OpBlock,
                    vec![IR::op(
                        IROpCode::SetLocals,
                        vec![IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]), IR::Int(1), IR::Bool(false)],
                    )],
                ),
            ])]),
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
            // Real lowering: OpIf([Tuple([(cond, then)]), else]).
            IR::Tuple(vec![IR::Tuple(vec![
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
            ])]),
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

// A `Color` match arm `variant => block_value`, real OpMatch case shape.
fn color_arm(variant: &str, value: i64) -> IR {
    IR::Tuple(vec![
        IR::PatternEnum {
            enum_name: "Color".into(),
            variant_name: variant.into(),
        },
        IR::None,
        IR::op(IROpCode::OpBlock, vec![IR::Int(value)]),
    ])
}

// `c = Color.<variant>` as a SetLocals (single-target, no unpack).
fn assign_c_color(variant: &str) -> IR {
    IR::op(
        IROpCode::SetLocals,
        vec![
            IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]),
            IR::op(
                IROpCode::GetAttr,
                vec![IR::Ref("Color".into(), 0, 0), IR::String(variant.into())],
            ),
            IR::Bool(false),
        ],
    )
}

fn color_enum_def() -> IR {
    IR::op(
        IROpCode::EnumDef,
        vec![
            IR::String("Color".into()),
            IR::Tuple(vec![
                IR::String("red".into()),
                IR::String("green".into()),
                IR::String("blue".into()),
            ]),
        ],
    )
}

fn match_c_all_colors() -> IR {
    IR::Op {
        opcode: IROpCode::OpMatch,
        args: vec![
            IR::Ref("c".into(), 0, 0),
            IR::Tuple(vec![color_arm("red", 1), color_arm("green", 2), color_arm("blue", 3)]),
        ],
        kwargs: IndexMap::new(),
        tail: false,
        start_byte: 100,
        end_byte: 200,
    }
}

#[test]
fn test_i103_join_widens_divergent_reassignment() {
    // c = Color.red
    // if flag { c = 5 }                 (real OpIf layout: [Tuple([(cond, block)])])
    // match c { red => .., green => .., blue => .. }
    //
    // The branch reassigns c to a non-Color value, so after the if c is no
    // longer provably Color. The join widens it to Top -- whereas the old reset
    // restored c = Color and wrongly suppressed the warning. I103 must fire.
    let mut a = SemanticAnalyzer::new();
    let if_stmt = IR::op(
        IROpCode::OpIf,
        vec![IR::Tuple(vec![IR::Tuple(vec![
            IR::Ref("flag".into(), 0, 0),
            IR::op(
                IROpCode::OpBlock,
                vec![IR::op(
                    IROpCode::SetLocals,
                    vec![IR::Tuple(vec![IR::Ref("c".into(), 0, 0)]), IR::Int(5), IR::Bool(false)],
                )],
            ),
        ])])],
    );
    let ir = IR::Program(vec![
        color_enum_def(),
        assign_c_color("red"),
        if_stmt,
        match_c_all_colors(),
    ]);
    let result = a.analyze_full(&ir).unwrap();
    assert!(
        result.diagnostics.iter().any(|d| d.code == "I103"),
        "divergent reassignment in a branch must widen the scrutinee type (hole closed)"
    );
}

#[test]
fn test_i103_join_keeps_type_when_branches_agree() {
    // c = Color.red
    // if flag { c = Color.green } else { c = Color.blue }   (both stay Color)
    // match c { red, green, blue }                          (still exhaustive)
    //
    // Every path leaves c as a Color, so the join keeps the concrete type and
    // the match stays exhaustive: no spurious widening, no I103.
    let mut a = SemanticAnalyzer::new();
    let if_stmt = IR::op(
        IROpCode::OpIf,
        vec![
            IR::Tuple(vec![IR::Tuple(vec![
                IR::Ref("flag".into(), 0, 0),
                IR::op(IROpCode::OpBlock, vec![assign_c_color("green")]),
            ])]),
            IR::op(IROpCode::OpBlock, vec![assign_c_color("blue")]),
        ],
    );
    let ir = IR::Program(vec![
        color_enum_def(),
        assign_c_color("red"),
        if_stmt,
        match_c_all_colors(),
    ]);
    let result = a.analyze_full(&ir).unwrap();
    assert!(
        !result.diagnostics.iter().any(|d| d.code == "I103"),
        "branches that all keep c as Color must not widen the scrutinee (no spurious warning)"
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

// -----------------------------------------------------------------------
// SÉ3: type-hint binding + verification (E300)
// -----------------------------------------------------------------------

/// Diagnostic codes produced for a source program (parser -> IR -> analyzer).
fn diag_codes(source: &str) -> Vec<String> {
    let ir = parse_program(source);
    let mut a = SemanticAnalyzer::new();
    a.analyze_full(&ir)
        .unwrap()
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

/// Run the execution-path analysis (`analyze`, not `analyze_full`): an
/// `Error`-severity diagnostic (E300) is fatal and surfaces as `Err`.
fn analyze_result(source: &str) -> Result<IR, String> {
    let ir = parse_program(source);
    SemanticAnalyzer::new().analyze(&ir)
}

#[test]
fn test_se3_typed_param_enables_exhaustiveness() {
    // A typed param lets the match on it be checked: all variants covered and no
    // wildcard -> exhaustive -> no diagnostic.
    let src = "enum Color { red, green, blue }\n\
               f = (c: Color) => { match c { Color.red => { 1 } Color.green => { 2 } Color.blue => { 3 } } }";
    assert!(
        diag_codes(src).is_empty(),
        "typed param + exhaustive match: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_se3_untyped_param_unknown_scrutinee() {
    // Same match, untyped param: scrutinee type unknown -> I103 still fires.
    let src = "enum Color { red, green, blue }\n\
               f = (c) => { match c { Color.red => { 1 } Color.green => { 2 } Color.blue => { 3 } } }";
    assert!(
        diag_codes(src).iter().any(|c| c == "I103"),
        "untyped param -> I103: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_se3_param_default_type_mismatch() {
    assert_eq!(diag_codes("f = (x: int = \"no\") => { 0 }"), ["E300"]);
    assert!(
        diag_codes("f = (x: int = 0) => { 0 }").is_empty(),
        "matching default: no E300"
    );
}

#[test]
fn test_se3_param_no_default_no_false_positive() {
    // `(x: int)` has no default (encoded as None); must not be flagged int vs None.
    assert!(diag_codes("f = (x: int) => { 0 }").is_empty());
}

#[test]
fn test_se3_struct_field_default_mismatch() {
    assert_eq!(diag_codes("struct P { x: int = \"no\" }"), ["E300"]);
    assert!(diag_codes("struct P { x: int = 0 }").is_empty());
}

#[test]
fn test_se3_return_type_mismatch() {
    assert_eq!(diag_codes("f = (): int => { \"no\" }"), ["E300"]);
    assert_eq!(diag_codes("f = (): int => { return \"no\" }"), ["E300"]);
    assert!(diag_codes("f = (): int => { 0 }").is_empty());
}

#[test]
fn test_se3_return_type_descends_into_branches() {
    // A `match`/`if` tail is a return position: a concrete mismatch in any
    // branch is caught, not only when the body is a single leaf expression.
    assert_eq!(diag_codes("f = (b): int => { if b { 0 } else { \"no\" } }"), ["E300"]);
    assert_eq!(
        diag_codes("f = (b): int => { match b { True => { 0 } _ => { \"no\" } } }"),
        ["E300"]
    );
    // Every branch matching the declared type: no diagnostic.
    assert!(diag_codes("f = (b): int => { if b { 0 } else { 1 } }").is_empty());
    assert!(diag_codes("f = (b): int => { match b { True => { 0 } _ => { 1 } } }").is_empty());
    // A union return accepts a branch producing any member (Option-like), so
    // the descent must not false-positive on `P | None`.
    assert!(diag_codes("struct P { x }\nf = (b): P | None => { if b { P(1) } else { None } }").is_empty());
    // Nested: a mismatch in a branch nested one level deeper is still caught.
    assert_eq!(
        diag_codes("f = (b): int => { match b { True => { if b { 0 } else { \"no\" } } _ => { 1 } } }"),
        ["E300"]
    );
}

// TH2-B étape 0(a): an Error-severity diagnostic (E300) is fatal on the
// execution path. The same programs only surface a diagnostic via the lint
// path (`analyze_full`, exercised by `diag_codes` above); here `analyze` must
// refuse them outright so a proven mismatch never reaches the VM.

/// Helper: assert `analyze` fails and the message carries the E300 code.
fn assert_e300_fatal(source: &str) {
    let err = analyze_result(source).expect_err("proven mismatch must be fatal on the execution path");
    assert!(err.contains("E300"), "fatal error must name E300, got: {err}");
}

#[test]
fn test_se3_analyze_rejects_proven_arg_mismatch() {
    assert_e300_fatal("f = (x: int) => { x }\nf(\"no\")");
    // Well-typed call on the same function must still pass.
    assert!(
        analyze_result("f = (x: int) => { x }\nf(1)").is_ok(),
        "matching positional arg must not be fatal"
    );
}

#[test]
fn test_se3_analyze_rejects_param_default_mismatch() {
    assert_e300_fatal("f = (x: int = \"no\") => { 0 }");
}

#[test]
fn test_se3_analyze_rejects_struct_field_default_mismatch() {
    assert_e300_fatal("struct P { x: int = \"no\" }");
}

#[test]
fn test_se3_analyze_rejects_return_mismatch() {
    assert_e300_fatal("f = (): int => { \"no\" }");
}

// Numeric tower (PEP 484): widening must not raise E300 -- a false positive
// that became code-breaking once E300 turned fatal. Narrowing stays an error.

#[test]
fn test_se3_numeric_tower_no_false_positive() {
    // int/bool widen to float; bool widens to int. Declaration and call sites.
    assert!(
        diag_codes("f = (x: float) => { x }\nf(1)").is_empty(),
        "int arg into float param"
    );
    assert!(
        diag_codes("f = (x: int) => { x }\nf(True)").is_empty(),
        "bool arg into int param"
    );
    assert!(
        diag_codes("f = (x: float = 0) => { 0 }").is_empty(),
        "int default for float param"
    );
    assert!(
        diag_codes("f = (): float => { 1 }").is_empty(),
        "int body for float return"
    );
    // The widening calls must also pass the fatal execution path.
    assert!(analyze_result("f = (x: float) => { x }\nf(1)").is_ok());
}

#[test]
fn test_se3_numeric_narrowing_still_fatal() {
    // float into an int slot is one-way refused (tower direction).
    assert_eq!(diag_codes("f = (x: int) => { x }\nf(1.5)"), ["E300"]);
    assert_e300_fatal("f = (x: int) => { x }\nf(1.5)");
}

// A union variant's concrete payload field is checked at its constructor
// (`U.A(...)`), reached through a GetAttr callee, exactly like a struct field.

#[test]
fn test_se3_union_variant_field_mismatch_fatal() {
    assert_eq!(diag_codes("union U { A(x: int) }\nU.A(1.5)"), ["E300"]);
    assert_e300_fatal("union U { A(x: int) }\nU.A(1.5)");
    // Multi-field: the mismatching argument is reported by its position.
    assert_eq!(diag_codes("union U { A(x: int, y: str) }\nU.A(1, 2)"), ["E300"]);
}

#[test]
fn test_se3_union_variant_field_conforming_ok() {
    // Exact type and numeric widening (int -> float) both pass, like structs.
    assert!(diag_codes("union U { A(x: int) }\nU.A(3)").is_empty());
    assert!(diag_codes("union U { A(x: float) }\nU.A(3)").is_empty());
}

#[test]
fn test_se3_union_generic_field_not_checked_at_construction() {
    // A type-parameter payload field (`Some(v: T)`) is not fixed at
    // construction: no concrete check fires, whatever the argument type.
    assert!(diag_codes("union Opt[T] { Some(v: T)\n None }\nOpt.Some(1.5)").is_empty());
    assert!(diag_codes("union Opt[T] { Some(v: T)\n None }\nOpt.Some(\"x\")").is_empty());
}

#[test]
fn test_se3_union_constructor_shadowed_name_not_checked() {
    // A reassigned union name may not refer to the union at the call site, so
    // its constructor is not trusted -- no false E300 (sound, mirrors structs).
    assert!(diag_codes("union U { A(x: int) }\nU = 5\nU.A(1.5)").is_empty());
}

#[test]
fn test_se3_struct_field_inference_drives_exhaustiveness() {
    // p.x is a Color field; matching only one variant -> I103 on Color.
    let src = "enum Color { red, green, blue }\n\
               struct P { x: Color }\n\
               p = P(Color.red)\n\
               match p.x { Color.red => { 1 } }";
    let codes = diag_codes(src);
    assert!(
        codes.iter().any(|c| c == "I103"),
        "field type should drive exhaustiveness: {codes:?}"
    );
}

#[test]
fn test_se3_unresolved_annotation_no_error() {
    // `Option[int]` is an unmodeled generic -> Top -> no binding, no E300.
    assert!(diag_codes("f = (x: Option[int] = 0) => { 0 }").is_empty());
}

#[test]
fn test_se3_composite_constructor_modeled() {
    // list/set/dict are modeled at the constructor level: a provable primitive
    // default into a `list` slot is a mismatch (E300), params ignored.
    assert!(diag_codes("f = (x: list[int] = 0) => { 0 }").contains(&"E300".to_string()));
    assert!(diag_codes("f = (x: set[int] = 0) => { 0 }").contains(&"E300".to_string()));
    assert!(diag_codes("f = (x: dict[str, int] = 0) => { 0 }").contains(&"E300".to_string()));
    // A bare `list`/`set`/`dict` annotation with no provable default stays clean.
    assert!(diag_codes("f = (x: list) => { 0 }").is_empty());
    assert!(diag_codes("f = (x: set) => { 0 }").is_empty());
}

#[test]
fn test_se3_composite_literal_inference() {
    // A list/dict literal infers to its constructor type, so a literal in a slot
    // of the wrong composite is caught statically (E300).
    assert!(diag_codes("f = (x: list) => { x }\nf({1: 2})").contains(&"E300".to_string()));
    assert!(diag_codes("f = (x: dict) => { x }\nf([1, 2, 3])").contains(&"E300".to_string()));
    // A list literal into a primitive slot is also a provable mismatch.
    assert!(diag_codes("f = (x: int) => { x }\nf([1, 2, 3])").contains(&"E300".to_string()));
    // A matching literal stays clean (constructor matches, params ignored).
    assert!(diag_codes("f = (x: list) => { x }\nf([1, 2, 3])").is_empty());
    assert!(diag_codes("f = (x: dict) => { x }\nf({1: 2})").is_empty());
    // Set is a distinct container: a set literal does not satisfy a list slot,
    // nor a list literal a set slot; a set literal in a set slot is clean.
    assert!(diag_codes("f = (x: list) => { x }\nf(set(1, 2, 3))").contains(&"E300".to_string()));
    assert!(diag_codes("f = (x: set) => { x }\nf([1, 2, 3])").contains(&"E300".to_string()));
    assert!(diag_codes("f = (x: set) => { x }\nf(set(1, 2, 3))").is_empty());
    // An untyped param is unaffected.
    assert!(diag_codes("f = (x) => { x }\nf([1, 2, 3])").is_empty());
}

// --- SÉ-bis: inter-branch isolation across an elif chain --------------------
// An `if/elif/else` lowers to OpIf([Tuple([(c1, b1), (c2, b2), ...]), else?]).
// Each (cond, block) pair is mutually exclusive, so an assignment in an earlier
// branch must not be visible when analyzing a later one. Without per-pair reset,
// the branches leak sequentially inside the Tuple and the scrutinee type seen in
// a later `elif` is wrong in both directions.

#[test]
fn test_sebis_elif_does_not_inherit_earlier_branch_type() {
    // c is an int on entry; the first branch reassigns it to a Color, the elif
    // then matches it. The elif must see c as int (entry), not Color (leaked):
    // matching it against Color variants is non-exhaustive -> I103 must fire.
    let src = "enum Color { red, green, blue }\n\
               c = 5\n\
               if flag1 { c = Color.red }\n\
               elif flag2 { match c { Color.red => { 1 } Color.green => { 2 } Color.blue => { 3 } } }";
    let codes = diag_codes(src);
    assert!(
        codes.iter().any(|c| c == "I103"),
        "elif must see c as int (entry), not the Color leaked from the prior branch: {codes:?}"
    );
}

#[test]
fn test_sebis_elif_no_false_positive_from_earlier_branch() {
    // Mirror image: c is a Color on entry, the first branch reassigns it to an
    // int, the elif matches it exhaustively over Color. The elif must see c as
    // Color (entry), not int (leaked): the match is exhaustive -> no I103.
    let src = "enum Color { red, green, blue }\n\
               c = Color.red\n\
               if flag1 { c = 5 }\n\
               elif flag2 { match c { Color.red => { 1 } Color.green => { 2 } Color.blue => { 3 } } }";
    let codes = diag_codes(src);
    assert!(
        !codes.iter().any(|c| c == "I103"),
        "elif must see c as Color (entry), not the int leaked from the prior branch: {codes:?}"
    );
}

// --- TH3 step 2: monomorphic inter-procedural argument checking -------------
// A call to a free function with a provably unique binding (or a struct
// constructor) is checked argument by argument against the declared types.
// E300 fires only on a provable concrete mismatch; anything unknown stays silent.

#[test]
fn test_th3_call_positional_arg_mismatch() {
    assert_eq!(diag_codes("f = (x: int) => { x }\nf(\"no\")"), ["E300"]);
    assert!(
        diag_codes("f = (x: int) => { x }\nf(3)").is_empty(),
        "matching positional arg: no E300"
    );
}

#[test]
fn test_th3_call_keyword_arg_mismatch() {
    assert_eq!(diag_codes("f = (x: int) => { x }\nf(x=\"no\")"), ["E300"]);
    assert!(diag_codes("f = (x: int) => { x }\nf(x=3)").is_empty());
}

#[test]
fn test_union_param_accepts_any_member() {
    // `int | str` accepts an int or a str arg; a float matches neither -> E300.
    assert!(diag_codes("f = (x: int | str) => { x }\nf(3)").is_empty());
    assert!(diag_codes("f = (x: int | str) => { x }\nf(\"a\")").is_empty());
    assert_eq!(diag_codes("f = (x: int | str) => { x }\nf(1.5)"), ["E300"]);
}

#[test]
fn test_union_param_follows_numeric_tower() {
    // A `bool` is acceptable where `int` is a member (bool <: int) -> no E300.
    assert!(diag_codes("f = (x: int | str) => { x }\nf(True)").is_empty());
}

#[test]
fn test_union_default_mismatch() {
    // Default provably outside the union -> E300; a member default is fine.
    assert_eq!(diag_codes("f = (x: int | str = 1.5) => { 0 }"), ["E300"]);
    assert!(diag_codes("f = (x: int | str = 1) => { 0 }").is_empty());
}

#[test]
fn test_union_optional_nominal_resolves() {
    // `Point | None` (the canonical optional) resolves to an enforceable union,
    // not `Top`: a default provably outside it is flagged at the declaration
    // site (proves the nominal member resolved, since `bind_params` runs during
    // the walk with `struct_fields` populated).
    let defs = "struct Point { x; y }\n";
    assert_eq!(
        diag_codes(&format!("{defs}f = (p: Point | None = \"bad\") => {{ 0 }}")),
        ["E300"]
    );
    // Nominal *arguments* at a call site are not statically checked: structs are
    // collected during the walk, after the `collect_unique_fns` pre-pass that
    // fills `fn_sigs`, so a nominal param resolves to `Top` there. A bad nominal
    // arg is caught by the runtime boundary, not E300. Pre-existing limit: same
    // behavior with the union and with a plain nominal param.
    assert!(diag_codes(&format!("{defs}f = (p: Point | None) => {{ p }}\nf(\"a\")")).is_empty());
    assert!(diag_codes(&format!("{defs}g = (p: Point) => {{ p }}\ng(\"a\")")).is_empty());
}

#[test]
fn test_union_unresolved_member_is_inert() {
    // A member that does not resolve (unmodeled generic `Option[int]`) makes the
    // whole union inert (Top), never partially enforced -> no E300 even on a bad
    // arg.
    assert!(
        diag_codes("f = (x: int | Option[int]) => { x }\nf(\"a\")").is_empty(),
        "union with unresolved member must stay inert: {:?}",
        diag_codes("f = (x: int | Option[int]) => { x }\nf(\"a\")")
    );
}

#[test]
fn test_union_with_composite_member_enforced() {
    // With list/set/dict modeled, a union of a primitive and a composite is
    // enforced: a provably-mismatched arg (str) is caught by E300.
    assert!(
        diag_codes("f = (x: int | list[int]) => { x }\nf(\"a\")").contains(&"E300".to_string()),
        "int | list should reject a str statically: {:?}",
        diag_codes("f = (x: int | list[int]) => { x }\nf(\"a\")")
    );
    assert!(
        diag_codes("f = (x: int | set[int]) => { x }\nf(\"a\")").contains(&"E300".to_string()),
        "int | set should reject a str statically: {:?}",
        diag_codes("f = (x: int | set[int]) => { x }\nf(\"a\")")
    );
}

#[test]
fn test_th3_reassigned_fn_not_checked() {
    // Two assignments -> not provably unique -> no check (sound).
    let src = "f = (x: int) => { x }\nf = (x: str) => { x }\nf(3)";
    assert!(
        diag_codes(src).is_empty(),
        "reassigned fn must not be checked: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_aliased_fn_not_checked() {
    // `g = f` uses f as a value -> not unique -> no check.
    let src = "f = (x: int) => { x }\ng = f\nf(\"no\")";
    assert!(
        diag_codes(src).is_empty(),
        "aliased fn must not be checked: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_param_shadowed_name_not_checked() {
    // `f` is also a parameter name -> excluded globally (sound vs shadowing).
    let src = "f = (x: int) => { x }\ng = (f) => { f(\"no\") }";
    assert!(
        diag_codes(src).is_empty(),
        "shadowed name must not be checked: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_vararg_stops_positional_check() {
    // First param is typed and checked; args at/after the variadic are not.
    assert_eq!(diag_codes("f = (x: int, *rest) => { x }\nf(\"no\", 1, 2)"), ["E300"]);
    assert!(
        diag_codes("f = (x: int, *rest) => { x }\nf(1, \"ok\", \"ok\")").is_empty(),
        "args absorbed by the variadic are not checked"
    );
}

#[test]
fn test_th3_default_param_arg_checked_when_provided() {
    // A provided arg is checked; omitting it is fine (no arity errors here).
    assert_eq!(diag_codes("f = (x: int = 0) => { x }\nf(\"no\")"), ["E300"]);
    assert!(
        diag_codes("f = (x: int = 0) => { x }\nf()").is_empty(),
        "omitted arg: no error"
    );
}

#[test]
fn test_th3_struct_constructor_arg_mismatch() {
    assert_eq!(diag_codes("struct P { x: int; y: int }\nP(1, \"no\")"), ["E300"]);
    assert!(
        diag_codes("struct P { x: int; y: int }\nP(1, 2)").is_empty(),
        "matching constructor args: no E300"
    );
}

#[test]
fn test_th3_forward_reference_checked() {
    // g calls f defined later: the global pre-pass records f's signature
    // regardless of textual order (covers mutual recursion).
    let src = "g = (n: int) => { f(\"no\") }\nf = (x: int) => { x }";
    assert_eq!(diag_codes(src), ["E300"]);
}

#[test]
fn test_th3_unknown_arg_type_no_error() {
    // Argument of unknown (Top) type -> no E300 (sound).
    let src = "f = (x: int) => { x }\ng = (y) => { f(y) }";
    assert!(
        diag_codes(src).is_empty(),
        "unknown arg type must not fire: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_pattern_bound_name_not_checked() {
    // `f` is captured by a match pattern -> a binding (like a parameter) ->
    // excluded globally, so the shadowing call is not checked (sound).
    let src = "f = (x: int) => { x }\ng = (v) => { match v { f => { f(\"no\") } } }";
    assert!(
        diag_codes(src).is_empty(),
        "pattern-bound name must not be checked: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_constructor_shadowed_by_param_not_checked() {
    // `P` is a parameter here, not the struct constructor: P(...) must not be
    // checked against the struct fields (sound vs struct-name shadowing).
    let src = "struct P { x: int; y: int }\nf = (P) => { P(1, \"no\") }";
    assert!(
        diag_codes(src).is_empty(),
        "constructor name shadowed by a param must not be checked: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_constructor_shadowed_by_local_not_checked() {
    // `P` is reassigned to a non-struct value: P(...) must not be checked.
    let src = "struct P { x: int }\nP = 5\nP(\"no\")";
    assert!(
        diag_codes(src).is_empty(),
        "constructor name shadowed by a local must not be checked: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_constructor_shadowed_by_unpacking_not_checked() {
    // `P` is bound by an unpacking assignment, not the struct constructor:
    // P(...) must not be checked against the struct fields.
    let src = "struct P { x: int; y: int }\nP, q = pair()\nP(1, \"no\")";
    assert!(
        diag_codes(src).is_empty(),
        "constructor name bound by unpacking must not be checked: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_constructor_shadowed_by_for_var_not_checked() {
    // `P` is a loop variable, not the struct constructor: P(...) in the body
    // must not be checked against the struct fields.
    let src = "struct P { x: int }\nfor P in items { P(\"no\") }";
    assert!(
        diag_codes(src).is_empty(),
        "constructor name bound by a for-loop variable must not be checked: {:?}",
        diag_codes(src)
    );
}

#[test]
fn test_th3_constructor_shadowed_by_except_binding_not_checked() {
    // `P` is the exception binding, not the struct constructor.
    let src = "struct P { x: int }\ntry { risky() } except { P: Error => { P(\"no\") } }";
    assert!(
        diag_codes(src).is_empty(),
        "constructor name bound by an except clause must not be checked: {:?}",
        diag_codes(src)
    );
}

// --- TH4 canal A: typed-arithmetic rewrite (Add -> AddInt/AddFloat) ---

/// `(x: ty?) => x <op> rhs`, with `op` always `Add`. `ty` = None means no
/// annotation. Returns the lambda IR; the body add sits at `args[1]`.
fn lambda_add(param_ty: Option<&str>, lhs: IR, rhs: IR) -> IR {
    let ty_node = match param_ty {
        Some(t) => IR::String(t.into()),
        None => IR::None,
    };
    let param = IR::Tuple(vec![IR::String("x".into()), IR::None, ty_node]);
    let add = IR::op(IROpCode::Add, vec![lhs, rhs]);
    IR::op(IROpCode::OpLambda, vec![IR::Tuple(vec![param]), add])
}

fn body_opcode(lambda: &IR) -> Option<IROpCode> {
    match lambda {
        IR::Op { args, .. } => args.get(1).and_then(|b| b.opcode()),
        _ => None,
    }
}

/// `(x: ty, y: ty) => x <op> y`, where both params share the annotation `ty`.
fn lambda_binop(param_ty: &str, op: IROpCode) -> IR {
    let p = |name: &str| IR::Tuple(vec![IR::String(name.into()), IR::None, IR::String(param_ty.into())]);
    let body = IR::op(op, vec![IR::Ref("x".into(), 0, 0), IR::Ref("y".into(), 0, 0)]);
    IR::op(IROpCode::OpLambda, vec![IR::Tuple(vec![p("x"), p("y")]), body])
}

#[test]
fn test_rewrite_typed_sub_mul() {
    use IROpCode::*;
    assert_eq!(
        body_opcode(&rewrite_typed_arith(lambda_binop("int", Sub))),
        Some(SubInt)
    );
    assert_eq!(
        body_opcode(&rewrite_typed_arith(lambda_binop("float", Sub))),
        Some(SubFloat)
    );
    assert_eq!(
        body_opcode(&rewrite_typed_arith(lambda_binop("int", Mul))),
        Some(MulInt)
    );
    assert_eq!(
        body_opcode(&rewrite_typed_arith(lambda_binop("float", Mul))),
        Some(MulFloat)
    );
}

#[test]
fn test_rewrite_div_is_float_only() {
    use IROpCode::*;
    // float/float -> DivFloat
    assert_eq!(
        body_opcode(&rewrite_typed_arith(lambda_binop("float", Div))),
        Some(DivFloat)
    );
    assert_eq!(
        body_opcode(&rewrite_typed_arith(lambda_binop("float", TrueDiv))),
        Some(DivFloat)
    );
    // int/int via `/` yields a float, so it stays the polymorphic op (no DivInt).
    assert_eq!(body_opcode(&rewrite_typed_arith(lambda_binop("int", Div))), Some(Div));
    assert_eq!(
        body_opcode(&rewrite_typed_arith(lambda_binop("int", TrueDiv))),
        Some(TrueDiv)
    );
}

#[test]
fn test_rewrite_add_int_on_annotated_param() {
    let l = lambda_add(Some("int"), IR::Ref("x".into(), 0, 0), IR::Int(1));
    assert_eq!(body_opcode(&rewrite_typed_arith(l)), Some(IROpCode::AddInt));
}

#[test]
fn test_rewrite_add_float_on_annotated_param() {
    let l = lambda_add(Some("float"), IR::Ref("x".into(), 0, 0), IR::Float(1.0));
    assert_eq!(body_opcode(&rewrite_typed_arith(l)), Some(IROpCode::AddFloat));
}

#[test]
fn test_rewrite_keeps_add_when_unproven() {
    // Unannotated param: x is not a runtime-proof int -> polymorphic Add stays.
    let l = lambda_add(None, IR::Ref("x".into(), 0, 0), IR::Int(1));
    assert_eq!(body_opcode(&rewrite_typed_arith(l)), Some(IROpCode::Add));
    // int param + float literal: no single-primitive proof -> Add.
    let l = lambda_add(Some("int"), IR::Ref("x".into(), 0, 0), IR::Float(1.0));
    assert_eq!(body_opcode(&rewrite_typed_arith(l)), Some(IROpCode::Add));
}

#[test]
fn test_rewrite_chains_on_typed_result() {
    // (x: int) => (x + 1) + 1 : inner add is AddInt, so the outer add is too.
    let inner = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 0, 0), IR::Int(1)]);
    let l = lambda_add(Some("int"), inner, IR::Int(1));
    let out = rewrite_typed_arith(l);
    assert_eq!(body_opcode(&out), Some(IROpCode::AddInt));
    if let IR::Op { args, .. } = &out {
        assert_eq!(args[1].args().unwrap()[0].opcode(), Some(IROpCode::AddInt));
    }
}

#[test]
fn test_rewrite_leaves_free_variables_polymorphic() {
    // Top-level `y + 1` with y free (not a proven param): Add untouched.
    let add = IR::op(IROpCode::Add, vec![IR::Ref("y".into(), 0, 0), IR::Int(1)]);
    match rewrite_typed_arith(IR::Program(vec![add])) {
        IR::Program(items) => assert_eq!(items[0].opcode(), Some(IROpCode::Add)),
        _ => panic!("expected Program"),
    }
}

#[test]
fn test_rewrite_skips_param_bound_by_except() {
    // (x: int) => { try { 1 } except { x: Error => { 0 } }; x + 1 } : the except
    // binding is a bare name string (not a Ref), rebinding x -> Add.
    let param = IR::Tuple(vec![IR::String("x".into()), IR::None, IR::String("int".into())]);
    let handler = IR::Tuple(vec![
        IR::Tuple(vec![IR::String("Error".into())]),
        IR::String("x".into()),
        IR::op(IROpCode::OpBlock, vec![IR::Int(0)]),
    ]);
    let try_expr = IR::op(
        IROpCode::OpTry,
        vec![
            IR::op(IROpCode::OpBlock, vec![IR::Int(1)]),
            IR::List(vec![handler]),
            IR::None,
        ],
    );
    let add = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 0, 0), IR::Int(1)]);
    let body = IR::op(IROpCode::OpBlock, vec![try_expr, add]);
    let lambda = IR::op(IROpCode::OpLambda, vec![IR::Tuple(vec![param]), body]);
    match rewrite_typed_arith(lambda) {
        IR::Op { args, .. } => match args.get(1) {
            Some(IR::Op { args: block, .. }) => {
                assert_eq!(block.last().unwrap().opcode(), Some(IROpCode::Add));
            }
            _ => panic!("expected block body"),
        },
        _ => panic!("expected lambda"),
    }
}

#[test]
fn test_rewrite_skips_reassigned_param() {
    // (x: int) => { x = 2.5; x + 1 } : x is rebound, so the declared `int` is no
    // longer a runtime fact -> Add stays polymorphic (would skip overloads/concat
    // otherwise). Guards the unsoundness flagged by the Codex review.
    let param = IR::Tuple(vec![IR::String("x".into()), IR::None, IR::String("int".into())]);
    let reassign = IR::op(
        IROpCode::SetLocals,
        vec![IR::Tuple(vec![IR::Ref("x".into(), 0, 0)]), IR::Float(2.5)],
    );
    let add = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 0, 0), IR::Int(1)]);
    let body = IR::op(IROpCode::OpBlock, vec![reassign, add]);
    let lambda = IR::op(IROpCode::OpLambda, vec![IR::Tuple(vec![param]), body]);
    match rewrite_typed_arith(lambda) {
        IR::Op { args, .. } => match args.get(1) {
            Some(IR::Op { args: block, .. }) => {
                assert_eq!(block.last().unwrap().opcode(), Some(IROpCode::Add));
            }
            _ => panic!("expected block body"),
        },
        _ => panic!("expected lambda"),
    }
}

#[test]
fn test_rewrite_skips_param_shadowed_by_local_def() {
    // (x: int) => { struct x { a }; x + 1 } : the local struct binds the name x,
    // shadowing the param -> Add. Covers the type-definition binding site.
    let param = IR::Tuple(vec![IR::String("x".into()), IR::None, IR::String("int".into())]);
    let def = IR::op(IROpCode::OpStruct, vec![IR::String("x".into()), IR::Tuple(vec![])]);
    let add = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 0, 0), IR::Int(1)]);
    let body = IR::op(IROpCode::OpBlock, vec![def, add]);
    let lambda = IR::op(IROpCode::OpLambda, vec![IR::Tuple(vec![param]), body]);
    match rewrite_typed_arith(lambda) {
        IR::Op { args, .. } => match args.get(1) {
            Some(IR::Op { args: block, .. }) => {
                assert_eq!(block.last().unwrap().opcode(), Some(IROpCode::Add));
            }
            _ => panic!("expected block body"),
        },
        _ => panic!("expected lambda"),
    }
}

#[test]
fn test_rewrite_skips_param_bound_by_star_pattern() {
    // (x: int) => { match z { [a, *x] => { 0 } }; x + 1 } : the star pattern `*x`
    // binds x (transformed to ("*", "x")), so it is no longer proven -> Add.
    let param = IR::Tuple(vec![IR::String("x".into()), IR::None, IR::String("int".into())]);
    let star = IR::Tuple(vec![IR::String("*".into()), IR::String("x".into())]);
    let pat = IR::PatternTuple(vec![IR::PatternVar("a".into()), star]);
    let case = IR::Tuple(vec![pat, IR::None, IR::op(IROpCode::OpBlock, vec![IR::Int(0)])]);
    let match_expr = IR::op(
        IROpCode::OpMatch,
        vec![IR::Ref("z".into(), 0, 0), IR::Tuple(vec![case])],
    );
    let add = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 0, 0), IR::Int(1)]);
    let body = IR::op(IROpCode::OpBlock, vec![match_expr, add]);
    let lambda = IR::op(IROpCode::OpLambda, vec![IR::Tuple(vec![param]), body]);
    match rewrite_typed_arith(lambda) {
        IR::Op { args, .. } => match args.get(1) {
            Some(IR::Op { args: block, .. }) => {
                assert_eq!(block.last().unwrap().opcode(), Some(IROpCode::Add));
            }
            _ => panic!("expected block body"),
        },
        _ => panic!("expected lambda"),
    }
}

#[test]
fn test_rewrite_skips_param_bound_by_match_pattern() {
    // (x: int) => { match z { x => { 0 } }; x + 1 } : the variable pattern `x`
    // rebinds x, so it is no longer a proven int -> Add stays. Guards the match
    // pattern unsoundness flagged by the Codex review.
    let param = IR::Tuple(vec![IR::String("x".into()), IR::None, IR::String("int".into())]);
    let case = IR::Tuple(vec![
        IR::PatternVar("x".into()),
        IR::None,
        IR::op(IROpCode::OpBlock, vec![IR::Int(0)]),
    ]);
    let match_expr = IR::op(
        IROpCode::OpMatch,
        vec![IR::Ref("z".into(), 0, 0), IR::Tuple(vec![case])],
    );
    let add = IR::op(IROpCode::Add, vec![IR::Ref("x".into(), 0, 0), IR::Int(1)]);
    let body = IR::op(IROpCode::OpBlock, vec![match_expr, add]);
    let lambda = IR::op(IROpCode::OpLambda, vec![IR::Tuple(vec![param]), body]);
    match rewrite_typed_arith(lambda) {
        IR::Op { args, .. } => match args.get(1) {
            Some(IR::Op { args: block, .. }) => {
                assert_eq!(block.last().unwrap().opcode(), Some(IROpCode::Add));
            }
            _ => panic!("expected block body"),
        },
        _ => panic!("expected lambda"),
    }
}

// -----------------------------------------------------------------------
// Generic nominal unions (Option[int], Result[T, E]) -- static layer (1b)
// -----------------------------------------------------------------------

const OPTION_DEF: &str = "union Option[T] { Some(value: T); None }\n";
const RESULT_DEF: &str = "union Result[T, E] { Ok(value: T); Err(error: E) }\n";

#[test]
fn test_generic_union_arity_ok_no_value() {
    // A well-formed generic annotation with no value flowing is not flagged.
    let src = format!("{OPTION_DEF}f = (x: Option[int]) => {{ 0 }}");
    assert!(diag_codes(&src).is_empty(), "{:?}", diag_codes(&src));
}

#[test]
fn test_generic_union_arity_mismatch() {
    // Option has one type parameter; two arguments is a provable arity error.
    let src = format!("{OPTION_DEF}f = (x: Option[int, str]) => {{ 0 }}");
    assert_eq!(diag_codes(&src), ["E300"]);
}

#[test]
fn test_non_generic_union_with_args_is_arity_error() {
    // A union with no type parameters given an argument: 0-vs-1 arity mismatch.
    let src = "union Opt { Some(v); None }\nf = (x: Opt[int]) => { 0 }";
    assert_eq!(diag_codes(src), ["E300"]);
}

#[test]
fn test_generic_union_return_payload_mismatch() {
    // Extended inference: Option.Some("s") infers Option[str], mismatching the
    // declared Option[int] return type.
    let src = format!("{OPTION_DEF}f = (): Option[int] => {{ Option.Some(\"s\") }}");
    assert_eq!(diag_codes(&src), ["E300"]);
}

#[test]
fn test_generic_union_return_payload_ok() {
    let src = format!("{OPTION_DEF}f = (): Option[int] => {{ Option.Some(1) }}");
    assert!(diag_codes(&src).is_empty(), "{:?}", diag_codes(&src));
}

#[test]
fn test_generic_union_return_none_defers() {
    // Option.None carries no T information -> bare Option -> defers, no mismatch.
    let src = format!("{OPTION_DEF}f = (): Option[int] => {{ Option.None }}");
    assert!(diag_codes(&src).is_empty(), "{:?}", diag_codes(&src));
}

#[test]
fn test_generic_union_return_tower_widening() {
    // Covariant argument via the numeric tower: an int payload into Option[float].
    let src = format!("{OPTION_DEF}f = (): Option[float] => {{ Option.Some(1) }}");
    assert!(diag_codes(&src).is_empty(), "{:?}", diag_codes(&src));
}

#[test]
fn test_result_return_first_param_mismatch() {
    // Result.Ok("s") infers Result[str, ?]; the declared Result[int, str] rejects
    // position 0 (str vs int); position 1 (E) is unbound -> Top -> defers.
    let src = format!("{RESULT_DEF}f = (): Result[int, str] => {{ Result.Ok(\"s\") }}");
    assert_eq!(diag_codes(&src), ["E300"]);
}

#[test]
fn test_result_return_ok_matches() {
    let src = format!("{RESULT_DEF}f = (): Result[int, str] => {{ Result.Ok(1) }}");
    assert!(diag_codes(&src).is_empty(), "{:?}", diag_codes(&src));
}

// -----------------------------------------------------------------------
// FT: function types ((int) -> bool) -- static half (grammar, lattice, E300)
// -----------------------------------------------------------------------

#[test]
fn test_ft_conforming_lambda_accepted() {
    // Exact match, and contravariant widening (a float-taking callback serves
    // an int-taking slot; an int-returning callback serves a float slot).
    let src = "apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((y: int) => { y + 1 }, 2)";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
    let src = "apply = (cb: (int) -> float, x: int) => { cb(x) }\napply((y: float): int => { 1 }, 2)";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_lambda_arity_mismatch_rejected() {
    let src = "apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((a: int, b: int) => { a }, 2)";
    assert_eq!(diag_codes(src), ["E300"]);
}

#[test]
fn test_ft_lambda_param_contravariance() {
    // A str-taking callback cannot serve an int-taking slot (the slot will
    // feed it ints): provable mismatch.
    let src = "apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((s: str) => { 0 }, 2)";
    assert_eq!(diag_codes(src), ["E300"]);
}

#[test]
fn test_ft_lambda_return_covariance() {
    let src = "apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((y: int): str => { \"s\" }, 2)";
    assert_eq!(diag_codes(src), ["E300"]);
}

#[test]
fn test_ft_non_callable_rejected() {
    let src = "apply = (cb: (int) -> int, x: int) => { cb(x) }\napply(5, 2)";
    assert_eq!(diag_codes(src), ["E300"]);
}

#[test]
fn test_ft_unannotated_lambda_defers() {
    // Unannotated lambda params infer to Top: same arity is never a provable
    // mismatch (runtime boundary takes over).
    let src = "apply = (cb: (int) -> int, x: int) => { cb(x) }\napply((y) => { y }, 2)";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_callback_call_arity() {
    // The declared arity is the contract at the callback's own call sites.
    let src = "f = (cb: (int) -> int) => { cb(1, 2) }";
    assert_eq!(diag_codes(src), ["E300"]);
    let src = "f = (cb: (int) -> int) => { cb(1) }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_callback_call_arg_type() {
    let src = "f = (cb: (int) -> int) => { cb(\"s\") }";
    assert_eq!(diag_codes(src), ["E300"]);
}

#[test]
fn test_ft_return_propagates_through_call() {
    // The declared return feeds local inference: cb() is str, g wants int.
    let src = "g = (x: int) => { x }\nf = (cb: () -> str) => { g(cb()) }";
    assert_eq!(diag_codes(src), ["E300"]);
    let src = "g = (x: int) => { x }\nf = (cb: () -> int) => { g(cb()) }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_union_member_and_right_absorption() {
    // A function type as the last member of a union parses and stays inert
    // statically (OneOf slot); the arrow absorbs a trailing union into the
    // return type (one member, return int | None).
    let src = "f = (cb: None | (int) -> int) => { 0 }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
    let src = "f = (cb: (int) -> int | None) => { 0 }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_nested_forms_resolve() {
    // Higher-order (a function taking a function), and a function type as a
    // composite parameter: parse + resolve without diagnostics.
    let src = "f = (g: ((int) -> int) -> int) => { 0 }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
    let src = "f = (hs: list[(int) -> int]) => { 0 }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

// -----------------------------------------------------------------------
// FT review findings: binding shadows, match joins, invariant tolerance
// -----------------------------------------------------------------------

#[test]
fn test_ft_for_target_shadow_not_checked() {
    // The loop variable shadows the outer callback: its calls must not be
    // checked against the stale outer type (E300 is fatal).
    let src = "apply = (cb: (int) -> int, items: list) => { for cb in items { cb(1, 2) } }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_except_binding_shadow_not_checked() {
    let src = "g = (cb: (int) -> int) => { try { cb(1) } except Error => cb { cb(1, 2) } }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_star_pattern_shadow_not_checked() {
    // A star pattern binds `rest`; the outer function-typed `rest` must not
    // be call-checked inside the arm.
    let src = "g = (rest: (int) -> int, v) => { match v { (a, *rest) => { rest(1, 2) } _ => { 0 } } }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_match_arm_assignment_flows_out() {
    // Arms join like if branches: an assignment surviving every arm flows
    // out. Missed-detection direction: the arm rebinds x to str, g wants int.
    let src = "g = (n: int) => { n }\nx = 0\nmatch x { _ => { x = \"hello\" } }\ng(x)";
    assert_eq!(diag_codes(src), ["E300"]);
    // False-rejection direction: the arm makes x an int; no diagnostic.
    let src = "g = (n: int) => { n }\nx = \"hello\"\nmatch x { _ => { x = 5 } }\ng(x)";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_match_divergent_arms_widen() {
    // Two arms assign different types: the join widens to Top -> no check.
    let src = "g = (n: int) => { n }\nx = 0\nmatch x { 0 => { x = \"s\" } _ => { x = 1 } }\ng(x)";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_invariant_element_tolerates_top() {
    // A list of unannotated-return lambdas passed through a VARIABLE takes
    // the invariant path: Fn([Int], Top) vs Fn([Int], Int) must not be a
    // provable mismatch (structural equality was a fatal false rejection).
    let src = "f = (hs: list[(int) -> int]) => { 1 }\nxs = [(y: int) => { y }]\nf(xs)";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
    // A provable element mismatch through a variable still rejects.
    let src = "f = (hs: list[(int) -> str]) => { 1 }\nxs = [(y: int): int => { y }]\nf(xs)";
    assert_eq!(diag_codes(src), ["E300"]);
}

#[test]
fn test_ft_nested_generic_arity_checked() {
    // A generic-union arity error nested in a function type's parameter is
    // caught like everywhere else.
    let src = "union Option[T] { Some(value: T); None }\nf = (g: (Option[int, str]) -> int) => { 0 }";
    assert_eq!(diag_codes(src), ["E300"]);
    let src = "union Option[T] { Some(value: T); None }\nf = (g: (Option[int]) -> int) => { 0 }";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_assigned_fn_return_not_propagated() {
    // An assigned function variable can be rebound by a closure between the
    // visible flow points: its declared return must not feed inference (the
    // same trust rule as the call check).
    let src =
        "f = (x: int): str => { \"s\" }\nh = () => { f = (x: int): int => { 0 } }\nh()\ng = (z: int) => { z }\ng(f(1))";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
}

#[test]
fn test_ft_defaulted_lambda_not_arity_checked() {
    // A defaulted parameter makes the arity open: the static half must not
    // conclude from the raw count (the runtime boundary reads the real
    // defaults). Concluding rejected `(x, y = 10)` against `(int) -> int`.
    let src = "apply = (cb: (int) -> int) => { cb(1) }\napply((x, y = 10) => { x + y })";
    assert!(diag_codes(src).is_empty(), "{:?}", diag_codes(src));
    // A plain 2-param lambda is still a provable arity mismatch.
    let src = "apply = (cb: (int) -> int) => { cb(1) }\napply((x, y) => { x + y })";
    assert_eq!(diag_codes(src), ["E300"]);
}
