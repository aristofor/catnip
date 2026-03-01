// FILE: catnip_rs/src/semantic/optimizer.rs
//! IR optimization framework - base visitor pattern for optimization passes
//!
//! Port of catnip/semantic/optimizer.pyx

use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

/// Base trait for optimization passes
///
/// Each pass implements this trait to transform IR nodes.
/// Implementors typically delegate to the helper functions (default_visit, etc.)
pub trait OptimizationPass {
    /// Visit a node and apply optimizations
    ///
    /// Dispatches to visit_ir, visit_op, visit_ref based on type.
    /// Handles lists and tuples recursively.
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>>;

    /// Visit an IR node - override in subclasses for specific operations
    fn visit_ir(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>>;

    /// Visit an Op node
    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>>;

    /// Visit a Ref node - usually passes through unchanged
    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>>;
}

/// Default visit implementation - dispatches based on type
pub fn default_visit(
    pass: &dyn OptimizationPass,
    py: Python<'_>,
    node: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    // Handle lists recursively
    if node.is_instance_of::<PyList>() {
        let list = node.cast::<PyList>()?;
        let result = PyList::empty(py);
        for item in list.iter() {
            let visited = pass.visit(py, &item)?;
            result.append(visited)?;
        }
        return Ok(result.into());
    }

    // Handle tuples recursively
    if node.is_instance_of::<PyTuple>() {
        let tuple = node.cast::<PyTuple>()?;
        let mut result = Vec::new();
        for item in tuple.iter() {
            let visited = pass.visit(py, &item)?;
            result.push(visited);
        }
        return Ok(PyTuple::new(py, &result)?.into());
    }

    // Get node type name for dispatch
    let node_type = node.get_type();
    let type_name = node_type.name()?;
    let type_name_str = type_name.to_str()?;

    match type_name_str {
        "IR" => pass.visit_ir(py, node),
        "Op" => pass.visit_op(py, node),
        "Ref" => pass.visit_ref(py, node),
        _ => {
            // Literals and other nodes pass through unchanged
            Ok(node.clone().unbind())
        }
    }
}

/// Default visit_ir implementation - visits args and kwargs recursively
pub fn default_visit_ir(
    pass: &dyn OptimizationPass,
    py: Python<'_>,
    node: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    // Visit args
    let args = node.getattr("args")?;
    let mut visited_args = Vec::new();
    for arg in args.try_iter()? {
        let arg = arg?;
        let visited = pass.visit(py, &arg)?;
        visited_args.push(visited);
    }
    let visited_args_tuple = PyTuple::new(py, &visited_args)?;

    // Visit kwargs
    let kwargs = node.getattr("kwargs")?;
    let visited_kwargs = py.import("builtins")?.getattr("dict")?.call0()?;

    if let Ok(items) = kwargs.call_method0("items") {
        for item in items.try_iter()? {
            let item = item?;
            let item_tuple = item.cast::<PyTuple>()?;
            let key = item_tuple.get_item(0)?;
            let value = item_tuple.get_item(1)?;
            let visited_value = pass.visit(py, &value)?;
            visited_kwargs.set_item(key, visited_value)?;
        }
    }

    // Check if anything changed
    let original_args = node.getattr("args")?;
    let original_kwargs = node.getattr("kwargs")?;

    let args_equal = visited_args_tuple.eq(&original_args)?;
    let kwargs_equal = visited_kwargs.eq(&original_kwargs)?;

    if args_equal && kwargs_equal {
        // Nothing changed, return original
        return Ok(node.clone().unbind());
    }

    // Create new IR with visited children
    let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
    let ident = node.getattr("ident")?;
    let new_node = ir_class.call1((ident, visited_args_tuple, visited_kwargs))?;
    Ok(new_node.unbind())
}

/// Default visit_op implementation - visits args and kwargs recursively
pub fn default_visit_op(
    pass: &dyn OptimizationPass,
    py: Python<'_>,
    node: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    // Visit args
    let args = node.getattr("args")?;
    let mut visited_args = Vec::new();
    for arg in args.try_iter()? {
        let arg = arg?;
        let visited = pass.visit(py, &arg)?;
        visited_args.push(visited);
    }
    let visited_args_tuple = PyTuple::new(py, &visited_args)?;

    // Visit kwargs
    let kwargs = node.getattr("kwargs")?;
    let visited_kwargs = py.import("builtins")?.getattr("dict")?.call0()?;

    if let Ok(items) = kwargs.call_method0("items") {
        for item in items.try_iter()? {
            let item = item?;
            let item_tuple = item.cast::<PyTuple>()?;
            let key = item_tuple.get_item(0)?;
            let value = item_tuple.get_item(1)?;
            let visited_value = pass.visit(py, &value)?;
            visited_kwargs.set_item(key, visited_value)?;
        }
    }

    // Check if anything changed
    let original_args = node.getattr("args")?;
    let original_kwargs = node.getattr("kwargs")?;

    let args_equal = visited_args_tuple.eq(&original_args)?;
    let kwargs_equal = visited_kwargs.eq(&original_kwargs)?;

    if args_equal && kwargs_equal {
        // Nothing changed, return original
        return Ok(node.clone().unbind());
    }

    // Create new Op with visited children
    let op_class = py.import("catnip.nodes")?.getattr("Op")?;
    let ident = node.getattr("ident")?;
    let new_node = op_class.call1((ident, visited_args_tuple, visited_kwargs))?;
    Ok(new_node.unbind())
}
