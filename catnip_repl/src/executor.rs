// FILE: catnip_repl/src/executor.rs
//! Execution pipeline for REPL Phase 2b
//!
//! Pipeline: Source → Tree-sitter → IR → Semantic → Bytecode → VM
//!
//! Uses the VM with bytecode compilation.

use catnip_rs::constants::*;
use catnip_rs::get_tree_sitter_language;
use catnip_rs::ir;
use catnip_rs::ir::IROpCode;
use catnip_rs::parser::transform_pure;
use catnip_rs::pipeline::SemanticAnalyzer;
use catnip_rs::pragma::{Pragma, PragmaContext, PragmaType};
use catnip_rs::vm::VM;
use catnip_rs::vm::core::VMError;
use catnip_rs::vm::unified_compiler::UnifiedCompiler;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tree_sitter::Parser;

const PARSE_FAILED_MESSAGE: &str = "Parse failed";

/// REPL executor (VM-based)
pub struct ReplExecutor {
    parser: Parser,
    semantic: SemanticAnalyzer,
    vm: VM,
    context: Option<Py<PyAny>>,
    /// Pragma context, persists across evaluations
    pragma_context: Option<Py<PragmaContext>>,
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

        Ok(Self {
            parser,
            semantic,
            vm,
            context: None,
            pragma_context: None,
            initial_globals: std::collections::HashSet::new(),
        })
    }

    /// Ensure Python context is initialized with config defaults (auto-modules, policies)
    fn ensure_context(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.context.is_none() {
            let context_module = py.import(PY_MOD_CONTEXT)?;
            let context_class = context_module.getattr("Context")?;
            let context = context_class.call0()?;

            // Create and attach Registry for pattern matching
            let rs_module = py.import(PY_MOD_RS)?;
            let registry_class = rs_module.getattr("Registry")?;
            let registry = registry_class.call1((context.clone(),))?;
            context.setattr("_registry", registry)?;

            // Load config defaults (file + env) and apply to context
            let cm = rs_module.getattr("ConfigManager")?.call0()?;
            cm.call_method0("load_file")?;
            cm.call_method0("load_env")?;

            // Apply module policy
            let policy = cm.call_method0("get_module_policy")?;
            if !policy.is_none() {
                context.setattr("module_policy", policy)?;
            }

            // Load auto-import modules for repl mode
            let auto_modules: Vec<String> = cm.call_method1("get_auto_modules", ("repl",))?.extract()?;
            if !auto_modules.is_empty() {
                let loader_module = py.import(PY_MOD_LOADER)?;
                let loader_class = loader_module.getattr("ModuleLoader")?;
                let loader = loader_class.call1((context.clone(),))?;
                let modules_list = pyo3::types::PyList::new(py, &auto_modules)?;
                loader.call_method1("load_modules", (modules_list,))?;
            }

            // Snapshot initial globals (builtins + auto-modules) avant tout code utilisateur
            if let Ok(globals) = context.getattr("globals") {
                if let Ok(keys_obj) = globals.call_method0("keys") {
                    if let Ok(iter) = keys_obj.try_iter() {
                        for key_obj in iter.flatten() {
                            if let Ok(s) = key_obj.extract::<String>() {
                                self.initial_globals.insert(s);
                            }
                        }
                    }
                }
            }

            self.context = Some(context.unbind());
        }

        // Create PragmaContext if needed (persists across evaluations)
        if self.pragma_context.is_none() {
            self.pragma_context = Some(Py::new(py, PragmaContext::new(py))?);
        }

        Ok(())
    }

    /// Get the VM interrupt flag for external signal handlers.
    pub fn interrupt_flag(&self) -> Arc<AtomicBool> {
        self.vm.interrupt_flag()
    }

    /// Get attributes for each user variable via Python dir()
    pub fn get_variable_attrs(&self) -> std::collections::HashMap<String, Vec<String>> {
        Python::attach(|py| {
            let mut result = std::collections::HashMap::new();
            let Some(ctx) = &self.context else {
                return result;
            };
            let bound_ctx = ctx.bind(py);
            let Ok(globals) = bound_ctx.getattr("globals") else {
                return result;
            };

            let Ok(items) = globals.call_method0("items") else {
                return result;
            };
            let Ok(iter) = items.try_iter() else {
                return result;
            };

            let builtins = py.import("builtins").ok();

            for item_result in iter {
                let Ok(item) = item_result else { continue };
                let Ok((name, value)): Result<(String, pyo3::Bound<'_, PyAny>), _> = item.extract() else {
                    continue;
                };
                if self.initial_globals.contains(&name) || name.starts_with('_') {
                    continue;
                }

                // Skip primitive types -- no useful attrs to complete
                if let Some(ref b) = builtins {
                    let int_t = b.getattr("int").unwrap();
                    let float_t = b.getattr("float").unwrap();
                    let bool_t = b.getattr("bool").unwrap();
                    let is_prim = value.is_instance(&int_t).unwrap_or(false)
                        || value.is_instance(&float_t).unwrap_or(false)
                        || value.is_instance(&bool_t).unwrap_or(false);
                    if is_prim {
                        continue;
                    }
                }

                let Ok(dir_list) = value.dir() else { continue };
                let mut attrs = Vec::new();
                for attr_obj in dir_list.iter() {
                    let Ok(attr_name) = attr_obj.extract::<String>() else {
                        continue;
                    };
                    if !attr_name.starts_with('_') {
                        attrs.push(attr_name);
                    }
                }
                if !attrs.is_empty() {
                    result.insert(name, attrs);
                }
            }
            result
        })
    }

    /// Get user variables with type and truncated repr for `/context` display
    pub fn get_context_display(&self) -> Vec<(String, String, String)> {
        Python::attach(|py| {
            let Some(ctx) = &self.context else {
                return Vec::new();
            };
            let bound_ctx = ctx.bind(py);
            let Ok(globals) = bound_ctx.getattr("globals") else {
                return Vec::new();
            };
            let Ok(items) = globals.call_method0("items") else {
                return Vec::new();
            };
            let Ok(iter) = items.try_iter() else {
                return Vec::new();
            };

            let mut entries = Vec::new();
            for item_result in iter {
                let Ok(item) = item_result else { continue };
                let Ok((name, value)): Result<(String, pyo3::Bound<'_, PyAny>), _> = item.extract() else {
                    continue;
                };
                if self.initial_globals.contains(&name) || name.starts_with('_') {
                    continue;
                }
                let typ = value
                    .get_type()
                    .name()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|_| "?".to_string());
                let repr = value
                    .repr()
                    .map(|r| {
                        let s = r.to_string();
                        if s.len() > 60 { format!("{}...", &s[..57]) } else { s }
                    })
                    .unwrap_or_else(|_| "?".to_string());
                entries.push((name, typ, repr));
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            entries
        })
    }

    /// Get detail for a single variable (for `/context <var>`)
    pub fn get_variable_detail(&self, name: &str) -> Option<String> {
        Python::attach(|py| {
            let ctx = self.context.as_ref()?;
            let bound_ctx = ctx.bind(py);
            let globals = bound_ctx.getattr("globals").ok()?;
            let value = globals.call_method1("get", (name, py.None())).ok()?;
            if value.is_none() {
                return None;
            }
            let typ = value
                .get_type()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "?".to_string());
            let repr = value.repr().map(|r| r.to_string()).unwrap_or_else(|_| "?".to_string());
            Some(format!("{}: {} = {}", name, typ, repr))
        })
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
                            for key_obj in iter.flatten() {
                                if let Ok(s) = key_obj.extract::<String>() {
                                    // Exclure builtins et noms internes (_)
                                    if !self.initial_globals.contains(&s) && !s.starts_with('_') {
                                        names.push(s);
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
                            for key_obj in iter.flatten() {
                                if let Ok(s) = key_obj.extract::<String>() {
                                    if !self.initial_globals.contains(&s) && !s.starts_with('_') {
                                        names.push(s);
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

    /// Process a Pragma IR node: extract directive/value and apply to PragmaContext (Rust-native)
    fn process_pragma(&self, py: Python, ir_node: &ir::IR) -> Result<(), String> {
        let (args, kwargs) = match ir_node {
            ir::IR::Op {
                opcode: IROpCode::Pragma,
                args,
                kwargs,
                ..
            } => (args, kwargs),
            _ => return Ok(()),
        };

        if args.len() < 2 {
            return Err("pragma requires at least 2 arguments".into());
        }

        let directive = match &args[0] {
            ir::IR::String(s) => s.clone(),
            _ => return Err("pragma directive must be a string".into()),
        };

        let pragma_type = match PragmaType::from_directive(&directive.to_lowercase()) {
            Some(pt) => pt,
            None => {
                let known = PragmaType::all_directives().join(", ");
                return Err(format!("Unknown pragma directive: '{}'. Known: {}", directive, known));
            }
        };

        let value = ir_value_to_pyobject(py, &args[1]);

        let options = PyDict::new(py);
        for (key, val) in kwargs {
            options
                .set_item(key, ir_value_to_pyobject(py, val))
                .map_err(|e| format!("Failed to set pragma option: {}", e))?;
        }

        let pragma = Pragma::new(pragma_type, directive, value, options.unbind().into(), None);
        let pragma_py = Py::new(py, pragma).map_err(|e| format!("Failed to create Pragma: {}", e))?;

        let pragma_ctx = self.pragma_context.as_ref().unwrap();
        pragma_ctx
            .borrow_mut(py)
            .add(py, pragma_py)
            .map_err(|e| format!("{}", e.value(py)))?;

        Ok(())
    }

    /// Apply pragma effects to the execution context
    fn apply_pragma_effects(&self, py: Python) -> Result<(), String> {
        let Some(ref ctx) = self.context else {
            return Ok(());
        };
        let Some(ref pragma_ctx) = self.pragma_context else {
            return Ok(());
        };

        let pragma = pragma_ctx.borrow(py);
        let bound_ctx = ctx.bind(py);

        macro_rules! set_attr {
            ($name:expr, $val:expr) => {
                bound_ctx
                    .setattr($name, $val)
                    .map_err(|e| format!("Failed to set {}: {}", $name, e))?;
            };
        }

        set_attr!("tco_enabled", pragma.tco_enabled);
        set_attr!("jit_enabled", pragma.jit_enabled);
        set_attr!("jit_all", pragma.jit_all);
        set_attr!("nd_mode", &pragma.nd_mode);
        set_attr!("nd_workers", pragma.nd_workers);
        set_attr!("nd_memoize", pragma.nd_memoize);
        set_attr!("nd_batch_size", pragma.nd_batch_size);

        // Init JIT subsystem if enabled
        if pragma.jit_enabled {
            let _ = bound_ctx.call_method0("_init_jit");
        }

        Ok(())
    }

    /// Execute code through full pipeline (Parser → Semantic → VM) and return result
    /// Returns a string representation of the result for display
    pub fn execute(&mut self, code: &str) -> Result<(String, ValueKind), String> {
        // 1. Parse with Tree-sitter
        let tree = self
            .parser
            .parse(code, None)
            .ok_or_else(|| PARSE_FAILED_MESSAGE.to_string())?;

        let root = tree.root_node();

        // Check for syntax errors
        if let Some(error) = find_syntax_error(&root, code) {
            return Err(error);
        }

        // 2. Transform to IR
        let ir = transform_pure(root, code)?;

        // 3. Semantic analysis (validation + optimizations)
        let optimized_ir = self.semantic.analyze(&ir)?;

        // 4. Compile to bytecode and execute with VM
        Python::attach(|py| {
            // Ensure context exists and reuse it
            self.ensure_context(py)
                .map_err(|e| format!("Failed to initialize context: {}", e))?;

            // Set context in VM
            self.vm.set_context(self.context.as_ref().unwrap().clone_ref(py));

            // Handle List of statements
            let result = match &optimized_ir {
                ir::IR::List(statements) if !statements.is_empty() => {
                    let mut last_result = catnip_rs::vm::Value::NIL;
                    let mut pragmas_changed = false;

                    for stmt in statements {
                        if matches!(stmt, ir::IR::None) {
                            continue;
                        }

                        // Intercept pragma directives before compilation
                        if matches!(
                            stmt,
                            ir::IR::Op {
                                opcode: IROpCode::Pragma,
                                ..
                            }
                        ) {
                            self.process_pragma(py, stmt)?;
                            pragmas_changed = true;
                            continue;
                        }

                        let mut compiler = UnifiedCompiler::new();
                        let code_object = compiler
                            .compile_pure(py, stmt)
                            .map_err(|e| format!("Compilation error: {}", e))?;

                        // Execute bytecode
                        let args: Vec<catnip_rs::vm::Value> = Vec::new();
                        last_result = self
                            .vm
                            .execute(py, std::sync::Arc::new(code_object), &args)
                            .map_err(map_vm_error)?;
                    }

                    // Apply pragma effects after processing all statements
                    if pragmas_changed {
                        self.apply_pragma_effects(py)?;
                    }

                    last_result
                }
                other => {
                    // Single statement: check if it's a pragma
                    if matches!(
                        other,
                        ir::IR::Op {
                            opcode: IROpCode::Pragma,
                            ..
                        }
                    ) {
                        self.process_pragma(py, other)?;
                        self.apply_pragma_effects(py)?;
                        return Ok((String::new(), ValueKind::None));
                    }

                    let mut compiler = UnifiedCompiler::new();
                    let code_object = compiler
                        .compile_pure(py, &optimized_ir)
                        .map_err(|e| format!("Compilation error: {}", e))?;

                    let args: Vec<catnip_rs::vm::Value> = Vec::new();
                    self.vm
                        .execute(py, std::sync::Arc::new(code_object), &args)
                        .map_err(map_vm_error)?
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
            .ok_or_else(|| PARSE_FAILED_MESSAGE.to_string())?;

        let root = tree.root_node();

        // Check for syntax errors
        if let Some(error) = find_syntax_error(&root, code) {
            return Err(error);
        }

        // 2. Transform to IR
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
                ir::IR::List(statements) if !statements.is_empty() => {
                    output.push_str("=== Bytecode ===\n");
                    for (i, stmt) in statements.iter().enumerate() {
                        if matches!(stmt, ir::IR::None) {
                            continue;
                        }

                        output.push_str(&format!("Statement #{}\n", i + 1));

                        let mut compiler = UnifiedCompiler::new();
                        let code_object = compiler
                            .compile_pure(py, stmt)
                            .map_err(|e| format!("Compilation error: {}", e))?;

                        // Format bytecode with instructions
                        output.push_str("Instructions:\n");
                        for (i, instr) in code_object.instructions.iter().enumerate() {
                            output.push_str(&format!("  {:3}: {:?}\n", i, instr));
                        }
                        output.push_str(&format!("\nConstants: {} values\n", code_object.constants.len()));
                        output.push_str(&format!("Names: {:?}\n\n", code_object.names));
                    }
                }
                _ => {
                    output.push_str("=== Bytecode ===\n");
                    let mut compiler = UnifiedCompiler::new();
                    let code_object = compiler
                        .compile_pure(py, &optimized_ir)
                        .map_err(|e| format!("Compilation error: {}", e))?;

                    // Format bytecode with instructions
                    output.push_str("Instructions:\n");
                    for (i, instr) in code_object.instructions.iter().enumerate() {
                        output.push_str(&format!("  {:3}: {:?}\n", i, instr));
                    }
                    output.push_str(&format!("\nConstants: {} values\n", code_object.constants.len()));
                    output.push_str(&format!("Names: {:?}\n", code_object.names));
                }
            }

            Ok(output)
        })
    }
}

/// Convert simple IR literal to PyObject (for pragma args)
fn ir_value_to_pyobject(py: Python, ir: &ir::IR) -> Py<PyAny> {
    match ir {
        ir::IR::String(s) => s.into_pyobject(py).unwrap().into_any().unbind(),
        ir::IR::Bool(b) => b.into_pyobject(py).unwrap().to_owned().into_any().unbind(),
        ir::IR::Int(i) => i.into_pyobject(py).unwrap().into_any().unbind(),
        ir::IR::Float(f) => f.into_pyobject(py).unwrap().into_any().unbind(),
        ir::IR::None => py.None(),
        _ => py.None(),
    }
}

/// Store result in _ variable (like Python REPL)
fn store_last_result(py: Python, context: &Py<PyAny>, vm_value: catnip_rs::vm::Value) {
    use pyo3::types::{PyBool, PyFloat, PyInt};

    let bound_ctx = context.bind(py);

    // Convert VM Value to PyObject
    let pyobj: Option<Py<PyAny>> = if let Some(obj) = vm_value.as_pyobject(py) {
        Some(obj)
    } else if let Some(n) = vm_value.as_int() {
        Some(PyInt::new(py, n).into())
    } else if let Some(f) = vm_value.as_float() {
        Some(PyFloat::new(py, f).into())
    } else if let Some(b) = vm_value.as_bool() {
        Some(PyBool::new(py, b).to_owned().into())
    } else if vm_value.is_nil() {
        Some(py.None())
    } else if vm_value.is_bigint() {
        Some(vm_value.to_pyobject(py))
    } else {
        // Preserve any remaining VM-native values instead of dropping to None.
        Some(vm_value.to_pyobject(py))
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
fn vm_value_to_display_string(py: Python, vm_value: catnip_rs::vm::Value) -> Result<(String, ValueKind), String> {
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
        return Ok((if b { "True" } else { "False" }.to_string(), ValueKind::Bool));
    }

    if vm_value.is_bigint() {
        let py_obj = vm_value.to_pyobject(py);
        let s = py_obj
            .bind(py)
            .str()
            .map_err(|e| format!("Failed to format bigint: {}", e))?;
        return Ok((s.to_string(), ValueKind::Int));
    }

    if vm_value.is_struct_instance() {
        let py_obj = vm_value.to_pyobject(py);
        let repr = py_obj
            .bind(py)
            .repr()
            .map_err(|e| format!("Failed to get struct repr: {}", e))?;
        return Ok((repr.to_string(), ValueKind::Object));
    }

    if vm_value.is_pyobj() {
        let py_obj = vm_value.to_pyobject(py);
        let bound = py_obj.bind(py);

        // Detect Python strings for distinct coloring
        let kind = if bound.is_instance_of::<pyo3::types::PyString>() {
            ValueKind::String
        } else {
            ValueKind::Object
        };

        // Decimal -> display with d suffix
        if let Ok(decimal_cls) = py.import("decimal").and_then(|m| m.getattr("Decimal")) {
            if bound.is_instance(&decimal_cls).unwrap_or(false) {
                let s = bound.str().map_err(|e| format!("Failed to get str: {}", e))?;
                return Ok((format!("{}d", s), ValueKind::Float));
            }
        }

        let repr = bound.repr().map_err(|e| format!("Failed to get repr: {}", e))?;
        return Ok((repr.to_string(), kind));
    }

    // Fallback
    Ok(("None".to_string(), ValueKind::None))
}

/// Find syntax error in tree
fn find_syntax_error(node: &tree_sitter::Node, source: &str) -> Option<String> {
    if node.is_error() {
        return Some(format_syntax_error_with_context(node, source, "Syntax error"));
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
fn format_syntax_error_with_context(node: &tree_sitter::Node, source: &str, message: &str) -> String {
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

/// Convert VMError to string, with special format for Exit
fn map_vm_error(e: VMError) -> String {
    match e {
        VMError::Exit(code) => format!("exit({})", code),
        VMError::Interrupted => "KeyboardInterrupt".to_string(),
        other => format!("{other}"),
    }
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
    fn test_execute_bigint_display() {
        let mut executor = ReplExecutor::new().unwrap();

        let (result, kind) = executor.execute("2 ** 100").unwrap();
        assert_eq!(result, "1267650600228229401496703205376");
        assert_eq!(kind, ValueKind::Int);
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

    #[test]
    fn test_pragma_tco() {
        let mut executor = ReplExecutor::new().unwrap();

        // pragma should not error (Catnip uses True/False, not true/false)
        let result = executor.execute("pragma(\"tco\", True)");
        assert!(result.is_ok(), "pragma(\"tco\", True) failed: {:?}", result.err());

        // Should return empty (no visible result)
        let (text, kind) = result.unwrap();
        assert_eq!(kind, ValueKind::None);
        assert!(text.is_empty());
    }
}
