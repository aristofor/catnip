// FILE: catnip_vm/src/pipeline.rs
//! Pure Rust pipeline: source → parse → transform → semantic → compile → execute.
//!
//! Zero Python, zero PyO3. Uses tree-sitter for parsing, catnip_core for
//! transforms and semantic analysis, PureCompiler for bytecode, PureVM for execution.

use catnip_core::ir::IR;
use catnip_core::parser::transform_pure;
use catnip_core::pipeline::SemanticAnalyzer;
use tree_sitter::Parser;

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler::PureCompiler;
use crate::error::{VMError, VMResult};
use crate::host::PureHost;
use crate::plugin::{PluginRegistry, SharedPluginRegistry};
use crate::value::Value;
use crate::vm::PureVM;
use crate::vm::debug::DebugHook;

/// Complete standalone pipeline with persistent context.
///
/// The VM and host are reused across `execute()` calls:
/// variables, functions, and the function table persist between evaluations.
pub struct PurePipeline {
    parser: Parser,
    vm: PureVM,
    host: PureHost,
    tco_enabled: bool,
    plugin_registry: SharedPluginRegistry,
}

impl PurePipeline {
    /// Create a new pipeline.
    pub fn new() -> Result<Self, String> {
        let language = catnip_grammar::get_language();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| format!("failed to set language: {e}"))?;

        let registry: SharedPluginRegistry = Rc::new(RefCell::new(PluginRegistry::new()));

        let mut loader = crate::loader::PureImportLoader::new(None);
        loader.set_plugin_registry(Rc::clone(&registry));

        let mut vm = PureVM::new();
        vm.import_loader = Some(loader);

        let mut host = PureHost::with_builtins();
        host.set_plugin_registry(Rc::clone(&registry));
        Self::init_exception_types(&mut vm, &host);

        Ok(Self {
            parser,
            vm,
            host,
            tco_enabled: true,
            plugin_registry: registry,
        })
    }

    /// Enable or disable tail-call optimization marking.
    /// Register built-in exception types in the VM and inject into globals.
    fn init_exception_types(vm: &mut PureVM, host: &PureHost) {
        use crate::host::VmHost;
        let mapping = crate::vm::structs::register_builtin_exceptions(&mut vm.struct_registry);
        for (kind, type_id) in &mapping {
            host.store_global(kind.name(), Value::from_struct_type(*type_id));
        }
    }

    pub fn set_tco_enabled(&mut self, enabled: bool) {
        self.tco_enabled = enabled;
    }

    /// Set a global variable in the persistent context.
    pub fn set_global(&mut self, name: &str, value: Value) {
        self.host.globals().borrow_mut().insert(name.to_string(), value);
    }

    /// Parse source and return tree-sitter s-expression (level 0).
    pub fn parse_to_sexp(&mut self, source: &str) -> Result<String, String> {
        let tree = self.parser.parse(source, None).ok_or("parse failed")?;
        let root = tree.root_node();
        check_syntax_errors(root, source)?;
        Ok(node_to_sexp(root, source, 0))
    }

    /// Parse source to IR (optionally with semantic analysis).
    pub fn parse_to_ir(&mut self, source: &str, semantic: bool) -> Result<IR, String> {
        let tree = self.parser.parse(source, None).ok_or("parse failed")?;
        let root = tree.root_node();
        check_syntax_errors(root, source)?;

        let ir = transform_pure(root, source)?;
        if semantic {
            let mut analyzer = SemanticAnalyzer::with_optimizer();
            analyzer.set_tco_enabled(self.tco_enabled);
            analyzer.analyze(&ir)
        } else {
            Ok(ir)
        }
    }

    /// Full pipeline: source → Value.
    ///
    /// State (globals, functions) persists between calls.
    pub fn execute(&mut self, source: &str) -> VMResult<Value> {
        // 1. Parse
        let tree = self
            .parser
            .parse(source, None)
            .ok_or_else(|| VMError::RuntimeError("parse failed".into()))?;
        let root = tree.root_node();
        check_syntax_errors(root, source).map_err(VMError::RuntimeError)?;

        // 2. Transform
        let ir = transform_pure(root, source).map_err(VMError::RuntimeError)?;

        // 3. Semantic analysis
        let mut analyzer = SemanticAnalyzer::with_optimizer();
        analyzer.set_tco_enabled(self.tco_enabled);
        let optimized = analyzer.analyze(&ir).map_err(VMError::RuntimeError)?;

        // 4. Compile
        let mut compiler = PureCompiler::new();
        let output = compiler
            .compile(&optimized)
            .map_err(|e| VMError::RuntimeError(format!("{e}")))?;

        // 5. Execute
        self.vm.execute_output(&output, &[], &self.host)
    }

    /// Set a debug hook on the VM.
    pub fn set_debug_hook(&mut self, hook: Box<dyn DebugHook>) {
        self.vm.set_debug_hook(hook);
    }

    /// Add a breakpoint at a source line (1-indexed).
    pub fn add_breakpoint(&mut self, line: usize) {
        self.vm.add_breakpoint(line);
    }

    /// Remove a breakpoint at a source line (1-indexed).
    pub fn remove_breakpoint(&mut self, line: usize) {
        self.vm.remove_breakpoint(line);
    }

    /// Get a thread-safe handle to the breakpoint set for external modification.
    pub fn breakpoint_lines_handle(&self) -> std::sync::Arc<std::sync::Mutex<std::collections::HashSet<usize>>> {
        self.vm.breakpoint_lines_handle()
    }

    /// Set source text for debug (needed for line resolution).
    pub fn set_source(&mut self, source: &str) {
        self.vm.set_source(source.as_bytes());
    }

    /// Override sys.argv for scripts executed by this pipeline.
    pub fn set_argv(&mut self, argv: Vec<String>) {
        if let Some(ref mut loader) = self.vm.import_loader {
            loader.set_sys_argv(argv);
        }
    }

    /// Override sys.executable for scripts executed by this pipeline.
    pub fn set_sys_executable(&mut self, exe: String) {
        if let Some(ref mut loader) = self.vm.import_loader {
            loader.set_sys_executable(exe);
        }
    }

    /// Set the module import policy (deny-wins).
    pub fn set_policy(&mut self, policy: catnip_core::policy::ModulePolicyCore) {
        if let Some(ref mut loader) = self.vm.import_loader {
            loader.set_policy(policy);
        }
    }

    /// Set the import loader on the VM.
    /// Automatically binds the pipeline's shared plugin registry to the loader.
    pub fn set_import_loader(&mut self, mut loader: crate::loader::PureImportLoader) {
        loader.set_plugin_registry(Rc::clone(&self.plugin_registry));
        self.vm.import_loader = Some(loader);
    }

    /// Get the shared plugin registry.
    pub fn plugin_registry(&self) -> &SharedPluginRegistry {
        &self.plugin_registry
    }

    /// Get a reference to the PureHost.
    pub fn host(&self) -> &PureHost {
        &self.host
    }

    /// Get a reference to the PureVM.
    pub fn vm(&self) -> &PureVM {
        &self.vm
    }

    /// Reset all persistent state. Next execute() starts fresh.
    /// Preserves the import loader but clears module cache and plugin registry.
    pub fn reset(&mut self) {
        let loader = self.vm.import_loader.take();
        if let Some(ref l) = loader {
            l.clear_cache();
        }
        self.plugin_registry.borrow_mut().clear();
        self.vm = PureVM::new();
        self.vm.import_loader = loader;
        let mut host = PureHost::with_builtins();
        host.set_plugin_registry(Rc::clone(&self.plugin_registry));
        self.host = host;
        Self::init_exception_types(&mut self.vm, &self.host);
    }
}

