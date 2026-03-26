// FILE: catnip_rs/src/semantic/analyzer.rs
//! Semantic analyzer that transforms IR nodes to executable Op nodes
//!
//! This is a direct port of catnip/semantic/analyzer.pyx to Rust (PyO3).
//!
//! The semantic analyzer is the third pipeline stage after parsing and transformation.
//! It converts IR (Intermediate Representation) nodes from the transformer into executable
//! Op nodes, performing semantic validation and identifier resolution.
//!
//! Key responsibilities:
//! - IR validation (check opcodes exist in registry)
//! - Identifier resolution (convert to Ref nodes)
//! - Attribute handling (getattr, setattr, setitem)
//! - Control flow (if, while, for, match)
//! - Function definitions (lambda, set_locals)
//! - TCO detection (mark tail calls)
//! - Pattern matching
//! - Broadcasting
//! - Pragmas (compiler directives)
//! - Optimization (delegates to Optimizer)

use crate::cfg::builder_ir::IRCFGBuilder;
use crate::constants::*;
use crate::core::op::Op;
use crate::ir::opcode::IROpCode;
use crate::pragma::PragmaType;
use crate::semantic::optimizer::OptimizationPass;
use crate::semantic::tail_recursion_to_loop::TailRecursionToLoopPass;
use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyList, PyTuple};
use std::sync::Mutex;

/// Semantic analyzer state
#[pyclass(name = "Semantic")]
pub struct Semantic {
    /// Operation registry (validates opcodes)
    registry: Py<PyAny>,
    /// Execution context (kept for Python API compatibility)
    #[allow(dead_code)]
    context: Py<PyAny>,
    /// Enable optimization passes
    optimize: bool,
    /// Optimizer instance (Rust-compiled passes)
    optimizer: Option<Py<PyAny>>,
    /// Pragma context for compiler directives
    #[pyo3(get, set)]
    pragma_context: Option<Py<PyAny>>,

    /// Mutable state (interior mutability for tail position tracking)
    state: Mutex<SemanticState>,
}

/// Mutable state for semantic analysis
#[derive(Default)]
struct SemanticState {
    /// Whether we're currently in a tail position (for TCO detection)
    in_tail_position: bool,
    /// Name of the current function being analyzed (for TCO detection)
    current_function: Option<String>,
}

/// Helper struct for saving/restoring tail position state
struct TailPositionGuard<'a> {
    state: &'a Mutex<SemanticState>,
    saved_tail_position: bool,
    saved_function: Option<String>,
}

impl<'a> TailPositionGuard<'a> {
    fn new(state: &'a Mutex<SemanticState>) -> Self {
        let saved_state = state.lock().unwrap();
        Self {
            state,
            saved_tail_position: saved_state.in_tail_position,
            saved_function: saved_state.current_function.clone(),
        }
    }
}

impl<'a> Drop for TailPositionGuard<'a> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        state.in_tail_position = self.saved_tail_position;
        state.current_function = self.saved_function.clone();
    }
}

#[pymethods]
impl Semantic {
    #[new]
    #[pyo3(signature = (registry, context, optimize=true))]
    fn new(py: Python<'_>, registry: Py<PyAny>, context: Py<PyAny>, optimize: bool) -> PyResult<Self> {
        // Create optimizer if optimization is enabled
        let optimizer = if optimize {
            let optimizer_class = py.import(PY_MOD_SEMANTIC)?.getattr("Optimizer")?;
            Some(optimizer_class.call0()?.into())
        } else {
            None
        };

        Ok(Self {
            registry,
            context,
            optimize,
            optimizer,
            pragma_context: None,
            state: Mutex::new(SemanticState::default()),
        })
    }

    /// Main analysis entry point
    ///
    /// Pipeline:
    /// 1. Optimize the IR if enabled
    /// 2. Transform to executable nodes via visit()
    /// 3. Apply TCO if enabled (currently stub, returns False)
    /// 4. Return executable Op tree
    fn analyze(&self, py: Python<'_>, ast: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Step 1: Optimize the IR if enabled
        let mut ast_obj = ast.clone().unbind();
        if self.optimize {
            if let Some(ref optimizer) = self.optimizer {
                ast_obj = optimizer.bind(py).call_method1("optimize", (ast_obj,))?.unbind();
            }
        }

        // Step 1.5: Apply CFG-based optimizations (if optimization enabled)
        // CFG works on IR nodes (before semantic transformation)
        let ast_after_cfg = if self.optimize {
            self.apply_cfg_optimizations(py, ast_obj)?
        } else {
            ast_obj
        };

        // Step 2: Transform to executable nodes
        let ast_bound = ast_after_cfg.bind(py);
        let result = if ast_bound.is_instance_of::<PyList>() {
            // Handle list of statements
            let list = ast_bound.cast::<PyList>()?;
            let visited_items: Result<Vec<_>, _> = list.iter().map(|node| self.visit(py, &node)).collect();
            let visited_items = visited_items?;

            // Filter out _SKIP sentinel (from pragmas)
            let nodes_module = py.import(PY_MOD_NODES)?;
            let skip_sentinel = nodes_module.getattr("_SKIP")?;
            let filtered_items: Vec<_> = visited_items
                .into_iter()
                .filter(|obj| {
                    let obj_bound = obj.bind(py);
                    // Keep if it's not the _SKIP sentinel
                    !obj_bound.is(&skip_sentinel)
                })
                .collect();

            PyList::new(py, &filtered_items)?.unbind().into()
        } else {
            // Single statement
            self.visit(py, ast_bound)?
        };

        // Step 3: Reserved for future use
        let result_after_cfg = result;

        // Step 4: Apply TCO if enabled (stub - always returns False)
        // TCO is now handled at runtime in nodes_core.pyx
        // let should_apply_tco = self._should_apply_tco(py)?;
        // if should_apply_tco { ... }

        // Step 5: Apply tail recursion to loop transformation
        let final_result = if self.optimize {
            self.apply_tail_recursion_to_loop(py, result_after_cfg)?
        } else {
            result_after_cfg
        };

        Ok(final_result)
    }

