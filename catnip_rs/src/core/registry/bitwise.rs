// FILE: catnip_rs/src/core/registry/bitwise.rs
//! Bitwise operations: bit_or, bit_xor, bit_and, lshift, rshift
//!
//! Uses operator module (fold_left_operator) for correct reverse
//! operator dispatch (__rand__, __ror__, etc.).

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

impl Registry {
    /// Bitwise OR: fold left with operator.or_
    pub(crate) fn op_bit_or(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.or_.bind(py), None)
    }

    /// Bitwise XOR: fold left with operator.xor
    pub(crate) fn op_bit_xor(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.xor.bind(py), None)
    }

    /// Bitwise AND: fold left with operator.and_
    pub(crate) fn op_bit_and(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.and_.bind(py), None)
    }

    /// Left shift: fold left with operator.lshift
    pub(crate) fn op_lshift(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.lshift.bind(py), None)
    }

    /// Right shift: fold left with operator.rshift
    pub(crate) fn op_rshift(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.fold_left_operator(py, items, self.operator_cache.rshift.bind(py), None)
    }
}