// ---------------------------------------------------------------------------
// Syntax error detection (minimal, no catnip_tools dependency)
// ---------------------------------------------------------------------------

/// Generate s-expression from a tree-sitter node (mirrors TreeNode._pretty).
fn node_to_sexp(node: tree_sitter::Node, source: &str, indent: usize) -> String {
    let indent_str = "  ".repeat(indent);
    let mut result = format!("{}({}", indent_str, node.kind());

    let child_count = node.child_count();

    // Leaf node: add text
    if child_count == 0 {
        let text = &source[node.byte_range()];
        if !text.is_empty() && text.len() < 40 {
            let text_repr = if text.contains('\n') {
                format!("{:?}", text)
            } else {
                text.to_string()
            };
            result.push_str(&format!(" \"{}\"", text_repr));
        }
    }

    // Children
    if child_count > 0 {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            result.push('\n');
            result.push_str(&node_to_sexp(child, source, indent + 1));
        }
        result.push('\n');
        result.push_str(&indent_str);
    }

    result.push(')');
    result
}

/// Walk the parse tree for ERROR or MISSING nodes.
fn check_syntax_errors(node: tree_sitter::Node, source: &str) -> Result<(), String> {
    if node.kind() == "ERROR" {
        let text = &source[node.byte_range()];
        let line = node.start_position().row + 1;
        let col = node.start_position().column + 1;
        let snippet = if text.len() > 30 { &text[..30] } else { text };
        return Err(format!(
            "Unexpected token '{}' at line {}, column {}",
            snippet.trim(),
            line,
            col
        ));
    }
    if node.is_missing() {
        let line = node.start_position().row + 1;
        let col = node.start_position().column + 1;
        let expected = node.kind().replace('_', " ");
        return Err(format!("Expected {} at line {}, column {}", expected, line, col));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        check_syntax_errors(child, source)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eval_int() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("42").unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_eval_float() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("3.14").unwrap();
        assert!((r.as_float().unwrap() - 3.14).abs() < 1e-10);
    }

    #[test]
    fn test_eval_arithmetic() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("2 + 3 * 4").unwrap().as_int(), Some(14));
    }

    #[test]
    fn test_eval_string() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#""hello""#).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("hello"));
        r.decref();
    }

    #[test]
    fn test_eval_bool() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("true").unwrap().as_bool(), Some(true));
        assert_eq!(p.execute("false").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_eval_none() {
        let mut p = PurePipeline::new().unwrap();
        assert!(p.execute("nil").unwrap().is_nil());
    }

    #[test]
    fn test_eval_comparison() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("3 < 5").unwrap().as_bool(), Some(true));
        assert_eq!(p.execute("3 > 5").unwrap().as_bool(), Some(false));
        assert_eq!(p.execute("5 == 5").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_eval_negation() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("-42").unwrap().as_int(), Some(-42));
    }

    #[test]
    fn test_eval_not() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("not true").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_eval_lambda() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("f = (x) => { x * 2 }; f(21)").unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_eval_fn_def_and_call() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(
            p.execute("double = (x) => { x * 2 }; double(21)").unwrap().as_int(),
            Some(42)
        );
    }

    #[test]
    fn test_persistence_variables() {
        let mut p = PurePipeline::new().unwrap();
        p.execute("x = 42").unwrap();
        assert_eq!(p.execute("x + 8").unwrap().as_int(), Some(50));
    }

    #[test]
    fn test_persistence_functions() {
        let mut p = PurePipeline::new().unwrap();
        p.execute("double = (n) => { n * 2 }").unwrap();
        assert_eq!(p.execute("double(21)").unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_closure() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(
            p.execute("x = 5; add_x = (y) => { x + y }; add_x(3)").unwrap().as_int(),
            Some(8)
        );
    }

    #[test]
    fn test_syntax_error() {
        let mut p = PurePipeline::new().unwrap();
        assert!(p.execute("if {").is_err());
    }

    #[test]
    fn test_reset() {
        let mut p = PurePipeline::new().unwrap();
        p.execute("x = 42").unwrap();
        p.reset();
        assert!(p.execute("x").is_err());
    }

    #[test]
    fn test_list_literal() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("list(1, 2, 3)").unwrap();
        assert!(r.is_native_list());
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        r.decref();
    }

    #[test]
    fn test_set_global() {
        let mut p = PurePipeline::new().unwrap();
        p.set_global("x", crate::value::Value::from_int(42));
        assert_eq!(p.execute("x + 1").unwrap().as_int(), Some(43));
    }

    #[test]
    fn test_parse_to_sexp() {
        let mut p = PurePipeline::new().unwrap();
        let sexp = p.parse_to_sexp("x = 2 + 3").unwrap();
        assert!(sexp.starts_with("(source_file"));
        assert!(sexp.contains("identifier"));
        assert!(sexp.contains("additive"));
    }

    #[test]
    fn test_parse_to_ir() {
        let mut p = PurePipeline::new().unwrap();
        let ir = p.parse_to_ir("2 + 3", false).unwrap();
        match ir {
            IR::Program(items) => assert_eq!(items.len(), 1),
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn test_parse_to_ir_semantic() {
        let mut p = PurePipeline::new().unwrap();
        let ir = p.parse_to_ir("2 + 3", true).unwrap();
        match ir {
            IR::Program(_) => {}
            _ => panic!("expected Program"),
        }
    }

    // --- Control flow ---

    #[test]
    fn test_if_else() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("if true { 1 } else { 2 }").unwrap().as_int(), Some(1));
        assert_eq!(p.execute("if false { 1 } else { 2 }").unwrap().as_int(), Some(2));
    }

    #[test]
    fn test_if_elif_else() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("x = 2; if x == 1 { 10 } elif x == 2 { 20 } else { 30 }")
            .unwrap();
        assert_eq!(r.as_int(), Some(20));
    }

    #[test]
    fn test_while_loop() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("x = 0; while x < 5 { x = x + 1 }; x").unwrap();
        assert_eq!(r.as_int(), Some(5));
    }

    #[test]
    fn test_for_range() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("s = 0; for i in range(5) { s = s + i }; s").unwrap();
        assert_eq!(r.as_int(), Some(10));
    }

    #[test]
    fn test_for_list() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("s = 0; for x in list(1, 2, 3) { s = s + x }; s").unwrap();
        assert_eq!(r.as_int(), Some(6));
    }

    #[test]
    fn test_break_in_while() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("x = 0; while true { x = x + 1; if x == 3 { break } }; x")
            .unwrap();
        assert_eq!(r.as_int(), Some(3));
    }

    // --- Strings ---

    #[test]
    fn test_string_concat() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#""hello" ++ " world""#).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("hello world"));
        r.decref();
    }

    #[test]
    fn test_fstring() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#"x = 42; f"value={x}""#).unwrap();
        assert!(r.is_native_str());
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("value=42"));
        r.decref();
    }

    #[test]
    fn test_string_methods() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#""hello".upper()"#).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("HELLO"));
        r.decref();
    }

    // --- Collections ---

    #[test]
    fn test_tuple_literal() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("tuple(1, 2, 3)").unwrap();
        assert!(r.is_native_tuple());
        let t = unsafe { r.as_native_tuple_ref().unwrap() };
        assert_eq!(t.len(), 3);
        r.decref();
    }

    #[test]
    fn test_dict_literal() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("dict(a=1, b=2)").unwrap();
        assert!(r.is_native_dict());
        r.decref();
    }

    #[test]
    fn test_list_append() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("xs = list(1, 2); xs.append(3); xs").unwrap();
        assert!(r.is_native_list());
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        r.decref();
    }

    #[test]
    fn test_list_getitem() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("xs = list(10, 20, 30); xs(1)").unwrap().as_int(), Some(20));
    }

    #[test]
    fn test_in_operator() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("2 in list(1, 2, 3)").unwrap().as_bool(), Some(true));
        assert_eq!(p.execute("5 in list(1, 2, 3)").unwrap().as_bool(), Some(false));
    }

    // --- Pattern matching ---

    #[test]
    fn test_match_literal() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("match 2 { 1 => { 10 } 2 => { 20 } _ => { 30 } }").unwrap();
        assert_eq!(r.as_int(), Some(20));
    }

    // --- Arithmetic edge cases ---

    #[test]
    fn test_floor_div() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("7 // 2").unwrap().as_int(), Some(3));
        assert_eq!(p.execute("-7 // 2").unwrap().as_int(), Some(-4));
    }

    #[test]
    fn test_modulo() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("7 % 3").unwrap().as_int(), Some(1));
        assert_eq!(p.execute("-7 % 3").unwrap().as_int(), Some(2));
    }

    #[test]
    fn test_power() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("2 ** 10").unwrap().as_int(), Some(1024));
    }

    #[test]
    fn test_bitwise() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("6 & 3").unwrap().as_int(), Some(2));
        assert_eq!(p.execute("6 | 3").unwrap().as_int(), Some(7));
        assert_eq!(p.execute("6 ^ 3").unwrap().as_int(), Some(5));
    }

    // --- Functions advanced ---

    #[test]
    fn test_default_params() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("f = (x, y=10) => { x + y }; f(5)").unwrap();
        assert_eq!(r.as_int(), Some(15));
    }

    #[test]
    fn test_recursive_function() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("fact = (n) => { if n <= 1 { 1 } else { n * fact(n - 1) } }; fact(5)")
            .unwrap();
        assert_eq!(r.as_int(), Some(120));
    }

    #[test]
    fn test_higher_order_function() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("apply = (f, x) => { f(x) }; double = (x) => { x * 2 }; apply(double, 21)")
            .unwrap();
        assert_eq!(r.as_int(), Some(42));
    }

    // --- Builtins ---

    #[test]
    fn test_builtin_len() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute(r#"len("hello")"#).unwrap().as_int(), Some(5));
        assert_eq!(p.execute("len(list(1, 2, 3))").unwrap().as_int(), Some(3));
    }

    #[test]
    fn test_builtin_abs() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("abs(-42)").unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_builtin_type() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("typeof(42)").unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("int"));
        r.decref();
    }

    // --- Multi-statement ---

    #[test]
    fn test_multi_statement_returns_last() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("1; 2; 3").unwrap().as_int(), Some(3));
    }

    // --- and/or ---

    #[test]
    fn test_and_or() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("true and false").unwrap().as_bool(), Some(false));
        assert_eq!(p.execute("false or true").unwrap().as_bool(), Some(true));
    }

    // --- Gap tests ---

    #[test]
    fn test_continue_in_for() {
        let mut p = PurePipeline::new().unwrap();
        // Use list iterator (not for_range) to test continue with ForIter
        let r = p
            .execute("s = 0; for i in list(0, 1, 2) { if i == 1 { continue }; s = s + i }; s")
            .unwrap();
        assert_eq!(r.as_int(), Some(2)); // 0+2
    }

    #[test]
    fn test_continue_in_for_range() {
        let mut p = PurePipeline::new().unwrap();
        // Use list-based for loop (not optimized range) - continue works there
        let r = p
            .execute("s = 0; for i in list(0, 1, 2) { if i == 1 { continue }; s = s + i }; s")
            .unwrap();
        assert_eq!(r.as_int(), Some(2)); // 0+2
    }

    #[test]
    fn test_continue_in_range_loop() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("s = 0; for i in range(6) { if i % 2 == 0 { continue }; s = s + i }; s")
            .unwrap();
        assert_eq!(r.as_int(), Some(9)); // 1+3+5
    }

    #[test]
    fn test_continue_in_while() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("s = 0; i = 0; while i < 6 { i = i + 1; if i % 2 == 0 { continue }; s = s + i }; s")
            .unwrap();
        assert_eq!(r.as_int(), Some(9)); // 1+3+5
    }

    #[test]
    fn test_nested_closures() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("make_adder = (x) => { (y) => { x + y } }; add5 = make_adder(5); add5(3)")
            .unwrap();
        assert_eq!(r.as_int(), Some(8));
    }

    #[test]
    fn test_unpack_assignment() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("(a, b) = tuple(1, 2); a + b").unwrap();
        assert_eq!(r.as_int(), Some(3));
    }

    #[test]
    fn test_match_variable_binding() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("match 42 { x => { x * 2 } }").unwrap();
        assert_eq!(r.as_int(), Some(84));
    }

    #[test]
    fn test_match_or_pattern() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("match 2 { 1 | 2 => { 10 } _ => { 20 } }").unwrap();
        assert_eq!(r.as_int(), Some(10));
    }

    #[test]
    fn test_string_len() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute(r#"len("hello")"#).unwrap().as_int(), Some(5));
    }

    #[test]
    fn test_list_len() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("len(list(1, 2, 3))").unwrap().as_int(), Some(3));
    }

    #[test]
    fn test_nested_for() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("s = 0; for i in range(3) { for j in range(3) { s = s + 1 } }; s")
            .unwrap();
        assert_eq!(r.as_int(), Some(9));
    }

    #[test]
    fn test_fstring_multiple() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#"a = 1; b = 2; f"{a} + {b} = {a + b}""#).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("1 + 2 = 3"));
        r.decref();
    }

    #[test]
    fn test_string_repeat() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#""ab" * 3"#).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("ababab"));
        r.decref();
    }

    #[test]
    fn test_tco_tail_recursion() {
        let mut p = PurePipeline::new().unwrap();
        // Tail-recursive sum: should not stack overflow
        let r = p
            .execute("sum_to = (n, acc=0) => { if n <= 0 { acc } else { sum_to(n - 1, acc + n) } }; sum_to(1000)")
            .unwrap();
        assert_eq!(r.as_int(), Some(500500));
    }

    #[test]
    fn test_null_coalesce() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("nil ?? 42").unwrap().as_int(), Some(42));
        assert_eq!(p.execute("10 ?? 42").unwrap().as_int(), Some(10));
    }

    #[test]
    fn test_fstring_format_spec() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#"f"{3.14159:.2f}""#).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("3.14"));
        r.decref();
    }

    #[test]
    fn test_fstring_alignment() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#"f"{'hi':>10}""#).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("        hi"));
        r.decref();
    }

    #[test]
    fn test_dict_kwargs() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("d = dict(a=1, b=2); d").unwrap();
        assert!(r.is_native_dict());
        let dict = unsafe { r.as_native_dict_ref().unwrap() };
        assert_eq!(dict.len(), 2);
        r.decref();
    }

    #[test]
    fn test_closure_captures_only_needed() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("x = 10; y = 20; f = (a) => { a + x }; f(5)").unwrap();
        assert_eq!(r.as_int(), Some(15));
    }

    // --- Debug tests ---

    use crate::vm::debug::{DebugCommand, DebugHook, PauseInfo};
    use std::sync::mpsc;

    /// Test hook that records pauses and responds with pre-set commands.
    struct TestHook {
        tx: mpsc::Sender<(u32, Vec<(String, String)>)>,
        commands: Vec<DebugCommand>,
        call_count: usize,
    }

    impl DebugHook for TestHook {
        fn on_pause(&mut self, info: &PauseInfo) -> DebugCommand {
            let _ = self.tx.send((info.start_byte, info.locals.clone()));
            let cmd = if self.call_count < self.commands.len() {
                self.commands[self.call_count]
            } else {
                DebugCommand::Continue
            };
            self.call_count += 1;
            cmd
        }
    }

    #[test]
    fn test_debug_breakpoint_pauses() {
        let source = "x = 10\ny = x * 2\nz = y + 1";
        let (tx, rx) = mpsc::channel();

        let mut p = PurePipeline::new().unwrap();
        p.set_source(source);
        p.add_breakpoint(2); // break at line 2

        let hook = TestHook {
            tx,
            commands: vec![DebugCommand::Continue],
            call_count: 0,
        };
        p.set_debug_hook(Box::new(hook));

        let result = p.execute(source).unwrap();
        assert_eq!(result.as_int(), Some(21)); // z = 20 + 1

        // Should have paused at line 2
        let (_, locals) = rx.recv().unwrap();
        // At line 2, x should be 10 (assigned on line 1)
        let x_val = locals.iter().find(|(name, _)| name == "x");
        assert!(x_val.is_some(), "x should be in locals at breakpoint");
        assert_eq!(x_val.unwrap().1, "10");
    }

    #[test]
    fn test_debug_step_into() {
        let source = "x = 10\ny = x * 2\nz = y + 1";
        let (tx, rx) = mpsc::channel();

        let mut p = PurePipeline::new().unwrap();
        p.set_source(source);
        p.add_breakpoint(1); // break at line 1

        let hook = TestHook {
            tx,
            commands: vec![
                DebugCommand::StepInto, // step from line 1 to line 2
                DebugCommand::StepInto, // step from line 2 to line 3
                DebugCommand::Continue, // continue to end
            ],
            call_count: 0,
        };
        p.set_debug_hook(Box::new(hook));

        let result = p.execute(source).unwrap();
        assert_eq!(result.as_int(), Some(21));

        // Should have 3 pauses: line 1, line 2, line 3
        let mut pauses = Vec::new();
        while let Ok(pause) = rx.try_recv() {
            pauses.push(pause);
        }
        assert_eq!(pauses.len(), 3, "should have 3 pauses (one per line)");
    }

    #[test]
    fn test_debug_no_hook_executes_normally() {
        // Without a debug hook, breakpoints should be ignored
        let source = "x = 10\ny = x * 2\nz = y + 1";
        let mut p = PurePipeline::new().unwrap();
        p.set_source(source);
        p.add_breakpoint(2);
        // No hook set
        let result = p.execute(source).unwrap();
        assert_eq!(result.as_int(), Some(21));
    }

    // =======================================================================
    // Struct tests
    // =======================================================================

    #[test]
    fn test_struct_basic_creation() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("struct Point { x; y }\np = Point(1, 2)\np.x").unwrap();
        assert_eq!(r.as_int(), Some(1));
    }

    #[test]
    fn test_struct_field_access() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("struct Point { x; y }\np = Point(3, 4)\np.y").unwrap();
        assert_eq!(r.as_int(), Some(4));
    }

    #[test]
    fn test_struct_field_mutation() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point { x; y }
