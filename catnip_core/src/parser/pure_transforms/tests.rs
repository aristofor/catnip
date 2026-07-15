//! Tests for pure IR transforms.

use super::*;

fn parse_and_transform(source: &str) -> TransformResult {
    use tree_sitter::Parser;

    let language = catnip_grammar::get_language();
    let mut parser = Parser::new();
    parser.set_language(&language).unwrap();

    let tree = parser.parse(source, None).unwrap();
    let root = tree.root_node();
    let children = named_children(&root);
    transform(children[0], source)
}

#[test]
fn test_transform_number() {
    let result = parse_and_transform("42").unwrap();
    assert_eq!(result, IR::Int(42));
}

#[test]
fn test_transform_addition() {
    let result = parse_and_transform("2 + 3").unwrap();

    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::Add);
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], IR::Int(2));
            assert_eq!(args[1], IR::Int(3));
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_string() {
    let result = parse_and_transform("\"hello\"").unwrap();
    assert_eq!(result, IR::String("hello".into()));
}

#[test]
fn test_transform_bool() {
    let result = parse_and_transform("True").unwrap();
    assert_eq!(result, IR::Bool(true));
}

#[test]
fn test_transform_raise_bare() {
    let result = parse_and_transform("raise").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpRaise);
            assert!(args.is_empty());
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_raise_expr() {
    let result = parse_and_transform("raise ValueError(\"msg\")").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpRaise);
            assert_eq!(args.len(), 1);
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_try_except_wildcard() {
    let result = parse_and_transform("try { 1 } except { _ => { 2 } }").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpTry);
            assert_eq!(args.len(), 3);
            // args[2] = None (no finally)
            assert_eq!(args[2], IR::None);
            if let IR::List(handlers) = &args[1] {
                assert_eq!(handlers.len(), 1);
                if let IR::Tuple(clause) = &handlers[0] {
                    assert_eq!(clause[0], IR::List(vec![])); // wildcard = empty types
                    assert_eq!(clause[1], IR::None); // no binding
                } else {
                    panic!("Expected Tuple clause");
                }
            } else {
                panic!("Expected List handlers");
            }
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_try_except_typed_with_binding() {
    let result = parse_and_transform("try { 1 } except { e: TypeError => { 2 } }").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpTry);
            if let IR::List(handlers) = &args[1] {
                assert_eq!(handlers.len(), 1);
                if let IR::Tuple(clause) = &handlers[0] {
                    assert_eq!(clause[0], IR::List(vec![IR::String("TypeError".into())]));
                    assert_eq!(clause[1], IR::String("e".into()));
                } else {
                    panic!("Expected Tuple clause");
                }
            } else {
                panic!("Expected List handlers");
            }
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_try_finally() {
    let result = parse_and_transform("try { 1 } finally { 2 }").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpTry);
            assert_eq!(args.len(), 3);
            assert_eq!(args[1], IR::List(vec![])); // no handlers
            assert_ne!(args[2], IR::None); // finally present
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_union_nullary() {
    let result = parse_and_transform("union Color { red; green; blue }").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::UnionDef);
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], IR::String("Color".into()));
            assert_eq!(args[1], IR::List(vec![])); // no type params
            if let IR::List(variants) = &args[2] {
                assert_eq!(variants.len(), 3);
                assert_eq!(variants[0], IR::Tuple(vec![IR::String("red".into()), IR::List(vec![])]));
            } else {
                panic!("Expected variants list");
            }
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_union_with_payload() {
    let result = parse_and_transform("union Option { Some(value); None; }").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::UnionDef);
            assert_eq!(args[0], IR::String("Option".into()));
            if let IR::List(variants) = &args[2] {
                assert_eq!(variants.len(), 2);
                // Some(value) -- one field, no type
                if let IR::Tuple(some_tuple) = &variants[0] {
                    assert_eq!(some_tuple[0], IR::String("Some".into()));
                    if let IR::List(fields) = &some_tuple[1] {
                        assert_eq!(fields.len(), 1);
                        assert_eq!(fields[0], IR::Tuple(vec![IR::String("value".into()), IR::None]));
                    } else {
                        panic!("Expected fields list");
                    }
                } else {
                    panic!("Expected variant tuple");
                }
                // None -- nullary
                assert_eq!(
                    variants[1],
                    IR::Tuple(vec![IR::String("None".into()), IR::List(vec![])])
                );
            } else {
                panic!("Expected variants list");
            }
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_union_generics_and_types() {
    let result = parse_and_transform("union Result[T, E] { Ok(value: T); Err(error: E); }").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::UnionDef);
            assert_eq!(args[0], IR::String("Result".into()));
            assert_eq!(args[1], IR::List(vec![IR::String("T".into()), IR::String("E".into())]));
            if let IR::List(variants) = &args[2] {
                assert_eq!(variants.len(), 2);
                if let IR::Tuple(ok_tuple) = &variants[0] {
                    if let IR::List(fields) = &ok_tuple[1] {
                        assert_eq!(
                            fields[0],
                            IR::Tuple(vec![IR::String("value".into()), IR::String("T".into())])
                        );
                    } else {
                        panic!("Expected fields list");
                    }
                }
            } else {
                panic!("Expected variants list");
            }
        }
        _ => panic!("Expected Op node"),
    }
}

