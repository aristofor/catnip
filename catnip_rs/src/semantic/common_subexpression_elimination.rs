// FILE: catnip_rs/src/semantic/common_subexpression_elimination.rs
//! Common Subexpression Elimination (CSE) optimization pass
//!
//! Port of CommonSubexpressionEliminationPass from catnip/semantic/optimizer.pyx
//!
//! Detects repeated subexpressions and extracts them into temporary variables:
//! - a + b * 2 + b * 2 → _cse_0 = b * 2; a + _cse_0 + _cse_0
//!
//! Limited to pure operations (arithmetic, comparison, logical, bitwise, getattr, getitem).
//! Works at block level only (not inter-blocks).

use super::opcode::OpCode;
use super::optimizer::{default_visit_ir, OptimizationPass};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::collections::{HashMap, HashSet};

#[pyclass(name = "CommonSubexpressionEliminationPass")]
pub struct CommonSubexpressionEliminationPass {
    pure_ops: HashSet<i32>,
}

#[pymethods]
impl CommonSubexpressionEliminationPass {
    #[new]
    fn new() -> Self {
        let mut pure_ops = HashSet::new();

        // Arithmetic
        pure_ops.insert(OpCode::ADD as i32);
        pure_ops.insert(OpCode::SUB as i32);
        pure_ops.insert(OpCode::MUL as i32);
        pure_ops.insert(OpCode::TRUEDIV as i32);
        pure_ops.insert(OpCode::FLOORDIV as i32);
        pure_ops.insert(OpCode::MOD as i32);
        pure_ops.insert(OpCode::POW as i32);
        pure_ops.insert(OpCode::NEG as i32);
        pure_ops.insert(OpCode::POS as i32);

        // Comparison
        pure_ops.insert(OpCode::EQ as i32);
        pure_ops.insert(OpCode::NE as i32);
        pure_ops.insert(OpCode::LT as i32);
        pure_ops.insert(OpCode::LE as i32);
        pure_ops.insert(OpCode::GT as i32);
        pure_ops.insert(OpCode::GE as i32);

        // Logical
        pure_ops.insert(OpCode::AND as i32);
        pure_ops.insert(OpCode::OR as i32);
        pure_ops.insert(OpCode::NOT as i32);

        // Bitwise
        pure_ops.insert(OpCode::BAND as i32);
        pure_ops.insert(OpCode::BOR as i32);
        pure_ops.insert(OpCode::BXOR as i32);
        pure_ops.insert(OpCode::BNOT as i32);
        pure_ops.insert(OpCode::LSHIFT as i32);
        pure_ops.insert(OpCode::RSHIFT as i32);

        // Member access
        pure_ops.insert(OpCode::GETATTR as i32);
        pure_ops.insert(OpCode::GETITEM as i32);

        CommonSubexpressionEliminationPass { pure_ops }
    }

    /// Visit a node and apply optimizations
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for CommonSubexpressionEliminationPass {
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit(self, py, node)
    }

    fn visit_ir(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // First visit children
        let visited = default_visit_ir(self, py, node)?;
        let visited_bound = visited.bind(py);

        // Check if result is still an IR node
        let node_type = visited_bound.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;
        if type_name != "IR" {
            return Ok(visited);
        }

        // CSE only applies to blocks
        let ident = visited_bound.getattr("ident")?;
        if !self.is_op_match(&ident, OpCode::OP_BLOCK)? {
            return Ok(visited);
        }

        // Apply CSE to block
        self.cse_block(py, visited_bound)
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit_op(self, py, node)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node.clone().unbind())
    }
}

