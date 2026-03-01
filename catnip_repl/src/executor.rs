// FILE: catnip_repl/src/executor.rs
//! Execution pipeline for REPL Phase 2b
//!
//! Pipeline: Source → Tree-sitter → IRPure → Semantic → Bytecode → VM
//!
//! Uses the VM with bytecode compilation.

use catnip_rs::get_tree_sitter_language;
use catnip_rs::ir;
use catnip_rs::parser::transform_pure;
use catnip_rs::standalone::{convert, SemanticAnalyzer};
use catnip_rs::vm::compiler::Compiler;
use catnip_rs::vm::VM;
use pyo3::prelude::*;
use tree_sitter::Parser;

/// REPL executor (VM-based)
pub struct ReplExecutor {
    parser: Parser,
    semantic: SemanticAnalyzer,
    vm: VM,
    compiler: Compiler,
    context: Option<Py<PyAny>>,
    /// Names of initial globals (builtins) to distinguish them from user variables
    initial_globals: std::collections::HashSet<String>,
}

impl ReplExecutor {
    /// Create new executor with full pipeline + VM
    pub fn new() -> Result<Self, String> {
        let language = get_tree_sitter_language();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| format!("Failed to set language: {}", e))?;

        let semantic = SemanticAnalyzer::new();
        let vm = VM::new();
        let compiler = Compiler::new();