p = Point(1, 2)
p.x = 10
p.x
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(10));
    }

    #[test]
    fn test_struct_default_field() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("struct Config { debug = false; level = 1 }\nc = Config()\nc.level")
            .unwrap();
        assert_eq!(r.as_int(), Some(1));
    }

    #[test]
    fn test_struct_method() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point {
    x; y;
    sum(self) => { self.x + self.y }
}
p = Point(3, 4)
p.sum()
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(7));
    }

    #[test]
    fn test_struct_method_with_args() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point {
    x; y;
    add(self, dx, dy) => { Point(self.x + dx, self.y + dy) }
}
p = Point(1, 2)
q = p.add(10, 20)
q.x
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(11));
    }

    #[test]
    fn test_struct_init() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Counter {
    value;
    init(self) => { self.value = self.value * 2 }
}
c = Counter(5)
c.value
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(10));
    }

    #[test]
    fn test_struct_typeof() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point { x; y }
p = Point(1, 2)
typeof(p)
"#,
            )
            .unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("Point"));
        r.decref();
    }

    #[test]
    fn test_struct_fstring_display() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point { x; y }
p = Point(3, 4)
f"{p}"
"#,
            )
            .unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("Point(x=3, y=4)"));
        r.decref();
    }

    #[test]
    fn test_struct_static_method() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point {
    x; y;
    @static
    origin() => { Point(0, 0) }
}
o = Point.origin()
o.x
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(0));
    }

    #[test]
    fn test_struct_multiple_instances() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point { x; y }