impl CommonSubexpressionEliminationPass {
    /// Apply CSE to a block
    ///
    /// 1. Collect all subexpressions and count their occurrences
    /// 2. For those repeated 2+ times, create temporary variables
    /// 3. Inject assignments at the start of the block
    /// 4. Replace occurrences with Refs
    fn cse_block(&self, py: Python<'_>, block: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // eprintln!("CSE: Starting cse_block");
        let args = block.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        // Step 1: Collect and count subexpressions
        // eprintln!("CSE: Collecting subexpressions from {} statements", args_tuple.len());
        let mut expr_counts: HashMap<u64, (usize, Py<PyAny>)> = HashMap::new();
        for stmt in args_tuple.iter() {
            self.collect_subexpressions(py, &stmt, &mut expr_counts)?;
        }
        // eprintln!("CSE: Found {} unique subexpressions", expr_counts.len());

        // Step 2: Create temporary variables for repeated expressions (count >= 2)
        // eprintln!("CSE: Creating temp variables for repeated expressions");
        let mut expr_to_var: HashMap<u64, (String, Py<PyAny>)> = HashMap::new();
        let mut var_counter = 0;
        for (expr_hash, (count, first_node)) in expr_counts.iter() {
            if *count >= 2 {
                let var_name = format!("_cse_{}", var_counter);
                // eprintln!("CSE: Expression {} repeated {} times -> {}", expr_hash, count, var_name);
                expr_to_var.insert(*expr_hash, (var_name, first_node.clone_ref(py)));
                var_counter += 1;
            }
        }

        // If no CSE possible, return original block
        if expr_to_var.is_empty() {
            // eprintln!("CSE: No CSE opportunities found");
            return Ok(block.clone().unbind());
        }
        // eprintln!("CSE: Found {} CSE opportunities", expr_to_var.len());

        // Step 3 & 4: Replace occurrences AND inject CSE at the right time
        let mut new_statements = Vec::new();
        let mut cse_injected = HashSet::new();

        for stmt in args_tuple.iter() {
            // Determine which CSE are used in this statement
            let cse_used = self.find_cse_in_statement(py, &stmt, &expr_to_var)?;

            // Inject CSE not yet injected
            for (expr_hash, var_name) in &cse_used {
                if !cse_injected.contains(var_name) {
                    let (_var_name, expr_node) = &expr_to_var[expr_hash];

                    // Create assignment: SET_LOCALS((var_name,), expr_node)
                    let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
                    let set_locals_ident =
                        (OpCode::SET_LOCALS as i32).into_pyobject(py)?.into_any();

                    // First arg: tuple of variable names
                    let var_names_tuple = PyTuple::new(py, &[var_name.clone()])?;

                    // Second arg: expression node
                    let assignment_args = PyTuple::new(
                        py,
                        &[var_names_tuple.into_any().unbind(), expr_node.clone_ref(py)],
                    )?;

                    let empty_dict = PyDict::new(py);
                    let assignment =
                        ir_class.call1((set_locals_ident, assignment_args, empty_dict))?;

                    new_statements.push(assignment.unbind());
                    cse_injected.insert(var_name.clone());
                }
            }

            // Replace and add the statement
            let replaced_stmt = self.replace_subexpressions(py, &stmt, &expr_to_var)?;
            new_statements.push(replaced_stmt);
        }

        // Create new block with injected CSE assignments
        let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
        let block_ident = block.getattr("ident")?;
        let new_args = PyTuple::new(py, &new_statements)?;
        let kwargs = block.getattr("kwargs")?;
        let new_block = ir_class.call1((block_ident, new_args, kwargs))?;
        Ok(new_block.unbind())
    }

    /// Find all CSE used in this statement
    ///
    /// Returns a list of (expr_hash, var_name) for the found CSE
    fn find_cse_in_statement(
        &self,
        py: Python<'_>,
        node: &Bound<'_, PyAny>,
        expr_to_var: &HashMap<u64, (String, Py<PyAny>)>,
    ) -> PyResult<Vec<(u64, String)>> {
        let mut found = Vec::new();

        // Recursively traverse structures
        let node_type = node.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;

        // Handle lists and tuples (but not strings!)
        if type_name != "str" && type_name != "bytes" {
            if let Ok(iter) = node.try_iter() {
                for item in iter {
                    let item = item?;
                    let sub_found = self.find_cse_in_statement(py, &item, expr_to_var)?;
                    found.extend(sub_found);
                }
                return Ok(found);
            }
        }

        // Only IR nodes can be CSE
        if type_name != "IR" {
            return Ok(found);
        }

        // Check if this node itself is a CSE
        let ident = node.getattr("ident")?;
        if self.is_pure_op(&ident)? && self.is_worth_cse(py, node)? {
            if let Ok(hashable_repr) = self.make_hashable(py, node) {
                let node_hash = self.hash_pyobject(py, &hashable_repr)?;
                if let Some((var_name, _)) = expr_to_var.get(&node_hash) {
                    found.push((node_hash, var_name.clone()));
                }
            }
        }

        // Descend into arguments
        let args = node.getattr("args")?;
        if let Ok(args_tuple) = args.cast::<PyTuple>() {
            for arg in args_tuple.iter() {
                let sub_found = self.find_cse_in_statement(py, &arg, expr_to_var)?;
                found.extend(sub_found);
            }
        }

        let kwargs = node.getattr("kwargs")?;
        if let Ok(items) = kwargs.call_method0("items") {
            if let Ok(items_iter) = items.try_iter() {
                for item in items_iter {
                    let item = item?;
                    if let Ok(pair) = item.cast::<PyTuple>() {
                        if pair.len() >= 2 {
                            let v = pair.get_item(1)?;
                            let sub_found = self.find_cse_in_statement(py, &v, expr_to_var)?;
                            found.extend(sub_found);
                        }
                    }
                }
            }
        }

        Ok(found)
    }

