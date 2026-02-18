// FILE: catnip_rs/src/core/registry/control_flow.rs
//! Control flow operations: if, while, for, block, return, break, continue, set_locals

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::types::{catnip, exceptions};

impl Registry {
    /// Execute a for loop: for identifier in iterable { block }
    ///
    /// Args:
    ///     identifier: Variable name (Ref or string) or unpacking pattern (list)
    ///     iterable: Expression that evaluates to an iterable (unevaluated Op)
    ///     block: Block to execute for each iteration (unevaluated Op)
    ///
    /// Returns:
    ///     Value of the last iteration, or None
    pub(crate) fn op_for(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() < 3 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "for requires 3 arguments: identifier, iterable, block",
            ));
        }

        let identifier = args.get_item(0)?.unbind();
        let iterable_node = args.get_item(1)?.unbind();
        let block_node = args.get_item(2)?.unbind();

        // Evaluate the iterable
        let iterable_value = self.exec_stmt_impl(py, iterable_node)?;

        // Get context
        let ctx = self.ctx.bind(py);

        // Determine if scope is needed:
        // 1. Variable unpacking (complex patterns)
        // 2. Block body defines variables or contains nested control structures
        let var_is_simple = self.is_simple_identifier(py, &identifier);
        let block_needs_scope = !var_is_simple || self.stmt_needs_scope(py, &block_node);

        if block_needs_scope {
            ctx.call_method0("push_scope")?;
        }

        let mut result = py.None();

        // Execute loop
        let loop_result = (|| -> PyResult<Py<PyAny>> {
            // Get the iterator
            let iter = iterable_value.bind(py).try_iter()?;

            for item_result in iter {
                let item = item_result?;

                // Bind the loop variable
                self.bind_loop_variable(py, &identifier, &item)?;

                // Execute the block - handle Break/Continue exceptions
                match self.exec_stmt_impl(py, block_node.clone_ref(py)) {
                    Ok(value) => {
                        result = value;
                    }
                    Err(e) => {
                        // Check if it's a Break or Continue exception
                        if e.is_instance_of::<pyo3::exceptions::PyBaseException>(py) {
                            let exc_type = e.get_type(py);
                            let exc_type_name = exc_type.name()?;

                            if exc_type_name == exceptions::BREAK_LOOP {
                                // Break out of the loop
                                break;
                            } else if exc_type_name == exceptions::CONTINUE_LOOP {
                                // Continue to next iteration
                                continue;
                            }
                        }

                        // Not a Break/Continue, propagate the error
                        return Err(e);
                    }
                }
            }

            Ok(result)
        })();

        // Pop scope only if we pushed one
        if block_needs_scope {
            ctx.call_method0("pop_scope")?;
        }

        loop_result
    }

    /// Check if identifier is a simple variable (not a pattern)
    fn is_simple_identifier(&self, py: Python<'_>, identifier: &Py<PyAny>) -> bool {
        let bound = identifier.bind(py);
        let type_name = match bound.get_type().name() {
            Ok(name) => name,
            Err(_) => return false,
        };
        type_name == catnip::REF || type_name == catnip::LVALUE
    }

    /// Bind a loop variable (simple or unpacking) - supports nested patterns
    fn bind_loop_variable(
        &self,
        py: Python<'_>,
        identifier: &Py<PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        // Get context locals
        let ctx = self.ctx.bind(py);
        let locals = ctx.getattr("locals")?;

        // Fast path for simple identifiers (no unpacking)
        if self.is_simple_identifier(py, identifier) {
            let identifier_bound = identifier.bind(py);
            let type_name = identifier_bound.get_type().name()?;

            // Extract variable name
            let name = if type_name == catnip::REF {
                identifier_bound.getattr("ident")?.extract::<String>()?
            } else if type_name == catnip::LVALUE {
                identifier_bound.getattr("value")?.extract::<String>()?
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                    "Unexpected identifier type: {}",
                    type_name
                )));
            };

            let value_obj = value.clone().unbind();

            // Try direct Scope access (avoid Python call overhead)
            if let Ok(scope) = locals.extract::<pyo3::Bound<'_, crate::core::scope::Scope>>() {
                // Fast path: direct Rust Scope access
                scope.borrow_mut().set_catnip(py, name, value_obj);
                return Ok(());
            }

            // Fallback: Python method call (if locals is not a Rust Scope)
            locals.call_method1("_set", (name, value_obj))?;
            return Ok(());
        }

        // Slow path: complex patterns (unpacking)
        // Helper closure to assign a variable in locals
        let assign_var = |name: &str, val: &Py<PyAny>| -> PyResult<()> {
            locals.call_method1("_set", (name, val.clone_ref(py)))?;
            Ok(())
        };

        // Use pattern-matching unpacking (supports nested patterns and star operator)
        let value_obj = value.clone().unbind();
        self.unpack_pattern_recursive(py, identifier.bind(py), &value_obj, &assign_var)?;

        Ok(())
    }

    /// Extract variable names from a tuple/list of Ref objects for loop unpacking
    /// Execute a while loop: while condition { block }
    ///
    /// Args:
    ///     condition: Expression to evaluate before each iteration (unevaluated Op)
    ///     block: Block to execute while condition is true (unevaluated Op)
    ///
    /// Returns:
    ///     Value of the last iteration, or None
    pub(crate) fn op_while(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "while requires 2 arguments: condition, block",
            ));
        }

        let condition_node = args.get_item(0)?.unbind();
        let block_node = args.get_item(1)?.unbind();

        let mut result = py.None();

        loop {
            // Evaluate condition
            let condition_value = self.exec_stmt_impl(py, condition_node.clone_ref(py))?;
            let is_true = condition_value.bind(py).is_truthy()?;

            if !is_true {
                break;
            }

            // Execute block - handle Break/Continue exceptions
            match self.exec_stmt_impl(py, block_node.clone_ref(py)) {
                Ok(value) => {
                    result = value;
                }
                Err(e) => {
                    // Check if it's a BreakLoop or ContinueLoop exception
                    if e.is_instance_of::<pyo3::exceptions::PyBaseException>(py) {
                        let exc_type = e.get_type(py);
                        let exc_type_name = exc_type.name()?;

                        if exc_type_name == exceptions::BREAK_LOOP {
                            // Break out of the loop
                            break;
                        } else if exc_type_name == exceptions::CONTINUE_LOOP {
                            // Continue to next iteration
                            continue;
                        }
                    }

                    // Not a Break/Continue, propagate the error
                    return Err(e);
                }
            }
        }

        Ok(result)
    }

    /// Check if a single statement needs a scope
    ///
    /// Used to analyze loop bodies (single statement, could be a block)
    fn stmt_needs_scope(&self, py: Python<'_>, stmt: &Py<PyAny>) -> bool {
        let stmt_bound = stmt.bind(py);

        // Check if it's an Op node
        let stmt_type = match stmt_bound.get_type().name() {
            Ok(name) => name,
            Err(_) => return false,
        };

        if stmt_type != catnip::OP {
            return false; // Not an Op, probably a literal/ref
        }

        // Extract opcode
        let ident = match stmt_bound.getattr("ident").and_then(|i| i.extract::<i32>()) {
            Ok(id) => id,
            Err(_) => return false,
        };

        let op = &self.opcodes;

        // Check if this specific operation needs a scope
        if ident == op.set_locals {
            return true; // Variable definition
        }
        if ident == op.op_for || ident == op.op_while {
            return true; // Nested loop (will create its own scope if needed)
        }

        // If it's a block, analyze it recursively
        if ident == op.op_block {
            // Extract block args
            if let Ok(args) = stmt_bound.getattr("args") {
                if let Ok(args_tuple) = args.cast::<PyTuple>() {
                    return self.block_needs_scope(py, args_tuple);
                }
            }
        }

        false
    }

    /// Check if a block needs a new scope
    ///
    /// A block needs a scope if it:
    /// - Defines variables (SET_LOCALS)
    /// - Contains nested control structures (FOR, WHILE, BLOCK)
    /// - Contains function definitions (LAMBDA, FN_DEF)
    ///
    /// Simple blocks with just expressions/calls can skip scope creation.
    fn block_needs_scope(&self, _py: Python<'_>, args: &Bound<'_, PyTuple>) -> bool {
        let op = &self.opcodes;

        for stmt in args.iter() {
            // Check if stmt is an Op node
            if let Ok(stmt_type) = stmt.get_type().name() {
                if stmt_type != catnip::OP {
                    continue;
                }
            } else {
                continue;
            }

            // Extract opcode
            let ident = match stmt.getattr("ident").and_then(|i| i.extract::<i32>()) {
                Ok(id) => id,
                Err(_) => continue,
            };

            // Check if this operation requires a scope
            if ident == op.set_locals {
                return true; // Variable definition
            }
            if ident == op.op_for || ident == op.op_while || ident == op.op_block {
                return true; // Nested control structures
            }

            // Could also check for lambda/function definitions if needed
            // For now, we conservatively assume other ops don't need scope
        }

        false
    }

    /// Execute a block of statements: { stmt1; stmt2; ... }
    ///
    /// Conditionally creates a new scope based on static analysis.
    ///
    /// Args:
    ///     statements: Variable number of unevaluated statements
    ///
    /// Returns:
    ///     Value of the last statement, or None
    pub(crate) fn op_block(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        // Get context
        let ctx = self.ctx.bind(py);

        // Analyze block to determine if scope is needed
        let needs_scope = self.block_needs_scope(py, args);

        if needs_scope {
            ctx.call_method0("push_scope")?;
        }

        let mut result = py.None();

        // Execute block
        let block_result = (|| -> PyResult<Py<PyAny>> {
            // Execute each statement
            for stmt in args.iter() {
                result = self.exec_stmt_impl(py, stmt.unbind())?;
            }
            Ok(result)
        })();

        // Pop scope only if we pushed one
        if needs_scope {
            ctx.call_method0("pop_scope")?;
        }

        block_result
    }

    /// Set local variables: set_locals(names, value)
    ///
    /// Takes variable name(s) and a value, and assigns them in the local scope.
    /// Names can be:
    /// - A single string: "x"
    /// - A tuple of strings: ("x", "y")
    /// - A tuple containing a single name: ("x",)
    ///
    /// For backward compatibility with old tests: `set_locals("x", 10)`
    /// Standard format: `set_locals(("x",), 10)`
    /// Tuple unpacking: `set_locals(("x", "y"), (1, 2))`
    pub(crate) fn op_set_locals(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "set_locals requires at least 2 arguments: names, value",
            ));
        }

        // Get names and value nodes
        let names_node = args.get_item(0)?.unbind();
        let value_node = args.get_item(1)?.unbind();

        // Get optional explicit_unpack flag (3rd argument)
        let explicit_unpack = if args.len() >= 3 {
            args.get_item(2)?.extract::<bool>().unwrap_or(false)
        } else {
            false
        };

        // Evaluate the value first
        let mut value = self.exec_stmt_impl(py, value_node)?;

        // If explicit_unpack=true and value is a single-element sequence, unwrap it
        // BUT only if the pattern is also a single simple element (no star, no nested)
        // Example: (x,) = list(42) should give x = 42, not x = [42]
        // Counter-example: (a, *rest) = list(1) should give a=1, rest=[], NOT unwrap to a=1 (error)
        if explicit_unpack {
            // Check if pattern is a simple single-element tuple (no star patterns, no multi-element)
            let names_bound = names_node.bind(py);
            let mut is_simple_single_element = false;

            if let Ok(names_tuple) = names_bound.cast::<PyTuple>() {
                if names_tuple.len() == 1 {
                    let elem = names_tuple.get_item(0)?;
                    let mut is_simple = true;

                    // If elem is itself a tuple, check its structure
                    if let Ok(elem_tuple) = elem.cast::<PyTuple>() {
                        // Multi-element tuple: NOT a simple single element
                        if elem_tuple.len() > 1 {
                            is_simple = false;
                        } else if elem_tuple.len() == 2 {
                            // Check if it's a star pattern: ('*', 'name')
                            if let Ok(first) =
                                elem_tuple.get_item(0).and_then(|e| e.extract::<String>())
                            {
                                if first == "*" {
                                    is_simple = false; // Star pattern: don't unwrap
                                }
                            }
                        }
                    }

                    is_simple_single_element = is_simple;
                }
            }

            if is_simple_single_element {
                // Try to unwrap single-element sequences
                let value_bound = value.bind(py);
                if let Ok(iter) = value_bound.try_iter() {
                    let values: Result<Vec<_>, _> = iter.collect();
                    if let Ok(values_vec) = values {
                        if values_vec.len() == 1 {
                            // Single element: unwrap it
                            value = values_vec[0].clone().unbind();
                        }
                    }
                }
            }
        }

        // Extract names from the node structure (without evaluating Ref nodes)
        // Get context locals and globals
        let ctx = self.ctx.bind(py);
        let locals = ctx.getattr("locals")?;
        let globals = ctx.getattr("globals")?;

        // Check if we're at global scope (depth == 1)
        let depth: usize = locals.call_method0("depth")?.extract()?;
        let is_global_scope = depth == 1;

        // Helper to assign a single variable
        let assign_var = |name: &str, val: &Py<PyAny>| -> PyResult<()> {
            // Assign to locals
            locals.call_method1("_set", (name, val.clone_ref(py)))?;

            // Sync with globals only at global scope (depth == 1)
            if is_global_scope {
                globals.set_item(name, val.clone_ref(py))?;
            }
            Ok(())
        };

        // Use pattern-matching unpacking (supports nested patterns and star operator)
        self.unpack_pattern_recursive(py, names_node.bind(py), &value, &assign_var)?;

        // Return the value (for chaining assignments like x = y = 10)
        Ok(value)
    }

    /// Recursively unpack a pattern against a value (supports nested patterns and star operator)
    ///
    /// Examples:
    /// - `x = 5` → assigns x
    /// - `(x, y) = [1, 2]` → assigns x=1, y=2
    /// - `(x, (y, z)) = [1, [2, 3]]` → assigns x=1, y=2, z=3 (nested)
    /// - `(x, *rest, z) = [1, 2, 3, 4]` → assigns x=1, rest=[2, 3], z=4 (star)
    fn unpack_pattern_recursive<F>(
        &self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
        value: &Py<PyAny>,
        assign_var: &F,
    ) -> PyResult<()>
    where
        F: Fn(&str, &Py<PyAny>) -> PyResult<()>,
    {
        let pattern_type = pattern.get_type();
        let type_name = pattern_type.name()?;

        // Case 1: Single variable (Ref or Lvalue)
        if type_name == catnip::REF {
            let name: String = pattern.getattr("ident")?.extract()?;
            return assign_var(&name, value);
        }
        if type_name == catnip::LVALUE {
            let name: String = pattern.getattr("value")?.extract()?;
            return assign_var(&name, value);
        }

        // Case 2: Tuple pattern - unpack the value
        if let Ok(pattern_tuple) = pattern.cast::<PyTuple>() {
            // Unwrap single-element tuple: ((x, y),) → (x, y)
            if pattern_tuple.len() == 1 {
                let inner = pattern_tuple.get_item(0)?;
                // If inner is a tuple, unwrap pattern only (value stays as-is for nested unpacking)
                if inner.cast::<PyTuple>().is_ok() {
                    return self.unpack_pattern_recursive(py, &inner, value, assign_var);
                }
                // If inner is not a tuple, it's a simple pattern (Ref, Lvalue, or string)
                // Simple assignment: x = value (parser wrapped as (x,) = value)
                // DO NOT unwrap single-element lists/tuples - preserve the value as-is
                // This fixes:
                // - x = list(42) → x = [42] (not x = 42)
                // - x = tuple(42) → x = (42,) (not x = 42)
                // - f = (*rest) => rest; f(42) → rest = [42] (not rest = 42)
                return self.unpack_pattern_recursive(py, &inner, value, assign_var);
            }

            // Value must be iterable
            let value_bound = value.bind(py);
            let value_iter = match value_bound.try_iter() {
                Ok(iter) => iter,
                Err(_) => {
                    let exc_module = py.import("catnip.exc")?;
                    let catnip_type_error = exc_module.getattr("CatnipTypeError")?;
                    let type_name_str = value_bound.get_type().name()?;
                    return Err(PyErr::from_value(catnip_type_error.call1((format!(
                        "Cannot unpack non-iterable {}",
                        type_name_str
                    ),))?));
                }
            };

            let values: Vec<Py<PyAny>> = value_iter
                .map(|v| v.map(|x| x.unbind()))
                .collect::<PyResult<_>>()?;

            // Check for star pattern
            let (has_star, star_pos, star_name) = self.find_star_in_pattern(py, pattern_tuple)?;

            if has_star {
                // Star pattern: (a, *mid, b) = [1, 2, 3, 4]
                let before_count = star_pos;
                let after_count = pattern_tuple.len() - star_pos - 1;
                let min_values = before_count + after_count;

                if values.len() < min_values {
                    let exc_module = py.import("catnip.exc")?;
                    let catnip_runtime_error = exc_module.getattr("CatnipRuntimeError")?;
                    return Err(PyErr::from_value(catnip_runtime_error.call1((format!(
                        "Not enough values to unpack: expected at least {}, got {}",
                        min_values,
                        values.len()
                    ),))?));
                }

                // Unpack before star
                for i in 0..before_count {
                    let sub_pattern = pattern_tuple.get_item(i)?;
                    self.unpack_pattern_recursive(py, &sub_pattern, &values[i], assign_var)?;
                }

                // Assign star (middle part)
                let star_start = before_count;
                let star_end = values.len() - after_count;
                let star_values: Vec<Py<PyAny>> = values[star_start..star_end]
                    .iter()
                    .map(|v| v.clone_ref(py))
                    .collect();
                let py_list = pyo3::types::PyList::new(py, star_values)?;
                assign_var(&star_name, &py_list.into())?;

                // Unpack after star
                let after_start = values.len() - after_count;
                for i in 0..after_count {
                    let sub_pattern = pattern_tuple.get_item(star_pos + 1 + i)?;
                    self.unpack_pattern_recursive(
                        py,
                        &sub_pattern,
                        &values[after_start + i],
                        assign_var,
                    )?;
                }
            } else {
                // Regular pattern: (a, b, c) = [1, 2, 3]
                if values.len() != pattern_tuple.len() {
                    let exc_module = py.import("catnip.exc")?;
                    let catnip_runtime_error = exc_module.getattr("CatnipRuntimeError")?;
                    return Err(PyErr::from_value(catnip_runtime_error.call1((format!(
                        "Cannot unpack {} values into {} variables",
                        values.len(),
                        pattern_tuple.len()
                    ),))?));
                }

                // Recursively unpack each element
                for (i, sub_pattern) in pattern_tuple.iter().enumerate() {
                    self.unpack_pattern_recursive(py, &sub_pattern, &values[i], assign_var)?;
                }
            }

            return Ok(());
        }

        // Case 3: Already a string (fallback for simple names)
        if let Ok(name) = pattern.extract::<String>() {
            return assign_var(&name, value);
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "Cannot unpack pattern of type {}",
            type_name
        )))
    }

    /// Find star pattern in a tuple: (a, *mid, b) → (true, 1, "mid")
    fn find_star_in_pattern(
        &self,
        _py: Python<'_>,
        pattern_tuple: &Bound<'_, PyTuple>,
    ) -> PyResult<(bool, usize, String)> {
        for (i, item) in pattern_tuple.iter().enumerate() {
            // Star pattern is represented as a 2-tuple: ('*', 'name')
            if let Ok(item_tuple) = item.cast::<PyTuple>() {
                if item_tuple.len() == 2 {
                    if let (Ok(first), Ok(second)) = (
                        item_tuple.get_item(0)?.extract::<String>(),
                        item_tuple.get_item(1)?.extract::<String>(),
                    ) {
                        if first == "*" {
                            return Ok((true, i, second));
                        }
                    }
                }
            }
        }
        Ok((false, 0, String::new()))
    }

    /// Execute conditional branches: if/elif/else
    ///
    /// Args:
    ///     branches: Tuple of (condition, block) tuples for if/elif branches
    ///     else_block: Optional else block (unevaluated Op or None)
    ///
    /// Returns:
    ///     Value of the executed block, or None if no condition matched and no else
    pub(crate) fn op_if(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "if requires at least 1 argument: branches",
            ));
        }

        let branches = args.get_item(0)?;
        let else_block = if args.len() > 1 {
            Some(args.get_item(1)?.unbind())
        } else {
            None
        };

        // Try each branch in order
        let branches_iter = branches.try_iter()?;
        for branch_result in branches_iter {
            let branch = branch_result?;

            // Each branch is a (condition, block) tuple
            let branch_tuple = branch.cast::<PyTuple>()?;
            if branch_tuple.len() < 2 {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "Each branch must be a (condition, block) tuple",
                ));
            }

            let condition_node = branch_tuple.get_item(0)?.unbind();
            let block_node = branch_tuple.get_item(1)?.unbind();

            // Evaluate condition
            let cond_value = self.exec_stmt_impl(py, condition_node)?;
            let is_true = cond_value.bind(py).is_truthy()?;

            if is_true {
                // Execute this block and return
                let result = self.exec_stmt_impl(py, block_node)?;
                return Ok(result);
            }
        }

        // No condition was true, execute else block if present
        if let Some(else_node) = else_block {
            if !else_node.bind(py).is_none() {
                let result = self.exec_stmt_impl(py, else_node)?;
                return Ok(result);
            }
        }

        // No branch matched and no else: return None
        Ok(py.None())
    }

    /// Return from a function or lambda
    ///
    /// Args:
    ///     value: Value to return (unevaluated Op or None)
    ///
    /// Raises:
    ///     ReturnValue exception to exit function
    ///
    /// Returns:
    ///     TailCall object if the return value is a tail call (for TCO)
    pub(crate) fn op_return(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let value_node = if !args.is_empty() {
            Some(args.get_item(0)?.unbind())
        } else {
            None
        };

        // Evaluate the return value
        let result = if let Some(node) = value_node {
            self.exec_stmt_impl(py, node)?
        } else {
            py.None()
        };

        // Check if result is a TailCall - if so, return it to preserve TCO
        let result_bound = result.bind(py);
        let type_name = result_bound.get_type().name()?;
        if type_name == catnip::TAIL_CALL {
            return Ok(result);
        }

        // Raise ReturnValue exception
        let nodes_module = py.import("catnip.nodes")?;
        let return_value_class = nodes_module.getattr("ReturnValue")?;
        Err(PyErr::from_value(return_value_class.call1((result,))?))
    }

    /// Break out of the current loop
    ///
    /// Raises:
    ///     BreakLoop exception to exit loop
    pub(crate) fn op_break(
        &self,
        py: Python<'_>,
        _args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let nodes_module = py.import("catnip.nodes")?;
        let break_loop_class = nodes_module.getattr("BreakLoop")?;
        Err(PyErr::from_value(break_loop_class.call0()?))
    }

    /// Continue to the next iteration of the current loop
    ///
    /// Raises:
    ///     ContinueLoop exception to skip to next iteration
    pub(crate) fn op_continue(
        &self,
        py: Python<'_>,
        _args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let nodes_module = py.import("catnip.nodes")?;
        let continue_loop_class = nodes_module.getattr("ContinueLoop")?;
        Err(PyErr::from_value(continue_loop_class.call0()?))
    }

    /// Struct declaration - creates a CatnipStructType
    ///
    /// Args:
    ///     name: String - struct name
    ///     fields: Tuple of (name, default_or_None) pairs
    ///
    /// Behavior:
    ///     Creates a CatnipStructType (callable Rust pyclass) and stores in globals.
    ///     Handles inheritance (field merging, super_methods) and trait composition.
    ///
    /// Returns:
    ///     None
    pub(crate) fn op_struct(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "struct requires 2 arguments: name, fields",
            ));
        }

        // Extract struct name
        let name: String = args.get_item(0)?.extract()?;

        // Extract fields: each is a tuple (name, default_or_None)
        let fields_tuple = args.get_item(1)?.cast::<PyTuple>()?.clone();
        let mut field_names: Vec<String> = Vec::new();
        let mut field_defaults: Vec<Option<Py<PyAny>>> = Vec::new();

        for field_item in fields_tuple.iter() {
            let pair = field_item.cast::<PyTuple>()?;
            let fname: String = pair.get_item(0)?.extract()?;
            let default = pair.get_item(1)?;
            if default.is_none() {
                field_names.push(fname);
                field_defaults.push(None);
            } else {
                let default_val = self.exec_stmt_impl(py, default.unbind())?;
                field_names.push(fname);
                field_defaults.push(Some(default_val));
            }
        }

        // Parse optional args: implements (2), base (3), methods (4)
        // IR format: (name, fields, [implements], [base], [methods])
        let mut implements_list: Vec<String> = Vec::new();
        let mut base_name: Option<String> = None;
        let mut methods_idx: Option<usize> = None;
        if args.len() > 3 {
            let impl_obj = args.get_item(2)?;
            for imp in impl_obj.try_iter()? {
                implements_list.push(imp?.extract()?);
            }
            let item3 = args.get_item(3)?;
            if !item3.is_none() {
                if let Ok(base) = item3.extract::<String>() {
                    base_name = Some(base);
                }
            }
            if args.len() > 4 {
                methods_idx = Some(4);
            }
        } else if args.len() == 3 {
            let arg2 = args.get_item(2)?;
            let is_implements = if arg2.len()? > 0 {
                arg2.get_item(0)?.extract::<String>().is_ok()
            } else {
                true
            };
            if is_implements {
                for imp in arg2.try_iter()? {
                    implements_list.push(imp?.extract()?);
                }
            } else {
                methods_idx = Some(2);
            }
        }

        // Inheritance: merge parent fields, methods, and collect super_methods
        let mut super_methods: std::collections::HashMap<String, Py<PyAny>> =
            std::collections::HashMap::new();
        let mut parent_type_name: Option<String> = None;
        // Parent methods to inherit (child overrides win)
        let mut inherited_methods: std::collections::HashMap<String, Py<PyAny>> =
            std::collections::HashMap::new();

        if let Some(ref base) = base_name {
            let ctx = self.ctx.bind(py);
            let globals_dict = ctx.getattr("globals")?;
            let parent_obj = globals_dict.call_method1("get", (base.as_str(), py.None()))?;
            if parent_obj.is_none() {
                return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                    "Unknown base struct '{}' in extends",
                    base
                )));
            }

            parent_type_name = Some(base.clone());

            // Downcast parent to CatnipStructType
            let parent_st = parent_obj
                .cast::<crate::vm::CatnipStructType>()
                .map_err(|_| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                        "Base '{}' is not a CatnipStructType",
                        base
                    ))
                })?;
            let parent = parent_st.borrow();

            // Build super_methods: grandparent super_methods + parent methods
            for (k, v) in &parent.super_methods {
                super_methods.insert(k.clone(), v.clone_ref(py));
            }
            for (k, v) in &parent.methods {
                super_methods.insert(k.clone(), v.clone_ref(py));
                // Also collect for method inheritance (child override checked later)
                inherited_methods.insert(k.clone(), v.clone_ref(py));
            }

            // Prepend parent fields before child fields
            let mut merged_names = parent.field_names.clone();
            let mut merged_defaults: Vec<Option<Py<PyAny>>> = parent
                .field_defaults
                .iter()
                .map(|d| d.as_ref().map(|v| v.clone_ref(py)))
                .collect();
            merged_names.extend(field_names);
            merged_defaults.extend(field_defaults);
            field_names = merged_names;
            field_defaults = merged_defaults;
        }

        // Evaluate child methods
        let mut methods: std::collections::HashMap<String, Py<PyAny>> =
            std::collections::HashMap::new();
        let mut struct_method_names = std::collections::HashSet::new();
        let mut init_fn: Option<Py<PyAny>> = None;

        if let Some(midx) = methods_idx {
            let methods_obj = args.get_item(midx)?;
            for method_result in methods_obj.try_iter()? {
                let method_pair = method_result?;
                let pair_tuple = method_pair.cast::<PyTuple>()?;
                let method_name: String = pair_tuple.get_item(0)?.extract()?;
                let lambda_ir = pair_tuple.get_item(1)?;

                let callable = self.exec_stmt_impl(py, lambda_ir.unbind())?;

                if method_name == "init" {
                    init_fn = Some(callable.clone_ref(py));
                }

                struct_method_names.insert(method_name.clone());
                methods.insert(method_name, callable);
            }
        }

        // Merge inherited parent methods (child overrides win)
        for (k, v) in inherited_methods {
            if !struct_method_names.contains(&k) {
                methods.insert(k, v);
            }
        }

        // Resolve trait methods
        if !implements_list.is_empty() {
            let ctx = self.ctx.bind(py);
            let globals_dict = ctx.getattr("globals")?;
            let traits_dict = globals_dict.call_method1("get", ("__traits__", py.None()))?;

            if traits_dict.is_none() {
                return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                    "No traits registered (missing __traits__ in globals)",
                ));
            }

            let resolved =
                self.resolve_traits_ast(py, &traits_dict, &implements_list, &struct_method_names)?;

            for (mname, callable) in resolved {
                let mname_str: String = mname.extract(py)?;
                if !struct_method_names.contains(&mname_str) {
                    methods.insert(mname_str, callable);
                }
            }
        }

        // Create CatnipStructType
        let struct_type = Py::new(
            py,
            crate::vm::CatnipStructType {
                name: name.clone(),
                field_names,
                field_defaults,
                methods,
                init_fn,
                super_methods,
                parent_type_name,
            },
        )?;

        // Store in global context
        let ctx = self.ctx.bind(py);
        let globals = ctx.getattr("globals")?;
        globals.call_method1("__setitem__", (&name, &struct_type))?;

        Ok(py.None())
    }

    /// Resolve trait composition for AST mode.
    /// Uses __traits__ dict stored in ctx.globals.
    /// Returns Vec<(method_name, callable)>.
    fn resolve_traits_ast(
        &self,
        py: Python<'_>,
        traits_dict: &Bound<'_, pyo3::PyAny>,
        implements: &[String],
        struct_method_names: &std::collections::HashSet<String>,
    ) -> PyResult<Vec<(Py<PyAny>, Py<PyAny>)>> {
        // Linearize: post-order, cycle detection integrated
        let mut visiting = std::collections::HashSet::new();
        let mut visited = std::collections::HashSet::new();
        let mut linearization = Vec::new();

        for trait_name in implements {
            self.linearize_trait_ast(
                py,
                traits_dict,
                trait_name,
                &mut visiting,
                &mut visited,
                &mut linearization,
            )?;
        }

        // Merge methods: last-wins with strict conflict detection
        let mut merged_map: std::collections::HashMap<String, Py<PyAny>> =
            std::collections::HashMap::new();
        let mut method_source: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // Track insertion order
        let mut method_order: Vec<String> = Vec::new();

        for trait_name in &linearization {
            let trait_info = traits_dict.call_method1("__getitem__", (trait_name.as_str(),))?;
            let methods = trait_info.call_method1("get", ("methods", py.None()))?;
            if methods.is_none() {
                continue;
            }
            let items = methods.call_method0("items")?;
            for item in items.try_iter()? {
                let item = item?;
                let mname: String = item.get_item(0)?.extract()?;
                let callable = item.get_item(1)?;

                if let Some(prev) = method_source.get(&mname) {
                    if prev != trait_name && !struct_method_names.contains(&mname) {
                        if !self.is_ancestor_ast(traits_dict, prev, trait_name)? {
                            return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                                format!(
                                    "Method '{}' has conflicting definitions from traits '{}' and '{}'",
                                    mname, prev, trait_name
                                ),
                            ));
                        }
                    }
                } else {
                    method_order.push(mname.clone());
                }
                // Always overwrite (last-wins)
                method_source.insert(mname.clone(), trait_name.clone());
                merged_map.insert(mname.clone(), callable.unbind());
            }
        }

        // Reconstruct Vec in insertion order
        let merged = method_order
            .into_iter()
            .map(|mname| {
                let callable = merged_map.remove(&mname).unwrap();
                let key = mname.into_pyobject(py).unwrap().into_any().unbind();
                (key, callable)
            })
            .collect();

        Ok(merged)
    }

    fn linearize_trait_ast(
        &self,
        py: Python<'_>,
        traits_dict: &Bound<'_, pyo3::PyAny>,
        name: &str,
        visiting: &mut std::collections::HashSet<String>,
        visited: &mut std::collections::HashSet<String>,
        result: &mut Vec<String>,
    ) -> PyResult<()> {
        if visited.contains(name) {
            return Ok(());
        }
        if visiting.contains(name) {
            return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Cycle detected in trait hierarchy involving '{}'",
                name,
            )));
        }
        // Check trait exists
        let has = traits_dict
            .call_method1("__contains__", (name,))?
            .extract::<bool>()?;
        if !has {
            return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Trait '{}' not found",
                name,
            )));
        }

        let trait_info = traits_dict.call_method1("__getitem__", (name,))?;
        let extends = trait_info.call_method1("__getitem__", ("extends",))?;

        visiting.insert(name.to_string());
        // Recurse parents first (post-order)
        for parent in extends.try_iter()? {
            let parent = parent?;
            let parent_name: String = parent.extract()?;
            self.linearize_trait_ast(py, traits_dict, &parent_name, visiting, visited, result)?;
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        result.push(name.to_string());
        Ok(())
    }

    /// Check if `ancestor` is an ancestor of `descendant` via extends in __traits__ dict.
    fn is_ancestor_ast(
        &self,
        traits_dict: &Bound<'_, pyo3::PyAny>,
        ancestor: &str,
        descendant: &str,
    ) -> PyResult<bool> {
        let mut stack = vec![descendant.to_string()];
        let mut seen = std::collections::HashSet::new();
        while let Some(current) = stack.pop() {
            let has = traits_dict
                .call_method1("__contains__", (current.as_str(),))?
                .extract::<bool>()?;
            if !has {
                continue;
            }
            let trait_info = traits_dict.call_method1("__getitem__", (current.as_str(),))?;
            let extends = trait_info.call_method1("__getitem__", ("extends",))?;
            for parent in extends.try_iter()? {
                let parent_name: String = parent?.extract()?;
                if parent_name == ancestor {
                    return Ok(true);
                }
                if seen.insert(parent_name.clone()) {
                    stack.push(parent_name);
                }
            }
        }
        Ok(false)
    }

    /// Register a trait definition in AST mode.
    /// Stores in ctx.globals["__traits__"][name].
    pub(crate) fn op_trait_def(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 3 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "trait requires at least 3 arguments: name, extends, fields",
            ));
        }

        let name: String = args.get_item(0)?.extract()?;
        let extends_obj = args.get_item(1)?;
        let fields_obj = args.get_item(2)?;

        // Build extends list
        let mut extends: Vec<String> = Vec::new();
        for e in extends_obj.try_iter()? {
            extends.push(e?.extract()?);
        }

        // Build fields (evaluate defaults)
        let fields_list = pyo3::types::PyList::empty(py);
        for field_item in fields_obj.try_iter()? {
            let pair = field_item?.cast::<PyTuple>()?.clone();
            let fname: String = pair.get_item(0)?.extract()?;
            let default = pair.get_item(1)?;
            if default.is_none() {
                fields_list.append(PyTuple::new(
                    py,
                    &[fname.into_pyobject(py)?.into_any().unbind(), py.None()],
                )?)?;
            } else {
                let default_val = self.exec_stmt_impl(py, default.unbind())?;
                fields_list.append(PyTuple::new(
                    py,
                    &[fname.into_pyobject(py)?.into_any().unbind(), default_val],
                )?)?;
            }
        }

        // Build methods dict {name: callable}
        let methods_dict = pyo3::types::PyDict::new(py);
        if args.len() > 3 {
            let methods_obj = args.get_item(3)?;
            for method_result in methods_obj.try_iter()? {
                let method_pair = method_result?;
                let pair_tuple = method_pair.cast::<PyTuple>()?;
                let method_name: String = pair_tuple.get_item(0)?.extract()?;
                let lambda_ir = pair_tuple.get_item(1)?;

                let callable = self.exec_stmt_impl(py, lambda_ir.unbind())?;
                methods_dict.set_item(method_name, callable)?;
            }
        }

        // Build trait info dict
        let trait_info = pyo3::types::PyDict::new(py);
        trait_info.set_item(
            "extends",
            pyo3::types::PyList::new(
                py,
                extends
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )?,
        )?;
        trait_info.set_item("fields", fields_list)?;
        trait_info.set_item("methods", methods_dict)?;

        // Store in ctx.globals["__traits__"]
        let ctx = self.ctx.bind(py);
        let globals = ctx.getattr("globals")?;
        let traits_dict = globals.call_method1("get", ("__traits__", py.None()))?;
        if traits_dict.is_none() {
            let new_dict = pyo3::types::PyDict::new(py);
            new_dict.set_item(name.as_str(), trait_info)?;
            globals.call_method1("__setitem__", ("__traits__", new_dict))?;
        } else {
            traits_dict.call_method1("__setitem__", (name.as_str(), trait_info))?;
        }

        Ok(py.None())
    }
}