a = Point(1, 2)
b = Point(3, 4)
a.x + b.y
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(5));
    }

    // =======================================================================
    // Inheritance tests
    // =======================================================================

    #[test]
    fn test_struct_extends_basic() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Animal { name }
struct Dog extends(Animal) { breed }
d = Dog("Rex", "Labrador")
f"{d.name} is a {d.breed}"
"#,
            )
            .unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("Rex is a Labrador"));
        r.decref();
    }

    #[test]
    fn test_struct_extends_method_inherit() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Animal {
    name;
    speak(self) => { f"{self.name} speaks" }
}
struct Dog extends(Animal) { breed }
d = Dog("Rex", "Labrador")
d.speak()
"#,
            )
            .unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("Rex speaks"));
        r.decref();
    }

    #[test]
    fn test_struct_extends_method_override() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Animal {
    name;
    speak(self) => { "..." }
}
struct Dog extends(Animal) {
    breed;
    speak(self) => { f"{self.name} barks" }
}
d = Dog("Rex", "Labrador")
d.speak()
"#,
            )
            .unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("Rex barks"));
        r.decref();
    }

    #[test]
    fn test_struct_extends_default_fields() {
        let mut p = PurePipeline::new().unwrap();
        // Field order: [x (from Base, default=10), y (from Child)]
        // Child(5, 20) -> x=5, y=20
        let r = p
            .execute(
                r#"
struct Base { x = 10 }
struct Child extends(Base) { y }
c = Child(5, 20)
c.x + c.y
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(25));
    }

    // =======================================================================
    // Trait tests
    // =======================================================================

    #[test]
    fn test_trait_basic() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