    /// Replace subexpressions with Refs to temporary variables
    fn replace_subexpressions(
        &self,
        py: Python<'_>,
        node: &Bound<'_, PyAny>,
        expr_to_var: &HashMap<u64, (String, Py<PyAny>)>,
    ) -> PyResult<Py<PyAny>> {
        let node_type = node.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;

        // Handle lists
        if type_name == "list" {
            if let Ok(iter) = node.try_iter() {
                let mut result = Vec::new();
                for item in iter {
                    let item = item?;
                    let replaced = self.replace_subexpressions(py, &item, expr_to_var)?;
                    result.push(replaced);
                }
                return Ok(py
                    .import("builtins")?
                    .getattr("list")?
                    .call1((result,))?
                    .unbind());
            }
        }

        // Handle tuples
        if type_name == "tuple" {
            if let Ok(tuple) = node.cast::<PyTuple>() {
                let mut result = Vec::new();
                for item in tuple.iter() {
                    let replaced = self.replace_subexpressions(py, &item, expr_to_var)?;
                    result.push(replaced);
                }
                return Ok(PyTuple::new(py, &result)?.into_any().unbind());
            }
        }

        // Only IR nodes can be replaced
        if type_name != "IR" {
            return Ok(node.clone().unbind());
        }

        // Check if this node should be replaced
        let ident = node.getattr("ident")?;
        if self.is_pure_op(&ident)? && self.is_worth_cse(py, node)? {
            if let Ok(hashable_repr) = self.make_hashable(py, node) {
                let node_hash = self.hash_pyobject(py, &hashable_repr)?;
                if let Some((var_name, _)) = expr_to_var.get(&node_hash) {
                    // Replace with Ref
                    let ref_class = py.import("catnip.nodes")?.getattr("Ref")?;
                    return ref_class.call1((var_name,)).map(|r| r.unbind());
                }
            }
        }

        // Recursively replace in arguments
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;
        let mut replaced_args = Vec::new();
        for arg in args_tuple.iter() {
            let replaced = self.replace_subexpressions(py, &arg, expr_to_var)?;
            replaced_args.push(replaced);
        }

        let kwargs = node.getattr("kwargs")?;
        let replaced_kwargs = PyDict::new(py);
        if let Ok(items) = kwargs.call_method0("items") {
            if let Ok(items_iter) = items.try_iter() {
                for item in items_iter {
                    let item = item?;
                    if let Ok(pair) = item.cast::<PyTuple>() {
                        if pair.len() >= 2 {
                            let k = pair.get_item(0)?;
                            let v = pair.get_item(1)?;
                            let replaced = self.replace_subexpressions(py, &v, expr_to_var)?;
                            replaced_kwargs.set_item(k, replaced)?;
                        }
                    }
                }
            }
        }

        // Check if anything changed
        let new_args_tuple = PyTuple::new(py, &replaced_args)?;
        let args_equal = args_tuple.eq(&new_args_tuple)?;
        let kwargs_equal = kwargs.eq(&replaced_kwargs)?;

        if args_equal && kwargs_equal {
            return Ok(node.clone().unbind());
        }

        // Return new IR with replaced args
        let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
        let new_node = ir_class.call1((ident, new_args_tuple, replaced_kwargs))?;
        Ok(new_node.unbind())
    }