        Ok(Self {
            parser,
            semantic,
            vm,
            compiler,
            context: None,
            initial_globals: std::collections::HashSet::new(),
        })
    }

    /// Ensure Python context is initialized
    fn ensure_context(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.context.is_none() {
            let locals = pyo3::types::PyDict::new(py);
            let context_module = py.import("catnip.context")?;
            let context_class = context_module.getattr("Context")?;
            let context = context_class.call0()?;
            context.setattr("locals", locals)?;

            // Create and attach Registry for pattern matching
            let registry_module = py.import("catnip._rs")?;
            let registry_class = registry_module.getattr("Registry")?;
            let registry = registry_class.call1((context.clone(),))?;
            context.setattr("_registry", registry)?;

            // Snapshot initial globals (builtins) avant tout code utilisateur
            if let Ok(globals) = context.getattr("globals") {
                if let Ok(keys_obj) = globals.call_method0("keys") {
                    if let Ok(iter) = keys_obj.try_iter() {
                        for key_result in iter {
                            if let Ok(key_obj) = key_result {
                                if let Ok(s) = key_obj.extract::<String>() {
                                    self.initial_globals.insert(s);
                                }
                            }
                        }
                    }
                }
            }

            self.context = Some(context.unbind());
        }
        Ok(())
    }

    /// Get user-defined variable names (exclut builtins et noms internes)
    pub fn get_variable_names(&self) -> Vec<String> {
        Python::attach(|py| {
            if let Some(ctx) = &self.context {
                let bound_ctx = ctx.bind(py);

                // Try globals first (VM stores variables there)
                if let Ok(globals) = bound_ctx.getattr("globals") {
                    if let Ok(keys_obj) = globals.call_method0("keys") {
                        let mut names = Vec::new();
                        if let Ok(iter) = keys_obj.try_iter() {
                            for key_result in iter {
                                if let Ok(key_obj) = key_result {
                                    if let Ok(s) = key_obj.extract::<String>() {
                                        // Exclure builtins et noms internes (_)
                                        if !self.initial_globals.contains(&s) && !s.starts_with('_')
                                        {
                                            names.push(s);
                                        }
                                    }
                                }
                            }
                        }
                        names.sort();
                        return names;
                    }
                }

                // Fall back to locals if globals doesn't work
                if let Ok(locals) = bound_ctx.getattr("locals") {
                    if let Ok(keys_obj) = locals.call_method0("keys") {
                        let mut names = Vec::new();
                        if let Ok(iter) = keys_obj.try_iter() {
                            for key_result in iter {
                                if let Ok(key_obj) = key_result {
                                    if let Ok(s) = key_obj.extract::<String>() {
                                        if !self.initial_globals.contains(&s) && !s.starts_with('_')
                                        {
                                            names.push(s);
                                        }
                                    }
                                }
                            }
                        }
                        names.sort();
                        return names;
                    }
                }
            }
            Vec::new()
        })
    }

    /// Execute code through full pipeline (Parser → Semantic → VM) and return result
    /// Returns a string representation of the result for display
    pub fn execute(&mut self, code: &str) -> Result<(String, ValueKind), String> {
        // 1. Parse with Tree-sitter
        let tree = self
            .parser
            .parse(code, None)
            .ok_or_else(|| "Parse failed".to_string())?;

        let root = tree.root_node();

        // Check for syntax errors
        if let Some(error) = find_syntax_error(&root, code) {
            return Err(error);
        }

        // 2. Transform to IRPure
        let ir = transform_pure(root, code)?;

        // 3. Semantic analysis (validation + optimizations)
        let optimized_ir = self.semantic.analyze(&ir)?;

        // 4. Compile to bytecode and execute with VM
        Python::attach(|py| {
            // Ensure context exists and reuse it
            self.ensure_context(py)
                .map_err(|e| format!("Failed to initialize context: {}", e))?;

            // Set context in VM
            self.vm
                .set_context(self.context.as_ref().unwrap().clone_ref(py));

            // Handle List of statements
            let result = match &optimized_ir {
                ir::IRPure::List(statements) if !statements.is_empty() => {
                    let mut last_result = catnip_rs::vm::Value::NIL;

                    for stmt in statements {
                        // Skip None/empty statements
                        if matches!(stmt, ir::IRPure::None) {
                            continue;
                        }

                        // Convert IRPure → Op
                        let op = convert::irpure_to_op(py, stmt)
                            .map_err(|e| format!("Conversion error: {}", e))?;

                        // Compile Op → Bytecode
                        let code_object = self
                            .compiler
                            .compile(py, op.bind(py))
                            .map_err(|e| format!("Compilation error: {}", e))?;

                        // Execute bytecode
                        let args: Vec<catnip_rs::vm::Value> = Vec::new();
                        last_result = self
                            .vm
                            .execute(py, std::sync::Arc::new(code_object), &args)
                            .map_err(|e| format!("VM execution error: {:?}", e))?;
                    }

                    last_result
                }
                _ => {
                    // Single statement
                    let op = convert::irpure_to_op(py, &optimized_ir)
                        .map_err(|e| format!("Conversion error: {}", e))?;

                    let code_object = self
                        .compiler
                        .compile(py, op.bind(py))
                        .map_err(|e| format!("Compilation error: {}", e))?;

                    let args: Vec<catnip_rs::vm::Value> = Vec::new();
                    self.vm
                        .execute(py, std::sync::Arc::new(code_object), &args)
                        .map_err(|e| format!("VM execution error: {:?}", e))?
                }
            };

            // Store result in _ variable (like Python REPL)
            store_last_result(py, self.context.as_ref().unwrap(), result);

            // Convert VM Value to display string
            vm_value_to_display_string(py, result)
        })
    }

    /// Check if last result was None (to skip display)
    pub fn last_result_is_none(&self) -> bool {
        Python::attach(|py| {
            if let Some(ctx) = &self.context {
                let bound_ctx = ctx.bind(py);
                // Check globals first (VM stores variables there)
                if let Ok(globals) = bound_ctx.getattr("globals") {
                    if let Ok(has_key) = globals.call_method1("__contains__", ("_",)) {
                        if let Ok(has) = has_key.extract::<bool>() {
                            if !has {
                                return true;
                            }
                            // Check if _ is None
                            if let Ok(value) = globals.call_method1("get", ("_",)) {
                                return value.is_none();
                            }
                        }
                    }
                }
            }
            true
        })
    }

    /// Debug pipeline: show IR and bytecode without executing
    pub fn debug_pipeline(&mut self, code: &str) -> Result<String, String> {
        // 1. Parse with Tree-sitter
        let tree = self
            .parser
            .parse(code, None)
            .ok_or_else(|| "Parse failed".to_string())?;

        let root = tree.root_node();

        // Check for syntax errors
        if let Some(error) = find_syntax_error(&root, code) {
            return Err(error);
        }

        // 2. Transform to IRPure
        let ir = transform_pure(root, code)?;

        // 3. Semantic analysis (validation + optimizations)
        let optimized_ir = self.semantic.analyze(&ir)?;

        // 4. Format output with IR and bytecode
        let mut output = String::new();
        output.push_str("=== IR (after semantic analysis) ===\n");
        output.push_str(&format!("{:#?}\n\n", optimized_ir));

        // 5. Compile to bytecode (but don't execute)
        Python::attach(|py| {
            // Ensure context exists
            self.ensure_context(py)
                .map_err(|e| format!("Failed to initialize context: {}", e))?;

            // Handle List of statements
            match &optimized_ir {
                ir::IRPure::List(statements) if !statements.is_empty() => {
                    output.push_str("=== Bytecode ===\n");
                    for (i, stmt) in statements.iter().enumerate() {
                        if matches!(stmt, ir::IRPure::None) {
                            continue;
                        }

                        output.push_str(&format!("Statement #{}\n", i + 1));

                        // Convert IRPure → Op
                        let op = convert::irpure_to_op(py, stmt)
                            .map_err(|e| format!("Conversion error: {}", e))?;

                        // Compile Op → Bytecode
                        let code_object = self
                            .compiler
                            .compile(py, op.bind(py))
                            .map_err(|e| format!("Compilation error: {}", e))?;

                        // Format bytecode with instructions
                        output.push_str(&format!("Instructions:\n"));
                        for (i, instr) in code_object.instructions.iter().enumerate() {
                            output.push_str(&format!("  {:3}: {:?}\n", i, instr));
                        }
                        output.push_str(&format!(
                            "\nConstants: {} values\n",
                            code_object.constants.len()
                        ));
                        output.push_str(&format!("Names: {:?}\n\n", code_object.names));
                    }
                }
                _ => {
                    output.push_str("=== Bytecode ===\n");
                    // Single statement
                    let op = convert::irpure_to_op(py, &optimized_ir)
                        .map_err(|e| format!("Conversion error: {}", e))?;

                    let code_object = self
                        .compiler
                        .compile(py, op.bind(py))
                        .map_err(|e| format!("Compilation error: {}", e))?;

                    // Format bytecode with instructions
                    output.push_str(&format!("Instructions:\n"));
                    for (i, instr) in code_object.instructions.iter().enumerate() {
                        output.push_str(&format!("  {:3}: {:?}\n", i, instr));
                    }
                    output.push_str(&format!(
                        "\nConstants: {} values\n",
                        code_object.constants.len()
                    ));
                    output.push_str(&format!("Names: {:?}\n", code_object.names));
                }
            }

            Ok(output)
        })
    }
}