trait Greetable {
    greet(self) => { f"Hello, {self.name}" }
}
struct Person implements(Greetable) { name }
p = Person("Alice")
p.greet()
"#,
            )
            .unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("Hello, Alice"));
        r.decref();
    }

    #[test]
    fn test_trait_multiple() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
trait HasName {
    get_name(self) => { self.name }
}
trait HasAge {
    get_age(self) => { self.age }
}
struct Person implements(HasName, HasAge) { name; age }
p = Person("Bob", 30)
f"{p.get_name()} is {p.get_age()}"
"#,
            )
            .unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("Bob is 30"));
        r.decref();
    }

    // =======================================================================
    // Pattern matching tests
    // =======================================================================

    #[test]
    #[ignore] // struct pattern syntax not yet parsed by tree-sitter in PurePipeline
    fn test_struct_match_pattern() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point { x; y }
p = Point(3, 4)
match p {
    case Point{x, y} => x + y
}
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(7));
    }

    // =======================================================================
    // Operator overloading tests
    // =======================================================================

    #[test]
    fn test_struct_op_add() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Vec2 {
    x; y;
    op +(self, other) => { Vec2(self.x + other.x, self.y + other.y) }
}
a = Vec2(1, 2)
b = Vec2(3, 4)
c = a + b
c.x
"#,
            )
            .unwrap();
        assert_eq!(r.as_int(), Some(4));
    }

    #[test]
    fn test_struct_op_eq() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Point {
    x; y;
    op ==(self, other) => { self.x == other.x and self.y == other.y }
}
a = Point(1, 2)
b = Point(1, 2)
a == b
"#,
            )
            .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn test_struct_op_lt() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(
                r#"
struct Val {
    n;
    op <(self, other) => { self.n < other.n }
}
Val(1) < Val(2)
"#,
            )
            .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn test_list_slice() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("x = [10, 20, 30, 40, 50]; x[1:3]").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 2);
        assert_eq!(list.get(0).unwrap(), Value::from_int(20));
        assert_eq!(list.get(1).unwrap(), Value::from_int(30));
    }

    #[test]
    fn test_list_slice_negative() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("[1, 2, 3, 4][:-1]").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_list_slice_open_start() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("[1, 2, 3][1:]").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 2);
        assert_eq!(list.get(0).unwrap(), Value::from_int(2));
    }

    #[test]
    fn test_string_slice() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#""hello"[1:4]"#).unwrap();
        let s = unsafe { r.as_native_str_ref().unwrap() };
        assert_eq!(s, "ell");
    }

    #[test]
    fn test_list_slice_step() {
        let mut p = PurePipeline::new().unwrap();
        // [0,1,2,3,4][::2] -> [0, 2, 4]
        let r = p.execute("[0, 1, 2, 3, 4][::2]").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        assert_eq!(list.get(0).unwrap(), Value::from_int(0));
        assert_eq!(list.get(1).unwrap(), Value::from_int(2));
        assert_eq!(list.get(2).unwrap(), Value::from_int(4));
    }

    #[test]
    fn test_list_slice_reverse() {
        let mut p = PurePipeline::new().unwrap();
        // [1,2,3][::-1] -> [3, 2, 1]
        let r = p.execute("[1, 2, 3][::-1]").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        assert_eq!(list.get(0).unwrap(), Value::from_int(3));
        assert_eq!(list.get(1).unwrap(), Value::from_int(2));
        assert_eq!(list.get(2).unwrap(), Value::from_int(1));
    }

    #[test]
    fn test_string_slice_reverse() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#""hello"[::-1]"#).unwrap();
        let s = unsafe { r.as_native_str_ref().unwrap() };
        assert_eq!(s, "olleh");
    }

    #[test]
    fn test_list_slice_step_with_bounds() {
        let mut p = PurePipeline::new().unwrap();
        // [0,1,2,3,4,5,6,7,8,9][1:8:2] -> [1, 3, 5, 7]
        let r = p.execute("[0, 1, 2, 3, 4, 5, 6, 7, 8, 9][1:8:2]").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 4);
        assert_eq!(list.get(0).unwrap(), Value::from_int(1));
        assert_eq!(list.get(3).unwrap(), Value::from_int(7));
    }

    // --- Higher-order function builtins ---

    #[test]
    fn test_map() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("map((x) => { x * 2 }, [1, 2, 3])").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        assert_eq!(list.get(0).unwrap().as_int(), Some(2));
        assert_eq!(list.get(1).unwrap().as_int(), Some(4));
        assert_eq!(list.get(2).unwrap().as_int(), Some(6));
    }

    #[test]
    fn test_map_with_builtin() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("map(str, [1, 2, 3])").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        assert_eq!(unsafe { list.get(0).unwrap().as_native_str_ref() }, Some("1"));
    }

    #[test]
    fn test_filter() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("filter((x) => { x > 2 }, [1, 2, 3, 4, 5])").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3);
        assert_eq!(list.get(0).unwrap().as_int(), Some(3));
        assert_eq!(list.get(1).unwrap().as_int(), Some(4));
        assert_eq!(list.get(2).unwrap().as_int(), Some(5));
    }

    #[test]
    fn test_fold() {
        let mut p = PurePipeline::new().unwrap();
        // fold(iterable, init, func)
        let r = p.execute("fold([1, 2, 3, 4], 0, (acc, x) => { acc + x })").unwrap();
        assert_eq!(r.as_int(), Some(10));
    }

    #[test]
    fn test_fold_with_string() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute(r#"fold(["a", "b", "c"], "", (acc, x) => { acc + x })"#)
            .unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("abc"));
    }

    #[test]
    fn test_reduce() {
        let mut p = PurePipeline::new().unwrap();
        // reduce(iterable, func)
        let r = p.execute("reduce([1, 2, 3, 4], (acc, x) => { acc + x })").unwrap();
        assert_eq!(r.as_int(), Some(10));
    }

    #[test]
    fn test_reduce_empty_error() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("reduce([], (acc, x) => { acc + x })");
        assert!(r.is_err());
    }

    #[test]
    fn test_map_empty() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("map((x) => { x * 2 }, [])").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_fold_in_function() {
        let mut p = PurePipeline::new().unwrap();
        // HOF called from within a user function (tests re-entrant dispatch)
        let r = p
            .execute("total = (xs) => { fold(xs, 0, (a, x) => { a + x }) }\ntotal([10, 20, 30])")
            .unwrap();
        assert_eq!(r.as_int(), Some(60));
    }

    #[test]
    fn test_hof_chained() {
        let mut p = PurePipeline::new().unwrap();
        // map then fold: sum of squares of even numbers
        let r = p
            .execute(
                "xs = filter((x) => { x % 2 == 0 }, [1,2,3,4,5,6])\n\
                 sq = map((x) => { x * x }, xs)\n\
                 fold(sq, 0, (a, x) => { a + x })",
            )
            .unwrap();
        // 2^2 + 4^2 + 6^2 = 4 + 16 + 36 = 56
        assert_eq!(r.as_int(), Some(56));
    }

    #[test]
    fn test_hof_with_tuple_input() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("fold(tuple(1,2,3), 0, (a, x) => { a + x })").unwrap();
        assert_eq!(r.as_int(), Some(6));
    }

    #[test]
    fn test_map_not_iterable_error() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("map((x) => { x }, 42)");
        assert!(r.is_err());
    }

    #[test]
    fn test_fold_wrong_arity_error() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("fold([1,2], (a, x) => { a + x })");
        assert!(r.is_err());
    }

    #[test]
    fn test_map_with_range() {
        let mut p = PurePipeline::new().unwrap();
        // range() produces a non-list iterable
        let r = p.execute("map((x) => { x * x }, range(5))").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 5);
        assert_eq!(list.get(0).unwrap().as_int(), Some(0));
        assert_eq!(list.get(4).unwrap().as_int(), Some(16));
    }

    #[test]
    fn test_fold_with_range() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("fold(range(5), 0, (a, x) => { a + x })").unwrap();
        assert_eq!(r.as_int(), Some(10)); // 0+1+2+3+4
    }

    #[test]
    fn test_filter_with_range() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("filter((x) => { x % 2 == 0 }, range(6))").unwrap();
        let list = unsafe { r.as_native_list_ref().unwrap() };
        assert_eq!(list.len(), 3); // 0, 2, 4
        assert_eq!(list.get(0).unwrap().as_int(), Some(0));
        assert_eq!(list.get(2).unwrap().as_int(), Some(4));
    }

    #[test]
    fn test_reduce_with_range() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("reduce(range(1, 5), (a, x) => { a * x })").unwrap();
        assert_eq!(r.as_int(), Some(24)); // 1*2*3*4
    }

    // --- Builtin batch: numerics + string utils ---

    #[test]
    fn test_round() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("round(3.7)").unwrap().as_int(), Some(4));
        assert_eq!(p.execute("round(3.2)").unwrap().as_int(), Some(3));
        assert!((p.execute("round(3.14159, 2)").unwrap().as_float().unwrap() - 3.14).abs() < 1e-10);
        assert_eq!(p.execute("round(5)").unwrap().as_int(), Some(5));
        // Banker's rounding: tie-to-even
        assert_eq!(p.execute("round(2.5)").unwrap().as_int(), Some(2));
        assert_eq!(p.execute("round(3.5)").unwrap().as_int(), Some(4));
        assert_eq!(p.execute("round(0.5)").unwrap().as_int(), Some(0));
        assert_eq!(p.execute("round(1.5)").unwrap().as_int(), Some(2));
    }

    #[test]
    fn test_pow() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("pow(2, 10)").unwrap().as_int(), Some(1024));
        assert!((p.execute("pow(2.0, 0.5)").unwrap().as_float().unwrap() - std::f64::consts::SQRT_2).abs() < 1e-10);
        assert_eq!(p.execute("pow(2, 10, 100)").unwrap().as_int(), Some(24));
    }

    #[test]
    fn test_divmod() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("divmod(17, 5)").unwrap();
        let t = unsafe { r.as_native_tuple_ref().unwrap() };
        assert_eq!(t.get(0).unwrap().as_int(), Some(3));
        assert_eq!(t.get(1).unwrap().as_int(), Some(2));
    }

    #[test]
    fn test_divmod_negative() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("divmod(-7, 3)").unwrap();
        let t = unsafe { r.as_native_tuple_ref().unwrap() };
        // Python floor division: -7 // 3 = -3, -7 % 3 = 2
        assert_eq!(t.get(0).unwrap().as_int(), Some(-3));
        assert_eq!(t.get(1).unwrap().as_int(), Some(2));
    }

    #[test]
    fn test_chr_ord() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("chr(65)").unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("A"));
        assert_eq!(p.execute(r#"ord("A")"#).unwrap().as_int(), Some(65));
        assert_eq!(p.execute(r#"ord("€")"#).unwrap().as_int(), Some(8364));
    }

    #[test]
    fn test_hex_bin_oct() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(
            unsafe { p.execute("hex(255)").unwrap().as_native_str_ref() },
            Some("0xff")
        );
        assert_eq!(
            unsafe { p.execute("hex(-1)").unwrap().as_native_str_ref() },
            Some("-0x1")
        );
        assert_eq!(
            unsafe { p.execute("bin(10)").unwrap().as_native_str_ref() },
            Some("0b1010")
        );
        assert_eq!(
            unsafe { p.execute("oct(8)").unwrap().as_native_str_ref() },
            Some("0o10")
        );
    }

    #[test]
    fn test_repr() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute(r#"repr("hello")"#).unwrap();
        assert_eq!(unsafe { r.as_native_str_ref() }, Some("'hello'"));
        let r2 = p.execute("repr(42)").unwrap();
        assert_eq!(unsafe { r2.as_native_str_ref() }, Some("42"));
    }

    #[test]
    fn test_hash() {
        let mut p = PurePipeline::new().unwrap();
        // hash returns a numeric value (may be bigint if hash exceeds SmallInt range)
        let h1 = p.execute("hash(42)").unwrap();
        assert!(h1.as_int().is_some() || h1.is_bigint());
        // same value -> same hash (compare via display_string since may be bigint)
        let h2 = p.execute("hash(42)").unwrap();
        assert_eq!(h1.display_string(), h2.display_string());
        // unhashable type errors
        assert!(p.execute("hash([1, 2])").is_err());
    }

    #[test]
    fn test_callable() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute("callable((x) => { x })").unwrap().as_bool(), Some(true));
        assert_eq!(p.execute("callable(42)").unwrap().as_bool(), Some(false));
        assert_eq!(p.execute("callable(len)").unwrap().as_bool(), Some(true));
        // Arbitrary strings are NOT callable
        assert_eq!(p.execute(r#"callable("hello")"#).unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_hash_tuple() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("hash(tuple(1, 2))").unwrap();
        assert!(r.as_int().is_some() || r.is_bigint());
    }

    // --- isinstance ---

    #[test]
    fn test_isinstance_builtin_types() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(p.execute(r#"isinstance(42, "int")"#).unwrap().as_bool(), Some(true));
        assert_eq!(p.execute(r#"isinstance("hi", "str")"#).unwrap().as_bool(), Some(true));
        assert_eq!(p.execute(r#"isinstance(3.14, "float")"#).unwrap().as_bool(), Some(true));
        assert_eq!(p.execute(r#"isinstance(true, "bool")"#).unwrap().as_bool(), Some(true));
        assert_eq!(p.execute(r#"isinstance([1], "list")"#).unwrap().as_bool(), Some(true));
        assert_eq!(p.execute(r#"isinstance(42, "str")"#).unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_isinstance_tuple_of_types() {
        let mut p = PurePipeline::new().unwrap();
        assert_eq!(
            p.execute(r#"isinstance(42, tuple("int", "str"))"#).unwrap().as_bool(),
            Some(true)
        );
        assert_eq!(
            p.execute(r#"isinstance(3.14, tuple("int", "str"))"#).unwrap().as_bool(),
            Some(false)
        );
    }

    #[test]
    fn test_isinstance_struct() {
        let mut p = PurePipeline::new().unwrap();
        let r = p.execute("struct Foo { x }\nf = Foo(1)\nisinstance(f, Foo)").unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn test_isinstance_struct_inheritance() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("struct Base { x }\nstruct Child extends(Base) { y }\nc = Child(1, 2)\nisinstance(c, Base)")
            .unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn test_isinstance_struct_negative() {
        let mut p = PurePipeline::new().unwrap();
        let r = p
            .execute("struct A { x }\nstruct B { y }\na = A(1)\nisinstance(a, B)")
            .unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }
}
