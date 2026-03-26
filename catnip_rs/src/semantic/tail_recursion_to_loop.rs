// FILE: catnip_rs/src/semantic/tail_recursion_to_loop.rs
//! Tail recursion to loop transformation
//!
//! Transforms tail-recursive functions into iterative while loops.
//! This eliminates the trampoline overhead and provides native loop performance.
//!
//! Example transformation:
//! ```ignore
//! factorial = (n, acc=1) => {
//!     if n <= 1 { acc }
//!     else { factorial(n - 1, n * acc) }
//! }
//! ```
//!
//! Becomes (internally):
//! ```ignore
//! factorial = (n, acc=1) => {
//!     while true {
//!         if n <= 1 { return acc }
//!         // Rebind parameters for next iteration
//!         tmp_n = n - 1
//!         tmp_acc = n * acc
//!         n = tmp_n
//!         acc = tmp_acc
//!     }
//! }
//! ```

use super::opcode::OpCode;
use super::optimizer::{OptimizationPass, default_visit_ir};
use crate::constants::*;
use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyTuple};

#[pyclass(name = "TailRecursionToLoopPass")]
pub struct TailRecursionToLoopPass;

impl Default for TailRecursionToLoopPass {
    fn default() -> Self {
        Self::new()
    }
}

impl TailRecursionToLoopPass {
    pub fn new() -> Self {
        TailRecursionToLoopPass
    }
}

#[pymethods]
impl TailRecursionToLoopPass {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for TailRecursionToLoopPass {
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit(self, py, node)
    }

    fn visit_ir(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        default_visit_ir(self, py, node)
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // First visit children
        let visited = super::optimizer::default_visit_op(self, py, node)?;
        let visited_bound = visited.bind(py);

        // Check if this is a SET_LOCALS with a lambda
        let ident = visited_bound.getattr("ident")?;
        let ident_int: i32 = ident.extract()?;

        if let Some(opcode) = OpCode::from_i32(ident_int) {
            if opcode == OpCode::SET_LOCALS {
                // Try to transform the lambda if it's tail-recursive
                if let Ok(transformed) = self.try_transform_set_locals(py, visited_bound) {
                    return Ok(transformed);
                }
            }
        }

        Ok(visited)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node.clone().unbind())
    }
}

impl TailRecursionToLoopPass {
    /// Try to transform a SET_LOCALS if it contains a tail-recursive lambda
    fn try_transform_set_locals(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        if args_tuple.len() < 2 {
            return Ok(node.clone().unbind());
        }

        let names = args_tuple.get_item(0)?;
        let value = args_tuple.get_item(1)?;

        // Check if value is a lambda
        let value_type = value.get_type();
        let type_name_bound = value_type.name()?;
        let type_name = type_name_bound.to_str()?;
        if type_name != catnip::OP {
            return Ok(node.clone().unbind());
        }

        let value_ident = value.getattr("ident")?;
        let value_ident_int: i32 = value_ident.extract()?;
        if let Some(opcode) = OpCode::from_i32(value_ident_int) {
            if opcode != OpCode::OP_LAMBDA {
                return Ok(node.clone().unbind());
            }
        } else {
            return Ok(node.clone().unbind());
        }

        // Get function name from names tuple
        let names_tuple = match names.cast::<PyTuple>() {
            Ok(tuple) => tuple,
            Err(_) => {
                return Ok(node.clone().unbind());
            }
        };
        if names_tuple.is_empty() {
            return Ok(node.clone().unbind());
        }
        let func_name_lvalue = names_tuple.get_item(0)?;

        // Lvalue has a .value attribute containing the string
        let func_name_str: String = match func_name_lvalue.getattr("value") {
            Ok(attr) => match attr.extract() {
                Ok(s) => s,
                Err(_) => {
                    return Ok(node.clone().unbind());
                }
            },
            Err(_) => {
                return Ok(node.clone().unbind());
            }
        };

        // Check if lambda is tail-recursive
        if !self.is_tail_recursive(py, &value, &func_name_str)? {
            return Ok(node.clone().unbind());
        }

        // Transform the lambda
        let transformed_lambda = self.transform_lambda_to_loop(py, &value, &func_name_str)?;

        // Create new SET_LOCALS with transformed lambda
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let new_args = PyTuple::new(py, &[names.unbind(), transformed_lambda])?;
        let ident = node.getattr("ident")?;
        let kwargs = node.getattr("kwargs")?;
        op_class.call1((ident, new_args, kwargs)).map(|obj| obj.unbind())
    }