    /// Recursively collect all pure subexpressions
    ///
    /// Updates expr_counts: {hash(expr): (count, node)}
    fn collect_subexpressions(
        &self,
        py: Python<'_>,
        node: &Bound<'_, PyAny>,
        expr_counts: &mut HashMap<u64, (usize, Py<PyAny>)>,
    ) -> PyResult<()> {
        // eprintln!("CSE: collect_subexpressions called");
        let node_type = node.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;
        // eprintln!("CSE: collect node type: {}", type_name);

        // Recursively collect from structures (but not strings!)
        if type_name != "str" && type_name != "bytes" {
            if let Ok(iter) = node.try_iter() {
                // eprintln!("CSE: Node is iterable, recursing");
                for item in iter {
                    let item = item?;
                    self.collect_subexpressions(py, &item, expr_counts)?;
                }
                return Ok(());
            }
        }

        // For non-IR nodes, nothing to do
        if type_name != "IR" {
            // eprintln!("CSE: Non-IR node, skipping");
            return Ok(());
        }

        // Don't descend into control flow operations
        let ident = node.getattr("ident")?;
        // eprintln!("CSE: IR node, checking control flow");
        if self.is_control_flow(&ident)? {
            // eprintln!("CSE: Control flow node, skipping");
            return Ok(());
        }

        // Try to count this node if it's pure and worth CSE
        // eprintln!("CSE: Checking if pure and worth CSE");
        if self.is_pure_op(&ident)? && self.is_worth_cse(py, node)? {
            // eprintln!("CSE: Node is pure and worth CSE, making hashable");
            if let Ok(hashable_repr) = self.make_hashable(py, node) {
                // eprintln!("CSE: Hashable repr created, hashing");
                if let Ok(node_hash) = self.hash_pyobject(py, &hashable_repr) {
                    // eprintln!("CSE: Hash computed: {}", node_hash);
                    expr_counts
                        .entry(node_hash)
                        .and_modify(|(count, _)| *count += 1)
                        .or_insert((1, node.clone().unbind()));
                }
            }
        }

        // Descend into arguments to visit subexpressions
        let args = node.getattr("args")?;
        if let Ok(args_tuple) = args.cast::<PyTuple>() {
            for arg in args_tuple.iter() {
                self.collect_subexpressions(py, &arg, expr_counts)?;
            }
        }

        let kwargs = node.getattr("kwargs")?;
        if let Ok(items) = kwargs.call_method0("items") {
            if let Ok(items_iter) = items.try_iter() {
                for item in items_iter {
                    let item = item?;
                    if let Ok(pair) = item.cast::<PyTuple>() {
                        if pair.len() >= 2 {
                            let v = pair.get_item(1)?;
                            self.collect_subexpressions(py, &v, expr_counts)?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Recursively convert an object to a hashable form
    ///
    /// Replaces lists with tuples, etc.
    fn make_hashable(&self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let obj_type = obj.get_type();
        let type_name_obj = obj_type.name()?;
        let type_name = type_name_obj.to_str()?;
        // eprintln!("CSE: make_hashable for type: {}", type_name);

        match type_name {
            "list" => {
                if let Ok(iter) = obj.try_iter() {
                    let mut result = Vec::new();
                    for item in iter {
                        let item = item?;
                        let hashable = self.make_hashable(py, &item)?;
                        result.push(hashable);
                    }
                    return Ok(PyTuple::new(py, &result)?.into_any().unbind());
                }
            }
            "tuple" => {
                if let Ok(tuple) = obj.cast::<PyTuple>() {
                    let mut result = Vec::new();
                    for item in tuple.iter() {
                        let hashable = self.make_hashable(py, &item)?;
                        result.push(hashable);
                    }
                    return Ok(PyTuple::new(py, &result)?.into_any().unbind());
                }
            }
            "dict" => {
                if let Ok(items) = obj.call_method0("items") {
                    if let Ok(items_iter) = items.try_iter() {
                        let mut pairs = Vec::new();
                        for item in items_iter {
                            let item = item?;
                            if let Ok(pair) = item.cast::<PyTuple>() {
                                if pair.len() >= 2 {
                                    let k = pair.get_item(0)?;
                                    let v = pair.get_item(1)?;
                                    let hashable_v = self.make_hashable(py, &v)?;
                                    pairs.push((k.unbind(), hashable_v));
                                }
                            }
                        }
                        // Sort pairs for consistent hashing
                        pairs.sort_by_key(|(k, _)| format!("{:?}", k));
                        return Ok(PyTuple::new(py, &pairs)?.into_any().unbind());
                    }
                }
            }
            "IR" => {
                // Hash based on ident and hashable args ONLY
                // DO NOT include kwargs - they contain metadata like start_byte/end_byte
                // which differ between identical expressions at different positions
                let ident = obj.getattr("ident")?;
                let args = obj.getattr("args")?;

                let hashable_args = self.make_hashable(py, &args)?;

                // Build hashable representation: ("IR", ident, args)
                // kwargs are intentionally excluded to allow CSE of identical expressions
                // at different source positions
                let result_list = vec![
                    "IR".into_pyobject(py)?.into_any().unbind(),
                    ident.unbind(),
                    hashable_args,
                ];
                return Ok(PyTuple::new(py, &result_list)?.into_any().unbind());
            }
            "Identifier" => {
                let str_repr = obj.str()?;
                let result_list = vec![
                    "Ident".into_pyobject(py)?.into_any().unbind(),
                    str_repr.into_any().unbind(),
                ];
                return Ok(PyTuple::new(py, &result_list)?.into_any().unbind());
            }
            "Ref" => {
                let ident = obj.getattr("ident")?;
                let result_list =
                    vec!["Ref".into_pyobject(py)?.into_any().unbind(), ident.unbind()];
                return Ok(PyTuple::new(py, &result_list)?.into_any().unbind());
            }
            _ => {}
        }

        // Check if object has items() method (dict-like)
        if let Ok(items) = obj.call_method0("items") {
            if let Ok(items_iter) = items.try_iter() {
                let mut pairs = Vec::new();
                for item in items_iter {
                    let item = item?;
                    if let Ok(pair) = item.cast::<PyTuple>() {
                        if pair.len() >= 2 {
                            let k = pair.get_item(0)?;
                            let v = pair.get_item(1)?;
                            let hashable_v = self.make_hashable(py, &v)?;
                            pairs.push((k.unbind(), hashable_v));
                        }
                    }
                }
                pairs.sort_by_key(|(k, _)| format!("{:?}", k));
                return Ok(PyTuple::new(py, &pairs)?.into_any().unbind());
            }
        }

        // Primitives (int, str, etc.) are directly hashable
        // Try to hash it, if it fails return str representation
        match py.import("builtins")?.getattr("hash")?.call1((obj,)) {
            Ok(_) => Ok(obj.clone().unbind()),
            Err(_) => {
                let str_repr = obj.str()?;
                Ok(str_repr.into_any().unbind())
            }
        }
    }

    /// Hash a PyObject using Python's hash() function
    fn hash_pyobject(&self, py: Python<'_>, obj: &Py<PyAny>) -> PyResult<u64> {
        let hash_value = py.import("builtins")?.getattr("hash")?.call1((obj,))?;
        let hash_int = hash_value.extract::<i64>()?;
        Ok(hash_int as u64)
    }

    /// Check if an expression deserves to be CSE'd
    ///
    /// Ignores expressions that are too simple (e.g., a single Identifier/Ref, a constant)
    fn is_worth_cse(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<bool> {
        let node_type = node.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;

        if type_name != "IR" {
            return Ok(false);
        }

        // An expression is worth it if it has at least 1 non-trivial element
        let mut complexity = 0;

        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;

        // Check if first arg is a list or tuple (chained operations)
        let mut args_to_check: Vec<Bound<'_, PyAny>> = Vec::new();
        if args_tuple.len() > 0 {
            let first_arg = args_tuple.get_item(0)?;
            // Check if it's a list/tuple (not string!)
            let first_type = first_arg.get_type();
            let first_type_name_obj = first_type.name()?;
            let first_type_name = first_type_name_obj.to_str()?;
            if first_type_name != "str" && first_type_name != "bytes" {
                if let Ok(iter) = first_arg.try_iter() {
                    for item in iter {
                        args_to_check.push(item?);
                    }
                } else {
                    for arg in args_tuple.iter() {
                        args_to_check.push(arg);
                    }
                }
            } else {
                for arg in args_tuple.iter() {
                    args_to_check.push(arg);
                }
            }
        }

        for arg in &args_to_check {
            let arg_type = arg.get_type();
            let arg_type_name_obj = arg_type.name()?;
            let arg_type_name = arg_type_name_obj.to_str()?;

            match arg_type_name {
                "IR" => complexity += 2,         // Sub-operation = complex
                "Identifier" => complexity += 1, // Identifier = somewhat complex
                "Ref" => complexity += 1,        // Ref = somewhat complex
                _ => {}                          // Constants don't count
            }
        }

        Ok(complexity >= 1)
    }

    /// Check if ident matches an opcode
    fn is_op_match(&self, ident: &Bound<'_, PyAny>, opcode: OpCode) -> PyResult<bool> {
        if let Ok(int_val) = ident.extract::<i32>() {
            return Ok(int_val == opcode as i32);
        }
        Ok(false)
    }

    /// Check if ident is a pure operation
    fn is_pure_op(&self, ident: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(int_val) = ident.extract::<i32>() {
            return Ok(self.pure_ops.contains(&int_val));
        }
        Ok(false)
    }

    /// Check if ident is a control flow operation
    fn is_control_flow(&self, ident: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(int_val) = ident.extract::<i32>() {
            return Ok(matches!(
                int_val,
                x if x == OpCode::OP_IF as i32
                    || x == OpCode::OP_WHILE as i32
                    || x == OpCode::OP_FOR as i32
                    || x == OpCode::OP_MATCH as i32
                    || x == OpCode::OP_BLOCK as i32
                    || x == OpCode::OP_RETURN as i32
                    || x == OpCode::OP_BREAK as i32
                    || x == OpCode::OP_CONTINUE as i32
            ));
        }
        Ok(false)
    }
}
