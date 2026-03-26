// FILE: catnip_rs/src/core/registry/arithmetic.rs
//! Arithmetic operations: add, sub, mul, div, mod, pow, neg, pos, inv

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

impl Registry {
    /// Unary negation: -value
    pub(crate) fn op_neg(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let value = self.exec_stmt_impl(py, stmt)?;
        value.call_method0(py, "__neg__")
    }

    /// Unary positive: +value
    pub(crate) fn op_pos(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let value = self.exec_stmt_impl(py, stmt)?;
        value.call_method0(py, "__pos__")
    }

    /// Bitwise inversion: ~value
    pub(crate) fn op_inv(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let value = self.exec_stmt_impl(py, stmt)?;
        value.call_method0(py, "__invert__")
    }

    /// Addition: fold left with operator.add (handles NotImplemented correctly)
    pub(crate) fn op_add(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(
            py,
            items,
            self.operator_cache.add.bind(py),
            Some(0i32.into_pyobject(py).unwrap().to_owned().unbind().into()),
        )
    }

    /// Subtraction: fold left with operator.sub
    pub(crate) fn op_sub(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(
            py,
            items,
            self.operator_cache.sub.bind(py),
            Some(0i32.into_pyobject(py).unwrap().to_owned().unbind().into()),
        )
    }

    /// Multiplication: fold left with operator.mul
    pub(crate) fn op_mul(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(
            py,
            items,
            self.operator_cache.mul.bind(py),
            Some(1i32.into_pyobject(py).unwrap().to_owned().unbind().into()),
        )
    }

    /// True division: fold left with operator.truediv
    pub(crate) fn op_truediv(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.truediv.bind(py), None)
    }

    /// Floor division: fold left with operator.floordiv
    pub(crate) fn op_floordiv(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.floordiv.bind(py), None)
    }

    /// Modulo: fold left with operator.mod
    pub(crate) fn op_mod(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.mod_.bind(py), None)
    }

    /// Power: fold left with operator.pow
    pub(crate) fn op_pow(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.pow.bind(py), None)
    }
}
