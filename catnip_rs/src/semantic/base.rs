// FILE: catnip_rs/src/semantic/base.rs
//! Base classes for optimization passes - Python exports
//!
//! Port of catnip/semantic/_optimizer.pyx

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

/// Base class for optimization passes (Python export)
///
/// This is a Python-facing wrapper that provides the OptimizationPass interface.
/// Subclasses override visit_ir() to implement specific optimizations.
#[pyclass(subclass)]
#[derive(Debug)]
pub struct OptimizationPassBase {
    #[pyo3(get, set)]
    pub name: String,
}

#[pymethods]
impl OptimizationPassBase {
    #[new]
    #[pyo3(signature = (name="OptimizationPass"))]
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    /// Visit a node and apply optimizations
    ///
    /// Dispatches to visit_ir, visit_op, visit_ref based on type.
    /// Handles lists and tuples recursively.
    fn visit(&self, py: Python<'_>, node: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let bound = node.bind(py);

        // Handle lists recursively
        if bound.is_instance_of::<PyList>() {
            let list = bound.cast::<PyList>()?;
            let result = PyList::empty(py);
            for item in list.iter() {
                let visited = self.visit(py, item.unbind())?;
                result.append(visited)?;
            }
            return Ok(result.into());
        }

        // Handle tuples recursively
        if bound.is_instance_of::<PyTuple>() {
            let tuple = bound.cast::<PyTuple>()?;
            let mut result = Vec::new();
            for item in tuple.iter() {
                let visited = self.visit(py, item.unbind())?;
                result.push(visited);
            }
            return Ok(PyTuple::new(py, &result)?.into());
        }

        // Get node type name for dispatch
        let node_type = bound.get_type();
        let type_name = node_type.name()?;
        let type_name_str = type_name.to_str()?;

        match type_name_str {
            "IR" => self.visit_ir(py, node),
            "Op" => self.visit_op(py, node),
            "Ref" => self.visit_ref(py, node),
            _ => {
                // Literals and other nodes pass through unchanged
                Ok(node)
            }
        }
    }

    /// Visit an IR node - override in subclasses for specific operations
    fn visit_ir(&self, py: Python<'_>, node: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let bound = node.bind(py);

        // Visit args
        let args = bound.getattr("args")?;
        let mut visited_args = Vec::new();
        for arg in args.try_iter()? {
            let arg = arg?;
            let visited = self.visit(py, arg.unbind())?;
            visited_args.push(visited);
        }
        let visited_args_tuple = PyTuple::new(py, &visited_args)?;

        // Visit kwargs
        let kwargs = bound.getattr("kwargs")?;
        let visited_kwargs = PyDict::new(py);

        if let Ok(items) = kwargs.call_method0("items") {
            for item in items.try_iter()? {
                let item = item?;
                let item_tuple = item.cast::<PyTuple>()?;
                let key = item_tuple.get_item(0)?;
                let value = item_tuple.get_item(1)?;
                let visited_value = self.visit(py, value.unbind())?;
                visited_kwargs.set_item(key, visited_value)?;
            }
        }

        // Check if anything changed
        let original_args = bound.getattr("args")?;
        let original_kwargs = bound.getattr("kwargs")?;

        let args_equal = visited_args_tuple.eq(&original_args)?;
        let kwargs_equal = visited_kwargs.eq(&original_kwargs)?;

        if args_equal && kwargs_equal {
            // Nothing changed, return original
            return Ok(node);
        }

        // Create new IR with visited children
        let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
        let ident = bound.getattr("ident")?;
        let new_node = ir_class.call1((ident, visited_args_tuple, visited_kwargs))?;
        Ok(new_node.unbind())
    }

    /// Visit an Op node (rare before semantic analysis)
    fn visit_op(&self, py: Python<'_>, node: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let bound = node.bind(py);

        // Visit args
        let args = bound.getattr("args")?;
        let mut visited_args = Vec::new();
        for arg in args.try_iter()? {
            let arg = arg?;
            let visited = self.visit(py, arg.unbind())?;
            visited_args.push(visited);
        }
        let visited_args_tuple = PyTuple::new(py, &visited_args)?;

        // Visit kwargs
        let kwargs = bound.getattr("kwargs")?;
        let visited_kwargs = PyDict::new(py);

        if let Ok(items) = kwargs.call_method0("items") {
            for item in items.try_iter()? {
                let item = item?;
                let item_tuple = item.cast::<PyTuple>()?;
                let key = item_tuple.get_item(0)?;
                let value = item_tuple.get_item(1)?;
                let visited_value = self.visit(py, value.unbind())?;
                visited_kwargs.set_item(key, visited_value)?;
            }
        }

        // Check if anything changed
        let original_args = bound.getattr("args")?;
        let original_kwargs = bound.getattr("kwargs")?;

        let args_equal = visited_args_tuple.eq(&original_args)?;
        let kwargs_equal = visited_kwargs.eq(&original_kwargs)?;

        if args_equal && kwargs_equal {
            // Nothing changed, return original
            return Ok(node);
        }

        // Create new Op with visited children
        let op_class = py.import("catnip._rs")?.getattr("Op")?;
        let ident = bound.getattr("ident")?;
        let new_node = op_class.call1((ident, visited_args_tuple, visited_kwargs))?;
        Ok(new_node.unbind())
    }