/// Store result in _ variable (like Python REPL)
fn store_last_result(py: Python, context: &Py<PyAny>, vm_value: catnip_rs::vm::Value) {
    use pyo3::types::{PyBool, PyFloat, PyInt};

    let bound_ctx = context.bind(py);

    // Convert VM Value to PyObject
    let pyobj: Option<Py<PyAny>> = if let Some(ptr) = unsafe { vm_value.as_pyobj_ptr() } {
        if !ptr.is_null() {
            unsafe { Some(pyo3::Bound::from_borrowed_ptr(py, ptr).unbind()) }
        } else {
            None
        }
    } else if let Some(n) = vm_value.as_int() {
        Some(PyInt::new(py, n).into())
    } else if let Some(f) = vm_value.as_float() {
        Some(PyFloat::new(py, f).into())
    } else if let Some(b) = vm_value.as_bool() {
        Some(PyBool::new(py, b).to_owned().into())
    } else if vm_value.is_nil() {
        Some(py.None())
    } else {
        None
    };

    // Store in globals under _
    if let Some(obj) = pyobj {
        if let Ok(globals) = bound_ctx.getattr("globals") {
            let _ = globals.call_method1("__setitem__", ("_", obj));
        }
    }
}

/// Kind tag returned alongside the display string
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    None,
    Int,
    Float,
    Bool,
    String,
    Object,
}