#[test]
fn test_transform_union_duplicate_variant() {
    let result = parse_and_transform("union Bad { a; a; }");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("duplicate variant"), "got: {err}");
}

#[test]
fn test_transform_union_duplicate_field() {
    let result = parse_and_transform("union Bad { Variant(x, x); }");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("duplicate field"), "got: {err}");
}

#[test]
fn test_transform_union_empty() {
    let result = parse_and_transform("union Empty { }");
    // empty union is rejected by the parser layer
    assert!(result.is_err());
}

/// Extract the first match-case pattern from a parsed `match` expression.
///
/// OpMatch args layout: `[scrutinee, Tuple([case_1, case_2, ...])]`
/// where each case is `Tuple([pattern, guard, block])`.
fn match_first_pattern(result: &IR) -> &IR {
    let match_args = match result {
        IR::Op { args, .. } => args,
        _ => panic!("Expected OpMatch"),
    };
    let cases_tuple = match &match_args[1] {
        IR::Tuple(c) => c,
        other => panic!("Expected cases Tuple, got {:?}", other),
    };
    match &cases_tuple[0] {
        IR::Tuple(case) => &case[0],
        other => panic!("Expected case tuple, got {:?}", other),
    }
}

/// Plain struct pattern: `Point{x, y}` -- no variant, two fields.
#[test]
fn test_transform_pattern_struct_plain() {
    let result = parse_and_transform("match p { Point{x, y} => { x } }").unwrap();
    match match_first_pattern(&result) {
        IR::PatternStruct { name, variant, fields } => {
            assert_eq!(name, "Point");
            assert_eq!(*variant, None);
            assert_eq!(fields, &vec!["x".to_string(), "y".to_string()]);
        }
        other => panic!("Expected PatternStruct, got {:?}", other),
    }
}

/// Union variant pattern: `Option.Some{value}` -- variant=Some, fields=[value].
/// Prior to this fix, `Some` was being read as a field name, producing
/// `fields=["Some", "value"]` which would silently bind a variable named
/// `Some`. The fix uses `child_by_field_name`.
#[test]
fn test_transform_pattern_struct_union_variant() {
    let result = parse_and_transform("match opt { Option.Some{value} => { value } }").unwrap();
    match match_first_pattern(&result) {
        IR::PatternStruct { name, variant, fields } => {
            assert_eq!(name, "Option");
            assert_eq!(variant.as_deref(), Some("Some"));
            assert_eq!(fields, &vec!["value".to_string()]);
        }
        other => panic!("Expected PatternStruct, got {:?}", other),
    }
}

// --- SÉ2: type annotations carried (parsed, inert) -----------------------

/// A typed lambda param carries its annotation as the 3rd tuple element
/// `(name, default, type)`. The type is parsed but unused at this stage.
#[test]
fn test_lambda_param_carries_type_annotation() {
    let result = parse_and_transform("(x: int) => { x }").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpLambda);
            let params = match &args[0] {
                IR::Tuple(p) => p,
                other => panic!("Expected params tuple, got {:?}", other),
            };
            assert_eq!(
                params[0],
                IR::Tuple(vec![IR::String("x".into()), IR::None, IR::String("int".into())])
            );
        }
        other => panic!("Expected OpLambda, got {:?}", other),
    }
}

/// An unannotated param keeps the same 3-element shape with type = None.
#[test]
fn test_lambda_param_unannotated_type_is_none() {
    let result = parse_and_transform("(x) => { x }").unwrap();
    match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpLambda);
            let params = match &args[0] {
                IR::Tuple(p) => p,
                other => panic!("Expected params tuple, got {:?}", other),
            };
            assert_eq!(params[0], IR::Tuple(vec![IR::String("x".into()), IR::None, IR::None]));
        }
        other => panic!("Expected OpLambda, got {:?}", other),
    }
}

