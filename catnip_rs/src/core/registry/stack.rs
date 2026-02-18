// FILE: catnip_rs/src/core/registry/stack.rs
//! Stack operations: push, pop, push_peek

use super::Registry;
use pyo3::prelude::*;

impl Registry {
    /// Push a value onto the stack (no return value)
    pub(crate) fn op_push(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<()> {
        let result = self.exec_stmt_impl(py, stmt)?;
        self.stack.borrow_mut().push(result);
        Ok(())
    }

    /// Push a value onto the stack and return it
    pub(crate) fn op_push_peek(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let result = self.exec_stmt_impl(py, stmt)?;
        self.stack.borrow_mut().push(result.clone_ref(py));
        Ok(result)
    }

    /// Pop a value from the stack
    pub(crate) fn op_pop(&self, _py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.stack
            .borrow_mut()
            .pop()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyIndexError, _>("Stack is empty"))
    }
}