    /// Apply CFG-based optimizations on IR nodes (before semantic transformation)
    ///
    /// Pipeline:
    /// 1. Extract IR nodes (Op instances from transformer)
    /// 2. Build CFG from IR nodes
    /// 3. Apply CFG optimizations (dead code, merge blocks, empty removal, constant branches)
    /// 4. Reconstruct IR nodes from optimized CFG
    /// 5. Return optimized IR nodes for semantic transformation
    fn apply_cfg_optimizations(&self, py: Python<'_>, node: Py<PyAny>) -> PyResult<Py<PyAny>> {
        // Skip CFG optimizations if optimize level < 3
        if let Some(ref pragma_ctx) = self.pragma_context {
            let opt_level: i32 = pragma_ctx.bind(py).getattr("optimize_level")?.extract()?;
            if opt_level < 3 {
                return Ok(node);
            }
        } else {
            // No pragma context - skip CFG
            return Ok(node);
        }

        let node_bound = node.bind(py);

        // Extract Op nodes to a Vec
        let ops: Vec<Op> = if node_bound.is_instance_of::<PyList>() {
            let list = node_bound.cast::<PyList>()?;
            let mut ops = Vec::new();
            for item in list.iter() {
                if let Ok(op_ref) = item.extract::<PyRef<Op>>() {
                    ops.push(op_ref.clone());
                } else {
                    // Not an Op node, skip CFG optimization
                    return Ok(node);
                }
            }
            ops
        } else {
            // Single node
            if let Ok(op_ref) = node_bound.extract::<PyRef<Op>>() {
                vec![op_ref.clone()]
            } else {
                // Not an Op node, skip CFG optimization
                return Ok(node);
            }
        };

        // Build CFG
        let builder = IRCFGBuilder::new("semantic_cfg");
        let mut cfg = builder.build(ops);

        // Compute dominators (required for SSA and reconstruction)
        crate::cfg::analysis::compute_dominators(&mut cfg);

        // SSA construction
        let ssa = crate::cfg::ssa_builder::SSABuilder::build(&cfg);

        // SSA optimization passes
        let _cse_result = crate::cfg::ssa_cse::inter_block_cse(&cfg, &ssa);
        crate::cfg::ssa_cse::apply_cse(&mut cfg, &_cse_result);

        let _licm_result = crate::cfg::ssa_licm::licm(&mut cfg, &ssa);

        let _gvn_result = crate::cfg::ssa_gvn::gvn(&cfg, &ssa);

        // IV detection (Phase 1: uses SSA info)
        let _iv_result = crate::cfg::ssa_iv::detect_ivs(&mut cfg, &ssa);

        let _dse_result = crate::cfg::ssa_dse::global_dse(&cfg, &ssa);
        crate::cfg::ssa_dse::apply_dse(&mut cfg, &_dse_result);

        // SSA destruction
        crate::cfg::ssa_destruction::destroy_ssa(&mut cfg, &ssa);

        // IV strength reduction (Phase 2: creates new Ops post-SSA)
        crate::cfg::ssa_iv::apply_iv_strength_reduction(&mut cfg, &_iv_result);

        // Apply CFG optimizations
        let (_dead, _merged, _empty, _branches) = crate::cfg::optimization::optimize_cfg(&mut cfg);

        // Reconstruct Op nodes from optimized CFG using region detection
        let optimized_ops = crate::cfg::region::reconstruct_from_cfg(py, &cfg)?;

        // Convert back to Python objects
        if node_bound.is_instance_of::<PyList>() {
            let py_list = PyList::empty(py);
            for op in optimized_ops {
                let py_op: Py<PyAny> = Py::new(py, op)?.into();
                py_list.append(py_op)?;
            }
            Ok(py_list.unbind().into())
        } else if optimized_ops.len() == 1 {
            // Single node
            let py_op: Py<PyAny> = Py::new(py, optimized_ops.into_iter().next().unwrap())?.into();
            Ok(py_op)
        } else if optimized_ops.is_empty() {
            // Empty result (all dead code)
            Ok(py.None())
        } else {
            // Multiple nodes but input was single - wrap in list
            let py_list = PyList::empty(py);
            for op in optimized_ops {
                let py_op: Py<PyAny> = Py::new(py, op)?.into();
                py_list.append(py_op)?;
            }
            Ok(py_list.unbind().into())
        }
    }

    /// Apply tail recursion to loop transformation on Op nodes
    fn apply_tail_recursion_to_loop(&self, py: Python<'_>, node: Py<PyAny>) -> PyResult<Py<PyAny>> {
        // Use Rust TailRecursionToLoopPass directly
        let tail_pass = TailRecursionToLoopPass::new();

        // Apply the transformation
        let result = tail_pass.visit(py, node.bind(py))?;
        Ok(result)
    }

    /// Main visitor dispatcher
    ///
    /// Routes nodes to appropriate handlers based on type:
    /// - IR nodes → visit_ir()
    /// - Call nodes → visit_call()
    /// - Identifier nodes → visit_identifier()
    /// - Broadcast nodes → visit_broadcast()
    /// - Pattern nodes → visit_pattern()
    /// - Iterables → visit_iterable()
    /// - Literals → pass through unchanged
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Get type name for dispatch
        let node_type = node.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;

        // Type-based dispatch
        let result = match type_name {
            "IR" => self.visit_ir(py, node),
            "Op" => self.visit_ir(py, node), // Rust-generated Op nodes
            "Call" => self.visit_call(py, node),
            "Identifier" => self.visit_identifier(py, node),
            "Broadcast" => self.visit_broadcast(py, node),
            // Pattern nodes (check via isinstance for union types)
            _ if self.is_pattern_node(py, node)? => self.visit_pattern(py, node),
            // Iterables (but not strings/bytes)
            _ if self.is_iterable_not_string(py, node, type_name)? => self.visit_iterable(py, node),
            // Literals pass through unchanged
            _ => Ok(node.clone().unbind()),
        }?;

        // Propagate source positions from input node to output Op
        Self::propagate_position(py, node, &result);

        Ok(result)
    }
}

