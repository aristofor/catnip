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
    /// CFG+SSA round-trip enabled (opt-in, off by default).
    cfg_enabled: bool,
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
            cfg_enabled: false,
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

    /// Enable or disable the CFG+SSA round-trip in semantic analysis.
    pub fn set_cfg_enabled(&mut self, enabled: bool) {
        self.cfg_enabled = enabled;
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
            analyzer.set_cfg_enabled(self.cfg_enabled);
            analyzer.analyze(&ir)
        } else {
            Ok(ir)
        }
    }

    /// Full pipeline: source → Value.
    ///
    /// State (globals, functions) persists between calls.
    ///
    /// Ownership: the returned `Value` is OWNED by the caller. A caller that
    /// discards it must `decref()` it -- ignoring a heap-valued result (struct
    /// instance, closure, bigint, list) leaks one ref per call (the
    /// `load_cat_file`/`catnip_mcp` family of fixes, 2026-07-06).
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

        // 3-5. Semantic analysis, compile, execute.
        self.execute_ir(&ir)
    }

    /// Compile and run an already-transformed IR: semantic analysis → compile →
    /// execute. State (globals, functions) persists between calls, like `execute`.
    ///
    /// Lets callers run an IR they built or transformed in-process — e.g. compare
    /// an IR against a copy round-tripped through the CFG.
    ///
    /// Ownership: same contract as `execute` -- the returned `Value` is owned
    /// by the caller and must be `decref()`'d if discarded.
    pub fn execute_ir(&mut self, ir: &IR) -> VMResult<Value> {
        // 1. Semantic analysis
        let mut analyzer = SemanticAnalyzer::with_optimizer();
        analyzer.set_tco_enabled(self.tco_enabled);
        analyzer.set_cfg_enabled(self.cfg_enabled);
        let optimized = analyzer.analyze(ir).map_err(VMError::RuntimeError)?;

        // 2. Compile
        let mut compiler = PureCompiler::new();
        let output = compiler
            .compile(&optimized)
            .map_err(|e| VMError::RuntimeError(format!("{e}")))?;

        // 3. Execute
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

    /// Get a mutable reference to the PureVM.
    pub fn vm_mut(&mut self) -> &mut PureVM {
        &mut self.vm
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
        // Release the old globals before dropping the old host: Value is Copy so
        // the map's Drop would otherwise leak every heap global (the old VM, hence
        // its closures sharing this Rc, is already gone above -- host is sole holder).
        self.host.clear_globals();
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
mod tests;