/// Convert VM Value to display string + kind tag
fn vm_value_to_display_string(
    py: Python,
    vm_value: catnip_rs::vm::Value,
) -> Result<(String, ValueKind), String> {
    // Try each type in order (NaN-boxed value)
    if vm_value.is_nil() {
        return Ok((String::new(), ValueKind::None));
    }

    if let Some(n) = vm_value.as_int() {
        return Ok((n.to_string(), ValueKind::Int));
    }

    if let Some(f) = vm_value.as_float() {
        // Format floats nicely
        let s = if f.fract() == 0.0 && f.abs() < 1e10 {
            format!("{:.1}", f)
        } else {
            f.to_string()
        };
        return Ok((s, ValueKind::Float));
    }

    if let Some(b) = vm_value.as_bool() {
        return Ok((
            if b { "True" } else { "False" }.to_string(),
            ValueKind::Bool,
        ));
    }

    if vm_value.is_pyobj() {
        // Get string representation from PyObject
        if let Some(ptr) = unsafe { vm_value.as_pyobj_ptr() } {
            if !ptr.is_null() {
                unsafe {
                    let bound = pyo3::Bound::from_borrowed_ptr(py, ptr);

                    // Detect Python strings for distinct coloring
                    let kind = if bound.is_instance_of::<pyo3::types::PyString>() {
                        ValueKind::String
                    } else {
                        ValueKind::Object
                    };

                    // Decimal → display with d suffix
                    if let Ok(decimal_cls) = py.import("decimal").and_then(|m| m.getattr("Decimal"))
                    {
                        if bound.is_instance(&decimal_cls).unwrap_or(false) {
                            let s = bound
                                .str()
                                .map_err(|e| format!("Failed to get str: {}", e))?;
                            return Ok((format!("{}d", s), ValueKind::Float));
                        }
                    }

                    let repr = bound
                        .repr()
                        .map_err(|e| format!("Failed to get repr: {}", e))?;
                    return Ok((repr.to_string(), kind));
                }
            }
        }
    }

    // Fallback
    Ok(("None".to_string(), ValueKind::None))
}

/// Find syntax error in tree
fn find_syntax_error(node: &tree_sitter::Node, source: &str) -> Option<String> {
    if node.is_error() {
        return Some(format_syntax_error_with_context(
            node,
            source,
            "Syntax error",
        ));
    }

    if node.is_missing() {
        return Some(format_syntax_error_with_context(
            node,
            source,
            &format!("Missing '{}'", node.kind()),
        ));
    }

    // Check children recursively
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(error) = find_syntax_error(&child, source) {
            return Some(error);
        }
    }

    None
}

/// Format syntax error with context (shows line and column with visual pointer)
fn format_syntax_error_with_context(
    node: &tree_sitter::Node,
    source: &str,
    message: &str,
) -> String {
    let pos = node.start_position();
    let line_num = pos.row + 1;
    let col_num = pos.column + 1;

    let lines: Vec<&str> = source.lines().collect();

    let mut output = format!("{} at line {}, column {}\n", message, line_num, col_num);

    // Show context: previous line, error line, next line
    let start_line = if pos.row > 0 { pos.row - 1 } else { pos.row };
    let end_line = (pos.row + 1).min(lines.len().saturating_sub(1));

    for i in start_line..=end_line {
        if i < lines.len() {
            output.push_str(&format!("  {:3} | {}\n", i + 1, lines[i]));

            // Add pointer on error line
            if i == pos.row {
                output.push_str(&format!("      | {}^\n", " ".repeat(pos.column)));
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_creation() {
        let executor = ReplExecutor::new();
        assert!(executor.is_ok());
    }

    #[test]
    fn test_execute_integer() {
        let mut executor = ReplExecutor::new().unwrap();

        let (result, _) = executor.execute("42").unwrap();
        assert_eq!(result, "42");
    }

    #[test]
    fn test_execute_arithmetic() {
        let mut executor = ReplExecutor::new().unwrap();

        let (result, _) = executor.execute("2 + 3").unwrap();
        assert_eq!(result, "5");
    }

    #[test]
    fn test_execute_assignment() {
        let mut executor = ReplExecutor::new().unwrap();

        executor.execute("x = 10").unwrap();
        let (result, _) = executor.execute("x * 2").unwrap();
        assert_eq!(result, "20");
    }

    #[test]
    fn test_get_variable_names() {
        let mut executor = ReplExecutor::new().unwrap();

        executor.execute("x = 10").unwrap();
        executor.execute("y = 20").unwrap();

        let names = executor.get_variable_names();
        assert!(names.contains(&"x".to_string()));
        assert!(names.contains(&"y".to_string()));
    }

    #[test]
    fn test_debug_ir() {
        let mut executor = ReplExecutor::new().unwrap();

        // Test arithmetic
        let tree = executor.parser.parse("2 + 3", None).unwrap();
        let ir = transform_pure(tree.root_node(), "2 + 3").unwrap();
        eprintln!("=== IR for '2 + 3' ===\n{:#?}\n", ir);

        // Test assignment
        let tree = executor.parser.parse("x = 10", None).unwrap();
        let ir = transform_pure(tree.root_node(), "x = 10").unwrap();
        eprintln!("=== IR for 'x = 10' ===\n{:#?}\n", ir);
    }
}
