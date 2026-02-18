// FILE: catnip_rs/src/core/registry/args.rs
//! Argument unwrapping helpers shared across registry ops.

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

impl Registry {
    /// Helper: unwrap args and apply a conversion function (eval or passthrough).
    fn unwrap_args_with<F>(
        &self,
        items: &Bound<'_, PyTuple>,
        mut convert: F,
    ) -> PyResult<Vec<Py<PyAny>>>
    where
        F: FnMut(Bound<'_, PyAny>) -> PyResult<Py<PyAny>>,
    {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        // Get first argument (unevaluated - it's an AST node)
        let first_arg = items.get_item(0)?;

        // Check if it's a list (transformer wraps args in list)
        if first_arg.is_instance_of::<PyList>() {
            let list = first_arg.cast::<PyList>()?;
            let mut result = Vec::with_capacity(list.len());
            for item in list.iter() {
                result.push(convert(item)?);
            }
            return Ok(result);
        }

        // Check if it's a tuple AND items.len() == 1 (wrapped args)
        // This handles cases like ((a, b),) where args are wrapped in tuple
        if items.len() == 1 && first_arg.is_instance_of::<PyTuple>() {
            let tuple = first_arg.cast::<PyTuple>()?;
            // Only unwrap if it looks like wrapped args (multiple elements or elements are AST nodes)
            if tuple.len() > 1
                || (tuple.len() == 1 && !tuple.get_item(0)?.is_instance_of::<PyTuple>())
            {
                let mut result = Vec::with_capacity(tuple.len());
                for item in tuple.iter() {
                    result.push(convert(item)?);
                }
                return Ok(result);
            }
        }

        // Default: evaluate each item from the outer tuple directly
        let mut result = Vec::with_capacity(items.len());
        for i in 0..items.len() {
            let item = items.get_item(i)?;
            result.push(convert(item)?);
        }
        Ok(result)
    }

    /// Helper: unwrap and evaluate arguments from tuple.
    pub(crate) fn unwrap_and_eval_args(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        self.unwrap_args_with(items, |item| self.exec_stmt_impl(py, item.unbind()))
    }

    /// Helper: unwrap args nodes without evaluating (for short-circuit).
    pub(crate) fn unwrap_args_nodes(&self, items: &Bound<'_, PyTuple>) -> PyResult<Vec<Py<PyAny>>> {
        self.unwrap_args_with(items, |item| Ok(item.unbind()))
    }
}