    /// Visit a Ref node - usually passes through unchanged
    fn visit_ref(&self, _py: Python<'_>, node: Py<PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node)
    }
}

/// Main optimizer that applies multiple optimization passes to IR
#[pyclass]
#[derive(Debug)]
pub struct Optimizer {
    #[pyo3(get, set)]
    pub passes: Py<PyList>,
}

#[pymethods]
impl Optimizer {
    #[new]
    #[pyo3(signature = (passes=None))]
    fn new(py: Python<'_>, passes: Option<Py<PyList>>) -> PyResult<Self> {
        let passes_list = if let Some(p) = passes {
            p
        } else {
            // Default optimization pipeline
            let semantic_module = py.import("catnip.semantic")?;

            let blunt_code = semantic_module.getattr("BluntCodePass")?.call0()?;
            let constant_propagation = semantic_module
                .getattr("ConstantPropagationPass")?
                .call0()?;
            let constant_folding = semantic_module.getattr("ConstantFoldingPass")?.call0()?;
            let copy_propagation = semantic_module.getattr("CopyPropagationPass")?.call0()?;
            let function_inlining = semantic_module.getattr("FunctionInliningPass")?.call0()?;
            let dead_store = semantic_module
                .getattr("DeadStoreEliminationPass")?
                .call0()?;
            let strength_reduction = semantic_module.getattr("StrengthReductionPass")?.call0()?;
            let block_flattening = semantic_module.getattr("BlockFlatteningPass")?.call0()?;
            let dead_code = semantic_module
                .getattr("DeadCodeEliminationPass")?
                .call0()?;
            let cse = semantic_module
                .getattr("CommonSubexpressionEliminationPass")?
                .call0()?;
            // TailRecursionToLoopPass moved to post-semantic phase (see analyzer.rs)
            // NOTE: BlockFlatteningPass and DeadCodeEliminationPass will be replaced
            // by CFG-based optimizations once CFG reconstruction is properly implemented

            let list = PyList::empty(py);
            list.append(blunt_code)?;
            list.append(constant_propagation)?;
            list.append(constant_folding)?;
            list.append(copy_propagation)?;
            list.append(function_inlining)?;
            list.append(dead_store)?;
            list.append(strength_reduction)?;
            list.append(block_flattening)?;
            list.append(dead_code)?;
            list.append(cse)?;

            list.unbind()
        };

        Ok(Self {
            passes: passes_list,
        })
    }

    /// Apply all optimization passes to the IR
    ///
    /// Multiple iterations allow optimizations to enable each other.
    #[pyo3(signature = (ir, max_iterations=10))]
    fn optimize(
        &self,
        py: Python<'_>,
        ir: Py<PyAny>,
        max_iterations: usize,
    ) -> PyResult<Py<PyAny>> {
        let mut optimized = ir;

        for _iteration in 0..max_iterations {
            let prev = optimized.clone_ref(py);

            // Apply all passes
            let passes_ref = self.passes.bind(py);
            for pass_obj in passes_ref.iter() {
                let pass_bound = pass_obj;
                let result = pass_bound.call_method1("visit", (optimized,))?;
                optimized = result.unbind();
            }

            // Check if anything changed
            let changed = !optimized.bind(py).eq(&prev)?;
            if !changed {
                break; // No more optimizations possible
            }
        }

        Ok(optimized)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let passes_ref = self.passes.bind(py);
        let n_passes = passes_ref.len();
        Ok(format!("<Optimizer with {} passes>", n_passes))
    }
}

pub fn register_module(parent_module: &Bound<'_, PyModule>) -> PyResult<()> {
    parent_module.add_class::<OptimizationPassBase>()?;
    parent_module.add_class::<Optimizer>()?;
    Ok(())
}
