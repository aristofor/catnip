// FILE: catnip_rs/src/core/registry/execution.rs
//! Core execution functions: exec_stmt and resolve_ident
//!
//! This module contains the heart of the Catnip execution engine.

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use crate::types::catnip;

const OP_AND: &str = "and";
const OP_OR: &str = "or";

impl Registry {
    /// Execute a statement - core dispatch function
    ///
    /// This is the main execution entry point that handles:
    /// - Op nodes: look up and call registered operations
    /// - Ref nodes: resolve identifiers
    /// - Broadcast nodes: handle broadcasting
    /// - Literals: return as-is
    pub(crate) fn exec_stmt_impl(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let stmt_bound = stmt.bind(py);

        // Get the type of stmt for dispatch
        let stmt_type = stmt_bound.get_type();
        let type_name = stmt_type.name()?;

        // Handle Broadcast nodes first (before Op check)
        if type_name == catnip::BROADCAST {
            return self.handle_broadcast(py, stmt);
        }

        // Handle Op nodes (most common case)
        if type_name == catnip::OP {
            return self.exec_op(py, stmt_bound);
        }

        // Handle Ref nodes (identifier references)
        if type_name == catnip::REF {
            let ident: String = stmt_bound.getattr("ident")?.extract()?;

            // Try to resolve and enrich errors with position info if needed
            match self.resolve_ident_impl(py, &ident, true)? {
                Some(value) => return Ok(value),
                None => {
                    // Should not happen if check=true, but handle gracefully
                    return Err(PyErr::new::<pyo3::exceptions::PyNameError, _>(format!(
                        "name '{}' is not defined",
                        ident
                    )));
                }
            }
        }

        // Default: return literal as-is
        Ok(stmt)
    }

