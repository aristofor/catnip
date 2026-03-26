// FILE: catnip_rs/src/core/registry/logical.rs
//! Logical operations: bool_not, bool_or, bool_and, lt, le, gt, ge, eq, ne

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

impl Registry {
    /// Boolean NOT: not value
    pub(crate) fn op_bool_not(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let value = self.exec_stmt_impl(py, stmt)?;
        let is_true = value.bind(py).is_truthy()?;
        Ok((!is_true).into_pyobject(py).unwrap().to_owned().unbind().into())
    }

    /// Boolean OR: short-circuit evaluation
    /// Note: For short-circuit, we need to eval items one by one, not all at once
    pub(crate) fn op_bool_or(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let nodes = self.unwrap_args_nodes(items)?;
        for node in nodes {
            let value = self.exec_stmt_impl(py, node)?;
            if value.bind(py).is_truthy()? {
                return Ok(true.into_pyobject(py).unwrap().to_owned().unbind().into());
            }
        }
        Ok(false.into_pyobject(py).unwrap().to_owned().unbind().into())
    }

    /// Boolean AND: short-circuit evaluation
    pub(crate) fn op_bool_and(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let nodes = self.unwrap_args_nodes(items)?;
        for node in nodes {
            let value = self.exec_stmt_impl(py, node)?;
            if !value.bind(py).is_truthy()? {
                return Ok(false.into_pyobject(py).unwrap().to_owned().unbind().into());
            }
        }
        Ok(true.into_pyobject(py).unwrap().to_owned().unbind().into())
    }

    /// Nil-coalescing: a ?? b (short-circuit, returns a if not None, else b)
    pub(crate) fn op_null_coalesce(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let nodes = self.unwrap_args_nodes(items)?;
        let left = self.exec_stmt_impl(py, nodes[0].clone_ref(py))?;
        if !left.bind(py).is_none() {
            return Ok(left);
        }
        self.exec_stmt_impl(py, nodes[1].clone_ref(py))
    }

    /// Less than: a < b < c < ...
    pub(crate) fn op_lt(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.chained_comparison(py, items, |a, b| a.lt(b))
    }

    /// Less than or equal: a <= b <= c <= ...
    pub(crate) fn op_le(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.chained_comparison(py, items, |a, b| a.le(b))
    }

    /// Greater than: a > b > c > ...
    pub(crate) fn op_gt(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.chained_comparison(py, items, |a, b| a.gt(b))
    }

    /// Greater than or equal: a >= b >= c >= ...
    pub(crate) fn op_ge(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.chained_comparison(py, items, |a, b| a.ge(b))
    }

    /// Equality: a == b == c == ...
    pub(crate) fn op_eq(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.chained_comparison(py, items, |a, b| a.eq(b))
    }

    /// Not equal: a != b != c != ...
    pub(crate) fn op_ne(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.chained_comparison(py, items, |a, b| a.ne(b))
    }

    /// Membership test: item in container
    pub(crate) fn op_in(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let args = self.unwrap_and_eval_args(py, items)?;
        if args.len() < 2 {
            return Err(pyo3::exceptions::PyTypeError::new_err("'in' requires two arguments"));
        }
        let item = args[0].bind(py);
        let container = args[1].bind(py);
        let result = container.contains(item)?;
        Ok(result.into_pyobject(py).unwrap().to_owned().unbind().into())
    }

    /// Negated membership test: item not in container
    pub(crate) fn op_not_in(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let args = self.unwrap_and_eval_args(py, items)?;
        if args.len() < 2 {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "'not in' requires two arguments",
            ));
        }
        let item = args[0].bind(py);
        let container = args[1].bind(py);
        let result = !container.contains(item)?;
        Ok(result.into_pyobject(py).unwrap().to_owned().unbind().into())
    }

    /// Identity test: a is b
    pub(crate) fn op_is(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let args = self.unwrap_and_eval_args(py, items)?;
        if args.len() < 2 {
            return Err(pyo3::exceptions::PyTypeError::new_err("'is' requires two arguments"));
        }
        let a = args[0].bind(py);
        let b = args[1].bind(py);
        let result = a.is(b);
        Ok(result.into_pyobject(py).unwrap().to_owned().unbind().into())
    }

    /// Negated identity test: a is not b
    pub(crate) fn op_is_not(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let args = self.unwrap_and_eval_args(py, items)?;
        if args.len() < 2 {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "'is not' requires two arguments",
            ));
        }
        let a = args[0].bind(py);
        let b = args[1].bind(py);
        let result = !a.is(b);
        Ok(result.into_pyobject(py).unwrap().to_owned().unbind().into())
    }
}