/// A return type rides as OpLambda args[2], present only when annotated.
#[test]
fn test_lambda_return_type_carried() {
    let typed = parse_and_transform("(x) : int => { x }").unwrap();
    match typed {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpLambda);
            assert_eq!(args.len(), 3);
            assert_eq!(args[2], IR::String("int".into()));
        }
        other => panic!("Expected OpLambda, got {:?}", other),
    }

    let untyped = parse_and_transform("(x) => { x }").unwrap();
    match untyped {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpLambda);
            assert_eq!(args.len(), 2);
        }
        other => panic!("Expected OpLambda, got {:?}", other),
    }
}

/// A struct field carries its annotation as a 4th tuple element appended after
/// `(name, has_default, default)`; readers indexing 0..=2 stay unaffected.
#[test]
fn test_struct_field_carries_type_annotation() {
    let result = parse_and_transform("struct P { x: int; y; z: str = w }").unwrap();
    let args = match result {
        IR::Op { opcode, args, .. } => {
            assert_eq!(opcode, IROpCode::OpStruct);
            args
        }
        other => panic!("Expected OpStruct, got {:?}", other),
    };
    let fields = match &args[1] {
        IR::Tuple(f) => f,
        other => panic!("Expected fields tuple, got {:?}", other),
    };
    // typed, no default
    assert_eq!(
        fields[0],
        IR::Tuple(vec![
            IR::String("x".into()),
            IR::Bool(false),
            IR::None,
            IR::String("int".into())
        ])
    );
    // untyped, no default -> type slot is None
    assert_eq!(
        fields[1],
        IR::Tuple(vec![IR::String("y".into()), IR::Bool(false), IR::None, IR::None])
    );
    // typed WITH default -> has_default true, type at index 3 (default value not asserted)
    match &fields[2] {
        IR::Tuple(t) => {
            assert_eq!(t.len(), 4);
            assert_eq!(t[0], IR::String("z".into()));
            assert_eq!(t[1], IR::Bool(true));
            assert_eq!(t[3], IR::String("str".into()));
        }
        other => panic!("Expected field tuple, got {:?}", other),
    }
}

/// Returns the method tuples of an OpStruct/TraitDef (always the last arg when
/// methods are present).
fn methods_of(ir: &IR) -> &Vec<IR> {
    match ir {
        IR::Op { args, .. } => match args.last() {
            Some(IR::List(m)) => m,
            other => panic!("Expected methods list as last arg, got {:?}", other),
        },
        other => panic!("Expected Op node, got {:?}", other),
    }
}

/// SÉ2 fix: an abstract method has no body/OpLambda, so its parsed signature
/// (param tuple + return type) is preserved in a 4th element instead of being
/// dropped. Abstract is still flagged by the None body slot (index 1).
#[test]
fn test_abstract_struct_method_carries_signature() {
    let result = parse_and_transform("struct S { @abstract area(self): float }").unwrap();
    let methods = methods_of(&result);
    assert_eq!(methods.len(), 1);
    let method = match &methods[0] {
        IR::Tuple(t) => t,
        other => panic!("Expected method tuple, got {:?}", other),
    };
    assert_eq!(method.len(), 4);
    assert_eq!(method[0], IR::String("area".into()));
    assert_eq!(method[1], IR::None); // abstract: no body
    match &method[3] {
        IR::Tuple(sig) => {
            // signature = (params, return_type) ; struct wraps params in a Tuple
            assert_eq!(sig[1], IR::String("float".into()));
            match &sig[0] {
                IR::Tuple(params) => {
                    assert_eq!(
                        params[0],
                        IR::Tuple(vec![IR::String("self".into()), IR::None, IR::None])
                    );
                }
                other => panic!("Expected params tuple, got {:?}", other),
            }
        }
        other => panic!("Expected signature tuple, got {:?}", other),
    }
}

/// Same fix on the trait branch (params wrapped in a List there).
#[test]
fn test_abstract_trait_method_carries_signature() {
    let result = parse_and_transform("trait T { @abstract area(self): float }").unwrap();
    let methods = methods_of(&result);
    assert_eq!(methods.len(), 1);
    let method = match &methods[0] {
        IR::Tuple(t) => t,
        other => panic!("Expected method tuple, got {:?}", other),
    };
    assert_eq!(method.len(), 4);
    assert_eq!(method[1], IR::None);
    match &method[3] {
        IR::Tuple(sig) => {
            assert_eq!(sig[1], IR::String("float".into()));
            match &sig[0] {
                IR::List(params) => {
                    assert_eq!(
                        params[0],
                        IR::Tuple(vec![IR::String("self".into()), IR::None, IR::None])
                    );
                }
                other => panic!("Expected params list, got {:?}", other),
            }
        }
        other => panic!("Expected signature tuple, got {:?}", other),
    }
}