    /// Check if a lambda is tail-recursive (all recursive calls are tail calls)
    fn is_tail_recursive(&self, py: Python<'_>, lambda_node: &Bound<'_, PyAny>, func_name: &str) -> PyResult<bool> {
        let args = lambda_node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;
        if args_tuple.len() < 2 {
            return Ok(false);
        }

        let body = args_tuple.get_item(1)?;

        // Collect all calls to func_name
        let calls = self.find_all_calls(py, &body, func_name)?;

        if calls.is_empty() {
            return Ok(false); // Not recursive
        }

        // Check if all calls have tail=True
        for call in calls.iter() {
            let call_bound = call.bind(py);
            let has_tail = call_bound.hasattr("tail")?;
            if !has_tail {
                return Ok(false);
            }
            let tail_value: bool = call_bound.getattr("tail")?.extract()?;
            if !tail_value {
                return Ok(false); // Found a non-tail recursive call
            }
        }

        Ok(true)
    }

    /// Find all CALL nodes to a specific function
    fn find_all_calls(&self, py: Python<'_>, node: &Bound<'_, PyAny>, func_name: &str) -> PyResult<Vec<Py<PyAny>>> {
        let mut calls = Vec::new();

        // Check if this node is a CALL to func_name
        if self.is_call_to_function(py, node, func_name)? {
            calls.push(node.clone().unbind());
        }

        // Recursively search in children
        if let Ok(args) = node.getattr("args") {
            if let Ok(args_iter) = args.try_iter() {
                for arg in args_iter {
                    let arg = arg?;
                    // Handle tuples/lists
                    if let Ok(iter) = arg.try_iter() {
                        for item in iter {
                            let item = item?;
                            calls.extend(self.find_all_calls(py, &item, func_name)?);
                        }
                    } else {
                        calls.extend(self.find_all_calls(py, &arg, func_name)?);
                    }
                }
            }
        }

        Ok(calls)
    }