// Internal helper methods
impl Semantic {
    /// Check if node is a pattern node (PatternLiteral, PatternVar, PatternWildcard, PatternOr)
    fn is_pattern_node(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<bool> {
        let nodes_module = py.import(PY_MOD_NODES)?;
        let pattern_types = [
            nodes_module.getattr("PatternLiteral")?,
            nodes_module.getattr("PatternVar")?,
            nodes_module.getattr("PatternWildcard")?,
            nodes_module.getattr("PatternOr")?,
        ];

        for pattern_type in &pattern_types {
            if node.is_instance(pattern_type)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Check if node is an iterable (but not string or bytes)
    fn is_iterable_not_string(&self, py: Python<'_>, node: &Bound<'_, PyAny>, type_name: &str) -> PyResult<bool> {
        // Exclude strings and bytes
        if type_name == "str" || type_name == "bytes" {
            return Ok(false);
        }

        // Check if iterable
        let collections_abc = py.import("collections.abc")?;
        let iterable_class = collections_abc.getattr("Iterable")?;
        node.is_instance(&iterable_class)
    }

    /// Visit an IR node (dispatch to specialized handlers or default)
    fn visit_ir(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Get ident (OpCode)
        let ident = node.getattr("ident")?;

        // Try to get as i32 (OpCode enum value)
        let ident_int: i32 = if let Ok(val) = ident.extract::<i32>() {
            val
        } else {
            // Might be OpCode enum, try calling int() on it
            let builtins = py.import("builtins")?;
            let int_func = builtins.getattr("int")?;
            int_func.call1((ident.clone(),))?.extract::<i32>()?
        };

        // Dispatch based on opcode (using generated IROpCode enum)
        let result = match ident_int {
            x if x == IROpCode::Nop as i32 => {
                // Nop: dead instruction marker from CFG passes.
                // If it has args, pass through the first arg (placeholder pattern).
                let args = node.getattr("args")?;
                let args_tuple = args.cast::<PyTuple>()?;
                if !args_tuple.is_empty() {
                    self.visit(py, &args_tuple.get_item(0)?)
                } else {
                    Ok(py.None())
                }
            }
            x if x == IROpCode::Pragma as i32 => self.visit_pragma(py, node),
            x if x == IROpCode::GetAttr as i32 => self.visit_getattr(py, node),
            x if x == IROpCode::SetAttr as i32 => self.visit_setattr(py, node),
            x if x == IROpCode::SetItem as i32 => self.visit_setitem(py, node),
            x if x == IROpCode::SetLocals as i32 => self.visit_set_locals(py, node),
            x if x == IROpCode::OpIf as i32 => self.visit_if(py, node),
            x if x == IROpCode::OpWhile as i32 => self.visit_while(py, node),
            x if x == IROpCode::OpFor as i32 => self.visit_for(py, node),
            x if x == IROpCode::OpMatch as i32 => self.visit_match(py, node),
            x if x == IROpCode::OpLambda as i32 => self.visit_lambda(py, node),
            x if x == IROpCode::OpBlock as i32 => self.visit_block(py, node),
            x if x == IROpCode::OpReturn as i32 => self.visit_return(py, node),
            x if x == IROpCode::Fstring as i32 => self.visit_fstring(py, node),
            x if x == IROpCode::OpStruct as i32 => self.visit_struct(py, node),
            x if x == IROpCode::Call as i32 => self.visit_call_op(py, node),
            // Intrinsics: no args to visit, pass through as-is
            x if x == IROpCode::Globals as i32 || x == IROpCode::Locals as i32 => self.visit_ir_default(py, node),
            _ => self.visit_ir_default(py, node), // All other opcodes
        }?;

        Ok(result)
    }

    /// Copy start_byte/end_byte from source IR node to output Op.
    fn propagate_position(py: Python<'_>, source: &Bound<'_, PyAny>, target: &Py<PyAny>) {
        let Ok(sb) = source.getattr("start_byte") else {
            return;
        };
        let Ok(sb_val) = sb.extract::<isize>() else {
            return;
        };
        if sb_val < 0 {
            return;
        }
        let bound = target.bind(py);
        let _ = bound.setattr("start_byte", sb_val);
        if let Ok(eb) = source.getattr("end_byte") {
            let _ = bound.setattr("end_byte", eb);
        }
    }

    /// Default IR visitor for opcodes without special handling
    ///
    /// Validates opcode exists in registry, then visits all args/kwargs
    fn visit_ir_default(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Save tail position state
        let _guard = TailPositionGuard::new(&self.state);

        // Arguments are NOT in tail position
        self.state.lock().unwrap().in_tail_position = false;

        // Get ident
        let ident = node.getattr("ident")?;

        // Validate opcode exists in registry
        let registry = self.registry.bind(py);
        let internals = registry.getattr("internals")?;
        if !internals.contains(&ident)? {
            let exc_module = py.import(PY_MOD_EXC)?;
            let semantic_error = exc_module.getattr("CatnipSemanticError")?;
            let kwargs = PyDict::new(py);
            if let Ok(sb) = node.getattr("start_byte") {
                let _ = kwargs.set_item("start_byte", sb);
            }
            return Err(PyErr::from_value(
                semantic_error.call((format!("Unknown opcode: {:?}", ident),), Some(&kwargs))?,
            ));
        }

        // Visit args
        let args_tuple = node.getattr("args")?;
        let args_list = args_tuple.cast::<PyTuple>()?;
        let visited_args: Result<Vec<_>, _> = args_list.iter().map(|arg| self.visit(py, &arg)).collect();
        let visited_args = visited_args?;
        let new_args = PyTuple::new(py, &visited_args)?;

        // Visit kwargs
        let kwargs_dict = node.getattr("kwargs")?;
        let new_kwargs = PyDict::new(py);
        if let Ok(dict) = kwargs_dict.cast::<PyDict>() {
            for (key, value) in dict.iter() {
                let visited_value = self.visit(py, &value)?;
                new_kwargs.set_item(key, visited_value)?;
            }
        }

        // Create new Op node
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        op_class.call1((ident, new_args, new_kwargs)).map(|obj| obj.unbind())
    }

    /// Visit identifier - convert to Ref (variable reference)
    fn visit_identifier(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Identifier is a str subclass - use it directly as the name
        let name = node.extract::<String>()?;
        let ref_class = py.import(PY_MOD_NODES)?.getattr("Ref")?;
        ref_class.call1((name,)).map(|obj| obj.unbind())
    }

    /// Visit iterable (list, tuple) - recursively visit items, preserve type
    fn visit_iterable(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Try to iterate
        if let Ok(iter) = node.try_iter() {
            let visited_items: Result<Vec<_>, _> = iter
                .map(|item| {
                    let item = item?;
                    self.visit(py, &item)
                })
                .collect();
            let visited_items = visited_items?;

            // Preserve type (list vs tuple)
            let node_type = node.get_type();
            let type_name_obj = node_type.name()?;
            let type_name = type_name_obj.to_str()?;

            if type_name == "tuple" {
                Ok(PyTuple::new(py, &visited_items)?.unbind().into())
            } else {
                // Default to list
                Ok(PyList::new(py, &visited_items)?.unbind().into())
            }
        } else {
            // Not iterable, return as-is
            Ok(node.clone().unbind())
        }
    }

    /// Visit pattern node - pass through (but visit nested values in PatternLiteral)
    fn visit_pattern(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let node_type = node.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;

        // PatternLiteral has a value field that needs visiting
        if type_name == catnip::PATTERN_LITERAL {
            let value = node.getattr("value")?;
            let visited_value = self.visit(py, &value)?;

            // Create new PatternLiteral with visited value
            let pattern_literal = py.import(PY_MOD_NODES)?.getattr("PatternLiteral")?;
            return pattern_literal.call1((visited_value,)).map(|obj| obj.unbind());
        }

        // Other patterns pass through unchanged
        Ok(node.clone().unbind())
    }

    /// Visit broadcast node
    fn visit_broadcast(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let target = node.getattr("target")?;
        let operator = node.getattr("operator")?;
        let operand = node.getattr("operand")?;
        let is_filter = node.getattr("is_filter")?;

        let visited_target = self.visit(py, &target)?;
        let visited_operator = self.visit(py, &operator)?;
        let visited_operand = self.visit(py, &operand)?;

        // Operator is now visited (converts IR lambdas to Op nodes)
        let broadcast_class = py.import(PY_MOD_NODES)?.getattr("Broadcast")?;
        broadcast_class
            .call((visited_target, visited_operator, visited_operand, is_filter), None)
            .map(|obj| obj.unbind())
    }

    // Specialized visitors for control flow and special cases
    // These are declared in separate methods below
}

// This file contains the fixed visitor implementations
// Copy-paste this to replace the "Specialized visitor implementations" section

// Specialized visitor implementations
impl Semantic {
    /// Visit pragma directive - processes compiler directives
    /// Visit struct declaration - pass-through to default visitor
    fn visit_struct(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.visit_ir_default(py, node)
    }

    fn visit_pragma(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let pragma_module = py.import(PY_MOD_PRAGMA)?;
        // PragmaType resolved via Rust enum, no Python lookup needed

        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() < 2 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1((
                "pragma",
                (2, f64::INFINITY),
                args.len(),
            ))?));
        }

        let directive = args.get_item(0)?;
        let value = args.get_item(1)?;
        let options = PyDict::new(py);

        if args.len() > 2 {
            let directive_str = directive.extract::<String>()?;
            let directive_lower = directive_str.to_lowercase();

            if directive_lower == "workaround" {
                if args.len() > 2 {
                    let arg2 = args.get_item(2)?;
                    options.set_item("enabled", arg2)?;
                }
            } else {
                for i in 2..args.len() {
                    let arg = args.get_item(i)?;
                    if let Ok(arg_str) = arg.extract::<String>() {
                        if let Some((k, v)) = arg_str.split_once('=') {
                            options.set_item(k, v)?;
                        } else {
                            options.set_item(arg_str, true)?;
                        }
                    } else {
                        options.set_item(arg.to_string(), true)?;
                    }
                }
            }
        }

        let directive_str = directive.extract::<String>()?;
        let directive_lower = directive_str.to_lowercase();

        let pt = match PragmaType::from_directive(&directive_lower) {
            Some(pt) => pt,
            None => {
                let exc_module = py.import(PY_MOD_EXC)?;
                let semantic_error = exc_module.getattr("CatnipSemanticError")?;
                let err_kwargs = PyDict::new(py);
                if let Ok(sb) = node.getattr("start_byte") {
                    let _ = err_kwargs.set_item("start_byte", sb);
                }
                let known = PragmaType::all_directives().join(", ");
                return Err(PyErr::from_value(semantic_error.call(
                    (format!(
                        "Unknown pragma directive: '{}'. Known: {}",
                        directive_str, known
                    ),),
                    Some(&err_kwargs),
                )?));
            }
        };
        let pragma_type = pt.into_pyobject(py)?;

        // Validate pragma value early (before pragma_context check)
        // so that invalid values are caught even without a PragmaContext
        let exc_module_lazy = || py.import(PY_MOD_EXC);
        let pragma_err = |msg: String| -> PyResult<Py<PyAny>> {
            let exc_module = exc_module_lazy()?;
            let pragma_error_cls = exc_module.getattr("CatnipPragmaError")?;
            let err_kwargs = PyDict::new(py);
            if let Ok(sb) = node.getattr("start_byte") {
                let _ = err_kwargs.set_item("start_byte", sb);
            }
            Err(PyErr::from_value(pragma_error_cls.call((msg,), Some(&err_kwargs))?))
        };

        match pt {
            PragmaType::Tco | PragmaType::Cache | PragmaType::Debug | PragmaType::Warning | PragmaType::NdMemoize => {
                if value.extract::<bool>().is_err() {
                    return pragma_err(format!("Pragma '{}' requires True or False", directive_str));
                }
            }
            PragmaType::Optimize => match value.extract::<i32>() {
                Ok(level) if !(0..=3).contains(&level) => {
                    return pragma_err(format!("Optimization level must be 0-3, got {}", level));
                }
                Err(_) => {
                    return pragma_err("Pragma 'optimize' requires an integer 0-3".to_string());
                }
                _ => {}
            },
            PragmaType::NdMode => match value.extract::<String>() {
                Ok(mode) => {
                    let m = mode.to_lowercase();
                    if !PragmaType::nd_mode_values().contains(&m.as_str()) {
                        return pragma_err(format!(
                            "Unknown ND mode: '{}'. Use ND.sequential, ND.thread, or ND.process",
                            mode
                        ));
                    }
                }
                Err(_) => {
                    return pragma_err("Pragma 'nd_mode' requires ND.sequential, ND.thread, or ND.process".to_string());
                }
            },
            PragmaType::NdWorkers => match value.extract::<i32>() {
                Ok(n) if n < 0 => {
                    return pragma_err(format!("ND workers must be non-negative, got {}", n));
                }
                Err(_) => {
                    return pragma_err("Pragma 'nd_workers' requires an integer".to_string());
                }
                _ => {}
            },
            PragmaType::NdBatchSize => match value.extract::<i32>() {
                Ok(n) if n < 0 => {
                    return pragma_err(format!("ND batch size must be non-negative, got {}", n));
                }
                Err(_) => {
                    return pragma_err("Pragma 'nd_batch_size' requires an integer".to_string());
                }
                _ => {}
            },
            PragmaType::Jit => {
                if value.extract::<bool>().is_err() {
                    if let Ok(s) = value.extract::<String>() {
                        if s != "all" {
                            return pragma_err("Pragma 'jit' requires True, False, or \"all\"".to_string());
                        }
                    } else {
                        return pragma_err("Pragma 'jit' requires True, False, or \"all\"".to_string());
                    }
                }
            }
            _ => {} // Inline, Pure: validated elsewhere
        }

        let pragma_class = pragma_module.getattr("Pragma")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("type", pragma_type)?;
        kwargs.set_item("directive", directive)?;
        kwargs.set_item("value", value)?;
        kwargs.set_item("options", options)?;
        let pragma = pragma_class.call((), Some(&kwargs))?;

        // Apply pragma to context if available (from Semantic instance)
        if let Some(ref pragma_ctx) = self.pragma_context {
            pragma_ctx.bind(py).call_method1("add", (pragma.clone(),))?;
        }

        // Return sentinel object _SKIP instead of None to distinguish from None literals
        let nodes_module = py.import(PY_MOD_NODES)?;
        nodes_module.getattr("_SKIP").map(|obj| obj.unbind())
    }

