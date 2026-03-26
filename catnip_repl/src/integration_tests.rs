// FILE: catnip_repl/src/integration_tests.rs
//! Integration tests: executor + completer + hints working together.
//!
//! Verifies that variables defined via execution become available
//! for completion and hints, matching the real REPL workflow.

use crate::completer::CatnipCompleter;
use crate::executor::ReplExecutor;
use crate::hints::HintEngine;

#[test]
fn test_completer_after_variable() {
    let mut executor = ReplExecutor::new().unwrap();
    executor.execute("my_var = 42").unwrap();

    let mut completer = CatnipCompleter::new();
    completer.set_variables(executor.get_variable_names());

    let suggestions = completer.complete("my_", 3);
    let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
    assert!(values.contains(&"my_var"));
}

#[test]
fn test_completer_after_function() {
    let mut executor = ReplExecutor::new().unwrap();
    executor.execute("fact = (n) => { n }").unwrap();

    let mut completer = CatnipCompleter::new();
    completer.set_variables(executor.get_variable_names());

    let suggestions = completer.complete("fac", 3);
    let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
    assert!(values.contains(&"fact"));
}

#[test]
fn test_hints_after_variable() {
    let mut executor = ReplExecutor::new().unwrap();
    executor.execute("fibonacci = 1").unwrap();

    let mut hints = HintEngine::new();
    hints.set_variables(executor.get_variable_names());

    let hint = hints.get_hint("fib", 3);
    assert_eq!(hint, Some("onacci".to_string()));
}

#[test]
fn test_variable_accumulation() {
    let mut executor = ReplExecutor::new().unwrap();
    executor.execute("x = 1").unwrap();
    executor.execute("y = 2").unwrap();
    executor.execute("z = 3").unwrap();

    let mut completer = CatnipCompleter::new();
    completer.set_variables(executor.get_variable_names());

    let names = executor.get_variable_names();
    assert!(names.contains(&"x".to_string()));
    assert!(names.contains(&"y".to_string()));
    assert!(names.contains(&"z".to_string()));
}

#[test]
fn test_builtins_not_duplicated() {
    let mut executor = ReplExecutor::new().unwrap();
    executor.execute("x = 1").unwrap();

    let mut completer = CatnipCompleter::new();
    completer.set_variables(executor.get_variable_names());

    let suggestions = completer.complete("sor", 3);
    let sorted_count = suggestions.iter().filter(|s| s.text == "sorted").count();
    assert_eq!(sorted_count, 1);
    let sorted_s = suggestions.iter().find(|s| s.text == "sorted").unwrap();
    assert_eq!(sorted_s.category, "builtin");
}

#[test]
fn test_struct_field_completion() {
    let mut executor = ReplExecutor::new().unwrap();
    executor.execute("struct Point { x; y }").unwrap();

    let mut completer = CatnipCompleter::new();
    completer.set_variables(executor.get_variable_names());

    let suggestions = completer.complete("Poi", 3);
    let values: Vec<_> = suggestions.iter().map(|s| s.text.as_str()).collect();
    assert!(values.contains(&"Point"));
}