    /// Execute an Op node
    fn exec_op(&self, py: Python<'_>, op: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Extract ident, args, kwargs from Op
        let ident: i32 = op.getattr("ident")?.extract()?;
        let args = op.getattr("args")?;
        let kwargs = op.getattr("kwargs")?;

        // Convert to proper types
        // Note: args is always a tuple, but kwargs can be dict
        let args_tuple = args.cast::<PyTuple>()?;
        let kwargs_dict = &kwargs; // Keep as PyAny to support PyDict

        // Fast path for common arithmetic operations (binary, 2 args)
        // Inline evaluation + operation to avoid dispatch overhead
        let op_cache = &self.opcodes;
        if args_tuple.len() == 2 && kwargs_dict.is_none() {
            let arg0 = args_tuple.get_item(0)?.unbind();
            let arg1 = args_tuple.get_item(1)?.unbind();

            // Evaluate both arguments
            let val0 = self.exec_stmt_impl(py, arg0)?;
            let val1 = self.exec_stmt_impl(py, arg1)?;

            // Fast arithmetic dispatch
            if ident == op_cache.add {
                return val0
                    .bind(py)
                    .call_method1("__add__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.sub {
                return val0
                    .bind(py)
                    .call_method1("__sub__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.mul {
                return val0
                    .bind(py)
                    .call_method1("__mul__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.truediv {
                return val0
                    .bind(py)
                    .call_method1("__truediv__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.lt {
                return val0
                    .bind(py)
                    .call_method1("__lt__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.gt {
                return val0
                    .bind(py)
                    .call_method1("__gt__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.le {
                return val0
                    .bind(py)
                    .call_method1("__le__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.ge {
                return val0
                    .bind(py)
                    .call_method1("__ge__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.eq {
                return val0
                    .bind(py)
                    .call_method1("__eq__", (val1,))
                    .map(|r| r.unbind());
            } else if ident == op_cache.ne {
                return val0
                    .bind(py)
                    .call_method1("__ne__", (val1,))
                    .map(|r| r.unbind());
            }
        }

        // Special case: CALL needs the node for tail-call detection
        if ident == op_cache.call {
            return self.op_call(py, args_tuple, Some(op.clone().unbind()));
        }

        // Direct dispatch for Rust-implemented operations
        if let Some(result) = self.try_rust_dispatch(py, ident, args_tuple)? {
            return Ok(result);
        }

        // Look up the operation function (with caching)
        let func = if self.cache_enabled {
            // Check cache first (short borrow)
            let cached = self.op_cache.borrow().get(&ident).map(|f| f.clone_ref(py));
            if let Some(func) = cached {
                func
            } else {
                // Cache miss - look up and cache
                let func = self.lookup_operation(py, ident)?;
                self.op_cache.borrow_mut().insert(ident, func.clone_ref(py));
                func
            }
        } else {
            self.lookup_operation(py, ident)?
        };

        // Check if this is a control flow operation
        let is_control_flow = self.control_flow_ops.contains(&ident);

        // Prepare arguments
        if is_control_flow {
            // Control flow ops get unevaluated args
            // Convert kwargs (PyDict) to a new PyDict
            let kwargs_dict_clone = PyDict::new(py);

            // Copy all items from original kwargs
            let items = kwargs_dict.call_method0("items")?;
            for item in items.try_iter()? {
                let item = item?;
                let key = item.get_item(0)?;
                let value = item.get_item(1)?;
                kwargs_dict_clone.set_item(key, value)?;
            }

            // Special case: CALL needs the node passed as _node kwarg
            let opcode_module = py.import("catnip.semantic.opcode")?;
            let opcode_class = opcode_module.getattr("OpCode")?;
            let call_value: i32 = opcode_class.getattr("CALL")?.extract()?;

            if ident == call_value {
                kwargs_dict_clone.set_item("_node", op)?;
            }

            // Call with unevaluated args
            func.call(py, args_tuple, Some(&kwargs_dict_clone))
        } else {
            // Normal operations: evaluate all arguments
            let mut eval_args_list = Vec::new();
            for arg in args_tuple.iter() {
                let evaluated = self.exec_stmt_impl(py, arg.unbind())?;
                eval_args_list.push(evaluated);
            }

            // Create a new PyDict for evaluated kwargs
            let eval_kwargs_dict = PyDict::new(py);

            // Iterate over kwargs.items() to get key-value pairs
            // Use .call_method0("items") to work with PyDict
            let items = kwargs_dict.call_method0("items")?;
            for item in items.try_iter()? {
                let item = item?;
                let key = item.get_item(0)?;
                let value = item.get_item(1)?;
                let evaluated = self.exec_stmt_impl(py, value.unbind())?;
                eval_kwargs_dict.set_item(key, evaluated)?;
            }

            let eval_args_tuple = PyTuple::new(py, &eval_args_list)?;

            // Call the operation function
            func.call(py, eval_args_tuple, Some(&eval_kwargs_dict))
        }
    }

    /// Try to dispatch to a Rust-implemented operation
    /// Returns Some(result) if handled, None if not implemented in Rust
    fn try_rust_dispatch(
        &self,
        py: Python<'_>,
        opcode: i32,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Option<Py<PyAny>>> {
        // Use cached OpCode values for fast comparison
        let op = &self.opcodes;

        // Match against Rust-implemented operations
        // Unary operations (1 arg)
        if opcode == op.neg {
            if args.len() >= 1 {
                return Ok(Some(self.op_neg(py, args.get_item(0)?.unbind())?));
            }
        } else if opcode == op.pos {
            if args.len() >= 1 {
                return Ok(Some(self.op_pos(py, args.get_item(0)?.unbind())?));
            }
        } else if opcode == op.bnot {
            if args.len() >= 1 {
                return Ok(Some(self.op_inv(py, args.get_item(0)?.unbind())?));
            }
        }
        // Binary operations (variadic)
        else if opcode == op.add {
            return Ok(Some(self.op_add(py, args)?));
        } else if opcode == op.sub {
            return Ok(Some(self.op_sub(py, args)?));
        } else if opcode == op.mul {
            return Ok(Some(self.op_mul(py, args)?));
        } else if opcode == op.truediv {
            return Ok(Some(self.op_truediv(py, args)?));
        } else if opcode == op.floordiv {
            return Ok(Some(self.op_floordiv(py, args)?));
        } else if opcode == op.mod_ {
            return Ok(Some(self.op_mod(py, args)?));
        } else if opcode == op.pow {
            return Ok(Some(self.op_pow(py, args)?));
        }
        // Logical operations
        else if opcode == op.not {
            if args.len() >= 1 {
                return Ok(Some(self.op_bool_not(py, args.get_item(0)?.unbind())?));
            }
        } else if opcode == op.or {
            return Ok(Some(self.op_bool_or(py, args)?));
        } else if opcode == op.and {
            return Ok(Some(self.op_bool_and(py, args)?));
        } else if opcode == op.lt {
            return Ok(Some(self.op_lt(py, args)?));
        } else if opcode == op.le {
            return Ok(Some(self.op_le(py, args)?));
        } else if opcode == op.gt {
            return Ok(Some(self.op_gt(py, args)?));
        } else if opcode == op.ge {
            return Ok(Some(self.op_ge(py, args)?));
        } else if opcode == op.eq {
            return Ok(Some(self.op_eq(py, args)?));
        } else if opcode == op.ne {
            return Ok(Some(self.op_ne(py, args)?));
        }
        // Bitwise operations
        else if opcode == op.bor {
            return Ok(Some(self.op_bit_or(py, args)?));
        } else if opcode == op.bxor {
            return Ok(Some(self.op_bit_xor(py, args)?));
        } else if opcode == op.band {
            return Ok(Some(self.op_bit_and(py, args)?));
        } else if opcode == op.lshift {
            return Ok(Some(self.op_lshift(py, args)?));
        } else if opcode == op.rshift {
            return Ok(Some(self.op_rshift(py, args)?));
        }
        // Stack operations
        else if opcode == op.push {
            if args.len() >= 1 {
                self.op_push(py, args.get_item(0)?.unbind())?;
                return Ok(Some(py.None()));
            }
        } else if opcode == op.push_peek {
            if args.len() >= 1 {
                return Ok(Some(self.op_push_peek(py, args.get_item(0)?.unbind())?));
            }
        } else if opcode == op.pop {
            return Ok(Some(self.op_pop(py)?));
        }
        // Literal operations
        else if opcode == op.list_literal {
            return Ok(Some(self.op_list_literal(py, args)?));
        } else if opcode == op.tuple_literal {
            return Ok(Some(self.op_tuple_literal(py, args)?));
        } else if opcode == op.set_literal {
            return Ok(Some(self.op_set_literal(py, args)?));
        } else if opcode == op.dict_literal {
            return Ok(Some(self.op_dict_literal(py, args)?));
        } else if opcode == op.fstring {
            return Ok(Some(self.op_fstring(py, args)?));
        }
        // Access operations
        else if opcode == op.getattr {
            return Ok(Some(self.op_getattr(py, args)?));
        } else if opcode == op.getitem {
            return Ok(Some(self.op_getitem(py, args)?));
        } else if opcode == op.setattr {
            return Ok(Some(self.op_setattr(py, args)?));
        } else if opcode == op.setitem {
            return Ok(Some(self.op_setitem(py, args)?));
        } else if opcode == op.slice {
            return Ok(Some(self.op_slice(py, args)?));
        }
        // Control flow operations
        else if opcode == op.set_locals {
            return Ok(Some(self.op_set_locals(py, args)?));
        } else if opcode == op.op_for {
            return Ok(Some(self.op_for(py, args)?));
        } else if opcode == op.op_while {
            return Ok(Some(self.op_while(py, args)?));
        } else if opcode == op.op_block {
            return Ok(Some(self.op_block(py, args)?));
        } else if opcode == op.op_if {
            return Ok(Some(self.op_if(py, args)?));
        } else if opcode == op.op_return {
            return Ok(Some(self.op_return(py, args)?));
        } else if opcode == op.op_break {
            return Ok(Some(self.op_break(py, args)?));
        } else if opcode == op.op_continue {
            return Ok(Some(self.op_continue(py, args)?));
        }
        // Pattern matching
        else if opcode == op.op_match {
            return Ok(Some(self.op_match(py, args)?));
        }
        // ND operations
        else if opcode == op.nd_empty_topos {
            return Ok(Some(self.op_nd_empty_topos(py, args)?));
        } else if opcode == op.nd_recursion {
            return Ok(Some(self.op_nd_recursion(py, args)?));
        } else if opcode == op.nd_map {
            return Ok(Some(self.op_nd_map(py, args)?));
        }
        // Struct
        else if opcode == op.op_struct {
            return Ok(Some(self.op_struct(py, args)?));
        }
        // Trait
        else if opcode == op.trait_def {
            return Ok(Some(self.op_trait_def(py, args)?));
        }

        // Not implemented in Rust, fall back to Python
        // Note: OP_LAMBDA uses special bound_self signature, handled via Python fallback
        Ok(None)
    }

    /// Look up an operation by its OpCode
    fn lookup_operation(&self, py: Python<'_>, ident: i32) -> PyResult<Py<PyAny>> {
        // Use .get() method instead of __getitem__ to return None if key doesn't exist
        // This works with PyDict
        let internals_bound = self.internals.bind(py);
        let result = internals_bound.call_method1("get", (ident,))?;

        if result.is_none() {
            Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Unknown operation: {}",
                ident
            )))
        } else {
            Ok(result.unbind())
        }
    }

    /// Resolve an identifier from locals/globals
    pub(crate) fn resolve_ident_impl(
        &self,
        py: Python<'_>,
        ident: &str,
        check: bool,
    ) -> PyResult<Option<Py<PyAny>>> {
        // Special case: '_' returns last result
        if ident == "_" {
            let ctx = self.ctx.bind(py);
            let result = ctx.getattr("result")?;
            return Ok(Some(result.unbind()));
        }

        // Try locals first
        let ctx = self.ctx.bind(py);

        // Access context.locals (should be a Scope object)
        if let Ok(locals) = ctx.getattr("locals") {
            // Check if the key exists first (to handle None values correctly)
            if let Ok(exists) = locals.call_method1("contains", (ident,)) {
                if exists.is_truthy()? {
                    // Key exists, get the value (even if it's None)
                    if let Ok(value) = locals.call_method1("get", (ident,)) {
                        return Ok(Some(value.unbind()));
                    }
                }
            }
        }

        // Try globals
        if let Ok(globals) = ctx.getattr("globals") {
            if let Ok(globals_dict) = globals.cast::<PyDict>() {
                if let Some(value) = globals_dict.get_item(ident)? {
                    return Ok(Some(value.unbind()));
                }
            }
        }

        // Not found
        if check {
            // Try to suggest a similar name
            let suggestion = self.suggest_name(py, ident)?;
            let error_msg = if let Some(sugg) = suggestion {
                format!("name '{}' is not defined. Did you mean '{}'?", ident, sugg)
            } else {
                format!("name '{}' is not defined", ident)
            };

            // Use CatnipNameError for better error handling
            let exc_module = py.import("catnip.exc")?;
            let catnip_name_error = exc_module.getattr("CatnipNameError")?;
            Err(PyErr::from_value(catnip_name_error.call1((error_msg,))?))
        } else {
            Ok(None)
        }
    }

    /// Suggest a similar name for an undefined identifier
    fn suggest_name(&self, py: Python<'_>, ident: &str) -> PyResult<Option<String>> {
        // Import suggest_name from catnip.suggest
        let suggest_module = py.import("catnip.suggest")?;
        let suggest_fn = suggest_module.getattr("suggest_name")?;

        // Get all available names (locals + globals)
        let ctx = self.ctx.bind(py);
        let mut available = Vec::new();

        // Add locals (Scope has items() method, returns list of (key, value) tuples)
        if let Ok(locals) = ctx.getattr("locals") {
            if let Ok(items) = locals.call_method0("items") {
                if let Ok(items_iter) = items.try_iter() {
                    for item in items_iter {
                        if let Ok(i) = item {
                            // item is a (key, value) tuple
                            if let Ok(tuple) = i.cast::<PyTuple>() {
                                if let Ok(key) = tuple.get_item(0) {
                                    if let Ok(name) = key.extract::<String>() {
                                        available.push(name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Add globals (dict has keys() method)
        if let Ok(globals) = ctx.getattr("globals") {
            if let Ok(globals_dict) = globals.cast::<PyDict>() {
                for key in globals_dict.keys() {
                    if let Ok(name) = key.extract::<String>() {
                        available.push(name);
                    }
                }
            }
        }

        // Call suggest_name(ident, available)
        let result = suggest_fn.call1((ident, available))?;

        // suggest_name returns a list of strings, extract the first one if any
        if let Ok(suggestions) = result.extract::<Vec<String>>() {
            if !suggestions.is_empty() {
                return Ok(Some(suggestions[0].clone()));
            }
        }
        Ok(None)
    }

    /// Handle Broadcast nodes
    pub fn handle_broadcast(
        &self,
        py: Python<'_>,
        broadcast_node: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        // Extract target, operator, operand from Broadcast node
        let node = broadcast_node.bind(py);
        let target = node.getattr("target")?;
        let operator = node.getattr("operator")?;
        let operand_opt = node.getattr("operand")?;
        let is_filter: bool = node.getattr("is_filter")?.extract()?;

        // Evaluate target
        let eval_target = self.exec_stmt_impl(py, target.unbind())?;

        // Check if operator is an ND operation (Op with ND opcode)
        let op_type = operator.get_type();
        if op_type.name()? == "Op" {
            let op_ident: i32 = operator.getattr("ident")?.extract()?;

            if op_ident == self.opcodes.nd_recursion {
                // Extract lambda from operator.args[0]
                // ND ops are created as: (lambda/func, None)
                let op_args = operator.getattr("args")?;
                let op_args_tuple = op_args.cast::<PyTuple>()?;
                let lambda_node = op_args_tuple.get_item(0)?;
                return self.broadcast_nd_recursion(py, eval_target, lambda_node.unbind());
            } else if op_ident == self.opcodes.nd_map {
                // Extract func from operator.args[0]
                // ND ops are created as: (func, None)
                let op_args = operator.getattr("args")?;
                let op_args_tuple = op_args.cast::<PyTuple>()?;
                let func_node = op_args_tuple.get_item(0)?;
                return self.broadcast_nd_map(py, eval_target, func_node.unbind());
            }
        }

        // Evaluate operand if present
        let eval_operand = if operand_opt.is_none() {
            py.None()
        } else {
            self.exec_stmt_impl(py, operand_opt.unbind())?
        };

        // Apply broadcasting
        self.apply_broadcast(py, eval_target, operator.unbind(), eval_operand, is_filter)
    }

    /// Apply broadcasting operation
    ///
    /// Uses Rust implementation from broadcast module.
    /// Replaces deprecated Cython RegistryCore._apply_broadcast().
    pub(crate) fn apply_broadcast(
        &self,
        py: Python<'_>,
        target: Py<PyAny>,
        operator: Py<PyAny>,
        operand: Py<PyAny>,
        is_filter: bool,
    ) -> PyResult<Py<PyAny>> {
        use crate::core::registry::broadcast;

        let target_bound = target.bind(py);
        let operator_bound = operator.bind(py);

        // Check if operand is None
        let operand_opt = if operand.is_none(py) {
            None
        } else {
            Some(operand.bind(py))
        };

        // Get context globals
        let ctx = self.ctx.bind(py);
        let ctx_globals = ctx.getattr("globals")?;

        // CASE 1: Filter mode (.[if condition])
        if is_filter {
            // SIMD fast path pour filtres numériques
            if let Ok(op_str) = operator_bound.cast::<pyo3::types::PyString>() {
                if let Some(operand_ref) = operand_opt {
                    if let Some(result) = broadcast::simd::try_simd_filter(
                        py,
                        target_bound,
                        op_str.to_str().unwrap_or(""),
                        operand_ref,
                    ) {
                        return result;
                    }
                }
            }

            // Build condition function from operator and operand
            // Create exec_func closure that calls self.exec_stmt_impl
            let exec_func_closure = |stmt: &Bound<'_, PyAny>| -> PyResult<Py<PyAny>> {
                self.exec_stmt_impl(py, stmt.clone().unbind())
            };

            let condition_func = self.make_condition_func_internal(
                py,
                operator_bound,
                operand_opt,
                exec_func_closure,
            )?;
            return broadcast::filter_conditional(py, target_bound, condition_func.bind(py));
        }

        // CASE 2: Boolean mask indexing (.[mask])
        if operand_opt.is_none() {
            let is_string = operator_bound.cast::<pyo3::types::PyString>().is_ok();
            let is_callable = operator_bound.is_callable();

            if !is_string && !is_callable {
                // Evaluate operator to see if it's a boolean mask
                let mask = if operator_bound.is_instance_of::<pyo3::types::PyList>()
                    || operator_bound.is_instance_of::<pyo3::types::PyTuple>()
                {
                    operator_bound.clone()
                } else {
                    self.exec_stmt_impl(py, operator_bound.clone().unbind())?
                        .bind(py)
                        .clone()
                };

                if broadcast::is_boolean_mask(py, &mask)? {
                    return broadcast::filter_by_mask(py, target_bound, &mask);
                }

                if mask.is_instance_of::<pyo3::types::PyList>()
                    || mask.is_instance_of::<pyo3::types::PyTuple>()
                {
                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "Mask must be a list or tuple of booleans, got {} with non-boolean elements",
                        mask.get_type().name()?
                    )));
                }
            }
        }

        // CASE 3: Map mode (normal broadcast)
        if operator_bound.is_callable() {
            // Check if function is marked as pure (has is_pure attribute)
            if let Ok(true) = operator_bound
                .getattr("is_pure")
                .and_then(|attr| attr.is_truthy())
            {
                // Add function name to context.pure_functions
                if let Ok(func_name) = operator_bound
                    .getattr("__name__")
                    .and_then(|n| n.extract::<String>())
                {
                    let ctx_bound = self.ctx.bind(py);
                    if let Ok(pure_funcs) = ctx_bound.getattr("pure_functions") {
                        let _ = pure_funcs.call_method1("add", (func_name,));
                    }
                }
            }
            return broadcast::broadcast_map(py, target_bound, operator_bound);
        }

        // Handle string operators
        if let Ok(op_str) = operator_bound.cast::<pyo3::types::PyString>() {
            let op_str_val = op_str.to_str()?;
            return self.broadcast_string_op_internal(
                py,
                target_bound,
                op_str_val,
                operand_opt,
                &ctx_globals,
            );
        }

        // If operator is an expression node, evaluate it first
        let operator_value = self.exec_stmt_impl(py, operator_bound.clone().unbind())?;
        let operator_value_bound = operator_value.bind(py);
        if operator_value_bound.is_callable() {
            // Check if function is marked as pure (has is_pure attribute)
            if let Ok(true) = operator_value_bound
                .getattr("is_pure")
                .and_then(|attr| attr.is_truthy())
            {
                // Add function name to context.pure_functions
                if let Ok(func_name) = operator_value_bound
                    .getattr("__name__")
                    .and_then(|n| n.extract::<String>())
                {
                    let ctx_bound = self.ctx.bind(py);
                    if let Ok(pure_funcs) = ctx_bound.getattr("pure_functions") {
                        let _ = pure_funcs.call_method1("add", (func_name,));
                    }
                }
            }
            return broadcast::broadcast_map(py, target_bound, operator_value_bound);
        }

        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "Invalid broadcast operator: {}",
            operator_bound
        )))
    }

    /// Internal helper to create condition function (avoids Python callable overhead)
    fn make_condition_func_internal<F>(
        &self,
        py: Python<'_>,
        operator: &Bound<'_, PyAny>,
        operand: Option<&Bound<'_, PyAny>>,
        exec_func: F,
    ) -> PyResult<Py<PyAny>>
    where
        F: Fn(&Bound<'_, PyAny>) -> PyResult<Py<PyAny>>,
    {
        // If already callable, return it
        if operator.is_callable() {
            return Ok(operator.clone().unbind());
        }

        // Handle string operators - delegate to broadcast module's make_condition_func
        // but we need to pass exec_func as Python callable
        // For now, just handle common cases directly
        if let Ok(op_str) = operator.cast::<pyo3::types::PyString>() {
            let op_str_val = op_str.to_str()?;
            // Binary operators
            if let Some(op_func) = self.get_binary_op(py, op_str_val)? {
                let operand = operand.ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(format!(
                        "Filter operator '{}' requires an operand",
                        op_str_val
                    ))
                })?;
                // Create a proper closure using a wrapper that captures values
                let op_func_bound = op_func.bind(py);
                let locals = PyDict::new(py);
                locals.set_item("__op_func__", op_func_bound)?;
                locals.set_item("__operand__", operand)?;
                return py
                    .eval(
                        c"lambda x: __op_func__(x, __operand__)",
                        Some(&locals),
                        Some(&locals),
                    )
                    .map(|o| o.unbind());
            }

            // Unary operators
            if let Some(op_func) = self.get_unary_op(py, op_str_val)? {
                return Ok(op_func);
            }

            // Logical operators
            if op_str_val == OP_AND {
                let operand = operand.ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(
                        "Filter operator 'and' requires an operand",
                    )
                })?;
                let locals = PyDict::new(py);
                locals.set_item("__operand__", operand)?;
                return py
                    .eval(c"lambda x: x and __operand__", Some(&locals), Some(&locals))
                    .map(|o| o.unbind());
            }

            if op_str_val == OP_OR {
                let operand = operand.ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(
                        "Filter operator 'or' requires an operand",
                    )
                })?;
                let locals = PyDict::new(py);
                locals.set_item("__operand__", operand)?;
                return py
                    .eval(c"lambda x: x or __operand__", Some(&locals), Some(&locals))
                    .map(|o| o.unbind());
            }

            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Unknown filter operator: {}",
                op_str_val
            )));
        }

        // If operator is an expression node, evaluate it
        let operator_value = exec_func(operator)?;
        let operator_value_bound = operator_value.bind(py);
        if operator_value_bound.is_callable() {
            return Ok(operator_value);
        }

        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "Invalid filter condition: {}",
            operator
        )))
    }

    /// Internal helper for broadcast_string_op
    fn broadcast_string_op_internal(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        operator: &str,
        operand: Option<&Bound<'_, PyAny>>,
        ctx_globals: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        use crate::core::registry::broadcast;

        // Handle ND operators (~~, ~>) via shared broadcast_nd_* methods
        if operator == "~~" {
            let operand_bound = operand.ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(
                    "ND operator '~~' requires an operand (lambda or function)",
                )
            })?;
            return self.broadcast_nd_recursion(
                py,
                target.clone().unbind(),
                operand_bound.clone().unbind(),
            );
        }

        if operator == "~>" {
            let operand_bound = operand.ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(
                    "ND operator '~>' requires an operand (function)",
                )
            })?;
            return self.broadcast_nd_map(
                py,
                target.clone().unbind(),
                operand_bound.clone().unbind(),
            );
        }

        // Check if operator is a function name in globals
        if let Ok(globals_dict) = ctx_globals.cast::<PyDict>() {
            if let Ok(Some(func)) = globals_dict.get_item(operator) {
                if func.is_callable() {
                    // Check if function is marked as pure (has is_pure attribute)
                    if let Ok(true) = func.getattr("is_pure").and_then(|attr| attr.is_truthy()) {
                        // Add function name to context.pure_functions
                        if let Ok(func_name) =
                            func.getattr("__name__").and_then(|n| n.extract::<String>())
                        {
                            let ctx_bound = self.ctx.bind(py);
                            if let Ok(pure_funcs) = ctx_bound.getattr("pure_functions") {
                                let _ = pure_funcs.call_method1("add", (func_name,));
                            }
                        }
                    }
                    return broadcast::broadcast_map(py, target, &func);
                }
            }
        }

        // SIMD fast path pour listes numériques homogènes
        if let Some(operand_bound) = operand {
            if let Some(result) = broadcast::simd::try_simd_map(py, target, operator, operand_bound)
            {
                return result;
            }
        }

        // Binary operators
        if let Some(op_func) = self.get_binary_op(py, operator)? {
            let operand = operand.ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "Binary operator '{}' requires an operand",
                    operator
                ))
            })?;

            // Element-wise for list/tuple operands
            if operand.is_instance_of::<pyo3::types::PyList>()
                || operand.is_instance_of::<pyo3::types::PyTuple>()
            {
                if !target.is_instance_of::<pyo3::types::PyList>()
                    && !target.is_instance_of::<pyo3::types::PyTuple>()
                {
                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "Cannot broadcast {} with {}",
                        target.get_type().name()?,
                        operand.get_type().name()?
                    )));
                }
                let target_len = target.len()?;
                let operand_len = operand.len()?;
                if target_len != operand_len {
                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                        "Broadcast size mismatch: {} vs {}",
                        target_len, operand_len
                    )));
                }

                let target_is_tuple = target.is_instance_of::<pyo3::types::PyTuple>();
                let result_list = pyo3::types::PyList::empty(py);

                for (t, o) in target.try_iter()?.zip(operand.try_iter()?) {
                    let res = op_func.call1(py, (t?, o?))?;
                    result_list.append(res)?;
                }

                return if target_is_tuple {
                    Ok(pyo3::types::PyTuple::new(py, &result_list)?
                        .into_any()
                        .unbind())
                } else {
                    Ok(result_list.into())
                };
            }

            // Scalar operand
            let locals = PyDict::new(py);
            locals.set_item("__op_func__", op_func.bind(py))?;
            locals.set_item("__operand__", operand)?;
            let lambda = py.eval(
                c"lambda x: __op_func__(x, __operand__)",
                Some(&locals),
                Some(&locals),
            )?;
            return broadcast::broadcast_map(py, target, &lambda);
        }

        // Unary operators
        if let Some(op_func) = self.get_unary_op(py, operator)? {
            return broadcast::broadcast_map(py, target, op_func.bind(py));
        }

        // Logical operators
        if operator == OP_AND {
            let operand = operand.ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("Operator 'and' requires an operand")
            })?;
            let locals = PyDict::new(py);
            locals.set_item("__operand__", operand)?;
            let lambda = py.eval(c"lambda x: x and __operand__", Some(&locals), Some(&locals))?;
            return broadcast::broadcast_map(py, target, &lambda);
        }

        if operator == OP_OR {
            let operand = operand.ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("Operator 'or' requires an operand")
            })?;
            let locals = PyDict::new(py);
            locals.set_item("__operand__", operand)?;
            let lambda = py.eval(c"lambda x: x or __operand__", Some(&locals), Some(&locals))?;
            return broadcast::broadcast_map(py, target, &lambda);
        }

        Err(pyo3::exceptions::PyValueError::new_err(format!(
            "Unknown broadcast operator: {}",
            operator
        )))
    }

    /// Get binary operator function
    fn get_binary_op(&self, py: Python<'_>, op: &str) -> PyResult<Option<Py<PyAny>>> {
        let cached = match op {
            "+" => Some(self.operator_cache.add.clone_ref(py)),
            "-" => Some(self.operator_cache.sub.clone_ref(py)),
            "*" => Some(self.operator_cache.mul.clone_ref(py)),
            "/" => Some(self.operator_cache.truediv.clone_ref(py)),
            "//" => Some(self.operator_cache.floordiv.clone_ref(py)),
            "%" => Some(self.operator_cache.mod_.clone_ref(py)),
            "**" => Some(self.operator_cache.pow.clone_ref(py)),
            "<" => Some(self.operator_cache.lt.clone_ref(py)),
            "<=" => Some(self.operator_cache.le.clone_ref(py)),
            ">" => Some(self.operator_cache.gt.clone_ref(py)),
            ">=" => Some(self.operator_cache.ge.clone_ref(py)),
            "==" => Some(self.operator_cache.eq.clone_ref(py)),
            "!=" => Some(self.operator_cache.ne.clone_ref(py)),
            "&" => Some(self.operator_cache.and_.clone_ref(py)),
            "|" => Some(self.operator_cache.or_.clone_ref(py)),
            "^" => Some(self.operator_cache.xor.clone_ref(py)),
            "<<" => Some(self.operator_cache.lshift.clone_ref(py)),
            ">>" => Some(self.operator_cache.rshift.clone_ref(py)),
            _ => None,
        };

        Ok(cached)
    }

    /// Get unary operator function
    fn get_unary_op(&self, py: Python<'_>, op: &str) -> PyResult<Option<Py<PyAny>>> {
        match op {
            "abs" => Ok(Some(self.operator_cache.abs.clone_ref(py))),
            "-" => Ok(Some(self.operator_cache.neg.clone_ref(py))),
            "+" => Ok(Some(self.operator_cache.pos.clone_ref(py))),
            "~" => Ok(Some(self.operator_cache.invert.clone_ref(py))),
            "not" => Ok(Some(self.operator_cache.not_.clone_ref(py))),
            _ => Ok(None),
        }
    }

    /// Handle ND recursion broadcasting
    fn broadcast_nd_recursion(
        &self,
        py: Python<'_>,
        target: Py<PyAny>,
        lambda_node: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        // Track pure functions
        let lambda_bound = lambda_node.bind(py);
        if let Ok(true) = lambda_bound
            .getattr("is_pure")
            .and_then(|attr| attr.is_truthy())
        {
            if let Ok(func_name) = lambda_bound
                .getattr("__name__")
                .and_then(|n| n.extract::<String>())
            {
                let ctx_bound = self.ctx.bind(py);
                if let Ok(pure_funcs) = ctx_bound.getattr("pure_functions") {
                    let _ = pure_funcs.call_method1("add", (func_name,));
                }
            }
        }

        // Broadcast: apply ND recursion to each element
        let target_bound = target.bind(py);

        // Check if target is a tuple (preserve type)
        let is_tuple = target_bound.is_instance_of::<pyo3::types::PyTuple>();

        // Collect results
        let result_list = pyo3::types::PyList::empty(py);

        for elem in target_bound.try_iter()? {
            let elem = elem?;
            // Call op_nd_recursion(element, lambda)
            let lambda_bound = lambda_node.bind(py);
            let args = pyo3::types::PyTuple::new(py, vec![&elem, &lambda_bound])?;
            let res = self.op_nd_recursion(py, &args)?;
            result_list.append(res)?;
        }

        // Convert back to tuple if needed
        if is_tuple {
            Ok(pyo3::types::PyTuple::new(py, result_list)?
                .into_any()
                .unbind())
        } else {
            Ok(result_list.into_any().unbind())
        }
    }

    /// Handle ND map broadcasting
    fn broadcast_nd_map(
        &self,
        py: Python<'_>,
        target: Py<PyAny>,
        func_node: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        // Track pure functions
        let func_bound = func_node.bind(py);
        if let Ok(true) = func_bound
            .getattr("is_pure")
            .and_then(|attr| attr.is_truthy())
        {
            if let Ok(func_name) = func_bound
                .getattr("__name__")
                .and_then(|n| n.extract::<String>())
            {
                let ctx_bound = self.ctx.bind(py);
                if let Ok(pure_funcs) = ctx_bound.getattr("pure_functions") {
                    let _ = pure_funcs.call_method1("add", (func_name,));
                }
            }
        }

        // Broadcast: apply ND map to each element
        let target_bound = target.bind(py);

        // Check if target is a tuple (preserve type)
        let is_tuple = target_bound.is_instance_of::<pyo3::types::PyTuple>();

        // Collect results
        let result_list = pyo3::types::PyList::empty(py);

        for elem in target_bound.try_iter()? {
            let elem = elem?;
            // Call op_nd_map(element, func)
            let func_bound = func_node.bind(py);
            let args = pyo3::types::PyTuple::new(py, vec![&elem, &func_bound])?;
            let res = self.op_nd_map(py, &args)?;
            result_list.append(res)?;
        }

        // Convert back to tuple if needed
        if is_tuple {
            Ok(pyo3::types::PyTuple::new(py, result_list)?
                .into_any()
                .unbind())
        } else {
            Ok(result_list.into_any().unbind())
        }
    }
}