    /// Check if a node is a CALL to a specific function
    fn is_call_to_function(&self, _py: Python<'_>, node: &Bound<'_, PyAny>, func_name: &str) -> PyResult<bool> {
        // Check if it's an Op node
        let node_type = node.get_type();
        let type_name_bound = node_type.name()?;
        let type_name = type_name_bound.to_str()?;
        if type_name != catnip::OP {
            return Ok(false);
        }

        // Check if opcode is CALL
        let ident = node.getattr("ident")?;
        let ident_int: i32 = ident.extract()?;
        if let Some(opcode) = OpCode::from_i32(ident_int) {
            if opcode != OpCode::CALL {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }

        // Check if first argument is a Ref to func_name
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;
        if args_tuple.is_empty() {
            return Ok(false);
        }

        let first_arg = args_tuple.get_item(0)?;
        let first_arg_type = first_arg.get_type();
        let first_arg_type_name_bound = first_arg_type.name()?;
        let first_arg_type_name = first_arg_type_name_bound.to_str()?;
        if first_arg_type_name != "Ref" {
            return Ok(false);
        }

        // Access .ident attribute directly instead of str() which returns "<Ref name>"
        let ref_name: String = first_arg.getattr("ident")?.extract()?;
        Ok(ref_name == func_name)
    }

    /// Transform a tail-recursive lambda into a loop-based implementation
    fn transform_lambda_to_loop(
        &self,
        py: Python<'_>,
        lambda_node: &Bound<'_, PyAny>,
        func_name: &str,
    ) -> PyResult<Py<PyAny>> {
        let args = lambda_node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        let params = args_tuple.get_item(0)?;
        let body = args_tuple.get_item(1)?;

        // Transform body to loop form
        let loop_body = self.transform_body_to_loop(py, &body, &params, func_name)?;

        // Create new lambda with loop body
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let ident = lambda_node.getattr("ident")?;
        let new_args = PyTuple::new(py, &[params.unbind(), loop_body])?;
        let kwargs = lambda_node.getattr("kwargs")?;
        let result = op_class.call1((ident, new_args, kwargs)).map(|obj| obj.unbind())?;
        Ok(result)
    }

    /// Transform the body of a tail-recursive function into a while loop
    fn transform_body_to_loop(
        &self,
        py: Python<'_>,
        body: &Bound<'_, PyAny>,
        params: &Bound<'_, PyAny>,
        func_name: &str,
    ) -> PyResult<Py<PyAny>> {
        // Transform tail-recursive body into while loop:
        // factorial = (n, acc) => {
        //     if n <= 1 { acc }
        //     else { factorial(n - 1, n * acc) }
        // }
        // Becomes:
        // factorial = (n, acc) => {
        //     while true {
        //         if n <= 1 { return acc }
        //         tmp_0 = n - 1
        //         tmp_1 = n * acc
        //         n = tmp_0
        //         acc = tmp_1
        //     }
        // }

        // Extract parameter names from Params object

        // Check the actual type of params
        let params_type = params.get_type();
        let _params_type_name = params_type.name()?;

        // params is a Params object with .args attribute
        let params_args = match params.getattr("args") {
            Ok(args) => args,
            Err(_) => {
                // Maybe params is already the tuple
                params.clone()
            }
        };
        let params_tuple = params_args.cast::<PyTuple>()?;

        let param_names: Vec<String> = (0..params_tuple.len())
            .filter_map(|i| {
                let param = params_tuple.get_item(i).ok()?;

                // Each param is a tuple (name, default_value)
                // Try to downcast to tuple first
                if let Ok(param_tuple) = param.cast::<PyTuple>() {
                    if param_tuple.len() >= 1 {
                        let name_item = param_tuple.get_item(0).ok()?;

                        // The name might be an Identifier with .value, or a direct string
                        if let Ok(value_attr) = name_item.getattr("value") {
                            if let Ok(name) = value_attr.extract::<String>() {
                                return Some(name);
                            }
                        }

                        if let Ok(name) = name_item.extract::<String>() {
                            return Some(name);
                        }

                        if let Ok(name_py) = name_item.str() {
                            if let Ok(name) = name_py.extract::<String>() {
                                return Some(name);
                            }
                        }
                    }
                }

                None
            })
            .collect();

        // Try to detect simple pattern: if cond { base_case } else { tail_call }
        // This allows generating optimized: while !cond { rebindings } return base_case
        match self.detect_simple_if_pattern(py, body, &param_names) {
            Ok((exit_cond, base_case, tail_call)) => {
                self.generate_optimized_loop(py, &exit_cond, &base_case, &tail_call, &param_names)
            }
            Err(_e) => {
                // Generic transformation: while true { transformed_body }
                // transform_node_for_loop handles nested ifs, tail calls → rebinding, base cases → return
                let transformed_body = self.transform_node_for_loop(py, body, &param_names, func_name)?;

                // Wrap in while true { ... }
                // In Catnip, literals are direct Python values, not Op nodes
                let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
                let while_ident = OpCode::OP_WHILE as i32;
                let true_literal: Py<PyAny> = PyBool::new(py, true).to_owned().into_any().unbind();
                let while_args = PyTuple::new(py, &[true_literal, transformed_body])?;
                let kwargs = PyDict::new(py);
                op_class
                    .call1((while_ident, while_args, kwargs))
                    .map(|obj| obj.unbind())
            }
        }
    }

    /// Wrap a node in OP_RETURN
    fn wrap_in_return(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let return_ident = OpCode::OP_RETURN as i32;
        let return_args = PyTuple::new(py, &[node.clone().unbind()])?;
        let kwargs = PyDict::new(py);
        op_class
            .call1((return_ident, return_args, kwargs))
            .map(|obj| obj.unbind())
    }

    /// Transform a node for loop body (recursively handle if/match branches, replace tail calls)
    /// - Tail calls to func_name → rebinding (loop continues)
    /// - Other tail calls and base cases → wrapped in RETURN (exits the loop)
    fn transform_node_for_loop(
        &self,
        py: Python<'_>,
        node: &Bound<'_, PyAny>,
        param_names: &[String],
        func_name: &str,
    ) -> PyResult<Py<PyAny>> {
        // Get node type
        let node_type = node.get_type();
        let type_name_bound = node_type.name()?;
        let type_name = type_name_bound.to_str()?;

        // Non-Op nodes (Ref, literals, etc.) are base cases → wrap in RETURN
        if type_name != catnip::OP {
            return self.wrap_in_return(py, node);
        }

        // Get opcode
        let ident = node.getattr("ident")?;
        let ident_int: i32 = ident.extract()?;

        if let Some(opcode) = OpCode::from_i32(ident_int) {
            let result = match opcode {
                OpCode::OP_IF => {
                    // Transform if branches recursively
                    self.transform_if_for_loop(py, node, param_names, func_name)
                }
                OpCode::OP_RETURN => {
                    // Already a return, transform its content
                    let args = node.getattr("args")?;
                    let args_tuple = args.cast::<PyTuple>()?;
                    if args_tuple.is_empty() {
                        return Ok(node.clone().unbind());
                    }
                    let inner = args_tuple.get_item(0)?;
                    self.transform_node_for_loop(py, &inner, param_names, func_name)
                }
                OpCode::CALL => {
                    // Check if this is a tail call TO THE RECURSIVE FUNCTION
                    let has_tail = node.hasattr("tail")?;
                    if has_tail {
                        let tail_value: bool = node.getattr("tail")?.extract()?;
                        if tail_value && self.is_call_to_function(py, node, func_name)? {
                            // Recursive tail call → rebinding (loop continues)
                            return self.transform_tail_call_to_rebinding(py, node, param_names);
                        }
                    }
                    // Non-recursive call or non-tail call is a base case → wrap in RETURN
                    self.wrap_in_return(py, node)
                }
                OpCode::OP_BLOCK => {
                    // Transform block: only the last statement is the "return value"
                    let args = node.getattr("args")?;
                    let args_tuple = args.cast::<PyTuple>()?;
                    if args_tuple.is_empty() {
                        return Ok(node.clone().unbind());
                    }

                    let len = args_tuple.len();
                    let mut transformed_stmts = Vec::new();

                    // Keep intermediate statements as-is (side effects)
                    for i in 0..len - 1 {
                        let stmt = args_tuple.get_item(i)?;
                        transformed_stmts.push(stmt.unbind());
                    }

                    // Transform only the last statement (the return expression)
                    let last_stmt = args_tuple.get_item(len - 1)?;
                    let transformed_last = self.transform_node_for_loop(py, &last_stmt, param_names, func_name)?;
                    transformed_stmts.push(transformed_last);

                    let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
                    let new_args = PyTuple::new(py, &transformed_stmts)?;
                    let kwargs = node.getattr("kwargs")?;
                    op_class.call1((ident, new_args, kwargs)).map(|obj| obj.unbind())
                }
                _ => {
                    // Other operations (arithmetic, comparison, etc.) are base cases → wrap in RETURN
                    self.wrap_in_return(py, node)
                }
            }?;
            Ok(result)
        } else {
            // Unknown opcode, treat as base case
            self.wrap_in_return(py, node)
        }
    }

    /// Transform if branches for loop body
    /// Each branch is transformed recursively - transform_node_for_loop handles RETURN wrapping
    fn transform_if_for_loop(
        &self,
        py: Python<'_>,
        if_node: &Bound<'_, PyAny>,
        param_names: &[String],
        func_name: &str,
    ) -> PyResult<Py<PyAny>> {
        let args = if_node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        if args_tuple.is_empty() {
            return Ok(if_node.clone().unbind());
        }

        // op_if expects: args[0] = list of (condition, block) tuples, args[1] = else block
        let branches = args_tuple.get_item(0)?;
        let branches_list = branches.cast::<PyTuple>()?;

        // Transform each branch - transform_node_for_loop handles base case → RETURN
        let mut transformed_branches = Vec::new();
        for branch in branches_list.iter() {
            let branch_tuple = branch.cast::<PyTuple>()?;
            if branch_tuple.len() < 2 {
                transformed_branches.push(branch.unbind());
                continue;
            }

            let condition = branch_tuple.get_item(0)?;
            let block = branch_tuple.get_item(1)?;

            // Transform the block - this will wrap base cases in RETURN, leave rebindings as-is
            let transformed_block = self.transform_node_for_loop(py, &block, param_names, func_name)?;

            // Create new branch tuple
            let new_branch = PyTuple::new(py, &[condition.unbind(), transformed_block])?;
            transformed_branches.push(new_branch.unbind().into());
        }

        let transformed_branches_tuple = PyTuple::new(py, &transformed_branches)?;

        // Transform else branch if exists
        let transformed_else = if args_tuple.len() >= 2 {
            let else_branch = args_tuple.get_item(1)?;

            // The semantic analyzer may wrap tail calls in RETURN.
            // We need to unwrap the RETURN to access the actual tail call for transformation.
            let else_unwrapped = if let Ok(ident) = else_branch.getattr("ident") {
                let ident_int: i32 = ident.extract()?;
                if let Some(OpCode::OP_RETURN) = OpCode::from_i32(ident_int) {
                    // Unwrap the RETURN to get the actual content
                    let return_args = else_branch.getattr("args")?;
                    let return_args_tuple = return_args.cast::<PyTuple>()?;
                    if !return_args_tuple.is_empty() {
                        return_args_tuple.get_item(0)?
                    } else {
                        else_branch
                    }
                } else {
                    else_branch
                }
            } else {
                else_branch
            };

            self.transform_node_for_loop(py, &else_unwrapped, param_names, func_name)?
        } else {
            py.None()
        };

        // Create new if with transformed branches
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let ident = if_node.getattr("ident")?;
        let new_args = PyTuple::new(py, &[transformed_branches_tuple.unbind().into(), transformed_else])?;
        let kwargs = if_node.getattr("kwargs")?;

        op_class.call1((ident, new_args, kwargs)).map(|obj| obj.unbind())
    }

    /// Transform a tail call into parameter rebinding
    ///
    /// Uses a two-phase approach (like SSA phi nodes):
    /// Phase 1: Evaluate all arguments into temporaries
    /// Phase 2: Rebind all parameters from temporaries
    ///
    /// This ensures arguments are evaluated with OLD parameter values.
    /// Handles both positional args and keyword args.
    fn transform_tail_call_to_rebinding(
        &self,
        py: Python<'_>,
        call_node: &Bound<'_, PyAny>,
        param_names: &[String],
    ) -> PyResult<Py<PyAny>> {
        // Extract call arguments
        let args = call_node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        if args_tuple.len() < 1 {
            return Ok(call_node.clone().unbind());
        }

        // Get positional arguments (skip first which is the function reference)
        // args_tuple is: (function_ref, arg1, arg2, ...)
        let num_positional = args_tuple.len() - 1;

        // Extract kwargs
        let kwargs_obj = call_node.getattr("kwargs")?;
        let kwargs_dict = kwargs_obj.cast::<PyDict>().ok();

        // Build arg_values: maps param index → value
        // Start with positional args
        let mut arg_values: Vec<Option<Py<PyAny>>> = std::iter::repeat_with(|| None).take(param_names.len()).collect();
        for (i, slot) in arg_values.iter_mut().enumerate().take(num_positional) {
            let arg = args_tuple.get_item(i + 1)?;
            *slot = Some(arg.unbind());
        }

        // Add kwargs
        if let Some(kwargs) = kwargs_dict {
            for (key, value) in kwargs.iter() {
                let key_str: String = key.extract()?;
                if let Some(param_idx) = param_names.iter().position(|p| p == &key_str) {
                    arg_values[param_idx] = Some(value.unbind());
                }
            }
        }

        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let set_locals_ident = OpCode::SET_LOCALS as i32;
        let mut statements = Vec::new();

        // Phase 1: Evaluate all arguments into temporaries
        // This ensures all args are evaluated with OLD parameter values
        for (i, arg_opt) in arg_values.iter().enumerate() {
            if let Some(arg) = arg_opt {
                let tmp_name = format!("_tmp_{}", i);
                let names = PyTuple::new(py, [tmp_name])?;
                let arg_bound = arg.bind(py);
                let set_local_args = PyTuple::new(py, [names.into_any().unbind(), arg_bound.clone().unbind()])?;
                let kwargs = PyDict::new(py);
                let stmt = op_class.call1((set_locals_ident, set_local_args, kwargs))?;
                statements.push(stmt.unbind());
            }
        }

        // Phase 2: Rebind all parameters from temporaries
        let ref_class = py.import(PY_MOD_NODES)?.getattr("Ref")?;
        for (i, arg_opt) in arg_values.iter().enumerate() {
            if arg_opt.is_some() {
                let param_name = &param_names[i];
                let tmp_name = format!("_tmp_{}", i);
                let names = PyTuple::new(py, std::slice::from_ref(param_name))?;
                let ref_value = ref_class.call1((tmp_name,))?.unbind();
                let set_local_args = PyTuple::new(py, [names.into_any().unbind(), ref_value])?;
                let kwargs = PyDict::new(py);
                let stmt = op_class.call1((set_locals_ident, set_local_args, kwargs))?;
                statements.push(stmt.unbind());
            }
        }

        // Add None as last statement so BLOCK doesn't return the rebindings
        statements.push(py.None());

        // Wrap all statements in a BLOCK
        let block_ident = OpCode::OP_BLOCK as i32;
        let block_args = PyTuple::new(py, &statements)?;
        let kwargs = PyDict::new(py);

        let result = op_class
            .call1((block_ident, block_args, kwargs))
            .map(|obj| obj.unbind())?;
        Ok(result)
    }

    /// Detect simple if pattern: if cond { base_case } else { tail_call }
    /// Returns (exit_condition, base_case, tail_call) if pattern matches
    fn detect_simple_if_pattern<'py>(
        &self,
        py: Python<'py>,
        body: &Bound<'py, PyAny>,
        _param_names: &[String],
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>, Bound<'py, PyAny>)> {
        // Check if body is OP_BLOCK
        let body_type = body.get_type();
        let body_type_name = body_type.name()?;
        if body_type_name.to_str()? != catnip::OP {
            return Err(pyo3::exceptions::PyValueError::new_err("Not an Op node"));
        }

        // Unwrap all nested OP_BLOCKs to find the core OP_IF
        // Body might be OP_BLOCK(OP_BLOCK(...OP_IF...)) due to semantic analysis
        let if_node = self.unwrap_block(py, body)?;

        // Check if if_node is OP_IF
        let if_ident = if_node.getattr("ident")?;
        let if_ident_int: i32 = if_ident.extract()?;
        let if_opcode =
            OpCode::from_i32(if_ident_int).ok_or_else(|| pyo3::exceptions::PyValueError::new_err("Invalid opcode"))?;

        if if_opcode != OpCode::OP_IF {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Not an if node, got opcode {}",
                if_ident_int
            )));
        }

        // Extract branches: args = (branches_tuple, else_block)
        let args = if_node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;
        if args_tuple.len() != 2 {
            return Err(pyo3::exceptions::PyValueError::new_err("Invalid if structure"));
        }

        let branches = args_tuple.get_item(0)?;
        let else_block = args_tuple.get_item(1)?;

        // We only handle single branch (no elif)
        let branches_tuple = branches.cast::<PyTuple>()?;
        if branches_tuple.len() != 1 {
            return Err(pyo3::exceptions::PyValueError::new_err("Multiple branches"));
        }

        // Get the single (condition, then_body) pair
        let branch_pair = branches_tuple.get_item(0)?;
        let pair_tuple = branch_pair.cast::<PyTuple>()?;
        if pair_tuple.len() != 2 {
            return Err(pyo3::exceptions::PyValueError::new_err("Invalid branch structure"));
        }

        let condition = pair_tuple.get_item(0)?;
        let then_body = pair_tuple.get_item(1)?;

        // Determine which branch is the base case and which is the tail call
        let then_has_tail = self.contains_tail_call(py, &then_body)?;
        let else_has_tail = self.contains_tail_call(py, &else_block)?;

        let (exit_cond, base_case, tail_call) = if then_has_tail && !else_has_tail {
            // Pattern: if cond { tail_call } else { base_case }
            // We want: while cond { tail_call }; return base_case
            // So exit condition is !cond (when !cond, we exit and return base_case)
            let not_cond = self.negate_condition(py, &condition)?;
            (not_cond, else_block, then_body)
        } else if !then_has_tail && else_has_tail {
            // Pattern: if cond { base_case } else { tail_call }
            // We want: while !cond { tail_call }; return base_case
            // So exit condition is cond (when cond, we exit and return base_case)
            (condition, then_body, else_block)
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Both or neither branch has tail call",
            ));
        };

        Ok((exit_cond, base_case, tail_call))
    }

    /// Check if a node contains a tail call
    fn contains_tail_call(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<bool> {
        // Unwrap OP_BLOCK if present
        let unwrapped = self.unwrap_block(py, node)?;

        // Check if it's a CALL node
        let node_type = unwrapped.get_type();
        let type_name_bound = node_type.name()?;
        let type_name = type_name_bound.to_str()?;
        if type_name != catnip::OP {
            return Ok(false);
        }

        let ident = unwrapped.getattr("ident")?;
        let ident_int: i32 = ident.extract()?;
        let opcode = OpCode::from_i32(ident_int);

        Ok(opcode == Some(OpCode::CALL))
    }

    /// Unwrap OP_BLOCK to get the inner node (recursively unwraps nested blocks)
    fn unwrap_block<'py>(&self, _py: Python<'py>, node: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let node_type = node.get_type();
        let type_name_bound = node_type.name()?;
        let type_name = type_name_bound.to_str()?;
        if type_name != catnip::OP {
            return Ok(node.clone());
        }

        let ident = node.getattr("ident")?;
        let ident_int: i32 = ident.extract()?;
        let opcode = OpCode::from_i32(ident_int);

        if opcode != Some(OpCode::OP_BLOCK) {
            return Ok(node.clone());
        }

        // Get single statement from block
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;
        if args_tuple.len() == 1 {
            // Recursively unwrap nested blocks
            let inner = args_tuple.get_item(0)?;
            self.unwrap_block(_py, &inner)
        } else {
            Ok(node.clone())
        }
    }

    /// Negate a condition (converts cond to NOT(cond))
    fn negate_condition<'py>(&self, py: Python<'py>, cond: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let not_ident = OpCode::NOT as i32;
        let args = PyTuple::new(py, &[cond.clone().unbind()])?;
        let kwargs = PyDict::new(py);
        op_class.call1((not_ident, args, kwargs))
    }

    /// Generate optimized loop: while !exit_cond { rebindings } return base_case
    fn generate_optimized_loop(
        &self,
        py: Python<'_>,
        exit_cond: &Bound<'_, PyAny>,
        base_case: &Bound<'_, PyAny>,
        tail_call: &Bound<'_, PyAny>,
        param_names: &[String],
    ) -> PyResult<Py<PyAny>> {
        // Unwrap blocks
        let tail_call_unwrapped = self.unwrap_block(py, tail_call)?;

        // Transform tail call to rebindings
        let rebindings = self.transform_tail_call_to_rebinding(py, &tail_call_unwrapped, param_names)?;

        // Create while loop: while !exit_cond { rebindings }
        let op_class = py.import(PY_MOD_NODES)?.getattr("Op")?;
        let while_ident = OpCode::OP_WHILE as i32;

        // Negate exit condition for while loop
        let loop_cond = self.negate_condition(py, exit_cond)?;

        let while_args = PyTuple::new(py, &[loop_cond.unbind(), rebindings])?;
        let kwargs = PyDict::new(py);
        let while_node = op_class.call1((while_ident, while_args, kwargs))?;

        // Unwrap base case from OP_BLOCK if needed
        let base_case_unwrapped = self.unwrap_block(py, base_case)?;

        // Create block: { while_loop; return base_case }
        let block_ident = OpCode::OP_BLOCK as i32;
        let block_stmts = PyTuple::new(py, &[while_node.unbind(), base_case_unwrapped.unbind()])?;
        let block_kwargs = PyDict::new(py);
        let block_node = op_class.call1((block_ident, block_stmts, block_kwargs))?;

        Ok(block_node.unbind())
    }
}