    fn visit_getattr(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() < 2 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1((
                "getattr",
                (2, f64::INFINITY),
                args.len(),
            ))?));
        }

        let obj = args.get_item(0)?;
        let visited_obj = self.visit(py, &obj)?;

        let attr_name = args.get_item(1)?;
        let attr_str = attr_name.to_string();

        let mut visited_args = vec![visited_obj, attr_str.into_pyobject(py)?.to_owned().unbind().into()];

        for i in 2..args.len() {
            let arg = args.get_item(i)?;
            visited_args.push(self.visit(py, &arg)?);
        }

        let kwargs_dict = node.getattr("kwargs")?;
        let new_kwargs = PyDict::new(py);
        if let Ok(dict) = kwargs_dict.cast::<PyDict>() {
            for (key, value) in dict.iter() {
                let visited_value = self.visit(py, &value)?;
                new_kwargs.set_item(key, visited_value)?;
            }
        }

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(py, &visited_args)?;
        op_class.call1((ident, new_args, new_kwargs)).map(|obj| obj.unbind())
    }

    fn visit_setattr(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() != 3 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1(("setattr", 3, args.len()))?));
        }

        let obj = args.get_item(0)?;
        let attr_name = args.get_item(1)?;
        let value = args.get_item(2)?;

        let visited_obj = self.visit(py, &obj)?;
        let visited_value = self.visit(py, &value)?;
        let attr_str = attr_name.to_string();

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(
            py,
            &[
                visited_obj,
                attr_str.into_pyobject(py)?.to_owned().unbind().into(),
                visited_value,
            ],
        )?;
        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_setitem(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() != 3 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1(("setitem", 3, args.len()))?));
        }

        let obj = args.get_item(0)?;
        let index = args.get_item(1)?;
        let value = args.get_item(2)?;

        let visited_obj = self.visit(py, &obj)?;
        let visited_index = self.visit(py, &index)?;
        let visited_value = self.visit(py, &value)?;

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(py, &[visited_obj, visited_index, visited_value])?;
        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_set_locals(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() < 2 || args.len() > 3 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1((
                "set_locals",
                (2, 3),
                args.len(),
            ))?));
        }

        let names = args.get_item(0)?;
        let value = args.get_item(1)?;

        let _guard = TailPositionGuard::new(&self.state);

        let value_type = value.get_type();
        let value_type_name_obj = value_type.name()?;
        let value_type_name = value_type_name_obj.to_str()?;
        if value_type_name == catnip::IR {
            let value_ident = value.getattr("ident")?;
            let value_ident_int: i32 = if let Ok(val) = value_ident.extract::<i32>() {
                val
            } else {
                let builtins = py.import("builtins")?;
                let int_func = builtins.getattr("int")?;
                int_func.call1((value_ident,))?.extract::<i32>()?
            };

            if value_ident_int == IROpCode::OpLambda as i32 {
                if let Ok(names_iter) = names.try_iter() {
                    let names_vec: Vec<_> = names_iter.collect::<Result<Vec<_>, _>>()?;
                    if names_vec.len() == 1 {
                        let func_name = names_vec[0].to_string();
                        self.state.lock().unwrap().current_function = Some(func_name);
                    }
                }
            }
        }

        self.state.lock().unwrap().in_tail_position = false;
        let visited_value = self.visit(py, &value)?;

        // Get explicit_unpack from args[2] if present, or from kwargs for backward compatibility
        let explicit_unpack = if args.len() == 3 {
            // New format: explicit_unpack as 3rd positional argument
            args.get_item(2)?.extract::<bool>().unwrap_or(false)
        } else if let Ok(kwargs) = node.getattr("kwargs") {
            // Old format: explicit_unpack in kwargs
            kwargs
                .get_item("explicit_unpack")
                .ok()
                .and_then(|v| v.extract::<bool>().ok())
                .unwrap_or(false)
        } else {
            false
        };

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;

        // Add explicit_unpack as 3rd argument if True
        let new_args = if explicit_unpack {
            let true_val = pyo3::types::PyBool::new(py, true).to_owned();
            PyTuple::new(py, &[names.unbind(), visited_value, true_val.into()])?
        } else {
            PyTuple::new(py, &[names.unbind(), visited_value])?
        };

        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_if(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.is_empty() {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1(("if", (1, 2), 0))?));
        }

        let saved_tail_position = self.state.lock().unwrap().in_tail_position;

        let branches = args.get_item(0)?;
        let else_block = if args.len() > 1 { Some(args.get_item(1)?) } else { None };

        let mut visited_branches = Vec::new();
        if let Ok(branches_iter) = branches.try_iter() {
            for branch in branches_iter {
                let branch = branch?;
                let condition = branch.get_item(0)?;
                let block = branch.get_item(1)?;

                self.state.lock().unwrap().in_tail_position = false;
                let visited_condition = self.visit(py, &condition)?;

                self.state.lock().unwrap().in_tail_position = saved_tail_position;
                let visited_block = self.visit(py, &block)?;

                visited_branches.push((visited_condition, visited_block));
            }
        }

        let visited_else = if let Some(else_blk) = else_block {
            self.state.lock().unwrap().in_tail_position = saved_tail_position;
            Some(self.visit(py, &else_blk)?)
        } else {
            None
        };

        self.state.lock().unwrap().in_tail_position = saved_tail_position;

        let branches_tuple = PyTuple::new(py, &visited_branches)?;
        let new_args_vec: Vec<Py<PyAny>> = if let Some(visited_else) = visited_else {
            vec![branches_tuple.clone().unbind().into(), visited_else]
        } else {
            vec![branches_tuple.unbind().into()]
        };
        let new_args = PyTuple::new(py, &new_args_vec)?;

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_while(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() != 2 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1(("while", 2, args.len()))?));
        }

        let condition = args.get_item(0)?;
        let block = args.get_item(1)?;

        let saved_tail_position = self.state.lock().unwrap().in_tail_position;
        self.state.lock().unwrap().in_tail_position = false;

        let visited_condition = self.visit(py, &condition)?;
        let visited_block = self.visit(py, &block)?;

        self.state.lock().unwrap().in_tail_position = saved_tail_position;

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(py, &[visited_condition, visited_block])?;
        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_for(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() != 3 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1(("for", 3, args.len()))?));
        }

        let identifier = args.get_item(0)?;
        let iterable = args.get_item(1)?;
        let block = args.get_item(2)?;

        let saved_tail_position = self.state.lock().unwrap().in_tail_position;
        self.state.lock().unwrap().in_tail_position = false;

        let visited_iterable = self.visit(py, &iterable)?;
        let visited_block = self.visit(py, &block)?;

        self.state.lock().unwrap().in_tail_position = saved_tail_position;

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(py, &[identifier.unbind(), visited_iterable, visited_block])?;
        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_match(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() != 2 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1(("match", 2, args.len()))?));
        }

        let saved_tail_position = self.state.lock().unwrap().in_tail_position;

        let value_expr = args.get_item(0)?;
        let cases = args.get_item(1)?;

        self.state.lock().unwrap().in_tail_position = false;
        let visited_value = self.visit(py, &value_expr)?;

        let mut visited_cases = Vec::new();
        if let Ok(cases_iter) = cases.try_iter() {
            for case in cases_iter {
                let case = case?;
                let pattern = case.get_item(0)?;
                let guard = case.get_item(1)?;
                let body = case.get_item(2)?;

                self.state.lock().unwrap().in_tail_position = false;
                let visited_pattern = self.visit(py, &pattern)?;
                let visited_guard = if guard.is_none() {
                    py.None()
                } else {
                    self.visit(py, &guard)?
                };

                self.state.lock().unwrap().in_tail_position = saved_tail_position;
                let visited_body = self.visit(py, &body)?;

                visited_cases.push((visited_pattern, visited_guard, visited_body));
            }
        }

        self.state.lock().unwrap().in_tail_position = saved_tail_position;

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let cases_tuple = PyTuple::new(py, &visited_cases)?;
        let new_args = PyTuple::new(py, &[visited_value, cases_tuple.unbind().into()])?;
        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_lambda(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        if args.len() != 2 {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1(("lambda", 2, args.len()))?));
        }

        let _guard = TailPositionGuard::new(&self.state);

        let params = args.get_item(0)?;
        let block = args.get_item(1)?;

        self.state.lock().unwrap().in_tail_position = false;
        let mut visited_params = Vec::new();
        if let Ok(params_iter) = params.try_iter() {
            for param in params_iter {
                let param = param?;
                if param.is_none() {
                    continue;
                }
                let param_name = param.get_item(0)?;
                let default = param.get_item(1)?;

                if default.is_none() {
                    visited_params.push((param_name.unbind(), py.None()));
                } else {
                    let visited_default = self.visit(py, &default)?;
                    visited_params.push((param_name.unbind(), visited_default));
                }
            }
        }

        self.state.lock().unwrap().in_tail_position = true;
        let mut visited_block = self.visit(py, &block)?;

        let block_obj = visited_block.bind(py);
        let block_type = block_obj.get_type();
        let block_type_name_obj = block_type.name()?;
        let block_type_name = block_type_name_obj.to_str()?;

        if block_type_name == catnip::OP {
            let block_ident = block_obj.getattr("ident")?;
            let block_ident_int: i32 = if let Ok(val) = block_ident.extract::<i32>() {
                val
            } else {
                let builtins = py.import("builtins")?;
                let int_func = builtins.getattr("int")?;
                int_func.call1((block_ident,))?.extract::<i32>()?
            };

            if block_ident_int == IROpCode::OpBlock as i32 {
                let block_args_obj = block_obj.getattr("args")?;
                let block_args = block_args_obj.cast::<PyTuple>()?;
                if block_args.len() == 1 {
                    let first_stmt = block_args.get_item(0)?;
                    let first_stmt_type = first_stmt.get_type();
                    let first_stmt_type_name_obj = first_stmt_type.name()?;
                    let first_stmt_type_name = first_stmt_type_name_obj.to_str()?;

                    if first_stmt_type_name == catnip::OP {
                        let first_stmt_ident = first_stmt.getattr("ident")?;
                        let first_stmt_ident_int: i32 = if let Ok(val) = first_stmt_ident.extract::<i32>() {
                            val
                        } else {
                            let builtins = py.import("builtins")?;
                            let int_func = builtins.getattr("int")?;
                            int_func.call1((first_stmt_ident,))?.extract::<i32>()?
                        };

                        if first_stmt_ident_int == IROpCode::OpIf as i32 {
                            let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
                            let block_opcode = py.import(PY_MOD_SEMANTIC)?.getattr("OpCode")?.getattr("OP_BLOCK")?;
                            let wrapped_args = PyTuple::new(py, &[visited_block])?;
                            visited_block = op_class.call1((block_opcode, wrapped_args, PyDict::new(py)))?.unbind();
                        }
                    }
                }
            }
        }

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let params_tuple = PyTuple::new(py, &visited_params)?;
        let new_args = PyTuple::new(py, &[params_tuple.unbind().into(), visited_block])?;
        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_block(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let saved_tail_position = self.state.lock().unwrap().in_tail_position;

        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;
        let mut visited_statements = Vec::new();

        for (i, stmt) in args.iter().enumerate() {
            let is_last = i == args.len() - 1;
            self.state.lock().unwrap().in_tail_position = if is_last { saved_tail_position } else { false };
            visited_statements.push(self.visit(py, &stmt)?);
        }

        self.state.lock().unwrap().in_tail_position = saved_tail_position;

        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(py, &visited_statements)?;
        op_class
            .call1((ident, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_return(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let saved_tail_position = self.state.lock().unwrap().in_tail_position;

        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;
        let ident = node.getattr("ident")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;

        let result = if args.is_empty() {
            let new_args = PyTuple::new(py, &[py.None()])?;
            op_class.call1((ident, new_args, PyDict::new(py)))?
        } else if args.len() == 1 {
            self.state.lock().unwrap().in_tail_position = true;
            let value = args.get_item(0)?;
            let visited_value = self.visit(py, &value)?;
            let new_args = PyTuple::new(py, &[visited_value])?;
            op_class.call1((ident, new_args, PyDict::new(py)))?
        } else {
            let exc_module = py.import(PY_MOD_EXC)?;
            let arity_error = exc_module.getattr("CatnipArityError")?;
            return Err(PyErr::from_value(arity_error.call1(("return", (0, 1), args.len()))?));
        };

        self.state.lock().unwrap().in_tail_position = saved_tail_position;

        Ok(result.unbind())
    }

    fn visit_fstring(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args_obj = node.getattr("args")?;
        let args = args_obj.cast::<PyTuple>()?;

        let mut visited_parts: Vec<Py<PyAny>> = Vec::new();

        for part in args.iter() {
            // String → text part, pass through
            let type_name = part.get_type().name()?;
            let type_str = type_name.to_str()?;

            if type_str == "str" {
                visited_parts.push(part.unbind());
            } else if part.is_instance_of::<PyTuple>() {
                // Tuple([expr, Int(conv), spec]) → visit expr, keep conv and spec
                let tuple = part.cast::<PyTuple>()?;
                let expr = tuple.get_item(0)?;
                let conv = tuple.get_item(1)?;
                let spec = tuple.get_item(2)?;

                let visited_expr = self.visit(py, &expr)?;

                let new_tuple = PyTuple::new(py, &[visited_expr.into_bound(py), conv, spec])?;
                visited_parts.push(new_tuple.unbind().into());
            } else {
                // Other types (e.g. int from Rust IR) pass through
                visited_parts.push(part.unbind());
            }
        }

        let fstring_opcode = py.import(PY_MOD_SEMANTIC)?.getattr("OpCode")?.getattr("FSTRING")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(py, &visited_parts)?;
        op_class
            .call1((fstring_opcode, new_args, PyDict::new(py)))
            .map(|obj| obj.unbind())
    }

    fn visit_call(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let saved_tail_position = self.state.lock().unwrap().in_tail_position;

        let func = node.getattr("func")?;
        let call_args = node.getattr("args")?;

        // Intercept breakpoint() calls and emit Breakpoint opcode
        if let Ok(func_name) = func.getattr("ident").and_then(|n| n.extract::<String>()) {
            if func_name == "breakpoint" {
                let bp_opcode = py.import(PY_MOD_SEMANTIC)?.getattr("OpCode")?.getattr("BREAKPOINT")?;
                let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
                let empty_args = PyTuple::empty(py);
                return Ok(op_class.call1((bp_opcode, empty_args, PyDict::new(py)))?.unbind());
            }

            // Intercept typeof(expr) calls and emit TypeOf opcode
            if func_name == "typeof" {
                let args_list: Vec<Bound<'_, PyAny>> = call_args
                    .try_iter()
                    .ok()
                    .map(|iter| iter.filter_map(|a| a.ok()).collect())
                    .unwrap_or_default();
                if args_list.len() == 1 {
                    let visited_arg = self.visit(py, &args_list[0])?;
                    let typeof_opcode = py.import(PY_MOD_SEMANTIC)?.getattr("OpCode")?.getattr("TYPE_OF")?;
                    let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
                    let args_tuple = PyTuple::new(py, &[visited_arg])?;
                    return Ok(op_class.call1((typeof_opcode, args_tuple, PyDict::new(py)))?.unbind());
                }
            }

            // Intercept globals()/locals() calls and emit intrinsic opcodes
            if func_name == "globals" || func_name == "locals" {
                let args_list: Vec<Bound<'_, PyAny>> = call_args
                    .try_iter()
                    .ok()
                    .map(|iter| iter.filter_map(|a| a.ok()).collect())
                    .unwrap_or_default();
                if args_list.is_empty() {
                    let opcode_name = if func_name == "globals" { "GLOBALS" } else { "LOCALS" };
                    let opcode = py.import(PY_MOD_SEMANTIC)?.getattr("OpCode")?.getattr(opcode_name)?;
                    let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
                    let empty_args = PyTuple::empty(py);
                    return Ok(op_class.call1((opcode, empty_args, PyDict::new(py)))?.unbind());
                }
            }
        }

        self.state.lock().unwrap().in_tail_position = false;
        let visited_func = self.visit(py, &func)?;

        let mut visited_args = vec![visited_func.clone_ref(py)];
        if let Ok(args_iter) = call_args.try_iter() {
            for arg in args_iter {
                let arg = arg?;
                visited_args.push(self.visit(py, &arg)?);
            }
        }

        let kwargs = node.getattr("kwargs")?;
        let visited_kwargs = PyDict::new(py);

        if !kwargs.is_none() && kwargs.hasattr("items")? {
            // Has items() method - iterate over it (works for dict)
            let items = kwargs.call_method0("items")?;
            for item in items.try_iter()? {
                let item = item?;
                let key = item.get_item(0)?;
                let value = item.get_item(1)?;
                let visited_value = self.visit(py, &value)?;
                visited_kwargs.set_item(key, visited_value)?;
            }
        }

        let call_opcode = py.import(PY_MOD_SEMANTIC)?.getattr("OpCode")?.getattr("CALL")?;
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(py, &visited_args)?;
        let result = op_class.call1((call_opcode, new_args, visited_kwargs))?;

        let current_function = self.state.lock().unwrap().current_function.clone();

        let args_slice = &visited_args[1..];
        let args_tuple = PyTuple::new(py, args_slice)?;
        let has_self_call_in_args = self.contains_self_call(py, args_tuple.as_any(), current_function.as_deref())?;
        let has_self_call_in_func = self.contains_self_call(py, visited_func.bind(py), current_function.as_deref())?;

        if saved_tail_position && current_function.is_some() && !has_self_call_in_args && !has_self_call_in_func {
            result.setattr("tail", true)?;
        }

        self.state.lock().unwrap().in_tail_position = saved_tail_position;

        Ok(result.unbind())
    }

    /// Visit a CALL Op node with tail-call detection.
    ///
    /// Op(CALL, [func_ref, arg1, arg2, ...], kwargs)
    /// Same as visit_ir_default but marks tail=true when appropriate.
    fn visit_call_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let saved_tail_position = self.state.lock().unwrap().in_tail_position;

        // Visit args with in_tail_position = false (args are never in tail position)
        self.state.lock().unwrap().in_tail_position = false;

        let ident = node.getattr("ident")?;
        let args_tuple = node.getattr("args")?.cast::<PyTuple>()?.clone();
        let visited_args: Result<Vec<_>, _> = args_tuple.iter().map(|arg| self.visit(py, &arg)).collect();
        let visited_args = visited_args?;
        let new_args = PyTuple::new(py, &visited_args)?;

        let kwargs_dict = node.getattr("kwargs")?;
        let new_kwargs = PyDict::new(py);
        if let Ok(dict) = kwargs_dict.cast::<PyDict>() {
            for (key, value) in dict.iter() {
                let visited_value = self.visit(py, &value)?;
                new_kwargs.set_item(key, visited_value)?;
            }
        }

        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let result = op_class.call1((&ident, &new_args, &new_kwargs))?;
        Self::propagate_position(py, node, &result.clone().unbind());

        // Tail-call detection: mark if in tail position within a named function
        let current_function = self.state.lock().unwrap().current_function.clone();
        if saved_tail_position && current_function.is_some() {
            // Check args[0] is a Ref to current_function (direct self-call)
            if !visited_args.is_empty() {
                let func_ref = visited_args[0].bind(py);
                let is_self_call = func_ref
                    .getattr("ident")
                    .and_then(|id| id.extract::<String>())
                    .map(|name| Some(name) == current_function)
                    .unwrap_or(false);

                if is_self_call {
                    // Check no recursive calls hiding in the argument expressions
                    let call_args = &visited_args[1..];
                    let args_py = PyTuple::new(py, call_args)?;
                    let has_self_call_in_args =
                        self.contains_self_call(py, args_py.as_any(), current_function.as_deref())?;

                    if !has_self_call_in_args {
                        result.setattr("tail", true)?;
                    }
                }
            }
        }

        self.state.lock().unwrap().in_tail_position = saved_tail_position;
        Ok(result.unbind())
    }

    fn contains_self_call(&self, py: Python<'_>, node: &Bound<'_, PyAny>, func_name: Option<&str>) -> PyResult<bool> {
        if func_name.is_none() {
            return Ok(false);
        }

        let node_type = node.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;

        if type_name == catnip::CALL {
            let func_expr = node.getattr("func")?;
            let func_type = func_expr.get_type();
            let func_type_name_obj = func_type.name()?;
            let func_type_name = func_type_name_obj.to_str()?;

            if func_type_name == catnip::IDENTIFIER {
                let ident_str = func_expr.to_string();
                if ident_str == func_name.unwrap() {
                    return Ok(true);
                }
            }

            if let Ok(args) = node.getattr("args") {
                if let Ok(args_iter) = args.try_iter() {
                    for child in args_iter {
                        let child = child?;
                        if self.contains_self_call(py, &child, func_name)? {
                            return Ok(true);
                        }
                    }
                }
            }
            return Ok(false);
        }

        if type_name == catnip::IR || type_name == catnip::OP {
            if let Ok(ident) = node.getattr("ident") {
                let ident_int: i32 = if let Ok(val) = ident.extract::<i32>() {
                    val
                } else {
                    let builtins = py.import("builtins")?;
                    let int_func = builtins.getattr("int")?;
                    int_func.call1((ident,))?.extract::<i32>()?
                };

                if ident_int == IROpCode::Call as i32 {
                    if let Ok(args_obj) = node.getattr("args") {
                        if let Ok(args_tuple) = args_obj.cast::<PyTuple>() {
                            if !args_tuple.is_empty() {
                                let func_expr = args_tuple.get_item(0)?;
                                let func_type = func_expr.get_type();
                                let func_type_name_obj = func_type.name()?;
                                let func_type_name = func_type_name_obj.to_str()?;

                                if func_type_name == catnip::REF {
                                    let ref_ident = func_expr.getattr("ident")?;
                                    let ref_ident_str = ref_ident.extract::<String>()?;
                                    if ref_ident_str == func_name.unwrap() {
                                        return Ok(true);
                                    }
                                }
                            }
                        }
                    }
                }

                if let Ok(args) = node.getattr("args") {
                    if let Ok(args_iter) = args.try_iter() {
                        for child in args_iter {
                            let child = child?;
                            if self.contains_self_call(py, &child, func_name)? {
                                return Ok(true);
                            }
                        }
                    }
                }
            }
            return Ok(false);
        }

        // Exclude strings and bytes to avoid infinite recursion
        // (strings are iterable and iterate over single-char strings, which iterate over themselves)
        let node_type = node.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;
        if type_name == "str" || type_name == "bytes" {
            return Ok(false);
        }

        if let Ok(iter) = node.try_iter() {
            for child in iter {
                let child = child?;
                if self.contains_self_call(py, &child, func_name)? {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that IROpCode values match Python OpCode values
    ///
    /// This test ensures that the generated IROpCode enum stays in sync with
    /// catnip/semantic/opcode.py. If this test fails, run:
    ///     python catnip_rs/gen_opcodes.py
    #[test]
    fn test_opcode_values_match_python() {
        Python::initialize();
        Python::attach(|py| {
            let opcode_mod = py.import(PY_MOD_SEMANTIC_OPCODE).unwrap();
            let opcode_class = opcode_mod.getattr("OpCode").unwrap();

            // Test critical opcodes used in dispatch table
            assert_eq!(
                opcode_class.getattr("PRAGMA").unwrap().extract::<i32>().unwrap(),
                IROpCode::Pragma as i32,
                "PRAGMA opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("GETATTR").unwrap().extract::<i32>().unwrap(),
                IROpCode::GetAttr as i32,
                "GETATTR opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("SETATTR").unwrap().extract::<i32>().unwrap(),
                IROpCode::SetAttr as i32,
                "SETATTR opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("SETITEM").unwrap().extract::<i32>().unwrap(),
                IROpCode::SetItem as i32,
                "SETITEM opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("SET_LOCALS").unwrap().extract::<i32>().unwrap(),
                IROpCode::SetLocals as i32,
                "SET_LOCALS opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("OP_IF").unwrap().extract::<i32>().unwrap(),
                IROpCode::OpIf as i32,
                "OP_IF opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("OP_WHILE").unwrap().extract::<i32>().unwrap(),
                IROpCode::OpWhile as i32,
                "OP_WHILE opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("OP_FOR").unwrap().extract::<i32>().unwrap(),
                IROpCode::OpFor as i32,
                "OP_FOR opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("OP_MATCH").unwrap().extract::<i32>().unwrap(),
                IROpCode::OpMatch as i32,
                "OP_MATCH opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("OP_LAMBDA").unwrap().extract::<i32>().unwrap(),
                IROpCode::OpLambda as i32,
                "OP_LAMBDA opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("OP_BLOCK").unwrap().extract::<i32>().unwrap(),
                IROpCode::OpBlock as i32,
                "OP_BLOCK opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("OP_RETURN").unwrap().extract::<i32>().unwrap(),
                IROpCode::OpReturn as i32,
                "OP_RETURN opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("FSTRING").unwrap().extract::<i32>().unwrap(),
                IROpCode::Fstring as i32,
                "FSTRING opcode mismatch"
            );
            assert_eq!(
                opcode_class.getattr("CALL").unwrap().extract::<i32>().unwrap(),
                IROpCode::Call as i32,
                "CALL opcode mismatch"
            );
        });
    }

    /// Test TailPositionGuard RAII pattern
    #[test]
    fn test_tail_position_guard() {
        let state = Mutex::new(SemanticState {
            in_tail_position: false,
            current_function: None,
        });

        // Save initial state
        assert!(!state.lock().unwrap().in_tail_position);

        // Modify state and use guard
        state.lock().unwrap().in_tail_position = true;
        {
            let _guard = TailPositionGuard::new(&state);
            // Inside guard, state is saved
            // Modify to false
            state.lock().unwrap().in_tail_position = false;
            assert!(!state.lock().unwrap().in_tail_position);
        }
        // After guard drop, state should be restored to true
        assert!(state.lock().unwrap().in_tail_position);
    }

    /// Test SemanticState initialization
    #[test]
    fn test_semantic_state_default() {
        let state = SemanticState::default();
        assert!(!state.in_tail_position);
        assert!(state.current_function.is_none());
    }
}
